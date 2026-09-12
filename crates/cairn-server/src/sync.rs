//! Sync ingest and read-back (FR-055, FR-056, contracts/server-api.md).
//!
//! Two rules make this endpoint the boundary the privacy model depends on:
//! every item is applied at most once, keyed by its idempotency key; and any
//! item carrying observation content — or a local-only session field — is
//! rejected outright.

use crate::auth::{self, SettledUser};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use cairn_core::wire::codes;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

/// Fields that would carry observation content. Never accepted (FR-055).
///
/// Refused **at any depth** — see `contains_key_recursive` and FR-535. Every
/// name here describes content: what a command printed, where a file lives, what
/// a check observed. A name generic enough that a legitimate payload might use it
/// for something else does not belong on this list; it belongs on
/// [`FORBIDDEN_OBSERVATION_FIELDS_TOP_LEVEL`].
const FORBIDDEN_OBSERVATION_FIELDS: &[&str] = &[
    "summary",
    "path",
    "command",
    "details",
    "exit_code",
    "observations",
    // ---- Feature 003 (FR-506) -------------------------------------------
    //
    // Field names that would carry evidence, diagnostic or checkpoint content.
    // Refused **on the wire** rather than trusted not to exist: the boundary is
    // enforced by the server, not only by what the client happens to send.
    "observed_value",
    "source_locator",
    "value_digest",
    "fingerprint",
    "relevant_paths",
    "criteria_snapshot",
    "sanitization_report",
    "origin_ref",
    "alternative_cause",
    "signal_digest",
    "pin_reason",
    "rationale",
    "basis_evidence_id",
    "path_fingerprints",
    "task_snapshot_at_bind",
    "detail",
    "prior_value",
    "new_value",
    "content_norm_digest",
    // A task's local concurrency token. Meaningless on another machine, and
    // unsound if it travelled (D80).
    "local_revision",
];

/// Entity types the server refuses outright, by name.
///
/// This list **is** the privacy boundary, stated once. A payload naming one of
/// these is rejected exactly as `observation` is — so a malformed or malicious
/// client cannot create a table's worth of local-only content by asking nicely.
const FORBIDDEN_ENTITY_TYPES: &[&str] = &[
    "observation",
    "observation_ref",
    "evidence_fact",
    "verification_run",
    "continuity_checkpoint",
    "reusable_pattern",
    "pattern_application",
    "task_change",
    "criterion_evidence",
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
    user: SettledUser,
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
    auth::require_member(&state.pool, body.project_id, user.id()).await?;

    // Read fresh, once per request rather than once per item: an admin can
    // throw the cutover switch while this process is running, and every item
    // in the batch answers against the same instant rather than one flipping
    // mid-batch (`migration-cutover.md` §2's "read at request time").
    let cutover = cutover_active(&state.pool, state.schema_version).await?;

    let mut results = Vec::with_capacity(body.items.len());
    for item in &body.items {
        // Items are applied independently: one rejection does not fail a batch.
        match apply_item(
            &state.pool,
            state.schema_version,
            cutover,
            body.project_id,
            user.id(),
            item,
        )
        .await
        {
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
    schema_version: i64,
    cutover: bool,
    project_id: Uuid,
    user_id: Uuid,
    item: &SyncItem,
) -> Result<&'static str, ApiError> {
    reject_forbidden_fields(item)?;
    reject_beyond_capability(schema_version, item)?;
    reject_if_cutover(cutover, item)?;

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
        // The fifth validator entry point (D447, FR-545, FR-577).
        //
        // Screened **before** any insert, so a refused item leaves no record and
        // nothing to roll back — stronger than rolling back, because there is no
        // window in which the row existed.
        //
        // The identities are the union of every project this user belongs to,
        // which is broader than any client-side check and catches the one case a
        // client-side check structurally cannot: content naming project X pushed
        // by a client that was working in project Y.
        ("personal_knowledge" | "team_knowledge", "upsert") => {
            let identities = crate::global::identities_for(pool, user_id).await?;
            if let Err(refusal) = crate::global::screen_global_item(&item.payload, &identities) {
                // Rolled back explicitly rather than left to the drop: the
                // `sync_state` claim taken above must not survive, because the
                // refusal is about the content and a corrected payload deserves
                // its own attempt rather than being reported a duplicate.
                tx.rollback().await?;
                return Err(refusal.into_api_error());
            }
            if item.entity_type == "personal_knowledge" {
                crate::global::upsert_personal(&mut tx, user_id, item.entity_id, &item.payload)
                    .await?;
            } else {
                crate::global::upsert_team(&mut tx, user_id, item.entity_id, &item.payload).await?;
            }
        }
        ("project", "upsert") => upsert_project(&mut tx, project_id, item).await?,
        ("task", "upsert") => upsert_task(&mut tx, project_id, item).await?,
        ("session", "upsert") => upsert_session(&mut tx, project_id, user_id, item).await?,
        ("memory", "upsert") => upsert_memory(&mut tx, schema_version, project_id, item).await?,
        ("handoff", "upsert") => upsert_handoff(&mut tx, project_id, item).await?,
        ("memory_relation", "upsert") => upsert_relation(&mut tx, project_id, item).await?,
        ("task_criterion", "upsert") => upsert_criterion(&mut tx, project_id, item).await?,
        ("task_blocker", "upsert") => upsert_blocker(&mut tx, project_id, item).await?,
        (entity, "delete") => tombstone(&mut tx, entity, item.entity_id, project_id).await?,
        (entity, op) => {
            return Err(ApiError::invalid(format!("unsupported {entity}/{op}")));
        }
    }

    tx.commit().await?;
    Ok("applied")
}

/// Entity types this server can only hold once migration 2 has run.
/// Observation field names too generic to refuse at every depth.
///
/// **`outcome` is the whole list, and it is here because refusing it recursively
/// broke a payload the feature depends on.** An observation has an `outcome`, so
/// the name was added to the recursive list — but so does `TestRunRecord`, where
/// it holds a two-value verdict ("passed" / "failed") and nothing else. Once
/// FR-535 made the search recursive, every handoff carrying a completed test run
/// was refused outright: exactly the failure the `command` → `runner` rename
/// (FR-532) had just been made to prevent, reintroduced under a different name,
/// and invisible because no test pushed a handoff with a test run at a real
/// server.
///
/// Refusing it at the top level keeps what the name was added for. An
/// observation-shaped payload has `outcome` as its *own* field, so a payload
/// smuggling one under a permitted entity type is still refused here — and
/// `observation` is refused wholesale by `FORBIDDEN_ENTITY_TYPES` besides. What
/// the top-level check gives up is refusing the word wherever it appears, and
/// the word alone was never the thing worth refusing.
const FORBIDDEN_OBSERVATION_FIELDS_TOP_LEVEL: &[&str] = &["outcome"];

const SCHEMA_2_ENTITY_TYPES: &[&str] = &["memory_relation", "task_criterion", "task_blocker"];

/// The four entity types migration **3** adds tables for (FR-498, FR-522).
///
/// Held back with the same `unknown_entity_type` class as the schema-2 list, and
/// for the same reason: the daemon retains the item and delivers it after the
/// migration runs. Without this list a personal or team item pushed at a
/// schema-2 deployment passed the capability gate — `reject_beyond_capability`
/// returned early for any `schema_version >= 2` — and reached `upsert_personal`,
/// where the missing table surfaced as an internal error. An internal error is
/// not a held item: the daemon has no way to tell "retry after the upgrade" from
/// "this server is broken", so the whole namespace would have failed rather than
/// blocking, and FR-522's guarantee that project sync keeps draining at full
/// speed alongside it would not hold.
const SCHEMA_3_ENTITY_TYPES: &[&str] = &[
    "personal_knowledge",
    "personal_knowledge_relation",
    "team_knowledge",
    "team_knowledge_relation",
];

/// Memory fields migration 2 adds, and what each looks like when it says
/// nothing.
///
/// Every memory payload carries all of these, because the payload builder does
/// not vary its shape. Refusing on their mere presence would refuse every
/// memory a Feature 003 daemon sends — including a plain Feature 001 fact — and
/// SC-326 requires exactly the opposite: ordinary work keeps flowing while the
/// work this server cannot hold waits.
///
/// So the test is whether accepting the memory would **discard** something. A
/// field at its default discards nothing.
fn carries_meaning(field: &str, value: &Value) -> bool {
    match field {
        "topic_key" | "value_key" => value.is_string(),
        "importance" => value.as_str().is_some_and(|s| s != "normal"),
        "pinned" => value.as_bool().unwrap_or(false),
        "reinforcement_count" => value.as_i64().unwrap_or(0) > 0,
        // One distinct origin is the memory's own session. Anything a
        // reinforcement added is what this server would lose.
        "distinct_origin_count" => value.as_i64().unwrap_or(0) > 1,
        // The five-key object exists on every payload; what matters is whether
        // it reports a verification that happened.
        "verification" => value
            .get("state")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s != "unverified"),
        _ => !value.is_null(),
    }
}

const SCHEMA_2_MEMORY_FIELDS: &[&str] = &[
    "topic_key",
    "value_key",
    "importance",
    "pinned",
    "reinforcement_count",
    "distinct_origin_count",
    "verification",
];

/// Work this deployment cannot hold **yet** — as distinct from work it will
/// never accept (FR-415, FR-418, D81).
///
/// The refusal names its class, so the daemon can retain the item and deliver
/// it after the migration runs instead of marking it permanently failed. A
/// generic `invalid_request` here would be indistinguishable from a privacy
/// refusal, and retaining those would be exactly wrong.
fn reject_beyond_capability(schema_version: i64, item: &SyncItem) -> Result<(), ApiError> {
    // Newest first: a schema-2 deployment must be told about a `team_knowledge`
    // item before the `schema_version >= 2` early return below sends it on to a
    // table that does not exist.
    if schema_version < 3 && SCHEMA_3_ENTITY_TYPES.contains(&item.entity_type.as_str()) {
        return Err(held_until_migrated(schema_version, &item.entity_type));
    }
    if schema_version >= 2 {
        return Ok(());
    }
    if SCHEMA_2_ENTITY_TYPES.contains(&item.entity_type.as_str()) {
        return Err(held_until_migrated(schema_version, &item.entity_type));
    }
    if item.entity_type == "memory" {
        if let Some(object) = item.payload.as_object() {
            for field in SCHEMA_2_MEMORY_FIELDS {
                if object
                    .get(*field)
                    .is_some_and(|v| carries_meaning(field, v))
                {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        codes::UNKNOWN_FIELD,
                        format!(
                            "this deployment is at schema {schema_version} and has no \
                             column for `{field}`; it will be accepted once the \
                             migration runs"
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The refusal a client is meant to retain and retry, not fail permanently.
///
/// One constructor for both lists, so the class and the wording a daemon
/// branches on cannot drift between the two schema generations.
fn held_until_migrated(schema_version: i64, entity_type: &str) -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        codes::UNKNOWN_ENTITY_TYPE,
        format!(
            "this deployment is at schema {schema_version} and has nowhere to put \
             a `{entity_type}`; it will be accepted once the migration runs"
        ),
    )
}

/// Entity types the cutover refuses (`migration-cutover.md` §3.1).
///
/// A `delete` naming one of these is refused exactly as an `upsert` naming it
/// is — the match is on `entity_type` alone, not on `(entity_type, operation)`,
/// because the refusal is about further dual-authority writes against
/// knowledge this feature moved off that path, and a tombstone is a write.
const CUTOVER_REFUSED_ENTITY_TYPES: &[&str] = &[
    "memory",
    "memory_relation",
    "personal_knowledge",
    "team_knowledge",
];

/// The code a cutover refusal carries.
///
/// A **new** `&'static str`, deliberately not `codes::UNKNOWN_ENTITY_TYPE`
/// (`migration-cutover.md` §3.2). The two look identical on the wire — same
/// `409`, same envelope shape — but they mean opposite things to a retry loop:
/// `unknown_entity_type` says "wait, this deployment is not ready yet, hold the
/// item and retry after the migration runs"; `upgrade_required` says "stop
/// retrying against this route — this store itself must migrate." Collapsing
/// them into one code would make an upgraded client's retry loop
/// indistinguishable from a pre-migration deployment's, and FR-876b1 requires a
/// client to tell them apart.
const UPGRADE_REQUIRED: &str = "upgrade_required";

/// Whether this deployment has completed its Feature 005 cutover
/// (`migration-cutover.md` §1, §2).
///
/// Read fresh from the database on every call, never cached on `AppState`: an
/// administrator can flip `server_authority.mode` while this process keeps
/// running, and a value captured once would keep answering `pre_cutover` until
/// the next restart — exactly the answer that must not be stale here.
///
/// `false` below schema 4, where `server_authority` does not exist at all: a
/// deployment that has not even applied the table cannot have cut over.
async fn cutover_active(pool: &PgPool, schema_version: i64) -> ApiResult<bool> {
    if schema_version < 4 {
        return Ok(false);
    }
    let mode: Option<(String,)> = sqlx::query_as("SELECT mode FROM server_authority WHERE id = 1")
        .fetch_optional(pool)
        .await?;
    Ok(mode.is_some_and(|(m,)| m == "server_authoritative"))
}

/// Refuse a knowledge-bearing write once this deployment has cut over
/// (`migration-cutover.md` §3, FR-876c).
///
/// Checked before the idempotency claim and before anything is written — no
/// row anywhere is touched by producing this refusal (§3.3) — and it is
/// shape-based rather than a client-version check: a caller still emitting one
/// of these entity types is, by construction, still speaking the pre-005
/// dual-authority protocol, whatever its own binary version happens to be
/// (§3.1). Non-knowledge entity types are untouched by this check and keep
/// working in the same batch, cut over or not (FR-877).
fn reject_if_cutover(cutover: bool, item: &SyncItem) -> Result<(), ApiError> {
    if cutover && CUTOVER_REFUSED_ENTITY_TYPES.contains(&item.entity_type.as_str()) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            UPGRADE_REQUIRED,
            "this server has completed its Feature 005 cutover; personal and team \
             knowledge synchronization now requires a migrated client",
        ));
    }
    Ok(())
}

/// True if `field` appears as a key anywhere inside `value`, at any nesting
/// depth — inside a nested object, or inside an object nested in an array.
///
/// A top-level-only check is a check a client can defeat by wrapping the same
/// forbidden field one level deeper, e.g. `{"provenance": {"summary": "..."}}`.
/// The privacy boundary is what the field name *means* (observation content,
/// or local-only session state), not where in the document it sits, so the
/// search has to follow the payload's whole shape.
fn contains_key_recursive(value: &Value, field: &str) -> bool {
    match value {
        Value::Object(map) => {
            map.contains_key(field) || map.values().any(|v| contains_key_recursive(v, field))
        }
        Value::Array(items) => items.iter().any(|v| contains_key_recursive(v, field)),
        _ => false,
    }
}

/// The denylist enforced on the wire.
///
/// Not an allowlist: the client does not have to declare every field it wants
/// accepted, only avoid the names on `FORBIDDEN_OBSERVATION_FIELDS`,
/// `FORBIDDEN_ENTITY_TYPES` and `FORBIDDEN_SESSION_FIELDS`. The search is
/// recursive — see `contains_key_recursive` — so a forbidden name is refused
/// wherever it appears in the payload, not only at its top level.
fn reject_forbidden_fields(item: &SyncItem) -> Result<(), ApiError> {
    if FORBIDDEN_ENTITY_TYPES.contains(&item.entity_type.as_str()) {
        return Err(ApiError::invalid(format!(
            "`{}` is local to the machine that produced it; the server does not accept it",
            item.entity_type
        )));
    }
    if item.payload.as_object().is_none() {
        return Ok(());
    }

    for field in FORBIDDEN_OBSERVATION_FIELDS {
        if contains_key_recursive(&item.payload, field) {
            return Err(ApiError::invalid(format!(
                "`{field}` carries observation content, which never leaves the machine"
            )));
        }
    }
    // Top level only. See the constant for why the depth differs.
    if let Some(object) = item.payload.as_object() {
        for field in FORBIDDEN_OBSERVATION_FIELDS_TOP_LEVEL {
            if object.contains_key(*field) {
                return Err(ApiError::invalid(format!(
                    "`{field}` carries observation content, which never leaves the machine"
                )));
            }
        }
    }
    if item.entity_type == "session" {
        for field in FORBIDDEN_SESSION_FIELDS {
            if contains_key_recursive(&item.payload, field) {
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

/// Refuse unless the caller's own project was the one written.
///
/// Every `ON CONFLICT (id) DO UPDATE` in this module carries
/// `WHERE <table>.project_id = $n`. When the row that already holds that id
/// belongs to a different project the predicate is false, the update touches
/// nothing, and `rows_affected()` is zero. A fresh insert reports one, and so
/// does a legitimate re-upsert, because every one of them sets
/// `updated_at = now()` — so zero means exactly one thing here.
///
/// Putting the check in the predicate rather than in a `SELECT` first is
/// deliberate. A read followed by a write leaves a window in which the row
/// arrives between the two; under `READ COMMITTED` the write would then proceed
/// against a row the read never saw. One statement has no window.
fn scoped(rows: u64, entity: &str) -> ApiResult<()> {
    if rows == 0 {
        return Err(ApiError::forbidden(format!(
            "that {entity} belongs to a different project"
        )));
    }
    Ok(())
}

/// Every id must already live in the caller's own project.
///
/// Used where a row references another table by id — a relation's two memories,
/// a criterion's task. `ON CONFLICT` cannot express this, because the reference
/// is not the conflicting key.
async fn all_in_project(
    tx: &mut Transaction<'_, Postgres>,
    table: &str,
    ids: &[Uuid],
    project_id: Uuid,
    entity: &str,
) -> ApiResult<()> {
    let sql = format!("SELECT count(*) FROM {table} WHERE id = ANY($1) AND project_id = $2");
    let found: i64 = sqlx::query_scalar(&sql)
        .bind(ids)
        .bind(project_id)
        .fetch_one(&mut **tx)
        .await?;
    if found != ids.len() as i64 {
        return Err(ApiError::forbidden(format!(
            "that {entity} names a row in a different project"
        )));
    }
    Ok(())
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
    let rows = sqlx::query(
        "INSERT INTO tasks (id, project_id, title, goal, acceptance_criteria, status, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, now())
         ON CONFLICT (id) DO UPDATE SET
             title = EXCLUDED.title, goal = EXCLUDED.goal,
             acceptance_criteria = EXCLUDED.acceptance_criteria,
             status = EXCLUDED.status, updated_at = now()
         WHERE tasks.project_id = $2",
    )
    .bind(item.entity_id)
    .bind(project_id)
    .bind(text(&item.payload, "title"))
    .bind(text(&item.payload, "goal"))
    .bind(array(&item.payload, "acceptance_criteria"))
    .bind(text(&item.payload, "status"))
    .execute(&mut **tx)
    .await?;
    scoped(rows.rows_affected(), "task")
}

async fn upsert_session(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    user_id: Uuid,
    item: &SyncItem,
) -> ApiResult<()> {
    let rows = sqlx::query(
        "INSERT INTO sessions
            (id, project_id, task_id, user_id, agent, branch, commit_sha,
             previous_session_id, status, started_at, ended_at, end_reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, COALESCE($10, now()), $11, $12)
         ON CONFLICT (id) DO UPDATE SET
             task_id = EXCLUDED.task_id, status = EXCLUDED.status,
             ended_at = EXCLUDED.ended_at, end_reason = EXCLUDED.end_reason,
             commit_sha = EXCLUDED.commit_sha
         WHERE sessions.project_id = $2",
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
    scoped(rows.rows_affected(), "session")
}

/// `pub(crate)`, not `fn`: the migration drain route (`api.rs`) reuses this
/// exact function rather than writing a second ingest path for project memory
/// (`migration-cutover.md` §12.1, §4.2 — drain is the transfer path a project
/// memory needs precisely because the ordinary sync route refuses it post-cutover).
pub(crate) async fn upsert_memory(
    tx: &mut Transaction<'_, Postgres>,
    schema_version: i64,
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

    // The Feature 003 columns, from the payload the sender built.
    //
    // `authority` is taken as sent — `cairn` or `attested`. The sender never
    // transmits `remote_*`, and the *receiving daemon* is what maps it, so the
    // server stores what established the state on the machine that ran the
    // check rather than a claim about anyone else's (FR-368, D76).
    let verification = item
        .payload
        .get("verification")
        .cloned()
        .unwrap_or_else(|| json!({}));

    // A schema-1 database has none of the columns below, and naming one is a
    // hard SQL error rather than a rejection the daemon could act on — which
    // would strand *every* memory, including the Feature 001 ones this server
    // can hold perfectly well. `reject_beyond_capability` has already refused
    // anything that would lose meaning here, so what reaches this branch is a
    // memory whose Feature 003 fields are all at their defaults (SC-326).
    if schema_version < 2 {
        let rows = sqlx::query(
            "INSERT INTO memories
                (id, project_id, type, scope, scope_key, content, state, superseded_by_id,
                 origin_session_id, observation_ids, evidence_count, evidence_digest, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, now())
             ON CONFLICT (id) DO UPDATE SET
                 content = EXCLUDED.content, state = EXCLUDED.state,
                 superseded_by_id = EXCLUDED.superseded_by_id,
                 observation_ids = EXCLUDED.observation_ids,
                 evidence_count = EXCLUDED.evidence_count, updated_at = now()
         WHERE memories.project_id = $2",
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
        return scoped(rows.rows_affected(), "memory");
    }

    let rows = sqlx::query(
        "INSERT INTO memories
            (id, project_id, type, scope, scope_key, content, state, superseded_by_id,
             origin_session_id, observation_ids, evidence_count, evidence_digest, updated_at,
             topic_key, value_key, importance, pinned, reinforcement_count,
             distinct_origin_count, verification, verification_authority,
             last_verified_at, verification_basis, evidence_fact_count)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, now(),
                 $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23)
         ON CONFLICT (id) DO UPDATE SET
             content = EXCLUDED.content, state = EXCLUDED.state,
             superseded_by_id = EXCLUDED.superseded_by_id,
             observation_ids = EXCLUDED.observation_ids,
             evidence_count = EXCLUDED.evidence_count,
             topic_key = EXCLUDED.topic_key, value_key = EXCLUDED.value_key,
             importance = EXCLUDED.importance, pinned = EXCLUDED.pinned,
             reinforcement_count = EXCLUDED.reinforcement_count,
             distinct_origin_count = EXCLUDED.distinct_origin_count,
             verification = EXCLUDED.verification,
             verification_authority = EXCLUDED.verification_authority,
             last_verified_at = EXCLUDED.last_verified_at,
             verification_basis = EXCLUDED.verification_basis,
             evidence_fact_count = EXCLUDED.evidence_fact_count,
             updated_at = now()
         WHERE memories.project_id = $2",
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
    .bind(opt_text(&item.payload, "topic_key"))
    .bind(opt_text(&item.payload, "value_key"))
    .bind(opt_text(&item.payload, "importance").unwrap_or_else(|| "normal".to_string()))
    .bind(
        item.payload
            .get("pinned")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    )
    .bind(int(&item.payload, "reinforcement_count"))
    .bind(int(&item.payload, "distinct_origin_count"))
    .bind(opt_text(&verification, "state"))
    .bind(opt_text(&verification, "authority"))
    .bind(
        opt_text(&verification, "last_verified_at")
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&chrono::Utc)),
    )
    .bind(
        verification
            .get("basis")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )
    .bind(int(&verification, "fact_count"))
    .execute(&mut **tx)
    .await?;
    scoped(rows.rows_affected(), "memory")
}

fn int(payload: &Value, key: &str) -> i32 {
    payload.get(key).and_then(|v| v.as_i64()).unwrap_or(0) as i32
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
    let rows = sqlx::query(
        "INSERT INTO handoffs
            (id, project_id, session_id, trigger, goal, progress, completed_work,
             remaining_work, changed_files, decisions, failures, tests_executed,
             repository_state, next_step, agent_note, observation_ids, evidence_count)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
         ON CONFLICT (id) DO UPDATE SET
             agent_note = EXCLUDED.agent_note, next_step = EXCLUDED.next_step
         WHERE handoffs.project_id = $2",
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
    scoped(rows.rows_affected(), "handoff")
}

/// A tombstone clears content server-side and is idempotent (FR-052).
async fn tombstone(
    tx: &mut Transaction<'_, Postgres>,
    entity: &str,
    id: Uuid,
    project_id: Uuid,
) -> ApiResult<()> {
    // `project_id = $2` is the whole point of this function's signature.
    //
    // Without it these statements read `WHERE id = $1` and nothing else, and
    // `id` is supplied by the client. Any member of any project could blank the
    // content of, and delete, any memory, handoff, session, task or project on
    // this server given only its UUID — and `sync_batch` had already verified
    // membership of a *different* project, so the request looked entirely
    // legitimate on the way in. This was the most destructive of the
    // authorization defects, because a tombstone also clears the content.
    let sql = match entity {
        "memory" => {
            "UPDATE memories SET deleted_at = now(), content = ''
             WHERE id = $1 AND project_id = $2"
        }
        "handoff" => {
            "UPDATE handoffs SET deleted_at = now(), goal = '', progress = '', next_step = '',
                                 agent_note = NULL
             WHERE id = $1 AND project_id = $2"
        }
        "session" => {
            "UPDATE sessions SET deleted_at = now(), end_reason = NULL
             WHERE id = $1 AND project_id = $2"
        }
        "task" => "UPDATE tasks SET deleted_at = now() WHERE id = $1 AND project_id = $2",
        // A project's own row has no `project_id` column, so the scope is the
        // identity: the only project this request may tombstone is the one it
        // authenticated against.
        "project" => "UPDATE projects SET deleted_at = now() WHERE id = $1 AND id = $2",
        other => return Err(ApiError::invalid(format!("cannot delete {other}"))),
    };
    // Zero rows is left as a no-op rather than an error, because a tombstone is
    // idempotent by contract (FR-052) and a second delete of an
    // already-deleted row must still succeed. It also declines to confirm
    // whether the id exists in some other project, which an error would.
    sqlx::query(sql)
        .bind(id)
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
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
    user: SettledUser,
    Query(q): Query<ChangesQuery>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, q.project_id, user.id()).await?;

    // Read once for this page, same reasoning as `sync_batch`'s own read: an
    // admin can cut over while this process is running, and the answer must
    // not be stale (`migration-cutover.md` §1, §"Reads are never refused").
    let cutover = cutover_active(&state.pool, state.schema_version).await?;

    let since = q
        .since
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap());

    let rows = sqlx::query(
        "SELECT * FROM memories
         WHERE project_id = $1 AND updated_at > $2 AND deleted_at IS NULL
         ORDER BY updated_at ASC LIMIT $3",
    )
    .bind(q.project_id)
    .bind(since)
    .bind(PAGE)
    .fetch_all(&state.pool)
    .await?;

    let memories: Vec<Value> = rows
        .iter()
        .map(|r| {
            // A `serde_json::Map` rather than one `json!` literal, because the
            // post-cutover shape has to *omit* a key, not merely null it.
            let mut memory = serde_json::Map::new();
            memory.insert("id".into(), json!(r.get::<Uuid, _>("id")));
            memory.insert("type".into(), json!(r.get::<String, _>("type")));
            memory.insert("scope".into(), json!(r.get::<String, _>("scope")));
            memory.insert("scope_key".into(), json!(r.get::<String, _>("scope_key")));
            memory.insert("content".into(), json!(r.get::<String, _>("content")));
            memory.insert("state".into(), json!(r.get::<String, _>("state")));
            memory.insert(
                "provenance".into(),
                json!({
                    "session_id": r.get::<Uuid, _>("origin_session_id"),
                    "observation_ids": r.get::<Value, _>("observation_ids"),
                    "evidence_count": r.get::<i32, _>("evidence_count"),
                }),
            );
            // Feature 003. Absent columns read as null on a server whose 0002
            // migration has not run, which is what an older peer sees.
            memory.insert(
                "topic_key".into(),
                json!(r.try_get::<Option<String>, _>("topic_key").ok().flatten()),
            );
            memory.insert(
                "value_key".into(),
                json!(r.try_get::<Option<String>, _>("value_key").ok().flatten()),
            );
            memory.insert(
                "importance".into(),
                json!(r.try_get::<Option<String>, _>("importance").ok().flatten()),
            );
            memory.insert(
                "pinned".into(),
                json!(r.try_get::<Option<bool>, _>("pinned").ok().flatten()),
            );
            // Once this deployment has cut over, the server no longer derives
            // this object from client-asserted state at all (`legacy_verification_audit`
            // is where that state went), so the key is absent rather than
            // present-and-empty — a present empty object would still read as
            // "the server has an answer about verification" (`migration-cutover.md`
            // §"Changes feed stops carrying server verification").
            if !cutover {
                memory.insert(
                    "verification".into(),
                    json!({
                        "state": r.try_get::<Option<String>, _>("verification").ok().flatten(),
                        "authority": r
                            .try_get::<Option<String>, _>("verification_authority")
                            .ok()
                            .flatten(),
                        "last_verified_at": r
                            .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_verified_at")
                            .ok()
                            .flatten()
                            .map(|t| t.to_rfc3339()),
                        "fact_count": r
                            .try_get::<Option<i32>, _>("evidence_fact_count")
                            .ok()
                            .flatten()
                            .unwrap_or(0),
                        "basis": r
                            .try_get::<Option<Value>, _>("verification_basis")
                            .ok()
                            .flatten()
                            .unwrap_or_else(|| json!([])),
                    }),
                );
            }
            Value::Object(memory)
        })
        .collect();

    // The three Feature 003 arrays, read under the **same** cursor as the
    // memories (FR-413).
    //
    // One cursor over `updated_at` across all four is what stops a partial read
    // leaving a relation whose memory has not arrived: the client holds a
    // relation it cannot place and retries it, rather than the server handing
    // out a consistent-looking page that is not.
    let relations = read_after(&state.pool, q.project_id, since, RELATIONS_SQL).await?;
    let tasks = read_after(&state.pool, q.project_id, since, TASKS_SQL).await?;
    let criteria = read_after(&state.pool, q.project_id, since, CRITERIA_SQL).await?;
    let blockers = read_after(&state.pool, q.project_id, since, BLOCKERS_SQL).await?;

    let cursor =
        page_cursor(&[&rows, &relations, &tasks, &criteria, &blockers], since).to_rfc3339();

    Ok(Json(json!({
        "memories": memories,
        "relations": relations.iter().map(relation_json).collect::<Vec<_>>(),
        // Tasks are handed back as well as accepted (`contracts/privacy-sync.md`
        // §What crosses). Without them a criterion arrived naming a `task_id`
        // that could never arrive, so a task created on one machine existed
        // nowhere else and US11's two machines could not converge on a state
        // digest they had no task to compute one from.
        "tasks": tasks.iter().map(task_json).collect::<Vec<_>>(),
        "criteria": criteria.iter().map(criterion_json).collect::<Vec<_>>(),
        "blockers": blockers.iter().map(blocker_json).collect::<Vec<_>>(),
        "cursor": cursor,
    })))
}

/// One read-back page. The four arrays share it, and the cursor respects it —
/// as do the personal and team read-backs in [`crate::global`], so one page size
/// governs every pull rather than each namespace inventing its own.
pub(crate) const PAGE: i64 = 500;

const RELATIONS_SQL: &str = "SELECT * FROM memory_relations
     WHERE project_id = $1 AND updated_at > $2 AND deleted_at IS NULL
     ORDER BY updated_at ASC LIMIT $3";
const TASKS_SQL: &str = "SELECT * FROM tasks
     WHERE project_id = $1 AND updated_at > $2
     ORDER BY updated_at ASC LIMIT $3";
const CRITERIA_SQL: &str = "SELECT * FROM task_criteria
     WHERE project_id = $1 AND updated_at > $2
     ORDER BY updated_at ASC LIMIT $3";
const BLOCKERS_SQL: &str = "SELECT * FROM task_blockers
     WHERE project_id = $1 AND updated_at > $2
     ORDER BY updated_at ASC LIMIT $3";

/// PostgreSQL's `undefined_table`.
const UNDEFINED_TABLE: &str = "42P01";

/// Rows a project changed after `since`.
///
/// A table the 0002 migration has not created yields an empty list: an older
/// deployment simply has nothing of this kind to hand out, and read-back must
/// not fail because of it. **Only** that error is absorbed — a dropped
/// connection or a permission failure must not read as "this deployment has no
/// relations", because the cursor would then advance past records nobody ever
/// received.
async fn read_after(
    pool: &PgPool,
    project_id: Uuid,
    since: chrono::DateTime<chrono::Utc>,
    sql: &str,
) -> ApiResult<Vec<sqlx::postgres::PgRow>> {
    match sqlx::query(sql)
        .bind(project_id)
        .bind(since)
        .bind(PAGE)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => Ok(rows),
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some(UNDEFINED_TABLE) => {
            tracing::debug!(error = %e, "read-back skipped a table this deployment lacks");
            Ok(Vec::new())
        }
        Err(e) => Err(e.into()),
    }
}

/// How far the cursor may advance after a page.
///
/// A table that filled its page still has rows the client has not seen, and
/// some of them may carry an `updated_at` earlier than another table's newest
/// row. Advancing to the newest row across all four would step over them
/// permanently, so a full page pins the cursor to its own last row and the
/// smallest such bound wins. When nothing truncated, every table is exhausted
/// and the newest row any of them returned is safe.
///
/// Re-delivery is the cost, and it is free: every importer is idempotent —
/// `INSERT OR IGNORE` for memories and relations, upsert by id for criteria and
/// blockers.
fn page_cursor(
    pages: &[&[sqlx::postgres::PgRow]],
    since: chrono::DateTime<chrono::Utc>,
) -> chrono::DateTime<chrono::Utc> {
    let pinned = pages
        .iter()
        .filter(|p| p.len() as i64 >= PAGE)
        .filter_map(|p| newest(p))
        .min();
    pinned
        .or_else(|| pages.iter().filter_map(|p| newest(p)).max())
        .unwrap_or(since)
}

fn newest(rows: &[sqlx::postgres::PgRow]) -> Option<chrono::DateTime<chrono::Utc>> {
    rows.last().and_then(|r| {
        r.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")
            .ok()
    })
}

fn relation_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "from_memory_id": r.get::<Uuid, _>("from_memory_id"),
        "to_memory_id": r.get::<Uuid, _>("to_memory_id"),
        "kind": r.get::<String, _>("kind"),
        "decided_by_session": r.get::<Uuid, _>("decided_by_session"),
        "basis": r.get::<String, _>("basis"),
    })
}

/// A task as a peer receives it.
///
/// `local_revision` is deliberately absent: it is a private concurrency token
/// and is neither transmitted nor stored here (D80). The state digest is absent
/// for the same reason it is nowhere on the wire — both sides derive it from the
/// criteria and blockers that did cross, which is what makes two machines
/// agreeing on it a guarantee rather than a copied value.
fn task_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.get::<Uuid, _>("id"),
        "title": r.get::<String, _>("title"),
        "goal": r.get::<String, _>("goal"),
        "status": r.get::<String, _>("status"),
        "acceptance_criteria": r
            .try_get::<Vec<String>, _>("acceptance_criteria")
            .unwrap_or_default(),
        "deleted": r
            .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("deleted_at")
            .ok()
            .flatten()
            .is_some(),
    })
}

fn criterion_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.get::<Uuid, _>("id"),
        "task_id": r.get::<Uuid, _>("task_id"),
        "ordinal": r.get::<i32, _>("ordinal"),
        "label": r.get::<String, _>("label"),
        "text": r.get::<String, _>("text"),
        "state": r.get::<String, _>("state"),
        "verification": r.get::<String, _>("verification"),
        "deleted": r
            .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("deleted_at")
            .ok()
            .flatten()
            .is_some(),
    })
}

fn blocker_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.get::<Uuid, _>("id"),
        "task_id": r.get::<Uuid, _>("task_id"),
        "description": r.get::<String, _>("description"),
        "state": r.get::<String, _>("state"),
        "opened_by_session": r.get::<Uuid, _>("opened_by_session"),
        "cleared_by_session": r
            .try_get::<Option<Uuid>, _>("cleared_by_session")
            .ok()
            .flatten(),
        "deleted": r
            .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("deleted_at")
            .ok()
            .flatten()
            .is_some(),
    })
}

// ---------------------------------------------------------------------------
// Feature 003 entities (`contracts/privacy-sync.md`)
// ---------------------------------------------------------------------------

/// A reconciliation decision.
///
/// `INSERT ... ON CONFLICT DO NOTHING` on the endpoint-pair primary key, so the
/// same decision arriving from two machines is absorbed rather than duplicated —
/// idempotent by construction, with no clock consulted (D78, FR-411).
/// `pub(crate)`, reused by the migration drain route for the same reason
/// [`upsert_memory`] is.
pub(crate) async fn upsert_relation(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    item: &SyncItem,
) -> ApiResult<()> {
    let from = opt_uuid(&item.payload, "from_memory_id")
        .ok_or_else(|| ApiError::invalid("a relation must name from_memory_id"))?;
    let to = opt_uuid(&item.payload, "to_memory_id")
        .ok_or_else(|| ApiError::invalid("a relation must name to_memory_id"))?;

    // A relation is keyed by `(from, to, kind)` and conflicts `DO NOTHING`, so
    // the `rows_affected` trick the other upserts use cannot work here: a
    // legitimate duplicate also reports zero. Both endpoints are checked
    // instead — which is the stronger statement anyway, since it also forbids a
    // relation that spans two projects rather than only one that overwrites.
    all_in_project(tx, "memories", &[from, to], project_id, "relation").await?;

    sqlx::query(
        "INSERT INTO memory_relations
            (from_memory_id, to_memory_id, kind, project_id, decided_by_session, basis, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, now())
         ON CONFLICT (from_memory_id, to_memory_id, kind) DO NOTHING",
    )
    .bind(from)
    .bind(to)
    .bind(text(&item.payload, "kind"))
    .bind(project_id)
    .bind(
        opt_uuid(&item.payload, "decided_by_session")
            .ok_or_else(|| ApiError::invalid("a relation must name its author"))?,
    )
    .bind(text(&item.payload, "basis"))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// One acceptance criterion, by stable id.
///
/// Upserted per criterion rather than per task, which is the whole mechanism
/// behind "two sessions edit different criteria and both survive": different
/// criteria are different rows and cannot collide (FR-413, SC-317).
async fn upsert_criterion(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    item: &SyncItem,
) -> ApiResult<()> {
    let task_id = opt_uuid(&item.payload, "task_id")
        .ok_or_else(|| ApiError::invalid("a criterion must name its task"))?;

    // The id guard below catches an attempt to overwrite another project's
    // criterion. This catches the other direction: attaching a new criterion to
    // another project's task, where there is no existing row to conflict with.
    all_in_project(tx, "tasks", &[task_id], project_id, "criterion").await?;

    let rows = sqlx::query(
        "INSERT INTO task_criteria
            (id, task_id, project_id, ordinal, label, text, state, verification,
             updated_at, deleted_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now(), $9)
         ON CONFLICT (id) DO UPDATE SET
             ordinal = EXCLUDED.ordinal, label = EXCLUDED.label, text = EXCLUDED.text,
             state = EXCLUDED.state, verification = EXCLUDED.verification,
             deleted_at = EXCLUDED.deleted_at, updated_at = now()
         WHERE task_criteria.project_id = $3",
    )
    .bind(item.entity_id)
    .bind(task_id)
    .bind(project_id)
    .bind(
        item.payload
            .get("ordinal")
            .and_then(|v| v.as_i64())
            .unwrap_or(1) as i32,
    )
    .bind(text(&item.payload, "label"))
    .bind(text(&item.payload, "text"))
    .bind(text(&item.payload, "state"))
    .bind(text(&item.payload, "verification"))
    .bind(deleted_at(&item.payload))
    .execute(&mut **tx)
    .await?;
    scoped(rows.rows_affected(), "criterion")
}

/// One blocker. Append-only with a single transition, both ends attributed.
async fn upsert_blocker(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    item: &SyncItem,
) -> ApiResult<()> {
    let task_id = opt_uuid(&item.payload, "task_id")
        .ok_or_else(|| ApiError::invalid("a blocker must name its task"))?;

    all_in_project(tx, "tasks", &[task_id], project_id, "blocker").await?;

    let rows = sqlx::query(
        "INSERT INTO task_blockers
            (id, task_id, project_id, description, state, opened_by_session,
             cleared_by_session, updated_at, deleted_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, now(), $8)
         ON CONFLICT (id) DO UPDATE SET
             state = EXCLUDED.state,
             cleared_by_session = EXCLUDED.cleared_by_session,
             deleted_at = EXCLUDED.deleted_at, updated_at = now()
         WHERE task_blockers.project_id = $3",
    )
    .bind(item.entity_id)
    .bind(task_id)
    .bind(project_id)
    .bind(text(&item.payload, "description"))
    .bind(text(&item.payload, "state"))
    .bind(
        opt_uuid(&item.payload, "opened_by_session")
            .ok_or_else(|| ApiError::invalid("a blocker must name who opened it"))?,
    )
    .bind(opt_uuid(&item.payload, "cleared_by_session"))
    .bind(deleted_at(&item.payload))
    .execute(&mut **tx)
    .await?;
    scoped(rows.rows_affected(), "blocker")
}

/// A tombstone timestamp for a payload that reports itself deleted.
fn deleted_at(payload: &Value) -> Option<chrono::DateTime<chrono::Utc>> {
    payload
        .get("deleted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        .then(chrono::Utc::now)
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
