//! The shared reviewer prompt and its `{{name}}` renderer.

use shephrd::claude_request::PermissionRequest;
use shephrd::prompt::{REVIEWER_TEMPLATE, render, reviewer_prompt};
use shephrd::transcript::SessionContext;
use std::path::PathBuf;

fn event(command: &str) -> PermissionRequest {
    PermissionRequest {
        session_id: "s".into(),
        transcript_path: PathBuf::from("/nonexistent.jsonl"),
        cwd: PathBuf::from("/work/proj"),
        permission_mode: Some("default".into()),
        hook_event_name: "PermissionRequest".into(),
        tool_name: "Bash".into(),
        tool_input: serde_json::json!({"command": command}),
    }
}

fn ctx() -> SessionContext {
    SessionContext {
        user_turns: vec!["TRUSTED-TURN".into()],
        plan: Some("TRUSTED-PLAN".into()),
    }
}

#[test]
fn render_fills_known_and_keeps_unknown() {
    assert_eq!(render("a {{x}} b {{y}}", &[("x", "1")]), "a 1 b {{y}}");
}

#[test]
fn render_handles_unclosed_braces() {
    assert_eq!(render("a {{x}} b {{oops", &[("x", "1")]), "a 1 b {{oops");
}

#[test]
fn render_is_single_pass() {
    // A value that looks like a placeholder must come out literally.
    assert_eq!(
        render("{{a}} {{b}}", &[("a", "{{b}}"), ("b", "B")]),
        "{{b}} B"
    );
}

#[test]
fn template_has_no_unfilled_placeholders() {
    let prompt = reviewer_prompt(REVIEWER_TEMPLATE, &event("ls"), &ctx(), "ACCESS-LINE");
    assert!(
        !prompt.contains("{{"),
        "unfilled placeholder in prompts/reviewer.md:\n{prompt}"
    );
    for expected in [
        "TRUSTED-TURN",
        "TRUSTED-PLAN",
        "ACCESS-LINE",
        "/work/proj",
        "Bash",
    ] {
        assert!(prompt.contains(expected), "missing {expected}");
    }
}

#[test]
fn agent_text_cannot_smuggle_in_trusted_context() {
    // The agent controls tool_input. If it writes a placeholder name, that
    // must stay literal and stay inside <request>, not pull trusted text in.
    let prompt = reviewer_prompt(
        REVIEWER_TEMPLATE,
        &event("echo {{user_turns}} {{plan}}"),
        &ctx(),
        "x",
    );
    let request = &prompt[prompt.find("<request>").unwrap()..prompt.find("</request>").unwrap()];
    assert!(request.contains("{{user_turns}} {{plan}}"));
    assert!(!request.contains("TRUSTED-"));
}

#[test]
fn trusted_context_stays_in_its_block() {
    let prompt = reviewer_prompt(REVIEWER_TEMPLATE, &event("ls"), &ctx(), "x");
    let trusted = &prompt
        [prompt.find("<trusted_intent>").unwrap()..prompt.find("</trusted_intent>").unwrap()];
    assert!(trusted.contains("TRUSTED-TURN") && trusted.contains("TRUSTED-PLAN"));
}

#[test]
fn template_is_the_file_on_disk() {
    // include_str! is baked in at compile time; this catches a stale build.
    let on_disk = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("prompts/reviewer.md"),
    )
    .unwrap();
    assert_eq!(REVIEWER_TEMPLATE, on_disk);
}
