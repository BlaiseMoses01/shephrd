//! `shephrd hook`: the thin client Claude Code runs as the PermissionRequest hook.
//!
//! Golden rule: if anything goes wrong, print nothing and exit 0. Claude Code
//! then shows the normal permission dialog, so failures fall back to you,
//! never to an automatic allow.

use crate::claude_request::{HookOutput, PermissionRequest};
use crate::judges::{Review, Verdict};
use std::io::Read;
use std::time::Duration;

/// `timeout` is how long to wait for the daemon: the judge's timeout plus a
/// little headroom, so the daemon's own timeout fires first and is reported.
pub async fn run(url: &str, timeout: Duration) {
    // Recursion guard #2: if this somehow fires inside the judge's own session, stay out.
    if std::env::var_os("SHEPHRD_JUDGE").is_some() {
        return;
    }

    match ask_daemon(url, timeout).await {
        Ok(Some(output)) => match serde_json::to_string(&output) {
            Ok(json) => println!("{json}"),
            Err(e) => eprintln!("shephrd: couldn't serialize hook output: {e}"),
        },
        Ok(None) => {} // daemon chose not to decide; fall through to the dialog
        Err(e) => eprintln!("shephrd: falling back to the permission dialog: {e:#}"),
    }
}

async fn ask_daemon(url: &str, timeout: Duration) -> anyhow::Result<Option<HookOutput>> {
    let mut stdin = String::new();
    std::io::stdin().read_to_string(&mut stdin)?;
    // Parse locally first so bad input fails here, not as a confusing HTTP error.
    let event: PermissionRequest = serde_json::from_str(&stdin)?;

    let resp = reqwest::Client::new()
        .post(format!("{url}/review"))
        .json(&event)
        .timeout(timeout)
        .send()
        .await?
        .error_for_status()?;

    if resp.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(None);
    }

    let review: Review = resp.json().await?;
    // stderr shows up in `claude --debug`, handy for seeing why something was allowed.
    eprintln!("shephrd: {:?}: {}", review.verdict, review.reason);

    Ok(Some(hook_output(&review)))
}

/// Turn a verdict into what Claude Code should do. Only ALLOW decides on its
/// own; anything else hands the call to you, with the judge's reasoning shown
/// so you know whether it saw a risk or just couldn't tell.
pub fn hook_output(review: &Review) -> HookOutput {
    match review.verdict {
        Verdict::Allow => HookOutput::allow(),
        Verdict::Escalate => {
            HookOutput::ask_user(format!("shephrd couldn't verify this: {}", review.reason))
        }
        Verdict::Deny => HookOutput::ask_user(format!("shephrd flagged a risk: {}", review.reason)),
    }
}
