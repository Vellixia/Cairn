//! Sync envelopes + vector clocks (v0.5.0 Sprint 15a).
//!
//! Two devices exchanging edits need to agree on the causal order of operations. A
//! vector clock is the standard tool: each device keeps a counter per peer; the clock
//! advances on every local write; on merge, both sides take the per-peer max. Comparing
//! two clocks tells you whether one strictly precedes the other (no conflict), is
//! concurrent (conflict), or is equal.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Vector clock for tracking causal order across replicas. Each entry is "actor name ->
/// events seen from that actor". Two events are concurrent iff their clocks are
/// incomparable (neither dominates the other).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorClock {
    counters: BTreeMap<String, u64>,
}

impl VectorClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment this device's slot. Returns the new clock so the caller can attach
    /// it to the resulting event.
    pub fn tick(&mut self, actor: &str) -> Self {
        let entry = self.counters.entry(actor.to_string()).or_insert(0);
        *entry += 1;
        self.clone()
    }

    /// Merge another clock into this one. Per-actor max. After the merge, this clock
    /// dominates the other.
    pub fn merge(&mut self, other: &Self) {
        for (actor, count) in &other.counters {
            let entry = self.counters.entry(actor.clone()).or_insert(0);
            if *count > *entry {
                *entry = *count;
            }
        }
    }

    /// True if `self` strictly dominates `other` (every counter >=, and at least one >).
    pub fn dominates(&self, other: &Self) -> bool {
        let mut strictly_greater = false;
        for (actor, count) in &other.counters {
            let ours = self.counters.get(actor).copied().unwrap_or(0);
            if *count > ours {
                return false;
            }
            if *count < ours {
                strictly_greater = true;
            }
        }
        for (actor, count) in &self.counters {
            if !other.counters.contains_key(actor) && *count > 0 {
                strictly_greater = true;
            }
        }
        strictly_greater
    }

    /// True if the two clocks are equal or one dominates the other (no concurrent edits).
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.dominates(other) || other.dominates(self) || self == other
    }

    pub fn get(&self, actor: &str) -> u64 {
        self.counters.get(actor).copied().unwrap_or(0)
    }
}

/// One memory edit operation. Carries the vector clock so the receiver can detect
/// concurrent edits and apply conflict-resolution rules (e.g. OR-Set merge for tags,
/// last-write-by-clock for content).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MemoryOp {
    /// New memory (or upsert). The receiver stores it with the supplied id.
    Put {
        id: String,
        content: String,
        importance: f32,
        /// Tags and concepts are OR-Sets - concurrent adds on different sides merge.
        tags: BTreeSet<String>,
        concepts: BTreeSet<String>,
        confidence: f64,
        access_count: u64,
        ts: chrono::DateTime<chrono::Utc>,
        clock: VectorClock,
    },
    /// Counter increments for an existing memory. access_count and confidence use
    /// GCounter semantics so concurrent bumps on different sides sum.
    Bump {
        id: String,
        access_count_delta: u64,
        confidence_delta: u64,
        clock: VectorClock,
    },
    /// Remove a memory and all its variants.
    Tombstone { id: String, clock: VectorClock },
}

use std::collections::BTreeSet;

impl MemoryOp {
    pub fn clock(&self) -> &VectorClock {
        match self {
            MemoryOp::Put { clock, .. }
            | MemoryOp::Bump { clock, .. }
            | MemoryOp::Tombstone { clock, .. } => clock,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            MemoryOp::Put { id, .. }
            | MemoryOp::Bump { id, .. }
            | MemoryOp::Tombstone { id, .. } => id,
        }
    }
}

/// What one device sends to another during a sync round. The `clock` field is the
/// sender's current vector clock - the receiver merges it to update its own view of
/// the world before applying any ops.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncEnvelope {
    pub from: String,
    pub to: String,
    pub clock: VectorClock,
    pub ops: Vec<MemoryOp>,
}

/// The outcome of applying a remote envelope on the local store. `concurrent` lists
/// the ids where the remote op happened in parallel with a local op - those need
/// conflict-resolution handling before they're truly "merged".
#[derive(Debug, Clone, Default)]
pub struct SyncResult {
    pub applied: Vec<String>,
    pub concurrent: Vec<String>,
    pub skipped: Vec<String>,
}

/// State carried on each side of a sync - the actor's own name + its clock.
#[derive(Debug, Clone)]
pub struct SyncPeer {
    pub name: String,
    pub clock: VectorClock,
}

impl SyncPeer {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            clock: VectorClock::new(),
        }
    }

    /// Apply a single remote op to the local store model. This is where the CRDT
    /// merge logic lives - the caller hands us a function that knows how to read /
    /// write the local memory store.
    ///
    /// Returns the action taken: applied / concurrent / skipped.
    pub fn apply_op<F>(&mut self, op: &MemoryOp, is_known: F) -> SyncAction
    where
        F: Fn(&str) -> Option<VectorClock>,
    {
        match op {
            MemoryOp::Put { clock, .. } => {
                let local = is_known(op.id());
                match local {
                    None => SyncAction::Applied,
                    Some(local_clock) if clock.dominates(&local_clock) => SyncAction::Applied,
                    Some(local_clock) if local_clock.dominates(clock) => SyncAction::Skipped,
                    Some(_) => SyncAction::Concurrent,
                }
            }
            MemoryOp::Bump { clock, .. } => {
                let local = is_known(op.id());
                match local {
                    None => SyncAction::Applied,
                    Some(local_clock) if clock.is_compatible_with(&local_clock) => {
                        SyncAction::Applied
                    }
                    Some(_) => SyncAction::Concurrent,
                }
            }
            MemoryOp::Tombstone { clock, .. } => {
                let local = is_known(op.id());
                match local {
                    None => SyncAction::Skipped, // Already gone.
                    Some(local_clock) if clock.dominates(&local_clock) => SyncAction::Applied,
                    Some(_) => SyncAction::Concurrent,
                }
            }
        }
    }

    /// Apply an envelope, returning what happened. The caller passes a callback to
    /// look up the local clock for a given memory id (so this module stays storage-
    /// agnostic).
    pub fn apply_envelope<F>(&mut self, envelope: &SyncEnvelope, is_known: F) -> SyncResult
    where
        F: Fn(&str) -> Option<VectorClock>,
    {
        // Always advance our clock first.
        self.clock.merge(&envelope.clock);
        let mut result = SyncResult::default();
        for op in &envelope.ops {
            match self.apply_op(op, &is_known) {
                SyncAction::Applied => result.applied.push(op.id().to_string()),
                SyncAction::Concurrent => result.concurrent.push(op.id().to_string()),
                SyncAction::Skipped => result.skipped.push(op.id().to_string()),
            }
        }
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    Applied,
    Concurrent,
    Skipped,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn put(id: &str, actor: &str, content: &str) -> MemoryOp {
        let mut clock = VectorClock::new();
        clock.tick(actor);
        MemoryOp::Put {
            id: id.to_string(),
            content: content.to_string(),
            importance: 0.5,
            tags: BTreeSet::new(),
            concepts: BTreeSet::new(),
            confidence: 0.5,
            access_count: 0,
            ts: Utc::now(),
            clock,
        }
    }

    #[test]
    fn vector_clock_dominates_and_concurrent() {
        let mut alice = VectorClock::new();
        alice.tick("alice");
        alice.tick("alice");
        alice.tick("bob"); // alice sees bob@1

        let mut bob = VectorClock::new();
        bob.tick("bob");

        assert!(alice.dominates(&bob));
        assert!(!bob.dominates(&alice));

        // Now bob ticks once more - concurrent with alice's view of bob@2.
        bob.tick("bob");
        assert!(!alice.dominates(&bob));
        assert!(!bob.dominates(&alice));
        assert!(!alice.is_compatible_with(&bob));
    }

    #[test]
    fn sync_offline_puts_are_detected_as_concurrent() {
        // alice and bob both create "memory-1" while offline.
        let alice_put = put("memory-1", "alice", "alice's version");
        let bob_put = put("memory-1", "bob", "bob's version");

        let mut alice = SyncPeer::new("alice");
        let _bob = SyncPeer::new("bob");

        // Each side applies its own put - locally it's "applied".
        let r_alice_self = alice.apply_envelope(
            &SyncEnvelope {
                from: "alice".into(),
                to: "alice".into(),
                clock: alice_put.clock().clone(),
                ops: vec![alice_put.clone()],
            },
            |_| None,
        );
        assert_eq!(r_alice_self.applied, vec!["memory-1"]);

        // Bob sends his put to alice. The clock is concurrent - neither dominates.
        let r = alice.apply_envelope(
            &SyncEnvelope {
                from: "bob".into(),
                to: "alice".into(),
                clock: bob_put.clock().clone(),
                ops: vec![bob_put],
            },
            |id| {
                if id == "memory-1" {
                    Some(alice_put.clock().clone())
                } else {
                    None
                }
            },
        );
        assert_eq!(r.concurrent, vec!["memory-1"]);
        assert!(r.applied.is_empty());
    }

    #[test]
    fn sync_bump_concurrent_with_put_is_concurrent() {
        // alice creates memory-1, then bob bumps access_count. The clocks are
        // concurrent - neither dominates the other because alice bumped her clock
        // for the put but not for the bump she hasn't seen yet.
        let mut alice = SyncPeer::new("alice");
        let alice_put = put("memory-1", "alice", "hello");
        alice.apply_envelope(
            &SyncEnvelope {
                from: "alice".into(),
                to: "alice".into(),
                clock: alice_put.clock().clone(),
                ops: vec![alice_put],
            },
            |_| None,
        );

        // Bob's bump with his own clock - alice doesn't know about bob yet.
        let mut bob_clock = VectorClock::new();
        bob_clock.tick("bob");
        let bob_bump = MemoryOp::Bump {
            id: "memory-1".into(),
            access_count_delta: 1,
            confidence_delta: 0,
            clock: bob_clock.clone(),
        };

        let r = alice.apply_envelope(
            &SyncEnvelope {
                from: "bob".into(),
                to: "alice".into(),
                clock: bob_clock,
                ops: vec![bob_bump],
            },
            |id| {
                if id == "memory-1" {
                    Some(VectorClock {
                        counters: BTreeMap::from([("alice".into(), 1)]),
                    })
                } else {
                    None
                }
            },
        );
        assert_eq!(r.concurrent, vec!["memory-1"]);
    }

    #[test]
    fn sync_envelope_advances_local_clock() {
        let mut alice = SyncPeer::new("alice");
        alice.clock.tick("alice");
        let before = alice.clock.clone();

        let mut bob_clock = VectorClock::new();
        bob_clock.tick("bob");
        bob_clock.tick("bob");

        let envelope = SyncEnvelope {
            from: "bob".into(),
            to: "alice".into(),
            clock: bob_clock,
            ops: vec![],
        };
        alice.apply_envelope(&envelope, |_| None);
        // alice's clock should now include bob@2.
        assert_eq!(alice.clock.get("bob"), 2);
        assert!(alice.clock.get("alice") >= before.get("alice"));
    }
}
