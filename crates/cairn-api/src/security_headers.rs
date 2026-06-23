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
//! ## CSP nonce (v0.5.0 Sprint 7)
//!
//! For HTML responses (the dashboard shell), we inject a per-request CSP nonce and a
//! matching `Content-Security-Policy: script-src 'self' 'nonce-{rand}'` header. The static
//! handler writes the nonce into the HTML's `<script>` tags before serving, so the
//! inline bootstrap script can keep working. Static `.js`/`.css` references use `script-src
//! 'self'` so they don't need a nonce.

use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use rand::RngCore;

const X_FRAME_OPTIONS: HeaderName = HeaderName::from_static("x-frame-options");
const X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");
const REFERRER_POLICY: HeaderName = HeaderName::from_static("referrer-policy");
const PERMISSIONS_POLICY: HeaderName = HeaderName::from_static("permissions-policy");

pub async fn security_headers(mut req: Request, next: Next) -> Response {
    // Generate a per-request nonce. 16 random bytes → 32 hex chars; well under the 128-bit
    // CSP nonce limit and short enough to inline in every script tag.
    let mut nonce_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = hex::encode(nonce_bytes);
    // Stash the nonce in request extensions so the static handler (downstream) can find it.
    req.extensions_mut().insert(CspNonce(nonce.clone()));

    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(
        PERMISSIONS_POLICY,
        HeaderValue::from_static("clipboard-write=(self)"),
    );
    // Strict CSP: scripts must be either external ('self') or carry our nonce.
    let csp = format!(
        "default-src 'self'; script-src 'self' 'nonce-{nonce}'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'",
    );
    if let Ok(v) = HeaderValue::from_str(&csp) {
        headers.insert(HeaderName::from_static("content-security-policy"), v);
    }
    resp
}

/// Request extension type carrying the per-request CSP nonce. The static handler reads this
/// to rewrite the served HTML.
#[derive(Clone)]
pub struct CspNonce(pub String);

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

    #[tokio::test]
    async fn csp_header_includes_a_random_nonce() {
        let app = Router::new()
            .route("/", get(hello))
            .layer(from_fn(security_headers));
        let resp = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let csp = resp
            .headers()
            .get("content-security-policy")
            .expect("CSP header should be present")
            .to_str()
            .unwrap()
            .to_string();
        assert!(csp.contains("script-src 'self' 'nonce-"));
        // The nonce should be 32 hex chars (16 bytes).
        let nonce_start = csp.find("nonce-").unwrap() + "nonce-".len();
        let nonce = &csp[nonce_start..nonce_start + 32];
        assert!(
            nonce.chars().all(|c| c.is_ascii_hexdigit()),
            "nonce should be hex; got {nonce}"
        );
    }
}
