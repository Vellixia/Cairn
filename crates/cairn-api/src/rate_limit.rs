//! Simple in-memory sliding-window rate limiter for axum.
//!
//! Tracks request counts per client IP over a configurable window. Returns 429 Too Many Requests
//! when the limit is exceeded. Designed for single-server deployments; for distributed setups,
//! replace with a Redis-backed limiter.

use axum::{
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A sliding-window rate limiter keyed by client IP.
#[derive(Clone)]
pub struct RateLimiter {
    inner: std::sync::Arc<Mutex<HashMap<std::net::IpAddr, Window>>>,
    max_requests: u32,
    window: Duration,
}

struct Window {
    timestamps: Vec<Instant>,
}

impl RateLimiter {
    /// Create a new rate limiter: `max_requests` per `window` duration, per client IP.
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            inner: std::sync::Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window,
        }
    }

    /// Check whether the given IP is within the rate limit. Returns `true` if the request is
    /// allowed, `false` if it should be rejected (429).
    pub fn check(&self, ip: std::net::IpAddr) -> bool {
        let now = Instant::now();
        let cutoff = now - self.window;
        let mut map = self.inner.lock().unwrap();
        let window = map.entry(ip).or_insert_with(|| Window {
            timestamps: Vec::new(),
        });
        window.timestamps.retain(|t| *t > cutoff);
        if window.timestamps.len() >= self.max_requests as usize {
            return false;
        }
        window.timestamps.push(now);
        true
    }
}

/// Axum middleware: reject with 429 if the client IP exceeds the rate limit.
pub async fn rate_limit_middleware(
    axum::extract::State(limiter): axum::extract::State<RateLimiter>,
    req: Request,
    next: Next,
) -> Response {
    let ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or_else(|| std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));

    if !limiter.check(ip) {
        tracing::warn!(%ip, "rate limit exceeded");
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "error": "rate limit exceeded; try again later" })),
        )
            .into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_under_limit() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        let ip = "127.0.0.1".parse().unwrap();
        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
    }

    #[test]
    fn rejects_over_limit() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        let ip = "127.0.0.1".parse().unwrap();
        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
        assert!(!limiter.check(ip));
    }

    #[test]
    fn different_ips_have_separate_limits() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let ip1 = "127.0.0.1".parse().unwrap();
        let ip2 = "127.0.0.2".parse().unwrap();
        assert!(limiter.check(ip1));
        assert!(limiter.check(ip2));
        assert!(!limiter.check(ip1));
        assert!(!limiter.check(ip2));
    }
}
