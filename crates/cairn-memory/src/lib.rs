//! The memory engine: persist what matters and surface it again across sessions.
//!
//! This is the thin-slice version: dedup on exact content, and recall ranked by keyword overlap +
//! recency + importance. Hybrid retrieval (BM25 + vectors + graph, RRF) and 4-tier consolidation
//! land in later phases; the API here is designed to stay stable as those arrive.

use cairn_core::{ContentHash, Memory, MemoryKind, NewMemory, Result};
use cairn_store::Store;
use chrono::Utc;
use serde::Serialize;
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
    /// Ranked with BM25 over the memory corpus (real IR relevance, not keyword overlap), blended
    /// with light recency and importance signals so ties resolve sensibly.
    pub fn recall(&self, query: &str, limit: usize) -> Result<Vec<ScoredMemory>> {
        let mems = self.store.all_memories()?;
        if mems.is_empty() {
            return Ok(Vec::new());
        }
        let now = Utc::now();

        let docs: Vec<Vec<String>> = mems
            .iter()
            .map(|m| tokenize(&format!("{} {}", m.content, m.concepts.join(" "))))
            .collect();
        let bm25 = Bm25::new(&docs);
        let q_terms = tokenize(query);

        let mut scored: Vec<ScoredMemory> = mems
            .into_iter()
            .enumerate()
            .map(|(i, m)| {
                let relevance = bm25.score(i, &q_terms);
                let age_days = ((now - m.created_at).num_seconds() as f32 / 86_400.0).max(0.0);
                let recency = 1.0 / (1.0 + age_days);
                let score = relevance + 0.3 * m.importance + 0.2 * recency;
                ScoredMemory { memory: m, score }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
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
    let recency = 1.0 / (1.0 + age_days);
    kind_weight + m.importance + recency * 0.5
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
    use cairn_core::{Config, MemoryKind};
    use cairn_store::Store;

    fn engine() -> (MemoryEngine, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::resolve(Some(dir.path().join("data"))).unwrap();
        let store = Arc::new(Store::open(&cfg).unwrap());
        (MemoryEngine::new(store), dir)
    }

    #[test]
    fn identical_content_dedups() {
        let (mem, _d) = engine();
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
        let (mem, _d) = engine();
        mem.remember(NewMemory::new("use sqlite plus a content-hash blob store"))
            .unwrap();
        mem.remember(NewMemory::new("the weather today is sunny"))
            .unwrap();
        let hits = mem.recall("sqlite blob storage", 10).unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].memory.content.contains("sqlite"));
    }

    #[test]
    fn wakeup_prioritizes_decisions() {
        let (mem, _d) = engine();
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
}
