//! US7 — what crosses the sync boundary, and what cannot
//! (`contracts/privacy-sync.md`).
//!
//! Feature 001 drew this boundary **structurally**: no observation entity type,
//! no server table, and an allowlist enforced on the wire. Feature 003 keeps the
//! method — everything it declines to share, it declines by having nowhere to
//! put it, not by a rule someone must remember.
//!
//! These assertions are the readable form of that. Each one fails if a later
//! change quietly opens a path.

use cairn_e2e::Sandbox;
use serde_json::Value;

/// A shared memory says exactly five things about its evidence — no more
/// (FR-502, D66, D76, SC-329).
///
/// The count is the assertion. A sixth key is how content starts leaking: a
/// subject, an observed value, a locator or a fingerprint would each be one
/// small, reasonable-looking addition.
#[test]
fn a_shared_memory_says_five_things_about_evidence() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    s.git(&["add", "."]);
    s.git(&["commit", "-m", "config", "--no-gpg-sign"]);

    // A linked project, so the outbox actually queues a payload.
    link(&s);

    let m = s.json(&[
        "memory",
        "add",
        "the API listens on 8080",
        "--scope",
        "project",
    ]);
    let memory_id = m["memory"]["id"].as_str().expect("id").to_string();

    s.must(&[
        "evidence",
        "add",
        "--type",
        "configuration",
        "--subject",
        "API port",
        "--value",
        "8080",
        "--locator",
        "config/app.yml#server.port",
        "--collector",
        "cairn",
        "--memory",
        &memory_id,
    ]);

    let payloads = s.query_column(
        "SELECT payload FROM outbox WHERE entity_type = 'memory' ORDER BY rowid DESC LIMIT 1",
    );
    let payload: Value =
        serde_json::from_str(payloads.first().expect("a queued memory payload")).expect("JSON");

    let verification = payload["verification"]
        .as_object()
        .unwrap_or_else(|| panic!("no verification object in {payload}"));

    let mut keys: Vec<&str> = verification.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "authority",
            "basis",
            "fact_count",
            "last_verified_at",
            "state"
        ],
        "a shared memory carries exactly these five keys about its evidence"
    );

    // `basis` carries verifier **kind** names only — never a subject, a value,
    // a locator, a digest or a fingerprint.
    let basis = verification["basis"].to_string();
    for leaked in ["8080", "config/app.yml", "server.port", "API port"] {
        assert!(
            !basis.contains(leaked),
            "`basis` leaked `{leaked}`: {basis}"
        );
    }
}

/// Nothing that is local by design is ever queued for the server (FR-503, I8).
///
/// Checked against the outbox itself rather than against a description of it:
/// if some future write path enqueues an evidence fact or a checkpoint, this
/// fails.
#[test]
fn local_only_records_are_never_queued() {
    let s = Sandbox::new();
    link(&s);

    // Exercise the paths that produce local-only records.
    let task = s.json(&[
        "task",
        "new",
        "--title",
        "T",
        "--goal",
        "G",
        "--criterion",
        "one",
    ]);
    let task_id = task["task"]["id"].as_str().expect("id").to_string();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    s.git(&["add", "."]);
    s.git(&["commit", "-m", "c", "--no-gpg-sign"]);
    s.must(&[
        "evidence",
        "add",
        "--type",
        "configuration",
        "--subject",
        "API port",
        "--value",
        "8080",
        "--locator",
        "config/app.yml#server.port",
    ]);
    s.must(&["task", "update", &task_id, "--title", "T2"]);

    let queued = s.query_column("SELECT DISTINCT entity_type FROM outbox");
    for forbidden in [
        "observation",
        "evidence_fact",
        "verification_run",
        "continuity_checkpoint",
        "reusable_pattern",
        "pattern_application",
        "task_change",
        "criterion_evidence",
    ] {
        assert!(
            !queued.iter().any(|t| t == forbidden),
            "`{forbidden}` is local by design and must never be queued: {queued:?}"
        );
    }

    // And no payload carries a field that would smuggle the same content.
    let payloads = s.query_column("SELECT payload FROM outbox");
    for payload in &payloads {
        for field in [
            "observed_value",
            "source_locator",
            "value_digest",
            "fingerprint",
            "pin_reason",
            "rationale",
            "basis_evidence_id",
            "path_fingerprints",
            "task_snapshot_at_bind",
            "prior_value",
            "content_norm_digest",
            "local_revision",
        ] {
            assert!(
                !payload.contains(&format!("\"{field}\"")),
                "a queued payload carries `{field}`, which never leaves the machine: {payload}"
            );
        }
    }
}

/// A criterion travels by stable id, and its local concurrency token does not
/// (FR-413, D80).
#[test]
fn criteria_travel_by_identity_without_the_local_counter() {
    let s = Sandbox::new();
    link(&s);

    let task = s.json(&[
        "task",
        "new",
        "--title",
        "T",
        "--goal",
        "G",
        "--criterion",
        "one",
        "--criterion",
        "two",
    ]);
    let task_id = task["task"]["id"].as_str().expect("id").to_string();
    let shown = s.json(&["task", "show", &task_id]);
    let ac1 = shown["criteria"][0]["id"].as_str().expect("id").to_string();

    s.must(&["task", "criterion", "set", &ac1, "--state", "satisfied"]);

    let payloads =
        s.query_column("SELECT payload FROM outbox WHERE entity_type = 'task_criterion'");
    assert!(
        !payloads.is_empty(),
        "a criterion change must queue the criterion itself, not only the task"
    );

    let payload: Value = serde_json::from_str(&payloads[0]).expect("JSON");
    assert!(payload["id"].is_string(), "a criterion travels by its id");
    assert!(
        payload["state"].is_string() && payload["verification"].is_string(),
        "both axes travel, never collapsed: {payload}"
    );
    assert!(
        payload["revision"].is_null(),
        "the per-criterion revision is a local token and must not travel: {payload}"
    );
}

/// Mark the project linked and make the daemon see it.
///
/// The daemon caches the project row, so writing `linked` behind its back is not
/// enough — without the restart the outbox stays empty and every assertion in
/// this file would pass vacuously.
fn link(s: &Sandbox) {
    s.execute_sql(
        "UPDATE projects SET linked = 1, \
         server_project_id = '01a00000-0000-7000-8000-0000000000ff'",
    );
    s.restart_daemon();
}
