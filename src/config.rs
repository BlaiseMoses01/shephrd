//! User settings, read from a TOML file.
//!
//! Where it looks, first match wins:
//!   1. `--config <path>` on the command line
//!   2. `$SHEPHRD_CONFIG`
//!   3. `$XDG_CONFIG_HOME/shephrd/config.toml`, else `~/.config/shephrd/config.toml`
//!
//! A missing file is fine: everything has a default. A file that exists but
//! doesn't parse is an error, so a typo never silently falls back to defaults.

use crate::prompt;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The commented example, embedded so `shephrd config --init` can write it out.
pub const EXAMPLE: &str = include_str!("../shephrd.example.toml");

/// The prompt file `shephrd config --init` writes, and that's picked up from
/// the config directory without any setting.
pub const PROMPT_FILE: &str = "reviewer.md";

/// `#[serde(default)]` fills any field missing from the file from `Default`,
/// so a config can set just `model` and nothing else.
/// `deny_unknown_fields` turns a typo like `modle = "haiku"` into an error
/// instead of a setting that silently does nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Passed to `claude -p --model`.
    pub model: String,
    /// How long a judge run may take before it's killed.
    pub timeout_secs: u64,
    /// How many recent user prompts the judge sees.
    pub user_turns: usize,
    /// Daemon port on 127.0.0.1; the hook client connects here too.
    pub port: u16,
    /// How many judges may run at once; more requests wait their turn.
    pub max_concurrent: usize,
    /// Append-only verdict log. Unset means `default_log_path()`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_log: Option<PathBuf>,
    /// Reviewer prompt file. Unset means `reviewer.md` next to the config if
    /// it exists, else the prompt built into the binary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<PathBuf>,
    /// The reviewer prompt text in effect. Not a TOML setting: `load` fills it
    /// from the prompt file, and it's the built-in prompt otherwise.
    #[serde(skip)]
    pub reviewer_template: Arc<str>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            model: "sonnet".into(),
            timeout_secs: 90,
            user_turns: 5,
            port: 7878,
            max_concurrent: 4,
            decision_log: None,
            prompt: None,
            reviewer_template: prompt::REVIEWER_TEMPLATE.into(),
        }
    }
}

impl Config {
    /// Find and load the config and the reviewer prompt it points at. Returns
    /// the config path it used, so callers can show it.
    pub fn load(explicit: Option<&Path>) -> anyhow::Result<(Config, PathBuf)> {
        let path = match explicit {
            Some(p) => p.to_path_buf(),
            None => default_path(),
        };
        let mut config = Self::from_file(&path)?;
        if let Some(prompt_path) = config.prompt_path(&path) {
            config.reviewer_template = prompt::load_template(&prompt_path)?.into();
        }
        Ok((config, path))
    }

    /// Defaults if the file doesn't exist; an error if it exists but is invalid.
    /// Relative paths in the file are made relative to the file's directory,
    /// so they don't depend on where you start shephrd from.
    pub fn from_file(path: &Path) -> anyhow::Result<Config> {
        let mut config: Config = match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("invalid config in {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(e).with_context(|| format!("couldn't read {}", path.display())),
        };
        let dir = path.parent().unwrap_or(Path::new(""));
        for p in [&mut config.decision_log, &mut config.prompt]
            .into_iter()
            .flatten()
        {
            if p.is_relative() {
                *p = dir.join(&*p);
            }
        }
        Ok(config)
    }

    /// The prompt file in effect for a config loaded from `config_path`, or
    /// `None` for the built-in prompt.
    pub fn prompt_path(&self, config_path: &Path) -> Option<PathBuf> {
        if let Some(p) = &self.prompt {
            return Some(p.clone());
        }
        let beside = config_path.parent()?.join(PROMPT_FILE);
        beside.exists().then_some(beside)
    }

    pub fn decision_log_path(&self) -> PathBuf {
        self.decision_log.clone().unwrap_or_else(default_log_path)
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    pub fn daemon_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// `$SHEPHRD_CONFIG`, else the XDG-style path. We use ~/.config on macOS too
/// (rather than ~/Library/Application Support) because it's easier to find and edit.
pub fn default_path() -> PathBuf {
    if let Some(p) = std::env::var_os("SHEPHRD_CONFIG") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"));
    base.join("shephrd/config.toml")
}

/// `$XDG_STATE_HOME/shephrd/decisions.jsonl`, else `~/.local/state/shephrd/decisions.jsonl`.
pub fn default_log_path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/state"));
    base.join("shephrd/decisions.jsonl")
}
