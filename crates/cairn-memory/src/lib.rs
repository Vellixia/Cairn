//! The memory engine: persist what matters and surface it again across sessions.
//!
//! Dedup on exact content; recall ranked by BM25 over the corpus, blended with Ebbinghaus
//! retention (memories decay unless reinforced) and importance. Consolidation moves memories
//! across the four tiers (working -> episodic -> semantic -> procedural). Vector/graph hybrid
//! retrieval builds on this foundation.

use cairn_core::{ContentHash, Memory, MemoryKind, MemoryTier, NewMemory, OrgId, Result};
use cairn_store::Store;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub mod llm_consolidator;
pub use llm_consolidator::{apply_decay, Insight, LlmConsolidator, ProceduralStep, SemanticFact};
pub mod analysis;
pub use analysis::{generate_architecture_report, ArchitectureReport, BridgeEntry, GodNodeEntry};
pub mod followup_tracker;
pub use followup_tracker::FollowupTracker;
pub mod gotcha_tracker;
pub use gotcha_tracker::{FailureCluster, FailureEvent, GotchaTracker};
pub mod query_expander;
pub use query_expander::{ExpandedQuery, Expansion, QueryExpander};
pub mod rerank;
pub use cairn_rerank::{from_config as rerank_from_config, NullReranker};
pub use rerank::{RerankConfig, RerankError, RerankOutcome, Reranker, RerankerRef};

/// A recall hit with its relevance score.
#[derive(Debug, Clone, Serialize)]
pub struct ScoredMemory {
    pub memory: Memory,
    pub score: f32,
}

pub struct MemoryEngine {
    store: Arc<Store>,
    followup_tracker: std::sync::Mutex<FollowupTracker>,
    gotcha_tracker: std::sync::Mutex<GotchaTracker>,
    /// Optional cross-encoder reranker. When `None` (the default), `hybrid_search` and
    /// `hybrid_search_with_rerank` produce identical results.
    reranker: Option<Arc<dyn Reranker>>,
}

impl MemoryEngine {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            followup_tracker: std::sync::Mutex::new(FollowupTracker::new()),
            gotcha_tracker: std::sync::Mutex::new(GotchaTracker::new()),
            reranker: None,
        }
    }

    /// Builder method: install a cross-encoder reranker. Subsequent calls to
    /// `hybrid_search_with_rerank` will run MMR then re-score the post-MMR top-K with
    /// the provided reranker, blending the scores per `RerankConfig::blend_weight`.
    pub fn with_reranker(mut self, reranker: Arc<dyn Reranker>) -> Self {
        self.reranker = Some(reranker);
        self
    }

    /// Inspect the installed reranker (for diagnostics / metrics).
    pub fn has_reranker(&self) -> bool {
        self.reranker.is_some()
    }

    /// Access the followup tracker (for metrics / dashboard).
    pub fn followup_tracker(&self) -> &std::sync::Mutex<FollowupTracker> {
        &self.followup_tracker
    }

    /// Access the gotcha tracker (for metrics / dashboard).
    pub fn gotcha_tracker(&self) -> &std::sync::Mutex<GotchaTracker> {
        &self.gotcha_tracker
    }

    /// Record a failure and, if it crosses the cluster threshold, auto-promote to a
    /// `MemoryKind::Gotcha` memory. Returns the created gotcha memory (if any) so the
    /// caller can surface it (e.g. in a webhook or API response).
    pub fn record_failure(&self, event: FailureEvent) -> Result<Option<Memory>> {
        let cluster = {
            let mut tracker = self
                .gotcha_tracker
                .lock()
                .map_err(|e| cairn_core::Error::Other(format!("gotcha tracker poisoned: {e}")))?;
            tracker.record(event)
        };
        let Some(cluster) = cluster else {
            return Ok(None);
        };

        // Promote: write a gotcha memory summarizing the cluster.
        let session_count = cluster.session_ids.len();
        let refs_concat = cluster
            .events
            .iter()
            .flat_map(|e| e.refs.iter().cloned())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", ");
        let content = if session_count >= 2 {
            format!(
                "Gotcha: '{}' (seen {} times across {} sessions). Watch for: {}",
                cluster.topic(),
                cluster.size(),
                session_count,
                if refs_concat.is_empty() {
                    cluster.events[0].context.clone()
                } else {
                    format!("refs=[{}]", refs_concat)
                }
            )
        } else {
            format!(
                "Gotcha: '{}' (seen {} times). Watch for: {}",
                cluster.topic(),
                cluster.size(),
                cluster.events[0].context
            )
        };

        let mut input = NewMemory::new(content);
        input.kind = Some(cairn_core::MemoryKind::Gotcha);
        input.tier = Some(cairn_core::MemoryTier::Working);
        input.importance = Some(0.8);
        input.concepts = cluster
            .topic()
            .split_whitespace()
            .map(|s| s.to_string())
            .filter(|s| s.len() > 2)
            .collect();
        if !refs_concat.is_empty() {
            input.applies_to = refs_concat
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        Ok(Some(self.remember(input)?))
    }

    /// Top-K gotcha clusters by size. Useful for proactive recall at session start.
    pub fn top_gotcha_clusters(&self, n: usize) -> Result<Vec<FailureCluster>> {
        let tracker = self
            .gotcha_tracker
            .lock()
            .map_err(|e| cairn_core::Error::Other(format!("gotcha tracker poisoned: {e}")))?;
        Ok(tracker.top_clusters(n))
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
    /// vector index, semantic relevance (HNSW kNN) are fused with Reciprocal Rank Fusion - a
    /// scale-free combination of the two rankings. Importance and Ebbinghaus recency break ties.
    /// On a lexical-only backend (`semantic_recall` -> `None`) this degrades to pure BM25.
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

        // Semantic ranking (vector kNN) as id -> rank, when the backend supports it.
        let sem_rank: HashMap<String, usize> = self
            .store
            .semantic_recall(query, limit.max(SEMANTIC_K))?
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(rank, m)| (m.id, rank))
            .collect();

        // Graph stream: extract entities, find graph-proximate memories.
        let entities = extract_entities(query);
        let graph_boosted: HashMap<String, f64> = if entities.is_empty() {
            HashMap::new()
        } else {
            let g = self.graph()?;
            let mut gmap = HashMap::new();
            for node in &g.nodes {
                if entities.iter().any(|e| {
                    node.content_preview
                        .to_lowercase()
                        .contains(&e.to_lowercase())
                }) {
                    gmap.insert(node.id.clone(), 1.0);
                }
            }
            // Neighbors at depth 1
            for edge in &g.edges {
                if let Some(&score) = gmap.get(&edge.source) {
                    if !gmap.contains_key(&edge.target) {
                        gmap.insert(edge.target.clone(), score * 0.5);
                    }
                }
                if let Some(&score) = gmap.get(&edge.target) {
                    if !gmap.contains_key(&edge.source) {
                        gmap.insert(edge.source.clone(), score * 0.5);
                    }
                }
            }
            gmap
        };

        // Pre-compute RRF components for each memory.
        let n = mems.len();
        let mut bm25_rrf_scores = vec![0.0_f32; n];
        let mut vec_rrf_scores = vec![0.0_f32; n];
        let mut graph_rrf_scores = vec![0.0_f32; n];

        for i in 0..n {
            bm25_rrf_scores[i] = rrf(bm25_rank[i]);
            if let Some(&r) = sem_rank.get(&mems[i].id) {
                vec_rrf_scores[i] = rrf(r);
            }
            if let Some(&graph_score) = graph_boosted.get(&mems[i].id) {
                if graph_score > 0.0 {
                    let graph_rank = graph_boosted.len()
                        - graph_boosted.values().filter(|&&v| v > graph_score).count();
                    graph_rrf_scores[i] = rrf(graph_rank);
                }
            }
        }

        // Dynamic renormalization: scale weights by how many streams are active.
        let bm25_weight = 0.4_f64;
        let vec_weight = 0.6_f64;
        let graph_weight = 0.3_f64;
        let effective_bm25 = if bm25_scores.iter().any(|&s| s > 0.0_f32) {
            bm25_weight
        } else {
            0.0
        };
        let effective_vec = if !sem_rank.is_empty() {
            vec_weight
        } else {
            0.0
        };
        let effective_graph = if !graph_boosted.is_empty() {
            graph_weight
        } else {
            0.0
        };
        let total = effective_bm25 + effective_vec + effective_graph;
        let (norm_bm25, norm_vec, norm_graph) = if total > 0.0 {
            (
                effective_bm25 / total,
                effective_vec / total,
                effective_graph / total,
            )
        } else {
            let denom = bm25_weight + vec_weight + graph_weight;
            (
                bm25_weight / denom,
                vec_weight / denom,
                graph_weight / denom,
            )
        };

        let mut scored: Vec<ScoredMemory> = mems
            .into_iter()
            .enumerate()
            .map(|(i, m)| {
                let score = (norm_bm25 as f32) * bm25_rrf_scores[i]
                    + (norm_vec as f32) * vec_rrf_scores[i]
                    + (norm_graph as f32) * graph_rrf_scores[i];
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

        // Session diversification: cap at 3 per session, fill from remaining.
        let diversified = diversify_by_session(scored, limit, 3);

        for s in &diversified {
            let _ = self.store.touch_memory(&s.memory.id);
        }
        // Apply the agentmemory reinforcement curve on each returned memory. The bump is best-
        // effort - a transient store error must not break recall (the agent still gets its
        // answer; we just lose a small confidence nudge for this turn).
        for s in &diversified {
            if let Err(e) = self.store.reinforce_memory(&s.memory.id) {
                tracing::warn!(memory_id = %s.memory.id, error = %e, "reinforce failed");
            }
        }

        // P1.6: record this recall with the followup tracker so a disjoint re-query
        // in the window is counted as a followup. Best-effort: a poisoned mutex must
        // not break recall.
        {
            let ids: Vec<String> = diversified.iter().map(|s| s.memory.id.clone()).collect();
            if let Ok(mut tracker) = self.followup_tracker.lock() {
                tracker.record(query, &ids);
            }
        }

        Ok(diversified)
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

    /// P2.6: activity heatmap - returns a `YYYY-MM-DD -> count` map for the last
    /// `days` days. Powers `/api/memory/heatmap`. Backed by `analysis::activity_heatmap`
    /// which filters by `created_at` cutoff.
    pub fn activity_heatmap(&self, days: usize) -> Result<std::collections::HashMap<String, u32>> {
        let all = self.store.all_memories()?;
        Ok(crate::analysis::activity_heatmap(&all, days))
    }

    /// Consolidate memory across the four tiers (working -> episodic -> semantic -> procedural),
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
    /// `confidence` and `pinned` are deliberately NOT editable here - they have their own
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
    /// 20 is a good default - small enough to be cheap, large enough for a real "smallest
    /// high-signal working set").
    pub fn hybrid_search(
        &self,
        query: &str,
        limit: usize,
        rerank_depth: usize,
    ) -> Result<Vec<ScoredMemory>> {
        // Pull a wider candidate set than the user asked for - RRF + MMR both need more
        // than the final limit to work well.
        let candidates = self.recall(query, (limit + rerank_depth).max(50))?;
        Ok(mmr_rerank(candidates, limit, 0.7))
    }

    /// Hybrid search with cross-encoder reranking. Falls back to `hybrid_search` when no
    /// reranker is installed (the default).
    ///
    /// Pipeline: `recall -> RRF -> truncate -> diversify_by_session -> mmr -> rerank(top_k)
    /// -> min-max normalize -> alpha-blend with hybrid score`. The rerank cost is paid only
    /// for the post-MMR top-K candidates, so total inference is bounded.
    pub fn hybrid_search_with_rerank(
        &self,
        query: &str,
        limit: usize,
        rerank_depth: usize,
        reranker: &dyn Reranker,
        blend_weight: f32,
    ) -> Result<Vec<ScoredMemory>> {
        // 1. Same wide retrieval + MMR as the no-rerank path.
        let mmr = self.hybrid_search(query, limit, rerank_depth)?;
        if mmr.is_empty() {
            return Ok(mmr);
        }

        // 2. Rerank the post-MMR top-K (capped at `reranker` budget).
        let k = mmr.len().min(64); // hard cap: 64 forward passes
        let docs: Vec<String> = mmr[..k].iter().map(|h| h.memory.content.clone()).collect();
        let doc_refs: Vec<&str> = docs.iter().map(|s| s.as_str()).collect();
        let outcomes = match reranker.rerank(query, &doc_refs) {
            Ok(o) => o,
            Err(e) => {
                // Fail-soft: keep the MMR ordering if the reranker errors.
                tracing::warn!(error = %e, "reranker failed; returning MMR-only result");
                return Ok(mmr);
            }
        };

        // 3. Min-max normalize the cross-encoder scores so they live in [0, 1] and
        // can be blended with the hybrid score (which is already in [0, 1]).
        let (min, max) = outcomes
            .iter()
            .map(|o| o.score)
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), s| {
                (lo.min(s), hi.max(s))
            });
        let span = (max - min).max(f32::EPSILON);
        let norm = |s: f32| (s - min) / span;
        let alpha = blend_weight.clamp(0.0, 1.0);

        // 4. Apply the blend: out_index = original_index, score = alpha * cross + (1 - alpha) * hybrid.
        //    Re-sort the post-MMR slice by the blended score.
        let mut scored: Vec<(usize, f32)> = outcomes
            .iter()
            .map(|o| {
                let hybrid_score = mmr[o.original_index].score;
                let cross_norm = norm(o.score);
                let final_score = alpha * cross_norm + (1.0 - alpha) * hybrid_score;
                (o.original_index, final_score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // 5. Build the new ordering: reranked top-K first, then the rest of MMR.
        let mut new_order: Vec<usize> = scored.into_iter().map(|(i, _)| i).collect();
        new_order.extend(k..mmr.len());

        // 6. Re-apply the blended scores to the final ordering.
        let mut result: Vec<ScoredMemory> = Vec::with_capacity(mmr.len());
        for (new_idx, &orig_idx) in new_order.iter().enumerate() {
            let mut h = mmr[orig_idx].clone();
            // Only re-score the reranked slice - the rest keep their MMR score.
            if new_idx < k {
                h.score =
                    alpha * norm(outcomes[orig_idx].score) + (1.0 - alpha) * mmr[orig_idx].score;
            }
            result.push(h);
        }
        // Trim to the requested limit.
        result.truncate(limit);
        Ok(result)
    }

    /// Search the engine with LLM-driven query expansion. For each reformulation we run
    /// the full `recall`; results are merged by max `score` per memory id (per the spec's
    /// "merge by max combinedScore"). Final MMR rerank keeps the result set diverse.
    ///
    /// Falls back to a plain `hybrid_search` when:
    /// - the expander is disabled (short-circuit to single-query `ExpandedQuery`)
    /// - the expansion yields only the original query
    pub fn expanded_search(
        &self,
        query: &str,
        limit: usize,
        rerank_depth: usize,
        expander: &QueryExpander,
    ) -> Result<Vec<ScoredMemory>> {
        let expanded = expander.expand(query)?;
        if !expanded.is_expanded() {
            // Disabled or no reformulations produced - single-query path.
            return self.hybrid_search(query, limit, rerank_depth);
        }
        // Pull a wider candidate set per reformulation so MMR has headroom across the
        // merged pool.
        let per_query_k = (limit + rerank_depth).max(50);
        let mut by_id: std::collections::HashMap<String, ScoredMemory> =
            std::collections::HashMap::new();
        for q in &expanded.queries {
            for hit in self.recall(q, per_query_k)? {
                by_id
                    .entry(hit.memory.id.clone())
                    .and_modify(|existing| {
                        if hit.score > existing.score {
                            existing.score = hit.score;
                        }
                    })
                    .or_insert(hit);
            }
        }
        let merged: Vec<ScoredMemory> = by_id.into_values().collect();
        Ok(mmr_rerank(merged, limit, 0.7))
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
    /// into a single semantic-tier "crystal" memory - the agentmemory pattern. The crystal's
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
        // Mark each input as superseded by the crystal - this is the per-input edge update.
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
                // applies_to points at a file/symbol/project, not a memory id - we model it
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

/// A trimmed memory for graph rendering - keeps the payload small for the dashboard.
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
        out.push_str("...");
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
/// `score(i) = lambda * relevance(i) - (1-lambda) * max_{j in selected} sim(i, j)`
///
/// `lambda=1.0` is pure relevance (no diversity); `lambda=0.0` is pure diversity (max-spanning).
/// We default to `0.7` - strongly relevance-biased but breaks up obvious duplicates.
/// `sim` here is a cheap lexical similarity on the first 200 chars of content; in practice
/// cosine over embeddings would be better, but this keeps MMR self-contained and avoids an
/// embed round-trip per rerank step.
pub fn mmr_rerank(items: Vec<ScoredMemory>, limit: usize, lambda: f32) -> Vec<ScoredMemory> {
    if items.is_empty() || limit == 0 {
        return Vec::new();
    }
    if items.len() < limit {
        // Not enough candidates to make a choice - just return them in score-desc order
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

/// Extract candidate entities from a query for the graph leg. Returns both quoted
/// strings (stripped of quotes) and capitalized words of length >= 3. Pure function.
pub fn extract_entities(query: &str) -> Vec<String> {
    let mut entities = Vec::new();
    let mut quoted = String::new();
    let mut in_quote: Option<char> = None;
    for c in query.chars() {
        match in_quote {
            Some(q) if c == q => {
                let trimmed = quoted.trim().to_string();
                if !trimmed.is_empty() {
                    entities.push(trimmed);
                }
                quoted.clear();
                in_quote = None;
            }
            Some(_) => quoted.push(c),
            None if c == '"' || c == '\'' => in_quote = Some(c),
            None => {}
        }
    }
    // Any leftover quoted chunk (unterminated) - still emit it
    let trimmed = quoted.trim().to_string();
    if !trimmed.is_empty() && in_quote.is_some() {
        entities.push(trimmed);
    }
    // Capitalized words of length >= 3 (skip the very first word since capitalizing at
    // sentence start is meaningless for entity detection).
    let words: Vec<&str> = query.split_whitespace().collect();
    for (idx, w) in words.iter().enumerate() {
        let cleaned: String = w.chars().filter(|c| c.is_alphanumeric()).collect();
        if cleaned.len() < 3 {
            continue;
        }
        let mut chars = cleaned.chars();
        let first = chars.next().unwrap();
        if first.is_uppercase() && idx > 0 {
            entities.push(cleaned);
        }
    }
    // Dedup while preserving order
    let mut seen = HashSet::new();
    entities.retain(|e| seen.insert(e.clone()));
    entities
}

/// Compute graph-proximity scores for the graph leg. Extracts entities from the query,
/// finds graph nodes whose content_preview mentions them (start nodes), and propagates
/// a 0.5x score to immediate neighbors (depth 1). Pure function on the graph + entities.
pub fn graph_proximity_scores(graph: &MemoryGraph, entities: &[String]) -> HashMap<String, f64> {
    if entities.is_empty() {
        return HashMap::new();
    }
    let mut gmap: HashMap<String, f64> = HashMap::new();
    for node in &graph.nodes {
        let preview_lc = node.content_preview.to_lowercase();
        if entities
            .iter()
            .any(|e| preview_lc.contains(&e.to_lowercase()))
        {
            gmap.insert(node.id.clone(), 1.0);
        }
    }
    // Neighbors at depth 1 only (cheap BFS; depth 2 adds noise without much signal)
    for edge in &graph.edges {
        if let Some(&score) = gmap.get(&edge.source) {
            if !gmap.contains_key(&edge.target) {
                gmap.insert(edge.target.clone(), score * 0.5);
            }
        }
        if let Some(&score) = gmap.get(&edge.target) {
            if !gmap.contains_key(&edge.source) {
                gmap.insert(edge.source.clone(), score * 0.5);
            }
        }
    }
    gmap
}

/// Session diversification: cap at `max_per_session` memories per session_id, then
/// fill from the remainder if we still need more. `None` session_id counts as a unique
/// bucket (so ungrounded memories aren't all dropped together).
pub fn diversify_by_session(
    results: Vec<ScoredMemory>,
    limit: usize,
    max_per_session: usize,
) -> Vec<ScoredMemory> {
    if limit == 0 || results.is_empty() {
        return Vec::new();
    }
    let mut selected: Vec<ScoredMemory> = Vec::with_capacity(limit);
    let mut per_session: HashMap<Option<String>, usize> = HashMap::new();
    for r in results.iter() {
        let key = r.memory.session_id.clone();
        let count = per_session.get(&key).copied().unwrap_or(0);
        if count >= max_per_session {
            continue;
        }
        per_session.insert(key, count + 1);
        selected.push(r.clone());
        if selected.len() >= limit {
            break;
        }
    }
    // Fill from remainder if we under-shot the limit (all buckets hit their cap)
    if selected.len() < limit {
        for r in &results {
            if selected.iter().any(|s| s.memory.id == r.memory.id) {
                continue;
            }
            selected.push(r.clone());
            if selected.len() >= limit {
                break;
            }
        }
    }
    selected
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
/// become durable - facts/decisions/preferences become semantic knowledge, and gotchas (hard-won
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

    /// An engine backed by an isolated database, or `None` when `CAIRN_DB_URL` is unset
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

    // -- v0.5.0 Sprint 2: confidence + edit/delete/pin -----------------------------------

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

    // -- v0.5.0 Sprint 3: crystallize + memory graph -------------------------------------

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

    // -- v0.5.0 Sprint 7: hybrid search + MMR + graph boost ------------------------------

    #[test]
    fn mmr_rerank_returns_diverse_top_results() {
        // Two near-duplicate high-relevance hits plus one orthogonal hit. MMR (lambda=0.5) should
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
        // The second pick should NOT be the near-duplicate sqlite hit - it's too similar.
        assert!(
            reranked[1].memory.content.contains("ownership"),
            "MMR should break near-duplicates; got {}",
            reranked[1].memory.content
        );
    }

    #[test]
    fn mmr_lambda_one_is_pure_relevance() {
        // Three hits so MMR actually has to choose (the early-return when len<=limit doesn't
        // fire). Lambda 1.0 = relevance only - the top two by score should win.
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
        // Two near-duplicates + one orthogonal - MMR should give us one of the duplicates
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

    /// P3.3: `expanded_search` with the LLM gate off (default) short-circuits to the
    /// single-query `hybrid_search` path - same result, no LLM call.
    #[test]
    fn expanded_search_disabled_falls_back_to_hybrid_search() {
        let Some(mem) = engine() else { return };
        mem.remember(NewMemory::new("P3.3 disabled fallback target"))
            .unwrap();
        let cfg = cairn_core::LlmConsolidationConfig {
            enabled: false,
            url: "http://localhost:11434/v1/chat/completions".to_string(),
            model: "llama3.2".to_string(),
            api_key: None,
        };
        let expander = QueryExpander::new(cfg);
        let hits = mem
            .expanded_search("P3.3 disabled fallback", 5, 20, &expander)
            .unwrap();
        assert!(
            hits.iter()
                .any(|h| h.memory.content.contains("P3.3 disabled fallback")),
            "disabled expander should still find the memory"
        );
    }

    /// P4.2: `hybrid_search_with_rerank` with a hand-rolled stub reranker that returns
    /// deterministic scores. Verifies the alpha-blend and the post-MMR rerank pipeline.
    #[test]
    fn hybrid_search_with_rerank_blends_scores() {
        let Some(mem) = engine() else { return };
        mem.remember(NewMemory::new("how to configure cairn embeddings"))
            .unwrap();
        mem.remember(NewMemory::new("cairn embedding model guide"))
            .unwrap();
        mem.remember(NewMemory::new("rust async runtime tokio"))
            .unwrap();

        // Stub reranker: the second memory is the "most relevant" by cross-encoder
        // logic, the first is the second-most, the third is least.
        let stub = StubReranker::new(|docs| {
            let mut out = Vec::new();
            for (i, d) in docs.iter().enumerate() {
                let score = if d.contains("embedding model guide") {
                    1.0
                } else if d.contains("how to configure") {
                    0.5
                } else {
                    0.0
                };
                out.push(RerankOutcome {
                    original_index: i,
                    score,
                });
            }
            out
        });

        let hits = mem
            .hybrid_search_with_rerank(
                "cairn embedding configuration",
                2,
                20,
                &stub,
                0.6, // alpha - lean toward the reranker
            )
            .unwrap();
        // With alpha=0.6 and the cross-encoder putting doc 1 at #1, that should
        // surface at position 0. Doc 0 (the "how to configure" hit) gets blended
        // score ~0.6*0.5 + 0.4*hybrid. The async runtime should NOT be in top-2.
        assert_eq!(hits.len(), 2);
        assert!(
            hits[0].memory.content.contains("embedding model guide")
                || hits[0].memory.content.contains("how to configure"),
            "top result should be one of the two relevant docs, got {}",
            hits[0].memory.content
        );
        assert!(!hits.iter().any(|h| h.memory.content.contains("tokio")));
    }

    /// P4.2: when no reranker is installed via `with_reranker`, `hybrid_search_with_rerank`
    /// can still be called with a passed-in reranker. Verify the no-op contract.
    #[test]
    fn hybrid_search_with_rerank_falls_back_when_reranker_returns_error() {
        let Some(mem) = engine() else { return };
        mem.remember(NewMemory::new("P4.2 fallback test")).unwrap();

        // An always-erroring reranker - should fall back to MMR ordering and not
        // 500 the request.
        struct ErrorReranker;
        impl Reranker for ErrorReranker {
            fn rerank(
                &self,
                _q: &str,
                _docs: &[&str],
            ) -> std::result::Result<Vec<RerankOutcome>, RerankError> {
                Err(RerankError::Inference("simulated failure".into()))
            }
        }
        let hits = mem
            .hybrid_search_with_rerank("P4.2 fallback", 5, 20, &ErrorReranker, 0.6)
            .unwrap();
        assert!(hits
            .iter()
            .any(|h| h.memory.content.contains("P4.2 fallback")));
    }

    /// Tiny test stub: a Reranker that lets the test supply a closure for the score
    /// function. Avoids needing a real model artifact in the test.
    struct StubReranker<F>(F)
    where
        F: Fn(&[&str]) -> Vec<RerankOutcome>;
    impl<F> StubReranker<F>
    where
        F: Fn(&[&str]) -> Vec<RerankOutcome>,
    {
        fn new(f: F) -> Self {
            Self(f)
        }
    }
    impl<F> Reranker for StubReranker<F>
    where
        F: Fn(&[&str]) -> Vec<RerankOutcome> + Send + Sync,
    {
        fn rerank(
            &self,
            _q: &str,
            docs: &[&str],
        ) -> std::result::Result<Vec<RerankOutcome>, RerankError> {
            Ok((self.0)(docs))
        }
    }

    pub fn synth(content: &str) -> Memory {
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

    // --- P1.3 Triple-Stream tests ---

    #[test]
    fn extract_entities_parses_quoted_strings() {
        let entities = extract_entities("hello \"world test\" foo");
        assert!(entities.contains(&"world test".to_string()));
    }

    #[test]
    fn extract_entities_parses_capitalized_words() {
        let entities = extract_entities("Foo bar BazTest QuuxItem");
        // "Foo" is the first word (sentence-initial), skipped. Others should be present.
        assert!(entities.contains(&"BazTest".to_string()));
        assert!(entities.contains(&"QuuxItem".to_string()));
        assert!(!entities.contains(&"Foo".to_string()));
    }

    #[test]
    fn extract_entities_dedups() {
        let entities = extract_entities("BazTest and BazTest again");
        let count = entities.iter().filter(|e| *e == "BazTest").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn graph_proximity_scores_empty_when_no_entities() {
        let graph = MemoryGraph {
            nodes: vec![],
            edges: vec![],
        };
        let scores = graph_proximity_scores(&graph, &[]);
        assert!(scores.is_empty());
    }

    #[test]
    fn graph_proximity_scores_propagates_to_neighbors() {
        let node_a = MemoryGraphNode {
            id: "a".into(),
            kind: "fact".into(),
            tier: "semantic".into(),
            content_preview: "BazTest topic here".into(),
            confidence: 0.9,
            pinned: false,
            importance: 0.5,
        };
        let node_b = MemoryGraphNode {
            id: "b".into(),
            kind: "decision".into(),
            tier: "episodic".into(),
            content_preview: "unrelated content".into(),
            confidence: 0.7,
            pinned: false,
            importance: 0.5,
        };
        let edge = MemoryGraphEdge {
            source: "a".into(),
            target: "b".into(),
            kind: "derived_from".into(),
        };
        let graph = MemoryGraph {
            nodes: vec![node_a, node_b],
            edges: vec![edge],
        };
        let entities = vec!["BazTest".to_string()];
        let scores = graph_proximity_scores(&graph, &entities);
        assert_eq!(scores.get("a"), Some(&1.0));
        assert_eq!(scores.get("b"), Some(&0.5)); // neighbor at depth 1
    }

    #[test]
    fn diversify_by_session_caps_per_session() {
        // All 5 results from session s1, cap=3. Per the spec, we take 3 from s1 first,
        // then fill the remaining 2 slots from the s1 remainder since cap was reached.
        let results: Vec<ScoredMemory> = (0..5)
            .map(|i| ScoredMemory {
                memory: Memory {
                    id: format!("m{}", i),
                    session_id: Some("s1".to_string()),
                    ..synth("content")
                },
                score: 1.0 - i as f32 * 0.01,
            })
            .collect();
        let out = diversify_by_session(results.clone(), 3, 3);
        // With limit=3 and cap=3, we get exactly 3 results (the cap is also the limit)
        assert_eq!(out.len(), 3);
        // The 3 should be the highest-scored (m0, m1, m2)
        assert_eq!(out[0].memory.id, "m0");
        assert_eq!(out[1].memory.id, "m1");
        assert_eq!(out[2].memory.id, "m2");
    }

    #[test]
    fn diversify_by_session_caps_at_three() {
        // Mixed: 3 from A (high score), 3 from B. Cap=2, limit=4.
        // First pass: take 2 from A (top scores), then 2 from B.
        let mut results: Vec<ScoredMemory> = (0..3)
            .map(|i| ScoredMemory {
                memory: Memory {
                    id: format!("a{}", i),
                    session_id: Some("A".to_string()),
                    ..synth("a content")
                },
                score: 1.0 - i as f32 * 0.1,
            })
            .collect();
        results.extend((0..3).map(|i| ScoredMemory {
            memory: Memory {
                id: format!("b{}", i),
                session_id: Some("B".to_string()),
                ..synth("b content")
            },
            score: 0.5 - i as f32 * 0.1,
        }));
        let out = diversify_by_session(results, 4, 2);
        assert_eq!(out.len(), 4);
        // 2 from each session
        let a_count = out
            .iter()
            .filter(|s| s.memory.session_id == Some("A".into()))
            .count();
        let b_count = out
            .iter()
            .filter(|s| s.memory.session_id == Some("B".into()))
            .count();
        assert_eq!(a_count, 2);
        assert_eq!(b_count, 2);
    }

    #[test]
    fn diversify_by_session_fills_from_remaining() {
        // 8 from session A, 2 from session B. With cap=3 and limit=5, expect 3 from A + 2 from B.
        let mut results: Vec<ScoredMemory> = (0..8)
            .map(|i| ScoredMemory {
                memory: Memory {
                    id: format!("a{}", i),
                    session_id: Some("A".to_string()),
                    ..synth("a content")
                },
                score: 1.0 - i as f32 * 0.01,
            })
            .collect();
        results.extend((0..2).map(|i| ScoredMemory {
            memory: Memory {
                id: format!("b{}", i),
                session_id: Some("B".to_string()),
                ..synth("b content")
            },
            score: 0.5 - i as f32 * 0.01,
        }));
        let out = diversify_by_session(results, 5, 3);
        assert_eq!(out.len(), 5);
        let a_count = out
            .iter()
            .filter(|s| s.memory.session_id == Some("A".into()))
            .count();
        let b_count = out
            .iter()
            .filter(|s| s.memory.session_id == Some("B".into()))
            .count();
        assert_eq!(a_count, 3);
        assert_eq!(b_count, 2);
    }
}
