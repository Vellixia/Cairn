//! What the command line actually prints (T146).
//!
//! Every defect here was found by walking `quickstart.md` on a real repository
//! with a real daemon, and every one of them is the same shape as D2: the
//! daemon knew the answer, the JSON carried it, and the human-readable surface
//! did not print it. A test that asserts on the JSON envelope cannot catch
//! that, which is why these assert on stdout.

use cairn_e2e::Sandbox;

fn add(s: &Sandbox, args: &[&str]) -> String {
    let mut argv = vec!["memory", "add"];
    argv.extend_from_slice(args);
    argv.push("--json");
    let r = s.cairn(&argv);
    assert!(r.ok(), "{}", r.stderr);
    let v: serde_json::Value = serde_json::from_str(&r.stdout).expect("json");
    v["data"]["memory"]["id"].as_str().expect("id").to_string()
}

/// An identifier a person reads is not a JSON string literal.
#[test]
fn a_recorded_memory_names_its_id_without_quoting_it() {
    let s = Sandbox::new();
    let r = s.cairn(&["memory", "add", "Errors are returned", "--scope", "project"]);
    assert!(r.ok(), "{}", r.stderr);
    assert!(
        !r.stdout.contains('"'),
        "the id was printed as a JSON string: {}",
        r.stdout
    );
    assert!(r.stdout.starts_with("Remembered 0"), "{}", r.stdout);
}

/// A search result says whether it still stands, and what it asserts.
///
/// Printing the content alone made a superseded memory and a verified one look
/// identical, which is the distinction `--as-of` exists to draw.
#[test]
fn a_search_result_shows_its_state_and_value() {
    let s = Sandbox::new();
    add(
        &s,
        &[
            "Production runs PostgreSQL 16",
            "--scope",
            "project",
            "--topic-key",
            "infrastructure.production_database",
            "--value-key",
            "postgresql",
        ],
    );
    let r = s.cairn(&[
        "memory",
        "search",
        "--topic-key",
        "infrastructure.production_database",
    ]);
    assert!(r.ok(), "{}", r.stderr);
    assert!(
        r.stdout.contains("active"),
        "no lifecycle state: {}",
        r.stdout
    );
    assert!(
        r.stdout.contains("value: postgresql"),
        "no value key: {}",
        r.stdout
    );
}

/// A historical answer is labelled as one, so it cannot be read as current
/// (FR-342, D82).
#[test]
fn a_temporal_search_echoes_the_instant_it_answered_for() {
    let s = Sandbox::new();
    add(
        &s,
        &[
            "Production runs PostgreSQL 16",
            "--scope",
            "project",
            "--topic-key",
            "infrastructure.production_database",
            "--value-key",
            "postgresql",
        ],
    );
    let r = s.cairn(&[
        "memory",
        "search",
        "--topic-key",
        "infrastructure.production_database",
        "--as-of",
        "2030-01-01T00:00:00Z",
    ]);
    assert!(r.ok(), "{}", r.stderr);
    assert!(
        r.stdout.contains("as_of 2030-01-01T00:00:00Z"),
        "a historical answer did not say it was one: {}",
        r.stdout
    );
}

/// `cairn memory show` answers "does this still hold?", not "here is a JSON
/// document".
#[test]
fn memory_show_leads_with_state_and_verification() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    let id = add(
        &s,
        &[
            "The API listens on 8080",
            "--scope",
            "project",
            "--topic-key",
            "service.api_port",
            "--value-key",
            "8080",
        ],
    );
    let e = s.cairn(&[
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
        "--memory",
        &id,
        "--json",
    ]);
    assert!(e.ok(), "{}", e.stderr);
    assert!(s.cairn(&["verify", "--memory", &id, "--json"]).ok());

    let r = s.cairn(&["memory", "show", &id]);
    assert!(r.ok(), "{}", r.stderr);
    assert!(
        !r.stdout.trim_start().starts_with('{'),
        "memory show printed raw JSON: {}",
        r.stdout
    );
    assert!(r.stdout.contains("state         active"), "{}", r.stdout);
    assert!(
        r.stdout.contains("verification  verified (cairn)"),
        "the verification state is missing: {}",
        r.stdout
    );
    assert!(
        r.stdout.contains("service.api_port = 8080"),
        "the subject is missing: {}",
        r.stdout
    );
}

/// A verification that did not verify says why.
///
/// The run records `inconclusive` with a reason. Reporting only `unverified`
/// hid the single line that says what to change — a locator that names no key.
#[test]
fn a_verification_that_did_not_verify_gives_the_reason() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    let id = add(
        &s,
        &[
            "The API listens on 8080",
            "--scope",
            "project",
            "--topic-key",
            "service.api_port",
            "--value-key",
            "8080",
        ],
    );
    // A locator with no fragment names a file and no key in it.
    let e = s.cairn(&[
        "evidence",
        "add",
        "--type",
        "configuration",
        "--subject",
        "API port",
        "--value",
        "8080",
        "--locator",
        "config/app.yml",
        "--memory",
        &id,
        "--json",
    ]);
    assert!(e.ok(), "{}", e.stderr);

    let r = s.cairn(&["verify", "--memory", &id]);
    assert!(r.ok(), "{}", r.stderr);
    assert!(r.stdout.contains("unverified"), "{}", r.stdout);
    assert!(
        r.stdout.contains("inconclusive"),
        "the run's result was not reported: {}",
        r.stdout
    );
    assert!(
        r.stdout.contains("locator"),
        "the reason was not reported: {}",
        r.stdout
    );
}

/// A subject's answers are what it says, not a column of identifiers (FR-307).
#[test]
fn a_subject_shows_the_value_and_the_statement() {
    let s = Sandbox::new();
    add(
        &s,
        &[
            "Production runs PostgreSQL 16",
            "--scope",
            "project",
            "--topic-key",
            "infrastructure.production_database",
            "--value-key",
            "postgresql",
        ],
    );
    let r = s.cairn(&["memory", "subject", "infrastructure.production_database"]);
    assert!(r.ok(), "{}", r.stderr);
    assert!(
        r.stdout
            .contains("postgresql — \"Production runs PostgreSQL 16\""),
        "the answer did not say what it is: {}",
        r.stdout
    );
}

/// A drift warning names the claim that no longer holds.
///
/// `⚠ DRIFT service.api_port — drifted` restates the warning's own kind as its
/// detail, which tells an agent nothing it can act on.
#[test]
fn a_drift_warning_names_the_claim_that_moved() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    let id = add(
        &s,
        &[
            "The API listens on 8080",
            "--scope",
            "project",
            "--topic-key",
            "service.api_port",
            "--value-key",
            "8080",
        ],
    );
    let e = s.cairn(&[
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
        "--memory",
        &id,
        "--json",
    ]);
    assert!(e.ok(), "{}", e.stderr);
    assert!(s.cairn(&["verify", "--memory", &id, "--json"]).ok());

    s.write_file("config/app.yml", "server:\n  port: 9000\n");
    let v = s.cairn(&["verify", "--memory", &id]);
    assert!(v.ok(), "{}", v.stderr);
    assert!(v.stdout.contains("drifted"), "{}", v.stdout);

    let c = s.cairn(&["context"]);
    assert!(c.ok(), "{}", c.stderr);
    assert!(
        c.stdout.contains("The API listens on 8080"),
        "the drift warning did not name the claim: {}",
        c.stdout
    );
    // The memory is untouched: drift is a state, never an edit (FR-371).
    let show = s.cairn(&["memory", "show", &id]);
    assert!(
        show.stdout.contains("state         active"),
        "{}",
        show.stdout
    );
    assert!(
        show.stdout.contains("verification  drifted"),
        "{}",
        show.stdout
    );
}

/// `2 conflict` reads as a truncated string rather than a count.
#[test]
fn the_warning_summary_counts_in_english() {
    let s = Sandbox::new();
    for (value, content) in [
        ("sqs", "Deploys queue through SQS"),
        ("rabbitmq", "Deploys queue through RabbitMQ"),
    ] {
        add(
            &s,
            &[
                content,
                "--scope",
                "project",
                "--topic-key",
                "deploy.queue_backend",
                "--value-key",
                value,
            ],
        );
    }
    for (value, content) in [
        ("redis", "Cache is Redis"),
        ("memcached", "Cache is Memcached"),
    ] {
        add(
            &s,
            &[
                content,
                "--scope",
                "project",
                "--topic-key",
                "cache.backend",
                "--value-key",
                value,
            ],
        );
    }
    let c = s.cairn(&["context"]);
    assert!(c.ok(), "{}", c.stderr);
    assert!(
        c.stdout.contains("2 conflicts"),
        "the summary did not pluralize: {}",
        c.stdout
    );
}

/// Superseding is reachable from the command line and reports what it replaced.
#[test]
fn supersede_names_what_it_replaced() {
    let s = Sandbox::new();
    let id = add(
        &s,
        &[
            "Production runs PostgreSQL 16",
            "--scope",
            "project",
            "--topic-key",
            "infrastructure.production_database",
            "--value-key",
            "postgresql",
        ],
    );
    let r = s.cairn(&[
        "memory",
        "supersede",
        "Migrated production to CockroachDB",
        "--memory-id",
        &id,
        "--type",
        "decision",
        "--scope",
        "project",
        "--topic-key",
        "infrastructure.production_database",
        "--value-key",
        "cockroachdb",
    ]);
    assert!(r.ok(), "{}", r.stderr);
    assert!(
        r.stdout.contains(&format!("supersedes {id}")),
        "the replaced memory was not named: {}",
        r.stdout
    );
    assert!(!r.stdout.contains('"'), "quoted json leaked: {}", r.stdout);
}

/// The briefing says which criteria are done, and how far along the task is.
///
/// Tier 0a has always carried the per-criterion state, the progress counts and
/// the readiness; the rendered briefing printed the criterion *text* and nothing
/// else. An agent reading it after a compaction saw a list of sentences with no
/// way to tell which were finished — which is the one thing FR-443 guarantees it
/// will still know.
#[test]
fn the_briefing_shows_criterion_state_and_progress() {
    let s = Sandbox::new();
    let t = s.cairn(&[
        "task",
        "new",
        "--title",
        "Retry backoff",
        "--goal",
        "Transient failures retry with jitter",
        "--criterion",
        "backoff is exponential with jitter",
        "--criterion",
        "the retry cap is configurable",
        "--json",
    ]);
    assert!(t.ok(), "{}", t.stderr);
    let task_id = serde_json::from_str::<serde_json::Value>(&t.stdout).expect("json")["data"]
        ["task"]["id"]
        .as_str()
        .expect("task id")
        .to_string();

    let shown = s.cairn(&["task", "show", &task_id, "--json"]);
    assert!(shown.ok(), "{}", shown.stderr);
    let first = serde_json::from_str::<serde_json::Value>(&shown.stdout).expect("json")["data"]
        ["criteria"][0]["id"]
        .as_str()
        .expect("criterion id")
        .to_string();
    assert!(s
        .cairn(&["task", "criterion", "set", &first, "--state", "satisfied"])
        .ok());

    assert!(s
        .cairn(&[
            "session",
            "start",
            "--agent",
            "claude-code",
            "--key",
            "w",
            "--task",
            &task_id
        ])
        .ok());
    let c = s.cairn(&["context"]);
    assert!(c.ok(), "{}", c.stderr);
    assert!(
        c.stdout.contains("AC-1 satisfied"),
        "the criterion's state is missing: {}",
        c.stdout
    );
    assert!(
        c.stdout.contains("AC-2 pending"),
        "the criterion's state is missing: {}",
        c.stdout
    );
    // Both axes, never collapsed (FR-483).
    assert!(c.stdout.contains("satisfied · unverified"), "{}", c.stdout);
    assert!(
        c.stdout
            .contains("Progress: 0 verified · 1 satisfied but unverified"),
        "the progress counts are missing: {}",
        c.stdout
    );
    assert!(c.stdout.contains("Readiness: not_ready"), "{}", c.stdout);
}

/// Every omission carries a reason (FR-461).
///
/// Warnings, constraints and criteria recorded one; memory dropped for budget
/// recorded nothing, so `--explain` could name the section it cut and never say
/// how much went or why — which is the question the explain view exists for.
#[test]
fn the_explain_view_accounts_for_the_memories_it_dropped() {
    let s = Sandbox::new();
    for i in 0..40 {
        assert!(s
            .cairn(&[
                "memory",
                "add",
                &format!("Module {i:03} owns the retry budget for its own transport"),
                "--type",
                "fact",
                "--scope",
                "project",
            ])
            .ok());
    }
    // Small enough that the memory Cairn *did* read cannot all fit — the
    // per-scope read cap means a merely tight budget drops nothing.
    let c = s.cairn(&["context", "--token-budget", "120", "--explain"]);
    assert!(c.ok(), "{}", c.stderr);
    assert!(
        c.stdout.contains("OMITTED"),
        "nothing was reported as omitted at a budget that cannot hold it: {}",
        c.stdout
    );
    assert!(
        c.stdout.contains("budget_exhausted"),
        "the omission carried no reason: {}",
        c.stdout
    );
    assert!(
        c.stdout.contains("memory ×"),
        "the omitted memories were not counted: {}",
        c.stdout
    );
}

/// A post-compaction read learns the ground moved, before it reads anything
/// written against the ground it moved from (FR-434).
#[test]
fn a_diverged_checkpoint_leads_the_briefing() {
    let s = Sandbox::new();
    let t = s.cairn(&[
        "task",
        "new",
        "--title",
        "Retry backoff",
        "--goal",
        "Transient failures retry",
        "--criterion",
        "backoff is exponential",
        "--json",
    ]);
    assert!(t.ok(), "{}", t.stderr);
    let task_id = serde_json::from_str::<serde_json::Value>(&t.stdout).expect("json")["data"]
        ["task"]["id"]
        .as_str()
        .expect("task id")
        .to_string();

    let started = s.cairn(&[
        "session",
        "start",
        "--agent",
        "claude-code",
        "--key",
        "w",
        "--task",
        &task_id,
        "--json",
    ]);
    assert!(started.ok(), "{}", started.stderr);
    let session = serde_json::from_str::<serde_json::Value>(&started.stdout).expect("json")["data"]
        ["session"]["id"]
        .as_str()
        .expect("session id")
        .to_string();

    assert!(s
        .cairn(&["session", "checkpoint", "--session", &session])
        .ok());

    // The ground moves: a new commit under the same session.
    s.write_file("src/config.rs", "pub fn load() { /* moved */ }\n");
    s.git(&["add", "."]);
    s.git(&["commit", "-m", "refactor config loading", "--no-gpg-sign"]);

    let c = s.cairn(&["context", "--reason", "post_compaction"]);
    assert!(c.ok(), "{}", c.stderr);
    assert!(
        c.stdout.starts_with("⚠ CHECKPOINT DIVERGED"),
        "the divergence did not lead the briefing: {}",
        c.stdout
    );
    assert!(
        c.stdout.contains("recorded at") && c.stdout.contains("now at"),
        "the commit divergence was not named: {}",
        c.stdout
    );
    // Never `next action` — it was written against a state that no longer holds.
    assert!(
        !c.stdout.contains("next action (may be stale)")
            || c.stdout.contains("previous next action (may be stale)"),
        "a stale action was presented as the next one: {}",
        c.stdout
    );
    // And the mode the integration actually delivers, never over-claimed.
    assert!(
        c.stdout.contains("continuity agent_initiated"),
        "the continuity mode was not reported: {}",
        c.stdout
    );
}
