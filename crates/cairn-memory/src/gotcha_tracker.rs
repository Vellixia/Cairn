//! Gotcha tracker (P4.3). Detects when the same failure pattern happens multiple times
//! across sessions, and auto-promotes to a `MemoryKind::Gotcha` memory once the cluster
//! reaches the threshold (default 2 = matches `next_tier()`'s `access_count >= 2` rule).
//!
//! Mirrors `followup_tracker.rs` structurally: rolling window, in-memory, lazy eviction,
//! and exposed via `MemoryEngine` for metrics + recall. The difference: followup tracker
//! counts *retrieval* failures; this one counts *task* failures (any topic the agent
//! signals with `record_failure`).
//!
//! The class of pattern we detect is identical to the agentmemory "gotcha" pattern:
//! "I keep hitting this error / mistake - surface it next time so I don't repeat it."

use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

const DEFAULT_WINDOW: Duration = Duration::from_secs(60 * 60); // 1 hour
const DEFAULT_CLUSTER_THRESHOLD: usize = 2;
const MAX_ENTRIES: usize = 1024;

/// A single failure event. `topic` is a free-form string (typically the error class
/// or the file/concept involved); `refs` are optional entity links (file paths,
/// memory ids, etc.).
#[derive(Debug, Clone, Serialize)]
pub struct FailureEvent {
    pub topic: String,
    pub refs: Vec<String>,
    pub session_id: Option<String>,
    pub project_hash: Option<String>,
    pub context: String,
}

impl FailureEvent {
    pub fn new(topic: impl Into<String>, context: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            refs: Vec::new(),
            session_id: None,
            project_hash: None,
            context: context.into(),
        }
    }

    pub fn with_refs(mut self, refs: Vec<String>) -> Self {
        self.refs = refs;
        self
    }

    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }
}

/// One recorded failure (with its internal id + timestamp).
#[derive(Debug, Clone)]
struct FailureRecord {
    event: FailureEvent,
    fingerprint: String,
    at: Instant,
}

/// A cluster of failures sharing a fingerprint. Promoted to a gotcha memory once
/// `size` >= `cluster_threshold`.
#[derive(Debug, Clone, Serialize)]
pub struct FailureCluster {
    pub fingerprint: String,
    pub events: Vec<FailureEvent>,
    pub session_ids: HashSet<String>,
}

impl FailureCluster {
    pub fn size(&self) -> usize {
        self.events.len()
    }

    /// Stable identifier for the cluster topic (used in concept lists + apply_to).
    pub fn topic(&self) -> &str {
        self.events[0].topic.as_str()
    }
}

/// Rolling-window gotcha tracker.
#[derive(Debug)]
pub struct GotchaTracker {
    window: Duration,
    cluster_threshold: usize,
    records: VecDeque<FailureRecord>,
    /// Sessions seen in this window. When a failure spans >= 2 sessions, the cluster
    /// is marked "cross-session" and gets a confidence boost.
    pub total_failures: u64,
    pub promoted_clusters: u64,
}

impl GotchaTracker {
    pub fn new() -> Self {
        Self::with_window(DEFAULT_WINDOW)
    }

    pub fn with_window(window: Duration) -> Self {
        Self {
            window,
            cluster_threshold: DEFAULT_CLUSTER_THRESHOLD,
            records: VecDeque::with_capacity(64),
            total_failures: 0,
            promoted_clusters: 0,
        }
    }

    pub fn cluster_threshold(mut self, n: usize) -> Self {
        self.cluster_threshold = n.max(2);
        self
    }

    /// Record a failure. Returns the cluster it joined if that cluster is now large
    /// enough to be promoted (caller writes the gotcha memory).
    pub fn record(&mut self, event: FailureEvent) -> Option<FailureCluster> {
        self.total_failures += 1;
        let fingerprint = fingerprint_of(&event.topic);

        // Evict expired records (lazy). `Instant::now() - window` panics on
        // a system that has been up for less than `window`, so we fall back
        // to the earliest record we still hold (or the system epoch if the
        // tracker is empty) - either way, no expired records can exist yet.
        let now = Instant::now();
        let cutoff = now
            .checked_sub(self.window)
            .unwrap_or_else(|| self.records.front().map(|r| r.at).unwrap_or(now));
        while let Some(front) = self.records.front() {
            if front.at < cutoff {
                self.records.pop_front();
            } else {
                break;
            }
        }
        // Cap memory.
        while self.records.len() >= MAX_ENTRIES {
            self.records.pop_front();
        }

        self.records.push_back(FailureRecord {
            event,
            fingerprint: fingerprint.clone(),
            at: Instant::now(),
        });

        // Build the cluster for this fingerprint.
        let cluster = self.cluster(&fingerprint);
        if cluster.size() >= self.cluster_threshold {
            self.promoted_clusters += 1;
            Some(cluster)
        } else {
            None
        }
    }

    /// Build a cluster for the given fingerprint. Returns an empty cluster if no
    /// matching records exist.
    pub fn cluster(&self, fingerprint: &str) -> FailureCluster {
        let mut events = Vec::new();
        let mut session_ids = HashSet::new();
        for r in &self.records {
            if r.fingerprint == fingerprint {
                if let Some(sid) = &r.event.session_id {
                    session_ids.insert(sid.clone());
                }
                events.push(r.event.clone());
            }
        }
        FailureCluster {
            fingerprint: fingerprint.to_string(),
            events,
            session_ids,
        }
    }

    /// Top clusters by size, descending. Useful for proactive recall.
    pub fn top_clusters(&self, n: usize) -> Vec<FailureCluster> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<FailureCluster> = Vec::new();
        for r in &self.records {
            if seen.insert(r.fingerprint.clone()) {
                let c = self.cluster(&r.fingerprint);
                if !c.events.is_empty() {
                    out.push(c);
                }
            }
        }
        out.sort_by_key(|c| std::cmp::Reverse(c.events.len()));
        out.truncate(n);
        out
    }

    /// True when at least one cluster has reached the threshold (i.e. there's a
    /// gotcha to write).
    pub fn has_promotable(&self) -> bool {
        self.top_clusters(usize::MAX)
            .iter()
            .any(|c| c.size() >= self.cluster_threshold)
    }

    /// Reset all state. Useful for tests.
    pub fn reset(&mut self) {
        self.records.clear();
        self.total_failures = 0;
        self.promoted_clusters = 0;
    }
}

impl Default for GotchaTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Lowercase + collapsed-whitespace fingerprint. Same algorithm as followup_tracker.
fn fingerprint_of(topic: &str) -> String {
    let mut out = String::with_capacity(topic.len());
    let mut prev_ws = true;
    for c in topic.chars() {
        if c.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.extend(c.to_lowercase());
            prev_ws = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_failure_does_not_promote() {
        let mut t = GotchaTracker::new();
        let promoted = t.record(FailureEvent::new("E0308 type mismatch", "src/foo.rs:10"));
        assert!(promoted.is_none());
        assert_eq!(t.total_failures, 1);
        assert_eq!(t.promoted_clusters, 0);
    }

    #[test]
    fn two_failures_same_topic_promote() {
        let mut t = GotchaTracker::new();
        t.record(FailureEvent::new("E0308 type mismatch", "src/foo.rs:10"));
        let promoted = t.record(FailureEvent::new("E0308 type mismatch", "src/bar.rs:5"));
        assert!(
            promoted.is_some(),
            "two failures on same topic must promote"
        );
        let cluster = promoted.unwrap();
        assert_eq!(cluster.size(), 2);
        assert_eq!(cluster.fingerprint, "e0308 type mismatch");
    }

    #[test]
    fn different_topics_do_not_cluster() {
        let mut t = GotchaTracker::new();
        t.record(FailureEvent::new("E0308 type mismatch", "x"));
        let promoted = t.record(FailureEvent::new("E0432 unresolved import", "y"));
        assert!(promoted.is_none());
    }

    #[test]
    fn fingerprint_is_case_and_whitespace_insensitive() {
        let mut t = GotchaTracker::new();
        t.record(FailureEvent::new("E0308 Type Mismatch", "x"));
        let promoted = t.record(FailureEvent::new("  e0308  type  mismatch  ", "y"));
        assert!(promoted.is_some(), "normalized fingerprints should match");
    }

    #[test]
    fn cross_session_recurrence_promotes_with_two_sessions() {
        let mut t = GotchaTracker::new();
        t.record(FailureEvent::new("E0308", "x").with_session("session-a"));
        let promoted = t.record(FailureEvent::new("E0308", "y").with_session("session-b"));
        let cluster = promoted.unwrap();
        assert_eq!(cluster.session_ids.len(), 2);
    }

    #[test]
    fn outside_window_does_not_promote() {
        let mut t = GotchaTracker::with_window(Duration::from_millis(50));
        t.record(FailureEvent::new("E0308", "x"));
        std::thread::sleep(Duration::from_millis(80));
        let promoted = t.record(FailureEvent::new("E0308", "y"));
        assert!(promoted.is_none(), "after window expiry, no promotion");
    }

    #[test]
    fn top_clusters_returns_by_size_desc() {
        let mut t = GotchaTracker::new();
        for _ in 0..3 {
            t.record(FailureEvent::new("topic-A", "x"));
        }
        for _ in 0..2 {
            t.record(FailureEvent::new("topic-B", "x"));
        }
        t.record(FailureEvent::new("topic-C", "x"));
        let top = t.top_clusters(10);
        assert_eq!(top.len(), 3);
        assert_eq!(top[0].size(), 3);
        assert_eq!(top[1].size(), 2);
        assert_eq!(top[2].size(), 1);
    }

    #[test]
    fn has_promotable_true_only_after_threshold() {
        let mut t = GotchaTracker::new();
        assert!(!t.has_promotable());
        t.record(FailureEvent::new("topic-A", "x"));
        assert!(!t.has_promotable());
        t.record(FailureEvent::new("topic-A", "y"));
        assert!(t.has_promotable());
    }

    #[test]
    fn reset_clears_state() {
        let mut t = GotchaTracker::new();
        t.record(FailureEvent::new("topic-A", "x"));
        t.record(FailureEvent::new("topic-A", "y"));
        assert_eq!(t.promoted_clusters, 1);
        t.reset();
        assert_eq!(t.total_failures, 0);
        assert_eq!(t.promoted_clusters, 0);
    }

    #[test]
    fn cluster_collects_all_events_sharing_fingerprint() {
        let mut t = GotchaTracker::new();
        t.record(FailureEvent::new("E0308", "first"));
        t.record(FailureEvent::new("E9999", "different"));
        t.record(FailureEvent::new("E0308", "third"));
        let cluster = t.cluster("e0308");
        assert_eq!(cluster.size(), 2);
        assert_eq!(cluster.events[0].context, "first");
        assert_eq!(cluster.events[1].context, "third");
    }
}
