//! HTTP routes (contracts/server-api.md).

use crate::auth::{self, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/health", get(health))
        // Authentication
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
        .route("/api/tokens", get(list_tokens).post(create_token))
        .route("/api/tokens/{id}", delete(revoke_token))
        // Linking (FR-064)
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/lookup", get(lookup_projects))
        .route("/api/projects/{id}/join", post(join_project))
        // Sync
        .route("/api/sync/batch", post(sync_batch))
        .route("/api/sync/changes", get(sync_changes))
        // Read API for the web UI
        .route("/api/projects/{id}", get(project_overview))
        .route("/api/projects/{id}/tasks", get(project_tasks))
        .route("/api/projects/{id}/sessions", get(project_sessions))
        .route("/api/projects/{id}/memories", get(project_memories))
        .route("/api/projects/{id}/sync-status", get(project_sync_status))
        .route("/api/sessions/{id}", get(session_detail))
        .route("/api/sessions/{id}/handoff", get(session_handoff))
        .route(
            "/api/memories/{id}",
            get(memory_detail).delete(delete_memory),
        )
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RegisterBody {
    email: String,
    display_name: String,
    password: String,
}

async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterBody>,
) -> ApiResult<impl IntoResponse> {
    if body.password.len() < 8 {
        return Err(ApiError::invalid("password must be at least 8 characters"));
    }
    let id = Uuid::now_v7();
    let hash = auth::hash_password(&body.password)?;
    let result = sqlx::query(
        "INSERT INTO users (id, email, display_name, password_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(body.email.trim().to_lowercase())
    .bind(&body.display_name)
    .bind(hash)
    .execute(&state.pool)
    .await;

    match result {
        Ok(_) => Ok(Json(json!({ "id": id, "email": body.email }))),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            Err(ApiError::conflict("that email is already registered"))
        }
        Err(e) => Err(e.into()),
    }
}

#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> ApiResult<impl IntoResponse> {
    let row = sqlx::query("SELECT id, password_hash FROM users WHERE email = $1")
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

async fn me(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Value>> {
    let row = sqlx::query("SELECT id, email, display_name FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(json!({
        "id": row.try_get::<Uuid, _>("id")?,
        "email": row.try_get::<String, _>("email")?,
        "display_name": row.try_get::<String, _>("display_name")?,
    })))
}

#[derive(Deserialize)]
struct TokenBody {
    #[serde(default = "default_token_name")]
    name: String,
}

fn default_token_name() -> String {
    "cairn daemon".to_string()
}

/// The plaintext is returned exactly once and never stored (D10).
async fn create_token(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<TokenBody>,
) -> ApiResult<Json<Value>> {
    let token = auth::random_token();
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO api_tokens (id, user_id, name, token_hash) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(user.id)
        .bind(&body.name)
        .bind(auth::hash_token(&token))
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({ "id": id, "name": body.name, "token": token })))
}

async fn list_tokens(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT id, name, created_at, last_used_at, revoked_at FROM api_tokens
         WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user.id)
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
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    sqlx::query("UPDATE api_tokens SET revoked_at = now() WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({ "revoked": id })))
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
    user: CurrentUser,
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
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "name": body.name })))
}

async fn join_project(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM projects WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
    if exists.is_none() {
        return Err(ApiError::not_found("no such shared project"));
    }
    sqlx::query(
        "INSERT INTO project_members (project_id, user_id) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(user.id)
    .execute(&state.pool)
    .await?;
    Ok(Json(json!({ "id": id, "joined": true })))
}

#[derive(Deserialize)]
struct LookupQuery {
    #[serde(default)]
    remote: String,
}

/// A discovery *hint*. Returns only projects the caller may already see, and
/// never links anything on its own (D14).
async fn lookup_projects(
    State(state): State<AppState>,
    _user: CurrentUser,
    Query(q): Query<LookupQuery>,
) -> ApiResult<Json<Value>> {
    if q.remote.trim().is_empty() {
        return Ok(Json(json!({ "projects": [] })));
    }
    let rows = sqlx::query(
        "SELECT id, name FROM projects
         WHERE repository_remote = $1 AND deleted_at IS NULL ORDER BY created_at",
    )
    .bind(q.remote.trim())
    .fetch_all(&state.pool)
    .await?;
    let projects: Vec<Value> = rows
        .iter()
        .map(|r| json!({ "id": r.get::<Uuid, _>("id"), "name": r.get::<String, _>("name") }))
        .collect();
    Ok(Json(json!({ "projects": projects })))
}

async fn list_projects(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT p.id, p.name, p.repository_remote, p.created_at
         FROM projects p
         JOIN project_members m ON m.project_id = p.id
         WHERE m.user_id = $1 AND p.deleted_at IS NULL
         ORDER BY p.created_at DESC",
    )
    .bind(user.id)
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
// Sync (FR-055, FR-056)
// ---------------------------------------------------------------------------

pub use crate::sync::{sync_batch, sync_changes};

// ---------------------------------------------------------------------------
// Read API for the web UI
// ---------------------------------------------------------------------------

async fn project_overview(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, id, user.id).await?;

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
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(q): Query<TaskQuery>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, id, user.id).await?;
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
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, id, user.id).await?;
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
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let row = sqlx::query("SELECT * FROM sessions WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("no such session"))?;
    let project_id: Uuid = row.try_get("project_id")?;
    auth::require_member(&state.pool, project_id, user.id).await?;
    Ok(Json(
        json!({ "session": sessions_json(std::slice::from_ref(&row))[0] }),
    ))
}

async fn session_handoff(
    State(state): State<AppState>,
    user: CurrentUser,
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
    auth::require_member(&state.pool, project_id, user.id).await?;

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
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(q): Query<MemoryQueryParams>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, id, user.id).await?;
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
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let row = sqlx::query("SELECT * FROM memories WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("no such memory"))?;
    let project_id: Uuid = row.try_get("project_id")?;
    auth::require_member(&state.pool, project_id, user.id).await?;

    // Provenance is references; evidence content is local to the machine that
    // captured it and does not exist here (FR-055, FR-061).
    let mut value = memory_json(&row);
    value["provenance"]["evidence_content_available"] = json!(false);
    Ok(Json(json!({ "memory": value })))
}

async fn delete_memory(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let row = sqlx::query("SELECT project_id FROM memories WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("no such memory"))?;
    let project_id: Uuid = row.try_get("project_id")?;
    auth::require_member(&state.pool, project_id, user.id).await?;

    sqlx::query("UPDATE memories SET deleted_at = now(), content = '' WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok((StatusCode::OK, Json(json!({ "deleted": id }))))
}

async fn project_sync_status(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    auth::require_member(&state.pool, id, user.id).await?;
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
