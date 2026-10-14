//! The JSON we exchange with Claude Code. If these break, the hook silently
//! stops working, so they're pinned exactly.

use serde_json::{Value, json};
use shephrd::claude_request::{HookOutput, PermissionRequest};
use shephrd::client::hook_output;
use shephrd::judges::{Review, Verdict};
use std::path::Path;

#[test]
fn every_captured_event_parses() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/events");
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let raw = std::fs::read_to_string(&path).unwrap();
        let event: PermissionRequest = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("{} didn't parse: {e}", path.display()));
        assert_eq!(event.hook_event_name, "PermissionRequest");
        count += 1;
    }
    assert!(count > 0, "no fixtures found in {}", dir.display());
}

#[test]
fn allow_output_matches_claude_code_format() {
    let out = serde_json::to_value(HookOutput::allow()).unwrap();
    assert_eq!(
        out,
        json!({"hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": {"behavior": "allow"}
        }})
    );
}

#[test]
fn deny_output_carries_the_message() {
    let out: Value = serde_json::to_value(HookOutput::deny("nope".into())).unwrap();
    assert_eq!(
        out,
        json!({"hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": {"behavior": "deny", "message": "nope"}
        }})
    );
}

#[test]
fn ask_user_has_no_decision_only_a_message() {
    // No hookSpecificOutput at all: that's what makes Claude Code fall
    // through to your normal permission prompt.
    let out = serde_json::to_value(HookOutput::ask_user("look at this".into())).unwrap();
    assert_eq!(out, json!({"systemMessage": "look at this"}));
}

fn review(verdict: Verdict) -> Review {
    let raw = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/events/bash_gh_search.json"),
    )
    .unwrap();
    Review {
        verdict,
        reason: "REASON".into(),
        event: serde_json::from_str(&raw).unwrap(),
    }
}

#[test]
fn only_allow_decides_on_its_own() {
    let allow = serde_json::to_value(hook_output(&review(Verdict::Allow))).unwrap();
    assert_eq!(allow["hookSpecificOutput"]["decision"]["behavior"], "allow");

    for verdict in [Verdict::Escalate, Verdict::Deny] {
        let out = serde_json::to_value(hook_output(&review(verdict))).unwrap();
        assert!(
            out.get("hookSpecificOutput").is_none(),
            "{verdict:?} made a decision; it must leave it to the user: {out}"
        );
        let note = out["systemMessage"].as_str().unwrap();
        assert!(
            note.contains("REASON"),
            "{verdict:?} note lost the reason: {note}"
        );
    }
}

#[test]
fn escalate_and_deny_read_differently() {
    let note = |v| {
        serde_json::to_value(hook_output(&review(v))).unwrap()["systemMessage"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_ne!(note(Verdict::Escalate), note(Verdict::Deny));
}
