//! The context assembler - Cairn's answer to context rot.
//!
//! Research shows every model degrades as input grows, and that information in the *middle* of a
//! long context gets ignored ("lost in the middle"). So instead of dumping everything, the
//! assembler builds the smallest high-signal working set that fits a token budget and **orders it
//! so the best items sit at the two edges**, with weaker items in the middle. Anything that
//! doesn't fit is reported as dropped - and is always one memory recall away, so nothing is lost.

use cairn_core::Result;
use cairn_memory::MemoryEngine;
use cairn_store::Store;
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct AssembledItem {
    pub position: usize,
    pub source: String,
    pub kind: String,
    pub content: String,
    pub score: f32,
    pub est_tokens: usize,
}

/// A recall hit, memory or document chunk, normalized to a common shape before packing. Memory
/// and document scores come from unrelated scales (a tiny RRF-fused value vs. a raw HNSW cosine
/// similarity) - see [`normalize`] - so nothing downstream of this point needs to know which
/// modality a candidate came from.
struct Candidate {
    source: &'static str,
    kind: String,
    content: String,
    score: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DroppedItem {
    pub preview: String,
    pub score: f32,
    pub est_tokens: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssemblyReport {
    pub query: String,
    pub budget_tokens: usize,
    pub used_tokens: usize,
    pub included: Vec<AssembledItem>,
    pub dropped: Vec<DroppedItem>,
    /// The assembled, edge-ordered context block ready to hand to a model.
    pub context: String,
}

pub struct Assembler {
    mem: Arc<MemoryEngine>,
    store: Arc<Store>,
}

impl Assembler {
    pub fn new(mem: Arc<MemoryEngine>, store: Arc<Store>) -> Self {
        Self { mem, store }
    }

    /// Build the working set for `query` under `budget_tokens`. Merges two modalities -
    /// memories (`MemoryEngine::recall`) and RAG document chunks (v0.8.0 Sprint 6,
    /// `Store::search_documents`) - into one ranked, budget-packed, edge-ordered context.
    /// Both calls are synchronous; `cairn-assemble` has no async runtime of its own (every
    /// engine it wraps is sync, by design - see the crate's other engines).
    pub fn assemble(&self, query: &str, budget_tokens: usize) -> Result<AssemblyReport> {
        let mem_hits = self.mem.recall(query, 50)?;
        // Document search is supplementary - a hiccup there (e.g. an embedder error) degrades
        // to memory-only results instead of failing the whole assembly.
        let doc_hits = self.store.search_documents(query, 20).unwrap_or_default();

        // Memory scores are a tiny RRF-fused value (bounded well under 1.0 in practice) while
        // document scores are reciprocal-rank over an unrelated HNSW cosine ranking - mixing
        // them unnormalized would let one modality systematically drown out the other
        // regardless of true relevance. Normalize each list independently to [0, 1] first.
        let mem_scores = normalize(&mem_hits.iter().map(|h| h.score).collect::<Vec<_>>());
        let doc_scores = normalize(
            &(0..doc_hits.len())
                .map(|rank| 1.0 / (1.0 + rank as f32))
                .collect::<Vec<_>>(),
        );

        let mut candidates: Vec<Candidate> = mem_hits
            .into_iter()
            .zip(mem_scores)
            .map(|(h, score)| Candidate {
                source: "memory",
                kind: h.memory.kind.as_str().to_string(),
                content: h.memory.content,
                score,
            })
            .collect();
        candidates.extend(doc_hits.into_iter().zip(doc_scores).map(|(c, score)| Candidate {
            source: "doc",
            kind: "doc".to_string(),
            content: c.content,
            score,
        }));
        candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Greedily pack the highest-ranked items until the budget is exhausted.
        let mut packed: Vec<(Candidate, usize)> = Vec::new();
        let mut dropped = Vec::new();
        let mut used = 0usize;
        for c in candidates {
            let est = est_tokens(&c.content);
            if used + est <= budget_tokens {
                used += est;
                packed.push((c, est));
            } else {
                dropped.push(DroppedItem {
                    preview: preview(&c.content),
                    score: c.score,
                    est_tokens: est,
                    reason: "over token budget".to_string(),
                });
            }
        }

        // Place the best items at the edges, weakest in the middle.
        let ordered = edge_order(packed);

        let mut included = Vec::with_capacity(ordered.len());
        let mut context = format!("# Cairn context for: {query}\n");
        for (position, (c, est)) in ordered.into_iter().enumerate() {
            context.push_str(&format!("\n[{}] ({}) {}\n", position + 1, c.kind, c.content));
            included.push(AssembledItem {
                position,
                source: c.source.to_string(),
                kind: c.kind,
                content: c.content,
                score: c.score,
                est_tokens: est,
            });
        }

        Ok(AssemblyReport {
            query: query.to_string(),
            budget_tokens,
            used_tokens: used,
            included,
            dropped,
            context,
        })
    }
}

/// Min-max normalize to `[0, 1]`. A single-item (or all-equal) list normalizes to `1.0` for
/// every item - there's no useful relative ordering to preserve, and `1.0` keeps it competitive
/// against the other modality rather than collapsing to `0.0`.
fn normalize(scores: &[f32]) -> Vec<f32> {
    if scores.is_empty() {
        return Vec::new();
    }
    let min = scores.iter().cloned().fold(f32::INFINITY, f32::min);
    let max = scores.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let range = max - min;
    if range <= f32::EPSILON {
        return vec![1.0; scores.len()];
    }
    scores.iter().map(|s| (s - min) / range).collect()
}

/// Reorder by rank so the best items sit at both ends: `[r0, r2, r4, ..., r5, r3, r1]`.
fn edge_order<T>(items: Vec<T>) -> Vec<T> {
    let mut left = Vec::new();
    let mut right = Vec::new();
    for (i, it) in items.into_iter().enumerate() {
        if i % 2 == 0 {
            left.push(it);
        } else {
            right.push(it);
        }
    }
    right.reverse();
    left.extend(right);
    left
}

fn est_tokens(s: &str) -> usize {
    (s.len() / 4).max(1) + 4
}

fn preview(s: &str) -> String {
    let p: String = s.chars().take(80).collect();
    if s.chars().count() > 80 {
        format!("{p}...")
    } else {
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::NewMemory;
    use cairn_store::Store;

    /// `None` when `CAIRN_DB_URL` is unset (offline runs skip these integration tests).
    fn setup() -> Option<(Assembler, Arc<MemoryEngine>)> {
        let store = Arc::new(Store::open_for_test()?);
        let mem = Arc::new(MemoryEngine::new(store.clone()));
        Some((Assembler::new(mem.clone(), store), mem))
    }

    // - edge_order ---

    #[test]
    fn edge_order_empty() {
        let v: Vec<i32> = Vec::new();
        assert!(edge_order(v).is_empty());
    }

    #[test]
    fn edge_order_single() {
        assert_eq!(edge_order(vec![42i32]), vec![42]);
    }

    #[test]
    fn edge_order_two() {
        // items [0, 1]: left=[0], right=[1] -> reversed right=[1] -> [0, 1]
        assert_eq!(edge_order(vec!['a', 'b']), vec!['a', 'b']);
    }

    #[test]
    fn edge_order_three_best_at_edges() {
        // items [0,1,2] by rank: left=[0,2], right=[1]; right.rev()=[1]; result=[0,2,1]
        // Rank 0 is at position 0 (edge), rank 1 is at position 2 (edge), rank 2 is middle.
        let result = edge_order(vec![0usize, 1usize, 2usize]);
        assert_eq!(result[0], 0, "best rank at left edge");
        assert_eq!(*result.last().unwrap(), 1, "second-best at right edge");
        assert_eq!(result[1], 2, "weakest in middle");
    }

    #[test]
    fn edge_order_four_best_two_at_edges() {
        // items [0,1,2,3]: left=[0,2], right=[1,3]; right.rev()=[3,1]; result=[0,2,3,1]
        let result = edge_order(vec![0usize, 1, 2, 3]);
        assert_eq!(result[0], 0, "rank 0 at position 0");
        assert_eq!(*result.last().unwrap(), 1, "rank 1 at last position");
    }

    #[test]
    fn edge_order_five_preserves_all_items() {
        let input: Vec<usize> = (0..5).collect();
        let result = edge_order(input.clone());
        assert_eq!(result.len(), 5);
        let mut sorted = result.clone();
        sorted.sort();
        assert_eq!(sorted, input, "no items lost or duplicated");
    }

    // - normalize ---

    #[test]
    fn normalize_empty_is_empty() {
        assert!(normalize(&[]).is_empty());
    }

    #[test]
    fn normalize_single_value_is_one() {
        assert_eq!(normalize(&[0.003]), vec![1.0]);
    }

    #[test]
    fn normalize_all_equal_values_are_one() {
        assert_eq!(normalize(&[0.5, 0.5, 0.5]), vec![1.0, 1.0, 1.0]);
    }

    #[test]
    fn normalize_spreads_to_full_zero_one_range() {
        let out = normalize(&[0.01, 0.02, 0.03]);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[2], 1.0);
        assert!((out[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn normalize_preserves_relative_order() {
        let out = normalize(&[0.1, 0.9, 0.5]);
        assert!(out[1] > out[2]);
        assert!(out[2] > out[0]);
    }

    // - est_tokens ---

    #[test]
    fn est_tokens_empty_string() {
        // len=0: max(0/4, 1)+4 = 1+4 = 5
        assert_eq!(est_tokens(""), 5);
    }

    #[test]
    fn est_tokens_very_short() {
        // len=3: max(0, 1)+4 = 5
        assert_eq!(est_tokens("abc"), 5);
    }

    #[test]
    fn est_tokens_exactly_four_chars() {
        // len=4: max(1, 1)+4 = 5
        assert_eq!(est_tokens("abcd"), 5);
    }

    #[test]
    fn est_tokens_hundred_chars() {
        // len=100: max(25, 1)+4 = 29
        let s: String = "a".repeat(100);
        assert_eq!(est_tokens(&s), 29);
    }

    #[test]
    fn est_tokens_grows_with_length() {
        let s100 = "x".repeat(100);
        let s200 = "x".repeat(200);
        assert!(
            est_tokens(&s200) > est_tokens(&s100),
            "longer -> more tokens"
        );
    }

    // - preview ---

    #[test]
    fn preview_empty_string() {
        assert_eq!(preview(""), "");
    }

    #[test]
    fn preview_short_no_ellipsis() {
        assert_eq!(preview("hello"), "hello");
    }

    #[test]
    fn preview_exactly_80_chars_no_ellipsis() {
        let s: String = "a".repeat(80);
        let p = preview(&s);
        assert_eq!(p, s);
        assert!(!p.contains("..."));
    }

    #[test]
    fn preview_81_chars_adds_ellipsis() {
        let s: String = "a".repeat(81);
        let p = preview(&s);
        assert!(p.ends_with("..."));
        let char_count = p.chars().count();
        assert_eq!(char_count, 83, "80 chars + \"...\" (3 chars)");
    }

    #[test]
    fn preview_multibyte_unicode_counts_chars_not_bytes() {
        // "e" is 2 bytes but 1 char; 80 x "e" should not add ellipsis
        let s: String = "e".repeat(80);
        let p = preview(&s);
        assert!(!p.contains("..."), "80 unicode chars -> no ellipsis");
    }

    #[test]
    fn respects_budget_and_reports_dropped() {
        let Some((a, mem)) = setup() else { return };
        for i in 0..20 {
            mem.remember(NewMemory::new(format!(
                "memory item number {i} about sqlite storage and blobs"
            )))
            .unwrap();
        }
        let report = a.assemble("sqlite storage", 60).unwrap();
        assert!(
            report.used_tokens <= 60,
            "used {} > budget",
            report.used_tokens
        );
        assert!(!report.included.is_empty());
        assert!(!report.dropped.is_empty(), "tight budget should drop items");
    }

    #[test]
    fn best_item_sits_at_an_edge() {
        let Some((a, mem)) = setup() else { return };
        mem.remember(NewMemory::new("the unique keyword zephyrium lives here"))
            .unwrap();
        for i in 0..6 {
            mem.remember(NewMemory::new(format!("unrelated filler line {i}")))
                .unwrap();
        }
        let report = a.assemble("zephyrium", 10_000).unwrap();
        let n = report.included.len();
        assert!(n >= 2);
        let best = report
            .included
            .iter()
            .max_by(|x, y| x.score.partial_cmp(&y.score).unwrap())
            .unwrap();
        assert!(
            best.position == 0 || best.position == n - 1,
            "best item should be at an edge, was {} of {n}",
            best.position
        );
    }

    #[test]
    fn assemble_merges_document_chunks_alongside_memories() {
        let Some(store) = Store::open_for_test() else {
            return;
        };
        let store = Arc::new(store);
        let mem = Arc::new(MemoryEngine::new(store.clone()));
        let asm = Assembler::new(mem.clone(), store.clone());

        mem.remember(NewMemory::new(
            "the zephyrium project uses rust for its core engine",
        ))
        .unwrap();
        store
            .replace_document(
                "docs/zephyrium.md",
                "Zephyrium docs",
                &["zephyrium's architecture is a rust workspace with several crates.".to_string()],
            )
            .unwrap();

        let report = asm.assemble("zephyrium rust architecture", 10_000).unwrap();
        assert!(
            report.included.iter().any(|i| i.source == "doc"),
            "expected at least one [doc] item in {:?}",
            report.included
        );
        assert!(report.included.iter().any(|i| i.source == "memory"));
        assert!(report.context.contains("(doc)"));
    }

    #[test]
    fn assemble_still_works_with_no_documents_ingested() {
        let Some((a, mem)) = setup() else { return };
        mem.remember(NewMemory::new("a plain memory with no documents around"))
            .unwrap();
        let report = a.assemble("plain memory", 500).unwrap();
        assert!(report.included.iter().all(|i| i.source == "memory"));
    }
}
