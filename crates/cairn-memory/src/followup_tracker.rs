//! Followup tracker (P1.6). Detects when an agent re-queries with a disjoint result set
//! within a short window - a proxy for "retrieval failed the first time, the agent
//! tried again."
//!
//! Each `record()` call adds the query's result set to a rolling window. If the
//! next call in the window has the same query fingerprint AND its result set is
//! disjoint from the prior one, that's a followup. A high followup rate signals
//! that retrieval is producing poor recall for some queries.

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

const DEFAULT_WINDOW: Duration = Duration::from_secs(30);

/// One recorded recall.
#[derive(Debug, Clone)]
struct RecallEntry {
    fingerprint: String,
    ids: HashSet<String>,
    at: Instant,
}

/// Tracks recent recalls to detect followup retrievals.
#[derive(Debug)]
pub struct FollowupTracker {
    window: Duration,
    entries: VecDeque<RecallEntry>,
    pub queries: u64,
    pub followups: u64,
    max_entries: usize,
}

impl FollowupTracker {
    pub fn new() -> Self {
        Self::with_window(DEFAULT_WINDOW)
    }

    pub fn with_window(window: Duration) -> Self {
        Self {
            window,
            entries: VecDeque::with_capacity(64),
            queries: 0,
            followups: 0,
            max_entries: 256,
        }
    }

    /// Record a recall. `query` is used as the fingerprint (lowercased + trimmed).
    /// `ids` is the result set. Returns `true` if this counts as a followup.
    pub fn record(&mut self, query: &str, ids: &[String]) -> bool {
        self.queries += 1;
        let fingerprint = fingerprint_of(query);
        let ids_set: HashSet<String> = ids.iter().cloned().collect();

        // Evict expired entries. Same defensive clamp as `gotcha_tracker::record`:
        // `Instant::now() - window` panics on a system up for less than `window`.
        let now = Instant::now();
        let cutoff = now
            .checked_sub(self.window)
            .unwrap_or_else(|| self.entries.front().map(|e| e.at).unwrap_or(now));
        while let Some(front) = self.entries.front() {
            if front.at < cutoff {
                self.entries.pop_front();
            } else {
                break;
            }
        }
        // Cap memory
        while self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }

        // Check for followup: same fingerprint in the window AND disjoint result set
        let mut is_followup = false;
        for prior in self.entries.iter() {
            if prior.fingerprint != fingerprint {
                continue;
            }
            // disjoint = no overlap AND non-empty (an empty result isn't a followup candidate)
            if !prior.ids.is_empty() && !ids_set.is_empty() && prior.ids.is_disjoint(&ids_set) {
                is_followup = true;
                break;
            }
        }

        if is_followup {
            self.followups += 1;
        }

        self.entries.push_back(RecallEntry {
            fingerprint,
            ids: ids_set,
            at: Instant::now(),
        });

        is_followup
    }

    /// Fraction of queries that were followups. 0.0 if no queries yet.
    pub fn followup_rate(&self) -> f64 {
        if self.queries == 0 {
            0.0
        } else {
            self.followups as f64 / self.queries as f64
        }
    }

    /// Reset the tracker. Useful for tests.
    pub fn reset(&mut self) {
        self.entries.clear();
        self.queries = 0;
        self.followups = 0;
    }
}

impl Default for FollowupTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Lowercase + trimmed + collapsed-whitespace fingerprint. Good enough to catch
/// the same query repeated within 30s; not cryptographic.
fn fingerprint_of(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    let mut prev_ws = true;
    for c in query.chars() {
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
    fn first_query_is_not_a_followup() {
        let mut t = FollowupTracker::new();
        let is_fu = t.record("hello world", &["a".into(), "b".into()]);
        assert!(!is_fu);
        assert_eq!(t.queries, 1);
        assert_eq!(t.followups, 0);
    }

    #[test]
    fn disjoint_results_count_as_followup() {
        let mut t = FollowupTracker::new();
        t.record("how does auth work", &["a".into(), "b".into()]);
        let is_fu = t.record("how does auth work", &["c".into(), "d".into()]);
        assert!(is_fu, "disjoint result set in window should be a followup");
        assert_eq!(t.queries, 2);
        assert_eq!(t.followups, 1);
        assert!((t.followup_rate() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn overlapping_results_are_not_followup() {
        let mut t = FollowupTracker::new();
        t.record("q", &["a".into(), "b".into(), "c".into()]);
        let is_fu = t.record("q", &["a".into(), "d".into()]);
        assert!(!is_fu, "overlapping result set should NOT be a followup");
        assert_eq!(t.followups, 0);
    }

    #[test]
    fn empty_results_dont_count_as_followup() {
        let mut t = FollowupTracker::new();
        t.record("q", &["a".into()]);
        let is_fu = t.record("q", &[]);
        assert!(!is_fu, "empty result set should NOT be a followup");
    }

    #[test]
    fn different_queries_dont_count_as_followup() {
        let mut t = FollowupTracker::new();
        t.record("how does auth work", &["a".into()]);
        let is_fu = t.record("how does the cache work", &["b".into()]);
        assert!(!is_fu);
    }

    #[test]
    fn case_and_whitespace_insensitive() {
        let mut t = FollowupTracker::new();
        t.record("How does auth work?", &["a".into()]);
        let is_fu = t.record("  how DOES auth   work?  ", &["b".into()]);
        assert!(is_fu, "normalized fingerprints should match");
    }

    #[test]
    fn outside_window_not_a_followup() {
        let mut t = FollowupTracker::with_window(Duration::from_millis(50));
        t.record("q", &["a".into()]);
        std::thread::sleep(Duration::from_millis(80));
        let is_fu = t.record("q", &["b".into()]);
        assert!(!is_fu, "after window expiry, no followup");
    }

    #[test]
    fn reset_clears_state() {
        let mut t = FollowupTracker::new();
        t.record("q", &["a".into()]);
        t.record("q", &["b".into()]);
        assert_eq!(t.followups, 1);
        t.reset();
        assert_eq!(t.queries, 0);
        assert_eq!(t.followups, 0);
    }
}
