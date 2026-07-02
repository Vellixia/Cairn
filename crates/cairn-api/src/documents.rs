//! HTTP handlers for `/api/documents/*` (v0.8.0 Sprint 6 - RAG document store).
//!
//! `cairn-server` runs in a container with no access to the user's filesystem and no business
//! fetching arbitrary user-supplied URLs itself (SSRF risk, and it usually couldn't resolve a
//! local file path anyway). The client (`cairn documents ingest <path|url>`) reads the source
//! locally via `cairn_document::read_source` and POSTs the already-fetched text here; this
//! handler only chunks (`cairn_document::chunk_text`) and stores.

use axum::extract::{Path, Query, State};
use axum::Json;
use cairn_document::{chunk_text, DEFAULT_CHUNK_CHARS};
use cairn_store::{DocumentChunkRecord, DocumentSummary};
use serde::Deserialize;

use crate::{ApiError, AppState};

#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    /// A stable identifier for this document - typically the file path or URL the client read
    /// it from. Re-ingesting the same `source` replaces its chunks rather than duplicating them.
    pub source: String,
    /// The document's raw text content, already read/fetched client-side.
    pub content: String,
    /// Defaults to `source` when omitted.
    #[serde(default)]
    pub title: Option<String>,
}

/// `POST /api/documents/ingest` - chunk `content` and (re)store it under `source`.
pub async fn ingest(
    State(s): State<AppState>,
    Json(req): Json<IngestRequest>,
) -> Result<Json<DocumentSummary>, ApiError> {
    if req.content.trim().is_empty() {
        return Err(ApiError::bad_request("content must not be empty"));
    }
    let title = req.title.unwrap_or_else(|| req.source.clone());
    let chunks = chunk_text(&req.content, DEFAULT_CHUNK_CHARS);
    s.store.replace_document(&req.source, &title, &chunks)?;
    s.store
        .list_documents()?
        .into_iter()
        .find(|d| d.source == req.source)
        .map(Json)
        .ok_or_else(|| ApiError::bad_request("document vanished immediately after ingest"))
}

/// `GET /api/documents` - every ingested document, most-recently-updated first.
pub async fn list(State(s): State<AppState>) -> Result<Json<Vec<DocumentSummary>>, ApiError> {
    Ok(Json(s.store.list_documents()?))
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    q: String,
    #[serde(default)]
    limit: Option<usize>,
}

/// `GET /api/documents/search?q=...&limit=...` - the most relevant chunks across every
/// ingested document.
pub async fn search(
    State(s): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<DocumentChunkRecord>>, ApiError> {
    let limit = q.limit.unwrap_or(10).clamp(1, 100);
    Ok(Json(s.store.search_documents(&q.q, limit)?))
}

/// `DELETE /api/documents/:id` - `:id` is `DocumentSummary.id` (a hash of `source`, not `source`
/// itself - a path/URL isn't safe to put directly in a path segment).
pub async fn delete(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let doc = s
        .store
        .list_documents()?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| ApiError::not_found("no such document"))?;
    let deleted = s.store.delete_document(&doc.source)?;
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}
