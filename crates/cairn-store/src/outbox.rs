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

/// The four entity types [`enqueue_global`] carries — the ones migration 7's
/// `outbox.entity_type` CHECK requires a NULL `project_id` for
/// (`0007_collaborative_global_memory.sql`).
fn is_global_entity(entity_type: OutboxEntityType) -> bool {
    matches!(
        entity_type,
        OutboxEntityType::PersonalKnowledge
            | OutboxEntityType::PersonalKnowledgeRelation
            | OutboxEntityType::TeamKnowledge
            | OutboxEntityType::TeamKnowledgeRelation
    )
}

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
        // `namespace` is the routing and backoff key migration 0007 added
        // (D426, D427). Every entity type this function enqueues is
        // project-scoped, so its namespace is `project:<project_id>` — the same
        // value 0007's backfill wrote for every pre-existing row, so a row
        // enqueued before the migration and one enqueued after are routed
        // identically. The two project-less entity types Feature 004 adds do
        // not come through here: they carry no `project_id`, which the table's
        // own CHECK requires, so they get their own enqueue path rather than a
        // nullable argument here.
        "INSERT OR IGNORE INTO outbox
            (id, project_id, server_project_id, entity_type, entity_id, operation,
             idempotency_key, payload, state, attempts, created_at, namespace)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', 0, ?9, ?10)",
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
    .bind(format!("project:{project_id}"))
    .execute(executor)
    .await?;
    Ok(true)
}

/// Enqueue a personal or team knowledge change (D426, D438, FR-486, FR-491,
/// FR-568, `contracts/sync-namespaces.md` §1, §6, §7).
///
/// Project-less, by construction: `project_id` and `server_project_id` are
/// always NULL here, which is what migration 7's outbox CHECK requires for
/// exactly these four entity types — there is no [`SyncPolicy`] gate the way
/// [`enqueue`] has one, because there is no project to be linked or not.
///
/// **The idempotency key differs from [`enqueue`]'s on purpose (§7).** It mixes
/// in `writer_id`:
///
/// ```text
/// digest("{writer_id}:{entity_type}:{entity_id}:{operation}:{digest(body)}")
/// ```
///
/// rather than `enqueue`'s `digest("{entity_type}:{entity_id}:{operation}:
/// {digest(body)}")`. Two stores that independently produce byte-identical
/// content for the same entity id — two devices proposing the same personal
/// fact before ever syncing with each other — now compute two distinct keys,
/// so both rows reach the server and are both accepted; `classify_proposal`
/// (`global-memory.md` §6) decides they are duplicates, rather than the second
/// one silently colliding with the first at the transport layer and never
/// reaching reconciliation at all. This function is the **only** place a
/// project-less row is created, so this is also the only key shape any such
/// row is ever assigned — nothing here re-keys a row that already exists, and
/// [`enqueue`]'s project-namespace rows are entirely untouched by this change.
/// Eight arguments, one past the lint's limit. The eighth is
/// `authored_by_user_id`, and collapsing it into `writer_id` is exactly the
/// conflation this function exists to keep apart: one names the device, the other
/// the account (FR-594). A struct would be these same eight values behind one
/// name, since every caller supplies all of them.
#[allow(clippy::too_many_arguments)]
pub async fn enqueue_global<'e, E>(
    executor: E,
    namespace: &SyncNamespace,
    entity_type: OutboxEntityType,
    entity_id: Uuid,
    operation: OutboxOperation,
    writer_id: Uuid,
    // The account that authored this row — not this device's `writer_id`. See
    // the `authored_by_user_id` column comment in
    // `0007_collaborative_global_memory.sql` for why the two differ and why a
    // shared `team:*` lane needs the account recorded (FR-594).
    authored_by_user_id: Uuid,
    payload: &serde_json::Value,
) -> Result<bool>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    debug_assert!(
        is_global_entity(entity_type),
        "enqueue_global called with a project-namespace entity type: {entity_type}"
    );

    let body = payload.to_string();
    let key = cairn_core::digest(&format!(
        "{writer_id}:{entity_type}:{entity_id}:{operation}:{}",
        cairn_core::digest(&body)
    ));

    sqlx::query(
        "INSERT OR IGNORE INTO outbox
            (id, project_id, server_project_id, entity_type, entity_id, operation,
             idempotency_key, payload, state, attempts, created_at, namespace,
             authored_by_user_id)
         VALUES (?1, NULL, NULL, ?2, ?3, ?4, ?5, ?6, 'pending', 0, ?7, ?8, ?9)",
    )
    .bind(new_id().to_string())
    .bind(entity_type.as_str())
    .bind(entity_id.to_string())
    .bind(operation.as_str())
    .bind(key)
    .bind(body)
    .bind(rows::now_text())
    .bind(namespace.key())
    .bind(authored_by_user_id.to_string())
    .execute(executor)
    .await?;
    Ok(true)
}

/// Re-key every outbox row on one namespace.
///
/// The rows themselves are untouched — including their `idempotency_key`, which
/// does not include the namespace and must not change: an entry that was
/// partially delivered before the re-key has to be recognised as the same entry
/// afterwards, or it applies twice (FR-562).
///
/// Its one caller re-keys a lane opened against a server that had not yet
/// reported its instance id. See `cairn_store::cursor::rename`.
pub async fn rename_namespace(store: &Store, from: &str, to: &str) -> Result<u64> {
    if from == to {
        return Ok(0);
    }
    let result = sqlx::query("UPDATE outbox SET namespace = ?2 WHERE namespace = ?1")
        .bind(from)
        .bind(to)
        .execute(store.pool())
        .await?;
    Ok(result.rows_affected())
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
                 -- Spelled out rather than left implied by the two states
                 -- below. A `blocked` row must never be claimed, and a
                 -- predicate that says so survives someone adding a third
                 -- claimable state (FR-418).
                 AND state != 'blocked'
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

/// Work this server cannot hold yet (FR-418).
///
/// Neither a failure nor a delivery. The row keeps its idempotency key and its
/// payload, records the refusal class and the capability the server had at the
/// time, and is **not** claimable — so it is retried zero times against a
/// server known to lack the capability, rather than burning a drain cycle each
/// tick on work that cannot succeed.
///
/// The attempt counter is rolled back, because claiming a row that could never
/// have been delivered is not an attempt at delivering it. Leaving it counted
/// would make `attempts` read as futile retries when there were none.
pub async fn mark_blocked(
    store: &Store,
    id: Uuid,
    reason: &str,
    at_capability: &str,
    detail: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE outbox
            SET state = 'blocked', claimed_at = NULL,
                attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END,
                blocked_reason = ?1, blocked_at_capability = ?2, last_error = ?3
          WHERE id = ?4",
    )
    .bind(reason)
    .bind(at_capability)
    // What the server actually said. `blocked_reason` is the class; this is
    // the sentence that names the missing table or column, which is what
    // someone diagnosing an unexpected hold needs.
    .bind(detail)
    .bind(id.to_string())
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Blocked rows, and the capability gap each is waiting on.
pub async fn blocked(store: &Store, project_id: Uuid) -> Result<Vec<BlockedItem>> {
    let rs = sqlx::query(
        "SELECT entity_type, entity_id, blocked_reason, blocked_at_capability, attempts
           FROM outbox
          WHERE project_id = ?1 AND state = 'blocked'
          ORDER BY created_at",
    )
    .bind(project_id.to_string())
    .fetch_all(store.pool())
    .await?;
    rs.iter()
        .map(|r| {
            Ok(BlockedItem {
                entity_type: rows::enum_val(r, "entity_type")?,
                entity_id: rows::uuid(r, "entity_id")?,
                reason: r
                    .try_get::<Option<String>, _>("blocked_reason")?
                    .unwrap_or_default(),
                at_capability: r
                    .try_get::<Option<String>, _>("blocked_at_capability")?
                    .unwrap_or_default(),
                attempts: r.try_get("attempts")?,
            })
        })
        .collect()
}

/// One item the server could not hold.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BlockedItem {
    pub entity_type: cairn_core::domain::OutboxEntityType,
    pub entity_id: Uuid,
    /// `unknown_entity_type`, `unknown_field` or `schema_older`.
    pub reason: String,
    /// What the server reported it could do when the work was refused.
    pub at_capability: String,
    /// Always zero for a row that was never retried, which is the point.
    pub attempts: i64,
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

/// Work retained for a server that cannot hold it yet.
///
/// Reported separately from `pending` and from `failed`, because it is neither:
/// counting it as pending would make the queue look stuck, and counting it as
/// failed would tell the user work was lost that is in fact waiting (FR-415).
pub async fn blocked_count(store: &Store, project_id: Uuid) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM outbox WHERE project_id = ?1 AND state = 'blocked'",
    )
    .bind(project_id.to_string())
    .fetch_one(store.pool())
    .await?)
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
// Namespace-scoped operations (D426, D427, `contracts/sync-namespaces.md` §4)
//
// `personal:*` and `team:*` rows carry no `project_id` at all, so every
// project-scoped query above needs a namespace-keyed counterpart that works
// uniformly across all three namespace kinds. For a `project:*` row the two
// predicates agree — its `namespace` column is always `project:<project_id>`,
// written by `enqueue` — so these are not a second, divergent implementation;
// they are the same claim/count/release logic reached through the column every
// row now carries instead of the one only eight of twelve entity types have.
//
// `claim`, `counts`, `blocked`, `blocked_count`, `failures`
// and `total` above are left exactly as they were: `crates/cairnd/src/
// recover.rs` and this file's own tests still call `claim(store, project_id,
// _)` directly, and changing that signature out from under them is not this
// namespace widening's job.
// ---------------------------------------------------------------------------

/// [`claim`], scoped by namespace instead of by project.
///
/// The only path that can claim a `personal:*` or `team:*` row, since neither
/// carries a `project_id` for `claim`'s predicate to match against.
/// As [`claim_namespace`], claiming only rows the given account authored.
///
/// **Held, not claimed** (FR-594). Filtering in the claim rather than after it is
/// what keeps this from being a spin: a row skipped after claiming would already
/// have had `attempts` incremented and `state` moved to `in_flight`, so every
/// drain cycle would take another attempt against a row it was never going to
/// send, and the row would eventually look like a failing delivery instead of a
/// waiting one. A row that is never claimed simply stays `pending`, reports as
/// pending, and goes out unchanged the moment its author is authenticated again.
///
/// Rows with no recorded author — every project-namespace row, and any global row
/// written before this column existed — are claimable by anyone, which preserves
/// exactly the behaviour they had.
pub async fn claim_namespace_for_author(
    store: &Store,
    namespace: &str,
    author: Uuid,
    limit: i64,
) -> Result<Vec<(Uuid, SyncItem)>> {
    claim_in(store, namespace, Some(author), limit).await
}

pub async fn claim_namespace(
    store: &Store,
    namespace: &str,
    limit: i64,
) -> Result<Vec<(Uuid, SyncItem)>> {
    claim_in(store, namespace, None, limit).await
}

async fn claim_in(
    store: &Store,
    namespace: &str,
    author: Option<Uuid>,
    limit: i64,
) -> Result<Vec<(Uuid, SyncItem)>> {
    let now = chrono::Utc::now();
    let stale_before = rows::ts_text(now - chrono::Duration::seconds(CLAIM_TIMEOUT_SECONDS));

    // `?5 IS NULL` makes the unfiltered case one statement rather than two: with
    // no author bound the predicate is constant-true and the plan is the one
    // `outbox_claimable` already serves.
    let rs = sqlx::query(
        "UPDATE outbox
            SET state = 'in_flight', claimed_at = ?1, attempts = attempts + 1
          WHERE id IN (
              SELECT id FROM outbox
               WHERE namespace = ?2
                 AND state != 'blocked'
                 AND (state = 'pending'
                      OR (state = 'in_flight'
                          AND (claimed_at IS NULL OR claimed_at < ?3)))
                 AND (?5 IS NULL
                      OR authored_by_user_id IS NULL
                      OR authored_by_user_id = ?5)
               ORDER BY created_at, id
               LIMIT ?4
          )
          RETURNING *",
    )
    .bind(rows::ts_text(now))
    .bind(namespace)
    .bind(stale_before)
    .bind(limit)
    .bind(author.map(|a| a.to_string()))
    .fetch_all(store.pool())
    .await?;

    // `RETURNING` does not promise an order; the queue is oldest-first —
    // same sort as `claim`, for the same reason.
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

/// Release every held row on one namespace whose capability the server now has.
///
/// Replaced the project-scoped `release_blocked` outright rather than sitting
/// beside it: a project's namespace is `project:<id>`, so one function covers
/// both, and two spellings of "release what is now deliverable" is one more than
/// this needs.
pub async fn release_blocked_namespace(
    store: &Store,
    namespace: &str,
    entity_types: &[OutboxEntityType],
) -> Result<u64> {
    let mut released = 0;
    for entity_type in entity_types {
        let n = sqlx::query(
            "UPDATE outbox
                SET state = 'pending', blocked_reason = NULL, blocked_at_capability = NULL
              WHERE namespace = ?1 AND state = 'blocked' AND entity_type = ?2",
        )
        .bind(namespace)
        .bind(entity_type.as_str())
        .execute(store.pool())
        .await?
        .rows_affected();
        released += n;
    }
    Ok(released)
}

/// [`counts`], scoped by namespace instead of by project.
pub async fn counts_namespace(store: &Store, namespace: &str) -> Result<(i64, i64)> {
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM outbox
          WHERE namespace = ?1 AND state IN ('pending', 'in_flight')",
    )
    .bind(namespace)
    .fetch_one(store.pool())
    .await?;
    let failed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM outbox WHERE namespace = ?1 AND state = 'failed'")
            .bind(namespace)
            .fetch_one(store.pool())
            .await?;
    Ok((pending, failed))
}

/// [`blocked_count`], scoped by namespace instead of by project.
pub async fn blocked_count_namespace(store: &Store, namespace: &str) -> Result<i64> {
    Ok(
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM outbox WHERE namespace = ?1 AND state = 'blocked'",
        )
        .bind(namespace)
        .fetch_one(store.pool())
        .await?,
    )
}

/// Every namespace this store has ever queued work for, whatever its state.
///
/// Reads the outbox rather than `sync_cursor` because a namespace can have
/// queued work before its first successful pull ever ran (a brand-new
/// `personal:*` note, written before this machine has talked to the server
/// at all) — `sync_cursor` would miss exactly that namespace until a pull
/// succeeded once, and the background worker needs to know to drain it before
/// then, not only to pull it.
pub async fn known_namespaces(store: &Store) -> Result<Vec<String>> {
    let rs: Vec<(String,)> =
        sqlx::query_as("SELECT DISTINCT namespace FROM outbox ORDER BY namespace")
            .fetch_all(store.pool())
            .await?;
    Ok(rs.into_iter().map(|(n,)| n).collect())
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

/// The memory payload a linked project actually sends (FR-413, FR-502, D66).
///
/// Reads the Feature 003 columns and the verification summary from the store,
/// because they are not on the `Memory` domain struct. One place builds the wire
/// shape, which is where the privacy boundary is enforced.
///
/// What is deliberately absent, and why:
///
/// * `content_norm_digest` — a local index, useful to nobody else;
/// * `pin_reason` — free text a session wrote about local context;
/// * `local_revision` — a task's local concurrency token, meaningless elsewhere
///   and unsound if it travelled (D80);
/// * the memory's *local* authority when it is `remote_*` — relaying a third
///   machine's authority would be a claim this machine cannot support (FR-368).
///
/// Takes a connection rather than the `Store` because every caller builds this
/// **inside the transaction that wrote the memory**. Querying the pool there
/// would wait for a connection the open transaction is holding — a deadlock the
/// single-connection in-memory store makes certain and the file-backed one makes
/// intermittent.
pub async fn memory_payload_for(
    tx: &mut sqlx::SqliteConnection,
    m: &Memory,
) -> Result<serde_json::Value> {
    let mut payload = memory_payload(m);

    let row = sqlx::query(
        "SELECT topic_key, value_key, importance, effective_from, superseded_at,
                stale_at, pinned, reinforcement_count, distinct_origin_count
           FROM memories WHERE id = ?1",
    )
    .bind(m.id.to_string())
    .fetch_optional(&mut *tx)
    .await?;

    if let (Some(row), Some(obj)) = (row.as_ref(), payload.as_object_mut()) {
        use sqlx::Row as _;
        let topic: Option<String> = row.try_get("topic_key").unwrap_or(None);
        let value: Option<String> = row.try_get("value_key").unwrap_or(None);
        obj.insert("topic_key".into(), serde_json::json!(topic));
        obj.insert("value_key".into(), serde_json::json!(value));
        obj.insert(
            "importance".into(),
            serde_json::json!(row.try_get::<String, _>("importance").ok()),
        );
        obj.insert(
            "effective_from".into(),
            serde_json::json!(row
                .try_get::<Option<String>, _>("effective_from")
                .ok()
                .flatten()),
        );
        obj.insert(
            "superseded_at".into(),
            serde_json::json!(row
                .try_get::<Option<String>, _>("superseded_at")
                .ok()
                .flatten()),
        );
        obj.insert(
            "stale_at".into(),
            serde_json::json!(row.try_get::<Option<String>, _>("stale_at").ok().flatten()),
        );
        obj.insert(
            "pinned".into(),
            serde_json::json!(row.try_get::<i64, _>("pinned").unwrap_or(0) == 1),
        );
        obj.insert(
            "reinforcement_count".into(),
            serde_json::json!(row.try_get::<i64, _>("reinforcement_count").unwrap_or(0)),
        );
        obj.insert(
            "distinct_origin_count".into(),
            serde_json::json!(row.try_get::<i64, _>("distinct_origin_count").unwrap_or(0)),
        );
    }

    // The five-key verification object. `authority` is sent only as `cairn` or
    // `attested`: a receiver derives `remote_*` for itself, because "verified
    // here" is a claim only the local machine can make.
    if let Ok(summary) = crate::evidence::summary_tx(&mut *tx, m.id).await {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(
                "verification".into(),
                serde_json::json!({
                    "state": summary.state,
                    "authority": summary.authority.and_then(|a| match a {
                        cairn_core::domain::VerificationAuthority::Cairn => Some("cairn"),
                        cairn_core::domain::VerificationAuthority::Attested => Some("attested"),
                        // Never relayed. A peer's verification is that peer's
                        // claim, and this machine cannot vouch for it.
                        _ => None,
                    }),
                    "last_verified_at": summary.last_verified_at,
                    "fact_count": summary.fact_count,
                    "basis": summary.basis,
                }),
            );
        }
    }
    Ok(payload)
}

/// One `memory_relations` row on the wire (FR-413).
///
/// `basis_evidence_id` and `rationale` are **stripped**. A peer receiving
/// `basis: "evidence"` with no identifier reads it correctly — the decision was
/// evidence-backed on another machine — and learns nothing about the evidence
/// itself.
pub fn relation_payload(
    from: Uuid,
    to: Uuid,
    kind: &str,
    decided_by_session: Uuid,
    decided_at: &str,
    basis: &str,
) -> serde_json::Value {
    serde_json::json!({
        "from_memory_id": from,
        "to_memory_id": to,
        "kind": kind,
        "decided_by_session": decided_by_session,
        "decided_at": decided_at,
        "basis": basis,
    })
}

/// One criterion on the wire.
///
/// Carries the stable id and both axes, so disjoint edits converge by identity.
/// The per-criterion `revision` is local, like the task counter, and is absent.
pub fn criterion_payload(c: &crate::criteria::Criterion) -> serde_json::Value {
    serde_json::json!({
        "id": c.id,
        "task_id": c.task_id,
        "ordinal": c.ordinal,
        "label": c.label,
        "text": c.text,
        "state": c.state,
        "verification": c.verification,
        "deleted": c.deleted,
    })
}

/// One blocker on the wire. Both ends attributed; append-only.
pub fn blocker_payload(b: &crate::criteria::Blocker) -> serde_json::Value {
    serde_json::json!({
        "id": b.id,
        "task_id": b.task_id,
        "description": b.description,
        "state": b.state,
        "opened_by_session": b.opened_by_session,
        "cleared_by_session": b.cleared_by_session,
        "deleted": b.deleted,
    })
}

/// A stable identity for a relation, for the outbox's entity id.
///
/// A relation has no id column of its own — its primary key is the endpoint
/// pair and the kind. Deriving the outbox identity from the same three values
/// keeps the enqueue idempotent: the same decision recorded twice claims the
/// same row rather than queuing a duplicate.
pub fn relation_identity(from: Uuid, to: Uuid, kind: &str) -> Uuid {
    let digest = cairn_core::digest(&format!("{from}\u{1f}{to}\u{1f}{kind}"));
    let bytes = digest.as_bytes();
    let mut raw = [0u8; 16];
    for (i, slot) in raw.iter_mut().enumerate() {
        *slot = bytes.get(i).copied().unwrap_or(0);
    }
    Uuid::from_bytes(raw)
}

/// A project's sync policy, read inside a transaction.
///
/// Lets a write path decide whether to enqueue without every caller threading a
/// policy it does not otherwise need. An unreadable project is treated as
/// unlinked: failing to enqueue is recoverable, and refusing the write is not.
pub async fn policy_for_project_tx(
    tx: &mut sqlx::SqliteConnection,
    project_id: Uuid,
) -> Result<SyncPolicy> {
    use sqlx::Row as _;
    let row = sqlx::query("SELECT linked, server_project_id FROM projects WHERE id = ?1")
        .bind(project_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
    let Some(row) = row else {
        return Ok(SyncPolicy {
            linked: false,
            server_project_id: None,
        });
    };
    let linked: i64 = row.try_get("linked").unwrap_or(0);
    let server: Option<String> = row.try_get("server_project_id").unwrap_or(None);
    Ok(SyncPolicy {
        linked: linked == 1,
        server_project_id: server.and_then(|s| Uuid::parse_str(&s).ok()),
    })
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
            handoff_pending: false,
            handoff_attempts: 0,
            handoff_error: None,
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

    // -----------------------------------------------------------------------
    // `enqueue_global` — the writer-mixed idempotency key (T096, §7)
    // -----------------------------------------------------------------------

    /// Two stores (two `writer_id`s) producing byte-identical personal content
    /// for the same entity id must not collide as one row: the whole point of
    /// mixing `writer_id` into the key is that reconciliation, not the
    /// transport layer, decides whether they are duplicates (§7, FR-491).
    #[tokio::test]
    async fn two_writers_with_identical_content_enqueue_two_distinct_rows() {
        let store = Store::open_memory().await.unwrap();
        let namespace = SyncNamespace::Personal(new_id(), new_id());
        let entity = new_id();
        let payload = serde_json::json!({ "id": entity, "content": "same fact" });

        let mut tx = crate::tx::begin(&store, "test").await.unwrap();
        enqueue_global(
            &mut *tx,
            &namespace,
            OutboxEntityType::PersonalKnowledge,
            entity,
            OutboxOperation::Upsert,
            new_id(), // writer A
            new_id(), // authored by some account
            &payload,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let mut tx = crate::tx::begin(&store, "test").await.unwrap();
        enqueue_global(
            &mut *tx,
            &namespace,
            OutboxEntityType::PersonalKnowledge,
            entity,
            OutboxOperation::Upsert,
            new_id(), // writer B — different identity, same everything else
            new_id(), // authored by some account
            &payload,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        assert_eq!(
            rows_on(&store, &namespace.key()).await,
            2,
            "two different writers' identical content must not collide on one outbox row"
        );
    }

    /// Rows queued on one namespace, for the assertions below.
    ///
    /// A test helper rather than a public accessor: `sync status` computes every
    /// per-namespace count it reports in one aggregate query
    /// (`cairnd::handlers::namespace_sync_status`), so a public function per
    /// count was a second way to ask the same question that no production
    /// surface used.
    async fn rows_on(store: &Store, namespace: &str) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM outbox WHERE namespace = ?1")
            .bind(namespace.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap()
    }

    /// The **same** writer redelivering the same content is still idempotent —
    /// mixing in `writer_id` must not turn ordinary redelivery into a pile-up.
    #[tokio::test]
    async fn the_same_writer_redelivering_identical_content_stays_idempotent() {
        let store = Store::open_memory().await.unwrap();
        let namespace = SyncNamespace::Team(new_id());
        let entity = new_id();
        let writer = new_id();
        let payload = serde_json::json!({ "id": entity, "content": "a team fact" });

        for _ in 0..3 {
            let mut tx = crate::tx::begin(&store, "test").await.unwrap();
            enqueue_global(
                &mut *tx,
                &namespace,
                OutboxEntityType::TeamKnowledge,
                entity,
                OutboxOperation::Upsert,
                writer,
                new_id(),
                &payload,
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }
        assert_eq!(rows_on(&store, &namespace.key()).await, 1);
    }

    /// `enqueue_global` rows are project-less, exactly as migration 7's CHECK
    /// requires: `project_id` and `server_project_id` are NULL, never a stray
    /// zero UUID standing in for "none".
    #[tokio::test]
    async fn enqueue_global_rows_carry_no_project_id() {
        let store = Store::open_memory().await.unwrap();
        let namespace = SyncNamespace::Team(new_id());
        let mut tx = crate::tx::begin(&store, "test").await.unwrap();
        enqueue_global(
            &mut *tx,
            &namespace,
            OutboxEntityType::TeamKnowledge,
            new_id(),
            OutboxOperation::Upsert,
            new_id(),
            new_id(),
            &serde_json::json!({}),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let (project_id, server_project_id): (Option<String>, Option<String>) =
            sqlx::query_as("SELECT project_id, server_project_id FROM outbox WHERE namespace = ?1")
                .bind(namespace.key())
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(project_id, None);
        assert_eq!(server_project_id, None);
    }

    // -----------------------------------------------------------------------
    // Namespace-scoped claim, block and release (T093, T104, T106, T107)
    // -----------------------------------------------------------------------

    /// A shared `team:*` lane holds another account's queued rows instead of
    /// letting the current account send them (FR-594).
    ///
    /// The row stays `pending` rather than being claimed and skipped: a claim
    /// increments `attempts` and moves the row to `in_flight`, so a drain that
    /// claimed and then declined would spend an attempt per cycle on a row it was
    /// never going to send, and the row would eventually read as a failing
    /// delivery rather than a waiting one.
    #[tokio::test]
    async fn a_claim_scoped_to_one_author_leaves_another_accounts_rows_pending() {
        let store = Store::open_memory().await.unwrap();
        let namespace = SyncNamespace::Team(new_id());
        let (account_a, account_b) = (new_id(), new_id());
        queue_global_authored_by(&store, &namespace, account_a, 2).await;
        queue_global_authored_by(&store, &namespace, account_b, 3).await;

        let as_b = claim_namespace_for_author(&store, &namespace.key(), account_b, 100)
            .await
            .unwrap();
        assert_eq!(as_b.len(), 3, "B claimed rows that are not B's to send");

        let (pending, _) = counts_namespace(&store, &namespace.key()).await.unwrap();
        assert_eq!(
            pending, 5,
            "A's rows must still be pending, not consumed and not failed"
        );

        // A logging back in finds its own work exactly as it left it.
        let as_a = claim_namespace_for_author(&store, &namespace.key(), account_a, 100)
            .await
            .unwrap();
        assert_eq!(as_a.len(), 2, "A's own rows were not released back to A");
    }

    /// An unfiltered claim still takes everything, so project lanes and any row
    /// written before this column existed behave exactly as before.
    #[tokio::test]
    async fn an_unscoped_claim_is_unchanged_by_the_author_column() {
        let store = Store::open_memory().await.unwrap();
        let namespace = SyncNamespace::Team(new_id());
        queue_global_authored_by(&store, &namespace, new_id(), 2).await;
        queue_global_authored_by(&store, &namespace, new_id(), 2).await;

        let all = claim_namespace(&store, &namespace.key(), 100)
            .await
            .unwrap();
        assert_eq!(all.len(), 4);
    }

    async fn queue_global_of(store: &Store, namespace: &SyncNamespace, n: usize) {
        queue_global_authored_by(store, namespace, new_id(), n).await
    }

    async fn queue_global_authored_by(
        store: &Store,
        namespace: &SyncNamespace,
        author: Uuid,
        n: usize,
    ) {
        for i in 0..n {
            let mut tx = crate::tx::begin(store, "queue_global_of").await.unwrap();
            enqueue_global(
                &mut *tx,
                namespace,
                OutboxEntityType::TeamKnowledge,
                new_id(),
                OutboxOperation::Upsert,
                new_id(),
                author,
                &serde_json::json!({ "i": i }),
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }
    }

    /// A blocked or busy `personal:*` namespace must never surface another
    /// namespace's rows — the storage-level half of "one namespace's state
    /// never delays another's" (Invariant 2, FR-488).
    #[tokio::test]
    async fn claim_namespace_never_returns_another_namespaces_rows() {
        let (_dir, store) = file_store().await;
        let team = SyncNamespace::Team(new_id());
        let personal = SyncNamespace::Personal(new_id(), new_id());
        queue_global_of(&store, &team, 3).await;
        queue_global_of(&store, &personal, 2).await;

        let team_claimed = claim_namespace(&store, &team.key(), 10).await.unwrap();
        assert_eq!(team_claimed.len(), 3);

        let personal_claimed = claim_namespace(&store, &personal.key(), 10).await.unwrap();
        assert_eq!(
            personal_claimed.len(),
            2,
            "personal:* must still see its own rows after team:* drained"
        );
    }

    /// T108 (FR-502, FR-562): every namespace's unfinished claims release in
    /// **one** unscoped pass, so a `team:*` claim interrupted mid-flight never
    /// waits on `project:*` or `personal:*` being released first.
    #[tokio::test]
    async fn release_all_claims_covers_every_namespace_in_one_pass() {
        let (_dir, store) = file_store().await;
        let project = queue_of(&store, 2).await; // project:<project> via `enqueue`
        let team = SyncNamespace::Team(new_id());
        let personal = SyncNamespace::Personal(new_id(), new_id());
        queue_global_of(&store, &team, 2).await;
        queue_global_of(&store, &personal, 2).await;

        assert_eq!(claim(&store, project, 10).await.unwrap().len(), 2);
        assert_eq!(
            claim_namespace(&store, &team.key(), 10)
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            claim_namespace(&store, &personal.key(), 10)
                .await
                .unwrap()
                .len(),
            2
        );

        // All three namespaces' claims come back in the single unscoped release
        // `cairnd::recover::release_abandoned_claims` runs once at daemon start —
        // no per-namespace ordering, no namespace left waiting on another.
        assert_eq!(release_all_claims(&store).await.unwrap(), 6);

        assert_eq!(claim(&store, project, 10).await.unwrap().len(), 2);
        assert_eq!(
            claim_namespace(&store, &team.key(), 10)
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            claim_namespace(&store, &personal.key(), 10)
                .await
                .unwrap()
                .len(),
            2
        );
    }

    /// The capability release/re-probe cycle (T106, T107, §11a), at the
    /// storage layer: a `blocked` row releases to `pending` with its **original**
    /// idempotency key intact, and a namespace still holding an unsupported
    /// entity type stays blocked rather than being swept up by an unrelated
    /// release.
    #[tokio::test]
    async fn release_blocked_namespace_preserves_the_original_idempotency_key() {
        let (_dir, store) = file_store().await;
        let namespace = SyncNamespace::Team(new_id());
        queue_global_of(&store, &namespace, 1).await;
        let (id, item) = claim_namespace(&store, &namespace.key(), 10)
            .await
            .unwrap()
            .remove(0);
        let original_key = item.idempotency_key.clone();

        mark_blocked(
            &store,
            id,
            "unknown_entity_type",
            "schema=2;capabilities=",
            "the server does not know team_knowledge",
        )
        .await
        .unwrap();
        assert_eq!(
            blocked_count_namespace(&store, &namespace.key())
                .await
                .unwrap(),
            1
        );

        // Releasing a capability this row does not need leaves it blocked.
        let released =
            release_blocked_namespace(&store, &namespace.key(), &[OutboxEntityType::Memory])
                .await
                .unwrap();
        assert_eq!(
            released, 0,
            "an unrelated capability must not release this row"
        );
        assert_eq!(
            blocked_count_namespace(&store, &namespace.key())
                .await
                .unwrap(),
            1
        );

        // Releasing the capability it was actually waiting on frees it, under
        // the same key it was blocked with.
        let released =
            release_blocked_namespace(&store, &namespace.key(), &[OutboxEntityType::TeamKnowledge])
                .await
                .unwrap();
        assert_eq!(released, 1);
        assert_eq!(
            blocked_count_namespace(&store, &namespace.key())
                .await
                .unwrap(),
            0
        );

        let (_, released_item) = claim_namespace(&store, &namespace.key(), 10)
            .await
            .unwrap()
            .remove(0);
        assert_eq!(
            released_item.idempotency_key, original_key,
            "a released item must keep the key it was blocked with, so delivery stays exactly-once"
        );
    }

    /// A namespace holding both a permanently `failed` row and an ordinary
    /// `pending` one must still deliver the pending one — the storage-layer
    /// half of "one bad record does not stall the batch" (§4a).
    #[tokio::test]
    async fn a_permanently_failed_row_does_not_block_the_rest_of_its_namespace() {
        let (_dir, store) = file_store().await;
        let namespace = SyncNamespace::Personal(new_id(), new_id());
        queue_global_of(&store, &namespace, 2).await;

        let claimed = claim_namespace(&store, &namespace.key(), 10).await.unwrap();
        assert_eq!(claimed.len(), 2);
        let (bad, _) = claimed[0];
        let (good, _) = claimed[1];

        // An ingest content refusal (422, permanent): `mark_failed`, never
        // `mark_blocked` — it must not enter `blocked` (§4a, FR-581).
        mark_failed(&store, bad, "content_rejected: absolute_path")
            .await
            .unwrap();
        mark_delivered(&store, good).await.unwrap();

        let (pending, failed) = counts_namespace(&store, &namespace.key()).await.unwrap();
        assert_eq!((pending, failed), (0, 1));
        assert_eq!(
            blocked_count_namespace(&store, &namespace.key())
                .await
                .unwrap(),
            0,
            "an ingest refusal must never enter the blocked state"
        );
    }

    #[tokio::test]
    async fn known_namespaces_lists_every_distinct_namespace_with_queued_work() {
        let (_dir, store) = file_store().await;
        let project = queue_of(&store, 1).await;
        let team = SyncNamespace::Team(new_id());
        queue_global_of(&store, &team, 1).await;

        let known = known_namespaces(&store).await.unwrap();
        assert!(known.contains(&format!("project:{project}")));
        assert!(known.contains(&team.key()));
    }
}
