//! Per-IP sliding-window rate limiter for auth endpoints.
//!
//! Keeps a `VecDeque<Instant>` per IP address. On each request, entries older than
//! `window` are pruned; if the remaining count >= `max_requests` the middleware
//! returns 429. The inner map is never explicitly pruned of stale keys — auth
//! endpoints are low-volume so memory growth is negligible.

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Shared rate-limiter state. Cheap to clone (Arc + Mutex inside).
#[derive(Clone)]
pub struct AuthRateLimiter(Arc<Inner>);

struct Inner {
    buckets: Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
    window: Duration,
    max_requests: usize,
}

impl AuthRateLimiter {
    /// Allow `max_requests` per `window` per IP.
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self(Arc::new(Inner {
            buckets: Mutex::new(HashMap::new()),
            window,
            max_requests,
        }))
    }

    /// Returns `true` if the request should proceed, `false` if it should be rejected.
    pub fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let window = self.0.window;
        let mut map = self.0.buckets.lock().expect("rate limiter mutex");
        let bucket = map.entry(ip).or_default();
        while bucket.front().is_some_and(|t| now.duration_since(*t) > window) {
            bucket.pop_front();
        }
        if bucket.len() >= self.0.max_requests {
            return false;
        }
        bucket.push_back(now);
        true
    }
}

/// Axum middleware function — wrap with `middleware::from_fn` plus a captured `Arc<AuthRateLimiter>`.
pub async fn rate_limit_middleware(
    limiter: Arc<AuthRateLimiter>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]));

    if !limiter.check(ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("Retry-After", "60")],
            "Too many requests — please wait before retrying",
        )
            .into_response();
    }
    next.run(req).await
}
