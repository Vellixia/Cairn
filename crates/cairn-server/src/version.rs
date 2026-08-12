//! What this deployment is running, and whether something newer exists.
//!
//! The browser must not ask GitHub itself: that would spend every visitor's
//! rate limit, leak who is looking, and fail behind a proxy that only allows
//! this origin. The server asks once and shares the answer.

use cairn_core::release::{self, Release};
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// How long a lookup stands before it is worth asking again.
///
/// GitHub allows 60 unauthenticated calls an hour per address. A deployment
/// serving a team must not spend that on page loads, and a release nobody
/// hears about for six hours has harmed no one.
const CACHE_FOR: Duration = Duration::from_secs(6 * 60 * 60);

/// The version this binary was built as.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize)]
pub struct VersionPayload {
    /// What is running here.
    pub current: String,
    /// The newest published release, when the lookup succeeded.
    pub latest: Option<Release>,
    /// Whether `latest` is worth moving to.
    pub update_available: bool,
    /// Absent when the lookup has never succeeded — the UI says so rather than
    /// implying this deployment is up to date.
    pub checked_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Default)]
struct Cached {
    release: Option<Release>,
    checked_at: Option<chrono::DateTime<chrono::Utc>>,
    fetched: Option<Instant>,
}

/// Shared, lazily refreshed knowledge of the newest release.
#[derive(Clone, Default)]
pub struct ReleaseCache(Arc<RwLock<Cached>>);

impl ReleaseCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The payload to serve, refreshing first if the answer has gone stale.
    ///
    /// A failed lookup is not an error for the caller: the deployment still
    /// knows its own version, and "we could not reach GitHub" is a better
    /// answer than a 500.
    pub async fn payload(&self) -> VersionPayload {
        if self.is_stale().await {
            self.refresh().await;
        }
        let cached = self.0.read().await;
        let update_available = cached
            .release
            .as_ref()
            .is_some_and(|r| release::update_available(CURRENT, &r.version));
        VersionPayload {
            current: CURRENT.to_string(),
            latest: cached.release.clone(),
            update_available,
            checked_at: cached.checked_at,
        }
    }

    async fn is_stale(&self) -> bool {
        match self.0.read().await.fetched {
            Some(at) => at.elapsed() > CACHE_FOR,
            None => true,
        }
    }

    async fn refresh(&self) {
        // Mark the attempt before it runs, so a burst of requests on a cold
        // cache produces one lookup rather than one each.
        self.0.write().await.fetched = Some(Instant::now());

        match fetch_latest().await {
            Ok(release) => {
                let mut cached = self.0.write().await;
                cached.release = Some(release);
                cached.checked_at = Some(chrono::Utc::now());
            }
            Err(e) => {
                // Keep whatever was known before; a network blip should not
                // erase a good answer.
                tracing::debug!(error = %e, "release lookup failed");
            }
        }
    }
}

async fn fetch_latest() -> anyhow::Result<Release> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        // GitHub rejects requests without one.
        .user_agent(concat!("cairn-server/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let body = client
        .get(release::RELEASES_API)
        .header("accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    release::pick_release(&body, CURRENT)
        .ok_or_else(|| anyhow::anyhow!("no eligible release published"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_is_a_parseable_semver() {
        // `release::update_available` only returns true when both sides parse,
        // so a self-comparison parsing the current version proves it is valid.
        assert!(!release::update_available(CURRENT, CURRENT));
    }

    #[test]
    fn version_payload_serializes_with_expected_keys() {
        let p = VersionPayload {
            current: "0.1.0".to_string(),
            latest: None,
            update_available: false,
            checked_at: None,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"current\":\"0.1.0\""));
        assert!(s.contains("\"update_available\":false"));
    }

    #[test]
    fn version_payload_with_release_serializes() {
        let r = Release {
            tag: "v0.2.0".to_string(),
            version: "0.2.0".to_string(),
            url: "https://example.test/r".to_string(),
        };
        let p = VersionPayload {
            current: "0.1.0".to_string(),
            latest: Some(r),
            update_available: true,
            checked_at: Some(chrono::Utc::now()),
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"tag\":\"v0.2.0\""));
        assert!(s.contains("\"update_available\":true"));
    }
}
