//! The context assembler - Cairn's answer to context rot.
//!
//! Research shows every model degrades as input grows, and that information in the *middle* of a
//! long context gets ignored ("lost in the middle"). So instead of dumping everything, the
//! assembler builds the smallest high-signal working set that fits a token budget and **orders it
//! so the best items sit at the two edges**, with weaker items in the middle. Anything that
//! doesn't fit is reported as dropped - and is always one memory recall away, so nothing is lost.

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
        let mem = Arc::new(MemoryEngine::new(Arc::new(Store::open_for_test()?)));
        Some((Assembler::new(mem.clone()), mem))
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
}
