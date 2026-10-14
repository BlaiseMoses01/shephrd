use clap::{Parser, Subcommand};
use shephrd::claude_request::PermissionRequest;
use shephrd::config::{self, Config};
use shephrd::{client, prompt, server, settings};
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

/// shephrd: an LLM reviewer for Claude Code permission requests.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Config file to use instead of the default (see `shephrd config`).
    /// `global = true` lets it go before or after the subcommand.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the review daemon.
    Serve {
        /// Override the config's port for this run.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Hook client: read a PermissionRequest on stdin, ask the daemon, print the decision.
    Hook {
        /// Override the daemon URL (default: http://127.0.0.1:<config port>).
        #[arg(long)]
        url: Option<String>,
    },
    /// One-shot, no daemon: judge a PermissionRequest from stdin and print the review.
    Judge,
    /// Show which config and prompt files are used and the settings in effect.
    Config {
        /// Write the commented example config, and the reviewer prompt next to
        /// it as reviewer.md, if they don't exist yet.
        #[arg(long)]
        init: bool,
    },
    /// Turn shephrd on: add its hook to Claude Code's settings (~/.claude/settings.json).
    Enable {
        /// Use this project's .claude/settings.json instead.
        #[arg(long)]
        project: bool,
    },
    /// Turn shephrd off: remove its hook from Claude Code's settings.
    /// (To remove the program itself, use your package manager.)
    Disable {
        /// Use this project's .claude/settings.json instead.
        #[arg(long)]
        project: bool,
    },
    /// Check whether the daemon is running, and show the files it uses.
    Status,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let explicit = cli.config.as_deref();

    match cli.command {
        Command::Hook { url } => {
            // The hook must never fail loudly: a broken config means "no
            // verdict", which falls back to your normal permission prompt.
            let config = match Config::load(explicit) {
                Ok((config, _)) => config,
                Err(e) => {
                    eprintln!("shephrd: falling back to the permission dialog: {e:#}");
                    return Ok(());
                }
            };
            let url = url.unwrap_or_else(|| config.daemon_url());
            client::run(&url, config.timeout() + Duration::from_secs(10)).await;
        }
        Command::Serve { port } => {
            let (mut config, path) = Config::load(explicit)?;
            if let Some(port) = port {
                config.port = port;
            }
            eprintln!(
                "shephrd: config {} (model {}, timeout {}s)",
                describe(&path),
                config.model,
                config.timeout_secs
            );
            server::serve(config).await?;
        }
        Command::Judge => {
            let (config, _) = Config::load(explicit)?;
            let mut stdin = String::new();
            std::io::stdin().read_to_string(&mut stdin)?;
            let event: PermissionRequest = serde_json::from_str(&stdin)?;
            // One-shot has no session history, so there's nothing to pin against;
            // we still apply the same "not / or ~" check.
            let root = server::checked_root(&event.cwd)?;
            let review = server::judge_event(&event, &root, &config).await?;
            println!("{}", serde_json::to_string_pretty(&review)?);
        }
        Command::Config { init } => {
            let path = explicit
                .map(PathBuf::from)
                .unwrap_or_else(config::default_path);
            if init {
                // Write whichever of the two files is missing; never overwrite
                // either, so a second --init can't clobber your edits.
                let prompt_path = path.with_file_name(config::PROMPT_FILE);
                if path.exists() && prompt_path.exists() {
                    anyhow::bail!(
                        "{} and {} already exist; edit them directly",
                        path.display(),
                        prompt_path.display()
                    );
                }
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                for (file, text) in [
                    (&path, config::EXAMPLE),
                    (&prompt_path, prompt::REVIEWER_TEMPLATE),
                ] {
                    if !file.exists() {
                        std::fs::write(file, text)?;
                        println!("wrote {}", file.display());
                    }
                }
            }
            let (config, _) = Config::load(Some(&path))?;
            println!("# config: {}", describe(&path));
            println!("# prompt: {}", describe_prompt(&config, &path));
            println!("# log:    {}", config.decision_log_path().display());
            print!("{}", toml::to_string(&config)?);
        }
        Command::Enable { project } => {
            let (config, _) = Config::load(explicit)?;
            let exe = std::env::current_exe()?.canonicalize()?;
            let path = settings_path(project)?;
            let command = settings::hook_command(&exe);
            // The hook must outlast the judge, plus time for the hook itself.
            settings::enable(&path, &command, config.timeout_secs + 30)?;
            println!("enabled: added the shephrd hook to {}", path.display());
            println!("  command: {command}");
            println!("next: run `shephrd serve` and leave it running");
        }
        Command::Disable { project } => {
            let path = settings_path(project)?;
            match settings::disable(&path)? {
                0 => println!("no shephrd hook in {}", path.display()),
                n => println!("disabled: removed {n} shephrd hook(s) from {}", path.display()),
            }
        }
        Command::Status => {
            let (config, path) = Config::load(explicit)?;
            println!("config: {}", describe(&path));
            println!("prompt: {}", describe_prompt(&config, &path));
            println!("log:    {}", config.decision_log_path().display());
            let health = reqwest::Client::new()
                .get(format!("{}/health", config.daemon_url()))
                .timeout(Duration::from_secs(2))
                .send()
                .await
                .and_then(|r| r.error_for_status());
            match health {
                Ok(_) => println!("daemon: running at {}", config.daemon_url()),
                Err(_) => {
                    println!(
                        "daemon: not running at {} (start it with `shephrd serve`; \
                         until then every request falls back to your normal prompt)",
                        config.daemon_url()
                    );
                    std::process::exit(1);
                }
            }
        }
    }
    Ok(())
}

fn settings_path(project: bool) -> anyhow::Result<PathBuf> {
    Ok(if project {
        std::env::current_dir()?.join(".claude/settings.json")
    } else {
        settings::user_settings_path()
    })
}

fn describe_prompt(config: &Config, config_path: &std::path::Path) -> String {
    match config.prompt_path(config_path) {
        Some(p) => p.display().to_string(),
        None => "built in (run `shephrd config --init` to get an editable copy)".into(),
    }
}

fn describe(path: &std::path::Path) -> String {
    if path.exists() {
        path.display().to_string()
    } else {
        format!("{} (not found, using defaults)", path.display())
    }
}
