//! The context assembler — Cairn's answer to context rot.
//!
//! Research shows every model degrades as input grows, and that information in the *middle* of a
//! long context gets ignored ("lost in the middle"). So instead of dumping everything, the
//! assembler builds the smallest high-signal working set that fits a token budget and **orders it
//! so the best items sit at the two edges**, with weaker items in the middle. Anything that
//! doesn't fit is reported as dropped — and is always one memory recall away, so nothing is lost.

use cairn_core::Result;
use cairn_memory::{MemoryEngine, ScoredMemory};
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
}

impl Assembler {
    pub fn new(mem: Arc<MemoryEngine>) -> Self {
        Self { mem }
    }

    /// Build the working set for `query` under `budget_tokens`.
    pub fn assemble(&self, query: &str, budget_tokens: usize) -> Result<AssemblyReport> {
        let hits = self.mem.recall(query, 50)?;

        // Greedily pack the highest-ranked items until the budget is exhausted.
        let mut packed: Vec<(ScoredMemory, usize)> = Vec::new();
        let mut dropped = Vec::new();
        let mut used = 0usize;
        for h in hits {
            let est = est_tokens(&h.memory.content);
            if used + est <= budget_tokens {
                used += est;
                packed.push((h, est));
            } else {
                dropped.push(DroppedItem {
                    preview: preview(&h.memory.content),
                    score: h.score,
                    est_tokens: est,
                    reason: "over token budget".to_string(),
                });
            }
        }

        // Place the best items at the edges, weakest in the middle.
        let ordered = edge_order(packed);

        let mut included = Vec::with_capacity(ordered.len());
        let mut context = format!("# Cairn context for: {query}\n");
        for (position, (h, est)) in ordered.into_iter().enumerate() {
            let ScoredMemory { memory, score } = h;
            context.push_str(&format!(
                "\n[{}] ({}) {}\n",
                position + 1,
                memory.kind.as_str(),
                memory.content
            ));
            included.push(AssembledItem {
                position,
                source: "memory".to_string(),
                kind: memory.kind.as_str().to_string(),
                content: memory.content,
                score,
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

/// Reorder by rank so the best items sit at both ends: `[r0, r2, r4, …, r5, r3, r1]`.
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
        format!("{p}…")
    } else {
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::NewMemory;
    use cairn_store::Store;

    /// `None` when `CAIRN_HELIX_URL` is unset (offline runs skip these integration tests).
    fn setup() -> Option<(Assembler, Arc<MemoryEngine>)> {
        let mem = Arc::new(MemoryEngine::new(Arc::new(Store::open_for_test()?)));
        Some((Assembler::new(mem.clone()), mem))
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
}
