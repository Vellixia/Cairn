//! Reconciliation decisions and the subject read (`contracts/knowledge.md`).
//!
//! The derivation itself is pure and lives in `cairn-core`. This module is what
//! feeds it: it stores the decisions and reads the members, and stores **no**
//! canonical answer, because there is none to store (D44).
//!
//! That is the whole reason "no silent last-write-wins" is structural rather
//! than aspirational. There is no row to overwrite, so after any merge from any
//! device the answer is simply recomputed.

use crate::{rows, tx, Result, Store};
use cairn_core::domain::{
    Importance, MemoryScope, MemoryState, RelationBasis, RelationKind, VerificationAuthority,
    VerificationState,
};
use cairn_core::knowledge::{
    derive_subject, normalize_relation_endpoints, MemoryFacts, Relation, SubjectView,
};
use sqlx::Row;
use std::collections::BTreeSet;
use std::str::FromStr;
use uuid::Uuid;

/// A decision to record.
#[derive(Debug, Clone)]
pub struct NewRelation<'a> {
    pub project_id: Uuid,
    pub from: Uuid,
    pub to: Uuid,
    pub kind: RelationKind,
    pub decided_by_session: Uuid,
    pub basis: RelationBasis,
    /// Required when `basis = evidence`. A local-only reference: it is stripped
    /// from the sync payload, because the evidence it names never leaves the
    /// machine (FR-502).
    pub basis_evidence_id: Option<Uuid>,
    /// Bounded and redacted by the caller.
    pub rationale: Option<&'a str>,
}

/// Record a decision, or do nothing if it is already recorded.
///
/// Returns whether a row was written, so a caller can tell "decided" from
/// "already decided" — which is what makes recording the same decision twice a
/// no-op without also making it invisible (FR-305, I2).
///
/// Symmetric kinds have their endpoints normalized first, so two machines that
/// detect one conflict while offline write the same primary key and the merge
/// absorbs the second exactly as it absorbs a local duplicate (D78).
pub async fn record_relation(store: &Store, r: NewRelation<'_>) -> Result<bool> {
    let mut t = tx::begin(store, "record_relation").await?;
    let wrote = record_relation_tx(&mut t, r).await?;
    tx::commit(t, "record_relation").await?;
    Ok(wrote)
}

/// The same, inside a caller's transaction — used where a proposal and its
/// automatic relation must commit together
/// (`contracts/records-and-rebuild.md` §Aggregate ownership).
pub async fn record_relation_tx(
    tx: &mut sqlx::SqliteConnection,
    r: NewRelation<'_>,
) -> Result<bool> {
    let (from, to) = normalize_relation_endpoints(r.kind, r.from, r.to);
    let result = sqlx::query(
        "INSERT OR IGNORE INTO memory_relations
            (from_memory_id, to_memory_id, kind, project_id, decided_by_session,
             decided_at, basis, basis_evidence_id, rationale)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(from.to_string())
    .bind(to.to_string())
    .bind(r.kind.as_str())
    .bind(r.project_id.to_string())
    .bind(r.decided_by_session.to_string())
    .bind(rows::now_text())
    .bind(r.basis.as_str())
    .bind(r.basis_evidence_id.map(|id| id.to_string()))
    .bind(r.rationale)
    .execute(&mut *tx)
    .await?;
    Ok(result.rows_affected() > 0)
}

fn relation_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Relation> {
    Ok(Relation {
        from: rows::uuid(row, "from_memory_id")?,
        to: rows::uuid(row, "to_memory_id")?,
        kind: RelationKind::from_str(row.try_get::<String, _>("kind")?.as_str())
            .map_err(|e| crate::StoreError::Corrupt(e.to_string()))?,
        basis: RelationBasis::from_str(row.try_get::<String, _>("basis")?.as_str())
            .map_err(|e| crate::StoreError::Corrupt(e.to_string()))?,
    })
}

/// Every decision touching any of `ids`.
///
/// Order-independent by construction: `derive_subject` consumes relations as a
/// set, so nothing here needs a stable order to be correct — only to be stable
/// in output.
pub async fn relations_touching(store: &Store, ids: &[Uuid]) -> Result<Vec<Relation>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat("?")
        .take(ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT from_memory_id, to_memory_id, kind, basis FROM memory_relations
          WHERE deleted_at IS NULL
            AND (from_memory_id IN ({placeholders}) OR to_memory_id IN ({placeholders}))
          ORDER BY from_memory_id, to_memory_id, kind"
    );
    let mut q = sqlx::query(&sql);
    for id in ids.iter().chain(ids.iter()) {
        q = q.bind(id.to_string());
    }
    let rows = q.fetch_all(store.pool()).await?;
    rows.iter().map(relation_from_row).collect()
}

/// Every decision in a project, for `doctor` and for the rebuild procedures.
pub async fn relations_for_project(store: &Store, project_id: Uuid) -> Result<Vec<Relation>> {
    let rows = sqlx::query(
        "SELECT from_memory_id, to_memory_id, kind, basis FROM memory_relations
          WHERE project_id = ?1 AND deleted_at IS NULL
          ORDER BY from_memory_id, to_memory_id, kind",
    )
    .bind(project_id.to_string())
    .fetch_all(store.pool())
    .await?;
    rows.iter().map(relation_from_row).collect()
}

fn facts_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<MemoryFacts> {
    let corrupt = |e: cairn_core::domain::ParseEnumError| crate::StoreError::Corrupt(e.to_string());
    Ok(MemoryFacts {
        id: rows::uuid(row, "id")?,
        state: MemoryState::from_str(row.try_get::<String, _>("state")?.as_str())
            .map_err(corrupt)?,
        scope: MemoryScope::from_str(row.try_get::<String, _>("scope")?.as_str())
            .map_err(corrupt)?,
        scope_key: row.try_get("scope_key")?,
        topic_key: row.try_get("topic_key")?,
        value_key: row.try_get("value_key")?,
        content_norm_digest: row.try_get("content_norm_digest")?,
        verification: VerificationState::from_str(
            row.try_get::<String, _>("verification")?.as_str(),
        )
        .map_err(corrupt)?,
        verification_authority: row
            .try_get::<Option<String>, _>("verification_authority")?
            .map(|a| VerificationAuthority::from_str(&a))
            .transpose()
            .map_err(corrupt)?,
        evidence_fact_count: row.try_get::<i64, _>("evidence_fact_count")?.max(0) as usize,
        pinned: row.try_get::<i64, _>("pinned")? != 0,
        importance: Importance::from_str(row.try_get::<String, _>("importance")?.as_str())
            .map_err(corrupt)?,
        origin_session_id: rows::uuid(row, "origin_session_id")?,
    })
}

/// The columns `derive_subject` reads, plus the supporting-evidence count it
/// ranks a representative by.
const MEMBER_COLUMNS: &str = "m.id, m.state, m.scope, m.scope_key, m.topic_key, m.value_key,
     m.content_norm_digest, m.verification, m.verification_authority, m.pinned, m.importance,
     m.origin_session_id,
     (SELECT COUNT(*) FROM memory_evidence_facts mef
       WHERE mef.memory_id = m.id AND mef.role = 'supports') AS evidence_fact_count";

/// A subject, read and derived.
#[derive(Debug, Clone)]
pub struct SubjectRead {
    pub view: SubjectView,
    /// Every member, including superseded and stale ones, so a caller can show
    /// history without a second query.
    pub members: Vec<MemoryFacts>,
    /// True when the read hit its bound and stopped. Assembly reports Feature
    /// 001's existing `degraded` flag rather than inventing one (FR-474).
    pub degraded: bool,
}

/// Read one subject's members and derive its canonical answer.
///
/// One indexed read over `(project_id, scope, scope_key, topic_key)`, bounded by
/// `cap`. Nothing is stored: the answer is recomputed on every read, which is
/// cheaper than the invalidation a materialized projection would need and is
/// what makes the answer survive any merge (D44).
pub async fn subject(
    store: &Store,
    project_id: Uuid,
    scope: MemoryScope,
    scope_key: &str,
    topic_key: &str,
    cap: usize,
) -> Result<SubjectRead> {
    let sql = format!(
        "SELECT {MEMBER_COLUMNS} FROM memories m
          WHERE m.project_id = ?1 AND m.scope = ?2 AND m.scope_key = ?3
            AND m.topic_key = ?4 AND m.deleted_at IS NULL
          ORDER BY m.id
          LIMIT ?5"
    );
    // One more than the cap, so hitting the bound is distinguishable from
    // exactly filling it.
    let rows = sqlx::query(&sql)
        .bind(project_id.to_string())
        .bind(scope.as_str())
        .bind(scope_key)
        .bind(topic_key)
        .bind(cap as i64 + 1)
        .fetch_all(store.pool())
        .await?;

    let degraded = rows.len() > cap;
    let members: Vec<MemoryFacts> = rows
        .iter()
        .take(cap)
        .map(facts_from_row)
        .collect::<Result<_>>()?;

    let ids: Vec<Uuid> = members.iter().map(|m| m.id).collect();
    let relations = relations_touching(store, &ids).await?;
    let view = derive_subject(&members, &relations);

    Ok(SubjectRead {
        view,
        members,
        degraded,
    })
}

/// Every subject identity in a project, for the adoption metric and for
/// `doctor`.
pub async fn subject_keys(store: &Store, project_id: Uuid) -> Result<Vec<(MemoryScope, String, String)>> {
    let rows = sqlx::query(
        "SELECT DISTINCT scope, scope_key, topic_key FROM memories
          WHERE project_id = ?1 AND topic_key IS NOT NULL AND deleted_at IS NULL
          ORDER BY scope, scope_key, topic_key",
    )
    .bind(project_id.to_string())
    .fetch_all(store.pool())
    .await?;

    rows.iter()
        .map(|r| {
            Ok((
                MemoryScope::from_str(r.try_get::<String, _>("scope")?.as_str())
                    .map_err(|e| crate::StoreError::Corrupt(e.to_string()))?,
                r.try_get("scope_key")?,
                r.try_get("topic_key")?,
            ))
        })
        .collect()
}

/// The existing members of the subject a proposal would join.
///
/// Bounded by `cap`; the caller reports `reconciliation_deferred` when the
/// bound binds rather than scanning (FR-474).
pub async fn subject_members_tx(
    tx: &mut sqlx::SqliteConnection,
    project_id: Uuid,
    scope: MemoryScope,
    scope_key: &str,
    topic_key: &str,
    cap: usize,
) -> Result<(Vec<MemoryFacts>, bool)> {
    let sql = format!(
        "SELECT {MEMBER_COLUMNS} FROM memories m
          WHERE m.project_id = ?1 AND m.scope = ?2 AND m.scope_key = ?3
            AND m.topic_key = ?4 AND m.state = 'active' AND m.deleted_at IS NULL
          ORDER BY m.id
          LIMIT ?5"
    );
    let rows = sqlx::query(&sql)
        .bind(project_id.to_string())
        .bind(scope.as_str())
        .bind(scope_key)
        .bind(topic_key)
        .bind(cap as i64 + 1)
        .fetch_all(&mut *tx)
        .await?;

    let over = rows.len() > cap;
    let members = rows
        .iter()
        .take(cap)
        .map(facts_from_row)
        .collect::<Result<Vec<_>>>()?;
    Ok((members, over))
}

/// Recompute `reinforcement_count` and `distinct_origin_count` for a memory
/// (`contracts/records-and-rebuild.md` §Rebuild procedures).
///
/// The counts are derived, never authoritative. They count reinforcing and
/// duplicating decisions and the distinct origin sessions behind them — and
/// they are **never** presented as a number of independent verifications
/// (FR-322, FR-406).
pub async fn rebuild_reinforcement(store: &Store, memory_id: Uuid) -> Result<(i64, i64)> {
    let sources: Vec<Uuid> = sqlx::query(
        "SELECT from_memory_id FROM memory_relations
          WHERE to_memory_id = ?1 AND kind IN ('reinforces', 'duplicates')
            AND deleted_at IS NULL",
    )
    .bind(memory_id.to_string())
    .fetch_all(store.pool())
    .await?
    .iter()
    .map(|r| rows::uuid(r, "from_memory_id"))
    .collect::<Result<_>>()?;

    let reinforcement_count = sources.len() as i64;

    let mut origins: BTreeSet<Uuid> = BTreeSet::new();
    let mut ids = sources.clone();
    ids.push(memory_id);
    for id in ids {
        if let Some(row) = sqlx::query("SELECT origin_session_id FROM memories WHERE id = ?1")
            .bind(id.to_string())
            .fetch_optional(store.pool())
            .await?
        {
            origins.insert(rows::uuid(&row, "origin_session_id")?);
        }
    }
    let distinct_origin_count = origins.len().max(1) as i64;

    sqlx::query(
        "UPDATE memories SET reinforcement_count = ?2, distinct_origin_count = ?3 WHERE id = ?1",
    )
    .bind(memory_id.to_string())
    .bind(reinforcement_count)
    .bind(distinct_origin_count)
    .execute(store.pool())
    .await?;

    Ok((reinforcement_count, distinct_origin_count))
}

/// Recompute `memories.state` and `superseded_by_id` from the `supersedes`
/// relations (`rebuild_supersession`).
///
/// The Feature 001 columns become a **view** of the relation, which is what
/// makes FR-324 true and what lets a remotely decided supersession land on
/// import without any row being overwritten (D67, R5).
///
/// Returns how many rows differed from what the relations imply — a difference
/// is a bug report, not a normal outcome.
pub async fn rebuild_supersession(store: &Store, project_id: Uuid) -> Result<usize> {
    let relations = relations_for_project(store, project_id).await?;

    // The successor of each superseded memory. A memory with several
    // predecessors is fine; a memory with several *successors* is a conflict
    // the derivation reports rather than one this rebuild resolves.
    let mut successor: std::collections::BTreeMap<Uuid, Uuid> = Default::default();
    for r in relations.iter().filter(|r| r.kind == RelationKind::Supersedes) {
        successor.entry(r.to).or_insert(r.from);
    }

    let existing = sqlx::query(
        "SELECT id, state, superseded_by_id FROM memories
          WHERE project_id = ?1 AND deleted_at IS NULL",
    )
    .bind(project_id.to_string())
    .fetch_all(store.pool())
    .await?;

    let mut differed = 0usize;
    for row in &existing {
        let id = rows::uuid(row, "id")?;
        let state: String = row.try_get("state")?;
        let link: Option<String> = row.try_get("superseded_by_id")?;

        let expected_link = successor.get(&id).map(|s| s.to_string());
        let expected_state = if expected_link.is_some() {
            MemoryState::Superseded.as_str()
        } else if state == MemoryState::Stale.as_str() {
            // Staleness is scope, not supersession. The rebuild does not
            // resurrect a stale memory it has no relation for.
            MemoryState::Stale.as_str()
        } else {
            MemoryState::Active.as_str()
        };

        if link != expected_link || state != expected_state {
            differed += 1;
            sqlx::query(
                "UPDATE memories SET state = ?2, superseded_by_id = ?3 WHERE id = ?1",
            )
            .bind(id.to_string())
            .bind(expected_state)
            .bind(expected_link)
            .execute(store.pool())
            .await?;
        }
    }
    Ok(differed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outbox::SyncPolicy;
    use crate::repo::{self, NewMemory};
    use cairn_core::domain::MemoryType;
    use cairn_core::knowledge::ProposalOutcome;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: Store,
        project: Uuid,
        session_a: Uuid,
        session_b: Uuid,
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("cairn.sqlite3")).await.unwrap();
        let project = Uuid::now_v7();
        let now = rows::now_text();
        sqlx::query(
            "INSERT INTO projects (id, name, git_common_dir, repository_remote, linked,
                                   server_project_id, created_at, updated_at, deleted_at)
             VALUES (?1, 'test', ?2, NULL, 0, NULL, ?3, ?3, NULL)",
        )
        .bind(project.to_string())
        .bind(format!("/tmp/git-{project}"))
        .bind(&now)
        .execute(store.pool())
        .await
        .unwrap();
        Fixture {
            _dir: dir,
            store,
            project,
            session_a: Uuid::now_v7(),
            session_b: Uuid::now_v7(),
        }
    }

    impl Fixture {
        async fn propose(
            &self,
            session: Uuid,
            topic: Option<&str>,
            value: Option<&str>,
            content: &str,
        ) -> repo::CreateOutcome {
            let key = self.project.to_string();
            repo::create_memory_reconciled(
                &self.store,
                NewMemory {
                    project_id: self.project,
                    kind: MemoryType::Fact,
                    scope: MemoryScope::Project,
                    scope_key: &key,
                    content,
                    origin_session_id: session,
                    local_only: false,
                    evidence: &[],
                    topic_key: topic,
                    value_key: value,
                    importance: Importance::Normal,
                },
                SyncPolicy { linked: false, server_project_id: None },
                64,
            )
            .await
            .expect("propose")
        }

        async fn subject_of(&self, topic: &str) -> SubjectRead {
            let key = self.project.to_string();
            subject(
                &self.store,
                self.project,
                MemoryScope::Project,
                &key,
                topic,
                64,
            )
            .await
            .expect("subject")
        }
    }

    #[tokio::test]
    async fn recording_a_decision_twice_changes_nothing() {
        // I2 and FR-305. The primary key is the mechanism, and the return value
        // is what lets a caller tell "decided" from "already decided" without
        // making the second call an error.
        let f = fixture().await;
        let a = f.propose(f.session_a, Some("infra.db"), Some("postgresql"), "PostgreSQL.").await;
        let b = f.propose(f.session_b, Some("infra.db"), Some("mysql"), "MySQL.").await;

        let decision = |from, to| NewRelation {
            project_id: f.project,
            from,
            to,
            kind: RelationKind::Narrows,
            decided_by_session: f.session_a,
            basis: RelationBasis::ExplicitAgent,
            basis_evidence_id: None,
            rationale: Some("a documented scope exception"),
        };

        assert!(record_relation(&f.store, decision(a.memory.id, b.memory.id))
            .await
            .unwrap());
        assert!(
            !record_relation(&f.store, decision(a.memory.id, b.memory.id))
                .await
                .unwrap(),
            "the second recording wrote a row"
        );
        // Only the narrowing, once. The pair also carries the `conflicts_with`
        // the proposal path recorded automatically, which is why this counts by
        // kind rather than counting rows.
        let narrows = relations_for_project(&f.store, f.project)
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == RelationKind::Narrows)
            .count();
        assert_eq!(narrows, 1);
    }

    #[tokio::test]
    async fn a_symmetric_decision_is_one_row_whichever_way_it_is_written() {
        // D78: what makes two offline machines detecting one conflict converge
        // to one durable record rather than two facing opposite ways.
        let f = fixture().await;
        let a = f.propose(f.session_a, Some("infra.db"), Some("postgresql"), "PostgreSQL.").await;
        let b = f.propose(f.session_b, Some("infra.db"), Some("mysql"), "MySQL.").await;

        // The proposal path already recorded the conflict. Recording it again
        // from the other direction must not add a second row.
        let reversed = NewRelation {
            project_id: f.project,
            from: b.memory.id,
            to: a.memory.id,
            kind: RelationKind::ConflictsWith,
            decided_by_session: f.session_b,
            basis: RelationBasis::DeterministicRule,
            basis_evidence_id: None,
            rationale: None,
        };
        assert!(!record_relation(&f.store, reversed).await.unwrap());

        let conflicts: Vec<Relation> = relations_for_project(&f.store, f.project)
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == RelationKind::ConflictsWith)
            .collect();
        assert_eq!(conflicts.len(), 1, "{conflicts:?}");
        assert_eq!(
            (conflicts[0].from, conflicts[0].to),
            (a.memory.id.min(b.memory.id), a.memory.id.max(b.memory.id))
        );
    }

    #[tokio::test]
    async fn three_sessions_recording_the_same_thing_yield_one_answer() {
        // US1 scenario A, through the store.
        let f = fixture().await;
        let first = f
            .propose(f.session_a, Some("infra.db"), Some("postgresql"),
                     "The production database is PostgreSQL.")
            .await;
        let second = f
            .propose(f.session_b, Some("infra.db"), Some("postgresql"),
                     "the production   database is postgresql!")
            .await;
        let third = f
            .propose(Uuid::now_v7(), Some("infra.db"), Some("postgresql"),
                     "THE PRODUCTION DATABASE IS POSTGRESQL")
            .await;

        assert!(matches!(second.reconciliation, ProposalOutcome::Duplicate { .. }));
        assert!(matches!(third.reconciliation, ProposalOutcome::Duplicate { .. }));

        let read = f.subject_of("infra.db").await;
        assert_eq!(read.view.reconciliation.as_str(), "reinforced");
        assert_eq!(read.view.answers, vec![first.memory.id]);
        assert_eq!(read.view.accounting[0].distinct_origins, 3);

        // And all three remain individually retrievable with their own
        // provenance — the duplicate is dropped from the answer, never from
        // storage (FR-321).
        assert_eq!(read.members.len(), 3);
        for id in [first.memory.id, second.memory.id, third.memory.id] {
            assert!(repo::memory(&f.store, id).await.is_ok());
        }
    }

    #[tokio::test]
    async fn a_coarse_value_key_records_nothing_and_names_the_member() {
        // The false-merge path R12 closed, through the store this time.
        let f = fixture().await;
        let first = f
            .propose(f.session_a, Some("auth.strategy"), Some("jwt"),
                     "JWT uses HS256 with a shared secret.")
            .await;
        let second = f
            .propose(f.session_b, Some("auth.strategy"), Some("jwt"),
                     "JWT uses RS256 with rotating public keys.")
            .await;

        assert_eq!(
            second.reconciliation,
            ProposalOutcome::Corroborating {
                member: first.memory.id
            }
        );
        assert!(second.notes.contains(&"corroborating_member"));
        assert!(
            relations_for_project(&f.store, f.project).await.unwrap().is_empty(),
            "corroboration wrote a relation"
        );

        let read = f.subject_of("auth.strategy").await;
        assert_eq!(read.view.reconciliation.as_str(), "corroborated");
        assert_eq!(read.view.answers.len(), 2, "both statements are retained");
    }

    #[tokio::test]
    async fn incompatible_values_conflict_with_no_winner() {
        let f = fixture().await;
        let a = f.propose(f.session_a, Some("infra.db"), Some("postgresql"), "PostgreSQL.").await;
        let b = f.propose(f.session_b, Some("infra.db"), Some("cockroachdb"), "CockroachDB.").await;

        assert_eq!(
            b.reconciliation,
            ProposalOutcome::ConflictDetected {
                with: vec![a.memory.id]
            }
        );

        let read = f.subject_of("infra.db").await;
        assert_eq!(read.view.reconciliation.as_str(), "conflicted");
        assert_eq!(read.view.answers.len(), 2);
        // Neither is marked superseded to resolve it.
        assert!(read
            .members
            .iter()
            .all(|m| m.state == MemoryState::Active));
    }

    #[tokio::test]
    async fn an_unrepresentable_key_stores_the_memory_free_form() {
        // FR-312: the memory is stored regardless, and the reason is reported.
        let f = fixture().await;
        let out = f
            .propose(f.session_a, Some("데이터베이스"), None, "A claim with an unusable key.")
            .await;
        assert!(out.notes.contains(&"invalid_topic_key"));
        assert_eq!(out.reconciliation, ProposalOutcome::Created);

        let stored: Option<String> =
            sqlx::query_scalar("SELECT topic_key FROM memories WHERE id = ?1")
                .bind(out.memory.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!(stored, None);
        assert!(repo::memory(&f.store, out.memory.id).await.is_ok());
    }

    #[tokio::test]
    async fn a_value_key_without_a_topic_key_is_dropped_not_refused() {
        let f = fixture().await;
        let out = f.propose(f.session_a, None, Some("postgresql"), "A claim.").await;
        assert!(out.notes.contains(&"value_without_topic"));
        let stored: Option<String> =
            sqlx::query_scalar("SELECT value_key FROM memories WHERE id = ?1")
                .bind(out.memory.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .unwrap();
        assert_eq!(stored, None);
    }

    #[tokio::test]
    async fn an_oversized_subject_defers_rather_than_scanning() {
        // FR-474. The write completes; only the decision waits.
        let f = fixture().await;
        let key = f.project.to_string();
        for i in 0..4 {
            f.propose(Uuid::now_v7(), Some("infra.db"), Some(&format!("v{i}")), &format!("Claim {i}."))
                .await;
        }
        let out = repo::create_memory_reconciled(
            &f.store,
            NewMemory {
                project_id: f.project,
                kind: MemoryType::Fact,
                scope: MemoryScope::Project,
                scope_key: &key,
                content: "One more claim.",
                origin_session_id: f.session_b,
                local_only: false,
                evidence: &[],
                topic_key: Some("infra.db"),
                value_key: Some("v9"),
                importance: Importance::Normal,
            },
            SyncPolicy { linked: false, server_project_id: None },
            2,
        )
        .await
        .unwrap();

        assert_eq!(out.reconciliation, ProposalOutcome::Deferred);
        assert!(out.notes.contains(&"reconciliation_deferred"));
        assert!(repo::memory(&f.store, out.memory.id).await.is_ok(), "the write still completed");
    }

    #[tokio::test]
    async fn a_free_form_proposal_reconciles_against_nothing() {
        let f = fixture().await;
        f.propose(f.session_a, None, None, "The same sentence.").await;
        let second = f.propose(f.session_b, None, None, "The same sentence.").await;
        assert_eq!(second.reconciliation, ProposalOutcome::Created);
        assert!(relations_for_project(&f.store, f.project).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_subject_read_reports_when_it_hits_its_bound() {
        let f = fixture().await;
        for i in 0..5 {
            f.propose(Uuid::now_v7(), Some("infra.db"), Some(&format!("v{i}")), &format!("Claim {i}."))
                .await;
        }
        let key = f.project.to_string();
        let read = subject(&f.store, f.project, MemoryScope::Project, &key, "infra.db", 3)
            .await
            .unwrap();
        assert!(read.degraded, "the bound bound and was not reported");
        assert_eq!(read.members.len(), 3);

        let full = f.subject_of("infra.db").await;
        assert!(!full.degraded);
        assert_eq!(full.members.len(), 5);
    }

    #[tokio::test]
    async fn reinforcement_accounting_counts_origins_not_repetitions() {
        // FR-322 and FR-406: a repetition count is never an independent
        // confirmation count.
        let f = fixture().await;
        let first = f
            .propose(f.session_a, Some("infra.db"), Some("postgresql"), "PostgreSQL.")
            .await;
        // The same session says it twice, and a second session says it once.
        f.propose(f.session_a, Some("infra.db"), Some("postgresql"), "postgresql").await;
        f.propose(f.session_b, Some("infra.db"), Some("postgresql"), "  POSTGRESQL  ").await;

        let (reinforcements, origins) =
            rebuild_reinforcement(&f.store, first.memory.id).await.unwrap();
        assert_eq!(reinforcements, 2, "two duplicating decisions");
        assert_eq!(origins, 2, "but only two distinct origin sessions");
    }

    #[tokio::test]
    async fn rebuilding_supersession_reproduces_what_the_relations_imply() {
        // I7 and the rebuild equality: the Feature 001 columns are a view of the
        // relation, so discarding them and recomputing must give them back.
        let f = fixture().await;
        let old = f.propose(f.session_a, Some("infra.db"), Some("postgresql"), "PostgreSQL.").await;
        let new = f.propose(f.session_b, Some("infra.db"), Some("cockroachdb"), "CockroachDB.").await;

        record_relation(
            &f.store,
            NewRelation {
                project_id: f.project,
                from: new.memory.id,
                to: old.memory.id,
                kind: RelationKind::Supersedes,
                decided_by_session: f.session_b,
                basis: RelationBasis::ExplicitUser,
                basis_evidence_id: None,
                rationale: None,
            },
        )
        .await
        .unwrap();

        // The columns have not been touched yet, so the rebuild has work to do.
        let differed = rebuild_supersession(&f.store, f.project).await.unwrap();
        assert_eq!(differed, 1);

        let stored = repo::memory(&f.store, old.memory.id).await.unwrap();
        assert_eq!(stored.state, MemoryState::Superseded);
        assert_eq!(stored.superseded_by_id, Some(new.memory.id));

        // And it is idempotent: a second rebuild finds nothing to correct.
        assert_eq!(rebuild_supersession(&f.store, f.project).await.unwrap(), 0);

        let read = f.subject_of("infra.db").await;
        assert_eq!(read.view.reconciliation.as_str(), "settled");
        assert_eq!(read.view.answers, vec![new.memory.id]);
    }
}
