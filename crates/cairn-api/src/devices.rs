//! Admin-facing device management: issue device tokens from the dashboard.
//!
//! The web UI (You -> Tokens) drives these endpoints so the admin can issue tokens from the
//! browser without leaving the dashboard. The JWT is returned ONCE in the
//! `POST /api/devices/tokens` response; subsequent reads only return the token metadata (id,
//! name, scope, created_at, revoked). The server never persists the JWT itself beyond what
//! `create_token` does in the store (token id + metadata).

use crate::admin::require_admin;
use crate::AppState;
use axum::{
    extract::{Path, State},
    http::{header::HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use cairn_core::{DeviceToken, TokenScope};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub expires_in_days: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct IssuedToken {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// The bearer JWT, returned ONLY on issue. Never persisted in cleartext by the store.
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct TokenMetaView {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl TokenMetaView {
    fn from(t: &DeviceToken) -> Self {
        Self {
            id: t.id.clone(),
            name: t.name.clone(),
            scope: t.scope.as_str().to_string(),
            created_at: t.created_at,
            expires_at: t.expires_at,
            last_used_at: t.last_used_at,
        }
    }
}

pub async fn list_tokens(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = require_admin(&state, &headers).await {
        return resp;
    }
    let tokens = match state.store.list_tokens() {
        Ok(t) => t,
        Err(e) => return admin_error(&format!("list tokens: {e}")),
    };
    let views: Vec<TokenMetaView> = tokens.iter().map(TokenMetaView::from).collect();
    (StatusCode::OK, Json(views)).into_response()
}

pub async fn create_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateTokenRequest>,
) -> Response {
    let rec = match require_admin(&state, &headers).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    if req.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "name is required"})),
        )
            .into_response();
    }
    let scope: TokenScope = match req.scope.as_deref().unwrap_or("write").parse() {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "scope must be admin|write|read"})),
            )
                .into_response();
        }
    };
    let expires_at = req
        .expires_in_days
        .filter(|d| *d > 0)
        .map(|d| Utc::now() + chrono::Duration::days(d));
    let mut t = match state.store.create_token(&req.name, scope, expires_at) {
        Ok(t) => t,
        Err(e) => return admin_error(&format!("create: {e}")),
    };
    let bearer = state.sign_token(&t.id, &t.name, scope, expires_at);
    t.token = Some(bearer.clone());
    state.audit_log.record(
        &state.store,
        &state.events,
        "token_issued",
        &rec.username,
        format!("{} ({})", req.name, scope.as_str()),
    );
    let issued = IssuedToken {
        id: t.id,
        name: t.name,
        scope: scope.as_str().to_string(),
        created_at: t.created_at,
        expires_at,
        token: bearer,
    };
    (StatusCode::CREATED, Json(issued)).into_response()
}

pub async fn revoke_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let rec = match require_admin(&state, &headers).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    match state.store.revoke_token(&id) {
        Ok(true) => {
            state.audit_log.record(
                &state.store,
                &state.events,
                "token_revoked",
                &rec.username,
                id.clone(),
            );
            (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no such token"})),
        )
            .into_response(),
        Err(e) => admin_error(&format!("revoke: {e}")),
    }
}

fn admin_error(msg: &str) -> Response {
    tracing::error!("admin devices: {msg}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}
