//! The hook contract: what Claude Code sends us on stdin, and what we print back.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A `PermissionRequest` hook event, as captured in `.claude/captures/`.
///
/// Only the fields we actually use are required. Everything else is
/// `#[serde(default)]` so a new Claude Code version adding or dropping a field
/// doesn't break parsing. `Serialize` is derived too so we can echo the event
/// back in the `Review` and the decision log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub session_id: String,
    pub transcript_path: PathBuf,
    pub cwd: PathBuf,
    #[serde(default)]
    pub permission_mode: Option<String>,
    pub hook_event_name: String,
    pub tool_name: String,
    /// Shape depends on the tool (Bash has `command`, Edit has `file_path`...),
    /// so we keep it as untyped JSON.
    #[serde(default)]
    pub tool_input: serde_json::Value,
}

/// What we print to stdout. Two shapes:
///
/// A decision, which settles the request without asking you:
/// ```json
/// {"hookSpecificOutput":{"hookEventName":"PermissionRequest",
///   "decision":{"behavior":"allow"}}}
/// ```
///
/// No decision, just a message, so Claude Code shows you its normal
/// permission prompt with our note alongside:
/// ```json
/// {"systemMessage":"shephrd flagged a risk: ..."}
/// ```
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookOutput {
    /// `skip_serializing_if` leaves the key out entirely when it's `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<HookSpecificOutput>,
    /// Shown to you (the user), not to the agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookSpecificOutput {
    pub hook_event_name: &'static str,
    pub decision: Decision,
}

#[derive(Debug, Serialize)]
pub struct Decision {
    /// "allow" or "deny".
    pub behavior: &'static str,
    /// Deny only: shown to the agent, so it knows why and can adjust.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl HookOutput {
    /// Approve without asking you.
    pub fn allow() -> Self {
        Self::decide("allow", None)
    }

    /// Reject without asking you; `message` goes to the agent.
    /// Unused while every non-ALLOW goes to you, but kept for a future hard-deny tier.
    pub fn deny(message: String) -> Self {
        Self::decide("deny", Some(message))
    }

    /// Make no decision: Claude Code shows you its normal permission prompt,
    /// and `note` is displayed to you so you know why it's asking.
    pub fn ask_user(note: String) -> Self {
        HookOutput {
            hook_specific_output: None,
            system_message: Some(note),
        }
    }

    fn decide(behavior: &'static str, message: Option<String>) -> Self {
        HookOutput {
            hook_specific_output: Some(HookSpecificOutput {
                hook_event_name: "PermissionRequest",
                decision: Decision { behavior, message },
            }),
            system_message: None,
        }
    }
}
