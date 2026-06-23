//! GCounter — grow-only counter (a classic Shapiro / Preguiça CRDT).
//!
//! Each replica maintains a per-actor counter. On merge, every per-actor value is the
//! `max` of the two sides. On increment, the local actor's slot goes up by 1.
//!
//! The total count is the sum of every actor's slot. Concurrent increments on different
//! replicas both survive a merge — that's the whole point of a CRDT: data loss is
//! impossible by construction.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::Add;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GCounter {
    /// Per-actor counts. BTreeMap keeps the JSON output key-sorted so two replicas
    /// converge to identical bytes after the same operations.
    counts: BTreeMap<String, u64>,
}

impl GCounter {
    /// New empty counter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the local replica's counter by `delta` (default 1). Returns the new
    /// total count.
    pub fn increment(&mut self, actor: &str, delta: u64) -> u64 {
        let entry = self.counts.entry(actor.to_string()).or_insert(0);
        *entry = entry.saturating_add(delta);
        self.total()
    }

    /// Total count across every actor.
    pub fn total(&self) -> u64 {
        self.counts.values().sum()
    }

    /// Merge another counter into this one. Per-actor max.
    pub fn merge(&mut self, other: &Self) {
        for (actor, count) in &other.counts {
            let entry = self.counts.entry(actor.clone()).or_insert(0);
            if *count > *entry {
                *entry = *count;
            }
        }
    }

    /// Per-actor snapshot (for tests).
    pub fn per_actor(&self) -> &BTreeMap<String, u64> {
        &self.counts
    }
}

impl Add for GCounter {
    type Output = GCounter;
    fn add(mut self, rhs: GCounter) -> GCounter {
        self.merge(&rhs);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increment_tracks_per_actor_totals() {
        let mut c = GCounter::new();
        assert_eq!(c.increment("alice", 3), 3);
        assert_eq!(c.increment("bob", 5), 8);
        assert_eq!(c.increment("alice", 2), 10);
        assert_eq!(c.per_actor().get("alice"), Some(&5));
        assert_eq!(c.per_actor().get("bob"), Some(&5));
    }

    #[test]
    fn concurrent_increments_survive_a_merge() {
        // alice and bob both increment while offline. Each side has its own
        // per-actor count. After merge, both contributions survive.
        let mut alice = GCounter::new();
        alice.increment("alice", 4);

        let mut bob = GCounter::new();
        bob.increment("bob", 7);

        alice.merge(&bob);
        assert_eq!(alice.total(), 11);
    }

    #[test]
    fn merge_takes_per_actor_max_not_sum() {
        // If alice's local count is 5 and the incoming value is 3, merge keeps 5.
        // Sum would double-count.
        let mut a = GCounter::new();
        a.increment("alice", 5);

        let mut incoming = GCounter::new();
        incoming.increment("alice", 3);

        a.merge(&incoming);
        assert_eq!(a.per_actor().get("alice"), Some(&5));
        assert_eq!(a.total(), 5);
    }

    #[test]
    fn merge_is_commutative_and_associative() {
        // CRDTs converge under any merge order.
        let mut a = GCounter::new();
        a.increment("alice", 1);
        a.increment("bob", 2);

        let mut b = GCounter::new();
        b.increment("alice", 3);
        b.increment("carol", 4);

        let mut c = GCounter::new();
        c.increment("bob", 1);
        c.increment("carol", 5);

        let mut left = a.clone();
        left.merge(&b);
        left.merge(&c);

        let mut right = a.clone();
        right.merge(&c);
        right.merge(&b);

        assert_eq!(left, right, "GCounter merge must converge");
    }

    #[test]
    fn serializes_with_key_sorted_json_for_deterministic_convergence() {
        let mut a = GCounter::new();
        a.increment("alice", 1);
        a.increment("bob", 2);

        // Two independently-built counters with the same content produce identical
        // JSON — important for byte-level convergence.
        let mut b = GCounter::new();
        b.increment("bob", 2);
        b.increment("alice", 1);

        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }
}
