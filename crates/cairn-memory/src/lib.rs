//! The memory engine: persist what matters and surface it again across sessions.
//!
//! Dedup on exact content; recall ranked by BM25 over the corpus, blended with Ebbinghaus
//! retention (memories decay unless reinforced) and importance. Consolidation moves memories
//! across the four tiers (working → episodic → semantic → procedural). Vector/graph hybrid
//! retrieval builds on this foundation.

use cairn_core::{ContentHash, Memory, MemoryKind, MemoryTier, NewMemory, Result};
use cairn_store::Store;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

/// A recall hit with its relevance score.
#[derive(Debug, Clone, Serialize)]
pub struct ScoredMemory {
    pub memory: Memory,
    pub score: f32,
}

pub struct MemoryEngine {
    store: Arc<Store>,
}

impl MemoryEngine {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    /// Persist a memory. If an identical one already exists, return it instead of duplicating.
    pub fn remember(&self, input: NewMemory) -> Result<Memory> {
        let memory = input.into_memory();
        let hash = ContentHash::of_str(&memory.content);
        if let Some(existing) = self.store.find_memory_by_content_hash(hash.as_str())? {
            return Ok(existing);
        }
        self.store.insert_memory(&memory)?;
        Ok(memory)
    }

    /// Recall the most relevant memories for a query.
    ///
    /// **Hybrid retrieval:** lexical relevance (BM25 over the corpus) and, when the backend has a
    /// vector index, semantic relevance (HNSW kNN) are fused with Reciprocal Rank Fusion — a
    /// scale-free combination of the two rankings. Importance and Ebbinghaus recency break ties.
    /// On a lexical-only backend (`semantic_recall` → `None`) this degrades to pure BM25.
    pub fn recall(&self, query: &str, limit: usize) -> Result<Vec<ScoredMemory>> {
        let mems = self.store.all_memories()?;
        if mems.is_empty() {
            return Ok(Vec::new());
        }
        let now = Utc::now();

        // Lexical ranking (BM25 over content + concepts).
        let docs: Vec<Vec<String>> = mems
            .iter()
            .map(|m| tokenize(&format!("{} {}", m.content, m.concepts.join(" "))))
            .collect();
        let bm25 = Bm25::new(&docs);
        let q_terms = tokenize(query);
        let bm25_scores: Vec<f32> = (0..mems.len()).map(|i| bm25.score(i, &q_terms)).collect();
        let bm25_rank = ranks_desc(&bm25_scores);

        // Semantic ranking (vector kNN) as id → rank, when the backend supports it.
        let sem_rank: HashMap<String, usize> = self
            .store
            .semantic_recall(query, limit.max(SEMANTIC_K))?
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(rank, m)| (m.id, rank))
            .collect();

        let mut scored: Vec<ScoredMemory> = mems
            .into_iter()
            .enumerate()
            .map(|(i, m)| {
                let mut score = rrf(bm25_rank[i]);
                if let Some(&r) = sem_rank.get(&m.id) {
                    score += rrf(r);
                }
                ScoredMemory { memory: m, score }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    tiebreak(&b.memory, now)
                        .partial_cmp(&tiebreak(&a.memory, now))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
        scored.truncate(limit);

        for s in &scored {
            let _ = self.store.touch_memory(&s.memory.id);
        }
        Ok(scored)
    }

    /// The session-start bootstrap: the highest-value memories to inject so the agent never
    /// starts cold. Prioritizes decisions/tasks/preferences, then importance and recency.
    pub fn wakeup(&self, limit: usize) -> Result<Vec<Memory>> {
        let now = Utc::now();
        let mut all = self.store.all_memories()?;
        all.sort_by(|a, b| {
            priority(a, now)
                .partial_cmp(&priority(b, now))
                .unwrap_or(std::cmp::Ordering::Equal)
                .reverse()
        });
        all.truncate(limit);
        Ok(all)
    }

    /// Fetch a memory by id.
    pub fn get(&self, id: &str) -> Result<Option<Memory>> {
        self.store.get_memory(id)
    }

    /// All memories of a given kind, newest first.
    pub fn by_kind(&self, kind: MemoryKind) -> Result<Vec<Memory>> {
        let mut all = self.store.all_memories()?;
        all.retain(|m| m.kind == kind);
        Ok(all)
    }

    /// Consolidate memory across the four tiers (working → episodic → semantic → procedural),
    /// the way human memory turns transient experience into durable knowledge. Returns how many
    /// memories were promoted. Idempotent: a memory only advances when it meets the next bar.
    pub fn consolidate(&self) -> Result<usize> {
        let mut promoted = 0;
        for mut m in self.store.all_memories()? {
            if let Some(tier) = next_tier(&m) {
                m.tier = tier;
                m.updated_at = Utc::now();
                if self.store.upsert_memory(&m)? {
                    promoted += 1;
                }
            }
        }
        Ok(promoted)
    }
}

fn priority(m: &Memory, now: chrono::DateTime<Utc>) -> f32 {
    let kind_weight = match m.kind {
        MemoryKind::Decision => 1.0,
        MemoryKind::Task => 0.9,
        MemoryKind::Preference => 0.8,
        MemoryKind::Gotcha => 0.7,
        MemoryKind::Fact => 0.5,
        MemoryKind::Note => 0.3,
    };
    let age_days = ((now - m.created_at).num_seconds() as f32 / 86_400.0).max(0.0);
    kind_weight + m.importance + retention(age_days, m.access_count, m.importance) * 0.5
}

/// Ebbinghaus-style retention in `[0, 1]`: how strongly a memory is held right now. Stability
/// grows with repeated access and importance, so reinforced/important memories decay slowly while
/// untouched ones fade. A fresh memory (age 0) returns ~1.0.
fn retention(age_days: f32, access_count: i64, importance: f32) -> f32 {
    let stability = 1.0 + 0.5 * access_count.max(0) as f32 + 2.0 * importance.clamp(0.0, 1.0);
    (-age_days.max(0.0) / (5.0 * stability)).exp()
}

/// How many semantic candidates to pull from the vector index when fusing (>= the recall limit).
const SEMANTIC_K: usize = 50;

/// Reciprocal-rank-fusion contribution of a 0-based rank (the standard `k = 60`).
fn rrf(rank: usize) -> f32 {
    1.0 / (60.0 + rank as f32)
}

/// Dense 0-based ranks (highest score = rank 0) for a score vector, by index.
fn ranks_desc(scores: &[f32]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&a, &b| {
        scores[b]
            .partial_cmp(&scores[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut rank = vec![0usize; scores.len()];
    for (r, &i) in order.iter().enumerate() {
        rank[i] = r;
    }
    rank
}

/// Importance + Ebbinghaus recency, used only to break fusion-score ties.
fn tiebreak(m: &Memory, now: DateTime<Utc>) -> f32 {
    let age_days = ((now - m.created_at).num_seconds() as f32 / 86_400.0).max(0.0);
    0.3 * m.importance + 0.4 * retention(age_days, m.access_count, m.importance)
}

/// The tier a memory should advance to on consolidation, or `None` if it stays put. Working
/// memories survive their session into episodic; reinforced episodic memories (accessed again)
/// become durable — facts/decisions/preferences become semantic knowledge, and gotchas (hard-won
/// "avoid X" lessons) become procedural.
fn next_tier(m: &Memory) -> Option<MemoryTier> {
    match m.tier {
        MemoryTier::Working => Some(MemoryTier::Episodic),
        MemoryTier::Episodic if m.access_count >= 2 => match m.kind {
            MemoryKind::Fact | MemoryKind::Decision | MemoryKind::Preference => {
                Some(MemoryTier::Semantic)
            }
            MemoryKind::Gotcha => Some(MemoryTier::Procedural),
            _ => None,
        },
        _ => None,
    }
}

/// Lowercase, alphanumeric tokenizer (tokens of length >= 2).
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_string())
        .collect()
}

/// A compact BM25 ranker over an in-memory corpus.
struct Bm25 {
    doc_len: Vec<f32>,
    avgdl: f32,
    df: std::collections::HashMap<String, usize>,
    tf: Vec<std::collections::HashMap<String, usize>>,
    n: usize,
}

impl Bm25 {
    const K1: f32 = 1.2;
    const B: f32 = 0.75;

    fn new(docs: &[Vec<String>]) -> Self {
        let n = docs.len();
        let mut df: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut tf = Vec::with_capacity(n);
        let mut doc_len = Vec::with_capacity(n);
        for doc in docs {
            let mut counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for tok in doc {
                *counts.entry(tok.clone()).or_insert(0) += 1;
            }
            for tok in counts.keys() {
                *df.entry(tok.clone()).or_insert(0) += 1;
            }
            doc_len.push(doc.len() as f32);
            tf.push(counts);
        }
        let avgdl = if n == 0 {
            0.0
        } else {
            doc_len.iter().sum::<f32>() / n as f32
        };
        Self {
            doc_len,
            avgdl,
            df,
            tf,
            n,
        }
    }

    fn idf(&self, term: &str) -> f32 {
        let df = *self.df.get(term).unwrap_or(&0) as f32;
        (1.0 + (self.n as f32 - df + 0.5) / (df + 0.5)).ln()
    }

    fn score(&self, doc: usize, q_terms: &[String]) -> f32 {
        if self.avgdl == 0.0 {
            return 0.0;
        }
        let dl = self.doc_len[doc];
        let mut s = 0.0;
        for term in q_terms {
            let tf = *self.tf[doc].get(term).unwrap_or(&0) as f32;
            if tf == 0.0 {
                continue;
            }
            let denom = tf + Self::K1 * (1.0 - Self::B + Self::B * dl / self.avgdl);
            s += self.idf(term) * (tf * (Self::K1 + 1.0)) / denom;
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::{Config, MemoryKind, MemoryTier};
    use cairn_store::Store;

    /// An engine backed by an isolated Helix store, or `None` when `CAIRN_HELIX_URL` is unset
    /// (offline runs skip these integration tests; CI sets the URL and runs them for real).
    fn engine() -> Option<MemoryEngine> {
        Some(MemoryEngine::new(Arc::new(Store::open_for_test()?)))
    }

    #[test]
    fn identical_content_dedups() {
        let Some(mem) = engine() else { return };
        let a = mem
            .remember(NewMemory::new("use sqlite for storage"))
            .unwrap();
        let b = mem
            .remember(NewMemory::new("use sqlite for storage"))
            .unwrap();
        assert_eq!(
            a.id, b.id,
            "identical content must dedup to the same memory"
        );
    }

    #[test]
    fn recall_ranks_relevant_first() {
        let Some(mem) = engine() else { return };
        mem.remember(NewMemory::new("use sqlite plus a content-hash blob store"))
            .unwrap();
        mem.remember(NewMemory::new("the weather today is sunny"))
            .unwrap();
        let hits = mem.recall("sqlite blob storage", 10).unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].memory.content.contains("sqlite"));
    }

    #[test]
    fn ranks_desc_assigns_dense_positions() {
        // Highest score gets rank 0; ranks are by index.
        assert_eq!(ranks_desc(&[0.1, 0.9, 0.5]), vec![2, 0, 1]);
        // RRF is strictly decreasing in rank, so a better rank always fuses higher.
        assert!(rrf(0) > rrf(1) && rrf(1) > rrf(5));
    }

    #[test]
    fn wakeup_prioritizes_decisions() {
        let Some(mem) = engine() else { return };
        mem.remember(NewMemory::new("a passing note")).unwrap();
        mem.remember(NewMemory {
            content: "decided to build the engine in Rust".into(),
            kind: Some(MemoryKind::Decision),
            importance: Some(0.9),
            ..Default::default()
        })
        .unwrap();
        let w = mem.wakeup(5).unwrap();
        assert_eq!(w[0].kind, MemoryKind::Decision);
    }

    #[test]
    fn retention_rewards_reinforcement_and_penalizes_age() {
        assert!(retention(0.0, 0, 0.5) > 0.99);
        let stale = retention(30.0, 0, 0.1);
        let reinforced = retention(30.0, 8, 0.9);
        assert!(
            reinforced > stale,
            "reinforced should retain more than stale"
        );
        assert!(stale < 0.5, "an old untouched memory should have faded");
    }

    #[test]
    fn consolidate_promotes_across_tiers() {
        let Some(mem) = engine() else { return };

        // A working note consolidates into episodic.
        let note = mem
            .remember(NewMemory::new("a transient working note"))
            .unwrap();
        assert_eq!(note.tier, MemoryTier::Working);
        assert_eq!(mem.consolidate().unwrap(), 1);
        assert_eq!(
            mem.get(&note.id).unwrap().unwrap().tier,
            MemoryTier::Episodic
        );

        // A reinforced fact (accessed twice) advances episodic -> semantic.
        let fact = mem
            .remember(NewMemory {
                content: "rust uses ownership for memory safety".into(),
                kind: Some(MemoryKind::Fact),
                ..Default::default()
            })
            .unwrap();
        mem.consolidate().unwrap(); // working -> episodic
        mem.recall("rust ownership memory", 10).unwrap();
        mem.recall("rust ownership memory", 10).unwrap();
        mem.consolidate().unwrap(); // episodic + accessed -> semantic
        assert_eq!(
            mem.get(&fact.id).unwrap().unwrap().tier,
            MemoryTier::Semantic
        );
    }
}
