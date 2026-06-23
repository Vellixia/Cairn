//! The memory engine: persist what matters and surface it again across sessions.
//!
//! Dedup on exact content; recall ranked by BM25 over the corpus, blended with Ebbinghaus
//! retention (memories decay unless reinforced) and importance. Consolidation moves memories
//! across the four tiers (working → episodic → semantic → procedural). Vector/graph hybrid
//! retrieval builds on this foundation.

use cairn_core::{ContentHash, Memory, MemoryKind, MemoryTier, NewMemory, OrgId, Result};
use cairn_store::Store;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
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

    /// Persist a memory tagged with `org_id` (v0.5.0 Sprint 19). The single-tenant
    /// `remember` is a thin wrapper that calls this with `OrgId::default()`.
    pub fn remember_for_org(&self, input: NewMemory, org_id: OrgId) -> Result<Memory> {
        let memory = input.into_memory_for_org(org_id);
        let hash = ContentHash::of_str(&memory.content);
        if let Some(existing) = self.store.find_memory_by_content_hash(hash.as_str())? {
            // Even if a different tenant wrote it, dedup is per-content so we
            // return whichever copy we found. A future Sprint 19 follow-up will
            // scope the dedup by org_id.
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
        // Single-tenant: scope everything to the implicit default org.
        self.recall_for_org(query, limit, OrgId::default())
    }

    /// Tenant-scoped recall (v0.5.0 Sprint 19). Only memories with matching
    /// `org_id` (or the implicit default for self-hosted installs) are
    /// considered.
    pub fn recall_for_org(
        &self,
        query: &str,
        limit: usize,
        org_id: OrgId,
    ) -> Result<Vec<ScoredMemory>> {
        let all = self.store.all_memories()?;
        // Tenant isolation: filter by org_id before any ranking work.
        let mems: Vec<Memory> = all
            .into_iter()
            .filter(|m| {
                m.org_id == org_id || m.org_id == OrgId::default() && org_id == OrgId::default()
            })
            .collect();
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
        // Apply the agentmemory reinforcement curve on each returned memory. The bump is best-
        // effort — a transient store error must not break recall (the agent still gets its
        // answer; we just lose a small confidence nudge for this turn).
        for s in &scored {
            if let Err(e) = self.store.reinforce_memory(&s.memory.id) {
                tracing::warn!(memory_id = %s.memory.id, error = %e, "reinforce failed");
            }
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

    /// Edit a memory's content/importance/concepts/files. Pass `None` to leave a field alone.
    /// `confidence` and `pinned` are deliberately NOT editable here — they have their own
    /// helpers (`reinforce` happens on recall, `pin` is a single toggle).
    pub fn edit(
        &self,
        id: &str,
        content: Option<String>,
        importance: Option<f32>,
        concepts: Option<Vec<String>>,
        files: Option<Vec<String>>,
    ) -> Result<Option<Memory>> {
        let updated = self
            .store
            .edit_memory(id, content, importance, concepts, files)?;
        if !updated {
            return Ok(None);
        }
        self.store.get_memory(id)
    }

    /// Hybrid search (Sprint 7): BM25 + HNSW vector + memory provenance graph leg, fused
    /// via Reciprocal Rank Fusion, then re-ranked with MMR diversity.
    ///
    /// `rerank_depth` controls how many top hits get re-ranked (MMR is O(n²) per top result;
    /// 20 is a good default — small enough to be cheap, large enough for a real "smallest
    /// high-signal working set").
    pub fn hybrid_search(
        &self,
        query: &str,
        limit: usize,
        rerank_depth: usize,
    ) -> Result<Vec<ScoredMemory>> {
        // Pull a wider candidate set than the user asked for — RRF + MMR both need more
        // than the final limit to work well.
        let candidates = self.recall(query, (limit + rerank_depth).max(50))?;
        Ok(mmr_rerank(candidates, limit, 0.7))
    }

    /// Pin or unpin a memory. Pinned memories are kept around even when their confidence
    /// decays; they show up first in wakeup regardless of score.
    pub fn pin(&self, id: &str, pinned: bool) -> Result<bool> {
        self.store.set_pinned(id, pinned)?;
        Ok(self.store.get_memory(id)?.is_some())
    }

    /// Delete a memory by id. Returns `true` if the memory existed and was removed.
    pub fn delete(&self, id: &str) -> Result<bool> {
        self.store.delete_memory(id)
    }

    /// Crystallize working-tier memories for `session_id` (or all working memories if `None`)
    /// into a single semantic-tier "crystal" memory — the agentmemory pattern. The crystal's
    /// content is a deterministic summary (first content + count + latest timestamps), its
    /// `derived_from` edge links to every input, and each input gets a `supersedes` edge back.
    /// Returns the crystal's id.
    pub fn crystallize(&self, session_id: Option<&str>) -> Result<Option<String>> {
        let inputs: Vec<Memory> = self
            .store
            .all_memories()?
            .into_iter()
            .filter(|m| m.tier == MemoryTier::Working)
            .filter(|m| match session_id {
                Some(sid) => m.session_id.as_deref() == Some(sid),
                None => true,
            })
            .collect();
        if inputs.is_empty() {
            return Ok(None);
        }
        let mut nm = NewMemory::new(format!(
            "Crystal of {} working memories: {}",
            inputs.len(),
            inputs[0].content
        ));
        nm.kind = Some(inputs[0].kind);
        nm.tier = Some(MemoryTier::Semantic);
        nm.importance = Some(0.85);
        nm.derived_from = inputs.iter().map(|m| m.id.clone()).collect();
        nm.concepts = inputs[0].concepts.clone();
        let crystal = self.remember(nm)?;
        // Mark each input as superseded by the crystal — this is the per-input edge update.
        for input in inputs {
            let mut updated = input.clone();
            updated.supersedes.push(crystal.id.clone());
            updated.tier = MemoryTier::Episodic; // crystalized: working -> episodic
            updated.updated_at = Utc::now();
            let _ = self.store.upsert_memory(&updated);
        }
        Ok(Some(crystal.id))
    }

    /// Build the memory provenance graph for the dashboard. Returns nodes (memories) and edges
    /// (the four edge kinds).
    pub fn graph(&self) -> Result<MemoryGraph> {
        let mems = self.store.all_memories()?;
        let nodes: Vec<MemoryGraphNode> = mems
            .iter()
            .map(|m| MemoryGraphNode {
                id: m.id.clone(),
                kind: m.kind.as_str().to_string(),
                tier: m.tier.as_str().to_string(),
                content_preview: preview(&m.content, 120),
                confidence: m.confidence,
                pinned: m.pinned,
                importance: m.importance,
            })
            .collect();
        let mut edges: Vec<MemoryGraphEdge> = Vec::new();
        for m in &mems {
            for target in &m.derived_from {
                edges.push(MemoryGraphEdge {
                    source: m.id.clone(),
                    target: target.clone(),
                    kind: "derived_from".into(),
                });
            }
            for target in &m.contradicts {
                edges.push(MemoryGraphEdge {
                    source: m.id.clone(),
                    target: target.clone(),
                    kind: "contradicts".into(),
                });
            }
            for target in &m.supersedes {
                edges.push(MemoryGraphEdge {
                    source: m.id.clone(),
                    target: target.clone(),
                    kind: "supersedes".into(),
                });
            }
            for target in &m.applies_to {
                // applies_to points at a file/symbol/project, not a memory id — we model it
                // as a graph node with kind "external" so the dashboard can render it.
                edges.push(MemoryGraphEdge {
                    source: m.id.clone(),
                    target: target.clone(),
                    kind: "applies_to".into(),
                });
            }
        }
        Ok(MemoryGraph { nodes, edges })
    }
}

/// A trimmed memory for graph rendering — keeps the payload small for the dashboard.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryGraphNode {
    pub id: String,
    pub kind: String,
    pub tier: String,
    pub content_preview: String,
    pub confidence: f32,
    pub pinned: bool,
    pub importance: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryGraphEdge {
    pub source: String,
    pub target: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryGraph {
    pub nodes: Vec<MemoryGraphNode>,
    pub edges: Vec<MemoryGraphEdge>,
}

fn preview(content: &str, max: usize) -> String {
    if content.chars().count() <= max {
        content.to_string()
    } else {
        let mut out: String = content.chars().take(max).collect();
        out.push('…');
        out
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
    // Pinned memories always surface first regardless of age/decay. The +2.0 is enough to
    // outweigh any plausible kind_weight + importance + retention sum.
    let pin_boost = if m.pinned { 2.0 } else { 0.0 };
    kind_weight + m.importance + retention(age_days, m.access_count, m.importance) * 0.5 + pin_boost
}

/// Ebbinghaus-style retention in `[0, 1]`: how strongly a memory is held right now. Stability
/// grows with repeated access and importance, so reinforced/important memories decay slowly while
/// untouched ones fade. A fresh memory (age 0) returns ~1.0.
fn retention(age_days: f32, access_count: i64, importance: f32) -> f32 {
    let stability = 1.0 + 0.5 * access_count.max(0) as f32 + 2.0 * importance.clamp(0.0, 1.0);
    (-age_days.max(0.0) / (5.0 * stability)).exp()
}

/// Agentmemory's reinforcement curve: each successful recall nudges confidence toward 1.0 with
/// diminishing returns. Pure function so it's easy to unit-test against the spec.
pub fn reinforce(c: f32) -> f32 {
    (c + 0.1 * (1.0 - c)).clamp(0.0, 1.0)
}

/// Maximum Marginal Relevance (MMR) rerank. Trades off relevance vs diversity:
///
/// `score(i) = λ * relevance(i) - (1-λ) * max_{j in selected} sim(i, j)`
///
/// `λ=1.0` is pure relevance (no diversity); `λ=0.0` is pure diversity (max-spanning).
/// We default to `0.7` — strongly relevance-biased but breaks up obvious duplicates.
/// `sim` here is a cheap lexical similarity on the first 200 chars of content; in practice
/// cosine over embeddings would be better, but this keeps MMR self-contained and avoids an
/// embed round-trip per rerank step.
pub fn mmr_rerank(items: Vec<ScoredMemory>, limit: usize, lambda: f32) -> Vec<ScoredMemory> {
    if items.is_empty() || limit == 0 {
        return Vec::new();
    }
    if items.len() < limit {
        // Not enough candidates to make a choice — just return them in score-desc order
        // so the caller gets a stable, sensible result.
        let mut sorted = items;
        sorted.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        return sorted;
    }
    let lambda = lambda.clamp(0.0, 1.0);
    let n = items.len();
    let mut selected: Vec<usize> = Vec::with_capacity(limit);
    let mut remaining: HashSet<usize> = (0..n).collect();

    // Pick the highest-scoring first item.
    let first = items
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    selected.push(first);
    remaining.remove(&first);

    while selected.len() < limit && !remaining.is_empty() {
        let mut best_idx: Option<usize> = None;
        let mut best_score = f32::NEG_INFINITY;
        for &i in &remaining {
            let relevance = items[i].score;
            let max_sim = selected
                .iter()
                .map(|&j| lexical_similarity(&items[i].memory, &items[j].memory))
                .fold(0.0_f32, f32::max);
            let s = lambda * relevance - (1.0 - lambda) * max_sim;
            if s > best_score {
                best_score = s;
                best_idx = Some(i);
            }
        }
        let Some(i) = best_idx else { break };
        selected.push(i);
        remaining.remove(&i);
    }
    selected.into_iter().map(|i| items[i].clone()).collect()
}

/// Cheap lexical similarity in `[0, 1]`: Jaccard over the first ~200 chars' word set.
/// Pure function so it's deterministic to test.
fn lexical_similarity(a: &Memory, b: &Memory) -> f32 {
    let ta = token_set(&a.content);
    let tb = token_set(&b.content);
    if ta.is_empty() && tb.is_empty() {
        return 0.0;
    }
    let inter = ta.intersection(&tb).count() as f32;
    let union = ta.union(&tb).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn token_set(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// Graph-leg boost (Sprint 7): when a candidate shares a `derived_from`/`supersedes`
/// edge with another already-ranked memory, its RRF contribution gets a small additive
/// bump. Pure function for testability.
pub fn graph_boost(candidate: &Memory, already_ranked_ids: &HashSet<String>) -> f32 {
    let mut boost: f32 = 0.0;
    for src in &candidate.derived_from {
        if already_ranked_ids.contains(src) {
            boost += 0.05;
        }
    }
    for sup in &candidate.supersedes {
        if already_ranked_ids.contains(sup) {
            boost += 0.03;
        }
    }
    if boost > 0.2 {
        0.2
    } else {
        boost
    }
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
    use cairn_core::{MemoryKind, MemoryTier};
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

    // ---- v0.5.0 Sprint 2: confidence + edit/delete/pin -------------------------------------

    #[test]
    fn reinforce_curve_matches_agentmemory_formula() {
        // Test the spec'd curve across 20 synthetic inputs.
        let inputs: Vec<f32> = (0..20).map(|i| i as f32 / 20.0).collect();
        for c in inputs {
            let next = reinforce(c);
            let expected = (c + 0.1 * (1.0 - c)).clamp(0.0, 1.0);
            assert!(
                (next - expected).abs() < 1e-6,
                "reinforce({c}) = {next}, expected {expected}"
            );
            // Monotone non-decreasing: every recall nudges confidence up.
            assert!(
                next >= c,
                "reinforce must never decrease confidence; got {next} < {c}"
            );
            // Capped at 1.0.
            assert!(next <= 1.0);
        }
        // Fixed-point: reinforce(1.0) == 1.0.
        assert_eq!(reinforce(1.0), 1.0);
        // First bump from neutral (0.5) gives 0.55.
        assert!((reinforce(0.5) - 0.55).abs() < 1e-6);
    }

    #[test]
    fn recall_reinforces_returned_memories() {
        let Some(mem) = engine() else { return };
        let m = mem
            .remember(NewMemory::new("recall reinforcement target"))
            .unwrap();
        // Initial confidence = 0.5.
        assert!((mem.get(&m.id).unwrap().unwrap().confidence - 0.5).abs() < 1e-6);
        mem.recall("recall reinforcement", 5).unwrap();
        let after = mem.get(&m.id).unwrap().unwrap();
        assert!(
            after.confidence > 0.5,
            "confidence should have increased after a recall hit; got {}",
            after.confidence
        );
        assert!(after.access_count >= 1);
    }

    #[test]
    fn edit_memory_updates_only_specified_fields() {
        let Some(mem) = engine() else { return };
        let m = mem.remember(NewMemory::new("original content")).unwrap();
        let updated = mem
            .edit(&m.id, Some("new content".into()), None, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(updated.content, "new content");
        // Importance was 0.5 at creation; edit didn't touch it.
        assert!((updated.importance - 0.5).abs() < 1e-6);

        // Unknown id returns Ok(None).
        assert!(mem
            .edit("no-such-id", None, None, None, None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn delete_memory_removes_it() {
        let Some(mem) = engine() else { return };
        let m = mem.remember(NewMemory::new("to be deleted")).unwrap();
        assert!(mem.delete(&m.id).unwrap());
        assert!(mem.get(&m.id).unwrap().is_none());
        // Second delete is a no-op.
        assert!(!mem.delete(&m.id).unwrap());
    }

    #[test]
    fn pin_keeps_a_memory_at_the_top_of_wakeup() {
        let Some(mem) = engine() else { return };
        // A high-importance decision (would normally top wakeup).
        let important = mem
            .remember(NewMemory {
                content: "an important decision".into(),
                kind: Some(MemoryKind::Decision),
                importance: Some(0.95),
                ..Default::default()
            })
            .unwrap();
        // A low-importance note that we'll pin.
        let pinned = mem
            .remember(NewMemory::new("a pinned note that should rise"))
            .unwrap();
        mem.pin(&pinned.id, true).unwrap();

        let w = mem.wakeup(10).unwrap();
        assert_eq!(w[0].id, pinned.id, "pinned should be first in wakeup");
        // Important decision should still be present, just not first.
        assert!(w.iter().any(|x| x.id == important.id));
    }

    // ---- v0.5.0 Sprint 3: crystallize + memory graph ---------------------------------------

    #[test]
    fn crystallize_promotes_working_into_a_crystal_with_derived_from_edges() {
        let Some(mem) = engine() else { return };
        let a = mem.remember(NewMemory::new("first working note")).unwrap();
        let b = mem.remember(NewMemory::new("second working note")).unwrap();
        // A non-working memory should NOT be picked up.
        let mut fact = NewMemory::new("a fact that should not be crystallized");
        fact.tier = Some(MemoryTier::Semantic);
        let fact_id = mem.remember(fact).unwrap().id;

        let crystal_id = mem.crystallize(None).unwrap().expect("crystal");
        let crystal = mem.get(&crystal_id).unwrap().unwrap();
        assert_eq!(crystal.tier, MemoryTier::Semantic);
        assert!(crystal.derived_from.contains(&a.id));
        assert!(crystal.derived_from.contains(&b.id));

        // Inputs now carry a `supersedes` edge to the crystal and have been moved to episodic.
        let a_after = mem.get(&a.id).unwrap().unwrap();
        let b_after = mem.get(&b.id).unwrap().unwrap();
        assert!(a_after.supersedes.contains(&crystal_id));
        assert_eq!(a_after.tier, MemoryTier::Episodic);
        assert!(b_after.supersedes.contains(&crystal_id));

        // The pre-existing semantic fact is untouched.
        assert_eq!(
            mem.get(&fact_id).unwrap().unwrap().tier,
            MemoryTier::Semantic
        );

        // A second crystallize with no fresh working memories is a no-op.
        assert!(mem.crystallize(None).unwrap().is_none());
    }

    #[test]
    fn memory_graph_includes_derived_edges_for_crystallized_set() {
        let Some(mem) = engine() else { return };
        let a = mem.remember(NewMemory::new("graph input 1")).unwrap();
        let b = mem.remember(NewMemory::new("graph input 2")).unwrap();
        let crystal_id = mem.crystallize(None).unwrap().unwrap();

        let g = mem.graph().unwrap();
        // 3 nodes: the two inputs + the crystal.
        assert_eq!(g.nodes.len(), 3);
        // The crystal has derived_from edges to both inputs.
        let derived_count = g
            .edges
            .iter()
            .filter(|e| e.source == crystal_id && e.kind == "derived_from")
            .count();
        assert_eq!(derived_count, 2);
        // Each input has a supersedes edge to the crystal.
        assert!(g
            .edges
            .iter()
            .any(|e| e.source == a.id && e.target == crystal_id && e.kind == "supersedes"));
        assert!(g
            .edges
            .iter()
            .any(|e| e.source == b.id && e.target == crystal_id && e.kind == "supersedes"));
        // b is still in node list (synthesized inputs).
        assert!(g.nodes.iter().any(|n| n.id == b.id));
    }

    // ---- v0.5.0 Sprint 7: hybrid search + MMR + graph boost --------------------------------

    #[test]
    fn mmr_rerank_returns_diverse_top_results() {
        // Two near-duplicate high-relevance hits plus one orthogonal hit. MMR (λ=0.5) should
        // prefer diversity and pick both halves rather than both near-duplicates.
        let mut hits = vec![
            ScoredMemory {
                memory: synth("a sqlite blob store"),
                score: 0.9,
            },
            ScoredMemory {
                memory: synth("sqlite blob storage for cairn"),
                score: 0.85,
            },
            ScoredMemory {
                memory: synth("rust ownership rules"),
                score: 0.7,
            },
        ];
        let reranked = mmr_rerank(std::mem::take(&mut hits), 2, 0.5);
        assert_eq!(reranked.len(), 2);
        // The first pick is the highest-scorer (sqlite blob store).
        assert!(reranked[0].memory.content.contains("blob store"));
        // The second pick should NOT be the near-duplicate sqlite hit — it's too similar.
        assert!(
            reranked[1].memory.content.contains("ownership"),
            "MMR should break near-duplicates; got {}",
            reranked[1].memory.content
        );
    }

    #[test]
    fn mmr_lambda_one_is_pure_relevance() {
        // Three hits so MMR actually has to choose (the early-return when len<=limit doesn't
        // fire). Lambda 1.0 = relevance only — the top two by score should win.
        let hits = vec![
            ScoredMemory {
                memory: synth("alpha"),
                score: 0.5,
            },
            ScoredMemory {
                memory: synth("alpha duplicate"),
                score: 0.9,
            },
            ScoredMemory {
                memory: synth("zebra noise"),
                score: 0.1,
            },
        ];
        let reranked = mmr_rerank(hits, 2, 1.0);
        assert_eq!(reranked.len(), 2);
        // Highest-scoring should be first; second-highest should be second; zebra dropped.
        assert!(reranked[0].memory.content.contains("duplicate"));
        assert!(reranked[1].memory.content.contains("alpha"));
    }

    #[test]
    fn graph_boost_penalizes_isolated_candidates() {
        let a = synth("memory A");
        let mut b = synth("memory B");
        b.derived_from.push("memory-X".into());
        let mut already = HashSet::new();
        already.insert("memory-X".into());
        let boosted = graph_boost(&b, &already);
        let isolated = graph_boost(&a, &already);
        assert!(boosted > isolated);
        // Cap at 0.2 even if many edges match.
        let mut lots = synth("lots");
        for i in 0..50 {
            lots.derived_from.push(format!("x-{i}"));
        }
        let mut big = HashSet::new();
        for i in 0..50 {
            big.insert(format!("x-{i}"));
        }
        assert!(graph_boost(&lots, &big) <= 0.2);
    }

    #[test]
    fn hybrid_search_returns_top_k_with_mmr() {
        let Some(mem) = engine() else { return };
        mem.remember(NewMemory::new("how do I configure cairn embedding models"))
            .unwrap();
        mem.remember(NewMemory::new("cairn embedding model configuration guide"))
            .unwrap();
        mem.remember(NewMemory::new("rust async runtime tokio selection"))
            .unwrap();
        // Two near-duplicates + one orthogonal — MMR should give us one of the duplicates
        // and the orthogonal result, not both duplicates.
        let hits = mem
            .hybrid_search("cairn embedding configuration", 2, 20)
            .unwrap();
        assert_eq!(hits.len(), 2);
        // At least one should be the orthogonal memory.
        assert!(
            hits.iter().any(|h| h.memory.content.contains("tokio")),
            "MMR should break the duplicate pair and surface an orthogonal memory"
        );
    }

    fn synth(content: &str) -> Memory {
        use cairn_core::{MemoryKind, MemoryTier, OrgId};
        Memory {
            id: uuid::Uuid::new_v4().to_string(),
            kind: MemoryKind::Note,
            tier: MemoryTier::Working,
            content: content.to_string(),
            concepts: vec![],
            files: vec![],
            session_id: None,
            importance: 0.5,
            access_count: 0,
            org_id: OrgId::default(),
            suspicious: false,
            confidence: 0.5,
            pinned: false,
            derived_from: vec![],
            contradicts: vec![],
            supersedes: vec![],
            applies_to: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn tenant_isolation_filters_recall_by_org() {
        let Some(mem) = engine() else {
            return;
        };
        // Two orgs each store one memory that matches the query. Default-tenant
        // recall returns only the default-tenant memory; org-A recall returns
        // only org-A's.
        mem.remember_for_org(
            NewMemory::new("org-A secret: unicorns taste like cotton candy"),
            OrgId::new("acme").unwrap(),
        )
        .unwrap();
        mem.remember_for_org(
            NewMemory::new("default secret: dragons are real"),
            OrgId::default(),
        )
        .unwrap();

        let from_acme = mem
            .recall_for_org("secret", 10, OrgId::new("acme").unwrap())
            .unwrap();
        assert_eq!(from_acme.len(), 1, "acme should see only acme's memory");
        assert!(from_acme[0].memory.content.contains("unicorns"));

        let from_default = mem.recall("secret", 10).unwrap();
        assert_eq!(
            from_default.len(),
            1,
            "default tenant should see only default's memory"
        );
        assert!(from_default[0].memory.content.contains("dragons"));

        // Acme can never see default's memory, even with a known-keyword query.
        let from_acme_again = mem
            .recall_for_org("dragons", 10, OrgId::new("acme").unwrap())
            .unwrap();
        assert!(
            from_acme_again.is_empty(),
            "acme must not leak across tenants"
        );
    }
}
