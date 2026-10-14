//! `shephrd enable` / `shephrd disable`: add or remove our hook in a Claude
//! Code `settings.json`, leaving every other setting and hook alone. This only
//! switches the hook; installing the binary is the package manager's job.

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

/// The tools whose permission prompts shephrd reviews.
pub const MATCHER: &str = "Bash|Edit|Write|WebFetch";

/// `~/.claude/settings.json`, or under `$CLAUDE_CONFIG_DIR` if you've moved
/// Claude Code's config.
pub fn user_settings_path() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".claude")
        })
        .join("settings.json")
}

/// The hook command for the binary at `exe`, quoted if the path has spaces.
pub fn hook_command(exe: &Path) -> String {
    let exe = exe.display().to_string();
    if exe.contains(' ') {
        format!("\"{exe}\" hook")
    } else {
        format!("{exe} hook")
    }
}

/// Is this a hook command we wrote? Matches any install location, so
/// enabling from a new path replaces the old entry instead of adding one.
fn is_ours(command: &str) -> bool {
    let command = command.trim();
    let Some(exe) = command.strip_suffix(" hook") else {
        return false;
    };
    Path::new(exe.trim_matches('"'))
        .file_name()
        .is_some_and(|name| name == "shephrd")
}

/// Remove our hook entries from parsed settings. Returns how many were removed.
/// Matcher groups left with no hooks are dropped, and so are `PermissionRequest`
/// and `hooks` if they end up empty.
pub fn remove_hook(settings: &mut Value) -> usize {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return 0;
    };
    let Some(groups) = hooks
        .get_mut("PermissionRequest")
        .and_then(Value::as_array_mut)
    else {
        return 0;
    };

    let mut removed = 0;
    for group in groups.iter_mut() {
        if let Some(list) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            let before = list.len();
            list.retain(|h| !h["command"].as_str().is_some_and(is_ours));
            removed += before - list.len();
        }
    }
    groups.retain(|g| !g["hooks"].as_array().is_some_and(Vec::is_empty));

    if groups.is_empty() {
        hooks.remove("PermissionRequest");
    }
    if hooks.is_empty() {
        settings.as_object_mut().map(|s| s.remove("hooks"));
    }
    removed
}

/// Add our hook to parsed settings, replacing any earlier copy of it.
pub fn add_hook(settings: &mut Value, command: &str, timeout_secs: u64) -> anyhow::Result<()> {
    remove_hook(settings);
    let root = settings
        .as_object_mut()
        .context("settings.json isn't a JSON object")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("\"hooks\" in settings.json isn't an object")?;
    let groups = hooks
        .entry("PermissionRequest")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .context("\"hooks.PermissionRequest\" in settings.json isn't a list")?;
    groups.push(json!({
        "matcher": MATCHER,
        "hooks": [{
            "type": "command",
            "command": command,
            "timeout": timeout_secs,
        }]
    }));
    Ok(())
}

fn read_settings(path: &Path) -> anyhow::Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(json!({})),
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("{} isn't valid JSON; fix it first", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e).with_context(|| format!("couldn't read {}", path.display())),
    }
}

/// Write the settings back, after copying the old file to `settings.json.bak`.
fn write_settings(path: &Path, settings: &Value) -> anyhow::Result<()> {
    if path.exists() {
        let backup = path.with_extension("json.bak");
        std::fs::copy(path, &backup)
            .with_context(|| format!("couldn't back up to {}", backup.display()))?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(settings)? + "\n";
    std::fs::write(path, text).with_context(|| format!("couldn't write {}", path.display()))
}

/// Add (or update) our hook in the settings file at `path`.
pub fn enable(path: &Path, command: &str, timeout_secs: u64) -> anyhow::Result<()> {
    let mut settings = read_settings(path)?;
    if !settings.is_object() {
        bail!("{} isn't a JSON object", path.display());
    }
    add_hook(&mut settings, command, timeout_secs)?;
    write_settings(path, &settings)
}

/// Remove our hook from the settings file at `path`. Returns how many entries
/// were removed; the file is only rewritten if that's more than zero.
pub fn disable(path: &Path) -> anyhow::Result<usize> {
    let mut settings = read_settings(path)?;
    let removed = remove_hook(&mut settings);
    if removed > 0 {
        write_settings(path, &settings)?;
    }
    Ok(removed)
}
