//! T074 — SC-007: Cairn's capture-hook latency, bounded absolutely (D17).
//!
//! The criterion used to be a percentage of the agent's wall-clock time, which
//! turned out to measure the workload rather than Cairn: the same fixed hook
//! cost was +27% against a 13 ms synthetic tool call and under 1% against a
//! realistic one. So SC-007 now bounds what Cairn actually controls — the
//! latency of its own capture hook.
//!
//! **Process startup is included, never subtracted.** A developer pays for the
//! whole hook invocation, so the whole invocation is what gets measured.
//!
//! End-to-end wall-clock overhead is still reported, because it is what a
//! developer feels, but it is informational and gates nothing.

use cairn_e2e::Sandbox;
use serde_json::json;
use std::process::Command;
use std::time::{Duration, Instant};

const CALLS: usize = 200;
/// Repeated so a single lucky or unlucky run cannot decide the result.
const ROUNDS: usize = 3;

const MEDIAN_BUDGET_MS: f64 = 10.0;
const P95_BUDGET_MS: f64 = 25.0;
/// The production capture deadline, which this benchmark runs against.
const DEADLINE_MS: f64 = 250.0;

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f64) * p).ceil() as usize;
    sorted[index.saturating_sub(1).min(sorted.len() - 1)]
}

/// One round of 200 capture hooks. Returns every per-invocation latency.
fn measure_round(s: &Sandbox, round: usize) -> Vec<f64> {
    let mut samples = Vec::with_capacity(CALLS);
    for i in 0..CALLS {
        let started = Instant::now();
        s.hook(
            "PostToolUse",
            json!({
                "session_id": "perf",
                "tool_name": "Edit",
                "tool_input": { "file_path": format!("work/r{round}-f{i}.rs") }
            }),
        );
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples
}

#[test]
fn capture_hook_latency_stays_within_its_absolute_budget() {
    let s = Sandbox::new();

    // Run against the *production* deadline, not the relaxed one the rest of
    // the suite uses — "no hook exceeds its configured deadline" is only
    // meaningful at the value that ships.
    std::fs::write(
        s.home.path().join("config.json"),
        json!({ "capture_deadline_ms": 250, "context_deadline_ms": 1500 }).to_string(),
    )
    .expect("write config");

    s.hook(
        "SessionStart",
        json!({ "session_id": "perf", "source": "startup" }),
    );
    s.settle_session_count(1);

    let mut rounds = Vec::with_capacity(ROUNDS);
    for round in 0..ROUNDS {
        let mut samples = measure_round(&s, round);
        samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));

        let median = percentile(&samples, 0.50);
        let p95 = percentile(&samples, 0.95);
        let max = *samples.last().expect("samples");
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;

        println!(
            "SC-007 round {} of {ROUNDS} ({CALLS} capture hooks, {} build)\n  \
             median {median:.2} ms  p95 {p95:.2} ms  max {max:.2} ms  mean {mean:.2} ms",
            round + 1,
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
        );
        rounds.push((median, p95, max));
    }

    // Capture must actually have happened; a fast run that recorded nothing
    // would prove the opposite of what this test claims.
    s.settle(&format!("{} captured observations", ROUNDS * CALLS), |s| {
        s.json(&["status"])["observation_count"]
            .as_i64()
            .unwrap_or(0)
            >= (ROUNDS * CALLS) as i64
    });

    // Informational: what the same hooks cost end to end, alongside the same
    // work without Cairn. Reported, never asserted (D17).
    report_end_to_end(&s);

    // SC-007 is a claim about what ships.
    if cfg!(debug_assertions) {
        println!(
            "  (debug build: an unoptimised binary pays several extra milliseconds per \
             spawn; run `cargo test --release` to assert the budget)"
        );
        return;
    }

    for (round, (median, p95, max)) in rounds.iter().enumerate() {
        assert!(
            *median <= MEDIAN_BUDGET_MS,
            "round {}: median {median:.2} ms over the {MEDIAN_BUDGET_MS:.0} ms budget",
            round + 1
        );
        assert!(
            *p95 <= P95_BUDGET_MS,
            "round {}: p95 {p95:.2} ms over the {P95_BUDGET_MS:.0} ms budget",
            round + 1
        );
        assert!(
            *max < DEADLINE_MS,
            "round {}: a hook took {max:.2} ms, past its {DEADLINE_MS:.0} ms deadline",
            round + 1
        );
    }
}

/// The wall-clock cost of a short session with and without Cairn connected.
///
/// Kept because it is what a developer experiences; not a gate, because the
/// ratio is dominated by how long the agent's own tool call takes (D17).
fn report_end_to_end(s: &Sandbox) {
    const SAMPLE: usize = 50;

    let tool_call = |i: usize| {
        let path = s.repo_path().join(format!("e2e/file{i}.rs"));
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, format!("pub fn f{i}() {{}}\n")).expect("write");
        let _ = std::fs::read_to_string(&path).expect("read");
        let _ = Command::new("git")
            .arg("-C")
            .arg(s.repo_path())
            .args(["status", "--porcelain"])
            .output()
            .expect("git status");
    };

    let started = Instant::now();
    for i in 0..SAMPLE {
        tool_call(i);
    }
    let without = started.elapsed();

    let started = Instant::now();
    for i in 0..SAMPLE {
        tool_call(100_000 + i);
        s.hook(
            "PostToolUse",
            json!({
                "session_id": "perf",
                "tool_name": "Edit",
                "tool_input": { "file_path": format!("e2e/file{i}.rs") }
            }),
        );
    }
    let with = started.elapsed();

    let overhead = (with.as_secs_f64() - without.as_secs_f64()) / without.as_secs_f64() * 100.0;
    println!(
        "  informational: {SAMPLE} tool calls took {:.2}s without Cairn and {:.2}s with \
         ({overhead:+.1}%); the same fixed hook cost would be {:.1}% of a 500 ms tool call",
        without.as_secs_f64(),
        with.as_secs_f64(),
        (with.as_secs_f64() - without.as_secs_f64()) * 1000.0 / SAMPLE as f64 / 500.0 * 100.0,
    );
    let _: Duration = without;
}
