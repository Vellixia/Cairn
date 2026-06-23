//! Browser-extension capture endpoint (v0.5.0 Sprint 21).
//!
//! The Chrome / Firefox Manifest V3 extension posts `{ kind, url, title,
//! text, captured_at }` to this endpoint. We turn it into a Cairn
//! memory (kind = `Note`, applies_to = [url], concepts = []). This is
//! deliberately minimal — the dashboard can refine / categorize later.
//!
//! ## Auth
//!
//! The browser extension lives in the user's own browser session, so the
//! request is unauthenticated. We rely on the local-origin restriction:
//! the extension's `host_permissions` allowlist pins it to
//! `http://127.0.0.1:7777/*` and `http://localhost:7777/*`. Anything
//! else gets a 403.
//!
//! ## Why not a CLIP/multi-modal embedder?
//!
//! We don't embed page text on capture — the user can trigger a recall
//! from the dashboard later, which is when semantic recall adds value.
//! A bare `Note` lets the BM25 index do the right thing at recall time
//! without paying an embed cost at capture time.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use cairn_core::{NewMemory, OrgId};
use serde::{Deserialize, Serialize};

use crate::AppState;

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    Selection,
    Page,
}

/// Wire format from a browser extension capture request.
#[derive(Debug, Deserialize)]
pub struct CaptureRequest {
    pub kind: CaptureKind,
    pub url: String,
    pub title: String,
    pub text: String,
    pub captured_at: String,
}

#[derive(Debug, Serialize)]
pub struct CaptureResponse {
    pub memory_id: String,
    pub kind: String,
    pub url: String,
}

/// `POST /api/extensions/capture` — write a memory from a browser
/// extension capture. Returns the new memory id and echoes the URL.
pub async fn capture(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CaptureRequest>,
) -> Response {
    // Local-origin check — the extension declares its hosts in manifest.json;
    // anything else gets a 403. We check Origin header only (ConnectInfo
    // isn't reliably available through axum's middleware chain).
    if !is_local_request(&headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "extension endpoint is loopback-only"})),
        )
            .into_response();
    }

    // Build the memory content. Trim aggressively — a 100 KB page is not a
    // useful single memory; the dashboard can re-chunk later if needed.
    let trimmed = req.text.trim();
    let truncated: String = trimmed.chars().take(20_000).collect();
    let content = if truncated.is_empty() {
        format!("Captured page: {}", req.title)
    } else {
        format!(
            "[{}] {}\n\nURL: {}\nCaptured at: {}\n\n{}",
            match req.kind {
                CaptureKind::Selection => "selection",
                CaptureKind::Page => "page",
            },
            req.title,
            req.url,
            req.captured_at,
            truncated
        )
    };

    // Each browser capture gets a synthetic "kind" so the dashboard can
    // group them. We use the existing MemoryKind::Note — the URL is in
    // `applies_to` so the dashboard can build a "browser captures" view
    // by filtering on applies_to starting with "http".
    let mut new_mem = NewMemory::new(&content);
    new_mem.applies_to = vec![req.url.clone()];
    new_mem.concepts = vec!["browser-capture".to_string()];
    new_mem.org_id = Some(OrgId::default());

    // The store is behind an Arc<Store> — we call `insert_memory` directly so we
    // don't go through MemoryEngine (which would also write to the audit log
    // and run the embedding provider).
    match state.store.insert_memory_for(&new_mem) {
        Ok(mem) => (
            StatusCode::CREATED,
            Json(CaptureResponse {
                memory_id: mem.id,
                kind: match req.kind {
                    CaptureKind::Selection => "selection".into(),
                    CaptureKind::Page => "page".into(),
                },
                url: req.url,
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// Allow only when the request carries an `Origin` header that matches a loopback origin
/// (`http://127.0.0.1:<port>` or `http://localhost:<port>`). Reject when:
///   - no `Origin` header is present (the previous version silently accepted this — direct
///     API calls from `curl` on remote hosts would otherwise pass through), or
///   - the origin is a remote scheme/host.
///
/// This is a defense-in-depth check layered on top of the auth middleware, which already
/// verifies loopback via `ConnectInfo<SocketAddr>`. The middleware runs first; this
/// function tightens the policy at the handler boundary for the browser-extension
/// endpoint specifically, so a misconfigured `host_permissions` in the extension
/// manifest can't slip a request past the loopback requirement.
fn is_local_request(headers: &HeaderMap) -> bool {
    match headers.get("origin").and_then(|v| v.to_str().ok()) {
        Some(origin)
            if origin.starts_with("http://127.0.0.1:")
                || origin.starts_with("http://localhost:") =>
        {
            true
        }
        Some(_) => false,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_local_request_accepts_loopback_origin() {
        let mut h = HeaderMap::new();
        h.insert("origin", "http://127.0.0.1:7777".parse().unwrap());
        assert!(is_local_request(&h));
        h.insert("origin", "http://localhost:7777".parse().unwrap());
        assert!(is_local_request(&h));
    }

    #[test]
    fn is_local_request_rejects_remote_origin() {
        let mut h = HeaderMap::new();
        h.insert("origin", "https://evil.example".parse().unwrap());
        assert!(!is_local_request(&h));
    }

    #[test]
    fn is_local_request_rejects_missing_origin() {
        // No Origin header at all — must NOT silently pass through. The auth middleware
        // still catches the actual network path, but a missing Origin on a browser
        // request is suspicious enough to reject at the handler too.
        let h = HeaderMap::new();
        assert!(!is_local_request(&h));
    }
}
