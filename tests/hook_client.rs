//! The real `shephrd hook` binary. Golden rule under test: whenever it can't
//! get a verdict, it prints nothing and exits 0, so Claude Code shows you the
//! normal dialog. None of these call the judge.

use std::path::Path;
use std::process::Stdio;
use tokio::{io::AsyncWriteExt, process::Command};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/events")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

/// Run `shephrd hook --url <url>` with `stdin`, return (exit success, stdout).
async fn run_hook(url: &str, stdin: &str, env: &[(&str, &str)]) -> (bool, String) {
    // Cargo builds the binary for integration tests and tells us where it is.
    let mut child = Command::new(env!("CARGO_BIN_EXE_shephrd"))
        .args(["hook", "--url", url])
        // Isolate from your real ~/.config/shephrd; defaults apply.
        .env("SHEPHRD_CONFIG", "/nonexistent/shephrd.toml")
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take().unwrap();
    pipe.write_all(stdin.as_bytes()).await.unwrap();
    drop(pipe);
    let out = child.wait_with_output().await.unwrap();
    (out.status.success(), String::from_utf8(out.stdout).unwrap())
}

/// Serve the real router on a random free port; returns its base URL.
async fn start_daemon() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(
        async move { axum::serve(listener, shephrd::server::app(Default::default())).await },
    );
    url
}

#[tokio::test]
async fn daemon_down_falls_back_silently() {
    // Bind then drop, so we know the port is free and nothing listens on it.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (ok, stdout) = run_hook(
        &format!("http://127.0.0.1:{port}"),
        &fixture("bash_gh_search.json"),
        &[],
    )
    .await;
    assert!(ok);
    assert_eq!(stdout, "");
}

#[tokio::test]
async fn garbage_stdin_falls_back_silently() {
    let (ok, stdout) = run_hook("http://127.0.0.1:1", "not json", &[]).await;
    assert!(ok);
    assert_eq!(stdout, "");
}

#[tokio::test]
async fn judge_env_guard_stays_out() {
    let url = start_daemon().await;
    let (ok, stdout) = run_hook(
        &url,
        &fixture("bash_gh_search.json"),
        &[("SHEPHRD_JUDGE", "1")],
    )
    .await;
    assert!(ok);
    assert_eq!(stdout, "");
}

#[tokio::test]
async fn user_only_tools_are_never_judged() {
    let url = start_daemon().await;
    for name in ["ask_user_question.json", "exit_plan_mode.json"] {
        let (ok, stdout) = run_hook(&url, &fixture(name), &[]).await;
        assert!(ok, "{name}");
        assert_eq!(
            stdout, "",
            "{name} got a decision; it should be left to the user"
        );
    }
}

#[tokio::test]
async fn unpinnable_cwd_is_not_judged() {
    // A cwd that doesn't exist can't be pinned, so the daemon must hand it
    // back to the user rather than run the judge somewhere arbitrary.
    let url = start_daemon().await;
    let event = fixture("bash_gh_search.json").replace(
        "/Users/dev/projects/demo-app",
        "/definitely/not/here",
    );
    let (ok, stdout) = run_hook(&url, &event, &[]).await;
    assert!(ok);
    assert_eq!(stdout, "");
}
