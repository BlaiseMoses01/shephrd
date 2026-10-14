//! Pull the *trusted* context out of a session transcript (JSONL, one event per line).
//!
//! Trusted means things you wrote or approved: your typed prompts and the plan
//! you accepted. Tool output and assistant text are written by the agent (or the
//! repo), so we leave them out and let the judge inspect files itself instead.

use serde_json::Value;
use std::path::Path;

#[derive(Debug, Default)]
pub struct SessionContext {
    /// Your most recent typed prompts, oldest first.
    pub user_turns: Vec<String>,
    /// The last plan the agent submitted via ExitPlanMode.
    /// POC caveat: we don't yet check whether you approved or rejected it.
    pub plan: Option<String>,
}

/// Read the transcript and keep the last `keep_last` user turns.
///
/// Never fails: a missing or unreadable transcript just means no context.
/// The judge can still decide from the request alone.
pub fn read_transcript(path: &Path, keep_last: usize) -> SessionContext {
    let mut ctx = SessionContext::default();

    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) => {
            eprintln!("shephrd: couldn't read transcript {}: {e}", path.display());
            return ctx;
        }
    };

    for line in raw.lines() {
        // The file is still being written, so the last line may be half-done.
        // `let ... else` skips anything that doesn't parse.
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["isMeta"].as_bool().unwrap_or(false) {
            continue;
        }

        match v["type"].as_str() {
            Some("user") => {
                if let Some(text) = typed_text(&v["message"]["content"]) {
                    ctx.user_turns.push(text);
                }
            }
            Some("assistant") => {
                // `.into_iter().flatten()` turns Option<&Vec> into an iterator
                // over the Vec's items (or nothing if it's None).
                for block in v["message"]["content"].as_array().into_iter().flatten() {
                    // `&& let` (a "let chain", Rust 2024) matches and binds in one `if`.
                    if block["type"] == "tool_use"
                        && block["name"] == "ExitPlanMode"
                        && let Some(plan) = block["input"]["plan"].as_str()
                    {
                        ctx.plan = Some(plan.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    let n = ctx.user_turns.len();
    ctx.user_turns.drain(..n.saturating_sub(keep_last));
    ctx
}

/// A real typed prompt: either a plain string, or text blocks with no tool_result.
fn typed_text(content: &Value) -> Option<String> {
    if let Some(s) = content.as_str() {
        let is_noise = s.starts_with("<command-")
            || s.starts_with("<system-reminder")
            || s.starts_with("<local-command");
        return (!is_noise).then(|| s.to_string());
    }

    let blocks = content.as_array()?;
    // Tool results come back as "user" messages too; they're not you typing.
    if blocks.iter().any(|b| b["type"] == "tool_result") {
        return None;
    }
    let text: Vec<&str> = blocks
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .filter(|t| !t.starts_with("<system-reminder"))
        .collect();
    (!text.is_empty()).then(|| text.join("\n"))
}
