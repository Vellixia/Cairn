//! T116 — the promotion gate against the adversarial privacy corpus (FR-397,
//! SC-315).
//!
//! Thirty cases, each seeding one shape the redactor knows into a candidate
//! that is otherwise perfectly promotable: a provider key in every form, PEM
//! blocks, JWTs, bearer credentials, connection strings with credentials,
//! `KEY=value` assignments, absolute POSIX, Windows and UNC paths, the project
//! name in four casings, the repository remote with and without credentials, a
//! `server_project_id`, a `git_common_dir`, and an email address.
//!
//! Three assertions per case, and the second and third matter as much as the
//! first: **100% refused**, **no refusal echoes the value it found**, and **no
//! partial pattern exists afterwards**. A gate that refused but wrote a row
//! first, or that helpfully quoted the credential it spotted, would satisfy a
//! test that only counted refusals.

use cairn_core::domain::{
    EvidenceCollector, EvidenceKind, MemoryScope, MemoryType, VerifierKind, VerifyResult,
    VerifyTrigger,
};
use cairn_store::outbox::SyncPolicy;
use cairn_store::patterns::{self, Candidate, Promotion};
use cairn_store::{evidence, repo, Store};
use serde_json::Value;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const LOCAL: SyncPolicy = SyncPolicy {
    linked: false,
    server_project_id: None,
};

fn corpus() -> Vec<(String, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("knowledge/privacy");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let name = p
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .into_owned();
            let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{name}: {e}"));
            (
                name.clone(),
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}")),
            )
        })
        .collect()
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn text<'v>(value: &'v Value, key: &str) -> &'v str {
    value.get(key).and_then(|v| v.as_str()).unwrap_or_default()
}

/// A store holding one project with the case's identity, and one source memory
/// that passes every gate check the case is not about.
struct Bench {
    store: Store,
    memory: Uuid,
    _dir: tempfile::TempDir,
}

async fn bench(project_facts: &Value) -> Bench {
    // `machine_salt` writes under CAIRN_HOME. One home for the binary, set
    // once: see `shared_home`.
    cairn_e2e::shared_home();

    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(&dir.path().join("cairn.sqlite3"))
        .await
        .expect("store");
    let project = Uuid::now_v7();
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO projects (id, name, git_common_dir, repository_remote, linked,
                               server_project_id, created_at, updated_at, deleted_at)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?6, NULL)",
    )
    .bind(project.to_string())
    .bind(text(project_facts, "name"))
    .bind(text(project_facts, "git_common_dir"))
    .bind(
        project_facts
            .get("repository_remote")
            .and_then(|v| v.as_str()),
    )
    .bind(
        project_facts
            .get("server_project_id")
            .and_then(|v| v.as_str()),
    )
    .bind(&now)
    .execute(store.pool())
    .await
    .expect("project");

    let session = Uuid::now_v7();
    let scope_key = project.to_string();
    let memory = repo::create_memory(
        &store,
        repo::NewMemory {
            project_id: project,
            kind: MemoryType::Procedure,
            scope: MemoryScope::Project,
            scope_key: &scope_key,
            content: "A procedure worth sharing.",
            origin_session_id: session,
            local_only: false,
            evidence: &[],
            topic_key: None,
            value_key: None,
            importance: cairn_core::Importance::Normal,
        },
        LOCAL,
    )
    .await
    .expect("memory");

    // Verified by a deterministic Cairn check, with a fact behind it. Every
    // gate check except the two the corpus is about must pass, or a refusal
    // would prove nothing about the privacy scan.
    let fact = evidence::record(
        &store,
        evidence::NewEvidence {
            project_id: project,
            kind: EvidenceKind::File,
            collector: EvidenceCollector::Cairn,
            subject: "runbook",
            observed_value: "present",
            source_locator: "docs/runbook.md",
            fingerprint: "digest:abc",
            observation_id: None,
            repo_branch: "main",
            repo_commit: None,
            collected_by_session: session,
        },
        256,
        256,
    )
    .await
    .expect("fact");
    evidence::attach_to_memory(
        &store,
        memory.id,
        fact.id,
        cairn_core::domain::EvidenceRole::Supports,
        session,
    )
    .await
    .expect("attach");
    evidence::record_run(
        &store,
        evidence::NewRun {
            project_id: project,
            memory_id: Some(memory.id),
            criterion_id: None,
            verifier: VerifierKind::FileDigest,
            evidence_id: Some(fact.id),
            expected_digest: Some("digest:abc"),
            observed_digest: Some("digest:abc"),
            result: VerifyResult::Verified,
            detail: None,
            repo_branch: "main",
            repo_commit: None,
            trigger: VerifyTrigger::OnDemand,
        },
    )
    .await
    .expect("run");
    let (state, authority) = evidence::rebuild_verification(&store, memory.id)
        .await
        .expect("rebuild");
    assert_eq!(
        (format!("{state:?}"), format!("{authority:?}")),
        ("Verified".to_string(), "Some(Cairn)".to_string()),
        "the bench must present a source the gate would otherwise accept"
    );

    Bench {
        store,
        memory: memory.id,
        _dir: dir,
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// Every adversarial case is refused, quietly, and leaves nothing behind.
#[test]
fn every_adversarial_candidate_is_refused() {
    let cases = corpus();
    assert!(
        cases.len() >= 30,
        "the adversarial corpus should cover every shape the redactor knows, \
         found {}",
        cases.len()
    );

    runtime().block_on(async {
        for (name, case) in cases {
            let given = &case["input"]["extra"];
            let expected: Vec<String> = case["expect"]["refusals"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            let seeded = given["seeded_value"].as_str().unwrap_or_default();

            let b = bench(&given["project"]).await;
            let c = &given["candidate"];
            let signals = strings(c, "signals");
            let applicability = strings(c, "applicability");
            let constraints = strings(c, "constraints");

            let outcome = patterns::promote(
                &b.store,
                b.memory,
                Candidate {
                    title: text(c, "title"),
                    problem: text(c, "problem"),
                    signals: &signals,
                    applicability: &applicability,
                    root_cause: text(c, "root_cause"),
                    approach: text(c, "approach"),
                    constraints: &constraints,
                },
                2,
                false,
            )
            .await
            .expect("the gate runs");

            let (class, message) = match &outcome {
                Promotion::Refused { class, message } => (*class, message.clone()),
                Promotion::Promoted(p) => panic!(
                    "{name}: a candidate carrying a {} was promoted: {}",
                    given["seeded_class"].as_str().unwrap_or("secret"),
                    p.title
                ),
            };

            assert!(
                expected.iter().any(|e| e == class),
                "{name}: refused with `{class}`, expected one of {expected:?}"
            );

            // The refusal must not repeat what it found. A message quoting the
            // credential puts it wherever the message goes.
            assert!(
                !seeded.is_empty(),
                "{name}: the fixture must name the value it seeded"
            );
            assert!(
                !message.contains(seeded),
                "{name}: the refusal echoed the offending value: {message}"
            );

            // And nothing was written. A gate that refuses after writing has
            // already leaked whatever it refused.
            let written: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reusable_patterns")
                .fetch_one(b.store.pool())
                .await
                .expect("count");
            assert_eq!(written, 0, "{name}: a partial pattern was written");
        }
    });
}

/// The candidate that carries none of it promotes.
///
/// Without this the test above would pass against a gate that refused
/// everything, which is not a privacy property but a broken feature.
#[test]
fn a_clean_candidate_still_promotes() {
    runtime().block_on(async {
        let facts = serde_json::json!({
            "name": "Helios Ledger",
            "repository_remote": "github.com/acme/helios-ledger",
            "server_project_id": "6b1f2c34-0000-7000-8000-00000000abcd",
            "git_common_dir": "/Users/dev/src/helios-ledger/.git",
        });
        let b = bench(&facts).await;
        let signals = vec![
            "could not obtain advisory lock for migration".to_string(),
            "migration blocked waiting on lock".to_string(),
        ];
        let outcome = patterns::promote(
            &b.store,
            b.memory,
            Candidate {
                title: "Recover a stuck migration lock",
                problem: "A migration aborts and leaves an advisory lock held.",
                signals: &signals,
                applicability: &["the migration runner uses a database advisory lock".to_string()],
                root_cause: "The aborted process never released the advisory lock.",
                approach: "Confirm no runner is alive, release the lock, and rerun.",
                constraints: &[
                    "releasing a lock a live runner holds corrupts the migration".to_string(),
                ],
            },
            2,
            false,
        )
        .await
        .expect("the gate runs");

        let pattern = match outcome {
            Promotion::Promoted(p) => p,
            Promotion::Refused { class, message } => {
                panic!("a clean candidate was refused as `{class}`: {message}")
            }
        };
        assert_eq!(pattern.trust.as_str(), "sanitized");
        assert_eq!(
            pattern.sanitization_report["outcome"].as_str(),
            Some("passed")
        );
        // The report names classes and counts, never values.
        let report = pattern.sanitization_report.to_string();
        assert!(
            !report.contains("Helios") && !report.contains("/Users/"),
            "the sanitization report must name classes, never values: {report}"
        );
    });
}
