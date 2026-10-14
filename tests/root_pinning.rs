//! The judge may only read under a session's root. These pin the daemon-side
//! rules; the CLI-side read refusal is covered by the golden set's canaries.

use shephrd::claude_request::PermissionRequest;
use shephrd::server::{AppState, checked_root, pin_root};
use std::path::{Path, PathBuf};

fn event(session: &str, cwd: &Path) -> PermissionRequest {
    PermissionRequest {
        session_id: session.into(),
        transcript_path: PathBuf::from("/nonexistent.jsonl"),
        cwd: cwd.to_path_buf(),
        permission_mode: None,
        hook_event_name: "PermissionRequest".into(),
        tool_name: "Bash".into(),
        tool_input: serde_json::json!({"command": "ls"}),
    }
}

/// tempdir/proj/sub, returned canonicalized (on macOS /var -> /private/var).
fn layout() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("proj/sub")).unwrap();
    let proj = tmp.path().join("proj").canonicalize().unwrap();
    (tmp, proj)
}

#[test]
fn first_cwd_becomes_the_root() {
    let (_tmp, proj) = layout();
    let state = AppState::default();
    assert_eq!(pin_root(&state, &event("s", &proj)).unwrap(), proj);
}

#[test]
fn deeper_cwd_keeps_the_original_root() {
    let (_tmp, proj) = layout();
    let state = AppState::default();
    pin_root(&state, &event("s", &proj)).unwrap();
    assert_eq!(
        pin_root(&state, &event("s", &proj.join("sub"))).unwrap(),
        proj
    );
}

#[test]
fn cwd_above_the_root_is_refused() {
    let (_tmp, proj) = layout();
    let state = AppState::default();
    pin_root(&state, &event("s", &proj)).unwrap();
    assert!(pin_root(&state, &event("s", proj.parent().unwrap())).is_err());
}

#[test]
fn sibling_with_shared_prefix_is_refused() {
    // proj vs proj-evil: a string prefix check would wrongly accept this.
    let (tmp, proj) = layout();
    std::fs::create_dir(tmp.path().join("proj-evil")).unwrap();
    let state = AppState::default();
    pin_root(&state, &event("s", &proj)).unwrap();
    assert!(pin_root(&state, &event("s", &tmp.path().join("proj-evil"))).is_err());
}

#[test]
fn symlink_cwd_resolves_before_the_check() {
    let (tmp, proj) = layout();
    std::os::unix::fs::symlink(tmp.path(), proj.join("escape")).unwrap();
    let state = AppState::default();
    pin_root(&state, &event("s", &proj)).unwrap();
    assert!(pin_root(&state, &event("s", &proj.join("escape"))).is_err());
}

#[test]
fn sessions_are_pinned_independently() {
    let (tmp, proj) = layout();
    let state = AppState::default();
    pin_root(&state, &event("a", &proj)).unwrap();
    let other = tmp.path().canonicalize().unwrap();
    assert_eq!(pin_root(&state, &event("b", &other)).unwrap(), other);
}

#[test]
fn home_and_filesystem_root_are_refused() {
    assert!(checked_root(Path::new("/")).is_err());
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    assert!(checked_root(&home).is_err());
}

#[test]
fn missing_cwd_is_refused() {
    assert!(checked_root(Path::new("/definitely/not/here")).is_err());
}
