//! `shephrd serve`: the daemon. `POST /review` judges a request; `GET /health`
//! says it's up. One daemon serves every Claude Code session on the machine.

use crate::claude_request::PermissionRequest;
use crate::config::Config;
use crate::judges::{Review, cli_judge};
use crate::transcript::read_transcript;
use anyhow::{Context, bail};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::Semaphore;

/// Tools that are really questions for *you*, not permission checks.
/// The judge must never answer these on your behalf.
const USER_ONLY_TOOLS: &[&str] = &["AskUserQuestion", "ExitPlanMode"];

/// State shared by every request. axum clones it per request, so the map sits
/// behind `Arc` (shared ownership) + `Mutex` (one writer at a time).
#[derive(Clone)]
pub struct AppState {
    /// Read-only after startup, so `Arc` alone is enough (no Mutex).
    config: Arc<Config>,
    /// session_id -> the first cwd we saw for that session (canonicalized).
    roots: Arc<Mutex<HashMap<String, PathBuf>>>,
    /// One permit per judge allowed to run at once (`max_concurrent`), so
    /// many busy sessions can't start unlimited `claude -p` processes.
    judges: Arc<Semaphore>,
    /// Held while appending to the decision log, so lines never interleave.
    log: Arc<Mutex<()>>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        AppState {
            // At least one permit, or every request would wait forever.
            judges: Arc::new(Semaphore::new(config.max_concurrent.max(1))),
            config: Arc::new(config),
            roots: Arc::default(),
            log: Arc::default(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        AppState::new(Config::default())
    }
}

/// The router on its own, so tests can serve it on a random port.
pub fn app(config: Config) -> Router {
    Router::new()
        .route("/review", post(review))
        .route("/health", get(health))
        .with_state(AppState::new(config))
}

/// `shephrd status` calls this to check the daemon is up.
async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "model": state.config.model,
    }))
}

pub async fn serve(config: Config) -> anyhow::Result<()> {
    // 127.0.0.1 only: nothing off this machine should be able to ask for approvals.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", config.port)).await?;
    eprintln!("shephrd: listening on {}", config.daemon_url());
    axum::serve(listener, app(config)).await?;
    Ok(())
}

/// axum handler. Extractors in the arguments do the parsing: `State` hands us
/// the shared state, `Json(event)` parses the body (axum replies 4xx itself if
/// it can't). Returning `impl IntoResponse` lets each branch return a
/// different response type.
async fn review(
    State(state): State<AppState>,
    Json(event): Json<PermissionRequest>,
) -> impl IntoResponse {
    if USER_ONLY_TOOLS.contains(&event.tool_name.as_str()) {
        eprintln!("shephrd: skipping {} (user decides)", event.tool_name);
        return StatusCode::NO_CONTENT.into_response();
    }

    // No trustworthy root means no judge: 204 sends it back to your dialog.
    let root = match pin_root(&state, &event) {
        Ok(root) => root,
        Err(e) => {
            eprintln!("shephrd: not judging, {e:#}");
            return StatusCode::NO_CONTENT.into_response();
        }
    };

    eprintln!(
        "shephrd: reviewing {} in {}",
        event.tool_name,
        root.display()
    );
    // Wait for a free judge slot. `acquire` only fails if the semaphore is
    // closed, which we never do. The permit is released when it's dropped.
    let _permit = state
        .judges
        .acquire()
        .await
        .expect("semaphore never closed");
    match judge_event(&event, &root, &state.config).await {
        Ok(review) => {
            eprintln!("shephrd: {:?}: {}", review.verdict, review.reason);
            let log = state.config.decision_log_path();
            let written = {
                let _guard = state.log.lock().unwrap();
                log_review(&log, &review)
            };
            if let Err(e) = written {
                eprintln!("shephrd: couldn't write {}: {e:#}", log.display());
            }
            Json(review).into_response()
        }
        Err(e) => {
            eprintln!("shephrd: judge failed: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response()
        }
    }
}

/// The first cwd we see for a session becomes its root for good. Later events
/// must have a cwd at or under it, so an agent that `cd`s to `~` can't widen
/// what the judge is allowed to read.
pub fn pin_root(state: &AppState, event: &PermissionRequest) -> anyhow::Result<PathBuf> {
    let cwd = checked_root(&event.cwd)?;

    // `.lock()` only fails if another thread panicked while holding it;
    // `unwrap` is the usual choice for a POC.
    let mut roots = state.roots.lock().unwrap();
    // `entry().or_insert_with()` = "get it, or insert this if missing".
    let root = roots
        .entry(event.session_id.clone())
        .or_insert_with(|| cwd.clone());

    // `Path::starts_with` compares whole components, so /a/bc doesn't
    // "start with" /a/b the way a plain string check would.
    if !cwd.starts_with(&*root) {
        bail!(
            "cwd {} is outside the session root {}",
            cwd.display(),
            root.display()
        );
    }
    Ok(root.clone())
}

/// Canonicalize (resolve `..` and symlinks) and refuse roots that would
/// expose far too much: `/` and your home directory itself.
pub fn checked_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    let root = cwd
        .canonicalize()
        .with_context(|| format!("cwd {} doesn't exist", cwd.display()))?;

    let home = std::env::var_os("HOME").map(PathBuf::from);
    if root.parent().is_none() || Some(&root) == home.as_ref() {
        bail!("refusing to use {} as a judge root", root.display());
    }
    Ok(root)
}

/// Shared by the daemon and the one-shot `shephrd judge` command.
pub async fn judge_event(
    event: &PermissionRequest,
    root: &Path,
    config: &Config,
) -> anyhow::Result<Review> {
    let ctx = read_transcript(&event.transcript_path, config.user_turns);
    cli_judge::run_judge(event, root, &ctx, config).await
}

fn log_review(path: &Path, review: &Review) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", serde_json::to_string(review)?)?;
    Ok(())
}
