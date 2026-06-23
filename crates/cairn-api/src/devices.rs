//! Admin-facing device management: issue device tokens + pair codes from the dashboard.
//!
//! The CLI (`cairn token create`, `cairn pair-code`) still works — this module exists so the
//! admin can do the same operations from the web UI without leaving the browser. The JWT is
//! returned ONCE in the `POST /api/devices/tokens` response; subsequent reads only return the
//! token metadata (id, name, scope, created_at, revoked). The server never persists the JWT
//! itself beyond what `create_token` does in the store (token id + metadata).

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
use std::time::Duration;

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

#[derive(Debug, Deserialize)]
pub struct CreatePairCodeRequest {
    pub name: String,
    #[serde(default)]
    pub ttl_minutes: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct IssuedPairCode {
    pub code: String,
    pub name: String,
    pub expires_at: DateTime<Utc>,
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
    let mut t = match state.store.create_token(&req.name) {
        Ok(t) => t,
        Err(e) => return admin_error(&format!("create: {e}")),
    };
    t.scope = scope;
    let bearer = state.sign_token(&t.id, &t.name, scope, expires_at);
    t.token = Some(bearer.clone());
    state.audit_log.record(
        &state.store,
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
            state
                .audit_log
                .record(&state.store, "token_revoked", &rec.username, id.clone());
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

pub async fn create_pair_code(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreatePairCodeRequest>,
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
    let ttl = req.ttl_minutes.unwrap_or(10).clamp(1, 60);
    let (code, expires_at) =
        match pair_code::generate_for(&state, &req.name, Duration::from_secs((ttl as u64) * 60)) {
            Ok(v) => v,
            Err(e) => return admin_error(&format!("generate: {e}")),
        };
    state.audit_log.record(
        &state.store,
        "pair_code_issued",
        &rec.username,
        format!("{} (ttl {}m)", req.name, ttl),
    );
    let issued = IssuedPairCode {
        code,
        name: req.name,
        expires_at,
    };
    (StatusCode::CREATED, Json(issued)).into_response()
}

mod pair_code {
    use super::*;
    use rand::seq::SliceRandom;

    const CHARSET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789"; // no 0/O/1/I/L

    pub(super) fn generate_for(
        state: &AppState,
        name: &str,
        ttl: Duration,
    ) -> cairn_core::Result<(String, DateTime<Utc>)> {
        let mut rng = rand::thread_rng();
        let code: String = (0..8)
            .map(|_| *CHARSET.choose(&mut rng).unwrap() as char)
            .collect();
        let token = state.store.create_token(name)?;
        let expires_at = Utc::now() + chrono::Duration::seconds(ttl.as_secs() as i64);
        // Store the token id (not the bearer) — `pair_claim` signs a fresh JWT at claim time, same
        // pattern as the existing `/api/pair/new` flow.
        state
            .store
            .create_pairing(&code, &token.id, name, &expires_at.to_rfc3339())?;
        Ok((code, expires_at))
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
