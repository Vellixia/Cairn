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

// ---------------------------------------------------------------------------
// The promotion and refusal corpora, through the real gate (T114, T119,
// metric 25a)
// ---------------------------------------------------------------------------

/// Load a corpus group.
fn group(name: &str) -> Vec<(String, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("knowledge/patterns")
        .join(name);
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

/// Put the source memory into the state a case describes, then run the gate.
async fn run_case(case: &Value) -> Promotion {
    let given = &case["input"]["extra"];
    let source = &given["source"];
    let facts = serde_json::json!({
        "name": "Helios Ledger",
        "repository_remote": "github.com/acme/helios-ledger",
        "server_project_id": "6b1f2c34-0000-7000-8000-00000000abcd",
        "git_common_dir": "/Users/dev/src/helios-ledger/.git",
    });
    let b = bench(&facts).await;

    // The bench builds a source that passes every check; each case varies
    // exactly one thing from it, which is what makes the refusal attributable.
    let set = |column: &str, value: String| {
        let sql = format!("UPDATE memories SET {column} = ?1 WHERE id = ?2");
        (sql, value)
    };
    let mut updates = Vec::new();
    if let Some(state) = source["state"].as_str() {
        updates.push(set("state", state.to_string()));
    }
    if let Some(kind) = source["type"].as_str() {
        updates.push(set("type", kind.to_string()));
    }
    if let Some(v) = source["verification"].as_str() {
        updates.push(set("verification", v.to_string()));
    }
    if source["verification_authority"].is_null() && source.get("verification_authority").is_some()
    {
        updates.push(set("verification_authority", String::new()));
    } else if let Some(a) = source["verification_authority"].as_str() {
        updates.push(set("verification_authority", a.to_string()));
    }
    if source["local_only"].as_bool() == Some(true) {
        updates.push(set("local_only", "1".to_string()));
    }
    for (sql, value) in updates {
        let bind: Option<String> = (!value.is_empty()).then_some(value);
        sqlx::query(&sql)
            .bind(bind)
            .bind(b.memory.to_string())
            .execute(b.store.pool())
            .await
            .expect("set the source's state");
    }
    if source["evidence_facts"].as_i64() == Some(0) {
        sqlx::query("DELETE FROM memory_evidence_facts WHERE memory_id = ?1")
            .bind(b.memory.to_string())
            .execute(b.store.pool())
            .await
            .expect("detach evidence");
    }

    // A conflicted subject needs a subject to be conflicted about. The bench
    // memory is free-form, and gate check 5 reads the subject a memory belongs
    // to — so without this the case would skip the check it exists to exercise
    // and be promoted for the wrong reason.
    if source["subject_reconciliation"].as_str() == Some("conflicted") {
        let (project, scope_key): (String, String) =
            sqlx::query_as("SELECT project_id, scope_key FROM memories WHERE id = ?1")
                .bind(b.memory.to_string())
                .fetch_one(b.store.pool())
                .await
                .expect("the source memory");

        sqlx::query(
            "UPDATE memories SET topic_key = 'deploy.queue_backend', value_key = 'sqs'
              WHERE id = ?1",
        )
        .bind(b.memory.to_string())
        .execute(b.store.pool())
        .await
        .expect("give the source a subject");

        // A second, incompatible answer in the same scope.
        sqlx::query(
            "INSERT INTO memories
                (id, project_id, type, scope, scope_key, content, state, origin_session_id,
                 local_only, created_at, updated_at, topic_key, value_key, importance)
             VALUES (?1, ?2, 'decision', 'project', ?3, 'The queue runs on RabbitMQ',
                     'active', ?4, 0, ?5, ?5, 'deploy.queue_backend', 'rabbitmq', 'normal')",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(&project)
        .bind(&scope_key)
        .bind(Uuid::now_v7().to_string())
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(b.store.pool())
        .await
        .expect("a competing answer");
    }

    let c = &given["candidate"];
    let signals = strings(c, "signals");
    let applicability = strings(c, "applicability");
    let constraints = strings(c, "constraints");
    let candidate = Candidate {
        title: text(c, "title"),
        problem: text(c, "problem"),
        signals: &signals,
        applicability: &applicability,
        root_cause: text(c, "root_cause"),
        approach: text(c, "approach"),
        constraints: &constraints,
    };

    // A duplicate case promotes the same candidate twice.
    if given.get("already_promoted").is_some() {
        let _ = patterns::promote(&b.store, b.memory, candidate.clone(), 2, false).await;
    }
    patterns::promote(&b.store, b.memory, candidate, 2, false)
        .await
        .expect("the gate runs")
}

/// Every candidate the gate must let through, does (T114).
#[test]
fn the_promote_corpus_passes_the_gate() {
    let cases = group("promote");
    assert!(!cases.is_empty(), "the promote corpus is empty");
    runtime().block_on(async {
        for (name, case) in cases {
            match run_case(&case).await {
                Promotion::Promoted(p) => assert_eq!(
                    p.trust.as_str(),
                    case["expect"]["extra"]["trust"]
                        .as_str()
                        .unwrap_or("sanitized"),
                    "{name}: promoted with the wrong trust"
                ),
                Promotion::Refused { class, message } => {
                    panic!("{name}: refused as `{class}` — {message}")
                }
            }
        }
    });
}

/// One case per refusal class, refused with **that** class (T114, SC-328).
///
/// Named `attested_source` in `contracts/evaluation.md` metric 25a because the
/// row it exists for is the attestation one: a source verified only by an
/// agent's own claim, and an imported verification, are the two ways an agent
/// could otherwise launder its own assertion into cross-project knowledge.
#[test]
fn attested_source() {
    let cases = group("refuse");
    assert!(
        cases.len() >= 12,
        "the refusal corpus is short: {}",
        cases.len()
    );

    let mut seen: Vec<String> = Vec::new();
    runtime().block_on(async {
        for (name, case) in cases {
            let expected = case["expect"]["extra"]["refusal"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            match run_case(&case).await {
                Promotion::Refused { class, message } => {
                    assert_eq!(class, expected, "{name}: refused as `{class}` — {message}");
                    seen.push(class.to_string());
                }
                Promotion::Promoted(p) => {
                    panic!("{name}: promoted, expected `{expected}` — {}", p.title)
                }
            }
        }
    });

    // The two that matter most for SC-328 are both covered.
    for required in ["attested_not_sufficient", "imported_not_sufficient"] {
        assert!(
            seen.iter().any(|c| c == required),
            "the corpus does not exercise `{required}`: {seen:?}"
        );
    }
}

// ===========================================================================
// T151 / SC-423 — a promoted record outlives its source
// ===========================================================================
//
// Promotion copies; it does not link (FR-519). Nothing on a personal or team
// record points back at the project memory it came from, which is why forgetting
// or deleting that memory leaves the promoted record alone.
//
// That is worth a test rather than a comment because the natural implementation
// is the other one. A `source_memory_id` column is the obvious way to record
// provenance, it is what every other relation in this schema does, and it would
// be wrong twice over: it would name a row in a project the promoted record has
// deliberately shed every trace of, and it would make forgetting the source
// either cascade or dangle.

/// Forgetting the source project memory leaves the promoted personal record
/// exactly as it was.
///
/// Falsified by adding a cascade, a tombstone propagation, or a foreign key from
/// `personal_knowledge` back to `memories`.
#[test]
fn forgetting_a_promoted_source_leaves_the_personal_record_untouched() {
    let s = cairn_e2e::Sandbox::new();
    let mut mcp = cairn_e2e::Mcp::start(&s);
    let cwd = s.repo_path().to_string_lossy().to_string();

    // A subject key is required: the gate's `no_subject` check refuses a source
    // that could not participate in reconciliation at the far end.
    let source = s.json(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "--topic-key",
        "build.cache",
        "--value-key",
        "clear_on_stale",
        "Clear the build cache when a stale artifact is suspected",
    ]);
    let source_id = source["memory"]["id"].as_str().expect("memory id");

    let promoted = mcp.tool_result(
        "cairn_remember",
        serde_json::json!({
            "action": "promote",
            "target": "personal",
            "memory_id": source_id,
        }),
        &cwd,
    );
    assert_eq!(promoted["isError"], false, "promotion failed: {promoted}");
    let promoted_id = promoted["content"][0]["text"]["id"]
        .as_str()
        .expect("promoted id")
        .to_string();

    let before = s.query_column(&format!(
        "SELECT content || '|' || COALESCE(topic_key,'') || '|' || COALESCE(forgotten_at,'') \
           FROM personal_knowledge WHERE id = '{promoted_id}'"
    ));
    assert_eq!(before.len(), 1, "the promoted record is missing");

    // Forget the source, then delete it outright — both mutations, because
    // either could be the one that cascades.
    let forgotten = s.cairn(&["memory", "forget", source_id]);
    assert!(forgotten.ok(), "forget failed: {}", forgotten.stderr);
    let deleted = s.cairn(&["delete", "memory", source_id]);
    assert!(deleted.ok(), "delete failed: {}", deleted.stderr);

    let after = s.query_column(&format!(
        "SELECT content || '|' || COALESCE(topic_key,'') || '|' || COALESCE(forgotten_at,'') \
           FROM personal_knowledge WHERE id = '{promoted_id}'"
    ));
    assert_eq!(
        before, after,
        "forgetting or deleting the source changed the promoted record"
    );
}

/// There is no live reference, asserted against the schema rather than against
/// behaviour.
///
/// The behavioural test above would still pass on a schema that carried a
/// nullable `source_memory_id` nobody had wired a cascade to yet. This one fails
/// the moment such a column exists, which is the point at which the mistake is
/// cheap to undo.
#[test]
fn no_promoted_record_carries_a_reference_to_its_source() {
    let s = cairn_e2e::Sandbox::new();

    for table in ["personal_knowledge", "team_knowledge"] {
        let columns = s.query_column(&format!("SELECT name FROM pragma_table_info('{table}')"));
        assert!(
            !columns.is_empty(),
            "{table} does not exist; this test would pass vacuously"
        );
        for column in &columns {
            assert!(
                !column.contains("memory")
                    && !column.contains("source_id")
                    && !column.contains("project"),
                "{table} carries `{column}`, a reference back to the project the \
                 promoted record deliberately sheds (FR-517, FR-519)"
            );
        }
        // And no foreign key out of the table at all beyond its own domain.
        let foreign = s.query_column(&format!(
            "SELECT \"table\" FROM pragma_foreign_key_list('{table}')"
        ));
        assert!(
            foreign.iter().all(|t| t.starts_with(table)),
            "{table} has a foreign key out of its own domain: {foreign:?}"
        );
    }
}
