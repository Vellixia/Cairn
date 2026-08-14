//! T114 — capture latency, per adapter (FR-194, SC-007, SC-122).
//!
//! Feature 001 bounded the cost of Cairn's capture hook absolutely, because
//! that is the part Cairn controls and the part a developer pays on every tool
//! call. Feature 002 added three things to that path: an adapter that
//! normalizes the payload, a canonical event that carries it, and a binary
//! that now links `jsonc-parser`, `toml_edit` and the embedded Skill. All
//! three are on the hot path in the sense that matters — process start — so
//! the budget is re-measured per adapter rather than assumed to be unchanged.
//!
//! The last measurement is the one D27's scope matrix turns on: what a
//! *user-scope* installation costs in a repository Cairn does not manage. A
//! developer who installs Cairn's hooks once for every project pays that cost
//! in every unrelated checkout they open, and if it is material the shared
//! scope stops being the friendly default.

use cairn_e2e::{binary, Sandbox};
use serde_json::json;
use std::time::Instant;

/// SC-122 requires at least 200 capture-class invocations per adapter.
const CALLS: usize = 200;

/// Feature 001's own budget for the same hook (SC-007, D17).
const MEDIAN_BUDGET_MS: f64 = 10.0;
const P95_BUDGET_MS: f64 = 25.0;
/// The production capture deadline this benchmark runs against.
const DEADLINE_MS: f64 = 250.0;

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f64) * p).ceil() as usize;
    sorted[index.saturating_sub(1).min(sorted.len() - 1)]
}

struct Measured {
    median: f64,
    p95: f64,
    max: f64,
}

fn summarize(agent: &str, mut samples: Vec<f64>) -> Measured {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    let m = Measured {
        median: percentile(&samples, 0.50),
        p95: percentile(&samples, 0.95),
        max: *samples.last().expect("samples"),
    };
    println!(
        "SC-122 {agent:<12} {CALLS} capture hooks  median {:.2} ms  p95 {:.2} ms  max {:.2} ms",
        m.median, m.p95, m.max
    );
    m
}

/// One capture-class event for one adapter, in that adapter's own shape.
fn capture(s: &Sandbox, agent: &str, key: &str, i: usize) -> i32 {
    let file = format!("work/{agent}-{i}.rs");
    match agent {
        "claude-code" => s.hook(
            "PostToolUse",
            json!({
                "session_id": key,
                "tool_name": "Edit",
                "tool_input": { "file_path": file }
            }),
        ),
        "codex" => s.hook_as(
            "codex",
            "PostToolUse",
            json!({
                "session_id": key,
                "tool_name": "apply_patch",
                "tool_input": { "file_path": file },
                "tool_response": { "exit_code": 0 }
            }),
        ),
        "opencode" => s.hook_as(
            "opencode",
            "tool.execute.after",
            json!({
                "sessionID": key,
                "tool": "edit",
                "args": { "file_path": file },
                "output": { "exit_code": 0 }
            }),
        ),
        other => panic!("unknown agent {other}"),
    }
    .code
}

fn open_session(s: &Sandbox, agent: &str, key: &str) {
    match agent {
        "claude-code" => {
            s.hook(
                "SessionStart",
                json!({ "session_id": key, "source": "startup" }),
            );
        }
        "codex" => {
            s.hook_as(
                "codex",
                "SessionStart",
                json!({ "session_id": key, "source": "startup" }),
            );
        }
        "opencode" => {
            s.hook_as("opencode", "session.created", json!({ "sessionID": key }));
        }
        other => panic!("unknown agent {other}"),
    }
}

#[test]
fn capture_stays_within_budget_for_every_adapter() {
    let s = Sandbox::new();
    for a in ["claude-code", "codex", "opencode"] {
        s.install_agent(a);
    }
    s.must(&["init"]);
    // The production deadline, not the relaxed one the rest of the suite uses.
    s.set_deadlines(250, 1500);

    let mut measured = Vec::new();
    let mut failures = 0usize;
    for agent in ["claude-code", "codex", "opencode"] {
        let key = format!("perf-{agent}");
        open_session(&s, agent, &key);
        s.settle_session_count(measured.len() + 1);

        let mut samples = Vec::with_capacity(CALLS);
        for i in 0..CALLS {
            let started = Instant::now();
            let code = capture(&s, agent, &key, i);
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
            if code != 0 {
                failures += 1;
            }
        }
        measured.push((agent, summarize(agent, samples)));
    }

    // Zero Cairn failures aborting or visibly disrupting an agent session
    // (FR-194): the hook exits zero on every one of the 600 invocations.
    assert_eq!(failures, 0, "{failures} capture hooks failed the agent");

    // And capture really happened, so this is a latency measurement rather
    // than a measurement of a hook that dropped everything.
    let expected = (CALLS * 3) as i64;
    s.settle_within(
        &format!("{expected} captured observations"),
        std::time::Duration::from_secs(60),
        |s| {
            s.json(&["status"])["observation_count"]
                .as_i64()
                .unwrap_or(0)
                >= expected
        },
    );

    report_unmanaged_repository_cost(&s);

    if cfg!(debug_assertions) {
        println!(
            "  (debug build: an unoptimised binary pays several extra milliseconds per \
             spawn; run `cargo test --release` to assert the budget)"
        );
        return;
    }

    for (agent, m) in &measured {
        assert!(
            m.median <= MEDIAN_BUDGET_MS,
            "{agent}: median {:.2} ms over the {MEDIAN_BUDGET_MS:.0} ms budget",
            m.median
        );
        assert!(
            m.p95 <= P95_BUDGET_MS,
            "{agent}: p95 {:.2} ms over the {P95_BUDGET_MS:.0} ms budget",
            m.p95
        );
        assert!(
            m.max < DEADLINE_MS,
            "{agent}: a hook took {:.2} ms, past its {DEADLINE_MS:.0} ms deadline",
            m.max
        );
    }
}

/// What a user-scope installation costs where Cairn manages nothing.
///
/// The scenario D27 turns on: hooks installed once for every project, and the
/// developer opens an unrelated checkout. A user-scope installation means
/// "capture in every repository", so Cairn does start a project there — and
/// the cost of doing so is paid on every tool call of every unrelated session
/// that developer ever runs.
///
/// Reported rather than asserted. It is an input to a documentation decision
/// (D27's scope matrix), not a budget.
fn report_unmanaged_repository_cost(s: &Sandbox) {
    const SAMPLE: usize = 100;

    let elsewhere = s.cairn_home().join("unmanaged");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");
    let out = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&elsewhere)
        .output()
        .expect("git init");
    assert!(out.status.success(), "could not create the unmanaged repo");

    use std::io::Write;
    use std::process::Stdio;
    let payload = json!({
        "session_id": "unmanaged",
        "tool_name": "Edit",
        "tool_input": { "file_path": "src/other.rs" },
        "cwd": elsewhere.display().to_string()
    })
    .to_string();

    let mut samples = Vec::with_capacity(SAMPLE);
    for _ in 0..SAMPLE {
        let started = Instant::now();
        let mut child = std::process::Command::new(binary("cairn"))
            .args(["hook", "PostToolUse"])
            .current_dir(&elsewhere)
            .env("CAIRN_HOME", s.cairn_home())
            // The sandbox's own daemon: a hand-spelled path is a socket on
            // Unix and nothing at all on Windows, where the endpoint lives in
            // the `\\.\pipe\` namespace — and a hook that cannot reach a
            // daemon still exits 0, so the measurement would quietly become
            // one of a failed connection.
            .env("CAIRN_SOCKET", &s.socket)
            .env("CAIRND_BIN", binary("cairnd"))
            .env("HOME", s.fake_home())
            .env("XDG_CONFIG_HOME", s.fake_home().join(".config"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("hook runs");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(payload.as_bytes())
            .expect("write payload");
        let status = child.wait().expect("hook completes");
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
        assert!(
            status.success(),
            "the hook failed in a repository Cairn does not manage"
        );
    }

    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    println!(
        "SC-122 per-user hook cost in an unmanaged repository ({SAMPLE} invocations)\n  \
         median {:.2} ms  p95 {:.2} ms  max {:.2} ms",
        percentile(&samples, 0.50),
        percentile(&samples, 0.95),
        *samples.last().expect("samples"),
    );

    // A user-scope installation *is* "capture in every repository", so the
    // unrelated checkout does become a project of its own. That is the
    // behavior the scope buys, not a defect — and it is the reason the cost
    // above is worth reporting: a developer who did not want it should be
    // told to install at project scope instead (D27).
    let projects = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM projects");
    assert_eq!(
        projects.first().map(String::as_str),
        Some("2"),
        "a user-scope hook did not capture in the second repository, so the cost \
         reported above is not the cost of the thing D27 is about"
    );
    // Each project is its own: the unrelated repository does not join the
    // managed one (FR-192).
    let names = s.query_column("SELECT DISTINCT git_common_dir FROM projects");
    assert_eq!(
        names.len(),
        2,
        "two repositories collapsed into one project"
    );
}
