//! Judge output types. Only one judge for now: `cli_judge` (claude -p).

pub mod cli_judge;

use crate::claude_request::PermissionRequest;
use serde::{Deserialize, Serialize};

/// Serialized as "ALLOW" / "ESCALATE" / "DENY" to match what we ask the model to write.
///
/// Only ALLOW is acted on automatically. ESCALATE and DENY both bring up your
/// normal permission prompt; they differ in what they tell you:
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Verdict {
    /// Verified safe and in scope.
    Allow,
    /// "I couldn't verify enough to decide": code it can't read, unclear intent,
    /// or a judgment call that's yours to make.
    Escalate,
    /// "I can see a concrete risk": destructive, exfiltrating, out of scope.
    Deny,
}

/// The judge's answer plus the event it was about. This is what the daemon
/// returns over HTTP and what gets appended to `decisions.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    pub verdict: Verdict,
    pub reason: String,
    pub event: PermissionRequest,
}
