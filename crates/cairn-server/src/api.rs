//! HTTP routes (contracts/server-api.md).

use crate::auth::{self, AdminUser, CurrentUser, SettledUser};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/version", get(version))
        // Authentication
        //
        // There is deliberately no registration route. Self-service account
        // creation was the first step of a complete compromise chain against
        // this server: register, look a project up by its public git remote,
        // join it, then read and write everything in it. Each link is closed
        // separately below, but this one is closed by removal rather than by a
        // check, because an account-creation route that anyone can reach has no
        // safe configuration.
        //
        // An operator creates accounts with `cairn-server users add`, which
        // talks to the database directly and is reachable only by whoever
        // already controls the host.
        //
        // The route itself still answers, because removal is a compatibility
        // event for every client built before it and `404` is the one status
        // that cannot say so — it reads identically to a typo'd URL or a route
        // that never existed. `410 Gone` means "this existed and was
        // deliberately retired", and the body names the replacement so an
        // operator holding only the response can act on it (FR-587,
        // `compatibility.md` §1b).
        .route("/api/auth/register", post(register_removed))
        .route("/api/auth/login", post(login))
        .route("/api/auth/password", post(change_password))
        // Administration. Every route here takes `AdminUser`, so authorization
        // is the parameter list rather than a check a handler could forget.
        .route("/api/admin/users", get(list_users).post(create_user))
        .route("/api/admin/users/{id}", patch(patch_user))
        .route(
            "/api/admin/users/{id}/reset-password",
            post(reset_user_password),
        )
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
        .route("/api/tokens", get(list_tokens).post(create_token))
        .route("/api/tokens/{id}", delete(revoke_token))
        // Linking (FR-064)
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/lookup", get(lookup_projects))
        .route(
            "/api/projects/{id}/members",
            get(list_members).post(add_member).delete(remove_member),
        )
        // No join route either, for the same reason. It required only that the
        // project exist, so naming a UUID was enough to become a member of it —
        // and `lookup` below handed those UUIDs out. A client attaching a fresh
        // clone to a project it is already a member of does not need a route:
        // `GET /api/projects` already reports the caller's memberships, and
        // `cairn link --project` now checks that list instead of asking to be
        // added to it.
        //
        // Same `410 Gone` treatment, for the same reason (FR-587).
        .route("/api/projects/{id}/join", post(join_removed))
        // Sync
        // Safe-event ingest. A boundary of its own, not `/api/sync/batch`:
        // that one carries whole entities a client already decided to store,
        // this one carries typed observations the server decides about. Merged
        // rather than routed inline so its body limit stays its own — see
        // `event_ingest_route`.
        .merge(event_ingest_route())
        .route("/api/sync/batch", post(sync_batch))
        .route("/api/sync/changes", get(sync_changes))
        // Read-back for the two non-project domains (T101, T129).
        //
        // Two routes rather than one taking a namespace: see
        // `global::GlobalChangesQuery` for why, the short version being that a
        // namespace parameter is one field away from being an owner selector,
        // and personal knowledge must have no parameter capable of naming
        // someone else's.
        //
        // Deliberately **not** extra arrays on `/api/sync/changes`: that route
        // takes a `project_id` and answers under one cursor, and each namespace
        // has to keep its own pull position and its own backoff (FR-486,
        // FR-487, FR-488). Sharing the project route would couple a personal
        // pull to a project the personal domain does not belong to.
        .route("/api/sync/changes/personal", get(sync_personal_changes))
        .route("/api/sync/changes/team", get(sync_team_changes))
        // The team lifecycle. `AdminUser` on both handlers, so a member reaching
        // either is refused before the handler runs — an agent has no tool
        // action shaped like ratification and must not gain one through a route
        // (FR-455, FR-515).
        // The post-cutover command boundary (T026). Every one of these
        // replaces a shape the `memory` upsert used to allow, and the
        // difference is that a command states an intent the server acts on
        // rather than a row the server stores.
        .route(
            "/api/projects/{id}/memories",
            get(project_memories).post(crate::commands::create_memory),
        )
        .route(
            "/api/projects/{id}/memory-relations",
            post(crate::commands::record_relation),
        )
        .route(
            "/api/memories/{id}/supersede",
            post(crate::commands::supersede_memory),
        )
        .route(
            "/api/memories/{id}/reinforce",
            post(crate::commands::reinforce_memory),
        )
        .route("/api/memories/{id}/pin", post(crate::commands::pin_memory))
        .route(
            "/api/personal/knowledge",
            post(crate::commands::create_personal),
        )
        .route(
            "/api/personal/knowledge/{id}/forget",
            post(crate::commands::forget_personal),
        )
        .route("/api/team/knowledge", post(crate::commands::propose_team))
        .route(
            "/api/memories/{id}/forget",
            post(crate::commands::forget_memory),
        )
        // One authenticated route for every queued command, dispatching
        // internally to the handlers above. Not a second implementation of
        // command semantics: a second *way in* to the same ones, carrying the
        // deterministic `command_id` the per-command paths have nowhere to put.
        .route("/api/commands", post(crate::commands::command_envelope))
        // Pattern routes are interface-only until US3 supplies their lifecycle
        // repository (T083+). The shape, the owner binding, the server-assigned
        // trust and the content screening are the boundary's, and they are
        // here; what is missing is the store behind them.
        .route("/api/patterns", post(crate::commands::promote_pattern))
        .route(
            "/api/patterns/{id}/forget",
            post(crate::commands::forget_pattern),
        )
        // Ratify and retire already exist and are reused unchanged: each is one
        // compare-and-swap statement, `AdminUser`-gated, and re-implementing
        // them would be a second place for the transition rule to live.
        .route("/api/team/{id}/ratify", post(ratify_team))
        .route("/api/team/{id}/retire", post(retire_team))
        // Read API for the web UI
        .route("/api/projects/{id}", get(project_overview))
        .route("/api/projects/{id}/tasks", get(project_tasks))
        .route("/api/projects/{id}/sessions", get(project_sessions))
        .route("/api/projects/{id}/sync-status", get(project_sync_status))
        // Health and the capture funnel. One write path and one read path per
        // report, shared by US5's dashboard and US6's status (T035).
        .route(
            "/api/projects/{id}/health",
            get(read_health).post(report_health),
        )
        .route("/api/projects/{id}/dispositions", post(report_dispositions))
        .route("/api/sessions/{id}", get(session_detail))
        .route("/api/sessions/{id}/handoff", get(session_handoff))
        .route(
            "/api/memories/{id}",
            get(memory_detail).delete(delete_memory),
        )
}

// ---------------------------------------------------------------------------
// Health and disposition reporting (T035, FR-851-FR-860)
// ---------------------------------------------------------------------------

/// How many rows one health or disposition report may carry.
///
/// A report is a *summary* — one row per agent, capability and stage, or per
/// kind and day. A complete matrix for three agents is seventy-five rows, and a
/// day's dispositions across three agents and twenty-one kinds is a few
/// hundred. A thousand is comfortably above any honest report and well below
/// what an unbounded one could do to a request handler.
const REPORT_MAX_ROWS: usize = 1000;

/// Validate a reported health matrix and seed the rows a read API returns.
///
/// **Shared, because US5 and US6 ask the same two questions of the same rows**
/// and two implementations would be two places for "what counts as a healthy
/// cell" to drift. The write side validates; the read side is a plain select
/// over what the write side accepted.
///
/// Three rules the validation enforces, each of which is a way the matrix could
/// otherwise claim more than Cairn established:
///
/// - **Attribution is per machine** (FR-857). `writer_id` comes from the report
///   because it names the machine that observed the behaviour, but the project
///   and the account come from the credential — a client that could name those
///   could file health for somebody else's project.
/// - **A behavioural status needs an observation** (FR-852). Configuration
///   read-back is `introspection` and does not establish that anything ran.
/// - **The vocabulary is closed.** An unrecognized status is refused rather
///   than stored, because a matrix cell rendering as an unknown string is a
///   blank cell wearing a value.
async fn report_health(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<Uuid>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, project_id, user.id).await?;

    let rows = body
        .get("cells")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::invalid("`cells` must be an array"))?;
    if rows.len() > REPORT_MAX_ROWS {
        return Err(ApiError::invalid(format!(
            "a health report carries at most {REPORT_MAX_ROWS} cells"
        )));
    }

    let writer_id = body
        .get("writer_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ApiError::invalid("`writer_id` is required: a capability is observed on a machine")
        })?
        .to_string();

    let mut accepted = 0usize;
    let mut tx = state.pool.begin().await?;
    for row in rows {
        let cell: cairn_integrate::capability::MatrixCell = serde_json::from_value(row.clone())
            .map_err(|_| ApiError::invalid("a cell is not a health cell"))?;
        if cairn_integrate::capability::MatrixCapability::parse(&cell.capability).is_none() {
            return Err(ApiError::invalid(format!(
                "`{}` is not a capability this matrix has a cell for",
                cell.capability
            )));
        }
        if !cell.is_coherent() {
            // Named rather than silently downgraded: a client reporting
            // `supported` on configuration evidence has a bug worth knowing
            // about, and storing a weaker status would hide it.
            return Err(ApiError::invalid(format!(
                "`{}` reports {} without the evidence that status requires",
                cell.capability, cell.status
            )));
        }

        sqlx::query(
            "INSERT INTO integration_health
                 (project_id, account_id, writer_id, agent, capability, stage, status,
                  evidence_kind, observed_at, degraded)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (project_id, account_id, writer_id, agent, capability, stage)
             DO UPDATE SET status = EXCLUDED.status,
                           evidence_kind = EXCLUDED.evidence_kind,
                           observed_at = EXCLUDED.observed_at,
                           degraded = EXCLUDED.degraded",
        )
        .bind(project_id)
        .bind(user.id)
        .bind(&writer_id)
        .bind(&cell.agent)
        .bind(&cell.capability)
        .bind(&cell.stage)
        .bind(cell.status.as_str())
        .bind(cell.evidence_kind.map(|k| k.as_str()))
        .bind(
            cell.observed_at
                .as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&chrono::Utc)),
        )
        .bind(cell.degraded)
        .execute(&mut *tx)
        .await?;
        accepted += 1;
    }
    tx.commit().await?;
    Ok(Json(json!({ "accepted": accepted })))
}

/// The matrix as it stands, for one project.
///
/// A plain read over what the write side accepted. It does **not** synthesize
/// missing cells: a matrix with a cell absent is a real state — nothing has
/// ever reported it — and filling it in here would make "no report arrived"
/// indistinguishable from "reported as no evidence" (FR-855).
async fn read_health(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, project_id, user.id).await?;
    let rows = sqlx::query(
        "SELECT writer_id, agent, capability, stage, status, evidence_kind,
                observed_at, degraded
           FROM integration_health
          WHERE project_id = $1
          ORDER BY agent, capability, stage",
    )
    .bind(project_id)
    .fetch_all(&state.pool)
    .await?;

    let cells: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "writer_id": r.get::<String, _>("writer_id"),
                "agent": r.get::<String, _>("agent"),
                "capability": r.get::<String, _>("capability"),
                "stage": r.get::<String, _>("stage"),
                "status": r.get::<String, _>("status"),
                "evidence_kind": r.get::<Option<String>, _>("evidence_kind"),
                "observed_at": r
                    .get::<Option<chrono::DateTime<chrono::Utc>>, _>("observed_at")
                    .map(|t| t.to_rfc3339()),
                "degraded": r.get::<Option<bool>, _>("degraded"),
            })
        })
        .collect();
    Ok(Json(json!({ "cells": cells })))
}

/// Record capture dispositions — the funnel's client-reported half.
///
/// Counts, not records. A disposition carries no payload content (FR-749d,
/// FR-741), so there is nothing to keep beyond how often it happened, and the
/// vocabulary is closed by the column's own CHECK.
async fn report_dispositions(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(project_id): Path<Uuid>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, project_id, user.id).await?;
    let rows = body
        .get("counts")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::invalid("`counts` must be an array"))?;
    if rows.len() > REPORT_MAX_ROWS {
        return Err(ApiError::invalid(format!(
            "a disposition report carries at most {REPORT_MAX_ROWS} rows"
        )));
    }

    let mut accepted = 0usize;
    let mut tx = state.pool.begin().await?;
    for row in rows {
        let agent = row
            .get("agent")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::invalid("`agent` is required"))?;
        let kind = row
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::invalid("`kind` is required"))?;
        let disposition = row
            .get("disposition")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::invalid("`disposition` is required"))?;
        // Parsed rather than passed through, so an unrecognized disposition is
        // refused here with a name rather than by a constraint violation.
        if cairn_core::event::Disposition::from_str(disposition).is_err() {
            return Err(ApiError::invalid(format!(
                "`{disposition}` is not a capture disposition"
            )));
        }
        let day = row
            .get("day")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::invalid("`day` is required"))?;
        let n = row.get("n").and_then(Value::as_i64).unwrap_or(0);
        if n < 0 {
            return Err(ApiError::invalid("a count cannot be negative"));
        }

        // Counts accumulate: a client reports what it saw since last time, and
        // two reports of the same day are two batches of the same funnel rather
        // than a correction of it.
        sqlx::query(
            "INSERT INTO capture_dispositions
                 (project_id, account_id, agent, kind, disposition, day, n)
             VALUES ($1, $2, $3, $4, $5, $6::date, $7)
             ON CONFLICT (project_id, account_id, agent, kind, disposition, day)
             DO UPDATE SET n = capture_dispositions.n + EXCLUDED.n",
        )
        .bind(project_id)
        .bind(user.id)
        .bind(agent)
        .bind(kind)
        .bind(disposition)
        .bind(day)
        .bind(n)
        .execute(&mut *tx)
        .await?;
        accepted += 1;
    }
    tx.commit().await?;
    Ok(Json(json!({ "accepted": accepted })))
}

/// `/api/events/batch`, carrying its own request-body limit.
///
/// The bound is `BODY_MAX_BYTES` — 1 MiB, stated as a number in
/// `contracts/safe-events.md` §5 so SC-743 has something to fail against — and
/// it is enforced by the body boundary rather than by counting after the fact.
/// A batch is bounded twice, at two different layers, and both are needed: 256
/// events is a bound on *how many* an honest client sends, and 1 MiB is a bound
/// on how many bytes a hostile one can make the server buffer before anything
/// has been parsed or authenticated. Checking the length inside the handler is
/// too late — the body is already in memory by then.
///
/// A router of its own, merged in, so the limit applies to **this route only**.
/// Axum's `DefaultBodyLimit` is a layer, and putting it on the main router would
/// silently retighten every other endpoint from the 2 MB default to 1 MiB —
/// including `/api/sync/batch`, which is a different boundary with its own
/// bounds and no requirement asking for this one.
fn event_ingest_route() -> Router<AppState> {
    Router::new()
        .route("/api/events/batch", post(crate::events::ingest_batch))
        .layer(DefaultBodyLimit::max(cairn_core::event::BODY_MAX_BYTES))
}

/// The two routes the security prerequisite removed answer here (FR-587).
///
/// Neither takes an authentication extractor. A client that used to register
/// had no account to authenticate with, and a client that used to self-join
/// deserves to learn the route is gone rather than that its token is wrong —
/// answering `401` first would hide the actual fact behind an unrelated one.
///
/// The message names both the replacement route and the CLI verb, because the
/// two audiences that hit this are an integrator reading HTTP and an operator
/// reading a terminal, and neither should have to translate for the other
/// (`compatibility.md` §1b, SC-458).
async fn register_removed() -> ApiError {
    ApiError::new(
        StatusCode::GONE,
        "route_removed",
        "self-registration is disabled; an administrator creates accounts with          `POST /api/admin/users` (`cairn user create`)",
    )
}

/// See [`register_removed`].
///
/// The path segment is taken as a string rather than parsed as a UUID: a
/// malformed id would otherwise be refused as a bad request, which says
/// nothing about the route being gone.
async fn join_removed(Path(_id): Path<String>) -> ApiError {
    ApiError::new(
        StatusCode::GONE,
        "route_removed",
        "self-join is disabled; an existing member or admin adds you with          `POST /api/projects/{id}/members` (`cairn project member add`)",
    )
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// What this deployment runs, and whether a newer release exists.
///
/// Unauthenticated on purpose: the version of a service is not a secret, and
/// the sign-in page is a reasonable place to show it.
async fn version(State(state): State<AppState>) -> Json<Value> {
    // Read fresh rather than from the application state: an administrator can
    // cut this deployment over while it is running, and a client polls here to
    // learn that they did.
    let authority = crate::version::authority_for(&state.pool, state.schema_version).await;
    let payload = state
        .releases
        .payload(state.schema_version, state.server_instance_id, authority)
        .await;
    Json(serde_json::to_value(payload).unwrap_or_else(|_| json!({})))
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> ApiResult<impl IntoResponse> {
    // `status` arrives with migration 3, and a deployment held back for a staged
    // rollout is a supported configuration — selecting the column unconditionally
    // made every schema-2 server refuse every login. Same shape as
    // `auth::create_user`'s check, and for the same reason: the column is what
    // the statement depends on, not the schema number.
    let has_status = state.schema_version >= 3;
    let sql = if has_status {
        "SELECT id, password_hash, status FROM users WHERE email = $1"
    } else {
        "SELECT id, password_hash FROM users WHERE email = $1"
    };
    let row = sqlx::query(sql)
        .bind(body.email.trim().to_lowercase())
        .fetch_optional(&state.pool)
        .await?;

    let Some(row) = row else {
        return Err(ApiError::unauthorized(
            "no account with that email and password",
        ));
    };
    let hash: String = row.try_get("password_hash")?;
    if !auth::verify_password(&body.password, &hash) {
        return Err(ApiError::unauthorized(
            "no account with that email and password",
        ));
    }
    let user_id: Uuid = row.try_get("id")?;
    // A disabled account is refused authentication **by any means**, including a
    // password that remains otherwise correct (FR-410). Asserted separately from
    // token revocation by SC-436, because a regression in either must not be
    // masked by the other still working: revoking tokens without refusing the
    // password leaves a disabled account able to sign in and mint fresh ones.
    // Below schema 3 there is no status to read, and no way for an account to be
    // disabled — so every account is active, which is exactly what this server
    // did before the column existed.
    let status: String = if has_status {
        row.try_get("status")?
    } else {
        "active".to_string()
    };
    if status != "active" {
        return Err(ApiError::unauthorized(
            "no account with that email and password",
        ));
    }
    let token = auth::create_web_session(&state.pool, user_id).await?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        format!(
            "{}={token}; HttpOnly; SameSite=Lax; Path=/{}",
            auth::COOKIE_NAME,
            state.cookie_secure_attr()
        )
        .parse()
        .map_err(|_| ApiError::internal("bad cookie"))?,
    );
    Ok((headers, Json(json!({ "id": user_id }))))
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<impl IntoResponse> {
    if let Some(raw) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        if let Some(token) = raw
            .split(';')
            .filter_map(|c| c.trim().split_once('='))
            .find(|(k, _)| *k == auth::COOKIE_NAME)
            .map(|(_, v)| v)
        {
            auth::destroy_web_session(&state.pool, token).await?;
        }
    }
    let mut out = HeaderMap::new();
    out.insert(
        header::SET_COOKIE,
        format!(
            "{}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0{}",
            auth::COOKIE_NAME,
            state.cookie_secure_attr()
        )
        .parse()
        .map_err(|_| ApiError::internal("bad cookie"))?,
    );
    Ok((out, Json(json!({ "ok": true }))))
}

async fn me(State(state): State<AppState>, user: SettledUser) -> ApiResult<Json<Value>> {
    let row = sqlx::query("SELECT id, email, display_name FROM users WHERE id = $1")
        .bind(user.id())
        .fetch_one(&state.pool)
        .await?;
    // `role` and `status` come from the extractor rather than a second query:
    // the extractor already resolved them for this request, including the
    // schema-2 answer where the columns do not exist. A client that needs to
    // know whether it may ratify team knowledge asks the server rather than
    // caching an answer locally — an authority claim verified against a stale
    // local copy is not verified (FR-464, T121).
    Ok(Json(json!({
        "id": row.try_get::<Uuid, _>("id")?,
        "email": row.try_get::<String, _>("email")?,
        "display_name": row.try_get::<String, _>("display_name")?,
        "role": user.role().as_str(),
        "status": user.status().as_str(),
    })))
}

#[derive(Deserialize)]
struct TokenBody {
    #[serde(default = "default_token_name")]
    name: String,
    /// Optional expiry (FR-417). Omitted or null means no expiry — today's
    /// behaviour, unchanged for every existing caller.
    #[serde(default)]
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn default_token_name() -> String {
    "cairn daemon".to_string()
}

/// The plaintext is returned exactly once and never stored (D10).
async fn create_token(
    State(state): State<AppState>,
    user: SettledUser,
    Json(body): Json<TokenBody>,
) -> ApiResult<Json<Value>> {
    // `SettledUser` is doing the important work in the signature above.
    //
    // This is the one route where escaping the password-change gate would undo
    // it entirely: a temporary credential that can mint a bearer token has
    // bought itself unrestricted access, and the restriction it was under stops
    // meaning anything (FR-407). The gate lives on the extractor rather than
    // here so that a route added later inherits it by default instead of
    // needing to remember.
    let token = auth::random_token();
    let id = Uuid::now_v7();
    // `expires_at` arrives with migration 3. Below it there is no column to
    // write, and a caller asking for an expiry on a server that cannot record
    // one is refused rather than quietly given a token that never expires —
    // silently downgrading a security control is worse than declining it.
    if state.schema_version < 3 {
        if body.expires_at.is_some() {
            return Err(ApiError::invalid(
                "this server predates token expiry; upgrade it or omit expires_at",
            ));
        }
        sqlx::query(
            "INSERT INTO api_tokens (id, user_id, name, token_hash) VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(user.id())
        .bind(&body.name)
        .bind(auth::hash_token(&token))
        .execute(&state.pool)
        .await?;
        return Ok(Json(json!({ "id": id, "name": body.name, "token": token })));
    }
    sqlx::query(
        "INSERT INTO api_tokens (id, user_id, name, token_hash, expires_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(user.id())
    .bind(&body.name)
    .bind(auth::hash_token(&token))
    .bind(body.expires_at)
    .execute(&state.pool)
    .await?;
    Ok(Json(
        json!({ "id": id, "name": body.name, "token": token, "expires_at": body.expires_at }),
    ))
}

async fn list_tokens(State(state): State<AppState>, user: SettledUser) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT id, name, created_at, last_used_at, revoked_at FROM api_tokens
         WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user.id())
    .fetch_all(&state.pool)
    .await?;

    let tokens: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "name": r.get::<String, _>("name"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                "last_used_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_used_at"),
                "revoked_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("revoked_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "tokens": tokens })))
}

async fn revoke_token(
    State(state): State<AppState>,
    user: SettledUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    sqlx::query("UPDATE api_tokens SET revoked_at = now() WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id())
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({ "revoked": id })))
}

// ---------------------------------------------------------------------------
// Administration (FR-401 through FR-414, FR-553 through FR-559)
// ---------------------------------------------------------------------------

/// One account, as every admin route reports it.
fn user_json(row: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": row.get::<Uuid, _>("id"),
        "email": row.get::<String, _>("email"),
        "display_name": row.get::<String, _>("display_name"),
        "role": row.get::<String, _>("role"),
        "status": row.get::<String, _>("status"),
        "must_change_password": row.get::<bool, _>("must_change_password"),
        "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
    })
}

const USER_COLUMNS: &str =
    "id, email, display_name, role, status, must_change_password, created_at";

#[derive(Deserialize)]
struct CreateUserBody {
    email: String,
    display_name: String,
}

/// `POST /api/admin/users` — the only route that creates an account (FR-401).
///
/// The temporary password is returned **once**, in this response, and nowhere
/// else ever (FR-403). There is no route that reads it back — not for the
/// administrator who created the account, not for anyone. A password that can be
/// retrieved after creation is a password stored in retrievable form, whatever
/// the storage claims.
async fn create_user(
    State(state): State<AppState>,
    actor: AdminUser,
    Json(body): Json<CreateUserBody>,
) -> ApiResult<impl IntoResponse> {
    let temporary = auth::temporary_password();
    // `must_change_password = true`: the credential above authenticates to the
    // password-change route and to nothing else (FR-404, FR-407).
    let created = auth::create_user(
        &state.pool,
        &body.email,
        &body.display_name,
        &temporary,
        true,
    )
    .await;
    let (id, email) = match created {
        Ok(pair) => pair,
        // The account-exists case is a conflict, not an internal error, and it
        // is the one failure a caller can act on.
        Err(e) if e.to_string().contains("already has an account") => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "email_taken",
                "that email already has an account",
            ));
        }
        Err(e) => return Err(ApiError::internal(e.to_string())),
    };

    // The password is never logged, and this line is why the actor is carried on
    // the extractor: "an account was created" without "by whom" is not an audit
    // record, it is a statistic.
    tracing::info!(created_user = %id, by = %actor.id(), "account created");

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "email": email,
            "display_name": body.display_name,
            "role": "member",
            "status": "active",
            "must_change_password": true,
            "temporary_password": temporary,
        })),
    ))
}

/// `GET /api/admin/users` — every account, its role and its status (FR-411).
async fn list_users(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(&format!(
        "SELECT {USER_COLUMNS} FROM users ORDER BY created_at"
    ))
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        json!({ "users": rows.iter().map(user_json).collect::<Vec<_>>() }),
    ))
}

#[derive(Deserialize)]
struct PatchUserBody {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// The one advisory-lock key every admin-count-reducing operation serializes on
/// (D445, FR-574).
///
/// A fixed constant, because two transactions taking *different* keys serialize
/// against nothing.
const ADMIN_COUNT_LOCK: i64 = 4_770_040_001;

/// `PATCH /api/admin/users/{id}` — promote, demote, disable, enable (FR-408,
/// FR-412), under the last-admin guarantee (FR-413, FR-560, FR-574).
async fn patch_user(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchUserBody>,
) -> ApiResult<Json<Value>> {
    let role = parse_optional::<cairn_core::domain::ServerRole>(body.role.as_deref(), "role")?;
    let status =
        parse_optional::<cairn_core::domain::UserStatus>(body.status.as_deref(), "status")?;
    if role.is_none() && status.is_none() {
        return Err(ApiError::invalid("name a role, a status, or both"));
    }

    let mut tx = state.pool.begin().await?;

    let target: Option<(String,)> = sqlx::query_as("SELECT email FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((target_email,)) = target else {
        return Err(ApiError::not_found("no such account"));
    };

    // The environment-named account is refused outright, before any lock: a
    // change a restart would silently revert is worse than a rejection, because
    // the operator walks away believing it took (FR-541, FR-542).
    let reduces_authority = matches!(role, Some(cairn_core::domain::ServerRole::Member))
        || matches!(status, Some(cairn_core::domain::UserStatus::Disabled));
    if reduces_authority && state.is_environment_account(&target_email) {
        return Err(environment_account_refusal());
    }

    // Serialize *before* reading anything. Two demotions of the last two admins
    // each individually look legal against the state their own transaction sees;
    // the lock is what makes the second one see the first one's commit (D436).
    if reduces_authority {
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(ADMIN_COUNT_LOCK)
            .execute(&mut *tx)
            .await?;
    }

    // One statement. The `EXISTS` and the `UPDATE` are evaluated together, so
    // there is no read whose result a later write trusts — the anomaly a
    // count-then-update implementation permits has nowhere to happen.
    let guard = if reduces_authority {
        "AND EXISTS (SELECT 1 FROM users
                      WHERE role = 'admin' AND status = 'active' AND id <> $1)"
    } else {
        ""
    };
    let updated: Option<sqlx::postgres::PgRow> = sqlx::query(&format!(
        "UPDATE users
            SET role   = COALESCE($2, role),
                status = COALESCE($3, status)
          WHERE id = $1 {guard}
        RETURNING {USER_COLUMNS}"
    ))
    .bind(id)
    .bind(role.map(|r| r.as_str()))
    .bind(status.map(|s| s.as_str()))
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = updated else {
        // Zero rows, and the row exists, so the guard is what refused: this
        // account is the last active administrator.
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "last_admin",
            "this is the server's only remaining active administrator",
        ));
    };

    // Disabling revokes every live token **in the same transaction as the status
    // change** (FR-409). Two statements in one transaction, not two
    // transactions: a window in which the account is disabled and its tokens
    // still work is exactly the window a cached token exploits.
    //
    // Re-enabling deliberately does **not** clear `revoked_at` (FR-590). A
    // re-enabled account mints fresh tokens; it does not inherit the ones it
    // held before.
    if matches!(status, Some(cairn_core::domain::UserStatus::Disabled)) {
        sqlx::query(
            "UPDATE api_tokens SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM web_sessions WHERE user_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(Json(user_json(&row)))
}

/// `POST /api/admin/users/{id}/reset-password` (FR-553–FR-559, FR-573).
async fn reset_user_password(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let target: Option<(String,)> = sqlx::query_as("SELECT email FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
    let Some((email,)) = target else {
        return Err(ApiError::not_found("no such account"));
    };
    // Refused for the environment-named account: its password is re-established
    // from the environment on every start, so a reset would be silently undone
    // (FR-559).
    if state.is_environment_account(&email) {
        return Err(environment_account_refusal());
    }

    let temporary = auth::temporary_password();
    let hash = auth::hash_password(&temporary)?;
    let mut tx = state.pool.begin().await?;

    // The previous password stops working at the same instant the new one starts
    // (FR-555), and the account owes a change again (FR-557). `status` is
    // untouched on purpose: resetting a disabled account's password does **not**
    // re-enable it (FR-558). A reset is a credential operation and re-enabling
    // is an authority operation; conflating them means an administrator clearing
    // a forgotten password silently readmits an account they disabled on
    // purpose.
    sqlx::query(
        "UPDATE users
            SET password_hash = $2, must_change_password = true, password_changed_at = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(&hash)
    .execute(&mut *tx)
    .await?;

    // Every token the account held is refused from here on (FR-556): a reset
    // exists because the credential may be compromised, and leaving bearer
    // tokens alive would leave the compromise alive.
    sqlx::query(
        "UPDATE api_tokens SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM web_sessions WHERE user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    // Returned once, on this response, and never retrievable again (FR-554).
    Ok(Json(json!({ "id": id, "temporary_password": temporary })))
}

#[derive(Deserialize)]
struct ChangePasswordBody {
    new_password: String,
}

/// `POST /api/auth/password` — the only route a must-change account may reach
/// (FR-405, FR-572).
async fn change_password(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<ChangePasswordBody>,
) -> ApiResult<Json<Value>> {
    if body.new_password.chars().count() < auth::MIN_PASSWORD_LEN {
        return Err(ApiError::invalid(format!(
            "password must be at least {} characters",
            auth::MIN_PASSWORD_LEN
        )));
    }
    let hash = auth::hash_password(&body.new_password)?;
    // One statement clears the flag and overwrites the hash, so the temporary
    // credential is invalidated at the same instant the change takes effect
    // (FR-572). Two statements would leave a window in which both passwords
    // work.
    sqlx::query(
        "UPDATE users
            SET password_hash = $2, must_change_password = false, password_changed_at = now()
          WHERE id = $1",
    )
    .bind(user.id)
    .bind(&hash)
    .execute(&state.pool)
    .await?;
    Ok(Json(json!({ "changed": true })))
}

/// The refusal for any attempt to reduce or reset the environment-named account.
///
/// It names the setting, because the operator's next move is to edit that
/// variable and restart — and a refusal that does not say which one leaves them
/// guessing at the one account whose configuration is not in the database
/// (FR-541, FR-559).
fn environment_account_refusal() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "environment_account",
        "this account is defined by CAIRN_ADMIN_EMAIL; change that environment \
         setting and restart the server instead",
    )
}

fn parse_optional<T: std::str::FromStr>(value: Option<&str>, field: &str) -> ApiResult<Option<T>> {
    match value {
        None => Ok(None),
        Some(raw) => raw
            .parse::<T>()
            .map(Some)
            .map_err(|_| ApiError::invalid(format!("{field} is not one of the permitted values"))),
    }
}

// ---------------------------------------------------------------------------
// Linking (FR-064)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateProjectBody {
    name: String,
    #[serde(default)]
    repository_remote: Option<String>,
}

async fn create_project(
    State(state): State<AppState>,
    user: SettledUser,
    Json(body): Json<CreateProjectBody>,
) -> ApiResult<Json<Value>> {
    let id = Uuid::now_v7();
    let mut tx = state.pool.begin().await?;
    sqlx::query("INSERT INTO projects (id, name, repository_remote) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(&body.name)
        .bind(&body.repository_remote)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO project_members (project_id, user_id) VALUES ($1, $2)")
        .bind(id)
        .bind(user.id())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "name": body.name })))
}

#[derive(Deserialize)]
struct LookupQuery {
    #[serde(default)]
    remote: String,
}

/// A discovery *hint*. Returns only projects the caller may already see, and
/// never links anything on its own (D14).
///
/// The doc comment above is what this was always documented to do. The query
/// did not do it: it matched on `repository_remote` alone, with no reference to
/// the caller at all. A git remote is not a secret — it is in every clone of
/// the repository and often on a public forge — so any authenticated account
/// could turn a remote URL into the project UUIDs behind it, which was exactly
/// the input the join route needed. The membership join below is the fix; the
/// comment needed no change, only the SQL.
async fn lookup_projects(
    State(state): State<AppState>,
    user: SettledUser,
    Query(q): Query<LookupQuery>,
) -> ApiResult<Json<Value>> {
    if q.remote.trim().is_empty() {
        return Ok(Json(json!({ "projects": [] })));
    }
    let rows = sqlx::query(
        "SELECT p.id, p.name FROM projects p
         JOIN project_members m ON m.project_id = p.id
         WHERE p.repository_remote = $1 AND p.deleted_at IS NULL AND m.user_id = $2
         ORDER BY p.created_at",
    )
    .bind(q.remote.trim())
    .bind(user.id())
    .fetch_all(&state.pool)
    .await?;
    let projects: Vec<Value> = rows
        .iter()
        .map(|r| json!({ "id": r.get::<Uuid, _>("id"), "name": r.get::<String, _>("name") }))
        .collect();
    Ok(Json(json!({ "projects": projects })))
}

async fn list_projects(State(state): State<AppState>, user: SettledUser) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT p.id, p.name, p.repository_remote, p.created_at
         FROM projects p
         JOIN project_members m ON m.project_id = p.id
         WHERE m.user_id = $1 AND p.deleted_at IS NULL
         ORDER BY p.created_at DESC",
    )
    .bind(user.id())
    .fetch_all(&state.pool)
    .await?;

    let projects: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "name": r.get::<String, _>("name"),
                "repository_remote": r.get::<Option<String>, _>("repository_remote"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "projects": projects })))
}

// ---------------------------------------------------------------------------
// Project membership (FR-418 through FR-427)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct MemberBody {
    /// Looked up by id, not by email.
    ///
    /// An email-addressed grant route is an email-enumeration oracle: a caller
    /// learns which addresses have accounts by watching which grants return
    /// `404`. Ids come from `GET /api/admin/users`, which is admin-only.
    user_id: Uuid,
}

/// `POST /api/projects/{id}/members` — grant membership (FR-418, FR-419).
///
/// **Every grant names somebody else.** A caller cannot add itself: that is the
/// hole the deleted self-join route was, and re-opening it by accident is what
/// `SC-465` exists to catch. The check is explicit rather than implied by the
/// authorization, because "you must already be a member to add someone" and "you
/// may not add yourself" are different rules and only the second one closes the
/// hole — an existing member adding itself again is harmless, but a route shaped
/// to allow it is one refactor away from allowing a non-member to.
async fn add_member(
    State(state): State<AppState>,
    caller: SettledUser,
    Path(project_id): Path<Uuid>,
    Json(body): Json<MemberBody>,
) -> ApiResult<impl IntoResponse> {
    if body.user_id == caller.id() {
        return Err(ApiError::forbidden(
            "a membership grant names someone else; you cannot add yourself",
        ));
    }
    // An existing member, or a server administrator. The admin bypass exists so
    // membership can be bootstrapped on a project whose members have all been
    // removed — without it such a project is permanently unreachable, and
    // FR-419's "an existing member **or an admin**" is what makes that state
    // recoverable rather than terminal.
    if !caller.0.is_admin() {
        auth::require_member(&state.pool, project_id, caller.id()).await?;
    }

    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM projects WHERE id = $1")
        .bind(project_id)
        .fetch_optional(&state.pool)
        .await?;
    if exists.is_none() {
        return Err(ApiError::not_found("no such shared project"));
    }
    let target: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE id = $1")
        .bind(body.user_id)
        .fetch_optional(&state.pool)
        .await?;
    if target.is_none() {
        return Err(ApiError::not_found("no such account"));
    }

    if state.schema_version < 3 {
        // `added_by_user_id` arrives with migration 3, and FR-419 requires the
        // grant to record who made it. A server that cannot record it says so
        // rather than granting membership with the attribution silently missing
        // — an unattributed grant is exactly what this route exists to replace.
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "schema_too_old",
            "this server predates membership grants; upgrade it to schema 3",
        ));
    }
    let inserted = sqlx::query(
        "INSERT INTO project_members (project_id, user_id, added_by_user_id)
         VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(project_id)
    .bind(body.user_id)
    .bind(caller.id())
    .execute(&state.pool)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "already_member",
            "that account is already a member of this project",
        ));
    }

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "project_id": project_id,
            "user_id": body.user_id,
            "added_by_user_id": caller.id(),
        })),
    ))
}

/// `DELETE /api/projects/{id}/members` — revoke membership (FR-420, FR-421).
///
/// Takes effect on the very next request that checks it: there is no cache and
/// no session that carries a stale membership decision, because every
/// project-scoped route re-evaluates `require_member` per request.
async fn remove_member(
    State(state): State<AppState>,
    caller: SettledUser,
    Path(project_id): Path<Uuid>,
    Json(body): Json<MemberBody>,
) -> ApiResult<Json<Value>> {
    if !caller.0.is_admin() {
        auth::require_member(&state.pool, project_id, caller.id()).await?;
    }
    let removed = sqlx::query("DELETE FROM project_members WHERE project_id = $1 AND user_id = $2")
        .bind(project_id)
        .bind(body.user_id)
        .execute(&state.pool)
        .await?;
    if removed.rows_affected() == 0 {
        return Err(ApiError::not_found("that account is not a member"));
    }
    Ok(Json(
        json!({ "project_id": project_id, "user_id": body.user_id, "removed": true }),
    ))
}

/// `GET /api/projects/{id}/members` — the full membership list (FR-427).
///
/// A member sees who else is a member; that is not privileged information within
/// a project someone has already been granted. A non-member is refused, so the
/// route is not a way to enumerate a project's people from outside it.
async fn list_members(
    State(state): State<AppState>,
    caller: SettledUser,
    Path(project_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    if !caller.0.is_admin() {
        auth::require_member(&state.pool, project_id, caller.id()).await?;
    }
    if state.schema_version < 3 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "schema_too_old",
            "this server predates the membership list; upgrade it to schema 3",
        ));
    }
    let rows = sqlx::query(
        "SELECT u.id, u.email, u.display_name, m.added_by_user_id, m.created_at
           FROM project_members m
           JOIN users u ON u.id = m.user_id
          WHERE m.project_id = $1
          ORDER BY m.created_at",
    )
    .bind(project_id)
    .fetch_all(&state.pool)
    .await?;
    let members: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "user_id": r.get::<Uuid, _>("id"),
                "email": r.get::<String, _>("email"),
                "display_name": r.get::<String, _>("display_name"),
                "added_by_user_id": r.get::<Option<Uuid>, _>("added_by_user_id"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "members": members })))
}

// ---------------------------------------------------------------------------
// Sync (FR-055, FR-056)
// ---------------------------------------------------------------------------

pub use crate::global::{ratify_team, retire_team, sync_personal_changes, sync_team_changes};
pub use crate::sync::{sync_batch, sync_changes};

// ---------------------------------------------------------------------------
// Read API for the web UI
// ---------------------------------------------------------------------------

async fn project_overview(
    State(state): State<AppState>,
    user: SettledUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, id, user.id()).await?;

    let project = sqlx::query("SELECT id, name, repository_remote FROM projects WHERE id = $1")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;

    let counts = sqlx::query(
        "SELECT
            (SELECT COUNT(*) FROM tasks WHERE project_id = $1 AND deleted_at IS NULL) AS tasks,
            (SELECT COUNT(*) FROM tasks WHERE project_id = $1 AND status != 'done'
                AND deleted_at IS NULL) AS open_tasks,
            (SELECT COUNT(*) FROM sessions WHERE project_id = $1 AND deleted_at IS NULL) AS sessions,
            (SELECT COUNT(*) FROM memories WHERE project_id = $1 AND deleted_at IS NULL) AS memories",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;

    let branches = sqlx::query(
        "SELECT branch, COUNT(*) AS sessions, MAX(started_at) AS last_seen
         FROM sessions WHERE project_id = $1 AND deleted_at IS NULL
         GROUP BY branch ORDER BY last_seen DESC LIMIT 10",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;

    let recent = sessions_json(
        &sqlx::query(
            "SELECT * FROM sessions WHERE project_id = $1 AND deleted_at IS NULL
             ORDER BY started_at DESC LIMIT 10",
        )
        .bind(id)
        .fetch_all(&state.pool)
        .await?,
    );

    Ok(Json(json!({
        "project": {
            "id": project.get::<Uuid, _>("id"),
            "name": project.get::<String, _>("name"),
            "repository_remote": project.get::<Option<String>, _>("repository_remote"),
        },
        "counts": {
            "tasks": counts.get::<i64, _>("tasks"),
            "open_tasks": counts.get::<i64, _>("open_tasks"),
            "sessions": counts.get::<i64, _>("sessions"),
            "memories": counts.get::<i64, _>("memories"),
        },
        "branches": branches.iter().map(|r| json!({
            "branch": r.get::<String, _>("branch"),
            "sessions": r.get::<i64, _>("sessions"),
            "last_seen": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_seen"),
        })).collect::<Vec<_>>(),
        "recent_sessions": recent,
    })))
}

#[derive(Deserialize)]
struct TaskQuery {
    #[serde(default)]
    status: Option<String>,
}

async fn project_tasks(
    State(state): State<AppState>,
    user: SettledUser,
    Path(id): Path<Uuid>,
    Query(q): Query<TaskQuery>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, id, user.id()).await?;
    let rows =
        match &q.status {
            Some(status) => sqlx::query(
                "SELECT * FROM tasks WHERE project_id = $1 AND status = $2 AND deleted_at IS NULL
                 ORDER BY created_at DESC",
            )
            .bind(id)
            .bind(status)
            .fetch_all(&state.pool)
            .await?,
            None => {
                sqlx::query(
                    "SELECT * FROM tasks WHERE project_id = $1 AND deleted_at IS NULL
                 ORDER BY created_at DESC",
                )
                .bind(id)
                .fetch_all(&state.pool)
                .await?
            }
        };

    let tasks: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "title": r.get::<String, _>("title"),
                "goal": r.get::<String, _>("goal"),
                "acceptance_criteria": r.get::<Value, _>("acceptance_criteria"),
                "status": r.get::<String, _>("status"),
                "updated_at": r.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "tasks": tasks })))
}

fn sessions_json(rows: &[sqlx::postgres::PgRow]) -> Vec<Value> {
    rows.iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "task_id": r.get::<Option<Uuid>, _>("task_id"),
                "agent": r.get::<String, _>("agent"),
                "branch": r.get::<String, _>("branch"),
                "commit_sha": r.get::<Option<String>, _>("commit_sha"),
                "status": r.get::<String, _>("status"),
                "started_at": r.get::<chrono::DateTime<chrono::Utc>, _>("started_at"),
                "ended_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("ended_at"),
                "end_reason": r.get::<Option<String>, _>("end_reason"),
            })
        })
        .collect()
}

async fn project_sessions(
    State(state): State<AppState>,
    user: SettledUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, id, user.id()).await?;
    let rows = sqlx::query(
        "SELECT s.*, EXISTS (
             SELECT 1 FROM handoffs h WHERE h.session_id = s.id AND h.deleted_at IS NULL
         ) AS has_handoff
         FROM sessions s
         WHERE s.project_id = $1 AND s.deleted_at IS NULL
         ORDER BY s.started_at DESC LIMIT 100",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;

    let sessions: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut v = json!({
                "id": r.get::<Uuid, _>("id"),
                "task_id": r.get::<Option<Uuid>, _>("task_id"),
                "agent": r.get::<String, _>("agent"),
                "branch": r.get::<String, _>("branch"),
                "status": r.get::<String, _>("status"),
                "started_at": r.get::<chrono::DateTime<chrono::Utc>, _>("started_at"),
                "ended_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("ended_at"),
            });
            v["has_handoff"] = json!(r.get::<bool, _>("has_handoff"));
            v
        })
        .collect();
    Ok(Json(json!({ "sessions": sessions })))
}

async fn session_detail(
    State(state): State<AppState>,
    user: SettledUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let row = sqlx::query("SELECT * FROM sessions WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("no such session"))?;
    let project_id: Uuid = row.try_get("project_id")?;
    auth::require_member(&state.pool, project_id, user.id()).await?;
    Ok(Json(
        json!({ "session": sessions_json(std::slice::from_ref(&row))[0] }),
    ))
}

async fn session_handoff(
    State(state): State<AppState>,
    user: SettledUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let row = sqlx::query(
        "SELECT * FROM handoffs WHERE session_id = $1 AND deleted_at IS NULL
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("no handoff for that session"))?;

    let project_id: Uuid = row.try_get("project_id")?;
    auth::require_member(&state.pool, project_id, user.id()).await?;

    Ok(Json(json!({ "handoff": handoff_json(&row) })))
}

fn handoff_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.get::<Uuid, _>("id"),
        "session_id": r.get::<Uuid, _>("session_id"),
        "trigger": r.get::<String, _>("trigger"),
        "goal": r.get::<String, _>("goal"),
        "progress": r.get::<String, _>("progress"),
        "completed_work": r.get::<Value, _>("completed_work"),
        "remaining_work": r.get::<Value, _>("remaining_work"),
        "changed_files": r.get::<Value, _>("changed_files"),
        "decisions": r.get::<Value, _>("decisions"),
        "failures": r.get::<Value, _>("failures"),
        "tests_executed": r.get::<Value, _>("tests_executed"),
        "repository_state": r.get::<Value, _>("repository_state"),
        "next_step": r.get::<String, _>("next_step"),
        "agent_note": r.get::<Option<String>, _>("agent_note"),
        // References only. The observations stayed on the capturing machine.
        "evidence": {
            "observation_ids": r.get::<Value, _>("observation_ids"),
            "evidence_count": r.get::<i32, _>("evidence_count"),
        },
        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
    })
}

#[derive(Deserialize)]
struct MemoryQueryParams {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scope_key: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

/// Same scope-first ranking as the local path, so a query behaves the same in
/// the UI and in the agent (D3).
async fn project_memories(
    State(state): State<AppState>,
    user: SettledUser,
    Path(id): Path<Uuid>,
    Query(q): Query<MemoryQueryParams>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, id, user.id()).await?;
    let limit = q.limit.unwrap_or(25).clamp(1, 100);
    let want_state = q.state.unwrap_or_else(|| "active".to_string());

    let rows = sqlx::query(
        "SELECT *,
                CASE scope WHEN 'task' THEN 0 WHEN 'branch' THEN 1
                           WHEN 'project' THEN 2 ELSE 3 END AS scope_bucket,
                CASE WHEN $2::text IS NULL OR $2 = '' THEN 0
                     ELSE ts_rank(to_tsvector('english', content),
                                  plainto_tsquery('english', $2)) END AS relevance
         FROM memories
         WHERE project_id = $1
           AND deleted_at IS NULL
           AND state = $3
           AND ($2::text IS NULL OR $2 = ''
                OR to_tsvector('english', content) @@ plainto_tsquery('english', $2))
           AND ($4::text IS NULL OR scope = $4)
           AND ($5::text IS NULL OR scope_key = $5)
           AND ($6::text IS NULL OR type = $6)
         ORDER BY scope_bucket ASC, relevance DESC, created_at DESC
         LIMIT $7",
    )
    .bind(id)
    .bind(q.q.clone().unwrap_or_default())
    .bind(&want_state)
    .bind(&q.scope)
    .bind(&q.scope_key)
    .bind(&q.kind)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    let memories: Vec<Value> = rows.iter().map(memory_json).collect();
    Ok(Json(
        json!({ "memories": memories, "total": memories.len() }),
    ))
}

fn memory_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.get::<Uuid, _>("id"),
        "type": r.get::<String, _>("type"),
        "scope": r.get::<String, _>("scope"),
        "scope_key": r.get::<String, _>("scope_key"),
        "content": r.get::<String, _>("content"),
        "state": r.get::<String, _>("state"),
        "superseded_by_id": r.get::<Option<Uuid>, _>("superseded_by_id"),
        "provenance": {
            "session_id": r.get::<Uuid, _>("origin_session_id"),
            "observation_ids": r.get::<Value, _>("observation_ids"),
            "evidence_count": r.get::<i32, _>("evidence_count"),
        },
        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        "updated_at": r.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
    })
}

async fn memory_detail(
    State(state): State<AppState>,
    user: SettledUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let row = sqlx::query("SELECT * FROM memories WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("no such memory"))?;
    let project_id: Uuid = row.try_get("project_id")?;
    auth::require_member(&state.pool, project_id, user.id()).await?;

    // Provenance is references; evidence content is local to the machine that
    // captured it and does not exist here (FR-055, FR-061).
    let mut value = memory_json(&row);
    value["provenance"]["evidence_content_available"] = json!(false);
    Ok(Json(json!({ "memory": value })))
}

async fn delete_memory(
    State(state): State<AppState>,
    user: SettledUser,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let row = sqlx::query("SELECT project_id FROM memories WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("no such memory"))?;
    let project_id: Uuid = row.try_get("project_id")?;
    auth::require_member(&state.pool, project_id, user.id()).await?;

    sqlx::query("UPDATE memories SET deleted_at = now(), content = '' WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok((StatusCode::OK, Json(json!({ "deleted": id }))))
}

async fn project_sync_status(
    State(state): State<AppState>,
    user: SettledUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, id, user.id()).await?;
    let row = sqlx::query(
        "SELECT COUNT(*) AS applied, MAX(applied_at) AS last_applied
         FROM sync_state WHERE project_id = $1",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({
        "applied_items": row.get::<i64, _>("applied"),
        "last_applied_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_applied"),
    })))
}
