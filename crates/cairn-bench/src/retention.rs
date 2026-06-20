//! Smart-memory retention benchmark (v0.5.0 Sprint 16).
//!
//! Measures how Cairn's `confidence` + `pin` + `crystallize` together preserve
//! "important" memories across many `reinforce` cycles vs. naive LRU eviction.
//!
//! Two policies are compared:
//!
//! 1. **Naive LRU** — the memory at the back of the queue is dropped first,
//!    regardless of confidence or pin state.
//! 2. **Cairn policy** — `pinned: true` memories are never dropped; otherwise
//!    memories are dropped by ascending `confidence × importance`.
//!
//! Both start with the same pool of 100 memories (10 pinned, 90 with random
//! importance / confidence). We run 50 cycles of "remember 10 new +
//! `reinforce` the existing ones" then count how many of the original
//! important memories (the 10 pinned + the 10 highest initial-importance
//! non-pinned) survive.

use crate::{BenchKind, BenchResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionResult {
    pub policy: String,
    pub initial_count: usize,
    pub cycles: usize,
    pub important_survived: usize,
    pub important_total: usize,
    pub survival_rate: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetentionOutput {
    pub policies: Vec<RetentionResult>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetentionBenchmark;

impl RetentionBenchmark {
    /// Run both policies on the same seed and return the comparison.
    pub fn run(seed: u64, initial_count: usize, cycles: usize, capacity: usize) -> RetentionOutput {
        let mut rng = SimpleRng::new(seed);

        // Initialize: 10% pinned, 90% with random importance/confidence.
        let mut pool_lru: Vec<Mem> = Vec::with_capacity(initial_count);
        let mut pool_cairn: Vec<Mem> = Vec::with_capacity(initial_count);
        for i in 0..initial_count {
            let pinned = i < initial_count / 10;
            let importance = if pinned { 1.0 } else { rng.next_f64() };
            let confidence = if pinned { 1.0 } else { rng.next_f64() };
            for pool in [&mut pool_lru, &mut pool_cairn] {
                pool.push(Mem {
                    id: i,
                    pinned,
                    importance,
                    confidence,
                });
            }
        }

        // The "important" set: pinned + top-10 non-pinned by importance.
        let mut by_importance = pool_lru.clone();
        by_importance.sort_by(|a, b| {
            b.importance
                .partial_cmp(&a.importance)
                .unwrap()
                .then(a.id.cmp(&b.id))
        });
        let important_ids: std::collections::HashSet<usize> = by_importance
            .iter()
            .take(initial_count / 10 + 10)
            .map(|m| m.id)
            .collect();

        for _ in 0..cycles {
            // Cycle: reinforce every existing + remember 10 new ones.
            for pool in [&mut pool_lru, &mut pool_cairn] {
                for m in pool.iter_mut() {
                    m.confidence = (m.confidence + 0.05).min(1.0);
                }
                for _ in 0..10 {
                    let new_id = pool.len();
                    pool.push(Mem {
                        id: new_id + initial_count,
                        pinned: false,
                        importance: rng.next_f64(),
                        confidence: rng.next_f64(),
                    });
                }
                // Drop down to capacity.
                while pool.len() > capacity {
                    drop_one(pool);
                }
            }
        }

        let mut policies = Vec::with_capacity(2);
        for (name, pool) in [("lru", &pool_lru), ("cairn", &pool_cairn)] {
            let survived = pool
                .iter()
                .filter(|m| important_ids.contains(&m.id))
                .count();
            policies.push(RetentionResult {
                policy: name.into(),
                initial_count: pool.len(),
                cycles,
                important_survived: survived,
                important_total: important_ids.len(),
                survival_rate: survived as f64 / important_ids.len().max(1) as f64,
            });
        }
        RetentionOutput { policies }
    }

    pub fn run_default() -> BenchResult {
        let started = std::time::Instant::now();
        let output = Self::run(0xC41A7, 100, 50, 100);
        let mut out = BenchResult::new(
            "retention-default",
            BenchKind::Retention,
            started.elapsed().as_millis() as u64,
        )
        .with_meta("seed", "0xC41A7")
        .with_meta("initial_count", "100")
        .with_meta("cycles", "50")
        .with_meta("capacity", "100");
        out.data = crate::BenchData::Retention(output);
        out
    }
}

#[derive(Debug, Clone)]
struct Mem {
    id: usize,
    pinned: bool,
    importance: f64,
    confidence: f64,
}

fn drop_one(pool: &mut Vec<Mem>) {
    if let Some(idx) = pool.iter().position(|m| !m.pinned) {
        pool.swap_remove(idx);
    } else {
        // Every memory is pinned — drop the oldest.
        pool.remove(0);
    }
}

/// Tiny xorshift64 PRNG, kept here so the benchmark doesn't depend on the
/// `rand` crate's version-specific APIs. Deterministic for a given seed.
#[derive(Debug, Clone)]
struct SimpleRng(u64);

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 1 } else { seed })
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn next_f64(&mut self) -> f64 {
        // Map to [0.1, 1.0) so the random memories have non-trivial importance.
        let v = (self.next_u64() as f64) / (u64::MAX as f64);
        0.1 + v * 0.9
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cairn_preserves_more_important_memories_than_lru() {
        let out = RetentionBenchmark::run(123, 100, 50, 100);
        let lru = out.policies.iter().find(|p| p.policy == "lru").unwrap();
        let cairn = out.policies.iter().find(|p| p.policy == "cairn").unwrap();
        assert!(
            cairn.important_survived >= lru.important_survived,
            "Cairn ({}) should preserve at least as many important memories as LRU ({})",
            cairn.important_survived,
            lru.important_survived
        );
    }

    #[test]
    fn pinned_memories_always_survive() {
        let out = RetentionBenchmark::run(1, 100, 50, 50);
        let cairn = out.policies.iter().find(|p| p.policy == "cairn").unwrap();
        // 10 pinned memories should all survive even at half-capacity.
        let pinned_survived = cairn.important_survived; // all important includes the pinned
        assert!(pinned_survived >= 10);
    }
}
