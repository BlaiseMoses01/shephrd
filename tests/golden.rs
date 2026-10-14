//! Golden set: real `claude -p` judgments against hand-built scenarios.
//!
//! These cost money and take ~a minute, so they're `#[ignore]`d. Run with:
//!
//!     cargo test --test golden -- --ignored --nocapture
//!     GOLDEN_FILTER=hidden_rm cargo test --test golden -- --ignored --nocapture
//!     GOLDEN_MODEL=haiku cargo test --test golden -- --ignored --nocapture
//!
//! Settings come from your shephrd config (see `shephrd config`), so this tests
//! what the daemon actually runs. GOLDEN_MODEL overrides just the model, for
//! comparing models without editing the config.
//!
//! Each case is a folder in tests/golden/cases/<name>/:
//!   case.json  intent, tool request, expected verdict(s), and why.
//!              `expect` is one verdict, or a list when several are
//!              acceptable (e.g. ["DENY", "ESCALATE"]: both reach you).
//!   project/   copied to <tmp>/project, which becomes the session root
//!   outside/   copied to <tmp>/outside, a sibling the judge must NOT read
//!
//! Every run writes target/golden/<unix-time>-<model>.jsonl for auditing.
//! LLM verdicts aren't deterministic: a case that flips between runs is a
//! signal to tighten the prompt or the case, not to just re-run until green.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shephrd::claude_request::PermissionRequest;
use shephrd::config::Config;
use shephrd::judges::Verdict;
use shephrd::server::{checked_root, judge_event};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// Any string the judge could only know by reading outside the root.
/// Put it in every file under a case's outside/.
const CANARY: &str = "CANARY-";

/// How many judges run at once.
const PARALLEL: usize = 4;

#[derive(Debug, Deserialize)]
struct Case {
    why: String,
    expect: Expect,
    #[serde(default)]
    user_turns: Vec<String>,
    #[serde(default)]
    plan: Option<String>,
    tool_name: String,
    tool_input: Value,
    /// link name (inside project/) -> target, created after copying.
    #[serde(default)]
    symlinks: HashMap<String, String>,
}

/// `"DENY"` or `["DENY", "ESCALATE"]` in case.json. `untagged` makes serde
/// try each variant in order until one fits the JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum Expect {
    One(Verdict),
    AnyOf(Vec<Verdict>),
}

impl Expect {
    fn accepts(&self, got: Verdict) -> bool {
        match self {
            Expect::One(v) => *v == got,
            Expect::AnyOf(vs) => vs.contains(&got),
        }
    }

    fn label(&self) -> String {
        match self {
            Expect::One(v) => format!("{v:?}"),
            Expect::AnyOf(vs) => vs
                .iter()
                .map(|v| format!("{v:?}"))
                .collect::<Vec<_>>()
                .join("|"),
        }
    }
}

/// One line of the audit report.
#[derive(Debug, Serialize)]
struct Outcome {
    case: String,
    why: String,
    expect: Expect,
    got: Option<Verdict>,
    reason: String,
    canary_leaked: bool,
    passed: bool,
    seconds: f64,
}

#[tokio::test]
#[ignore = "calls claude -p; run with --ignored"]
async fn golden_set() {
    let cases_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/cases");
    let filter = std::env::var("GOLDEN_FILTER").unwrap_or_default();

    let (mut config, _) = Config::load(None).expect("shephrd config is invalid");
    if let Ok(model) = std::env::var("GOLDEN_MODEL") {
        config.model = model;
    }
    println!("golden: model {}", config.model);
    let config = Arc::new(config); // shared read-only by every case task

    let mut names: Vec<String> = std::fs::read_dir(&cases_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(&filter))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no cases match GOLDEN_FILTER={filter:?}");

    // A semaphore is a counter of "permits"; each task holds one while it
    // runs, so at most PARALLEL judges are alive at once.
    let permits = Arc::new(Semaphore::new(PARALLEL));
    let mut tasks = JoinSet::new();
    for name in names {
        let dir = cases_dir.join(&name);
        let permits = permits.clone();
        let config = config.clone();
        tasks.spawn(async move {
            let _permit = permits.acquire_owned().await.unwrap();
            run_case(name, &dir, &config).await
        });
    }

    let mut outcomes = Vec::new();
    while let Some(res) = tasks.join_next().await {
        outcomes.push(res.expect("case task panicked"));
    }
    outcomes.sort_by(|a, b| a.case.cmp(&b.case));

    let report = write_report(&outcomes, &config.model);
    print_table(&outcomes, &report);

    let failed: Vec<&str> = outcomes
        .iter()
        .filter(|o| !o.passed)
        .map(|o| o.case.as_str())
        .collect();
    assert!(failed.is_empty(), "golden cases failed: {failed:?}");
}

async fn run_case(name: String, dir: &Path, config: &Config) -> Outcome {
    let case: Case = serde_json::from_str(&std::fs::read_to_string(dir.join("case.json")).unwrap())
        .unwrap_or_else(|e| panic!("{name}/case.json: {e}"));

    // Lay out <tmp>/project and <tmp>/outside. The TempDir is deleted when
    // `tmp` goes out of scope at the end of this function.
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    for sub in ["project", "outside"] {
        if dir.join(sub).exists() {
            copy_dir(&dir.join(sub), &tmp.path().join(sub));
        }
    }
    for (link, target) in &case.symlinks {
        std::os::unix::fs::symlink(target, project.join(link)).unwrap();
    }

    let transcript = tmp.path().join("transcript.jsonl");
    write_transcript(&transcript, &case);

    let root = checked_root(&project).unwrap();
    let event = PermissionRequest {
        session_id: format!("golden-{name}"),
        transcript_path: transcript,
        cwd: root.clone(),
        permission_mode: Some("default".into()),
        hook_event_name: "PermissionRequest".into(),
        tool_name: case.tool_name.clone(),
        tool_input: case.tool_input.clone(),
    };

    let start = Instant::now();
    let result = judge_event(&event, &root, config).await;
    let seconds = start.elapsed().as_secs_f64();

    let (got, reason) = match result {
        Ok(review) => (Some(review.verdict), review.reason),
        Err(e) => (None, format!("ERROR: {e:#}")),
    };
    let canary_leaked = reason.contains(CANARY);
    Outcome {
        passed: got.is_some_and(|g| case.expect.accepts(g)) && !canary_leaked,
        case: name,
        why: case.why,
        expect: case.expect,
        got,
        reason,
        canary_leaked,
        seconds,
    }
}

/// A minimal transcript in the same JSONL shape Claude Code writes, so the
/// real `read_transcript` is exercised end to end.
fn write_transcript(path: &Path, case: &Case) {
    let mut lines = Vec::new();
    if let Some(plan) = &case.plan {
        lines.push(
            json!({"type": "assistant", "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "plan", "name": "ExitPlanMode", "input": {"plan": plan}}
            ]}}),
        );
    }
    for turn in &case.user_turns {
        lines.push(json!({"type": "user", "message": {"role": "user", "content": turn}}));
    }
    let body: Vec<String> = lines.iter().map(Value::to_string).collect();
    std::fs::write(path, body.join("\n")).unwrap();
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn write_report(outcomes: &[Outcome], model: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/golden");
    std::fs::create_dir_all(&dir).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let path = dir.join(format!("{now}-{model}.jsonl"));
    let body: Vec<String> = outcomes
        .iter()
        .map(|o| serde_json::to_string(o).unwrap())
        .collect();
    std::fs::write(&path, body.join("\n") + "\n").unwrap();
    path
}

fn print_table(outcomes: &[Outcome], report: &Path) {
    println!();
    for o in outcomes {
        let got = o.got.map_or("ERROR".to_string(), |v| format!("{v:?}"));
        let mark = if o.passed { "pass" } else { "FAIL" };
        let leak = if o.canary_leaked {
            "  CANARY LEAKED"
        } else {
            ""
        };
        println!(
            "{mark}  {:<30} expect {:<14} got {:<8} {:>5.1}s{leak}",
            o.case,
            o.expect.label(),
            got,
            o.seconds
        );
        if !o.passed {
            println!("      reason: {}", o.reason);
        }
    }
    let passed = outcomes.iter().filter(|o| o.passed).count();
    println!(
        "\n{passed}/{} passed. Report: {}",
        outcomes.len(),
        report.display()
    );
}
