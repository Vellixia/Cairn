//! T105 — US7 end to end: two real machines, one real server (SC-304, SC-329,
//! FR-417).
//!
//! `clock_swap_invariance.rs` proves the merge rule against two stores with the
//! server taken out of the picture. This proves the other half: that the rule
//! survives the wire. A payload that dropped a subject identity, a read-back
//! that never served a relation, or a schema the server had not migrated would
//! all leave that test green and this one red — and the second of those three
//! is exactly what happened.

use cairn_e2e::{attach_server, Sandbox, Server};
use serde_json::Value;

fn server() -> Option<Server> {
    match Server::start() {
        Some(s) => Some(s),
        None => {
            eprintln!(
                "SKIPPED: set CAIRN_TEST_DATABASE_URL (e.g. \
                 `docker run -p 5433:5432 postgres:17-alpine`) to run the server suite"
            );
            None
        }
    }
}

/// Two machines sharing one project on one server.
struct Pair {
    a: Sandbox,
    b: Sandbox,
}

fn pair(server: &Server, label: &str) -> Pair {
    let token = server.new_user_token(label);
    let a = Sandbox::new();
    attach_server(&a, server, &token);
    a.must(&["init"]);
    let project_id = a.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .expect("a shared project")
        .to_string();

    let b = Sandbox::new();
    attach_server(&b, server, &token);
    b.must(&["init"]);
    b.json(&["link", "--project", &project_id]);
    Pair { a, b }
}

impl Pair {
    /// Push and pull on both machines, twice.
    ///
    /// Twice because one round is not convergence: the first carries each
    /// machine's own work up and the other's down, and a relation whose
    /// endpoint arrived in that same round is held rather than applied. The
    /// second round delivers it. A test that synced once would be asserting
    /// that the hold never happens.
    fn settle(&self) {
        for _ in 0..2 {
            for s in [&self.a, &self.b] {
                s.json(&["sync", "now"]);
            }
        }
    }
}

fn propose(s: &Sandbox, topic: &str, value: &str, content: &str) -> String {
    let v = s.json(&[
        "memory",
        "add",
        content,
        "--type",
        "decision",
        "--scope",
        "project",
        "--topic-key",
        topic,
        "--value-key",
        value,
    ]);
    v["memory"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a memory id: {v}"))
        .to_string()
}

fn subject(s: &Sandbox, topic: &str) -> Value {
    let out = s.cairn(&["memory", "subject", topic, "--json"]);
    assert!(out.ok(), "{}", out.stderr);
    let v: Value = serde_json::from_str(&out.stdout).expect("json");
    v["data"]["subject"].clone()
}

// ---------------------------------------------------------------------------
// Incompatible proposals from two offline machines
// ---------------------------------------------------------------------------

/// Both survive with their provenance, and both machines report `Conflicted`.
///
/// Neither machine overwrote the other, because there is no canonical row to
/// overwrite: the answer is derived on read from every member (FR-336, SC-304).
#[test]
fn incompatible_proposals_from_two_machines_both_survive() {
    let Some(server) = server() else { return };
    let p = pair(&server, "offline-merge");

    // Offline: each machine writes its own answer before either syncs.
    propose(
        &p.a,
        "deploy.queue_backend",
        "sqs",
        "The queue runs on SQS in production",
    );
    propose(
        &p.b,
        "deploy.queue_backend",
        "rabbitmq",
        "We deploy RabbitMQ for the work queue",
    );

    p.settle();

    for (who, s) in [("A", &p.a), ("B", &p.b)] {
        let view = subject(s, "deploy.queue_backend");
        assert_eq!(
            view["reconciliation"].as_str(),
            Some("conflicted"),
            "machine {who} did not report the disagreement: {view}"
        );
        assert_eq!(
            view["answers"].as_array().map(|a| a.len()),
            Some(2),
            "machine {who} lost a proposal or picked a winner: {view}"
        );
    }

    // And the provenance survived: two distinct origin sessions, on both.
    for (who, s) in [("A", &p.a), ("B", &p.b)] {
        let origins = s.query_column(
            "SELECT CAST(COUNT(DISTINCT origin_session_id) AS TEXT) FROM memories
              WHERE topic_key = 'deploy.queue_backend' AND deleted_at IS NULL",
        );
        assert_eq!(
            origins,
            vec!["2".to_string()],
            "machine {who} collapsed the two proposals' provenance"
        );
    }
}

// ---------------------------------------------------------------------------
// A decision made on one machine
// ---------------------------------------------------------------------------

/// A supersession decided on A lands on B from the **recorded decision**, not
/// from a copied state column (FR-412, D67, R5).
///
/// This is the defect research B2 found: importing only the memory row left the
/// decision stranded on the machine that made it, so a supersession decided
/// elsewhere never landed.
#[test]
fn a_supersession_decided_elsewhere_lands() {
    let Some(server) = server() else { return };
    let p = pair(&server, "supersession");

    let old = propose(&p.a, "api.port", "8080", "The API listens on port 8080");
    let new = propose(&p.a, "api.port", "9000", "The API listens on port 9000");

    // B learns of both before the decision is made, which is what makes this a
    // decision arriving rather than a state arriving.
    p.settle();
    assert_eq!(
        subject(&p.b, "api.port")["reconciliation"].as_str(),
        Some("conflicted"),
        "B should hold both proposals and no answer yet"
    );

    // A decides. Never automatic — a supersession is always recorded (FR-325).
    let decided = p.a.json(&[
        "memory",
        "reconcile",
        "--from",
        &new,
        "--to",
        &old,
        "--relation",
        "supersedes",
        "--basis",
        "explicit_user",
    ]);
    assert!(decided.get("error").is_none(), "{decided}");

    p.settle();

    let view = subject(&p.b, "api.port");
    assert_eq!(
        view["reconciliation"].as_str(),
        Some("settled"),
        "the decision did not land on B: {view}"
    );
    assert_eq!(
        view["answers"].as_array().map(|a| a.len()),
        Some(1),
        "B should have exactly one current answer: {view}"
    );

    // And B derived the state from the relation rather than being told it.
    let superseded = p.b.query_column(
        "SELECT CAST(COUNT(*) AS TEXT) FROM memories
          WHERE topic_key = 'api.port' AND state = 'superseded'",
    );
    assert_eq!(
        superseded,
        vec!["1".to_string()],
        "B holds the relation but never re-derived from it"
    );
    let relation = p.b.query_column(
        "SELECT CAST(COUNT(*) AS TEXT) FROM memory_relations WHERE kind = 'supersedes'",
    );
    assert_eq!(
        relation,
        vec!["1".to_string()],
        "the decision itself never crossed the wire"
    );
}

/// Two machines proposing at the same time are each attributed.
///
/// Neither is credited to the other, and neither is anonymized into a single
/// merged record (FR-417).
#[test]
fn concurrent_proposals_are_each_attributed() {
    let Some(server) = server() else { return };
    let p = pair(&server, "attribution");

    let from_a = propose(
        &p.a,
        "build.cache",
        "sccache",
        "Builds use sccache on this machine",
    );
    let from_b = propose(
        &p.b,
        "build.cache",
        "none",
        "Builds run without a compiler cache here",
    );
    p.settle();

    for (who, s) in [("A", &p.a), ("B", &p.b)] {
        for id in [&from_a, &from_b] {
            let rows = s.query_column(&format!(
                "SELECT origin_session_id FROM memories WHERE id = '{id}'"
            ));
            assert_eq!(
                rows.len(),
                1,
                "machine {who} does not hold the proposal {id}"
            );
            assert!(
                !rows[0].is_empty(),
                "machine {who} holds {id} with no origin session"
            );
        }
        let origins = s.query_column(
            "SELECT CAST(COUNT(DISTINCT origin_session_id) AS TEXT) FROM memories
              WHERE topic_key = 'build.cache'",
        );
        assert_eq!(
            origins,
            vec!["2".to_string()],
            "machine {who} attributed both proposals to one session"
        );
    }
}

// ---------------------------------------------------------------------------
// A peer's verification says how it was established
// ---------------------------------------------------------------------------

/// A peer never renders an attested verification as a deterministic one
/// (FR-368, FR-370, SC-329).
///
/// An agent's claim that it checked something is worth recording and worth
/// distinguishing. Without this, `{state: verified, basis: [test_outcome]}`
/// would arrive from a peer and be rendered exactly like a check Cairn itself
/// ran here — which is how an agent could launder its own assertion into
/// something the next session treats as machine-verified.
#[test]
fn authority_survives() {
    let Some(server) = server() else { return };
    let p = pair(&server, "authority");

    let id = propose(
        &p.a,
        "test.suite_runner",
        "nextest",
        "The suite runs under cargo nextest",
    );

    // An agent attests it. No Cairn-collected evidence exists, so the strongest
    // authority available is `attested`.
    let added = p.a.json(&[
        "evidence",
        "add",
        "--type",
        "runtime_state",
        "--subject",
        "suite runner",
        "--value",
        "nextest",
        "--locator",
        "Cargo.toml",
        "--collector",
        "agent",
        "--memory",
        &id,
    ]);
    let fact_id = added["evidence"]["id"]
        .as_str()
        .or_else(|| added["fact"]["id"].as_str())
        .unwrap_or_else(|| panic!("an evidence fact id: {added}"))
        .to_string();

    // The attestation is recorded as a run, through the store's own API.
    //
    // `cairn verify` deliberately cannot do this: Cairn does not re-collect an
    // agent's observation, so it answers `inconclusive` and the memory stays
    // unverified — which is correct, and is not what this test is about. What
    // is under test is what happens to an `attested` verification **on the
    // wire**, so the local state is established the way the agent surface
    // establishes it.
    attest(&p.a, &id, &fact_id);

    let local = p.a.query_column(&format!(
        "SELECT verification_authority FROM memories WHERE id = '{id}'"
    ));
    assert_eq!(
        local,
        vec!["attested".to_string()],
        "an agent's attestation must not be recorded as Cairn's own check"
    );

    p.settle();

    let remote = p.b.query_column(&format!(
        "SELECT verification_authority FROM memories WHERE id = '{id}'"
    ));
    assert_eq!(
        remote,
        vec!["remote_attested".to_string()],
        "the peer's attestation arrived wearing the wrong badge"
    );

    // The two authorities a peer must never claim.
    for forbidden in ["cairn", "remote_cairn"] {
        assert_ne!(
            remote[0], forbidden,
            "an imported verification was rendered as `{forbidden}`"
        );
    }
}

/// Record an agent's attestation as a verification run, and rebuild the state.
///
/// The store's own API, opened against the sandbox's database, so the resulting
/// `attested` state is the one the product produces rather than one written by
/// hand into a column.
fn attest(s: &Sandbox, memory_id: &str, evidence_id: &str) {
    use cairn_core::domain::{VerifierKind, VerifyResult, VerifyTrigger};
    use uuid::Uuid;

    let (memory_id, evidence_id) = (
        Uuid::parse_str(memory_id).expect("memory id"),
        Uuid::parse_str(evidence_id).expect("evidence id"),
    );
    let path = s.db_path();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async move {
        let store = cairn_store::Store::open(&path).await.expect("store");
        let project: String =
            sqlx::query_scalar("SELECT id FROM projects WHERE deleted_at IS NULL LIMIT 1")
                .fetch_one(store.pool())
                .await
                .expect("project");
        let project_id = Uuid::parse_str(&project).expect("project id");

        cairn_store::evidence::record_run(
            &store,
            cairn_store::evidence::NewRun {
                project_id,
                memory_id: Some(memory_id),
                criterion_id: None,
                verifier: VerifierKind::RuntimeState,
                evidence_id: Some(evidence_id),
                expected_digest: None,
                observed_digest: None,
                result: VerifyResult::Verified,
                detail: None,
                repo_branch: "main",
                repo_commit: None,
                trigger: VerifyTrigger::OnDemand,
            },
        )
        .await
        .expect("record run");

        let (state, authority) = cairn_store::evidence::rebuild_verification(&store, memory_id)
            .await
            .expect("rebuild");
        assert_eq!(
            format!("{state:?}"),
            "Verified",
            "the attestation did not produce a verified state"
        );
        assert_eq!(
            format!("{authority:?}"),
            "Some(Attested)",
            "an agent's attestation must carry the `attested` authority"
        );
    });
}
