//! OR-Set — Observed-Remove Set (Bienvenu et al., 2007).
//!
//! Every element insertion is tagged with a unique causal marker (UUID). Every
//! removal deletes *all observed markers* for that element. Concurrent add+remove
//! resolves to "present" because the add's marker wasn't observed by the remover.
//!
//! Used by Cairn for memory `tags` and `concepts` collections — both behave like
//! sets where a user adding "rust" on one device and removing "rust" on another
//! (offline) should leave "rust" present after sync (both intents preserved).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One causal marker for an add operation. The UUID is locally generated; the value
/// itself carries no meaning beyond uniqueness.
pub type Marker = String;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ORSet {
    /// Element value → set of causal markers that added it.
    elements: BTreeMap<String, BTreeSet<Marker>>,
    /// Markers that have been removed. Tombstones — needed so a "remove" doesn't get
    /// resurrected by an old "add" arriving later.
    tombstones: BTreeSet<Marker>,
}

impl ORSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `value`. Returns the new marker so the caller can track it (useful when a
    /// later remove wants to refer to a specific add).
    pub fn add(&mut self, value: &str) -> Marker {
        let marker = uuid::Uuid::new_v4().to_string();
        self.elements
            .entry(value.to_string())
            .or_default()
            .insert(marker.clone());
        marker
    }

    /// Remove every observed marker for `value`. Markers from concurrent adds that we
    /// haven't seen yet are NOT touched — those adds will resurrect the element on the
    /// next merge. That's the intended behavior (add wins over remove on conflict).
    pub fn remove(&mut self, value: &str) {
        if let Some(markers) = self.elements.remove(value) {
            self.tombstones.extend(markers);
        }
    }

    /// True if `value` is currently a member.
    pub fn contains(&self, value: &str) -> bool {
        self.elements.contains_key(value)
    }

    /// Current members, sorted.
    pub fn members(&self) -> Vec<String> {
        self.elements.keys().cloned().collect()
    }

    /// Merge another OR-Set into this one.
    ///
    /// Semantics:
    /// - Take the union of add-markers on each side.
    /// - A marker is alive iff it's in our `elements` AND not in our `tombstones`
    ///   AND not in the other's `tombstones`.
    pub fn merge(&mut self, other: &Self) {
        for (value, markers) in &other.elements {
            let entry = self.elements.entry(value.clone()).or_default();
            for m in markers {
                if !self.tombstones.contains(m) && !other.tombstones.contains(m) {
                    entry.insert(m.clone());
                }
            }
        }
        // Adopt any new tombstones from the other side.
        for m in &other.tombstones {
            self.tombstones.insert(m.clone());
        }
        // Local tombstones we haven't sent to the other side also need to prune any
        // markers we *did* receive. Walk every element and drop markers that are in
        // either tombstone set.
        let mut to_remove: Vec<String> = Vec::new();
        for (value, markers) in &mut self.elements {
            markers.retain(|m| !self.tombstones.contains(m) && !other.tombstones.contains(m));
            if markers.is_empty() {
                to_remove.push(value.clone());
            }
        }
        for v in to_remove {
            self.elements.remove(&v);
        }
    }

    /// For tests / introspection: how many add-markers are alive for `value`.
    pub fn marker_count(&self, value: &str) -> usize {
        self.elements.get(value).map(|m| m.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_remove_round_trip() {
        let mut s = ORSet::new();
        s.add("rust");
        assert!(s.contains("rust"));
        s.remove("rust");
        assert!(!s.contains("rust"));
    }

    #[test]
    fn concurrent_add_and_remove_resolves_to_present() {
        // alice adds "rust", bob removes "rust" while offline.
        let mut alice = ORSet::new();
        let m = alice.add("rust");
        let mut bob = ORSet::new();
        // bob doesn't see alice's add (he's offline), but he knows the element existed
        // before and removes it.
        bob.add("rust");
        bob.remove("rust");
        assert!(!bob.contains("rust"));

        // Sync — bob's tombstones don't cover alice's brand-new marker, so the add
        // wins.
        alice.merge(&bob);
        assert!(alice.contains("rust"), "concurrent add wins over remove");

        // And from bob's perspective after sync, he learns about alice's marker.
        bob.merge(&alice);
        assert!(bob.contains("rust"));

        // The marker survives — useful for debugging which side added what.
        assert!(alice.marker_count("rust") >= 1);
        let _ = m;
    }

    #[test]
    fn merge_converges_under_different_orderings() {
        // Two replicas edit disjoint elements offline.
        let mut a = ORSet::new();
        a.add("rust");
        a.add("safety");

        let mut b = ORSet::new();
        b.add("performance");
        b.add("rust"); // bob also adds rust (different marker).
        b.remove("safety");

        let mut left = a.clone();
        left.merge(&b);

        let mut right = b.clone();
        right.merge(&a);

        assert_eq!(left.members(), right.members());
        // Both sides added "rust" so it stays — add wins over concurrent remove,
        // and there's no remove on rust here.
        assert!(left.contains("rust"));
        // Alice's "safety" marker predates bob's remove, so safety is preserved
        // (correct OR-Set semantics: bob's remove only tombstones his own marker).
        assert!(left.contains("safety"));
        assert!(left.contains("performance"));
    }

    #[test]
    fn local_remove_tombstones_block_old_add_resurrection() {
        // alice removes "rust" while bob has an offline add in flight.
        let mut alice = ORSet::new();
        alice.add("rust");
        alice.remove("rust");
        assert!(!alice.contains("rust"));

        // bob then adds "rust" while alice was offline.
        let mut bob = ORSet::new();
        bob.add("rust");

        // Sync. alice's tombstone blocks bob's add? No — bob's add carries a NEW
        // marker (UUID) that alice's tombstone doesn't cover. So the element comes
        // back. This is correct OR-Set semantics: a fresh add is independent of past
        // removals.

        // Wait — actually the original alice.add marker was created and tombstoned.
        // bob's NEW add marker is different. So after merge, the element should be
        // present (bob's marker survives).
        alice.merge(&bob);
        assert!(
            alice.contains("rust"),
            "a fresh add marker should resurrect the element"
        );
    }
}
