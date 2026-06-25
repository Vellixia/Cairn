//! Axum router for the cairn.sh proxy.

use crate::config::ProxyConfig;
use crate::fanout::{fanout_async, MergedPack};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

/// Build the proxy router bound to the given config. The router's state type
/// is `Arc<ProxyConfig>` - handlers derive any per-request state from it.
pub fn build(config: Arc<ProxyConfig>) -> Router {
    Router::new()
        .route("/registry/packs", get(list_packs))
        .route("/registry/search", get(search_packs))
        .route("/registry/federation/pull", get(federation_pull))
        .route("/health", get(health))
        .with_state(config)
}

async fn health() -> &'static str {
    "ok"
}

async fn list_packs(State(config): State<Arc<ProxyConfig>>) -> Result<Response, ProxyHttpError> {
    let result = fanout_async(config, "/registry/packs").await?;
    Ok(Json(result.packs).into_response())
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

async fn search_packs(
    State(config): State<Arc<ProxyConfig>>,
    Query(q): Query<SearchQuery>,
) -> Result<Response, ProxyHttpError> {
    let needle = q.q.unwrap_or_default();
    let result = fanout_async(config, "/registry/packs").await?;
    let q_lower = needle.to_ascii_lowercase();
    let filtered: Vec<MergedPack> = result
        .packs
        .into_iter()
        .filter(|m| {
            q_lower.is_empty()
                || m.pack.name.to_ascii_lowercase().contains(&q_lower)
                || m.pack.description.to_ascii_lowercase().contains(&q_lower)
                || m.pack.author.to_ascii_lowercase().contains(&q_lower)
        })
        .collect();
    Ok(Json(filtered).into_response())
}

#[derive(Debug, Deserialize)]
pub struct FederationQuery {
    pub since: Option<i64>,
}

async fn federation_pull(
    State(config): State<Arc<ProxyConfig>>,
    Query(q): Query<FederationQuery>,
) -> Result<Response, ProxyHttpError> {
    let since = q.since.unwrap_or(0);
    let path = format!("/registry/revocations?since={since}");
    let result = fanout_async(config, &path).await?;
    Ok(Json(result.packs).into_response())
}

#[derive(Debug, thiserror::Error)]
enum ProxyHttpError {
    #[error("upstream error: {0}")]
    Upstream(#[from] crate::ProxyError),
}

impl IntoResponse for ProxyHttpError {
    fn into_response(self) -> Response {
        let status = match &self {
            ProxyHttpError::Upstream(_) => StatusCode::BAD_GATEWAY,
        };
        (status, Json(serde_json::json!({"error": self.to_string()}))).into_response()
    }
}
