//! Sync ingest and read-back (FR-055, FR-056, contracts/server-api.md).
//!
//! Two rules make this endpoint the boundary the privacy model depends on:
//! every item is applied at most once, keyed by its idempotency key; and any
//! item carrying observation content — or a local-only session field — is
//! rejected outright.

use crate::auth::{self, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

/// Fields that would carry observation content. Never accepted (FR-055).
const FORBIDDEN_OBSERVATION_FIELDS: &[&str] = &[
    "summary",
    "path",
    "command",
    "details",
    "outcome",
    "exit_code",
    "observations",
];

/// Session fields that are local-only (contracts/server-api.md).
const FORBIDDEN_SESSION_FIELDS: &[&str] = &[
    "worktree_path",
    "agent_session_key",
    "daemon_run_id",
    "last_event_at",
    "last_turn_ended_at",
];

#[derive(Debug, Deserialize)]
pub struct SyncItem {
    pub idempotency_key: String,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub operation: String,
    pub payload: Value,
}

#[derive(Debug, Deserialize)]
pub struct SyncBatchBody {
    pub project_id: Uuid,
    pub items: Vec<SyncItem>,
}

pub async fn sync_batch(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<SyncBatchBody>,
) -> ApiResult<Json<Value>> {
    // A batch for an unknown project is rejected: the daemon must link first,
    // so a project is never created implicitly (contracts/server-api.md).
    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM projects WHERE id = $1")
        .bind(body.project_id)
        .fetch_optional(&state.pool)
        .await?;
    if exists.is_none() {
        return Err(ApiError::not_found("unknown project; link before syncing"));
    }
    auth::require_member(&state.pool, body.project_id, user.id).await?;

    let mut results = Vec::with_capacity(body.items.len());
    for item in &body.items {
        // Items are applied independently: one rejection does not fail a batch.
        match apply_item(&state.pool, body.project_id, user.id, item).await {
            Ok(status) => {
                results.push(json!({ "idempotency_key": item.idempotency_key, "status": status }))
            }
            // `rejected` is permanent, and the daemon treats it as permanent: it
            // stops retrying and surfaces the item as failed. A storage fault is
            // not permanent and must never wear that label, so the item is left
            // out of the results instead. The daemon reads a missing result as
            // "no answer for this item" and retries it (contracts/server-api.md).
            Err(e) if e.status.is_server_error() => tracing::warn!(
                idempotency_key = %item.idempotency_key,
                error = %e.message,
                "sync item deferred after a server-side failure"
            ),
            Err(e) => results.push(json!({
                "idempotency_key": item.idempotency_key,
                "status": "rejected",
                "error": { "code": e.code, "message": e.message },
            })),
        }
    }
    Ok(Json(json!({ "results": results })))
}

async fn apply_item(
    pool: &PgPool,
    project_id: Uuid,
    user_id: Uuid,
    item: &SyncItem,
) -> Result<&'static str, ApiError> {
    reject_forbidden_fields(item)?;

    // Applied at most once, and the claim on the key is what decides it.
    //
    // Reading the key and then inserting it would leave a window in which two
    // concurrent deliveries of the same key both read "unseen": both would
    // apply, and the loser's insert would fail on the primary key and surface
    // as a rejection of a perfectly valid item. `ON CONFLICT DO NOTHING` closes
    // that window — the second delivery blocks on the first, then finds nothing
    // inserted and reports `duplicate` (FR-056, SC-009).
    //
    // The claim is taken inside the same transaction as the change, so an item
    // that fails to apply releases its key and can be retried.
    let mut tx = pool.begin().await?;
    let claimed = sqlx::query(
        "INSERT INTO sync_state (idempotency_key, project_id, entity_type, entity_id)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (idempotency_key) DO NOTHING",
    )
    .bind(&item.idempotency_key)
    .bind(project_id)
    .bind(&item.entity_type)
    .bind(item.entity_id)
    .execute(&mut *tx)
    .await?;
    if claimed.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok("duplicate");
    }

    match (item.entity_type.as_str(), item.operation.as_str()) {
        ("project", "upsert") => upsert_project(&mut tx, project_id, item).await?,
        ("task", "upsert") => upsert_task(&mut tx, project_id, item).await?,
        ("session", "upsert") => upsert_session(&mut tx, project_id, user_id, item).await?,
        ("memory", "upsert") => upsert_memory(&mut tx, project_id, item).await?,
        ("handoff", "upsert") => upsert_handoff(&mut tx, project_id, item).await?,
        (entity, "delete") => tombstone(&mut tx, entity, item.entity_id).await?,
        (entity, op) => {
            return Err(ApiError::invalid(format!("unsupported {entity}/{op}")));
        }
    }

    tx.commit().await?;
    Ok("applied")
}

/// The allowlist enforced on the wire.
fn reject_forbidden_fields(item: &SyncItem) -> Result<(), ApiError> {
    if item.entity_type == "observation" || item.entity_type == "observation_ref" {
        return Err(ApiError::invalid(
            "observations are local; the server does not accept them",
        ));
    }
    let Some(object) = item.payload.as_object() else {
        return Ok(());
    };

    for field in FORBIDDEN_OBSERVATION_FIELDS {
        if object.contains_key(*field) {
            return Err(ApiError::invalid(format!(
                "`{field}` carries observation content, which never leaves the machine"
            )));
        }
    }
    if item.entity_type == "session" {
        for field in FORBIDDEN_SESSION_FIELDS {
            if object.contains_key(*field) {
                return Err(ApiError::invalid(format!("`{field}` is local-only")));
            }
        }
    }
    Ok(())
}

fn text(payload: &Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn opt_text(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn opt_uuid(payload: &Value, key: &str) -> Option<Uuid> {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
}

fn opt_time(payload: &Value, key: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
}

fn array(payload: &Value, key: &str) -> Value {
    payload.get(key).cloned().unwrap_or_else(|| json!([]))
}

async fn upsert_project(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    item: &SyncItem,
) -> ApiResult<()> {
    sqlx::query(
        "UPDATE projects SET name = COALESCE(NULLIF($2, ''), name),
                             repository_remote = COALESCE($3, repository_remote),
                             updated_at = now()
         WHERE id = $1",
    )
    .bind(project_id)
    .bind(text(&item.payload, "name"))
    .bind(opt_text(&item.payload, "repository_remote"))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn upsert_task(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    item: &SyncItem,
) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, goal, acceptance_criteria, status, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, now())
         ON CONFLICT (id) DO UPDATE SET
             title = EXCLUDED.title, goal = EXCLUDED.goal,
             acceptance_criteria = EXCLUDED.acceptance_criteria,
             status = EXCLUDED.status, updated_at = now()",
    )
    .bind(item.entity_id)
    .bind(project_id)
    .bind(text(&item.payload, "title"))
    .bind(text(&item.payload, "goal"))
    .bind(array(&item.payload, "acceptance_criteria"))
    .bind(text(&item.payload, "status"))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn upsert_session(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    user_id: Uuid,
    item: &SyncItem,
) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO sessions
            (id, project_id, task_id, user_id, agent, branch, commit_sha,
             previous_session_id, status, started_at, ended_at, end_reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, COALESCE($10, now()), $11, $12)
         ON CONFLICT (id) DO UPDATE SET
             task_id = EXCLUDED.task_id, status = EXCLUDED.status,
             ended_at = EXCLUDED.ended_at, end_reason = EXCLUDED.end_reason,
             commit_sha = EXCLUDED.commit_sha",
    )
    .bind(item.entity_id)
    .bind(project_id)
    .bind(opt_uuid(&item.payload, "task_id"))
    .bind(user_id)
    .bind(text(&item.payload, "agent"))
    .bind(text(&item.payload, "branch"))
    .bind(opt_text(&item.payload, "commit_sha"))
    .bind(opt_uuid(&item.payload, "previous_session_id"))
    .bind(text(&item.payload, "status"))
    .bind(opt_time(&item.payload, "started_at"))
    .bind(opt_time(&item.payload, "ended_at"))
    .bind(opt_text(&item.payload, "end_reason"))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn upsert_memory(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    item: &SyncItem,
) -> ApiResult<()> {
    let provenance = item
        .payload
        .get("provenance")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let origin = opt_uuid(&provenance, "session_id")
        .ok_or_else(|| ApiError::invalid("memory provenance must name an origin session"))?;
    let observation_ids = array(&provenance, "observation_ids");
    let evidence_count = provenance
        .get("evidence_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;

    sqlx::query(
        "INSERT INTO memories
            (id, project_id, type, scope, scope_key, content, state, superseded_by_id,
             origin_session_id, observation_ids, evidence_count, evidence_digest, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, now())
         ON CONFLICT (id) DO UPDATE SET
             content = EXCLUDED.content, state = EXCLUDED.state,
             superseded_by_id = EXCLUDED.superseded_by_id,
             observation_ids = EXCLUDED.observation_ids,
             evidence_count = EXCLUDED.evidence_count, updated_at = now()",
    )
    .bind(item.entity_id)
    .bind(project_id)
    .bind(text(&item.payload, "type"))
    .bind(text(&item.payload, "scope"))
    .bind(text(&item.payload, "scope_key"))
    .bind(text(&item.payload, "content"))
    .bind(text(&item.payload, "state"))
    .bind(opt_uuid(&item.payload, "superseded_by_id"))
    .bind(origin)
    .bind(observation_ids)
    .bind(evidence_count)
    .bind(opt_text(&provenance, "digest"))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn upsert_handoff(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    item: &SyncItem,
) -> ApiResult<()> {
    let evidence = item
        .payload
        .get("evidence")
        .cloned()
        .unwrap_or_else(|| json!({}));
    sqlx::query(
        "INSERT INTO handoffs
            (id, project_id, session_id, trigger, goal, progress, completed_work,
             remaining_work, changed_files, decisions, failures, tests_executed,
             repository_state, next_step, agent_note, observation_ids, evidence_count)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
         ON CONFLICT (id) DO UPDATE SET
             agent_note = EXCLUDED.agent_note, next_step = EXCLUDED.next_step",
    )
    .bind(item.entity_id)
    .bind(project_id)
    .bind(
        opt_uuid(&item.payload, "session_id")
            .ok_or_else(|| ApiError::invalid("handoff must name its session"))?,
    )
    .bind(text(&item.payload, "trigger"))
    .bind(text(&item.payload, "goal"))
    .bind(text(&item.payload, "progress"))
    .bind(array(&item.payload, "completed_work"))
    .bind(array(&item.payload, "remaining_work"))
    .bind(array(&item.payload, "changed_files"))
    .bind(array(&item.payload, "decisions"))
    .bind(array(&item.payload, "failures"))
    .bind(array(&item.payload, "tests_executed"))
    .bind(
        item.payload
            .get("repository_state")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
    .bind(text(&item.payload, "next_step"))
    .bind(opt_text(&item.payload, "agent_note"))
    .bind(array(&evidence, "observation_ids"))
    .bind(
        evidence
            .get("evidence_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// A tombstone clears content server-side and is idempotent (FR-052).
async fn tombstone(tx: &mut Transaction<'_, Postgres>, entity: &str, id: Uuid) -> ApiResult<()> {
    let sql = match entity {
        "memory" => "UPDATE memories SET deleted_at = now(), content = '' WHERE id = $1",
        "handoff" => {
            "UPDATE handoffs SET deleted_at = now(), goal = '', progress = '', next_step = '',
                                 agent_note = NULL WHERE id = $1"
        }
        "session" => "UPDATE sessions SET deleted_at = now(), end_reason = NULL WHERE id = $1",
        "task" => "UPDATE tasks SET deleted_at = now() WHERE id = $1",
        "project" => "UPDATE projects SET deleted_at = now() WHERE id = $1",
        other => return Err(ApiError::invalid(format!("cannot delete {other}"))),
    };
    sqlx::query(sql).bind(id).execute(&mut **tx).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Read-back
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ChangesQuery {
    pub project_id: Uuid,
    #[serde(default)]
    pub since: Option<String>,
}

/// Shared records produced by other members, so a teammate's memory becomes
/// locally searchable (FR-056).
pub async fn sync_changes(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ChangesQuery>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, q.project_id, user.id).await?;

    let since = q
        .since
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap());

    let rows = sqlx::query(
        "SELECT * FROM memories
         WHERE project_id = $1 AND updated_at > $2 AND deleted_at IS NULL
         ORDER BY updated_at ASC LIMIT 500",
    )
    .bind(q.project_id)
    .bind(since)
    .fetch_all(&state.pool)
    .await?;

    let cursor = rows
        .last()
        .map(|r| {
            r.get::<chrono::DateTime<chrono::Utc>, _>("updated_at")
                .to_rfc3339()
        })
        .unwrap_or_else(|| since.to_rfc3339());

    let memories: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "type": r.get::<String, _>("type"),
                "scope": r.get::<String, _>("scope"),
                "scope_key": r.get::<String, _>("scope_key"),
                "content": r.get::<String, _>("content"),
                "state": r.get::<String, _>("state"),
                "provenance": {
                    "session_id": r.get::<Uuid, _>("origin_session_id"),
                    "observation_ids": r.get::<Value, _>("observation_ids"),
                    "evidence_count": r.get::<i32, _>("evidence_count"),
                },
            })
        })
        .collect();

    Ok(Json(json!({ "memories": memories, "cursor": cursor })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(entity_type: &str, payload: Value) -> SyncItem {
        SyncItem {
            idempotency_key: "k".into(),
            entity_type: entity_type.into(),
            entity_id: Uuid::now_v7(),
            operation: "upsert".into(),
            payload,
        }
    }

    #[test]
    fn observation_entity_types_are_refused_outright() {
        for t in ["observation", "observation_ref"] {
            assert!(
                reject_forbidden_fields(&item(t, json!({}))).is_err(),
                "{t} accepted"
            );
        }
    }

    #[test]
    fn a_memory_carrying_observation_content_is_rejected() {
        for field in FORBIDDEN_OBSERVATION_FIELDS {
            let payload = json!({ "content": "x", *field: "leaked" });
            let err = reject_forbidden_fields(&item("memory", payload)).unwrap_err();
            assert!(err.message.contains(field), "{field}: {}", err.message);
        }
    }

    #[test]
    fn a_session_carrying_local_only_fields_is_rejected() {
        for field in FORBIDDEN_SESSION_FIELDS {
            let payload = json!({ "agent": "claude-code", *field: "leaked" });
            assert!(
                reject_forbidden_fields(&item("session", payload)).is_err(),
                "{field} accepted"
            );
        }
    }

    #[test]
    fn a_well_formed_memory_passes() {
        let payload = json!({
            "content": "a fact",
            "provenance": { "session_id": Uuid::now_v7(), "observation_ids": [], "evidence_count": 0 }
        });
        assert!(reject_forbidden_fields(&item("memory", payload)).is_ok());
    }
}
