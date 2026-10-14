//! The config file: defaults, partial files, typos, and the CLI around it.

use shephrd::config::{Config, EXAMPLE, PROMPT_FILE};
use shephrd::prompt::REVIEWER_TEMPLATE;
use std::io::Write;
use std::process::{Command, Stdio};

fn write_config(text: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(text.as_bytes()).unwrap();
    file
}

#[test]
fn missing_file_means_defaults() {
    let cfg = Config::from_file(std::path::Path::new("/definitely/not/here.toml")).unwrap();
    assert_eq!(cfg, Config::default());
}

#[test]
fn partial_file_keeps_other_defaults() {
    let file = write_config("model = \"haiku\"\n");
    let cfg = Config::from_file(file.path()).unwrap();
    assert_eq!(cfg.model, "haiku");
    assert_eq!(
        Config {
            model: "sonnet".into(),
            ..cfg
        },
        Config::default()
    );
}

#[test]
fn typo_is_an_error_not_a_silent_default() {
    let file = write_config("modle = \"haiku\"\n");
    let err = format!("{:#}", Config::from_file(file.path()).unwrap_err());
    assert!(
        err.contains("modle"),
        "error should name the bad key: {err}"
    );
}

#[test]
fn wrong_type_is_an_error() {
    let file = write_config("timeout_secs = \"ninety\"\n");
    assert!(Config::from_file(file.path()).is_err());
}

#[test]
fn example_file_documents_the_real_defaults() {
    // If someone changes a default in code but not in shephrd.example.toml
    // (or vice versa), this fails.
    let cfg: Config = toml::from_str(EXAMPLE).unwrap();
    assert_eq!(cfg, Config::default());
}

fn shephrd(args: &[&str], stdin: &str) -> (bool, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_shephrd"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.success(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn hook_with_broken_config_falls_back_silently() {
    let file = write_config("modle = \"haiku\"\n");
    let event = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/events/bash_gh_search.json"),
    )
    .unwrap();
    let path = file.path().to_str().unwrap();
    let (ok, stdout, stderr) = shephrd(&["hook", "--config", path], &event);
    assert!(ok, "hook must exit 0 even with a broken config");
    assert_eq!(stdout, "", "hook must not decide with a broken config");
    assert!(
        stderr.contains("modle"),
        "the reason should still be visible: {stderr}"
    );
}

#[test]
fn serve_with_broken_config_refuses_to_start() {
    let file = write_config("modle = \"haiku\"\n");
    let path = file.path().to_str().unwrap();
    let (ok, _, stderr) = shephrd(&["serve", "--config", path], "");
    assert!(!ok, "serve should fail loudly on a bad config");
    assert!(stderr.contains("modle"), "{stderr}");
}

#[test]
fn config_init_writes_the_example_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/config.toml");
    let p = path.to_str().unwrap();

    let (ok, stdout, _) = shephrd(&["config", "--init", "--config", p], "");
    assert!(ok);
    assert!(stdout.contains("model = \"sonnet\""), "{stdout}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), EXAMPLE);

    let prompt = dir.path().join("nested").join(PROMPT_FILE);
    assert_eq!(std::fs::read_to_string(&prompt).unwrap(), REVIEWER_TEMPLATE);

    let (ok, _, stderr) = shephrd(&["config", "--init", "--config", p], "");
    assert!(!ok, "a second --init must not overwrite your edits");
    assert!(stderr.contains("already exist"), "{stderr}");
}

#[test]
fn config_init_adds_a_missing_prompt_without_touching_the_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "model = \"haiku\"\n").unwrap();

    let (ok, _, stderr) = shephrd(&["config", "--init", "--config", path.to_str().unwrap()], "");
    assert!(ok, "{stderr}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "model = \"haiku\"\n");
    assert!(dir.path().join(PROMPT_FILE).exists());
}

/// A prompt with every placeholder, marked so tests can tell it apart.
fn custom_prompt() -> String {
    format!("CUSTOM-PROMPT\n{REVIEWER_TEMPLATE}")
}

#[test]
fn no_prompt_file_means_the_built_in_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let (cfg, _) = Config::load(Some(&path)).unwrap();
    assert_eq!(cfg.prompt_path(&path), None);
    assert_eq!(&*cfg.reviewer_template, REVIEWER_TEMPLATE);
}

#[test]
fn prompt_next_to_the_config_is_picked_up() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(dir.path().join(PROMPT_FILE), custom_prompt()).unwrap();
    let (cfg, _) = Config::load(Some(&path)).unwrap();
    assert!(cfg.reviewer_template.starts_with("CUSTOM-PROMPT"));
}

#[test]
fn prompt_setting_is_relative_to_the_config_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("prompts")).unwrap();
    std::fs::write(dir.path().join("prompts/strict.md"), custom_prompt()).unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "prompt = \"prompts/strict.md\"\n").unwrap();

    let (cfg, _) = Config::load(Some(&path)).unwrap();
    assert_eq!(cfg.prompt.as_deref(), Some(dir.path().join("prompts/strict.md").as_path()));
    assert!(cfg.reviewer_template.starts_with("CUSTOM-PROMPT"));
}

#[test]
fn prompt_missing_a_placeholder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let edited = REVIEWER_TEMPLATE.replace("{{user_turns}}", "");
    std::fs::write(dir.path().join(PROMPT_FILE), edited).unwrap();

    let err = format!("{:#}", Config::load(Some(&path)).unwrap_err());
    assert!(err.contains("{{user_turns}}"), "should name the placeholder: {err}");
}

#[test]
fn missing_prompt_file_is_an_error_not_a_silent_fallback() {
    let file = write_config("prompt = \"/definitely/not/here.md\"\n");
    assert!(Config::load(Some(file.path())).is_err());
}

#[test]
fn decision_log_defaults_to_the_state_dir_and_resolves_relative_paths() {
    let cfg = Config::default();
    assert!(
        cfg.decision_log_path().ends_with(".local/state/shephrd/decisions.jsonl")
            || std::env::var_os("XDG_STATE_HOME").is_some(),
        "{}",
        cfg.decision_log_path().display()
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "decision_log = \"logs/d.jsonl\"\n").unwrap();
    let cfg = Config::from_file(&path).unwrap();
    assert_eq!(cfg.decision_log_path(), dir.path().join("logs/d.jsonl"));
}
