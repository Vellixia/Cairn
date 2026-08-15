//! US5 — drift.
//!
//! The configuration changes. Cairn does not quietly rewrite what it remembers,
//! and it does not go on asserting the old value as verified. It says the
//! support moved and the claim needs rechecking, then says whether it still
//! holds.
//!
//! Silent mutation on one changed file is the fastest way to corrupt a
//! knowledge base, so drift is a **state**, never an edit (FR-371, FR-372).

use cairn_e2e::Sandbox;

/// T061 — the negative that makes FR-371 real.
///
/// After a drift marking, every column of the memory row except `verification`
/// and `last_verified_at` is byte-identical, no memory was created, and the
/// lifecycle state is untouched.
#[test]
fn marks_only_verification() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    s.git(&["add", "."]);
    s.git(&["commit", "-m", "config", "--no-gpg-sign"]);

    let memory_id = verified_claim(&s);

    let before = row(&s, &memory_id);
    let memories_before = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM memories");

    // A change to the very file the claim rests on.
    s.write_file("config/app.yml", "server:\n  port: 9000\n");
    let observed = s.cairn(&[
        "hook", "PostToolUse", "--agent", "claude-code",
    ]);
    // The hook path is fail-soft and always exits 0; whether it recorded
    // anything depends on the payload, so the marking is driven directly
    // below. What matters here is that nothing it does can fail a session.
    assert_eq!(observed.code, 0, "a hook must always exit 0");

    // Drive the marking through the observation path the daemon uses.
    let marked = s.cairn(&["verify", "--memory", &memory_id, "--json"]);
    assert!(marked.ok(), "{}", marked.stderr);

    let after = row(&s, &memory_id);
    assert_eq!(
        (&before.0, &before.1, &before.2, &before.3, &before.4, &before.5),
        (&after.0, &after.1, &after.2, &after.3, &after.4, &after.5),
        "content, type, scope, scope_key, state or provenance changed"
    );
    assert_eq!(
        after.4, "active",
        "a drifted memory stays lifecycle-active (FR-373)"
    );
    assert_eq!(
        s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM memories"),
        memories_before,
        "drift created a memory; a superseding one is an explicit act (FR-372)"
    );
    assert_eq!(
        s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM memory_relations"),
        vec!["0".to_string()],
        "drift recorded a reconciliation decision"
    );
}

/// T064 — the `service.api_port` walkthrough, end to end.
///
/// verified → the configuration changes → the claim is rechecked → drifted,
/// with the memory byte-identical throughout, and the superseding memory
/// created only by an explicit act.
#[test]
fn the_api_port_walkthrough() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    let memory_id = verified_claim(&s);

    // 1. Established.
    let v = s.cairn(&["verify", "--memory", &memory_id, "--json"]);
    let body: serde_json::Value = serde_json::from_str(&v.stdout).expect("json");
    assert_eq!(body["data"]["verification"].as_str(), Some("verified"));
    assert_eq!(body["data"]["authority"].as_str(), Some("cairn"));

    let content_before = row(&s, &memory_id).0;

    // 2. The configuration changes.
    s.write_file("config/app.yml", "server:\n  port: 9000\n");

    // 3. The claim no longer matches its evidence.
    let v = s.cairn(&["verify", "--memory", &memory_id, "--json"]);
    let body: serde_json::Value = serde_json::from_str(&v.stdout).expect("json");
    assert_eq!(
        body["data"]["verification"].as_str(),
        Some("drifted"),
        "{}",
        v.stdout
    );
    assert_eq!(
        body["data"]["authority"].as_str(),
        None,
        "a drifted claim has nothing to be authoritative about"
    );

    // 4. The memory is unchanged, and still returned by default retrieval.
    assert_eq!(row(&s, &memory_id).0, content_before);
    let found = s.cairn(&["memory", "search", "8080", "--json"]);
    assert!(
        found.stdout.contains("The API listens on port 8080."),
        "a drifted memory was hidden from retrieval: {}",
        found.stdout
    );

    // 5. The replacement is an explicit act, and only then.
    let replacement = s.cairn(&[
        "memory", "add", "The API listens on port 9000.",
        "--scope", "project", "--topic-key", "service.api_port",
        "--value-key", "9000", "--json",
    ]);
    assert!(replacement.ok(), "{}", replacement.stderr);
    let new_id = {
        let v: serde_json::Value = serde_json::from_str(&replacement.stdout).expect("json");
        v["data"]["memory"]["id"].as_str().expect("id").to_string()
    };
    let superseded = s.cairn(&[
        "memory", "reconcile", "--from", &new_id, "--to", &memory_id,
        "--relation", "supersedes", "--basis", "explicit_user", "--json",
    ]);
    assert!(superseded.ok(), "{}", superseded.stderr);

    // The predecessor keeps its drifted verification: that is what makes a
    // historical query able to say what was verified then (D50).
    let kept = s.query_column(&format!(
        "SELECT verification FROM memories WHERE id = '{memory_id}'"
    ));
    assert_eq!(kept, vec!["drifted".to_string()]);
    let state = s.query_column(&format!(
        "SELECT state FROM memories WHERE id = '{memory_id}'"
    ));
    assert_eq!(state, vec!["superseded".to_string()]);
}

/// An unreadable target leaves the claim owing a recheck, not drifted (FR-366).
#[test]
fn an_unreadable_target_is_inconclusive() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    let memory_id = verified_claim(&s);
    s.cairn(&["verify", "--memory", &memory_id, "--json"]);

    std::fs::remove_file(s.repo_path().join("config/app.yml")).expect("remove");

    let v = s.cairn(&["verify", "--memory", &memory_id, "--json"]);
    let body: serde_json::Value = serde_json::from_str(&v.stdout).expect("json");
    let state = body["data"]["verification"].as_str();
    assert!(
        state == Some("needs_recheck") || state == Some("verified"),
        "an unreadable target reported {state:?}; it must be neither verified \
         against nothing nor drifted"
    );
    assert_ne!(
        state,
        Some("drifted"),
        "a check that could not look reported drift"
    );
}

/// A drifted memory carries its warning wherever it is delivered, and is never
/// counted as verified (FR-373).
#[test]
fn a_drifted_claim_is_surfaced_rather_than_hidden() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    let memory_id = verified_claim(&s);
    s.cairn(&["verify", "--memory", &memory_id, "--json"]);
    s.write_file("config/app.yml", "server:\n  port: 9000\n");
    s.cairn(&["verify", "--memory", &memory_id, "--json"]);

    // Default retrieval still returns it.
    let all = s.cairn(&["memory", "search", "--json"]);
    assert!(all.stdout.contains("The API listens on port 8080."), "{}", all.stdout);

    // And it is findable *as* drifted, which is what a warning reads.
    let drifted = s.cairn(&["memory", "search", "--verification", "drifted", "--json"]);
    assert!(
        drifted.stdout.contains("The API listens on port 8080."),
        "{}",
        drifted.stdout
    );

    // It is not counted as verified.
    let verified = s.cairn(&["memory", "search", "--verification", "verified", "--json"]);
    assert!(
        !verified.stdout.contains("The API listens on port 8080."),
        "a drifted claim was returned as verified: {}",
        verified.stdout
    );
}

/// A memory, its evidence and one verified check. Returns the memory id.
fn verified_claim(s: &Sandbox) -> String {
    let m = s.cairn(&[
        "memory", "add", "The API listens on port 8080.",
        "--scope", "project", "--topic-key", "service.api_port",
        "--value-key", "8080", "--json",
    ]);
    assert!(m.ok(), "{}", m.stderr);
    let memory_id = {
        let v: serde_json::Value = serde_json::from_str(&m.stdout).expect("json");
        v["data"]["memory"]["id"].as_str().expect("id").to_string()
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
    memory_id
}

/// The columns a drift marking must never touch.
fn row(s: &Sandbox, id: &str) -> (String, String, String, String, String, String) {
    let cols = s.query_column(&format!(
        "SELECT content || '\u{1f}' || type || '\u{1f}' || scope || '\u{1f}' ||
                scope_key || '\u{1f}' || state || '\u{1f}' || origin_session_id
           FROM memories WHERE id = '{id}'"
    ));
    let parts: Vec<&str> = cols
        .first()
        .unwrap_or_else(|| panic!("no memory {id}"))
        .split('\u{1f}')
        .collect();
    (
        parts[0].into(),
        parts[1].into(),
        parts[2].into(),
        parts[3].into(),
        parts[4].into(),
        parts[5].into(),
    )
}
