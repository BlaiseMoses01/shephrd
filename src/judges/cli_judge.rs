//! The judge: a read-only `claude -p` run inside the agent's cwd.

use super::{Review, Verdict};
use crate::claude_request::PermissionRequest;
use crate::config::Config;
use crate::prompt;
use crate::transcript::SessionContext;
use anyhow::{Context, bail};
use serde::Deserialize;
use std::{path::Path, process::Stdio};
use tokio::{io::AsyncWriteExt, process::Command};

/// Extra settings for the judge's own session.
/// - `disableAllHooks`: the judge's tool calls must never trigger our hook
///   (otherwise judge -> hook -> judge -> ... forever).
/// - deny rules: belt-and-braces on top of `--tools`; if a flag ever gets
///   renamed or ignored, these still block writes and secret reads.
const JUDGE_SETTINGS: &str = r#"{
  "disableAllHooks": true,
  "permissions": {
    "deny": [
      "Read(**/.env)", "Read(**/.env.*)", "Read(**/*.pem)", "Read(**/*.key)",
      "Read(~/.ssh/**)", "Read(~/.aws/**)", "Read(~/.config/gh/**)",
      "Bash", "Edit", "Write", "NotebookEdit", "WebFetch", "WebSearch", "Task"
    ]
  }
}"#;

/// JSON Schema passed to `--json-schema`, so the CLI validates the reply shape for us.
/// `reason` comes first so the model reasons before committing to a verdict,
/// rather than writing a reason to justify one it already picked.
const VERDICT_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "reason":  { "type": "string" },
    "verdict": { "type": "string", "enum": ["ALLOW", "ESCALATE", "DENY"] }
  },
  "required": ["reason", "verdict"]
}"#;

/// The part of the model's reply we care about.
#[derive(Debug, Deserialize)]
struct JudgeReply {
    verdict: Verdict,
    reason: String,
}

/// The CLI judge's `{{tool_access}}` line: it can open files under the root.
const TOOL_ACCESS: &str = "You may use Read, Grep and Glob inside the project to check what the \
command will actually do, e.g. open a script it runs and follow its imports.";

/// `root` is the directory the judge may read. The caller pins and checks it
/// (see `server::pin_root`); we never trust `event.cwd` directly here.
pub async fn run_judge(
    event: &PermissionRequest,
    root: &Path,
    ctx: &SessionContext,
    config: &Config,
) -> anyhow::Result<Review> {
    let prompt = prompt::reviewer_prompt(&config.reviewer_template, event, ctx, TOOL_ACCESS);

    // `Command::new` + `.args`, never `sh -c "..."`: no shell means the prompt
    // and command text can't break quoting or inject anything.
    let mut child = Command::new("claude")
        // The read pin. Claude Code auto-approves reads inside its working
        // directory; anything outside would need a permission prompt, and in
        // -p mode there's nobody to answer, so it's refused. Symlinks are
        // resolved before the check, so `ln -s ~/.ssh x` doesn't escape.
        // Do NOT add a bare `--allowedTools Read`: that approves reads
        // *everywhere* and silently removes the pin.
        .current_dir(root)
        .args([
            "-p",
            "--model",
            &config.model,
            "--output-format",
            "json",
            "--json-schema",
            VERDICT_SCHEMA,
            "--tools",
            "Read,Grep,Glob",      // the only tools that exist in this session
            "--strict-mcp-config", // no MCP servers
            "--setting-sources",
            "user", // ignore the repo's .claude/ (agent-writable)
            "--settings",
            JUDGE_SETTINGS,
        ])
        .env("SHEPHRD_JUDGE", "1") // recursion guard #2, checked in `shephrd hook`
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true) // if the timeout fires and we drop this, the process dies too
        .spawn()
        .context("failed to spawn `claude` (is it on PATH?)")?;

    // Prompt goes on stdin: no argument-length limits. Dropping stdin sends EOF,
    // which is claude's cue to start; without it, it waits forever.
    let mut stdin = child.stdin.take().expect("stdin is piped");
    stdin.write_all(prompt.as_bytes()).await?;
    drop(stdin);

    // Two `?`s: the outer one is the timeout, the inner one is the process I/O.
    let out = tokio::time::timeout(config.timeout(), child.wait_with_output())
        .await
        .context("judge timed out")??;

    if !out.status.success() {
        // With --output-format json, claude reports most failures (auth, model,
        // API errors) as a JSON envelope on *stdout*, with the readable message
        // in `result`. Fall back to the raw text if it isn't that shape.
        let stdout = String::from_utf8_lossy(&out.stdout);
        let message = serde_json::from_str::<serde_json::Value>(&stdout)
            .ok()
            .and_then(|v| v["result"].as_str().map(str::to_string))
            .unwrap_or_else(|| stdout.trim().to_string());
        bail!(
            "judge exited with {}\n  message: {}\n  stderr: {}",
            out.status,
            message,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    let reply = parse_reply(&out.stdout)?;
    Ok(Review {
        verdict: reply.verdict,
        reason: reply.reason,
        event: event.clone(),
    })
}

/// `--output-format json` wraps the answer in an envelope. With `--json-schema`
/// the validated object lands in `structured_output`; as a fallback we try to
/// parse the plain-text `result`. If neither works it's an error, never ALLOW.
fn parse_reply(stdout: &[u8]) -> anyhow::Result<JudgeReply> {
    let envelope: serde_json::Value =
        serde_json::from_slice(stdout).context("judge output wasn't JSON")?;

    if envelope["is_error"].as_bool() == Some(true) {
        bail!("judge reported an error: {envelope}");
    }

    if let Ok(reply) = serde_json::from_value::<JudgeReply>(envelope["structured_output"].clone()) {
        return Ok(reply);
    }

    let result = envelope["result"].as_str().unwrap_or_default();
    serde_json::from_str::<JudgeReply>(result.trim())
        .with_context(|| format!("couldn't find a verdict in judge output: {result}"))
}
