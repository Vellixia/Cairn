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
use uuid::Uuid;

/// How long a lookup stands before it is worth asking again.
///
/// GitHub allows 60 unauthenticated calls an hour per address. A deployment
/// serving a team must not spend that on page loads, and a release nobody
/// hears about for six hours has harmed no one.
const CACHE_FOR: Duration = Duration::from_secs(6 * 60 * 60);

/// The version this binary was built as.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// What a Feature 003 daemon must know before it queues work here (FR-415).
///
/// Named rather than inferred from the version string, because a deployment can
/// be newer and still lack a capability, and because the daemon must be able to
/// ask about one thing without matching on releases.
///
/// A server that predates this field answers without it, and its **absence** is
/// the answer: no relations, no criteria, no blockers, no subject identity. The
/// daemon needs no probe endpoint and no version table (D81).
pub const SCHEMA_2_CAPABILITIES: &[&str] = &[
    "memory_relations",
    "task_criteria",
    "task_blockers",
    "memory_subject_identity",
    "memory_verification",
];

/// What a Feature 004 daemon must know before it queues personal or team
/// knowledge here (FR-521, FR-529).
///
/// Extends `SCHEMA_2_CAPABILITIES` additively — every Feature 003 capability
/// name stays present, unchanged — with the two new domains this schema
/// version's tables can hold. A server that predates this field answers
/// without it, the same "absence is the answer" discipline `SCHEMA_2_CAPABILITIES`
/// established: no probe endpoint, no version table, just the field itself.
pub const SCHEMA_3_CAPABILITIES: &[&str] = &[
    "memory_relations",
    "task_criteria",
    "task_blockers",
    "memory_subject_identity",
    "memory_verification",
    "personal_knowledge",
    "team_knowledge",
];

/// What a Feature 005 daemon must know before it queues autonomous-memory work
/// here (FR-774, FR-775).
///
/// Extends `SCHEMA_3_CAPABILITIES` additively, the same discipline the previous
/// two lists follow: every earlier name stays present and unchanged, so a
/// Feature 003 or 004 daemon reading this list finds exactly what it found
/// before and nothing it has to understand.
///
/// One name per kind of work a client can queue that this schema's tables must
/// exist to hold. `safe_events` is the one the cutover contract writes out
/// verbatim (`contracts/migration-cutover.md` §1); the rest follow the same
/// rule, because a daemon that queued a knowledge command, a pattern promotion
/// or a verification report against a v3 server would have it fail on every
/// attempt rather than being told once that the server cannot hold it.
pub const SCHEMA_4_CAPABILITIES: &[&str] = &[
    "memory_relations",
    "task_criteria",
    "task_blockers",
    "memory_subject_identity",
    "memory_verification",
    "personal_knowledge",
    "team_knowledge",
    "safe_events",
    "knowledge_commands",
    "shared_patterns",
    "verification_reports",
    "context_retrieval",
];

/// Whether this deployment has cut over to server authority
/// (`contracts/migration-cutover.md` §1).
///
/// A structure rather than two loose fields, because the pair is only
/// meaningful together: `cutover_at` without `mode` says nothing, and a
/// `cutover_at` on a `pre_cutover` deployment would be a contradiction rather
/// than an absence.
///
/// Absent below schema 4, where `server_authority` does not exist. Its absence
/// is the answer — the same "no probe endpoint, no version table" discipline
/// `SCHEMA_2_CAPABILITIES` established — and a client seeing no `authority`
/// field knows it is talking to a server that has no cutover to report.
#[derive(Debug, Clone, Serialize)]
pub struct AuthorityPayload {
    pub mode: String,
    pub cutover_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Read this deployment's cutover state.
///
/// From the database on every call, deliberately not cached on the
/// application state. An administrator can throw this switch while the process
/// is running, and a client polls this endpoint precisely to find out that they
/// have — a value captured at start-up would say `pre_cutover` until the next
/// restart, which is the one answer that must not be stale.
///
/// `None` below schema 4, where the table does not exist. A read error is also
/// `None`: the version endpoint is unauthenticated and must not fail, and an
/// absent field already means "this server has no cutover to report", which is
/// the safe reading.
pub async fn authority_for(pool: &sqlx::PgPool, schema_version: i64) -> Option<AuthorityPayload> {
    if schema_version < 4 {
        return None;
    }
    let row: Option<(String, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as("SELECT mode, cutover_at FROM server_authority WHERE id = 1")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    row.map(|(mode, cutover_at)| AuthorityPayload { mode, cutover_at })
}

/// What a deployment at `schema_version` can hold.
///
/// Derived from the schema the database **applied**, so a server held at an
/// earlier migration advertises what it really has rather than what its binary
/// could do. Advertising a capability whose table is absent would make the
/// daemon queue work that then fails on every attempt.
pub fn capabilities_for(schema_version: i64) -> &'static [&'static str] {
    if schema_version >= 4 {
        SCHEMA_4_CAPABILITIES
    } else if schema_version >= 3 {
        SCHEMA_3_CAPABILITIES
    } else if schema_version >= 2 {
        SCHEMA_2_CAPABILITIES
    } else {
        &[]
    }
}

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
    /// The highest migration this deployment has applied (FR-415).
    ///
    /// Added additively: every field a Feature 001 or 002 consumer reads keeps
    /// its name, its type and its value.
    pub schema_version: i64,
    /// What kinds of Feature 003 record this deployment can hold.
    pub capabilities: Vec<String>,
    /// This server's own identity, established once and never reassigned
    /// (FR-415, FR-416).
    ///
    /// Discoverable by every client that can reach the server, because a local
    /// store has to pin its team knowledge to one instance and refuse a second
    /// one's — which it cannot do without a way to ask "which server is this?".
    ///
    /// Absent below schema 3, where the table it comes from does not exist yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_instance_id: Option<Uuid>,
    /// Whether this deployment has cut over to server authority (FR-876).
    ///
    /// Absent below schema 4. A client probes this the way it already probes
    /// schema capability, so no new polling mechanism exists to maintain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<AuthorityPayload>,
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
    pub async fn payload(
        &self,
        schema_version: i64,
        server_instance_id: Option<Uuid>,
        authority: Option<AuthorityPayload>,
    ) -> VersionPayload {
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
            schema_version,
            capabilities: capabilities_for(schema_version)
                .iter()
                .map(|c| c.to_string())
                .collect(),
            server_instance_id,
            authority,
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
        // Asserted against a known-lower baseline, not against itself.
        // `update_available` returns false whenever *either* side fails to
        // parse, so `!update_available(CURRENT, CURRENT)` holds just as well
        // for a version that is not semver at all — it proved nothing. This
        // can only pass if `CURRENT` parses and orders above 0.0.0.
        assert!(
            release::update_available("0.0.0", CURRENT),
            "CURRENT ({CURRENT}) must be parseable semver above 0.0.0"
        );
    }

    #[test]
    fn version_payload_serializes_with_expected_keys() {
        let p = VersionPayload {
            current: "0.1.0".to_string(),
            latest: None,
            update_available: false,
            checked_at: None,
            schema_version: crate::db::SCHEMA_VERSION,
            capabilities: SCHEMA_2_CAPABILITIES
                .iter()
                .map(|c| c.to_string())
                .collect(),
            server_instance_id: None,
            authority: None,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"current\":\"0.1.0\""));
        assert!(s.contains("\"update_available\":false"));
    }

    /// Every earlier capability name survives every later list.
    ///
    /// The additive rule is the whole contract: a Feature 003 daemon reading a
    /// v4 server's list has to find the five names it knows, in a list it can
    /// ignore the rest of. Dropping or renaming one would make an older client
    /// stop queueing work it is perfectly able to queue, and it would do so
    /// silently.
    #[test]
    fn each_schema_advertises_a_superset_of_the_one_before() {
        for (lower, higher) in [
            (SCHEMA_2_CAPABILITIES, SCHEMA_3_CAPABILITIES),
            (SCHEMA_3_CAPABILITIES, SCHEMA_4_CAPABILITIES),
        ] {
            for name in lower {
                assert!(
                    higher.contains(name),
                    "capability {name} disappeared from a later schema"
                );
            }
            assert!(higher.len() > lower.len(), "a later schema added nothing");
        }
    }

    #[test]
    fn capabilities_follow_the_schema_the_database_applied() {
        // Not the binary's own version: a deployment held at an earlier
        // migration must advertise what it really has, or the daemon queues
        // work against tables that do not exist.
        assert!(capabilities_for(1).is_empty());
        assert_eq!(capabilities_for(2), SCHEMA_2_CAPABILITIES);
        assert_eq!(capabilities_for(3), SCHEMA_3_CAPABILITIES);
        assert_eq!(capabilities_for(4), SCHEMA_4_CAPABILITIES);
        // A server ahead of this build still advertises what this build knows
        // how to describe, rather than guessing at names it has never heard.
        assert_eq!(capabilities_for(99), SCHEMA_4_CAPABILITIES);
    }

    #[test]
    fn authority_is_absent_rather_than_guessed_when_the_server_has_none() {
        let p = VersionPayload {
            current: "0.1.0".to_string(),
            latest: None,
            update_available: false,
            checked_at: None,
            schema_version: 3,
            capabilities: vec![],
            server_instance_id: None,
            authority: None,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(
            !s.contains("authority"),
            "a pre-v4 server implied a cutover state it cannot have"
        );

        let p = VersionPayload {
            authority: Some(AuthorityPayload {
                mode: "pre_cutover".to_string(),
                cutover_at: None,
            }),
            schema_version: 4,
            ..p
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"mode\":\"pre_cutover\""));
        assert!(s.contains("\"cutover_at\":null"));
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
            schema_version: crate::db::SCHEMA_VERSION,
            capabilities: SCHEMA_2_CAPABILITIES
                .iter()
                .map(|c| c.to_string())
                .collect(),
            server_instance_id: None,
            authority: None,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"tag\":\"v0.2.0\""));
        assert!(s.contains("\"update_available\":true"));
    }
}
