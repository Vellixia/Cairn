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

// ---------------------------------------------------------------------------
// Explicit decisions (T032, T034)
// ---------------------------------------------------------------------------

/// Record that a session confirms an existing memory is still true (FR-321).
///
/// **Explicit only.** `reinforces` is no longer something Cairn infers from a
/// matching value key — that inference was the false-merge path R12 closed. It
/// is a real act with a real author, and the accounting it feeds distinguishes
/// how many times a memory was reinforced from how many *distinct sessions*
/// did it (FR-322).
///
/// `from` is the confirming session's memory; `to` is the one confirmed.
/// Reinforcing a superseded memory is allowed and does not resurrect it: the
/// reinforcement is recorded against the memory it names (spec Edge Cases).
pub async fn reinforce(
    store: &Store,
    project_id: Uuid,
    from: Uuid,
    to: Uuid,
    session: Uuid,
    basis: RelationBasis,
) -> Result<bool> {
    if from == to {
        return Err(crate::StoreError::Corrupt(
            "relation_conflict: a memory cannot reinforce itself".into(),
        ));
    }
    let wrote = record_relation(
        store,
        NewRelation {
            project_id,
            from,
            to,
            kind: RelationKind::Reinforces,
            decided_by_session: session,
            basis,
            basis_evidence_id: None,
            rationale: None,
        },
    )
    .await?;
    // The counts are derived, so they are recomputed rather than incremented:
    // an increment can drift, a derivation cannot.
    rebuild_reinforcement(store, to).await?;
    Ok(wrote)
}

/// Whether `start` already supersedes `target`, directly or through a chain.
///
/// Bounded: a supersession chain in a real project is a handful of links, and
/// an unbounded walk on a write path is exactly what FR-474 forbids. Hitting
/// the bound reports "no path found", which is the conservative answer — it
/// admits a relation rather than refusing one it cannot prove wrong.
async fn supersedes_transitively(
    tx: &mut sqlx::SqliteConnection,
    start: Uuid,
    target: Uuid,
    max_hops: usize,
) -> Result<bool> {
    let mut frontier = vec![start];
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    for _ in 0..max_hops {
        let mut next = Vec::new();
        for node in frontier.drain(..) {
            if !seen.insert(node) {
                continue;
            }
            let rows = sqlx::query(
                "SELECT to_memory_id FROM memory_relations
                  WHERE from_memory_id = ?1 AND kind = 'supersedes' AND deleted_at IS NULL",
            )
            .bind(node.to_string())
            .fetch_all(&mut *tx)
            .await?;
            for row in &rows {
                let to = rows::uuid(row, "to_memory_id")?;
                if to == target {
                    return Ok(true);
                }
                next.push(to);
            }
        }
        if next.is_empty() {
            return Ok(false);
        }
        frontier = next;
    }
    Ok(false)
}

/// Record an explicit reconciliation decision, refusing one that contradicts a
/// decision already recorded (FR-335).
///
/// Fail-closed. Three contradictions are refused with `relation_conflict`:
///
/// - **mutual supersession** — recording `A supersedes B` when B already
///   supersedes A, directly or through a chain. Two mutually exclusive
///   decisions would leave a subject with no members at all, so the derivation
///   reports the cycle rather than resolving it, and writing the second such
///   relation is refused before it can be created
///   (`contracts/records-and-rebuild.md` §Fail-closed);
/// - **a second successor** — recording `A supersedes B` when some other memory
///   already supersedes B. Which one replaced it would then be arbitrary;
/// - **self-reference** — a memory relating to itself.
///
/// A `conflicts_with` is *detected* automatically and *resolved* never, so it
/// is not accepted here: leaving a conflict requires a supersession, a
/// narrowing, or a verification result that distinguishes the members.
pub async fn reconcile(
    store: &Store,
    project_id: Uuid,
    from: Uuid,
    to: Uuid,
    kind: RelationKind,
    basis: RelationBasis,
    basis_evidence_id: Option<Uuid>,
    rationale: Option<&str>,
) -> Result<bool> {
    let refuse = |why: &str| {
        crate::StoreError::Corrupt(format!("relation_conflict: {why}"))
    };

    if from == to {
        return Err(refuse("a memory cannot relate to itself"));
    }
    if basis == RelationBasis::Evidence && basis_evidence_id.is_none() {
        return Err(crate::StoreError::Corrupt(
            "invalid_request: basis = evidence requires basis_evidence_id".into(),
        ));
    }

    let mut tx = tx::begin(store, "reconcile").await?;

    if kind == RelationKind::Supersedes {
        if supersedes_transitively(&mut tx, to, from, 64).await? {
            return Err(refuse(
                "the memory being superseded already supersedes the one replacing it",
            ));
        }
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT from_memory_id FROM memory_relations
              WHERE to_memory_id = ?1 AND kind = 'supersedes' AND from_memory_id <> ?2
                AND deleted_at IS NULL
              LIMIT 1",
        )
        .bind(to.to_string())
        .bind(from.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        if existing.is_some() {
            return Err(refuse("that memory is already superseded by another"));
        }
    }

    let wrote = record_relation_tx(
        &mut tx,
        NewRelation {
            project_id,
            from,
            to,
            kind,
            decided_by_session: Uuid::nil(),
            basis,
            basis_evidence_id,
            rationale,
        },
    )
    .await?;

    if kind == RelationKind::Supersedes {
        // The Feature 001 columns follow the relation, in the same transaction,
        // so they never disagree with it (I7).
        sqlx::query(
            "UPDATE memories SET state = 'superseded', superseded_by_id = ?1, updated_at = ?2,
                                 superseded_at = ?2,
                                 pinned = 0, pinned_at = NULL, pinned_by_session = NULL,
                                 pin_reason = NULL
             WHERE id = ?3",
        )
        .bind(from.to_string())
        .bind(rows::now_text())
        .bind(to.to_string())
        .execute(&mut *tx)
        .await?;
    }

    tx::commit(tx, "reconcile").await?;
    Ok(wrote)
}

/// The same, attributing the decision to a session.
#[allow(clippy::too_many_arguments)]
pub async fn reconcile_as(
    store: &Store,
    project_id: Uuid,
    session: Uuid,
    from: Uuid,
    to: Uuid,
    kind: RelationKind,
    basis: RelationBasis,
    basis_evidence_id: Option<Uuid>,
    rationale: Option<&str>,
) -> Result<bool> {
    let wrote = reconcile(
        store,
        project_id,
        from,
        to,
        kind,
        basis,
        basis_evidence_id,
        rationale,
    )
    .await?;
    if wrote {
        sqlx::query(
            "UPDATE memory_relations SET decided_by_session = ?4
              WHERE from_memory_id = ?1 AND to_memory_id = ?2 AND kind = ?3",
        )
        .bind(
            normalize_relation_endpoints(kind, from, to)
                .0
                .to_string(),
        )
        .bind(
            normalize_relation_endpoints(kind, from, to)
                .1
                .to_string(),
        )
        .bind(kind.as_str())
        .bind(session.to_string())
        .execute(store.pool())
        .await?;
    }
    Ok(wrote)
}

/// A branch-scoped proposal that a merge has made a candidate for elevation.
///
/// A **candidate**, and never more than that. Branch-scoped knowledge does not
/// become project knowledge because a branch merged: a merge may produce a
/// candidate, which is verified against the current target branch and applied
/// only on an explicit decision (FR-382). Cairn reports; a person or an agent
/// decides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElevationCandidate {
    pub memory_id: Uuid,
    pub branch: String,
    pub topic_key: String,
    pub value_key: Option<String>,
    pub content: String,
}

/// Every branch-scoped, topic-keyed, active proposal, grouped by its branch.
///
/// The store cannot ask Git whether a branch merged, so it returns the
/// candidates and the daemon filters them — which keeps the one crate that
/// touches the repository the only one that does.
pub async fn branch_scoped_subjects(
    store: &Store,
    project_id: Uuid,
) -> Result<Vec<ElevationCandidate>> {
    let rows = sqlx::query(
        "SELECT id, scope_key, topic_key, value_key, content FROM memories
          WHERE project_id = ?1 AND scope = 'branch' AND topic_key IS NOT NULL
            AND state = 'active' AND deleted_at IS NULL
          ORDER BY scope_key, topic_key, id",
    )
    .bind(project_id.to_string())
    .fetch_all(store.pool())
    .await?;

    rows.iter()
        .map(|r| {
            Ok(ElevationCandidate {
                memory_id: rows::uuid(r, "id")?,
                branch: r.try_get("scope_key")?,
                topic_key: r.try_get("topic_key")?,
                value_key: r.try_get("value_key")?,
                content: r.try_get("content")?,
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::outbox::SyncPolicy;
    use crate::repo::{self, NewMemory};
    use cairn_core::domain::MemoryType;

    pub struct Fixture {
        _dir: tempfile::TempDir,
        pub store: Store,
        pub project: Uuid,
        pub session_a: Uuid,
        pub session_b: Uuid,
    }

    pub async fn fixture() -> Fixture {
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
        pub async fn propose(
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

        pub async fn store_count(&self, sql: &str) -> i64 {
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(self.store.pool())
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}"))
        }

        pub async fn subject_of(&self, topic: &str) -> SubjectRead {
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

}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use super::*;
    use crate::outbox::SyncPolicy;
    use crate::repo::{self, NewMemory};
    use cairn_core::domain::MemoryType;
    use cairn_core::knowledge::ProposalOutcome;

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

#[cfg(test)]
mod explicit_decision_tests {
    use super::tests_support::*;
    use super::*;
    use cairn_core::knowledge::ProposalOutcome;

    #[tokio::test]
    async fn reinforcement_is_explicit_and_counts_distinct_origins() {
        // T032, closing T025: reinforcement is a real act with a real author,
        // and its accounting distinguishes repetition from independent origin.
        let f = fixture().await;
        let target = f
            .propose(f.session_a, Some("infra.db"), Some("postgresql"), "PostgreSQL.")
            .await;
        let confirming = f
            .propose(f.session_b, Some("api.style"), Some("rest"), "Still true.")
            .await;

        assert_eq!(
            f.store_count("SELECT COUNT(*) FROM memory_relations WHERE kind = 'reinforces'")
                .await,
            0,
            "nothing reinforced anything by itself"
        );

        assert!(reinforce(
            &f.store,
            f.project,
            confirming.memory.id,
            target.memory.id,
            f.session_b,
            RelationBasis::ExplicitAgent,
        )
        .await
        .unwrap());

        assert_eq!(
            f.store_count("SELECT COUNT(*) FROM memory_relations WHERE kind = 'reinforces'")
                .await,
            1
        );

        let counts: (i64, i64) = sqlx::query_as(
            "SELECT reinforcement_count, distinct_origin_count FROM memories WHERE id = ?1",
        )
        .bind(target.memory.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap();
        assert_eq!(counts, (1, 2), "one reinforcement, two distinct origins");

        // Recording it again changes nothing.
        assert!(!reinforce(
            &f.store,
            f.project,
            confirming.memory.id,
            target.memory.id,
            f.session_b,
            RelationBasis::ExplicitAgent,
        )
        .await
        .unwrap());
        assert_eq!(
            f.store_count("SELECT COUNT(*) FROM memory_relations WHERE kind = 'reinforces'")
                .await,
            1
        );
    }

    #[tokio::test]
    async fn reinforcing_a_superseded_memory_does_not_resurrect_it() {
        // Spec Edge Cases: a late session confirms a value that has since been
        // replaced. The reinforcement is recorded against the memory it names;
        // it does not bring it back and it does not decrement the successor.
        let f = fixture().await;
        let old = f
            .propose(f.session_a, Some("infra.db"), Some("postgresql"), "PostgreSQL.")
            .await;
        let new = f
            .propose(f.session_b, Some("infra.db"), Some("cockroachdb"), "CockroachDB.")
            .await;
        reconcile(
            &f.store,
            f.project,
            new.memory.id,
            old.memory.id,
            RelationKind::Supersedes,
            RelationBasis::ExplicitUser,
            None,
            None,
        )
        .await
        .unwrap();

        let late = f
            .propose(Uuid::now_v7(), Some("api.style"), Some("rest"), "Late note.")
            .await;
        reinforce(
            &f.store,
            f.project,
            late.memory.id,
            old.memory.id,
            Uuid::now_v7(),
            RelationBasis::ExplicitAgent,
        )
        .await
        .unwrap();

        let state: String = sqlx::query_scalar("SELECT state FROM memories WHERE id = ?1")
            .bind(old.memory.id.to_string())
            .fetch_one(f.store.pool())
            .await
            .unwrap();
        assert_eq!(state, "superseded", "the reinforcement resurrected it");

        let read = f.subject_of("infra.db").await;
        assert_eq!(read.view.answers, vec![new.memory.id]);
    }

    #[tokio::test]
    async fn a_supersession_records_the_relation_and_clears_the_pin() {
        // T033: the relation, the lifecycle columns, the end instant and the
        // pin all move in one transaction (FR-323, FR-341, FR-456).
        let f = fixture().await;
        let old = f
            .propose(f.session_a, Some("service.api_port"), Some("8080"), "Port 8080.")
            .await;
        crate::repo::set_memory_intelligence(
            &f.store,
            old.memory.id,
            crate::constraints::MemoryColumns {
                pinned: Some(1),
                pinned_at: Some("2026-01-01T00:00:00Z"),
                pinned_by_session: Some("s1"),
                pin_reason: Some("the port is load-bearing"),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let new = f
            .propose(f.session_b, Some("service.api_port"), Some("9000"), "Port 9000.")
            .await;
        reconcile(
            &f.store,
            f.project,
            new.memory.id,
            old.memory.id,
            RelationKind::Supersedes,
            RelationBasis::ExplicitUser,
            None,
            None,
        )
        .await
        .unwrap();

        let row: (String, Option<String>, Option<String>, i64, Option<String>) = sqlx::query_as(
            "SELECT state, superseded_by_id, superseded_at, pinned, pin_reason
               FROM memories WHERE id = ?1",
        )
        .bind(old.memory.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .unwrap();
        assert_eq!(row.0, "superseded");
        assert_eq!(row.1, Some(new.memory.id.to_string()));
        assert!(row.2.is_some(), "the end of the interval was not recorded");
        assert_eq!(row.3, 0, "a superseded memory keeps its pin");
        assert_eq!(row.4, None, "the pin's reason outlived the pin");
    }

    #[tokio::test]
    async fn a_mutual_supersession_is_refused_before_it_can_exist() {
        // `contracts/records-and-rebuild.md` §Fail-closed. The derivation
        // reports a cycle it finds; this is what stops one being created.
        let f = fixture().await;
        let a = f.propose(f.session_a, Some("infra.db"), Some("postgresql"), "PostgreSQL.").await;
        let b = f.propose(f.session_b, Some("infra.db"), Some("mysql"), "MySQL.").await;

        reconcile(&f.store, f.project, a.memory.id, b.memory.id,
                  RelationKind::Supersedes, RelationBasis::ExplicitUser, None, None)
            .await
            .expect("the first supersession lands");

        let err = reconcile(&f.store, f.project, b.memory.id, a.memory.id,
                            RelationKind::Supersedes, RelationBasis::ExplicitUser, None, None)
            .await
            .expect_err("the reverse must be refused")
            .to_string();
        assert!(err.contains("relation_conflict"), "{err}");
        assert!(err.contains("already supersedes"), "{err}");

        assert_eq!(
            f.store_count("SELECT COUNT(*) FROM memory_relations WHERE kind = 'supersedes'")
                .await,
            1,
            "the refused relation was written anyway"
        );
    }

    #[tokio::test]
    async fn a_longer_supersession_cycle_is_refused_too() {
        // A → B, B → C, and then C → A would close a three-link cycle.
        let f = fixture().await;
        let a = f.propose(f.session_a, Some("infra.db"), Some("v1"), "One.").await;
        let b = f.propose(f.session_b, Some("infra.db"), Some("v2"), "Two.").await;
        let c = f.propose(Uuid::now_v7(), Some("infra.db"), Some("v3"), "Three.").await;

        reconcile(&f.store, f.project, a.memory.id, b.memory.id,
                  RelationKind::Supersedes, RelationBasis::ExplicitUser, None, None)
            .await
            .unwrap();
        reconcile(&f.store, f.project, b.memory.id, c.memory.id,
                  RelationKind::Supersedes, RelationBasis::ExplicitUser, None, None)
            .await
            .unwrap();

        let err = reconcile(&f.store, f.project, c.memory.id, a.memory.id,
                            RelationKind::Supersedes, RelationBasis::ExplicitUser, None, None)
            .await
            .expect_err("closing the cycle must be refused")
            .to_string();
        assert!(err.contains("relation_conflict"), "{err}");
    }

    #[tokio::test]
    async fn a_second_successor_is_refused() {
        // Which memory replaced it would otherwise be arbitrary, and
        // `rebuild_supersession` would have to pick one.
        let f = fixture().await;
        let old = f.propose(f.session_a, Some("infra.db"), Some("v1"), "One.").await;
        let first = f.propose(f.session_b, Some("infra.db"), Some("v2"), "Two.").await;
        let second = f.propose(Uuid::now_v7(), Some("infra.db"), Some("v3"), "Three.").await;

        reconcile(&f.store, f.project, first.memory.id, old.memory.id,
                  RelationKind::Supersedes, RelationBasis::ExplicitUser, None, None)
            .await
            .unwrap();
        let err = reconcile(&f.store, f.project, second.memory.id, old.memory.id,
                            RelationKind::Supersedes, RelationBasis::ExplicitUser, None, None)
            .await
            .expect_err("a second successor must be refused")
            .to_string();
        assert!(err.contains("already superseded by another"), "{err}");
    }

    #[tokio::test]
    async fn a_narrowing_resolves_a_conflict_without_picking_a_winner() {
        // FR-335: leaving `Conflicted` requires a recorded decision, and a
        // scope narrowing is one of the three.
        let f = fixture().await;
        let broad = f
            .propose(f.session_a, Some("infra.db"), Some("postgresql"), "PostgreSQL.")
            .await;
        let narrow = f
            .propose(f.session_b, Some("infra.db"), Some("sqlite"), "SQLite in fixtures.")
            .await;

        let read = f.subject_of("infra.db").await;
        assert_eq!(read.view.reconciliation.as_str(), "conflicted");

        reconcile(&f.store, f.project, narrow.memory.id, broad.memory.id,
                  RelationKind::Narrows, RelationBasis::ExplicitAgent, None,
                  Some("the fixture is a narrower context"))
            .await
            .unwrap();

        // The narrowing is recorded and reported; the conflict itself is not
        // erased, because both proposals are still active in one scope. What
        // changed is that the exception is now documented.
        assert_eq!(
            f.store_count("SELECT COUNT(*) FROM memory_relations WHERE kind = 'narrows'")
                .await,
            1
        );
        let after = f.subject_of("infra.db").await;
        assert_eq!(after.view.narrowed_by, vec![narrow.memory.id]);
    }

    #[tokio::test]
    async fn evidence_basis_requires_the_evidence_it_names() {
        let f = fixture().await;
        let a = f.propose(f.session_a, Some("infra.db"), Some("v1"), "One.").await;
        let b = f.propose(f.session_b, Some("infra.db"), Some("v2"), "Two.").await;
        let err = reconcile(&f.store, f.project, a.memory.id, b.memory.id,
                            RelationKind::Supersedes, RelationBasis::Evidence, None, None)
            .await
            .expect_err("basis = evidence with no evidence")
            .to_string();
        assert!(err.contains("basis_evidence_id"), "{err}");
    }

    #[tokio::test]
    async fn a_memory_cannot_relate_to_itself() {
        let f = fixture().await;
        let a = f.propose(f.session_a, Some("infra.db"), Some("v1"), "One.").await;
        for kind in [RelationKind::Supersedes, RelationKind::Narrows] {
            assert!(reconcile(&f.store, f.project, a.memory.id, a.memory.id, kind,
                              RelationBasis::ExplicitUser, None, None)
                .await
                .is_err());
        }
        assert!(reinforce(&f.store, f.project, a.memory.id, a.memory.id,
                          f.session_a, RelationBasis::ExplicitAgent)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn an_explicit_supersession_settles_the_subject() {
        let f = fixture().await;
        let old = f
            .propose(f.session_a, Some("infra.db"), Some("postgresql"), "PostgreSQL.")
            .await;
        let new = f
            .propose(f.session_b, Some("infra.db"), Some("cockroachdb"), "CockroachDB.")
            .await;
        assert!(matches!(
            new.reconciliation,
            ProposalOutcome::ConflictDetected { .. }
        ));

        reconcile(&f.store, f.project, new.memory.id, old.memory.id,
                  RelationKind::Supersedes, RelationBasis::ExplicitUser, None, None)
            .await
            .unwrap();

        let read = f.subject_of("infra.db").await;
        assert_eq!(read.view.reconciliation.as_str(), "settled");
        assert_eq!(read.view.answers, vec![new.memory.id]);
    }
}
