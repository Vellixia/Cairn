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
        // The third of the same shape (T085). A pattern is a personal-domain
        // record, so this feed is owner-scoped exactly as `changes/personal`
        // is, and for the same reason a namespace parameter is not used to
        // reach it: a parameter that selects a namespace is one edit away from
        // selecting an owner.
        //
        // Unlike the two above, this one carries **tombstones**. A cache that
        // already holds a pattern only learns it was forgotten from the row
        // itself, so a forgotten pattern travels once more with its
        // `forgotten_at` and no content.
        .route(
            "/api/sync/changes/patterns",
            get(crate::commands::pattern_changes),
        )
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
        // Personal knowledge: one route, a write and a read, both bound to the
        // credential. The read is US5's Domains panel (T110) and takes no
        // parameter that could name an owner — see
        // `global::personal_knowledge_view` for why that is the guarantee
        // rather than a check.
        .route(
            "/api/personal/knowledge",
            get(crate::global::personal_knowledge_view).post(crate::commands::create_personal),
        )
        .route(
            "/api/personal/knowledge/{id}/forget",
            post(crate::commands::forget_personal),
        )
        // Team knowledge: propose, and read back what the caller may see. The
        // read is a list in front of `ratify`/`retire` and adds no mutation of
        // its own — `web-control-plane.md` §8 is explicit that a web-specific
        // curation handler would reopen the double-ratification race the
        // existing compare-and-swap statements close (FR-889a).
        .route(
            "/api/team/knowledge",
            get(crate::global::team_knowledge_view).post(crate::commands::propose_team),
        )
        .route(
            "/api/memories/{id}/forget",
            post(crate::commands::forget_memory),
        )
        // One authenticated route for every queued command, dispatching
        // internally to the handlers above. Not a second implementation of
        // command semantics: a second *way in* to the same ones, carrying the
        // deterministic `command_id` the per-command paths have nowhere to put.
        .route("/api/commands", post(crate::commands::command_envelope))
        // The pattern lifecycle (T085). Promotion is an upsert on
        // `(owner_user_id, content_key)`, so posting the same pattern twice is
        // one record; the list is the owner's own and takes no parameter
        // through which another account could be named.
        //
        // There is no route here that widens a pattern to a team. Widening is a
        // separate, explicit act with its own governance — the owner proposes
        // the content through `POST /api/team/knowledge` and a human
        // administrator ratifies it — and the personal pattern stays owner-only
        // and stays in the personal domain (FR-708e, Constitution V).
        .route(
            "/api/patterns",
            get(crate::commands::list_patterns).post(crate::commands::promote_pattern),
        )
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
        // The web control plane's project-scoped reads (T108, T109). Every one
        // of them calls `require_member` before its query, so a non-member is
        // refused rather than handed an empty list — an empty list would tell a
        // non-member the project exists and would make a missing guard
        // undetectable (FR-894a).
        //
        // `integration-health` is a second path onto `read_health`'s own query
        // and not a second implementation of it: the agents screen and US6's
        // status ask the same question, and the only thing that differed was the
        // envelope key each audience already depends on.
        .route("/api/projects/{id}/funnel", get(project_funnel))
        .route("/api/projects/{id}/activity", get(project_activity))
        .route(
            "/api/projects/{id}/consolidation-runs",
            get(project_consolidation_runs),
        )
        .route(
            "/api/projects/{id}/retrieval-traces",
            get(project_retrieval_traces),
        )
        .route(
            "/api/projects/{id}/integration-health",
            get(project_integration_health),
        )
        // Deployment-wide rather than project-scoped, so the gate is the role
        // and not a membership. `AdminUser` in the parameter list is the
        // authorization: a member reaching this route would be reading across
        // every project on the server (FR-891).
        .route("/api/system/health", get(system_health))
        // Consolidation's own backlog, readable while a pass is running and
        // immediately after a restart, because every field behind it is a
        // committed row rather than worker state (SC-748, FR-793c).
        .route("/api/consolidation/health", get(consolidation_health))
        // Retrieval, its trace, and the outcome of actually transmitting it.
        // Three routes and not one: generating a briefing, reading back what
        // was selected, and reporting what reached the agent are three
        // different claims, and collapsing them would let the first stand in
        // for the third (FR-843, FR-854).
        .route("/api/retrieve", post(retrieve_context))
        .route("/api/retrieval-traces/{trace_id}", get(retrieval_trace))
        .route(
            "/api/retrieval-traces/{trace_id}/transmission",
            post(retrieval_transmission),
        )
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
    let cells = integration_health_rows(&state.pool, project_id).await?;
    Ok(Json(json!({ "cells": cells })))
}

/// The matrix rows for one project — the single read both health surfaces use.
///
/// Extracted rather than copied because US5's agents screen and US6's status
/// output ask the *same* question of the same table, and a second query is a
/// second place for "which columns a matrix cell has" to drift. What kept them
/// apart was only the envelope key their two audiences already depend on
/// (`cells` on `/health`, `rows` on `/integration-health`), and an envelope is
/// not a reason to have two queries.
///
/// It synthesizes nothing. A capability with no row has never been reported,
/// which is a different state from a capability reported as `no_evidence`, and
/// filling the gap here would erase the distinction FR-855 draws.
async fn integration_health_rows(pool: &sqlx::PgPool, project_id: Uuid) -> ApiResult<Vec<Value>> {
    let rows = sqlx::query(
        "SELECT writer_id, agent, capability, stage, status, evidence_kind,
                observed_at, degraded
           FROM integration_health
          WHERE project_id = $1
          ORDER BY agent, capability, stage",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
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
        .collect())
}

/// `GET /api/projects/{id}/integration-health` — the agents screen (FR-887).
///
/// Reads through [`integration_health_rows`], which is `read_health`'s own
/// query: the path is new because `web-control-plane.md` §2 names it, and the
/// implementation is not, because a second one would be a second answer to
/// "which capabilities are working".
///
/// `stale` is deliberately absent from the row. §5 computes it client-side from
/// `observed_at` against a per-capability freshness window, and a server that
/// baked one window in would be asserting that every capability goes stale at
/// the same rate. What the row owes the view is the observation time and the
/// machine it came from (FR-857, FR-860); the judgement is the view's.
async fn project_integration_health(
    State(state): State<AppState>,
    user: SettledUser,
    Path(project_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, project_id, user.id()).await?;
    Ok(Json(json!({
        "rows": integration_health_rows(&state.pool, project_id).await?,
    })))
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

/// Generate a briefing for one session, and trace it.
///
/// The account comes from the credential and the project from the session; the
/// body names neither, and a caller that could name them could retrieve against
/// a project it has nothing to do with (FR-769, Principle XI).
async fn retrieve_context(
    State(state): State<AppState>,
    user: SettledUser,
    Json(body): Json<crate::retrieve::RetrieveRequest>,
) -> ApiResult<Json<Value>> {
    let reader = auth::ReaderContext::load(&state.pool, &user.0).await?;
    let config = cairn_core::CairnConfig::default();
    let answer = crate::retrieve::retrieve(
        &state.pool,
        &reader,
        &body,
        config.context_budget_tokens,
        config.context_deadline_ms as u128,
    )
    .await?;
    Ok(Json(
        serde_json::to_value(answer).unwrap_or_else(|_| json!({})),
    ))
}

/// What a retrieval considered and selected, filtered to this reader.
async fn retrieval_trace(
    State(state): State<AppState>,
    user: SettledUser,
    Path(trace_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let reader = auth::ReaderContext::load(&state.pool, &user.0).await?;
    Ok(Json(
        crate::retrieve::trace_detail(&state.pool, &reader, trace_id).await?,
    ))
}

/// What actually happened to a generated briefing.
///
/// The smallest boundary that can carry the fact: a server-issued `trace_id` in
/// the path and a bounded outcome in the body. Nothing else is accepted,
/// because everything else the server already holds and everything beyond that
/// is authority a caller must not assert.
async fn retrieval_transmission(
    State(state): State<AppState>,
    user: SettledUser,
    Path(trace_id): Path<Uuid>,
    Json(body): Json<crate::retrieve::TransmissionReport>,
) -> ApiResult<Json<Value>> {
    let reader = auth::ReaderContext::load(&state.pool, &user.0).await?;
    Ok(Json(
        crate::retrieve::report_transmission(&state.pool, &reader, trace_id, &body).await?,
    ))
}

/// How far behind consolidation is, for any authenticated caller.
///
/// Not project-scoped, and deliberately: the backlog is a property of the
/// deployment's single consolidation task, not of any one project, and every
/// field it reports is a count or a timestamp — no content, no keys, nothing
/// that names what any project knows. Scoping it per project would suggest a
/// per-project worker that does not exist.
async fn consolidation_health(
    State(state): State<AppState>,
    _user: SettledUser,
) -> ApiResult<Json<Value>> {
    let health = crate::consolidate::health(&state.pool).await?;
    Ok(Json(
        serde_json::to_value(health).unwrap_or_else(|_| json!({})),
    ))
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
    /// Accepted so the Domains screen can say which panel it is asking for, and
    /// refused for any value but `project`.
    ///
    /// **Refused rather than ignored.** Before this field existed the parameter
    /// was silently dropped, which meant `?domain=personal` returned this
    /// project's memories and read to the caller exactly like a personal feed.
    /// A route that answers a question it was not asked is worse than one that
    /// refuses: personal and team knowledge are not project-scoped and are
    /// reached at their own routes, and there is deliberately no value of this
    /// parameter that reaches them from here.
    #[serde(default)]
    domain: Option<String>,
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
    if let Some(domain) = q.domain.as_deref() {
        if domain != "project" {
            return Err(ApiError::invalid(format!(
                "`{domain}` is not a domain this route can answer for; a project's \
                 memories are the project domain, and personal and team knowledge \
                 are read at `/api/personal/knowledge` and `/api/team/knowledge`"
            )));
        }
    }
    let limit = q
        .limit
        .unwrap_or(crate::global::VIEW_PAGE_DEFAULT)
        .clamp(1, crate::global::VIEW_PAGE_MAX);
    let want_state = q.state.unwrap_or_else(|| "active".to_string());

    let rows = sqlx::query(
        "SELECT m.*,
                CASE m.scope WHEN 'task' THEN 0 WHEN 'branch' THEN 1
                             WHEN 'project' THEN 2 ELSE 3 END AS scope_bucket,
                CASE WHEN $2::text IS NULL OR $2 = '' THEN 0
                     ELSE ts_rank(to_tsvector('english', m.content),
                                  plainto_tsquery('english', $2)) END AS relevance,
                (SELECT COUNT(*) FROM memory_relations rel
                  WHERE rel.deleted_at IS NULL
                    AND (rel.from_memory_id = m.id OR rel.to_memory_id = m.id))
                  AS relation_count
         FROM memories m
         WHERE m.project_id = $1
           AND m.deleted_at IS NULL
           AND m.state = $3
           AND ($2::text IS NULL OR $2 = ''
                OR to_tsvector('english', m.content) @@ plainto_tsquery('english', $2))
           AND ($4::text IS NULL OR m.scope = $4)
           AND ($5::text IS NULL OR m.scope_key = $5)
           AND ($6::text IS NULL OR m.type = $6)
         ORDER BY scope_bucket ASC, relevance DESC, m.created_at DESC
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
    Ok(Json(json!({
        "memories": memories,
        "total": memories.len(),
        // The bound this page was actually taken under. `total` is how many
        // came back, which is the same number for a full page and for a project
        // with exactly that many memories — a client cannot tell "there is more"
        // from it, and `limit` is what makes the two distinguishable (FR-895).
        "limit": limit,
    })))
}

/// One memory as both the explorer and the detail view start from.
///
/// The six fields below `superseded_by_id` are what FR-883 asks the explorer to
/// filter and sort on, and they are added here rather than only on the detail
/// route because an explorer that had to open every record to learn its
/// verification state is not an explorer.
///
/// `relation_count` is a computed column and not one of `memories`, so it is
/// read leniently: the two queries that call this both provide it, and a third
/// that forgot should report "no relations counted" rather than fail the whole
/// page. That is the one field here where absence is a defensible answer —
/// everything else is a column the table guarantees.
fn memory_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.get::<Uuid, _>("id"),
        "type": r.get::<String, _>("type"),
        "scope": r.get::<String, _>("scope"),
        "scope_key": r.get::<String, _>("scope_key"),
        "content": r.get::<String, _>("content"),
        "state": r.get::<String, _>("state"),
        "superseded_by_id": r.get::<Option<Uuid>, _>("superseded_by_id"),
        "importance": r.get::<String, _>("importance"),
        "pinned": r.get::<bool, _>("pinned"),
        "verification": r.get::<Option<String>, _>("verification"),
        "verification_authority": r.get::<Option<String>, _>("verification_authority"),
        // FR-885: whether somebody asked for this record or consolidation
        // produced it. Nullable because rows written before migration 4 have no
        // answer, and inventing `explicit` for them would assert an origin
        // nobody recorded.
        "origin_kind": r.get::<Option<String>, _>("origin_kind"),
        "reinforcement_count": r.get::<i32, _>("reinforcement_count"),
        "relation_count": r.try_get::<i64, _>("relation_count").unwrap_or(0),
        "provenance": {
            "session_id": r.get::<Uuid, _>("origin_session_id"),
            "observation_ids": r.get::<Value, _>("observation_ids"),
            "evidence_count": r.get::<i32, _>("evidence_count"),
        },
        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        "updated_at": r.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
    })
}

/// How many retrievals a memory's detail view carries inline (§7).
///
/// Twenty, with no further pagination on the embed, because the full history is
/// reachable through the traces list filtered by this memory's reference. An
/// unbounded embed would make one record's detail page grow with how often the
/// project retrieves it, which is exactly backwards — the more useful a memory
/// is, the slower its page would load (FR-895).
const RETRIEVAL_USAGE_LIMIT: i64 = 20;

/// `GET /api/memories/{id}` — everything FR-884 asks a reader to be able to
/// determine about one record (T109).
///
/// Seven questions, and each is answered by a field rather than by inference:
/// what it says, where it came from, what evidence supports it, whether it is
/// verified, what it supersedes, what conflicts with or reinforces it, and where
/// it has been retrieved.
///
/// **The evidence summary carries no evidence.** Counts, the session that holds
/// the material, and the verifier *kinds* that have looked at it — never
/// content, never a path, never command output. That is not a redaction applied
/// here: the server has never held any of it (FR-055, FR-061, FR-893). The view
/// states the material is local rather than rendering an empty section, which is
/// why `local_to_session` travels beside `content_available: false`.
async fn memory_detail(
    State(state): State<AppState>,
    user: SettledUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let row = sqlx::query(
        "SELECT m.*,
                (SELECT COUNT(*) FROM memory_relations rel
                  WHERE rel.deleted_at IS NULL
                    AND (rel.from_memory_id = m.id OR rel.to_memory_id = m.id))
                  AS relation_count
           FROM memories m WHERE m.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("no such memory"))?;

    // **One answer to both questions**, and `require_member` was the wrong
    // guard here (found while building US5's reads).
    //
    // This route is *record-addressed*: the caller names a memory, and whether
    // it exists is precisely what must not leak. `require_member` refuses with
    // `403`, and a missing row with `404` — so anyone with an account could sort
    // memory ids into "real" and "not real", one guess at a time, without being
    // a member of anything. That is the enumeration oracle FR-894a closes and
    // that `feature005_authorization_audit` already states as a rule; the audit
    // enforces it by scanning `commands.rs`, and this route sat outside the
    // scan.
    //
    // `project_of_record` funnels both cases through one `404`, exactly as the
    // record-addressed commands do. The asymmetry with the project-addressed
    // reads is deliberate rather than an inconsistency: there the caller *named*
    // the project, so a `403` discloses nothing they did not already supply.
    let project_id: Uuid = row.try_get("project_id")?;
    crate::commands::project_of_record(&state.pool, "memories", id, user.id()).await?;

    // Provenance is references; evidence content is local to the machine that
    // captured it and does not exist here (FR-055, FR-061).
    let mut value = memory_json(&row);
    value["provenance"]["evidence_content_available"] = json!(false);

    let origin_session: Uuid = row.get("origin_session_id");
    value["evidence_summary"] = json!({
        "observation_count": row
            .get::<Value, _>("observation_ids")
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0),
        "evidence_count": row.get::<i32, _>("evidence_count"),
        "evidence_fact_count": row.get::<i32, _>("evidence_fact_count"),
        // Verifier *kinds*, which is what the basis column holds: a name for
        // the sort of check that ran, never the subject it ran against or what
        // it observed (FR-502, D66).
        "verifier_kinds": row.get::<Value, _>("verification_basis"),
        "content_available": false,
        "local_to_session": origin_session,
    });

    // FR-884's "whether it is verified", as a state with the authority that
    // established it. `stale` is the record's own expiry having passed, or its
    // state having been moved to `stale` — a verification that was true last
    // quarter is not a verification now, and reporting only the state would say
    // it is (FR-860's rule, applied to knowledge rather than to a capability).
    let stale_at = row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("stale_at");
    value["verification"] = json!({
        "state": row.get::<Option<String>, _>("verification"),
        "authority": row.get::<Option<String>, _>("verification_authority"),
        "last_verified_at": row
            .get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_verified_at")
            .map(|t| t.to_rfc3339()),
        "stale": row.get::<String, _>("state") == "stale"
            || stale_at.is_some_and(|t| t <= chrono::Utc::now()),
    });

    value["relations"] = json!(memory_relations(&state.pool, id).await?);
    value["retrieval_usage"] = json!(retrieval_usage(&state.pool, project_id, id).await?);
    Ok(Json(json!({ "memory": value })))
}

/// Both halves of the relation graph around one memory (FR-884).
///
/// **Both directions, and the direction is stated.** FR-884 asks two different
/// questions — what this record supersedes, and what reinforces it — and a list
/// of only the outgoing edges answers one of them. Which end of the edge this
/// memory is on decides which question the row answers, so it travels as a
/// field rather than being left for a reader to work out from the ids.
///
/// The other end is a complete `KnowledgeRef`, never a bare UUID. A relation
/// always joins two project memories (`memory_relations` has a `project_id` and
/// both endpoints are `memories` rows), so the domain is known — and writing it
/// out is what makes the reference resolvable by a client that holds nothing
/// else (SC-766).
async fn memory_relations(pool: &sqlx::PgPool, id: Uuid) -> ApiResult<Vec<Value>> {
    use cairn_core::domain::{KnowledgeRef, Reference};
    let rows = sqlx::query(
        "SELECT CASE WHEN from_memory_id = $1 THEN 'outgoing' ELSE 'incoming' END AS direction,
                CASE WHEN from_memory_id = $1 THEN to_memory_id ELSE from_memory_id END AS other,
                kind, basis, decided_by_session, decided_at
           FROM memory_relations
          WHERE deleted_at IS NULL AND (from_memory_id = $1 OR to_memory_id = $1)
          ORDER BY decided_at DESC, kind",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            json!({
                "direction": r.get::<String, _>("direction"),
                "kind": r.get::<String, _>("kind"),
                "basis": r.get::<String, _>("basis"),
                "decided_by_session": r.get::<Uuid, _>("decided_by_session"),
                "decided_at": r
                    .get::<chrono::DateTime<chrono::Utc>, _>("decided_at")
                    .to_rfc3339(),
                "other": reference_json(Reference::Knowledge(KnowledgeRef::project(
                    r.get::<Uuid, _>("other"),
                ))),
            })
        })
        .collect())
}

/// Where and when this memory has actually been retrieved (FR-884).
///
/// Bounded at [`RETRIEVAL_USAGE_LIMIT`] and scoped to the project the memory
/// belongs to. The project scope is not redundant with the caller's membership:
/// the caller was already checked against *this* memory's project, and a trace
/// in some other project that happened to reference the same id would be a row
/// the caller has no standing over. Filtering here is what stops the embed from
/// widening the guard the route already applied.
///
/// The reference is matched on `reference_key`, the generated column, so a
/// personal or team record that happens to share this UUID cannot match — which
/// is precisely the collision `reference_key` exists to prevent (SC-766).
async fn retrieval_usage(pool: &sqlx::PgPool, project_id: Uuid, id: Uuid) -> ApiResult<Vec<Value>> {
    use cairn_core::domain::{KnowledgeRef, Reference};
    let key = Reference::Knowledge(KnowledgeRef::project(id)).reference_key();
    let rows = sqlx::query(
        "SELECT t.trace_id, t.trigger, t.delivery_point, t.delivery_state,
                t.session_id, t.created_at, i.status, i.rank
           FROM retrieval_trace_items i
           JOIN retrieval_traces t ON t.trace_id = i.trace_id
          WHERE i.reference_key = $1 AND t.project_id = $2
          ORDER BY t.created_at DESC, t.trace_id DESC
          LIMIT $3",
    )
    .bind(&key)
    .bind(project_id)
    .bind(RETRIEVAL_USAGE_LIMIT)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            json!({
                "trace_id": r.get::<Uuid, _>("trace_id"),
                "session_id": r.get::<Uuid, _>("session_id"),
                "trigger": r.get::<String, _>("trigger"),
                "delivery_point": r.get::<String, _>("delivery_point"),
                "delivery_state": r.get::<String, _>("delivery_state"),
                "status": r.get::<String, _>("status"),
                "rank": r.get::<Option<i32>, _>("rank"),
                "at": r
                    .get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                    .to_rfc3339(),
            })
        })
        .collect())
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

// ---------------------------------------------------------------------------
// The web control plane's read API (T108, T109,
// `contracts/web-control-plane.md`, FR-879-FR-895)
// ---------------------------------------------------------------------------

/// The server schema at which the autonomous-memory tables exist.
///
/// Named rather than written as `4` at each use, because what the number means
/// is "the migration that created `safe_events`, `consolidation_runs`,
/// `knowledge_candidates`, `retrieval_traces` and `capture_dispositions`" — and
/// that is the fact the funnel below is deciding on, not a version.
const AUTONOMOUS_MEMORY_SCHEMA: i64 = 4;

/// The widest window a caller may ask the funnel for, in days.
///
/// A ceiling rather than a refusal, matching every other bound in this
/// contract. Without one, `?days=100000000` is an unindexed scan of every table
/// the funnel touches, asked for by anybody with an account.
const FUNNEL_MAX_DAYS: i64 = 365;

/// FR-879's twelve stages, in FR-879's order.
///
/// **The order is part of the contract, not a rendering preference.** The
/// funnel is read as a funnel: events arrive before candidates exist, and
/// candidates before knowledge does, so a stage appearing out of sequence
/// misreads as a stage that lost fewer records than it did. The array is the
/// single statement of both the names and the order (SC-728).
const FUNNEL_STAGES: [&str; 12] = [
    "active_agents",
    "sessions",
    "safe_events_received",
    "capture_failures",
    "consolidation_runs",
    "candidates_produced",
    "knowledge_accepted",
    "candidates_rejected_or_duplicate",
    "reinforcements",
    "conflicts",
    "retrievals",
    "delivery_failures",
];

/// The eleven stages the autonomous-memory migration is what makes countable.
///
/// `sessions` is not among them: it predates this feature, so it is a real
/// count on every deployment that can answer the route at all. Everything else
/// reads a table migration 4 created, and on a deployment below it the honest
/// answer is that the count cannot be established — never zero (FR-880).
const SCHEMA_4_STAGES: [&str; 11] = [
    "active_agents",
    "safe_events_received",
    "capture_failures",
    "consolidation_runs",
    "candidates_produced",
    "knowledge_accepted",
    "candidates_rejected_or_duplicate",
    "reinforcements",
    "conflicts",
    "retrievals",
    "delivery_failures",
];

/// Every schema-4 stage in one statement.
///
/// One round trip rather than eleven: a dashboard's first paint asks for all of
/// them at once, and eleven sequential counts against one project is eleven
/// times the latency for the same answer. Written out per stage rather than
/// generated, because each line is a *definition* — `knowledge_accepted` counts
/// `'accepted'` and deliberately not `'reinforced'` (FR-798a), and a generated
/// query would hide that decision behind a loop.
///
/// `knowledge_candidates` has no timestamp of its own, so the window is applied
/// to the run that produced it. That is the honest reading: a candidate happened
/// when its consolidation pass ran.
const FUNNEL_SQL: &str = "\
SELECT
  (SELECT COUNT(DISTINCT agent) FROM safe_events
    WHERE project_id = $1
      AND ($2::int IS NULL OR received_at >= now() - make_interval(days => $2)))
    AS active_agents,
  (SELECT COUNT(*) FROM safe_events
    WHERE project_id = $1
      AND ($2::int IS NULL OR received_at >= now() - make_interval(days => $2)))
    AS safe_events_received,
  (SELECT COALESCE(SUM(n), 0)::bigint FROM capture_dispositions
    WHERE project_id = $1 AND disposition = 'capture_deadline_exceeded'
      AND ($2::int IS NULL OR day >= (now() - make_interval(days => $2))::date))
    AS capture_failures,
  (SELECT COUNT(*) FROM consolidation_runs
    WHERE project_id = $1
      AND ($2::int IS NULL OR started_at >= now() - make_interval(days => $2)))
    AS consolidation_runs,
  (SELECT COUNT(*) FROM knowledge_candidates c JOIN consolidation_runs r ON r.run_id = c.run_id
    WHERE c.project_id = $1
      AND ($2::int IS NULL OR r.started_at >= now() - make_interval(days => $2)))
    AS candidates_produced,
  (SELECT COUNT(*) FROM knowledge_candidates c JOIN consolidation_runs r ON r.run_id = c.run_id
    WHERE c.project_id = $1 AND c.decision = 'accepted'
      AND ($2::int IS NULL OR r.started_at >= now() - make_interval(days => $2)))
    AS knowledge_accepted,
  (SELECT COUNT(*) FROM knowledge_candidates c JOIN consolidation_runs r ON r.run_id = c.run_id
    WHERE c.project_id = $1 AND c.decision IN ('refused', 'duplicate')
      AND ($2::int IS NULL OR r.started_at >= now() - make_interval(days => $2)))
    AS candidates_rejected_or_duplicate,
  (SELECT COUNT(*) FROM knowledge_candidates c JOIN consolidation_runs r ON r.run_id = c.run_id
    WHERE c.project_id = $1 AND c.decision = 'reinforced'
      AND ($2::int IS NULL OR r.started_at >= now() - make_interval(days => $2)))
    AS reinforcements,
  (SELECT COUNT(*) FROM knowledge_candidates c JOIN consolidation_runs r ON r.run_id = c.run_id
    WHERE c.project_id = $1 AND c.decision = 'conflicted'
      AND ($2::int IS NULL OR r.started_at >= now() - make_interval(days => $2)))
    AS conflicts,
  (SELECT COUNT(*) FROM retrieval_traces
    WHERE project_id = $1
      AND ($2::int IS NULL OR created_at >= now() - make_interval(days => $2)))
    AS retrievals,
  (SELECT COUNT(*) FROM retrieval_traces
    WHERE project_id = $1 AND delivery_state = 'failed'
      AND ($2::int IS NULL OR created_at >= now() - make_interval(days => $2)))
    AS delivery_failures";

#[derive(Deserialize)]
struct FunnelQuery {
    /// How far back to count. Absent means the project's whole history, which
    /// is what a dashboard shows before anybody narrows it.
    #[serde(default)]
    days: Option<i64>,
}

/// `GET /api/projects/{id}/funnel` — the dashboard's twelve stages (FR-879,
/// FR-880, SC-728).
///
/// **`count` is nullable, and the two answers it distinguishes are not
/// cosmetic.** `0` means the query ran against the mechanism and found nothing
/// happened. `null` means the mechanism does not exist on this deployment, so
/// nothing can be said either way. Collapsing them reports "nothing happened"
/// where the truth is "nobody looked", and an operator acts differently on
/// those two: one is a quiet project, the other is a deployment that has not
/// been migrated. The dashboard renders `0` as the number and `null` as an
/// em dash (FR-880).
///
/// The distinction is decided from `state.schema_version` before any statement
/// is built, rather than from a query that errored. A caught error would make
/// "unavailable" indistinguishable from "the database was briefly unreachable",
/// which is the same conflation one level down.
///
/// What would falsify this: a deployment below schema 4 reporting `0` for a
/// stage whose table does not exist, or a project with no events reporting
/// `null` for `safe_events_received`.
async fn project_funnel(
    State(state): State<AppState>,
    user: SettledUser,
    Path(project_id): Path<Uuid>,
    Query(q): Query<FunnelQuery>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, project_id, user.id()).await?;
    let window = q.days.map(|d| d.clamp(1, FUNNEL_MAX_DAYS) as i32);

    let mut counts: std::collections::BTreeMap<&str, Option<i64>> =
        FUNNEL_STAGES.iter().map(|s| (*s, None)).collect();

    // `sessions` is answerable on every schema this route can be reached on.
    let sessions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions
          WHERE project_id = $1 AND deleted_at IS NULL
            AND ($2::int IS NULL OR started_at >= now() - make_interval(days => $2))",
    )
    .bind(project_id)
    .bind(window)
    .fetch_one(&state.pool)
    .await?;
    counts.insert("sessions", Some(sessions));

    if state.schema_version >= AUTONOMOUS_MEMORY_SCHEMA {
        let row = sqlx::query(FUNNEL_SQL)
            .bind(project_id)
            .bind(window)
            .fetch_one(&state.pool)
            .await?;
        for stage in SCHEMA_4_STAGES {
            counts.insert(stage, Some(row.get::<i64, _>(stage)));
        }
    }

    let stages: Vec<Value> = FUNNEL_STAGES
        .iter()
        .map(|stage| json!({ "stage": stage, "count": counts[stage] }))
        .collect();
    Ok(Json(json!({
        "window_days": window,
        "stages": stages,
    })))
}

// ---------------------------------------------------------------------------
// The activity feed (FR-881, FR-882)
// ---------------------------------------------------------------------------

/// The default page for the activity feed. Wider than the other lists because
/// the feed is read by scrolling rather than by picking a row (§7).
const ACTIVITY_PAGE_DEFAULT: i64 = 50;

/// The seven event kinds the feed shows without being asked (§4).
///
/// **Declared here rather than derived from a rule**, because FR-882 forbids
/// leaving "low-value" to the implementation. Each of these marks a session
/// boundary, a durable artifact change, a test outcome, an explicit decision or
/// a capture failure — one meaningful thing each. The fourteen excluded kinds
/// fire once per tool call or internal transition and would make the feed a
/// record of Cairn's own bookkeeping rather than of what it is learning. They
/// are one query parameter away, never gone.
const DEFAULT_EVENT_KINDS: [&str; 7] = [
    "session_opened",
    "session_resumed",
    "session_closed",
    "file_changed",
    "test_result",
    "decision_signal",
    "capture_failed",
];

/// The candidate decisions the feed shows without being asked (§4).
///
/// `reinforced`, `duplicate` and `refused` are excluded for the same reason the
/// firehose event kinds are: reinforcement already has its own funnel stage, and
/// a duplicate is the pipeline working rather than something happening.
const DEFAULT_DECISIONS: [&str; 2] = ["accepted", "conflicted"];

/// The closed vocabulary `knowledge_candidates.decision` is CHECKed to.
///
/// Restated here so a `kinds` parameter naming a decision is validated against
/// the same list the column enforces, and an unrecognized one is refused by
/// name rather than by silently matching nothing.
const CANDIDATE_DECISIONS: [&str; 5] = [
    "accepted",
    "reinforced",
    "duplicate",
    "conflicted",
    "refused",
];

#[derive(Deserialize)]
struct ActivityQuery {
    /// A comma-separated subset of the twenty-one event kinds and the five
    /// candidate decisions. Absent is the declared default above; the UI's
    /// "show everything" control sends the full set explicitly.
    #[serde(default)]
    kinds: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

/// Split a `kinds` parameter into the two families it can name.
///
/// **Refused rather than ignored.** A name that matches neither an event kind
/// nor a decision is a 400 with the name in it, because a parameter the server
/// silently drops reads to the caller exactly like one it honoured — the client
/// would render "no matching activity" for what is actually a typo.
///
/// The event half is validated through `EventKind::from_str`, so the accepted
/// set is the canonical twenty-one and stays that way when a twenty-second is
/// added. A hand-written list here would be a second vocabulary.
fn split_activity_kinds(raw: Option<&str>) -> ApiResult<(Vec<String>, Vec<String>)> {
    let Some(raw) = raw else {
        return Ok((
            DEFAULT_EVENT_KINDS.iter().map(|k| k.to_string()).collect(),
            DEFAULT_DECISIONS.iter().map(|k| k.to_string()).collect(),
        ));
    };
    let mut events = Vec::new();
    let mut decisions = Vec::new();
    for token in raw.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        if cairn_core::event::EventKind::from_str(token).is_ok() {
            events.push(token.to_string());
        } else if CANDIDATE_DECISIONS.contains(&token) {
            decisions.push(token.to_string());
        } else {
            return Err(ApiError::invalid(format!(
                "`{token}` is neither an event kind nor a candidate decision"
            )));
        }
    }
    if events.is_empty() && decisions.is_empty() {
        return Err(ApiError::invalid(
            "`kinds` names nothing; omit it for the default subset",
        ));
    }
    Ok((events, decisions))
}

/// Both families, interleaved by time, newest first.
///
/// **A `UNION ALL` over one keyset rather than two lists merged in Rust.** The
/// page bound has to apply to the interleaved result: taking fifty of each and
/// merging would return a hundred rows for a page of fifty, and dropping half
/// of them would leave a cursor that skips whatever was dropped.
///
/// A candidate decision has no timestamp of its own — `knowledge_candidates`
/// records what was decided, not when — so it is placed at its run's finish, or
/// at its start if the run never finished. That is when the decision happened.
///
/// `content` travels for safe events and is `NULL` for decisions. The event's
/// content is the approved per-kind structure and nothing else — `safe_events`
/// has no column a transcript could land in, and every free-text field the
/// structure does have was put through `events::screen_event_text` before the
/// row existed. So this hands a project member exactly what the server accepted
/// for their project, and a feed that said `file_changed` without saying which
/// file would be withholding the only part that is actually semantic. A candidate's `content` is a *claim*, which is the memory
/// explorer's business and carries a domain this feed would have to authorize
/// per row; the reference to it travels instead.
const ACTIVITY_SQL: &str = "\
WITH arrivals AS (
    SELECT 'safe_event'::text          AS family,
           event_id                    AS id,
           received_at                 AS at,
           kind                        AS kind,
           agent                       AS agent,
           session_id                  AS session_id,
           content                     AS content,
           NULL::text                  AS refusal_reason,
           NULL::text                  AS ref_kind,
           NULL::text                  AS ref_domain,
           NULL::uuid                  AS ref_id
      FROM safe_events
     WHERE project_id = $1 AND kind = ANY($2)
), decisions AS (
    SELECT 'candidate_decision'::text  AS family,
           c.candidate_id              AS id,
           COALESCE(r.finished_at, r.started_at) AS at,
           c.decision                  AS kind,
           NULL::text                  AS agent,
           r.session_id                AS session_id,
           NULL::jsonb                 AS content,
           c.refusal_reason            AS refusal_reason,
           c.result_ref_kind           AS ref_kind,
           c.result_domain             AS ref_domain,
           c.result_knowledge_id       AS ref_id
      FROM knowledge_candidates c
      JOIN consolidation_runs r ON r.run_id = c.run_id
     WHERE c.project_id = $1 AND c.decision = ANY($3)
)
SELECT * FROM (SELECT * FROM arrivals UNION ALL SELECT * FROM decisions) feed
 WHERE ($4::timestamptz IS NULL OR (feed.at, feed.id) < ($4, $5::uuid))
 ORDER BY feed.at DESC, feed.id DESC
 LIMIT $6";

/// `GET /api/projects/{id}/activity` — recent activity at a semantic level
/// (FR-881, FR-882).
async fn project_activity(
    State(state): State<AppState>,
    user: SettledUser,
    Path(project_id): Path<Uuid>,
    Query(q): Query<ActivityQuery>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, project_id, user.id()).await?;
    let (events, decisions) = split_activity_kinds(q.kinds.as_deref())?;
    let limit = q
        .limit
        .unwrap_or(ACTIVITY_PAGE_DEFAULT)
        .clamp(1, crate::global::VIEW_PAGE_MAX);
    let (at, id) = crate::global::PageCursor::descending_bound(
        crate::global::PageCursor::decode_opt(q.cursor.as_deref()),
    );

    let rows = sqlx::query(ACTIVITY_SQL)
        .bind(project_id)
        .bind(&events)
        .bind(&decisions)
        .bind(at)
        .bind(id)
        .bind(limit)
        .fetch_all(&state.pool)
        .await?;

    let reader = auth::ReaderContext::load(&state.pool, &user.0).await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in &rows {
        // The reference a decision produced, resolved to a *complete* one or to
        // nothing. A consolidation pass in a project can produce a personal
        // record, and that record's audience is its owner, not this project's
        // members — so the reference is withheld exactly as a retrieval trace
        // withholds one (FR-846a). The decision itself still travels: that a
        // pass accepted something is a project fact.
        let reference = match reference_from_columns(
            row.get::<Option<String>, _>("ref_kind").as_deref(),
            row.get::<Option<String>, _>("ref_domain").as_deref(),
            row.get::<Option<Uuid>, _>("ref_id"),
        ) {
            Some(reference)
                if auth::reference_visibility(&state.pool, &reader, reference).await?
                    == auth::Visibility::Visible =>
            {
                reference_json(reference)
            }
            _ => Value::Null,
        };
        items.push(json!({
            "family": row.get::<String, _>("family"),
            "id": row.get::<Uuid, _>("id"),
            "at": row.get::<chrono::DateTime<chrono::Utc>, _>("at").to_rfc3339(),
            "kind": row.get::<String, _>("kind"),
            "agent": row.get::<Option<String>, _>("agent"),
            "session_id": row.get::<Option<Uuid>, _>("session_id"),
            "content": row.get::<Option<Value>, _>("content"),
            "refusal_reason": row.get::<Option<String>, _>("refusal_reason"),
            "reference": reference,
        }));
    }

    Ok(Json(json!({
        "items": items,
        "cursor": view_cursor(&rows, limit, "at", "id"),
        "limit": limit,
        // What the feed actually applied. FR-882 wants the default *declared*,
        // and a client that had to infer it from what arrived could not tell an
        // excluded kind from a kind nothing has produced yet.
        "kinds": events.iter().chain(decisions.iter()).collect::<Vec<_>>(),
    })))
}

/// The three columns a stored reference occupies, back to one value.
///
/// Returns `None` for a combination the reference grammar has no name for,
/// which is also what the table's own CHECK refuses — a `knowledge` row with no
/// domain, or a `pattern` row with one. Nothing is guessed: a reference that
/// cannot say which domain it means is not a reference, and emitting the bare
/// id would hand a reader something two domains could answer to (SC-766).
fn reference_from_columns(
    ref_kind: Option<&str>,
    domain: Option<&str>,
    id: Option<Uuid>,
) -> Option<cairn_core::domain::Reference> {
    use cairn_core::domain::{KnowledgeRef, PatternRef, Reference};
    let id = id?;
    match (ref_kind?, domain) {
        ("pattern", None) => Some(Reference::Pattern(PatternRef(id))),
        ("knowledge", Some("project")) => Some(Reference::Knowledge(KnowledgeRef::project(id))),
        ("knowledge", Some("personal")) => Some(Reference::Knowledge(KnowledgeRef::personal(id))),
        ("knowledge", Some("team")) => Some(Reference::Knowledge(KnowledgeRef::team(id))),
        _ => None,
    }
}

/// One reference, in the discriminated form every control-plane response uses.
///
/// All four fields, including the `reference_key` the database already computes
/// as a generated column. The three parts are what a client filters and routes
/// on; the key is what it compares, and shipping only the parts would make
/// every consumer re-derive a string the server already has — and get the
/// pattern case wrong, which omits the domain component rather than writing
/// `personal` into it.
fn reference_json(reference: cairn_core::domain::Reference) -> Value {
    use cairn_core::domain::Reference;
    let (ref_kind, domain, id) = match reference {
        Reference::Knowledge(k) => ("knowledge", Some(k.domain.as_str()), k.id),
        Reference::Pattern(p) => ("pattern", None, p.0),
    };
    json!({
        "ref_kind": ref_kind,
        "domain": domain,
        "knowledge_id": id,
        "reference_key": reference.reference_key(),
    })
}

/// The cursor a descending page hands back, or `None` at the end of the feed.
/// Takes the two column names because these lists order on `(at, id)`,
/// `(started_at, run_id)`, `(created_at, trace_id)` and `(changed_at, id)` —
/// four spellings of one convention, which is a reason to pass the names rather
/// than to write the function four times. The encoding is
/// `global::PageCursor`'s own, so a cursor from any of these lists is read back
/// by the same parser.
///
/// A short page ends the feed. Handing a position back on one would make every
/// list appear to have one more page that turns out to be empty.
pub(crate) fn view_cursor(
    rows: &[sqlx::postgres::PgRow],
    limit: i64,
    at: &str,
    id: &str,
) -> Option<String> {
    if (rows.len() as i64) < limit {
        return None;
    }
    let last = rows.last()?;
    Some(format!(
        "{}|{}",
        last.try_get::<chrono::DateTime<chrono::Utc>, _>(at)
            .ok()?
            .to_rfc3339(),
        last.try_get::<Uuid, _>(id).ok()?
    ))
}

// ---------------------------------------------------------------------------
// Consolidation runs (§10, FR-894a)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PageQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

impl PageQuery {
    fn page(&self) -> i64 {
        self.limit
            .unwrap_or(crate::global::VIEW_PAGE_DEFAULT)
            .clamp(1, crate::global::VIEW_PAGE_MAX)
    }
}

/// `GET /api/projects/{id}/consolidation-runs` — what each pass did (§10).
///
/// A run with zero candidates is still a run and is still listed: the
/// difference between "consolidation found nothing" and "consolidation never
/// happened" is exactly what this list exists to show, and it is the same
/// distinction the funnel makes one level up.
///
/// Refusal reasons are counted rather than listed per candidate. A pass that
/// turned away forty candidates for one reason is one fact with a count, and
/// forty rows would bury it — and the reasons are a closed vocabulary
/// (`consolidation.md` §9, FR-804a), so grouping loses nothing.
async fn project_consolidation_runs(
    State(state): State<AppState>,
    user: SettledUser,
    Path(project_id): Path<Uuid>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, project_id, user.id()).await?;
    let limit = q.page();
    let (at, id) = crate::global::PageCursor::descending_bound(
        crate::global::PageCursor::decode_opt(q.cursor.as_deref()),
    );

    let rows = sqlx::query(
        "SELECT run_id, session_id, started_at, finished_at, events_claimed,
                candidates_proposed, candidates_accepted, candidates_refused,
                extractor_kind, state
           FROM consolidation_runs
          WHERE project_id = $1
            AND ($2::timestamptz IS NULL OR (started_at, run_id) < ($2, $3::uuid))
          ORDER BY started_at DESC, run_id DESC
          LIMIT $4",
    )
    .bind(project_id)
    .bind(at)
    .bind(id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    // One grouped statement for the whole page rather than one per run: a page
    // of a hundred runs would otherwise be a hundred round trips to answer one
    // request.
    let run_ids: Vec<Uuid> = rows.iter().map(|r| r.get("run_id")).collect();
    let mut reasons: std::collections::HashMap<Uuid, Vec<Value>> = std::collections::HashMap::new();
    if !run_ids.is_empty() {
        let grouped = sqlx::query(
            "SELECT run_id, refusal_reason, COUNT(*) AS n
               FROM knowledge_candidates
              WHERE run_id = ANY($1) AND refusal_reason IS NOT NULL
              GROUP BY run_id, refusal_reason
              ORDER BY run_id, refusal_reason",
        )
        .bind(&run_ids)
        .fetch_all(&state.pool)
        .await?;
        for row in &grouped {
            reasons.entry(row.get("run_id")).or_default().push(json!({
                "reason": row.get::<String, _>("refusal_reason"),
                "n": row.get::<i64, _>("n"),
            }));
        }
    }

    let runs: Vec<Value> = rows
        .iter()
        .map(|r| {
            let run_id: Uuid = r.get("run_id");
            json!({
                "run_id": run_id,
                "session_id": r.get::<Option<Uuid>, _>("session_id"),
                "started_at": r.get::<chrono::DateTime<chrono::Utc>, _>("started_at").to_rfc3339(),
                "finished_at": r
                    .get::<Option<chrono::DateTime<chrono::Utc>>, _>("finished_at")
                    .map(|t| t.to_rfc3339()),
                "state": r.get::<String, _>("state"),
                "events_claimed": r.get::<Option<i32>, _>("events_claimed"),
                "candidates_proposed": r.get::<Option<i32>, _>("candidates_proposed"),
                "candidates_accepted": r.get::<Option<i32>, _>("candidates_accepted"),
                "candidates_refused": r.get::<Option<i32>, _>("candidates_refused"),
                "refusal_reasons": reasons.remove(&run_id).unwrap_or_default(),
                "extractor_kind": r.get::<String, _>("extractor_kind"),
            })
        })
        .collect();

    Ok(Json(json!({
        "runs": runs,
        "cursor": view_cursor(&rows, limit, "started_at", "run_id"),
        "limit": limit,
    })))
}

// ---------------------------------------------------------------------------
// The retrieval trace list (FR-886)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct TraceListQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
    /// Only traces that considered or selected this reference — the target of
    /// memory detail's "view all" link (§7).
    #[serde(default)]
    reference_key: Option<String>,
    /// Only traces for one session, so the path from a session to what it was
    /// given is walkable through the API alone (SC-727).
    #[serde(default)]
    session_id: Option<Uuid>,
}

/// `GET /api/projects/{id}/retrieval-traces` — the traces list (FR-886).
///
/// **The `reference_key` filter is authorized before it is applied.** Filtering
/// is a question about a record, and asking it about a record the caller may
/// not see would answer "does this exist and was it retrieved here" for another
/// account's personal knowledge or pattern. So a reference the reader cannot
/// see produces the same empty page a reference nothing ever retrieved produces
/// — the two answers are deliberately identical, which is what stops the filter
/// being an existence oracle (FR-846a).
///
/// The rows themselves carry no budget and no latency. Those are scoped to the
/// account that made the retrieval (`retrieve::trace_detail`), and a list that
/// carried them would hand every project member a per-retrieval cost breakdown
/// the detail view withholds.
async fn project_retrieval_traces(
    State(state): State<AppState>,
    user: SettledUser,
    Path(project_id): Path<Uuid>,
    Query(q): Query<TraceListQuery>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, project_id, user.id()).await?;
    let limit = q
        .limit
        .unwrap_or(crate::global::VIEW_PAGE_DEFAULT)
        .clamp(1, crate::global::VIEW_PAGE_MAX);
    let (at, id) = crate::global::PageCursor::descending_bound(
        crate::global::PageCursor::decode_opt(q.cursor.as_deref()),
    );

    let mut filter_key: Option<String> = None;
    if let Some(raw) = q.reference_key.as_deref() {
        // Parsed through the canonical grammar rather than matched as a string:
        // `knowledge:<domain>:<uuid>` and `pattern:<uuid>` are the only two
        // shapes, and anything else names nothing.
        let reference = cairn_core::domain::Reference::parse_key(raw)
            .map_err(|_| ApiError::invalid("`reference_key` is not a canonical reference"))?;
        let reader = auth::ReaderContext::load(&state.pool, &user.0).await?;
        if auth::reference_visibility(&state.pool, &reader, reference).await?
            == auth::Visibility::Visible
        {
            filter_key = Some(reference.reference_key());
        } else {
            // Deliberately not a refusal. A refusal here would say "that
            // reference exists and is not yours", which is the fact being
            // protected. An empty page is what a reference nobody retrieved
            // also produces.
            return Ok(Json(json!({
                "traces": [], "cursor": Value::Null, "limit": limit,
            })));
        }
    }

    let rows = sqlx::query(
        "SELECT t.trace_id, t.session_id, t.trigger, t.delivery_point,
                t.degradation_level, t.delivery_state, t.acknowledgement_state,
                t.failure_reason, t.created_at
           FROM retrieval_traces t
          WHERE t.project_id = $1
            AND ($2::timestamptz IS NULL OR (t.created_at, t.trace_id) < ($2, $3::uuid))
            AND ($4::text IS NULL OR EXISTS (
                    SELECT 1 FROM retrieval_trace_items i
                     WHERE i.trace_id = t.trace_id AND i.reference_key = $4))
            AND ($5::uuid IS NULL OR t.session_id = $5)
          ORDER BY t.created_at DESC, t.trace_id DESC
          LIMIT $6",
    )
    .bind(project_id)
    .bind(at)
    .bind(id)
    .bind(&filter_key)
    .bind(q.session_id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    let traces: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "trace_id": r.get::<Uuid, _>("trace_id"),
                "session_id": r.get::<Uuid, _>("session_id"),
                "trigger": r.get::<String, _>("trigger"),
                "delivery_point": r.get::<String, _>("delivery_point"),
                "degradation_level": r.get::<Option<String>, _>("degradation_level"),
                "delivery_state": r.get::<String, _>("delivery_state"),
                "acknowledgement_state": r.get::<String, _>("acknowledgement_state"),
                "failure_reason": r.get::<Option<String>, _>("failure_reason"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(json!({
        "traces": traces,
        "cursor": view_cursor(&rows, limit, "created_at", "trace_id"),
        "limit": limit,
    })))
}

// ---------------------------------------------------------------------------
// System health (FR-891)
// ---------------------------------------------------------------------------

/// The dispositions that mean an observation did not survive the trip.
///
/// `captured`, `spooled`, `transmitted`, `accepted` and `persisted` are the
/// funnel working and are deliberately absent. `declined_by_policy` is absent
/// too: a decline is Cairn choosing not to keep something, which is not a
/// failure and must not be reported as one (FR-856's distinction, one level up).
const INGEST_FAILURE_DISPOSITIONS: &str =
    "('capture_deadline_exceeded','redaction_failed','privacy_refused',\
      'no_safe_semantic_mapping','spool_overflow_dropped','spool_saturated_dropped',\
      'rejected_by_server')";

/// `GET /api/system/health` — ingest, consolidation and retrieval (FR-891).
///
/// **`AdminUser`, not `require_member`, and the difference is the subject.**
/// Every other read in this contract answers a question about one project and
/// is gated on belonging to it. This one answers a question about the
/// deployment: how far behind the single consolidation task is, how many events
/// the server has taken, how many retrievals failed. There is no project to be
/// a member of, so membership is the wrong gate and the right one is the role.
/// A member reaching it would be reading across every project on the server.
///
/// The consolidation section is [`crate::consolidate::health`] verbatim — the
/// read `/api/consolidation/health` already serves. A second query would be a
/// second answer to "how far behind is consolidation", and the two would
/// disagree the first time either changed.
///
/// Below schema 4 each section is `null` rather than zeroed, for the reason the
/// funnel gives: a deployment without the tables has not observed nothing, it
/// has observed nothing *yet knowable* (FR-880).
async fn system_health(State(state): State<AppState>, _admin: AdminUser) -> ApiResult<Json<Value>> {
    if state.schema_version < AUTONOMOUS_MEMORY_SCHEMA {
        return Ok(Json(json!({
            "ingest": Value::Null,
            "consolidation": Value::Null,
            "retrieval": Value::Null,
        })));
    }

    let ingest = sqlx::query(&format!(
        "SELECT
           (SELECT COUNT(*) FROM safe_events) AS events_received,
           (SELECT MAX(received_at) FROM safe_events) AS last_received_at,
           (SELECT COALESCE(SUM(n), 0)::bigint FROM capture_dispositions
             WHERE disposition IN {INGEST_FAILURE_DISPOSITIONS}) AS capture_failures"
    ))
    .fetch_one(&state.pool)
    .await?;

    let by_disposition = sqlx::query(&format!(
        "SELECT disposition, COALESCE(SUM(n), 0)::bigint AS n
           FROM capture_dispositions
          WHERE disposition IN {INGEST_FAILURE_DISPOSITIONS}
          GROUP BY disposition
          ORDER BY disposition"
    ))
    .fetch_all(&state.pool)
    .await?;

    let retrieval = sqlx::query(
        "SELECT COUNT(*) AS traces,
                COUNT(*) FILTER (WHERE delivery_state = 'failed') AS failed,
                COUNT(*) FILTER (WHERE delivery_state = 'requested') AS never_generated,
                COUNT(*) FILTER (WHERE delivery_state = 'generated') AS never_transmitted,
                COUNT(*) FILTER (WHERE delivery_state = 'transmitted') AS transmitted,
                MAX(created_at) AS last_trace_at
           FROM retrieval_traces",
    )
    .fetch_one(&state.pool)
    .await?;

    let consolidation = crate::consolidate::health(&state.pool).await?;

    Ok(Json(json!({
        "ingest": {
            "events_received": ingest.get::<i64, _>("events_received"),
            "last_received_at": ingest
                .get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_received_at")
                .map(|t| t.to_rfc3339()),
            "capture_failures": ingest.get::<i64, _>("capture_failures"),
            "failures_by_disposition": by_disposition.iter().map(|r| json!({
                "disposition": r.get::<String, _>("disposition"),
                "n": r.get::<i64, _>("n"),
            })).collect::<Vec<_>>(),
        },
        "consolidation": serde_json::to_value(consolidation).unwrap_or_else(|_| json!({})),
        "retrieval": {
            "traces": retrieval.get::<i64, _>("traces"),
            "failed": retrieval.get::<i64, _>("failed"),
            // A trace still `requested` was never generated, and one still
            // `generated` was never reported transmitted. Two different
            // backlogs: the first is retrieval not finishing, the second is a
            // briefing nobody confirmed reached an agent (Principle X).
            "never_generated": retrieval.get::<i64, _>("never_generated"),
            "never_transmitted": retrieval.get::<i64, _>("never_transmitted"),
            "transmitted": retrieval.get::<i64, _>("transmitted"),
            "last_trace_at": retrieval
                .get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_trace_at")
                .map(|t| t.to_rfc3339()),
        },
    })))
}
