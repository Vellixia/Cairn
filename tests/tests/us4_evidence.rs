//! US4 — evidence, verification and authority.
//!
//! The negative this slice exists to prevent: **an agent's attestation must
//! never become indistinguishable from a check Cairn performed** (FR-370). The
//! state says what was established; the authority says what established it, and
//! the two stay separate in storage, on every surface, and on the wire.

use cairn_core::{
    EvidenceCollector, EvidenceKind, EvidenceRole, VerificationAuthority, VerificationState,
    VerifierKind, VerifyResult, VerifyTrigger,
};
use cairn_e2e::store_fixture::Fixture;
use cairn_store::evidence::{self, NewEvidence, NewRun};
use uuid::Uuid;

async fn fact(
    f: &Fixture,
    kind: EvidenceKind,
    collector: EvidenceCollector,
    value: &str,
    locator: &str,
) -> evidence::EvidenceFact {
    evidence::record(
        &f.store,
        NewEvidence {
            project_id: f.project,
            kind,
            collector,
            subject: "database backend",
            observed_value: value,
            source_locator: locator,
            fingerprint: &cairn_core::digest(value),
            observation_id: None,
            repo_branch: "main",
            repo_commit: Some("abc123"),
            collected_by_session: Uuid::now_v7(),
        },
        256,
        256,
    )
    .await
    .expect("record evidence")
}

async fn run(
    f: &Fixture,
    memory: Uuid,
    evidence_id: Uuid,
    verifier: VerifierKind,
    result: VerifyResult,
) {
    evidence::record_run(
        &f.store,
        NewRun {
            project_id: f.project,
            memory_id: Some(memory),
            criterion_id: None,
            verifier,
            evidence_id: Some(evidence_id),
            expected_digest: Some("aaa"),
            observed_digest: Some("aaa"),
            result,
            detail: None,
            repo_branch: "main",
            repo_commit: Some("abc123"),
            trigger: VerifyTrigger::OnDemand,
        },
    )
    .await
    .expect("record run");
}

/// T045 — no surface renders `cairn` and `attested` alike, and no code path
/// produces a `verified` state with no authority at all.
///
/// This is what makes attestation-as-deterministic-check structurally visible
/// rather than a matter of care (metric 25b).
#[test]
fn authority_is_never_collapsed() {
    // Every rendering differs, pairwise.
    let mut rendered = std::collections::BTreeSet::new();
    for authority in [
        "cairn",
        "attested",
        "remote_cairn",
        "remote_attested",
    ] {
        let line = cairn_e2e::render_authority("verified", Some(authority));
        assert!(
            rendered.insert(line.clone()),
            "{authority} renders identically to another authority: {line}"
        );
    }
    assert_eq!(rendered.len(), 4, "four authorities, four renderings");

    // And the two that rest on attestation say so in words, not only in a
    // machine field: a person reading the line has to be able to tell.
    assert!(cairn_e2e::render_authority("verified", Some("attested")).contains("attested"));
    assert!(
        cairn_e2e::render_authority("verified", Some("remote_attested")).contains("attested")
    );
    assert!(
        cairn_e2e::render_authority("verified", Some("remote_cairn")).contains("elsewhere"),
        "an imported check must not read as a local one"
    );

    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        // A verified state always carries an authority in storage.
        let m = f
            .propose(Uuid::now_v7(), Some("infra.db"), Some("postgresql"), "PostgreSQL.")
            .await;
        let e = fact(
            &f,
            EvidenceKind::Configuration,
            EvidenceCollector::Cairn,
            "postgresql",
            "config/database.yml",
        )
        .await;
        run(&f, m.memory.id, e.id, VerifierKind::Configuration, VerifyResult::Verified).await;
        evidence::rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rebuild");

        let orphaned = f
            .count(
                "SELECT COUNT(*) FROM memories
                  WHERE verification = 'verified' AND verification_authority IS NULL",
            )
            .await;
        assert_eq!(orphaned, 0, "a verified memory carries no authority");

        // And the reverse: an authority never outlives the state.
        let stray = f
            .count(
                "SELECT COUNT(*) FROM memories
                  WHERE verification <> 'verified' AND verification_authority IS NOT NULL",
            )
            .await;
        assert_eq!(stray, 0);
    });
}

/// T046 — the state machine, driven through the store.
///
/// The exhaustive proof that every documented transition is reachable and every
/// undocumented one is not lives in `cairn-core` and runs over the whole product
/// of states and triggers. This asserts the three the store is responsible for
/// (SC-306).
#[test]
fn state_machine() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let m = f
            .propose(Uuid::now_v7(), Some("service.api_port"), Some("8080"), "Port 8080.")
            .await;
        let e = fact(
            &f,
            EvidenceKind::Configuration,
            EvidenceCollector::Cairn,
            "8080",
            "config/app.yml",
        )
        .await;

        // unverified → verified
        run(&f, m.memory.id, e.id, VerifierKind::Configuration, VerifyResult::Verified).await;
        let (state, _) = evidence::rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rebuild");
        assert_eq!(state, VerificationState::Verified);

        // verified → needs_recheck, on a fingerprint change and nothing else.
        let before = memory_row(&f, m.memory.id).await;
        evidence::set_verification(&f.store, m.memory.id, VerificationState::NeedsRecheck)
            .await
            .expect("mark");
        let after = memory_row(&f, m.memory.id).await;
        assert_eq!(
            (before.0, before.1, before.2, before.3),
            (after.0, after.1, after.2, after.3),
            "marking a recheck changed content, type, scope or provenance"
        );

        // needs_recheck → drifted
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        run(&f, m.memory.id, e.id, VerifierKind::Configuration, VerifyResult::Drifted).await;
        let (state, authority) = evidence::rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rebuild");
        assert_eq!(state, VerificationState::Drifted);
        assert_eq!(authority, None, "a drifted claim has nothing to be authoritative about");

        // drifted → verified, only on a run.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        run(&f, m.memory.id, e.id, VerifierKind::Configuration, VerifyResult::Verified).await;
        let (state, authority) = evidence::rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rebuild");
        assert_eq!(state, VerificationState::Verified);
        assert_eq!(authority, Some(VerificationAuthority::Cairn));

        // Supersession changes no verification state: a superseded memory keeps
        // its last verification, which is what lets a historical query say what
        // was verified then.
        let successor = f
            .propose(Uuid::now_v7(), Some("service.api_port"), Some("9000"), "Port 9000.")
            .await;
        cairn_store::knowledge::reconcile(
            &f.store,
            f.project,
            successor.memory.id,
            m.memory.id,
            cairn_core::RelationKind::Supersedes,
            cairn_core::RelationBasis::ExplicitUser,
            None,
            None,
        )
        .await
        .expect("supersede");

        let kept: (String, Option<String>) = sqlx::query_as(
            "SELECT verification, verification_authority FROM memories WHERE id = ?1",
        )
        .bind(m.memory.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .expect("row");
        assert_eq!(kept.0, "verified", "supersession moved the verification state");
        assert_eq!(kept.1.as_deref(), Some("cairn"));
    });
}

async fn memory_row(f: &Fixture, id: Uuid) -> (String, String, String, String) {
    sqlx::query_as("SELECT content, type, scope, origin_session_id FROM memories WHERE id = ?1")
        .bind(id.to_string())
        .fetch_one(f.store.pool())
        .await
        .expect("row")
}

/// T047 — Cairn runs nothing to establish a claim (FR-365, FR-477).
///
/// Asserted two ways: the verifier module contains no process-spawning or
/// networking call at all, and every locator that would leave the worktree is
/// refused before anything is read.
#[test]
fn cairn_runs_nothing() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/cairnd/src/verify.rs"),
    )
    .expect("the verifier module is readable");

    // Everything a verifier is allowed to do is a read. `cairn-git` is the one
    // exception and it is a *read* of Git, through the crate that owns it.
    for forbidden in [
        "Command::new",
        "process::Command",
        "TcpStream",
        "TcpListener",
        "reqwest",
        "UdpSocket",
        "std::net",
        "spawn_process",
    ] {
        assert!(
            !source.contains(forbidden),
            "the verifier module references {forbidden}; Cairn executes nothing \
             and reaches no network to establish a claim"
        );
    }

    // And a locator can never point outside the worktree.
    for escaping in [
        "/etc/passwd",
        "../../outside.yml",
        "C:\\Windows\\system.ini",
        "\\\\server\\share\\x",
    ] {
        assert!(
            cairn_store::evidence::validate_locator(escaping).is_err(),
            "{escaping} was accepted as a locator"
        );
    }
}

/// T059 — the walkthrough, end to end through the CLI.
#[test]
fn a_configuration_value_verifies_with_its_authority_named() {
    let s = cairn_e2e::Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");

    let m = s.cairn(&[
        "memory", "add", "The API listens on port 8080.",
        "--scope", "project", "--topic-key", "service.api_port",
        "--value-key", "8080", "--json",
    ]);
    assert!(m.ok(), "{}", m.stderr);
    let memory_id = extract(&m.stdout, &["memory", "id"]);

    // The value Cairn can read for itself, named by its key.
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

    // The fingerprint the fact was stored with is the digest of the value the
    // caller gave; the verifier reads the file and compares.
    let v = s.cairn(&["verify", "--memory", &memory_id, "--explain", "--json"]);
    assert!(v.ok(), "{}", v.stderr);
    let body: serde_json::Value = serde_json::from_str(&v.stdout).expect("json");
    let data = &body["data"];
    assert_eq!(data["verification"].as_str(), Some("verified"), "{}", v.stdout);
    assert_eq!(
        data["authority"].as_str(),
        Some("cairn"),
        "a deterministic check must report cairn authority"
    );

    // The run records what was checked, when, and at which repository state.
    let runs = data["runs"].as_array().expect("runs");
    assert!(!runs.is_empty());
    assert_eq!(runs[0]["verifier"].as_str(), Some("configuration"));
    assert!(runs[0]["checked_at"].as_str().is_some());
    assert!(runs[0]["repo_branch"].as_str().is_some());

    // Change the file: the claim no longer matches its evidence.
    s.write_file("config/app.yml", "server:\n  port: 9000\n");
    let drifted = s.cairn(&["verify", "--memory", &memory_id, "--json"]);
    let body: serde_json::Value = serde_json::from_str(&drifted.stdout).expect("json");
    assert_eq!(
        body["data"]["verification"].as_str(),
        Some("drifted"),
        "{}",
        drifted.stdout
    );

    // The memory itself is untouched throughout — evidence moving never
    // rewrites knowledge (FR-372).
    let shown = s.cairn(&["memory", "search", "8080", "--json"]);
    assert!(
        shown.stdout.contains("The API listens on port 8080."),
        "{}",
        shown.stdout
    );
}

/// An assertion of importance is not evidence (US4 #2).
#[test]
fn an_assertion_of_importance_leaves_the_memory_unverified() {
    let s = cairn_e2e::Sandbox::new();
    let m = s.cairn(&[
        "memory", "add", "This one really matters.",
        "--scope", "project", "--importance", "high", "--json",
    ]);
    assert!(m.ok(), "{}", m.stderr);
    let memory_id = extract(&m.stdout, &["memory", "id"]);

    let v = s.cairn(&["verify", "--memory", &memory_id, "--json"]);
    assert!(!v.ok(), "an unevidenced memory was verified");
    assert!(
        v.stdout.contains("no_evidence") || v.stderr.contains("no_evidence"),
        "stdout={} stderr={}",
        v.stdout,
        v.stderr
    );
}

/// A credential-bearing configuration file yields a redacted, bounded fact and
/// never the raw text (US4 #4, FR-354).
#[test]
fn a_credential_never_reaches_storage() {
    let s = cairn_e2e::Sandbox::new();
    let secret = "postgres://ledger:CORPUSFIXTUREpassword@db.internal:5432/ledger";
    s.write_file("config/database.yml", &format!("url: {secret}\n"));

    let e = s.cairn(&[
        "evidence", "add",
        "--type", "configuration",
        "--subject", "database url",
        "--value", secret,
        "--locator", "config/database.yml#url",
        "--json",
    ]);
    assert!(e.ok(), "{}", e.stderr);
    assert!(
        !e.stdout.contains("CORPUSFIXTUREpassword"),
        "the response echoed the credential: {}",
        e.stdout
    );

    let listed = s.cairn(&["evidence", "list", "--json"]);
    assert!(
        !listed.stdout.contains("CORPUSFIXTUREpassword"),
        "the stored fact carries the credential: {}",
        listed.stdout
    );
    // The safe fact, its digest and its locator are what remain.
    assert!(listed.stdout.contains("config/database.yml"), "{}", listed.stdout);
}

/// An absolute locator is refused, and nothing is written.
#[test]
fn an_absolute_locator_is_refused() {
    let s = cairn_e2e::Sandbox::new();
    let e = s.cairn(&[
        "evidence", "add",
        "--type", "configuration",
        "--subject", "database backend",
        "--value", "postgresql",
        "--locator", "/etc/cairn/database.yml",
        "--json",
    ]);
    assert!(!e.ok(), "an absolute locator was accepted");
    assert!(
        e.stdout.contains("absolute_locator") || e.stderr.contains("absolute_locator"),
        "stdout={} stderr={}",
        e.stdout,
        e.stderr
    );

    let listed = s.cairn(&["evidence", "list", "--json"]);
    assert!(
        listed.stdout.contains("\"total\": 0") || !listed.stdout.contains("evidence_facts"),
        "a refused locator still wrote a fact: {}",
        listed.stdout
    );
}

/// Attested evidence reaches `verified` and is labelled everywhere.
#[test]
fn attested_evidence_is_usable_and_visibly_weaker() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let m = f
            .propose(Uuid::now_v7(), Some("service.health"), Some("ok"), "The service is healthy.")
            .await;
        let e = fact(
            &f,
            EvidenceKind::RuntimeState,
            EvidenceCollector::Agent,
            "ok",
            "runtime/health",
        )
        .await;
        evidence::attach_to_memory(
            &f.store,
            m.memory.id,
            e.id,
            EvidenceRole::Supports,
            Uuid::now_v7(),
        )
        .await
        .expect("attach");
        run(&f, m.memory.id, e.id, VerifierKind::RuntimeState, VerifyResult::Verified).await;

        let (state, authority) = evidence::rebuild_verification(&f.store, m.memory.id)
            .await
            .expect("rebuild");
        assert_eq!(state, VerificationState::Verified, "attested evidence is usable");
        assert_eq!(authority, Some(VerificationAuthority::Attested));

        // And it does not satisfy a consumer that requires a deterministic
        // check (SC-328's half that lives here).
        assert!(!cairn_core::verify::satisfies_deterministic_requirement(authority));
        assert_eq!(
            cairn_core::verify::deterministic_refusal_code(authority),
            Some("attested_not_sufficient")
        );
    });
}

fn extract(json: &str, path: &[&str]) -> String {
    let mut v: serde_json::Value = serde_json::from_str(json).expect("json");
    if v.get("data").is_some() {
        v = v["data"].clone();
    }
    let mut cursor = &v;
    for key in path {
        cursor = &cursor[key];
    }
    cursor
        .as_str()
        .unwrap_or_else(|| panic!("no {path:?} in {json}"))
        .to_string()
}
