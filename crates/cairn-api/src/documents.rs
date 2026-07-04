//! HTTP handlers for `/api/documents/*` (v0.8.0 Sprint 6 - RAG document store).
//!
//! `cairn-server` runs in a container with no access to the user's filesystem and no business
//! fetching arbitrary user-supplied URLs itself (SSRF risk, and it usually couldn't resolve a
//! local file path anyway). The client (`cairn documents ingest <path|url>`) reads the source
//! locally via `cairn_document::read_source` and POSTs the already-fetched text here; this
//! handler only chunks (`cairn_document::chunk_text`) and stores.

use axum::extract::{Extension, Path, Query, State};
use axum::Json;
use cairn_core::ScopeCtx;
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

/// `POST /api/documents/ingest` - chunk `content` and (re)store it under `source`. Scoped to
/// the caller's project (`X-Cairn-Project`) when set - same visibility policy as `remember`;
/// `None` (no header, e.g. a manual `cairn documents ingest` outside a project) stays global.
pub async fn ingest(
    State(s): State<AppState>,
    Extension(scope): Extension<ScopeCtx>,
    Json(req): Json<IngestRequest>,
) -> Result<Json<DocumentSummary>, ApiError> {
    if req.content.trim().is_empty() {
        return Err(ApiError::bad_request("content must not be empty"));
    }
    let title = req.title.unwrap_or_else(|| req.source.clone());
    let chunks = chunk_text(&req.content, DEFAULT_CHUNK_CHARS);
    s.store
        .replace_document(&req.source, &title, &chunks, scope.project_id.as_deref())?;
    let doc = s
        .store
        .list_documents(None)?
        .into_iter()
        .find(|d| d.source == req.source)
        .ok_or_else(|| ApiError::bad_request("document vanished immediately after ingest"))?;
    crate::events::publish_document(&s.events, "ingested", &doc.id);
    Ok(Json(doc))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// Explicit override for the dashboard (which has no `X-Cairn-Project` header context of
    /// its own); falls back to the request's scope header when omitted so CLI/MCP callers get
    /// project-aware results for free.
    #[serde(default)]
    project_id: Option<String>,
}

/// `GET /api/documents?project_id=...` - every ingested document, most-recently-updated first.
/// `None` (no param, no scope header) returns everything, unfiltered.
pub async fn list(
    State(s): State<AppState>,
    Extension(scope): Extension<ScopeCtx>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<DocumentSummary>>, ApiError> {
    let filter = q.project_id.or(scope.project_id);
    Ok(Json(s.store.list_documents(filter.as_deref())?))
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    q: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    project_id: Option<String>,
}

/// `GET /api/documents/search?q=...&limit=...&project_id=...` - the most relevant chunks,
/// blended the same way `list` is (project-scoped + global).
pub async fn search(
    State(s): State<AppState>,
    Extension(scope): Extension<ScopeCtx>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<DocumentChunkRecord>>, ApiError> {
    let limit = q.limit.unwrap_or(10).clamp(1, 100);
    let filter = q.project_id.or(scope.project_id);
    Ok(Json(s.store.search_documents(
        &q.q,
        limit,
        filter.as_deref(),
    )?))
}

/// `DELETE /api/documents/:id` - `:id` is `DocumentSummary.id` (a hash of `source`, not `source`
/// itself - a path/URL isn't safe to put directly in a path segment).
pub async fn delete(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let doc = s
        .store
        .list_documents(None)?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| ApiError::not_found("no such document"))?;
    let deleted = s.store.delete_document(&doc.source)?;
    if deleted {
        crate::events::publish_document(&s.events, "deleted", &doc.id);
    }
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ingest_publishes_a_document_ingested_event() {
        let Some((state, _dir)) = crate::tests::test_state() else {
            return;
        };
        let mut rx = state.events.subscribe();
        let resp = ingest(
            State(state.clone()),
            Extension(ScopeCtx::default()),
            Json(IngestRequest {
                source: "sse-test.md".into(),
                content: "hello world, this is a test document".into(),
                title: None,
            }),
        )
        .await
        .unwrap();
        let ev = rx
            .try_recv()
            .expect("ingest should publish a document event");
        assert_eq!(ev.kind, crate::events::KIND_DOCUMENT);
        assert_eq!(ev.data["action"], "ingested");
        assert_eq!(ev.data["document_id"], resp.0.id);
    }

    #[tokio::test]
    async fn delete_publishes_a_document_deleted_event() {
        let Some((state, _dir)) = crate::tests::test_state() else {
            return;
        };
        let doc = ingest(
            State(state.clone()),
            Extension(ScopeCtx::default()),
            Json(IngestRequest {
                source: "sse-test-delete.md".into(),
                content: "content to be deleted".into(),
                title: None,
            }),
        )
        .await
        .unwrap();
        let mut rx = state.events.subscribe();
        let _ = delete(State(state.clone()), Path(doc.0.id.clone()))
            .await
            .unwrap();
        let ev = rx
            .try_recv()
            .expect("delete should publish a document event");
        assert_eq!(ev.kind, crate::events::KIND_DOCUMENT);
        assert_eq!(ev.data["action"], "deleted");
        assert_eq!(ev.data["document_id"], doc.0.id);
    }
}
