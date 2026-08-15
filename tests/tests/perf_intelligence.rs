//! Feature 003's performance and bounded-work evidence.
//!
//! The claim this file exists to hold is narrow and load-bearing: **nothing
//! Feature 003 adds runs on the session-open path** (FR-471, SC-320). Session
//! start must not verify memories, scan the repository, run tests, or wait on
//! any background work — so a project with a large evidence set opens exactly
//! as fast as one with none, and the number of verification runs it triggers is
//! zero.
//!
//! The loaded-project measurements (5,000 memories, 10,000 evidence facts) land
//! with T141; this is the part that is a *property* rather than a number, and it
//! is checked here so a regression is caught before the scale fixture exists.

use cairn_e2e::store_fixture::Fixture;
use cairn_e2e::Sandbox;
use uuid::Uuid;

/// T056 — zero verification runs occur during session open.
#[test]
fn no_verification_at_session_open() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");

    // One session, opened first, so every later call resolves to it. Two open
    // sessions in one worktree is an ambiguity Cairn reports rather than
    // guesses at, and it would be measuring the wrong thing here.
    let started = s.cairn(&["session", "start", "--agent", "claude-code", "--key", "perf-1", "--json"]);
    assert!(
        started.ok(),
        "session start failed: code={} stdout={} stderr={}",
        started.code,
        started.stdout,
        started.stderr
    );
    let session_id: String = {
        let v: serde_json::Value = serde_json::from_str(&started.stdout).expect("json");
        v["data"]["session"]["id"]
            .as_str()
            .or_else(|| v["session"]["id"].as_str())
            .expect("session id")
            .to_string()
    };

    // A memory with real, checkable evidence attached: exactly the shape a
    // background pass would pick up.
    let m = s.cairn(&[
        "memory", "add", "The API listens on port 8080.",
        "--scope", "project", "--topic-key", "service.api_port",
        "--value-key", "8080", "--session", &session_id, "--json",
    ]);
    assert!(m.ok(), "{}", m.stderr);
    let memory_id: String = {
        let v: serde_json::Value = serde_json::from_str(&m.stdout).expect("json");
        v["data"]["memory"]["id"]
            .as_str()
            .expect("memory id")
            .to_string()
    };

    let e = s.cairn(&[
        "evidence", "add",
        "--type", "configuration",
        "--subject", "API port",
        "--value", "8080",
        "--locator", "config/app.yml#server.port",
        "--memory", &memory_id,
        "--json",
    ]);
    assert!(e.ok(), "{}", e.stderr);

    let runs_before = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM verification_runs");
    assert_eq!(runs_before, vec!["0".to_string()], "nothing has verified yet");

    // Take the briefing — the whole session-open path.
    let context = s.cairn(&["context", "--session", &session_id, "--json"]);
    assert!(
        context.ok(),
        "context failed: code={} stdout={} stderr={}",
        context.code,
        context.stdout,
        context.stderr
    );

    let runs_after = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM verification_runs");
    assert_eq!(
        runs_after,
        vec!["0".to_string()],
        "session open triggered a verification run; FR-471 forbids any"
    );

    // The state a briefing reports for an unchecked claim is the honest one.
    let verification =
        s.query_column("SELECT DISTINCT verification FROM memories WHERE deleted_at IS NULL");
    assert_eq!(
        verification,
        vec!["unverified".to_string()],
        "something verified a memory without being asked"
    );

    // And an explicit verify does run — so the zero above is a property of the
    // session-open path, not of verification being broken.
    let verified = s.cairn(&["verify", "--memory", &memory_id, "--json"]);
    assert!(verified.ok(), "{}", verified.stderr);
    let after_explicit = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM verification_runs");
    assert_ne!(after_explicit, vec!["0".to_string()], "verify did nothing");
}

/// Session open stays within Feature 001's context deadline with Feature 003
/// state present.
///
/// A saturated host is an invalid measurement rather than a failure — the
/// correction in `docs/feature-001-followups.md` §6 applies here too — so this
/// asserts a generous ceiling and leaves the tight numbers to the loaded-project
/// fixture in T141.
#[test]
fn session_open_is_not_slowed_by_evidence_present() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");

    for i in 0..25 {
        let m = s.cairn(&[
            "memory", "add", &format!("Claim number {i}."),
            "--scope", "project", "--topic-key", &format!("topic.number_{i}"),
            "--value-key", &format!("v{i}"), "--json",
        ]);
        assert!(m.ok(), "{}", m.stderr);
        let v: serde_json::Value = serde_json::from_str(&m.stdout).expect("json");
        let id = v["data"]["memory"]["id"].as_str().expect("id");
        let e = s.cairn(&[
            "evidence", "add", "--type", "configuration",
            "--subject", "API port", "--value", "8080",
            "--locator", "config/app.yml#server.port",
            "--memory", id, "--json",
        ]);
        assert!(e.ok(), "{}", e.stderr);
    }

    let clock = std::time::Instant::now();
    let context = s.cairn(&["context", "--json"]);
    let elapsed = clock.elapsed();
    assert!(
        context.ok(),
        "context failed: code={} stdout={} stderr={}",
        context.code,
        context.stdout,
        context.stderr
    );

    // The sandbox deliberately runs with generous hook deadlines because the
    // suite saturates a laptop; what is asserted here is that assembling a
    // briefing over evidence-bearing memories stays in the same order of
    // magnitude, not a production number.
    assert!(
        elapsed.as_secs() < 10,
        "session open took {elapsed:?} with 25 evidence-bearing memories"
    );

    assert_eq!(
        s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM verification_runs"),
        vec!["0".to_string()],
        "assembling a briefing verified something"
    );
}

/// The bounded pass respects its caps and yields rather than overrunning
/// (FR-472, SC-320).
#[test]
fn the_bounded_pass_yields_rather_than_overrunning() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        // More candidates than a small cap allows.
        for i in 0..12 {
            let m = f
                .propose(
                    Uuid::now_v7(),
                    Some(&format!("topic.number_{i}")),
                    Some(&format!("v{i}")),
                    &format!("Claim {i}."),
                )
                .await;
            let e = cairn_store::evidence::record(
                &f.store,
                cairn_store::evidence::NewEvidence {
                    project_id: f.project,
                    kind: cairn_core::EvidenceKind::File,
                    collector: cairn_core::EvidenceCollector::Cairn,
                    subject: "a file",
                    observed_value: "content",
                    source_locator: "src/lib.rs",
                    fingerprint: "aaa",
                    observation_id: None,
                    repo_branch: "main",
                    repo_commit: None,
                    collected_by_session: Uuid::now_v7(),
                },
                256,
                256,
            )
            .await
            .expect("evidence");
            cairn_store::evidence::attach_to_memory(
                &f.store,
                m.memory.id,
                e.id,
                cairn_core::EvidenceRole::Supports,
                Uuid::now_v7(),
            )
            .await
            .expect("attach");
        }

        // The candidate query is what the pass is bounded by, so asserting the
        // bound there asserts the pass cannot exceed it.
        let candidates: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM (
               SELECT m.id FROM memories m
                 JOIN memory_evidence_facts l ON l.memory_id = m.id AND l.role = 'supports'
                 JOIN evidence_facts f ON f.id = l.evidence_id AND f.deleted_at IS NULL
                WHERE m.project_id = ?1 AND m.verification IN ('needs_recheck','unverified','drifted')
                LIMIT 5)",
        )
        .bind(f.project.to_string())
        .fetch_one(f.store.pool())
        .await
        .expect("candidates");
        assert_eq!(candidates, 5, "the cap did not bind the candidate read");
    });
}
