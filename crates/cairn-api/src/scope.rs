//! Scope context middleware (v0.8.0 Sprint 2).
//!
//! Reads `X-Cairn-Project` / `X-Cairn-Session` request headers and stashes a [`ScopeCtx`] in
//! request extensions, mirroring `security_headers`'s flat-module + `Extension` pattern - there
//! is no `middleware/` subdirectory in this crate. Handlers that need scope context take
//! `Extension(ctx): Extension<ScopeCtx>` as an argument; every other handler is unaffected.
//!
//! Auto-detection of the current project (from the client's git remote/root) lands in Sprint 3 -
//! today the header is the only source of `project_id`.

use axum::{extract::Request, http::HeaderMap, middleware::Next, response::Response};
use cairn_core::ScopeCtx;

pub const PROJECT_HEADER: &str = "X-Cairn-Project";
pub const SESSION_HEADER: &str = "X-Cairn-Session";

pub async fn scope_middleware(headers: HeaderMap, mut req: Request, next: Next) -> Response {
    let project_id = headers
        .get(PROJECT_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let session_id = headers
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    req.extensions_mut().insert(ScopeCtx {
        project_id,
        session_id,
    });
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request as HttpRequest,
        middleware::from_fn,
        routing::get,
        Extension, Router,
    };
    use tower::ServiceExt;

    async fn echo(Extension(ctx): Extension<ScopeCtx>) -> String {
        format!("{:?}/{:?}", ctx.project_id, ctx.session_id)
    }

    fn app() -> Router {
        Router::new()
            .route("/", get(echo))
            .layer(from_fn(scope_middleware))
    }

    #[tokio::test]
    async fn missing_headers_yield_default_scope() {
        let resp = app()
            .oneshot(HttpRequest::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), 4096).await.unwrap();
        assert_eq!(&body[..], b"None/None");
    }

    #[tokio::test]
    async fn headers_populate_scope_ctx() {
        let resp = app()
            .oneshot(
                HttpRequest::builder()
                    .uri("/")
                    .header(PROJECT_HEADER, "proj-alpha")
                    .header(SESSION_HEADER, "sess-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), 4096).await.unwrap();
        assert_eq!(&body[..], br#"Some("proj-alpha")/Some("sess-1")"#);
    }

    #[tokio::test]
    async fn empty_header_value_is_treated_as_absent() {
        let resp = app()
            .oneshot(
                HttpRequest::builder()
                    .uri("/")
                    .header(PROJECT_HEADER, "")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), 4096).await.unwrap();
        assert_eq!(&body[..], b"None/None");
    }
}
