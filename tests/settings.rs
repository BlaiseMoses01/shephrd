//! `shephrd enable` / `disable`: our hook goes in and out of settings.json
//! without disturbing anything else there.

use serde_json::{Value, json};
use shephrd::settings::{add_hook, disable, enable, hook_command, remove_hook, stable_path_for};
use std::path::{Path, PathBuf};

const CMD: &str = "/usr/local/bin/shephrd hook";

/// Settings with other keys and another PermissionRequest hook we must keep.
fn existing() -> Value {
    json!({
        "model": "opus",
        "hooks": {
            "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "audit"}]}],
            "PermissionRequest": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "other-tool"}]}]
        }
    })
}

fn ours(settings: &Value) -> Vec<Value> {
    settings["hooks"]["PermissionRequest"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|g| g["hooks"].as_array().unwrap().clone())
        .filter(|h| h["command"].as_str().unwrap().ends_with("shephrd hook"))
        .collect()
}

#[test]
fn add_keeps_everything_else() {
    let mut s = existing();
    add_hook(&mut s, CMD, 120).unwrap();
    assert_eq!(s["model"], "opus");
    assert_eq!(s["hooks"]["PreToolUse"], existing()["hooks"]["PreToolUse"]);
    assert_eq!(s["hooks"]["PermissionRequest"].as_array().unwrap().len(), 2);
    assert_eq!(
        ours(&s),
        vec![json!({"type": "command", "command": CMD, "timeout": 120})]
    );
}

#[test]
fn adding_twice_or_from_a_new_path_doesnt_duplicate() {
    let mut s = existing();
    add_hook(&mut s, "/old/target/debug/shephrd hook", 60).unwrap();
    add_hook(&mut s, CMD, 120).unwrap();
    add_hook(&mut s, CMD, 120).unwrap();
    assert_eq!(ours(&s).len(), 1);
    assert_eq!(ours(&s)[0]["command"], CMD);
}

#[test]
fn remove_leaves_other_hooks_and_cleans_up_empty_keys() {
    let mut s = existing();
    add_hook(&mut s, CMD, 120).unwrap();
    assert_eq!(remove_hook(&mut s), 1);
    assert_eq!(s, existing());

    let mut only_ours = json!({});
    add_hook(&mut only_ours, CMD, 120).unwrap();
    remove_hook(&mut only_ours);
    assert_eq!(
        only_ours,
        json!({}),
        "no empty hooks/PermissionRequest left behind"
    );
}

#[test]
fn similar_names_are_not_ours() {
    let mut s = json!({"hooks": {"PermissionRequest": [{"hooks": [
        {"type": "command", "command": "/bin/shephrd-legacy hook"},
        {"type": "command", "command": "shephrd judge"}
    ]}]}});
    assert_eq!(remove_hook(&mut s), 0);
}

#[test]
fn paths_with_spaces_are_quoted_and_still_recognized() {
    let cmd = hook_command(Path::new("/Users/me/My Tools/shephrd"));
    assert_eq!(cmd, "\"/Users/me/My Tools/shephrd\" hook");
    let mut s = json!({});
    add_hook(&mut s, &cmd, 120).unwrap();
    assert_eq!(remove_hook(&mut s), 1);
}

#[test]
fn enable_and_disable_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".claude/settings.json");

    // Creates the file (and directory) when there's none.
    enable(&path, CMD, 120).unwrap();
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(ours(&written).len(), 1);

    // An existing file is backed up before it's changed.
    std::fs::write(&path, serde_json::to_string(&existing()).unwrap()).unwrap();
    enable(&path, CMD, 120).unwrap();
    let backup: Value =
        serde_json::from_str(&std::fs::read_to_string(path.with_extension("json.bak")).unwrap())
            .unwrap();
    assert_eq!(backup, existing());

    assert_eq!(disable(&path).unwrap(), 1);
    let after: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(after, existing());
    assert_eq!(disable(&path).unwrap(), 0);
}

#[test]
fn invalid_settings_are_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path, "{ not json").unwrap();
    assert!(enable(&path, CMD, 120).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
}

#[cfg(unix)]
#[test]
fn hook_uses_the_path_symlink_not_the_versioned_file() {
    // Homebrew layout: bin/shephrd -> Cellar/shephrd/0.1.0/bin/shephrd.
    let tmp = tempfile::tempdir().unwrap();
    let real_dir = tmp.path().join("Cellar/shephrd/0.1.0/bin");
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&real_dir).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    let real = real_dir.join("shephrd");
    std::fs::write(&real, "").unwrap();
    std::os::unix::fs::symlink(&real, bin.join("shephrd")).unwrap();
    let real = real.canonicalize().unwrap();

    let elsewhere = tmp.path().join("other");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("shephrd"), "").unwrap(); // a different shephrd

    let got = stable_path_for(&real, [elsewhere, bin.clone()]);
    assert_eq!(got, bin.join("shephrd"));
}

#[test]
fn falls_back_to_the_real_path_when_not_on_path() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("shephrd");
    std::fs::write(&real, "").unwrap();
    let real = real.canonicalize().unwrap();
    assert_eq!(
        stable_path_for(&real, [PathBuf::from("/nonexistent")]),
        real
    );
}
