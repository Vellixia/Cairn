//! Security headers middleware.
//!
//! Adds a small set of headers that cost nothing and meaningfully tighten the browser's
//! default behavior for a same-origin authenticated UI:
//!
//! - `X-Frame-Options: DENY` — refuses framing; the dashboard should never appear in an iframe.
//! - `X-Content-Type-Options: nosniff` — refuses MIME-type guessing on static assets.
//! - `Referrer-Policy: no-referrer` — never leak the URL of an admin page to third parties.
//! - `Permissions-Policy: clipboard-write=(self)` — only this origin can write to the clipboard.
//!
//! CSP is intentionally *not* added here: the static fallback HTML embeds inline `<style>` and
//! a tiny inline `<script>`, which a strict CSP would break. A later iteration that ships the
//! dashboard from prebuilt assets can adopt a per-response nonce policy.

use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};

const X_FRAME_OPTIONS: HeaderName = HeaderName::from_static("x-frame-options");
const X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");
const REFERRER_POLICY: HeaderName = HeaderName::from_static("referrer-policy");
const PERMISSIONS_POLICY: HeaderName = HeaderName::from_static("permissions-policy");

pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(
        PERMISSIONS_POLICY,
        HeaderValue::from_static("clipboard-write=(self)"),
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, middleware::from_fn, routing::get, Router};
    use tower::ServiceExt;

    async fn hello() -> &'static str {
        "hi"
    }

    #[tokio::test]
    async fn headers_are_attached_to_every_response() {
        let app = Router::new()
            .route("/", get(hello))
            .layer(from_fn(security_headers));
        let resp = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
        assert_eq!(
            resp.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
        assert_eq!(
            resp.headers().get("referrer-policy").unwrap(),
            "no-referrer"
        );
        assert_eq!(
            resp.headers().get("permissions-policy").unwrap(),
            "clipboard-write=(self)"
        );
    }
}
