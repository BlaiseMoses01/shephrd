//! What counts as trusted context. See tests/fixtures/transcripts/mixed_turns.jsonl;
//! each line there exercises one rule.

use shephrd::transcript::read_transcript;
use std::path::Path;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transcripts/mixed_turns.jsonl")
}

#[test]
fn keeps_only_typed_prompts() {
    let ctx = read_transcript(&fixture(), 10);
    assert_eq!(
        ctx.user_turns,
        vec![
            "first prompt as a plain string",
            "second prompt as text blocks", // the inline <system-reminder> block is dropped
            "third prompt",
            // the half-written last line is skipped, not an error
        ]
    );
}

#[test]
fn tool_results_never_become_user_turns() {
    let ctx = read_transcript(&fixture(), 10);
    assert!(
        ctx.user_turns
            .iter()
            .all(|t| !t.contains("IGNORE PREVIOUS")),
        "agent-controlled tool output leaked into trusted context: {:?}",
        ctx.user_turns
    );
}

#[test]
fn keep_last_trims_from_the_front() {
    let ctx = read_transcript(&fixture(), 1);
    assert_eq!(ctx.user_turns, vec!["third prompt"]);
}

#[test]
fn takes_the_latest_plan() {
    let ctx = read_transcript(&fixture(), 10);
    assert_eq!(ctx.plan.as_deref(), Some("latest plan"));
}

#[test]
fn missing_transcript_is_empty_not_an_error() {
    let ctx = read_transcript(Path::new("/definitely/not/here.jsonl"), 5);
    assert!(ctx.user_turns.is_empty());
    assert!(ctx.plan.is_none());
}
