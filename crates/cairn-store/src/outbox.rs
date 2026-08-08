//! The transactional outbox (D9, FR-053, FR-055, FR-056).
//!
//! Every syncable change writes an outbox row in the same transaction as the
//! change, so a crash cannot lose it and a redelivery cannot double-apply it.
//!
//! There is no observation entity type. A payload carrying observation content
//! cannot be constructed here, which is what makes "raw observations never
//! sync" a property of the schema rather than a promise (SC-010).

use crate::{rows, Result, Store};
use cairn_core::domain::*;
use cairn_core::wire::{SyncFailure, SyncItem};
use sqlx::{Row, Sqlite};
use uuid::Uuid;

/// Whether a project may enqueue at all.
#[derive(Debug, Clone, Copy)]
pub struct SyncPolicy {
    pub linked: bool,
    pub server_project_id: Option<Uuid>,
}

impl SyncPolicy {
    pub fn from_project(p: &Project) -> Self {
        Self {
            linked: p.linked,
            server_project_id: p.server_project_id,
        }
    }
    /// Unlinked projects never produce a row (FR-053).
    pub fn target(&self) -> Option<Uuid> {
        if self.linked {
            self.server_project_id
        } else {
            None
        }
    }
}

/// Enqueue one change inside the caller's transaction.
///
/// A no-op when the project is not linked.
pub async fn enqueue<'e, E>(
    executor: E,
    policy: SyncPolicy,
    project_id: Uuid,
    entity_type: OutboxEntityType,
    entity_id: Uuid,
    operation: OutboxOperation,
    payload: &serde_json::Value,
) -> Result<bool>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let Some(server_project_id) = policy.target() else {
        return Ok(false);
    };

    let body = payload.to_string();
    let key = cairn_core::digest(&format!(
        "{entity_type}:{entity_id}:{operation}:{}",
        cairn_core::digest(&body)
    ));

    sqlx::query(
        "INSERT OR IGNORE INTO outbox
            (id, project_id, server_project_id, entity_type, entity_id, operation,
             idempotency_key, payload, state, attempts, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', 0, ?9)",
    )
    .bind(new_id().to_string())
    .bind(project_id.to_string())
    .bind(server_project_id.to_string())
    .bind(entity_type.as_str())
    .bind(entity_id.to_string())
    .bind(operation.as_str())
    .bind(key)
    .bind(body)
    .bind(rows::now_text())
    .execute(executor)
    .await?;
    Ok(true)
}

/// How long a claim may sit unacknowledged before another drainer may take it.
///
/// A drainer that dies mid-send leaves rows `in_flight` with nothing left to
/// acknowledge them. Rather than a lease service or a liveness protocol, the
/// claim simply expires. Sixty seconds is comfortably longer than the sync
/// client's own twenty-second request timeout, so a drainer merely waiting on a
/// slow server is never overtaken by one that assumes it died (FR-056, FR-059).
pub const CLAIM_TIMEOUT_SECONDS: i64 = 60;

/// Claim up to `limit` deliverable rows for this drainer, oldest first.
///
/// A single `UPDATE … RETURNING` is the whole mechanism. SQLite serializes
/// writers, so the statement that moves rows to `in_flight` is atomic against
/// every other connection: the background worker and `cairn sync now` receive
/// disjoint sets, and neither can send a row the other already owns. That is
/// what stops one drainer's delivery from arriving as the other's duplicate —
/// and, before the server made duplicates safe, as a false permanent failure
/// (FR-056, SC-009).
///
/// Eligible rows are those still `pending`, plus rows whose claim has gone
/// stale: an interrupted send returns to the queue rather than stranding a row
/// forever.
pub async fn claim(store: &Store, project_id: Uuid, limit: i64) -> Result<Vec<(Uuid, SyncItem)>> {
    let now = chrono::Utc::now();
    let stale_before = rows::ts_text(now - chrono::Duration::seconds(CLAIM_TIMEOUT_SECONDS));

    let rs = sqlx::query(
        "UPDATE outbox
            SET state = 'in_flight', claimed_at = ?1, attempts = attempts + 1
          WHERE id IN (
              SELECT id FROM outbox
               WHERE project_id = ?2
                 AND (state = 'pending'
                      OR (state = 'in_flight'
                          AND (claimed_at IS NULL OR claimed_at < ?3)))
               ORDER BY created_at, id
               LIMIT ?4
          )
          RETURNING *",
    )
    .bind(rows::ts_text(now))
    .bind(project_id.to_string())
    .bind(stale_before)
    .bind(limit)
    .fetch_all(store.pool())
    .await?;

    // `RETURNING` does not promise an order; the queue is oldest-first.
    let mut claimed = rs
        .iter()
        .map(|r| {
            let payload_raw: String = r.try_get("payload")?;
            let created_at: String = r.try_get("created_at")?;
            let id = rows::uuid(r, "id")?;
            Ok((
                created_at,
                id,
                SyncItem {
                    idempotency_key: r.try_get("idempotency_key")?,
                    entity_type: rows::enum_val(r, "entity_type")?,
                    entity_id: rows::uuid(r, "entity_id")?,
                    operation: rows::enum_val(r, "operation")?,
                    payload: serde_json::from_str(&payload_raw).unwrap_or(serde_json::Value::Null),
                },
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    claimed.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));

    Ok(claimed
        .into_iter()
        .map(|(_, id, item)| (id, item))
        .collect())
}

/// Return every claimed row to the queue.
///
/// Called once at daemon start, when nothing is draining yet: an `in_flight`
/// row there belongs to a run that is already gone, and would otherwise wait
/// out `CLAIM_TIMEOUT_SECONDS` for no reason.
pub async fn release_all_claims(store: &Store) -> Result<u64> {
    let result = sqlx::query(
        "UPDATE outbox SET state = 'pending', claimed_at = NULL WHERE state = 'in_flight'",
    )
    .execute(store.pool())
    .await?;
    Ok(result.rows_affected())
}

pub async fn mark_delivered(store: &Store, id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE outbox SET state = 'delivered', delivered_at = ?1, claimed_at = NULL
         WHERE id = ?2",
    )
    .bind(rows::now_text())
    .bind(id.to_string())
    .execute(store.pool())
    .await?;
    Ok(())
}

/// A permanent rejection. Surfaced with its identity rather than retried
/// forever in silence (FR-058).
pub async fn mark_failed(store: &Store, id: Uuid, error: &str) -> Result<()> {
    sqlx::query(
        "UPDATE outbox SET state = 'failed', last_error = ?1, claimed_at = NULL
         WHERE id = ?2",
    )
    .bind(error)
    .bind(id.to_string())
    .execute(store.pool())
    .await?;
    Ok(())
}

/// A transient failure: release the claim so the next drain retries it.
///
/// The attempt was already counted when the row was claimed.
pub async fn mark_retryable(store: &Store, id: Uuid, error: &str) -> Result<()> {
    sqlx::query(
        "UPDATE outbox SET state = 'pending', claimed_at = NULL, last_error = ?1
         WHERE id = ?2",
    )
    .bind(error)
    .bind(id.to_string())
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Queue depth and permanent failures.
///
/// A claimed row is still undelivered work, so `in_flight` counts as pending:
/// FR-058 reports what has not reached the server, not what happens to be
/// unclaimed at this instant.
pub async fn counts(store: &Store, project_id: Uuid) -> Result<(i64, i64)> {
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM outbox
          WHERE project_id = ?1 AND state IN ('pending', 'in_flight')",
    )
    .bind(project_id.to_string())
    .fetch_one(store.pool())
    .await?;
    let failed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM outbox WHERE project_id = ?1 AND state = 'failed'",
    )
    .bind(project_id.to_string())
    .fetch_one(store.pool())
    .await?;
    Ok((pending, failed))
}

pub async fn failures(store: &Store, project_id: Uuid) -> Result<Vec<SyncFailure>> {
    let rs = sqlx::query(
        "SELECT entity_type, entity_id, last_error FROM outbox
         WHERE project_id = ?1 AND state = 'failed' ORDER BY created_at",
    )
    .bind(project_id.to_string())
    .fetch_all(store.pool())
    .await?;
    rs.iter()
        .map(|r| {
            Ok(SyncFailure {
                entity_type: rows::enum_val(r, "entity_type")?,
                entity_id: rows::uuid(r, "entity_id")?,
                error: r
                    .try_get::<Option<String>, _>("last_error")?
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// Total queued rows for a project, whatever their state.
pub async fn total(store: &Store, project_id: Uuid) -> Result<i64> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM outbox WHERE project_id = ?1")
        .bind(project_id.to_string())
        .fetch_one(store.pool())
        .await?;
    Ok(n)
}

// ---------------------------------------------------------------------------
// Payload builders — the allowlist, in code (FR-055)
// ---------------------------------------------------------------------------

pub fn project_payload(p: &Project) -> serde_json::Value {
    serde_json::json!({
        "id": p.server_project_id,
        "name": p.name,
        "repository_remote": p.repository_remote,
    })
}

pub fn task_payload(t: &Task) -> serde_json::Value {
    serde_json::json!({
        "id": t.id,
        "title": t.title,
        "goal": t.goal,
        "acceptance_criteria": t.acceptance_criteria,
        "status": t.status,
        "created_at": t.created_at,
        "updated_at": t.updated_at,
    })
}

/// Minimal session provenance. `worktree_path`, `agent_session_key`,
/// `daemon_run_id` and `last_event_at` are local-only and never appear here.
pub fn session_payload(s: &Session) -> serde_json::Value {
    serde_json::json!({
        "id": s.id,
        "task_id": s.task_id,
        "agent": s.agent,
        "branch": s.branch,
        "commit_sha": s.commit_sha,
        "previous_session_id": s.previous_session_id,
        "status": s.status,
        "started_at": s.started_at,
        "ended_at": s.ended_at,
        "end_reason": s.end_reason,
    })
}

/// Memory with provenance *references* only — identifiers and a count. The
/// observations themselves stay on this machine (FR-055).
pub fn memory_payload(m: &Memory) -> serde_json::Value {
    serde_json::json!({
        "id": m.id,
        "type": m.kind,
        "scope": m.scope,
        "scope_key": m.scope_key,
        "content": m.content,
        "state": m.state,
        "superseded_by_id": m.superseded_by_id,
        "provenance": {
            "session_id": m.origin_session_id,
            "observation_ids": m.evidence.iter().map(|e| e.observation_id).collect::<Vec<_>>(),
            "evidence_count": m.evidence.len(),
        },
        "created_at": m.created_at,
        "updated_at": m.updated_at,
    })
}

pub fn handoff_payload(h: &Handoff) -> serde_json::Value {
    serde_json::json!({
        "id": h.id,
        "session_id": h.session_id,
        "trigger": h.trigger,
        "goal": h.goal,
        "progress": h.progress,
        "completed_work": h.completed_work,
        "remaining_work": h.remaining_work,
        "changed_files": h.changed_files,
        "decisions": h.decisions,
        "failures": h.failures,
        "tests_executed": h.tests_executed,
        "repository_state": h.repository_state,
        "next_step": h.next_step,
        "agent_note": h.agent_note,
        "evidence": { "observation_ids": h.evidence, "evidence_count": h.evidence.len() },
        "created_at": h.created_at,
    })
}

/// A tombstone. Content is already cleared locally; the server clears its copy.
pub fn delete_payload(entity_id: Uuid) -> serde_json::Value {
    serde_json::json!({ "id": entity_id, "deleted": true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::*;
    use chrono::Utc;

    fn sample_session() -> Session {
        Session {
            id: new_id(),
            project_id: new_id(),
            task_id: None,
            user_id: new_id(),
            agent: "claude-code".into(),
            branch: "main".into(),
            commit_sha: Some("abc".into()),
            worktree_path: "/Users/someone/secret-project".into(),
            agent_session_key: "private-key".into(),
            previous_session_id: None,
            status: SessionStatus::Completed,
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            last_event_at: Utc::now(),
            last_turn_ended_at: Some(Utc::now()),
            daemon_run_id: new_id(),
            end_reason: Some("clear".into()),
            deleted_at: None,
        }
    }

    #[test]
    fn session_payload_omits_every_local_only_field() {
        let s = sample_session();
        let text = session_payload(&s).to_string();
        for field in cairn_core::wire::REJECTED_SESSION_FIELDS {
            assert!(!text.contains(field), "{field} leaked into a sync payload");
        }
        assert!(!text.contains("secret-project"), "worktree path leaked");
    }

    #[test]
    fn memory_payload_carries_references_not_observation_content() {
        let m = Memory {
            id: new_id(),
            project_id: new_id(),
            kind: MemoryType::Fact,
            scope: MemoryScope::Project,
            scope_key: "p".into(),
            content: "a fact".into(),
            state: MemoryState::Active,
            superseded_by_id: None,
            origin_session_id: new_id(),
            local_only: false,
            evidence: vec![EvidenceRef {
                observation_id: new_id(),
                content_digest: "d".into(),
                deleted: false,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        let v = memory_payload(&m);
        assert_eq!(v["provenance"]["evidence_count"], 1);
        assert!(v["provenance"]["observation_ids"].is_array());
        // No place for a summary, path, command or details to live.
        let text = v.to_string();
        for field in ["\"summary\"", "\"path\"", "\"command\"", "\"details\""] {
            assert!(!text.contains(field), "{field} present in memory payload");
        }
    }

    #[tokio::test]
    async fn an_unlinked_project_never_produces_a_row() {
        let store = Store::open_memory().await.unwrap();
        let p = ensure_project(&store, "/tmp/u/.git", "u", None)
            .await
            .unwrap();
        let policy = SyncPolicy::from_project(&p);
        assert!(!policy.linked);

        let mut tx = crate::tx::begin(&store, "test").await.unwrap();
        let enqueued = enqueue(
            &mut *tx,
            policy,
            p.id,
            OutboxEntityType::Memory,
            new_id(),
            OutboxOperation::Upsert,
            &serde_json::json!({}),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        assert!(!enqueued);
        assert_eq!(total(&store, p.id).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn linked_project_enqueues_and_redelivery_is_idempotent() {
        let store = Store::open_memory().await.unwrap();
        let p = ensure_project(&store, "/tmp/l/.git", "l", None)
            .await
            .unwrap();
        let server_id = new_id();
        let p = link_project(&store, p.id, server_id).await.unwrap();
        let policy = SyncPolicy::from_project(&p);
        let entity = new_id();
        let payload = serde_json::json!({"id": entity, "content": "x"});

        for _ in 0..3 {
            let mut tx = crate::tx::begin(&store, "test").await.unwrap();
            enqueue(
                &mut *tx,
                policy,
                p.id,
                OutboxEntityType::Memory,
                entity,
                OutboxOperation::Upsert,
                &payload,
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }
        // The idempotency key is content-derived, so the same change enqueues once.
        assert_eq!(total(&store, p.id).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn unlinking_drops_queued_work() {
        let store = Store::open_memory().await.unwrap();
        let p = ensure_project(&store, "/tmp/d/.git", "d", None)
            .await
            .unwrap();
        let p = link_project(&store, p.id, new_id()).await.unwrap();
        let mut tx = crate::tx::begin(&store, "test").await.unwrap();
        enqueue(
            &mut *tx,
            SyncPolicy::from_project(&p),
            p.id,
            OutboxEntityType::Task,
            new_id(),
            OutboxOperation::Upsert,
            &serde_json::json!({"a": 1}),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(total(&store, p.id).await.unwrap(), 1);

        unlink_project(&store, p.id).await.unwrap();
        assert_eq!(total(&store, p.id).await.unwrap(), 0);
    }

    // -----------------------------------------------------------------------
    // Claiming (H-A): two drainers, one queue
    // -----------------------------------------------------------------------

    /// A store on disk, because the claim is only interesting across real
    /// connections — the in-memory store holds a single one.
    async fn file_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open(&dir.path().join("cairn.sqlite3"))
            .await
            .expect("store");
        (dir, store)
    }

    async fn queue_of(store: &Store, n: usize) -> Uuid {
        let p = ensure_project(store, "/tmp/claim/.git", "claim", None)
            .await
            .unwrap();
        let p = link_project(store, p.id, new_id()).await.unwrap();
        let policy = SyncPolicy::from_project(&p);
        for i in 0..n {
            let mut tx = crate::tx::begin(store, "queue_of").await.unwrap();
            enqueue(
                &mut *tx,
                policy,
                p.id,
                OutboxEntityType::Memory,
                new_id(),
                OutboxOperation::Upsert,
                &serde_json::json!({ "i": i }),
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }
        p.id
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_drainers_never_receive_the_same_row() {
        // The defect this exists to stop: the background worker and
        // `cairn sync now` both selecting the same pending rows, delivering
        // them at once, and turning a duplicate into a permanent failure.
        let (_dir, store) = file_store().await;
        let store = std::sync::Arc::new(store);
        let project = queue_of(&store, 60).await;

        let drainers: Vec<_> = (0..6)
            .map(|_| {
                let store = std::sync::Arc::clone(&store);
                tokio::spawn(async move {
                    let mut mine = Vec::new();
                    loop {
                        let batch = claim(&store, project, 7).await.expect("claim");
                        if batch.is_empty() {
                            break;
                        }
                        for (id, _) in batch {
                            mine.push(id);
                            // What a drainer does once the server has it.
                            mark_delivered(&store, id).await.expect("deliver");
                        }
                    }
                    mine
                })
            })
            .collect();

        let mut claimed = Vec::new();
        for drainer in drainers {
            claimed.extend(drainer.await.expect("drainer finishes"));
        }

        let distinct: std::collections::HashSet<_> = claimed.iter().copied().collect();
        assert_eq!(
            claimed.len(),
            distinct.len(),
            "a row was handed to two drainers at once"
        );
        assert_eq!(distinct.len(), 60, "every row must be claimed exactly once");
        assert_eq!(
            counts(&store, project).await.unwrap(),
            (0, 0),
            "nothing pending, nothing failed"
        );
    }

    #[tokio::test]
    async fn a_claimed_row_is_invisible_to_another_drainer_but_still_counted() {
        let (_dir, store) = file_store().await;
        let project = queue_of(&store, 3).await;

        assert_eq!(claim(&store, project, 10).await.unwrap().len(), 3);
        assert!(
            claim(&store, project, 10).await.unwrap().is_empty(),
            "a claimed row has an owner"
        );
        assert_eq!(
            counts(&store, project).await.unwrap(),
            (3, 0),
            "claimed work has not arrived yet, so it is still pending"
        );
    }

    #[tokio::test]
    async fn an_abandoned_claim_returns_to_the_queue_once_it_goes_stale() {
        let (_dir, store) = file_store().await;
        let project = queue_of(&store, 3).await;
        assert_eq!(claim(&store, project, 10).await.unwrap().len(), 3);

        // The owner dies before acknowledging anything. Age the claim rather
        // than waiting out the timeout for real.
        let abandoned =
            rows::ts_text(Utc::now() - chrono::Duration::seconds(CLAIM_TIMEOUT_SECONDS + 1));
        sqlx::query("UPDATE outbox SET claimed_at = ?1 WHERE state = 'in_flight'")
            .bind(&abandoned)
            .execute(store.pool())
            .await
            .unwrap();

        assert_eq!(
            claim(&store, project, 10).await.unwrap().len(),
            3,
            "an interrupted send must not strand a row forever"
        );
    }

    #[tokio::test]
    async fn a_transient_failure_returns_the_row_to_the_queue_at_once() {
        let (_dir, store) = file_store().await;
        let project = queue_of(&store, 2).await;
        for (id, _) in claim(&store, project, 10).await.unwrap() {
            mark_retryable(&store, id, "server unreachable")
                .await
                .unwrap();
        }
        assert_eq!(
            claim(&store, project, 10).await.unwrap().len(),
            2,
            "a transient failure is not a lost row"
        );
    }

    #[tokio::test]
    async fn daemon_start_releases_claims_left_by_a_previous_run() {
        let (_dir, store) = file_store().await;
        let project = queue_of(&store, 4).await;
        assert_eq!(claim(&store, project, 2).await.unwrap().len(), 2);

        assert_eq!(release_all_claims(&store).await.unwrap(), 2);
        assert_eq!(
            claim(&store, project, 10).await.unwrap().len(),
            4,
            "a previous run's claims are ours again at start"
        );
    }

    #[tokio::test]
    async fn a_permanently_failed_row_is_never_claimed_again() {
        // `failed` stays reserved for genuine permanent rejection (FR-058).
        let (_dir, store) = file_store().await;
        let project = queue_of(&store, 1).await;
        let (id, _) = claim(&store, project, 10).await.unwrap().remove(0);
        mark_failed(&store, id, "`summary` is local-only")
            .await
            .unwrap();

        assert!(claim(&store, project, 10).await.unwrap().is_empty());
        assert_eq!(counts(&store, project).await.unwrap(), (0, 1));
    }
}
