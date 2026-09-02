//! The post-cutover knowledge command boundary
//! (`contracts/knowledge-commands.md` §3, FR-701, FR-712, FR-815–FR-816).
//!
//! ## Why this replaces an upsert
//!
//! Not a preference. An audit of `sync.rs` on `main` found two things about the
//! `memory` upsert:
//!
//! - its conflict predicate is `ON CONFLICT (id) DO UPDATE … WHERE
//!   memories.project_id = $2` — scoped to the **project**, not to the author —
//!   so any member of a project can overwrite any other member's memory
//!   content, state and verification by naming its id;
//! - `reinforcement_count`, `verification`, `verification_authority` and the
//!   rest are bound straight from the payload, so a client asserts derived state
//!   the server never computed.
//!
//! A command states an **intent** and the server computes the consequences,
//! which makes both problems disappear at once: there is no command that
//! replaces another member's content, and there is no field in which a client
//! can assert a derived value.
//!
//! ## The three rules every command here obeys
//!
//! 1. **Intent only.** Every field in §3.1 is refused when present, rather than
//!    ignored. Ignoring it would let a client believe it had taken effect.
//! 2. **Identity from the credential.** `owner_user_id`, `proposed_by_user_id`
//!    and the session behind `origin_session_id` come from the authenticated
//!    caller, never from the body (Principle XI). A body that could name them
//!    could attribute one account's work to another.
//! 3. **Retries are idempotent.** A command may carry its own `command_id`,
//!    derived on the client from a durable ordinal, so a client that could not
//!    record its own success does not write twice (FR-770 applied to commands).
//!
//! Correcting somebody else's knowledge is `supersede`: a new record, linked to
//! the old one, attributed to whoever made it and reversible — rather than a
//! silent overwrite.

use crate::auth::{bind_session, require_member, CurrentUser, ReaderContext, SessionBindingError};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{Path, State};
use axum::Json;
use cairn_core::domain::{KnowledgeDomain, RelationKind, UNATTRIBUTED_OWNER};
use cairn_core::validate::{validate_global_content, validate_pattern_content, ProjectIdentity};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::str::FromStr;
use uuid::Uuid;

/// Everything the server computes and a client may therefore not send
/// (§3.1).
///
/// Refused rather than ignored. A request carrying `verification: "verified"`
/// that was quietly dropped would leave the caller believing it had asserted
/// something, and the whole point of the boundary is that it cannot.
const COMPUTED_FIELDS: &[&str] = &[
    "state",
    "superseded_by_id",
    "superseded_at",
    "stale_at",
    "reinforcement_count",
    "distinct_origin_count",
    "evidence_count",
    "evidence_fact_count",
    "verification",
    "verification_authority",
    "last_verified_at",
    "verification_basis",
    "created_at",
    "updated_at",
];

/// Identity a client may never name, because it is bound from the credential.
///
/// Separate from [`COMPUTED_FIELDS`] because the reason differs: those are
/// values the server *derives*, these are values the server *is told* by the
/// authentication layer. A client naming one is not asserting a computation, it
/// is asserting an identity (Principle XI).
const CREDENTIAL_BOUND_FIELDS: &[&str] = &[
    "origin_session_id",
    "owner_user_id",
    "proposed_by_user_id",
    "ratified_by_user_id",
    "retired_by_user_id",
    "account_id",
    "writer_id",
    "writer_seq",
];

/// Refuse a body that carries anything the server owns.
///
/// Checked recursively: a computed field nested inside an object is the same
/// assertion made one level down, and a check that only looked at the top level
/// would be defeated by wrapping.
fn reject_server_owned(body: &Value) -> ApiResult<()> {
    fn walk(value: &Value) -> Option<&'static str> {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    for owned in COMPUTED_FIELDS.iter().chain(CREDENTIAL_BOUND_FIELDS) {
                        if key == owned {
                            return Some(owned);
                        }
                    }
                    if let Some(found) = walk(child) {
                        return Some(found);
                    }
                }
                None
            }
            Value::Array(items) => items.iter().find_map(walk),
            _ => None,
        }
    }
    match walk(body) {
        // The field name is named and its value is not, so a refusal cannot
        // echo back content the boundary exists to screen.
        Some(field) => Err(ApiError::invalid(format!(
            "`{field}` is computed by the server and may not be sent"
        ))),
        None => Ok(()),
    }
}

fn text(body: &Value, field: &str) -> ApiResult<String> {
    body.get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ApiError::invalid(format!("`{field}` is required")))
}

fn optional_uuid(body: &Value, field: &str) -> ApiResult<Option<Uuid>> {
    match body.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .map(Some)
            .ok_or_else(|| ApiError::invalid(format!("`{field}` is not a UUID"))),
    }
}

/// The five knowledge kinds, validated rather than trusted.
fn knowledge_type(body: &Value) -> ApiResult<String> {
    let raw = text(body, "type").or_else(|_| text(body, "knowledge_type"))?;
    match raw.as_str() {
        "fact" | "decision" | "convention" | "failure" | "procedure" => Ok(raw),
        other => Err(ApiError::invalid(format!(
            "`{other}` is not a knowledge type"
        ))),
    }
}

fn memory_scope(body: &Value) -> ApiResult<String> {
    let raw = body
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("project")
        .to_string();
    match raw.as_str() {
        "project" | "branch" | "task" | "session" => Ok(raw),
        other => Err(ApiError::invalid(format!(
            "`{other}` is not a memory scope"
        ))),
    }
}

/// Resolve the session a project command is attributed to.
///
/// Optional, and verified when given. A command issued outside any session is a
/// real thing — the CLI permits memory operations outside one — and the honest
/// representation is the nil UUID meaning "no session", which is what the local
/// store already writes rather than inventing a throwaway session row
/// (`contracts/knowledge-commands.md` §4.1).
///
/// When a session *is* named it is verified against the credential exactly as
/// an event's is: a member naming a colleague's session would otherwise
/// attribute their own writes to that colleague (FR-769a).
async fn attributed_session(
    pool: &PgPool,
    reader: &ReaderContext,
    project_id: Uuid,
    body: &Value,
) -> ApiResult<Uuid> {
    let Some(session_id) = optional_uuid(body, "session_id")? else {
        return Ok(UNATTRIBUTED_OWNER);
    };
    match bind_session(pool, reader, session_id).await? {
        Ok(binding) if binding.project_id == project_id => Ok(session_id),
        Ok(_) => Err(ApiError::invalid(
            "the session named belongs to a different project",
        )),
        Err(SessionBindingError::NotOwned) | Err(SessionBindingError::Unresolvable) => {
            Err(ApiError::forbidden("no session you can write to was named"))
        }
    }
}

/// Screening tokens drawn from **every** project this caller belongs to.
///
/// Every project, not the one a request happens to name. Personal knowledge,
/// team guidance and patterns are all project-independent (FR-822, FR-708a),
/// and a record that named any project its author works on would disclose it —
/// so the set to screen against is the author's whole membership, not their
/// current context. Screening against one project would let a personal note
/// name a different one freely.
async fn all_identities_for(
    pool: &PgPool,
    reader: &ReaderContext,
) -> ApiResult<Vec<ProjectIdentity>> {
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT p.name, p.repository_remote
           FROM projects p JOIN project_members m ON m.project_id = p.id
          WHERE m.user_id = $1",
    )
    .bind(reader.user_id())
    .fetch_all(pool)
    .await?;
    let mut out = Vec::new();
    for (name, remote) in rows {
        out.push(ProjectIdentity(name));
        if let Some(remote) = remote {
            for token in remote.split(['/', ':', '@', '.']) {
                if token.len() > 2 {
                    out.push(ProjectIdentity(token.to_string()));
                }
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Idempotency
// ---------------------------------------------------------------------------

/// Reserve a command's identity, atomically, inside the effect's transaction.
///
/// ## The race this replaces
///
/// The first version read `applied_commands`, performed the effect, then
/// inserted the receipt with `ON CONFLICT DO NOTHING`. Two concurrent
/// deliveries of one command both read "not applied", both performed the
/// effect, and one lost the receipt insert — so the reinforcement counted
/// twice and one caller was told `duplicate` about a write that had happened
/// anyway. A check-then-act across a network boundary is not an idempotency
/// gate; it is a window.
///
/// ## The gate
///
/// The reservation is an `INSERT … ON CONFLICT DO NOTHING RETURNING`, taken in
/// the **same transaction** as the effect and **before** it. PostgreSQL's
/// unique index is the arbiter: exactly one concurrent inserter gets a row
/// back, and the other gets none and knows it lost. Because the reservation and
/// the effect share a transaction, a rollback takes both — so a failed command
/// leaves no reservation behind and a later retry is still executable.
///
/// Every command's result id is known before its effect runs — a create mints
/// its id, and the record-addressed commands are named by theirs — so
/// reserving first costs nothing.
///
/// `Reserved` means "you own this command, do the work". `AlreadyApplied`
/// carries what the original produced, returned verbatim: a retry that got a
/// fresh record would be the duplicate the identity exists to prevent.
enum Reservation {
    Reserved,
    AlreadyApplied { result_id: Uuid },
}

/// Take the reservation, or discover somebody already has.
///
/// Keyed `(account_id, command_id)`, and the account half is load-bearing. A
/// `command_id` is UUIDv5 over a scope kind, a scope key and an ordinal, and a
/// sessionless command's scope key is the *store's* `writer_id` — so two
/// accounts on one machine derive identical ids for their own first commands.
/// Keyed on `command_id` alone, the second account's write would be answered
/// `duplicate` and silently never happen.
async fn reserve_command(
    tx: &mut sqlx::PgConnection,
    account_id: Uuid,
    command_id: Option<Uuid>,
    result_id: Uuid,
) -> ApiResult<Reservation> {
    // A command with no id is not idempotent, and that is the caller's choice:
    // an interactive `cairn remember` typed twice is two claims. Every *queued*
    // command carries one, because the spool derives it.
    let Some(command_id) = command_id else {
        return Ok(Reservation::Reserved);
    };
    let reserved: Option<(Uuid,)> = sqlx::query_as(
        "INSERT INTO applied_commands (account_id, command_id, result_id)
         VALUES ($1, $2, $3)
         ON CONFLICT (account_id, command_id) DO NOTHING
         RETURNING result_id",
    )
    .bind(account_id)
    .bind(command_id)
    .bind(result_id)
    .fetch_optional(&mut *tx)
    .await?;
    if reserved.is_some() {
        return Ok(Reservation::Reserved);
    }
    // Lost the race, or this is a replay. Either way the answer is what the
    // winner produced — scoped to this account, so a different account's
    // identical id is invisible here.
    let existing: (Uuid,) = sqlx::query_as(
        "SELECT result_id FROM applied_commands
          WHERE account_id = $1 AND command_id = $2",
    )
    .bind(account_id)
    .bind(command_id)
    .fetch_one(&mut *tx)
    .await?;
    Ok(Reservation::AlreadyApplied {
        result_id: existing.0,
    })
}

/// The reply a duplicate gets.
///
/// A success, and it says so. A replayed `forget` answering `404` would tell a
/// client its instruction had failed when it had already been carried out, and
/// the client's only correct response to that is to retry forever.
fn duplicate_reply(result_id: Uuid) -> Json<Value> {
    Json(json!({ "id": result_id, "applied": "duplicate" }))
}

/// Resolve the project a record belongs to, for a caller entitled to know.
///
/// **One answer for "no such record" and "not your project".** A route that
/// answered `404` for a missing memory and `403` for a colleague's would let
/// anyone with an account enumerate memory ids across the whole server, one
/// guess at a time — the same oracle the ingest boundary closes for sessions
/// (FR-894a).
///
/// The asymmetry with the project-addressed routes is deliberate rather than an
/// inconsistency. There the caller *named* the project, so `403` discloses
/// nothing they did not already supply; here the caller names a record, and
/// whether it exists is precisely what must not leak.
async fn project_of_record(pool: &PgPool, table: &str, id: Uuid, user_id: Uuid) -> ApiResult<Uuid> {
    let hidden = || ApiError::not_found("no such memory");
    let sql = match table {
        "memories" => "SELECT project_id FROM memories WHERE id = $1",
        other => return Err(ApiError::internal(format!("unknown record table {other}"))),
    };
    let project_id: Uuid = sqlx::query_scalar(sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(hidden)?;
    // Membership decided here rather than by `require_member`, because its
    // refusal is a `403` and that is the distinguishable answer this function
    // exists to avoid.
    let member: Option<(Uuid,)> = sqlx::query_as(
        "SELECT user_id FROM project_members WHERE project_id = $1 AND user_id = $2",
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    member.map(|_| project_id).ok_or_else(hidden)
}

// ---------------------------------------------------------------------------
// Project memory
// ---------------------------------------------------------------------------

pub async fn create_memory(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<Uuid>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    reject_server_owned(&body)?;
    require_member(&state.pool, project_id, user.id).await?;
    let reader = ReaderContext::load(&state.pool, &user).await?;

    let command_id = optional_uuid(&body, "command_id")?;
    let kind = knowledge_type(&body)?;
    let scope = memory_scope(&body)?;
    let content = text(&body, "content")?;
    let topic_key = body.get("topic_key").and_then(Value::as_str);
    let value_key = body.get("value_key").and_then(Value::as_str);
    let session = attributed_session(&state.pool, &reader, project_id, &body).await?;
    let scope_key = body
        .get("scope_key")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| project_id.to_string());

    let id = Uuid::now_v7();
    let mut tx = state.pool.begin().await?;
    if let Reservation::AlreadyApplied { result_id } =
        reserve_command(&mut tx, user.id, command_id, id).await?
    {
        return Ok(duplicate_reply(result_id));
    }
    // `origin_kind = 'explicit'`: this memory exists because somebody asked for
    // it, which is exactly the distinction FR-816 asks the column to carry.
    sqlx::query(
        "INSERT INTO memories
             (id, project_id, type, scope, scope_key, content, origin_session_id,
              topic_key, value_key, origin_kind)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'explicit')",
    )
    .bind(id)
    .bind(project_id)
    .bind(&kind)
    .bind(&scope)
    .bind(&scope_key)
    .bind(&content)
    .bind(session)
    .bind(topic_key)
    .bind(value_key)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(Json(json!({ "id": id, "applied": "accepted" })))
}

pub async fn supersede_memory(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    reject_server_owned(&body)?;
    let reader = ReaderContext::load(&state.pool, &user).await?;

    let project_id = project_of_record(&state.pool, "memories", id, user.id).await?;

    let command_id = optional_uuid(&body, "command_id")?;
    let kind = knowledge_type(&body)?;
    let scope = memory_scope(&body)?;
    let content = text(&body, "content")?;
    let session = attributed_session(&state.pool, &reader, project_id, &body).await?;
    let replacement = Uuid::now_v7();

    // The replacement and the supersession commit together. A crash between
    // them would leave either a superseded record pointing at nothing, or a
    // correction nobody can find from the thing it corrects.
    let mut tx = state.pool.begin().await?;
    if let Reservation::AlreadyApplied { result_id } =
        reserve_command(&mut tx, user.id, command_id, replacement).await?
    {
        return Ok(duplicate_reply(result_id));
    }
    sqlx::query(
        "INSERT INTO memories
             (id, project_id, type, scope, scope_key, content, origin_session_id, origin_kind)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'explicit')",
    )
    .bind(replacement)
    .bind(project_id)
    .bind(&kind)
    .bind(&scope)
    .bind(project_id.to_string())
    .bind(&content)
    .bind(session)
    .execute(&mut *tx)
    .await?;

    // The superseded row's `content` is untouched, and there is no clause here
    // capable of writing it: a superseded record keeps saying what it said.
    let updated = sqlx::query(
        "UPDATE memories
            SET state = 'superseded', superseded_by_id = $2, updated_at = now()
          WHERE id = $1 AND state <> 'superseded'",
    )
    .bind(id)
    .bind(replacement)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::invalid("that memory is already superseded"));
    }
    tx.commit().await?;

    Ok(Json(json!({ "id": replacement, "supersedes": id })))
}

pub async fn reinforce_memory(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    reject_server_owned(&body)?;
    // The guard is the point; the project id itself is not needed again.
    project_of_record(&state.pool, "memories", id, user.id).await?;

    let command_id = optional_uuid(&body, "command_id")?;
    let mut tx = state.pool.begin().await?;
    // The gate is taken before the increment and in the same transaction, so
    // two concurrent deliveries of one command cannot both increment. The
    // count is the server's and a replay must not inflate it.
    if let Reservation::AlreadyApplied { .. } =
        reserve_command(&mut tx, user.id, command_id, id).await?
    {
        let count: i32 =
            sqlx::query_scalar("SELECT reinforcement_count FROM memories WHERE id = $1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        return Ok(Json(json!({ "id": id, "reinforcement_count": count,
                               "applied": "duplicate" })));
    }
    let count: i32 = sqlx::query_scalar(
        "UPDATE memories SET reinforcement_count = reinforcement_count + 1, updated_at = now()
          WHERE id = $1 RETURNING reinforcement_count",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(Json(json!({ "id": id, "reinforcement_count": count })))
}

pub async fn pin_memory(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    reject_server_owned(&body)?;
    project_of_record(&state.pool, "memories", id, user.id).await?;

    let command_id = optional_uuid(&body, "command_id")?;
    let pinned = body.get("pinned").and_then(Value::as_bool).unwrap_or(true);

    // Gated even though setting `pinned = true` twice is the same state.
    // Protocol idempotency is not the same property as state idempotency: the
    // caller needs to be told "already done" rather than "done again", and a
    // shape-idempotent command that skipped the gate would be the one command
    // whose replay behaviour differed from every other.
    let mut tx = state.pool.begin().await?;
    if let Reservation::AlreadyApplied { .. } =
        reserve_command(&mut tx, user.id, command_id, id).await?
    {
        tx.commit().await?;
        return Ok(Json(
            json!({ "id": id, "pinned": pinned, "applied": "duplicate" }),
        ));
    }
    sqlx::query("UPDATE memories SET pinned = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(pinned)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "pinned": pinned })))
}

pub async fn record_relation(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<Uuid>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    reject_server_owned(&body)?;
    require_member(&state.pool, project_id, user.id).await?;
    let reader = ReaderContext::load(&state.pool, &user).await?;

    let from = optional_uuid(&body, "from_memory_id")?
        .ok_or_else(|| ApiError::invalid("`from_memory_id` is required"))?;
    let to = optional_uuid(&body, "to_memory_id")?
        .ok_or_else(|| ApiError::invalid("`to_memory_id` is required"))?;
    let kind = RelationKind::from_str(&text(&body, "kind")?)
        .map_err(|_| ApiError::invalid("that is not a relation kind"))?;

    // Both endpoints in this project. A relation reaching across projects would
    // be a reference a member of one of them could not resolve.
    let both: i64 =
        sqlx::query_scalar("SELECT count(*) FROM memories WHERE id = ANY($1) AND project_id = $2")
            .bind(vec![from, to])
            .bind(project_id)
            .fetch_one(&state.pool)
            .await?;
    if both != 2 {
        return Err(ApiError::invalid(
            "both memories must belong to this project",
        ));
    }

    // `conflicts_with` has no direction, so its endpoints are ordered before
    // the write and two machines detecting one conflict produce one row.
    let (from, to) = cairn_core::knowledge::normalize_relation_endpoints(kind, from, to);
    let command_id = optional_uuid(&body, "command_id")?;
    // `decided_by_session` is NOT NULL: a relation records *who decided*, and
    // the column has no room for "nobody". A command issued outside any session
    // carries the nil UUID, which is the honest answer the local store already
    // gives for a CLI invocation with no author to name — rather than an
    // invented session, which would leave a second active session in the
    // worktree and make the next agent's context ambiguous.
    let session = attributed_session(&state.pool, &reader, project_id, &body).await?;

    let mut tx = state.pool.begin().await?;
    // A relation has no id of its own — it *is* the triple — so the gate is
    // keyed on the `from` endpoint as its result. That is enough: the answer a
    // replay needs is "already done", not a new identifier.
    //
    // Gated even though the insert is `ON CONFLICT DO NOTHING` and therefore
    // idempotent in shape. Protocol idempotency is a different property: the
    // caller has to be told "already done" rather than "done again", and a
    // command that skipped the gate would be the one whose replay behaviour a
    // client had to know about separately.
    if let Reservation::AlreadyApplied { .. } =
        reserve_command(&mut tx, user.id, command_id, from).await?
    {
        tx.commit().await?;
        return Ok(Json(json!({
            "from": from, "to": to, "kind": kind.as_str(), "applied": "duplicate"
        })));
    }
    sqlx::query(
        "INSERT INTO memory_relations
             (project_id, from_memory_id, to_memory_id, kind, decided_by_session, basis)
         VALUES ($1, $2, $3, $4, $5, 'explicit')
         ON CONFLICT DO NOTHING",
    )
    .bind(project_id)
    .bind(from)
    .bind(to)
    .bind(kind.as_str())
    .bind(session)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "from": from, "to": to, "kind": kind.as_str() }),
    ))
}

/// Forget a project memory.
///
/// The tombstone shape project knowledge already had, reached as a command so
/// that a queued `forget` has somewhere to land. Gated for the same reason
/// `forget_personal` is: a replay must be told the instruction succeeded.
pub async fn forget_memory(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    reject_server_owned(&body)?;
    project_of_record(&state.pool, "memories", id, user.id).await?;
    let command_id = optional_uuid(&body, "command_id")?;

    let mut tx = state.pool.begin().await?;
    if let Reservation::AlreadyApplied { .. } =
        reserve_command(&mut tx, user.id, command_id, id).await?
    {
        tx.commit().await?;
        return Ok(Json(
            json!({ "id": id, "forgotten": true, "applied": "duplicate" }),
        ));
    }
    sqlx::query(
        "UPDATE memories SET deleted_at = now(), content = '', updated_at = now()
          WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "forgotten": true })))
}

// ---------------------------------------------------------------------------
// Personal knowledge
// ---------------------------------------------------------------------------

pub async fn create_personal(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    reject_server_owned(&body)?;
    let reader = ReaderContext::load(&state.pool, &user).await?;

    let command_id = optional_uuid(&body, "command_id")?;
    let kind = knowledge_type(&body)?;
    let content = text(&body, "content")?;
    // Personal knowledge is project-independent and must not name a project
    // (FR-822) — any of the caller's projects, not merely a project they
    // happen to be in right now.
    let identities = all_identities_for(&state.pool, &reader).await?;
    validate_global_content(
        &content,
        body.get("topic_key").and_then(Value::as_str),
        body.get("value_key").and_then(Value::as_str),
        &[],
        &identities,
    )
    .map_err(|e| ApiError::invalid(format!("refused: {}", e.class)))?;

    let id = Uuid::now_v7();
    let mut tx = state.pool.begin().await?;
    if let Reservation::AlreadyApplied { result_id } =
        reserve_command(&mut tx, user.id, command_id, id).await?
    {
        return Ok(duplicate_reply(result_id));
    }
    sqlx::query(
        "INSERT INTO personal_knowledge
             (id, owner_user_id, knowledge_type, content, topic_key, value_key,
              writer_id, writer_seq)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(user.id)
    .bind(&kind)
    .bind(&content)
    .bind(body.get("topic_key").and_then(Value::as_str))
    .bind(body.get("value_key").and_then(Value::as_str))
    .bind(format!("server-{}", user.id))
    .bind(id.as_u128() as i64 & 0x7fff_ffff)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(Json(json!({ "id": id, "owner_user_id": user.id })))
}

/// Forget one's own personal knowledge.
///
/// The owner check is in the `WHERE` clause, not in a preceding read: a caller
/// who is not the owner affects zero rows and is answered `404`, which is the
/// same answer they get for a record that does not exist. Distinguishing the
/// two would confirm the existence of a colleague's private note to somebody
/// with no standing to know it (FR-846a, `data-model.md` §6.1).
///
/// There is deliberately no administrator exemption. An administrator's
/// standing is over team guidance, not over a colleague's private notes.
pub async fn forget_personal(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    reject_server_owned(&body)?;
    let command_id = optional_uuid(&body, "command_id")?;

    // **Forget is the command where a lost response hurts most**, and the
    // reason it must be gated even though the effect is idempotent in shape.
    // Without the gate a replay finds `forgotten_at` already set, affects zero
    // rows, and answers `404` — telling the client its instruction failed when
    // it had already been carried out. The client's only correct response to
    // that is to retry forever.
    let mut tx = state.pool.begin().await?;
    if let Reservation::AlreadyApplied { .. } =
        reserve_command(&mut tx, user.id, command_id, id).await?
    {
        tx.commit().await?;
        return Ok(Json(
            json!({ "id": id, "forgotten": true, "applied": "duplicate" }),
        ));
    }
    let forgotten = sqlx::query(
        "UPDATE personal_knowledge
            SET forgotten_at = now(), content = ''
          WHERE id = $1 AND owner_user_id = $2 AND forgotten_at IS NULL",
    )
    .bind(id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    if forgotten.rows_affected() == 0 {
        // Rolled back, so the reservation goes with it and a later retry is
        // still executable. A caller who never owned this record must not have
        // its command id burned by the attempt.
        return Err(ApiError::not_found("no such personal record"));
    }
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "forgotten": true })))
}

// ---------------------------------------------------------------------------
// Team knowledge
// ---------------------------------------------------------------------------

/// Propose team guidance.
///
/// Arrives `proposed` and cannot arrive otherwise: `state` is a computed field,
/// so a client cannot ask for `authoritative`, and only a human administrator
/// may ratify (FR-825). The existing compare-and-swap handlers do that and are
/// reused unchanged.
pub async fn propose_team(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    reject_server_owned(&body)?;
    let reader = ReaderContext::load(&state.pool, &user).await?;

    let command_id = optional_uuid(&body, "command_id")?;
    let kind = knowledge_type(&body)?;
    let content = text(&body, "content")?;
    let identities = all_identities_for(&state.pool, &reader).await?;
    validate_global_content(
        &content,
        body.get("topic_key").and_then(Value::as_str),
        body.get("value_key").and_then(Value::as_str),
        &[],
        &identities,
    )
    .map_err(|e| ApiError::invalid(format!("refused: {}", e.class)))?;

    let id = Uuid::now_v7();
    let mut tx = state.pool.begin().await?;
    if let Reservation::AlreadyApplied { result_id } =
        reserve_command(&mut tx, user.id, command_id, id).await?
    {
        return Ok(duplicate_reply(result_id));
    }
    sqlx::query(
        "INSERT INTO team_knowledge
             (id, knowledge_type, content, topic_key, value_key, state,
              proposed_by_user_id, writer_id, writer_seq)
         VALUES ($1, $2, $3, $4, $5, 'proposed', $6, $7, $8)",
    )
    .bind(id)
    .bind(&kind)
    .bind(&content)
    .bind(body.get("topic_key").and_then(Value::as_str))
    .bind(body.get("value_key").and_then(Value::as_str))
    .bind(user.id)
    .bind(format!("server-{}", user.id))
    .bind(id.as_u128() as i64 & 0x7fff_ffff)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(Json(json!({ "id": id, "state": "proposed",
                    "proposed_by_user_id": user.id })))
}

// ---------------------------------------------------------------------------
// Patterns — shape and owner guard only
// ---------------------------------------------------------------------------

/// The safe promotion shape, and the guards that belong to the boundary.
///
/// **The lifecycle repository is US3's** (T083 onward). What T025 owns is the
/// shape: the six fields that may cross, the six that may not, the owner bound
/// from the credential, the server-assigned `trust`, and the content screening
/// a pattern shares with personal and team knowledge.
///
/// `trust` is not accepted from the client at all. `validated` and `contested`
/// are derived locally from `pattern_applications`, which never leave the
/// machine, so the server has no evidence for them and a client asserting one
/// would be asserting a state it earned privately on a record the server cannot
/// check (`data-model.md` §6.2).
pub async fn promote_pattern(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    reject_server_owned(&body)?;
    // The six fields the safe shape drops. Refused rather than ignored: a
    // client whose `origin_ref` was silently dropped would believe the source
    // project had travelled with the pattern.
    for dropped in [
        "signals",
        "signal_digest",
        "origin_ref",
        "sanitization_report",
        "source_memory_id",
        "origin_deleted",
    ] {
        if body.get(dropped).is_some() {
            return Err(ApiError::invalid(format!(
                "`{dropped}` does not cross this boundary; the safe shape drops it"
            )));
        }
    }
    // Server-assigned, both of them.
    for assigned in ["trust", "domain", "pattern_id"] {
        if body.get(assigned).is_some() {
            return Err(ApiError::invalid(format!(
                "`{assigned}` is assigned by the server"
            )));
        }
    }

    let reader = ReaderContext::load(&state.pool, &user).await?;
    let title = text(&body, "title")?;
    let problem = text(&body, "problem")?;
    let root_cause = text(&body, "root_cause")?;
    let approach = text(&body, "approach")?;
    let constraints: Vec<String> = body
        .get("constraints")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let identities = all_identities_for(&state.pool, &reader).await?;
    validate_pattern_content(
        &title,
        &problem,
        &root_cause,
        &approach,
        &constraints,
        &[],
        &identities,
    )
    .map_err(|e| ApiError::invalid(format!("refused: {}", e.class)))?;

    // Identity is derived, so promotion is an upsert and a retry converges
    // (FR-708f, SC-760). The domain is `personal` and the owner is the
    // credential's; neither was nameable above.
    let content_key = cairn_core::eventid::content_key(&problem, &root_cause, &approach);
    let pattern_id = cairn_core::eventid::pattern_id(user.id, &content_key);
    debug_assert_eq!(
        KnowledgeDomain::Personal.as_str(),
        "personal",
        "a pattern is a personal-domain record"
    );

    Ok(Json(json!({
        "pattern_id": pattern_id,
        "owner_user_id": user.id,
        "domain": KnowledgeDomain::Personal.as_str(),
        "trust": "sanitized",
        "content_key": content_key,
        // US3 supplies the repository that makes this durable (T083+). The
        // boundary is what T025 owes, and it is complete: shape validated,
        // content screened, identity derived, owner bound.
        "stored": false,
    })))
}

/// Forget a pattern one owns.
///
/// The owner guard is delegated to the same rule personal knowledge uses, and
/// for the same reason: a pattern is a personal-domain record, so "only the
/// owner" is not a second policy to keep in step, it is the same one.
pub async fn forget_pattern(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(pattern_id): Path<Uuid>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    reject_server_owned(&body)?;
    let forgotten = sqlx::query(
        "UPDATE shared_patterns
            SET forgotten_at = now(), problem = '', root_cause = '', approach = ''
          WHERE pattern_id = $1 AND owner_user_id = $2 AND forgotten_at IS NULL",
    )
    .bind(pattern_id)
    .bind(user.id)
    .execute(&state.pool)
    .await?;
    if forgotten.rows_affected() == 0 {
        return Err(ApiError::not_found("no such pattern"));
    }
    Ok(Json(json!({ "pattern_id": pattern_id, "forgotten": true })))
}

// ---------------------------------------------------------------------------
// The command envelope (T026, T039)
// ---------------------------------------------------------------------------

/// `POST /api/commands` — one authenticated route for every queued command.
///
/// ## Why one route and not thirteen
///
/// A queued command has to reach the server carrying four things the
/// per-command routes cannot all express: its deterministic `command_id`, its
/// kind, whatever target it names, and its intent payload. The first version of
/// the drain posted `payload` alone to a path derived from the kind, which lost
/// the `command_id` — so nothing was idempotent — and named several paths the
/// server does not serve, so nothing was delivered either. Two bugs with one
/// cause: the wire form did not carry the command.
///
/// So the envelope carries all four, and dispatches **internally to the same
/// handlers** the direct routes call. There is one implementation of each
/// command's semantics; this is a second way in, not a second copy.
///
/// ## What the envelope may not carry
///
/// Nothing that decides who is acting. `account_id`, `owner_user_id`,
/// `proposed_by_user_id` and every verification authority are refused by
/// `reject_server_owned` on the payload, exactly as on a direct call, and the
/// account comes from the credential (Principle XI). The daemon cannot invent
/// authorization information because there is no field in which to put it.
///
/// ## Kinds this route does not carry yet
///
/// Pattern promotion and forgetting, and the two verification report shapes,
/// answer a **deferral** rather than a success: their owning phases are US3
/// (T083+) and US5 (T106+). A deferral is a recognisable, non-terminal refusal
/// — the drain leaves the row durable and retries after an upgrade — so a
/// queued pattern promotion waits rather than being silently marked delivered.
/// That distinction is the whole point of answering deferral instead of `404`.
pub async fn command_envelope(
    state: State<AppState>,
    user: CurrentUser,
    Json(envelope): Json<Value>,
) -> ApiResult<Json<Value>> {
    // Screened here as well as in each handler, and both are needed. The
    // envelope is checked **whole**, so a field named at envelope level rather
    // than inside `payload` is refused too — the folding below only carries
    // `payload` in, so an envelope-level `account_id` would otherwise be
    // ignored rather than refused. And a deferred kind never reaches a handler
    // at all, so without this its payload would go unscreened entirely.
    reject_server_owned(&envelope)?;

    let kind = envelope
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::invalid("`kind` is required"))?
        .to_string();
    let command_id = optional_uuid(&envelope, "command_id")?
        .ok_or_else(|| ApiError::invalid("a queued command must carry its `command_id`"))?;

    // The payload is the intent, and the envelope's own fields are folded into
    // it so each handler sees exactly the body it would have received directly.
    // `command_id` is added here rather than trusted from the payload, so an
    // envelope cannot name one id and carry another.
    let mut body = envelope
        .get("payload")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !body.is_object() {
        return Err(ApiError::invalid("`payload` must be an object"));
    }
    body["command_id"] = json!(command_id);

    let project_id = optional_uuid(&envelope, "project_id")?;
    let target_id = optional_uuid(&envelope, "target_id")?;
    let needs_project = || {
        project_id
            .ok_or_else(|| ApiError::invalid(format!("`{kind}` needs the project it applies to")))
    };
    let needs_target = || {
        target_id
            .ok_or_else(|| ApiError::invalid(format!("`{kind}` needs the record it applies to")))
    };

    match kind.as_str() {
        "remember" => create_memory(state, user, Path(needs_project()?), Json(body)).await,
        "supersede" => supersede_memory(state, user, Path(needs_target()?), Json(body)).await,
        "reinforce" => reinforce_memory(state, user, Path(needs_target()?), Json(body)).await,
        "pin" => pin_memory(state, user, Path(needs_target()?), Json(body)).await,
        "forget" => forget_memory(state, user, Path(needs_target()?), Json(body)).await,
        "relate" => record_relation(state, user, Path(needs_project()?), Json(body)).await,
        "personal_create" => create_personal(state, user, Json(body)).await,
        "personal_forget" => forget_personal(state, user, Path(needs_target()?), Json(body)).await,
        "team_propose" => propose_team(state, user, Json(body)).await,

        // Deferred, not refused. The row stays durable and the drain retries it
        // after the phase that owns it ships, which is what stops a queued
        // pattern promotion being marked delivered by a server that cannot
        // store one.
        "pattern_promote" | "pattern_forget" => Err(deferred(&kind, "US3 (T083 onward)")),
        "verification_run" | "verification_attestation" => {
            Err(deferred(&kind, "US5 (T106 onward)"))
        }

        other => Err(ApiError::invalid(format!(
            "`{other}` is not a command kind"
        ))),
    }
}

/// A command kind this build understands but cannot yet carry out.
///
/// `409` with the code the drain recognises as a deferral. Deliberately not
/// `404`, which the drain would read as a permanent refusal and mark the row
/// terminal — losing the user's instruction — and deliberately not `501`, which
/// says nothing about whether retrying will ever help.
///
/// The same shape and the same code the capability mechanism already uses for
/// an entity type a server cannot hold yet (FR-774, FR-775), so the drain needs
/// no second rule.
fn deferred(kind: &str, owner: &str) -> ApiError {
    ApiError::new(
        axum::http::StatusCode::CONFLICT,
        "unsupported_kind",
        format!(
            "`{kind}` is not carried by this build yet; it is owned by {owner}. \
             The command remains queued and will be retried."
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_computed_field_is_refused_at_any_depth() {
        for field in COMPUTED_FIELDS.iter().chain(CREDENTIAL_BOUND_FIELDS) {
            let flat = json!({ "content": "x", *field: "v" });
            assert!(
                reject_server_owned(&flat).is_err(),
                "`{field}` was accepted at the top level"
            );
            let nested = json!({ "content": "x", "inner": { *field: "v" } });
            assert!(
                reject_server_owned(&nested).is_err(),
                "`{field}` was accepted one level down, so wrapping defeats the check"
            );
            let in_array = json!({ "items": [ { *field: "v" } ] });
            assert!(reject_server_owned(&in_array).is_err());
        }
    }

    #[test]
    fn an_ordinary_intent_is_accepted() {
        assert!(reject_server_owned(&json!({
            "type": "decision",
            "scope": "project",
            "content": "storage authority moves to the server",
            "topic_key": "storage.authority",
            "value_key": "server",
            "command_id": Uuid::now_v7(),
            "session_id": Uuid::now_v7(),
        }))
        .is_ok());
    }

    #[test]
    fn a_refusal_names_the_field_and_not_its_value() {
        let err = reject_server_owned(&json!({ "verification": "ghp_secretlooking" }))
            .expect_err("refused");
        let rendered = format!("{err:?}");
        assert!(rendered.contains("verification"));
        assert!(
            !rendered.contains("ghp_"),
            "a refusal echoed the value it refused"
        );
    }
}
