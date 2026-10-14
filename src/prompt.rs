//! The reviewer prompt, shared by every judge.
//!
//! The text lives in `prompts/reviewer.md` so it can be read and edited as
//! prose. `include_str!` embeds it into the binary at compile time, so shephrd
//! works with no files installed. Users can override it with their own copy
//! (see `Config::prompt_path`), which `load_template` reads and checks.

use crate::claude_request::PermissionRequest;
use crate::transcript::SessionContext;
use anyhow::{Context, bail};
use std::path::Path;

pub const REVIEWER_TEMPLATE: &str = include_str!("../prompts/reviewer.md");

/// Every placeholder `reviewer_prompt` fills. A custom prompt must keep all of
/// them: dropping one would quietly hide that context from the judge.
pub const PLACEHOLDERS: &[&str] = &[
    "user_turns",
    "plan",
    "tool",
    "tool_input",
    "cwd",
    "mode",
    "tool_access",
];

/// Read a custom prompt file and refuse it if a placeholder is missing.
pub fn load_template(path: &Path) -> anyhow::Result<String> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("couldn't read prompt {}", path.display()))?;
    let missing: Vec<_> = PLACEHOLDERS
        .iter()
        .filter(|name| !text.contains(&format!("{{{{{name}}}}}")))
        .map(|name| format!("{{{{{name}}}}}"))
        .collect();
    if !missing.is_empty() {
        bail!(
            "prompt {} is missing {}; every placeholder must stay in the prompt",
            path.display(),
            missing.join(", ")
        );
    }
    Ok(text)
}

/// Fill a reviewer template (the built-in one or a user's copy) for one request.
///
/// `tool_access` is the one judge-specific line: what the judge can do to
/// investigate (e.g. "You may use Read, Grep and Glob..." for the CLI judge,
/// or "You cannot open files; judge from the request alone." for one without tools).
pub fn reviewer_prompt(
    template: &str,
    event: &PermissionRequest,
    ctx: &SessionContext,
    tool_access: &str,
) -> String {
    let user_turns = if ctx.user_turns.is_empty() {
        "(none found)".to_string()
    } else {
        ctx.user_turns
            .iter()
            .map(|t| format!("- {t}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let tool_input = serde_json::to_string_pretty(&event.tool_input).unwrap_or_default();
    let cwd = event.cwd.display().to_string();

    render(
        template,
        &[
            ("user_turns", &user_turns),
            ("plan", ctx.plan.as_deref().unwrap_or("(none)")),
            ("tool", &event.tool_name),
            ("tool_input", &tool_input),
            ("cwd", &cwd),
            (
                "mode",
                event.permission_mode.as_deref().unwrap_or("unknown"),
            ),
            ("tool_access", tool_access),
        ],
    )
}

/// Replace each `{{name}}` in `template` with its value, in a single pass.
///
/// Single pass matters: values are inserted into the output and never scanned
/// again. With chained `.replace()` calls, an agent could put the text
/// `{{user_turns}}` in its command and have a later replace splice *trusted*
/// text into the untrusted section (or vice versa). Unknown placeholders are
/// left as-is; the tests check the real template has none.
pub fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    // Find the next "{{", copy everything before it, then look for its "}}".
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            // Unclosed "{{": keep it and everything after, copied below.
            rest = &rest[start..];
            break;
        };
        let name = &after[..end];
        match vars.iter().find(|(k, _)| *k == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                out.push_str("{{");
                out.push_str(name);
                out.push_str("}}");
            }
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}
