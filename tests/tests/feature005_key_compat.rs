//! Legacy value keys across the normalization change (T015 ↔ T142).
//!
//! ## The interval this covers
//!
//! T015 turns on separator folding for value keys now. T142 rewrites existing
//! rows to the new canonical form during the explicit US7 migration (FR-867a),
//! and that is the permanent fix. Between the two, a store written by Feature
//! 004 holds `server authoritative` while every new proposal naming that same
//! value normalizes to `server_authoritative`.
//!
//! Under plain string equality that interval is not merely untidy. It changes
//! three answers a user reads:
//!
//! - corroboration stops being detected;
//! - a **conflict is recorded between a claim and a restatement of itself**;
//! - the subject view splits into two partitions and reads `Conflicted` with no
//!   winner.
//!
//! None of those is a disagreement anybody had. They are one value, seen across
//! a normalization change.
//!
//! ## What is asserted here
//!
//! The fixture is a **real v7 store**, migrated. Not a v8 store with an
//! old-looking string written into it: the point is that the row was written by
//! code that predates the folding, through the schema that predates it, so the
//! test cannot pass by accident of how the fixture was seeded.
//!
//! And the converse is asserted just as hard — a genuinely different value must
//! still conflict. A compatibility rule that made everything agree would pass
//! the first test and destroy the feature.

use cairn_core::domain::{MemoryScope, MemoryType};
use cairn_core::knowledge::{comparable_value_key, value_keys_agree, ProposalOutcome};
use cairn_e2e::feature005::{Local, LocalAt, LOCAL_SCHEMA_V7};
use cairn_store::outbox::SyncPolicy;
use cairn_store::repo::{self, CreateOutcome, NewMemory};
use uuid::Uuid;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// A v7 store holding one memory whose `value_key` is in the pre-folding form,
/// then migrated to v8.
///
/// The row is inserted through the v7 schema before the migration runs, which
/// is what makes it a legacy row rather than a modern row that happens to
/// contain a space.
async fn store_with_a_legacy_value_key(topic: &str, legacy_value: &str, content: &str) -> Local {
    let db = LocalAt::new(LOCAL_SCHEMA_V7).await;
    let project = db.project;
    let session = Uuid::now_v7();
    db.execute(&format!(
        "INSERT INTO sessions
             (id, project_id, user_id, agent, branch, worktree_path, agent_session_key,
              status, started_at, last_event_at, daemon_run_id)
         VALUES ('{session}', '{project}', 'tester', 'claude-code', 'main', '/fixture',
                 'key-{session}', 'active', '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z',
                 'run-1')"
    ))
    .await;
    db.execute(&format!(
        "INSERT INTO memories
             (id, project_id, type, scope, scope_key, content, state, origin_session_id,
              local_only, created_at, updated_at, topic_key, value_key)
         VALUES ('{}', '{project}', 'decision', 'project', '{project}', '{content}',
                 'active', '{session}', 0, '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z',
                 '{topic}', '{legacy_value}')",
        Uuid::now_v7()
    ))
    .await;

    let migrated = db.migrate_to_latest().await;
    // The migration must not have rewritten the key: that is T142's job, under
    // an explicit user-invoked procedure, and doing it inside a schema
    // migration would be a semantic migration nobody asked for.
    let stored: String = sqlx::query_scalar("SELECT value_key FROM memories LIMIT 1")
        .fetch_one(migrated.store.pool())
        .await
        .expect("the legacy row");
    assert_eq!(
        stored, legacy_value,
        "schema migration 0008 rewrote a value key; that is T142's work, not its own"
    );
    migrated
}

async fn propose(db: &Local, topic: &str, value: &str, content: &str) -> CreateOutcome {
    repo::create_memory_reconciled(
        &db.store,
        NewMemory {
            project_id: db.project,
            kind: MemoryType::Decision,
            scope: MemoryScope::Project,
            scope_key: &db.project.to_string(),
            content,
            origin_session_id: Uuid::now_v7(),
            local_only: false,
            evidence: &[],
            topic_key: Some(topic),
            value_key: Some(value),
            importance: cairn_core::Importance::Normal,
        },
        SyncPolicy {
            linked: false,
            server_project_id: None,
        },
        cairn_store::repo::DEFAULT_RECONCILE_MEMBERS_MAX,
    )
    .await
    .expect("the proposal is stored regardless of how it reconciles")
}

// ---------------------------------------------------------------------------
// The case the report named
// ---------------------------------------------------------------------------

#[test]
fn a_legacy_value_key_does_not_conflict_with_its_own_canonical_form() {
    rt().block_on(async {
        // Exactly the record from the report: stored `server authoritative`,
        // proposal input `server authoritative`, canonical form
        // `server_authoritative`.
        let db = store_with_a_legacy_value_key(
            "storage.authority",
            "server authoritative",
            "storage is server authoritative",
        )
        .await;

        let outcome = propose(
            &db,
            "storage.authority",
            "server authoritative",
            "the server decides what is durable",
        )
        .await;

        match outcome.reconciliation {
            ProposalOutcome::Corroborating { .. } => {}
            ProposalOutcome::ConflictDetected { with } => panic!(
                "a claim conflicted with a restatement of itself, purely because \
                 one row predates separator folding (conflicts with {with:?})"
            ),
            other => panic!("expected corroboration, got {other:?}"),
        }

        // And no `conflicts_with` row was written. The outcome is what a caller
        // reads; the relation is what stays in the database and shows up in
        // every later derivation.
        let conflicts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM memory_relations WHERE kind = 'conflicts_with'",
        )
        .fetch_one(db.store.pool())
        .await
        .unwrap();
        assert_eq!(conflicts, 0, "a false conflict was recorded durably");
    });
}

#[test]
fn a_genuinely_different_value_still_conflicts() {
    rt().block_on(async {
        // The converse, and the one that matters most: a compatibility rule
        // that made everything agree would pass the test above and quietly
        // destroy conflict detection.
        let db = store_with_a_legacy_value_key(
            "storage.authority",
            "server authoritative",
            "storage is server authoritative",
        )
        .await;

        let outcome = propose(
            &db,
            "storage.authority",
            "client authoritative",
            "the client decides what is durable",
        )
        .await;

        match outcome.reconciliation {
            ProposalOutcome::ConflictDetected { with } => {
                assert_eq!(
                    with.len(),
                    1,
                    "the legacy member should be the one conflict"
                )
            }
            other => panic!("a real disagreement was not detected: {other:?}"),
        }
        let conflicts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM memory_relations WHERE kind = 'conflicts_with'",
        )
        .fetch_one(db.store.pool())
        .await
        .unwrap();
        assert_eq!(conflicts, 1);
    });
}

#[test]
fn every_legacy_separator_form_agrees_with_its_canonical_form() {
    rt().block_on(async {
        // The old normalizer lower-cased and collapsed whitespace and stopped,
        // so a stored key could carry a space, a hyphen or a slash verbatim.
        // Each of those is now one canonical key.
        for legacy in [
            "server authoritative",
            "server-authoritative",
            "server/authoritative",
            "Server Authoritative",
            "server  authoritative",
        ] {
            let db = store_with_a_legacy_value_key("storage.authority", legacy, "the legacy claim")
                .await;
            let outcome = propose(
                &db,
                "storage.authority",
                "server_authoritative",
                "a restatement",
            )
            .await;
            assert!(
                matches!(
                    outcome.reconciliation,
                    ProposalOutcome::Corroborating { .. }
                ),
                "{legacy:?} conflicted with its own canonical form: {:?}",
                outcome.reconciliation
            );
        }
    });
}

#[test]
fn a_subject_holding_both_forms_settles_rather_than_reading_as_conflicted() {
    rt().block_on(async {
        // The read path, not the write path. A subject partitioned on the raw
        // string would split one value in two and report `Conflicted` with no
        // winner — a user opening the subject would see Cairn disagreeing with
        // itself about something nobody disputed.
        let db = store_with_a_legacy_value_key(
            "storage.authority",
            "server authoritative",
            "the legacy claim",
        )
        .await;
        propose(
            &db,
            "storage.authority",
            "server_authoritative",
            "the modern claim",
        )
        .await;

        let view = cairn_store::knowledge::subject(
            &db.store,
            db.project,
            MemoryScope::Project,
            &db.project.to_string(),
            "storage.authority",
            cairn_store::repo::DEFAULT_RECONCILE_MEMBERS_MAX,
        )
        .await
        .expect("subject");

        // `Corroborated`, not `Conflicted`. Two statements agreeing on one
        // value legitimately produce two answers — corroboration is "the value
        // is agreed and the statements are several" — so the assertion is about
        // the reconciliation state, which is what a user reads as a warning.
        assert_eq!(
            view.view.reconciliation,
            cairn_core::Reconciliation::Corroborated,
            "a subject holding one value in two spellings did not corroborate"
        );
        assert!(
            !view.view.reconciliation.is_warning(),
            "Cairn reported a warning about a disagreement nobody had"
        );
        // Both rows are still present and still their own records: the
        // compatibility rule changes what a comparison calls them, and merges
        // and discards nothing.
        assert_eq!(view.members.len(), 2, "a member went missing");
    });
}

#[test]
fn the_compatibility_rule_is_not_similarity() {
    // A pure-function guard on the boundary of the rule. `value_keys_agree` is
    // "already canonical, or folds to canonical" — nothing else. Two keys that
    // fold to different values stay different however alike they look.
    assert!(value_keys_agree(
        "server_authoritative",
        "server_authoritative"
    ));
    assert!(value_keys_agree(
        "server authoritative",
        "server_authoritative"
    ));
    assert!(value_keys_agree(
        "Server-Authoritative",
        "server_authoritative"
    ));

    for unrelated in [
        "client_authoritative",
        "server_authoritatively",
        "server",
        "authoritative",
        "servers_authoritative",
    ] {
        assert!(
            !value_keys_agree(unrelated, "server_authoritative"),
            "{unrelated:?} was treated as the same value"
        );
    }

    // A canonical key is a fixed point, so once T142 has rewritten a row this
    // degenerates to string equality and stops doing anything at all.
    assert_eq!(
        comparable_value_key("server_authoritative"),
        "server_authoritative"
    );
    assert_eq!(
        comparable_value_key("server authoritative"),
        "server_authoritative"
    );
    // A stored key that cannot be normalized keeps its literal form: agreeing
    // with nothing is wrong, and agreeing with everything is worse.
    assert_eq!(comparable_value_key(""), "");
}

#[test]
fn topic_key_normalization_did_not_move_at_all() {
    // Value keys are the only thing Feature 005 changed. Topic keys have folded
    // separators since Feature 003, so every stored topic key is already
    // canonical — and if the folding had shifted even for an unusual character,
    // a lookup keyed on the new form would stop finding rows written under the
    // old one and a proposal would silently start a second subject.
    use cairn_core::knowledge::normalize_topic_key;
    for (input, expected) in [
        ("Storage Authority", Some("storage_authority")),
        ("storage-authority", Some("storage_authority")),
        ("storage/authority", Some("storage_authority")),
        ("deploy.images", Some("deploy.images")),
        ("a..b", Some("a.b")),
        (".a.", Some("a")),
        // The pre-Feature-005 behaviour for a tab: dropped, not folded.
        ("a\tb", Some("ab")),
        ("a\nb", Some("ab")),
        ("storage@authority", Some("storageauthority")),
        ("", None),
    ] {
        assert_eq!(
            normalize_topic_key(input).as_deref(),
            expected,
            "topic normalization moved for {input:?}"
        );
    }
}
