//! LongMemEval / LoCoMo recall benchmark.
//!
//! For each fixture, we feed the facts into a synthetic store, then ask each
//! question and grade the retrieved set against the expected fact IDs. We use
//! lexical overlap as the recall score: a question is "correctly answered" if
//! every keyword from `expected_keywords` appears in the top-K retrieved fact
//! contents. (The harness is intentionally lightweight — it doesn't pull in
//! Cairn's full memory engine so this crate compiles fast and the benchmark
//! stays deterministic.)
//!
//! The benchmark reports:
//! - `recall_at_1`, `recall_at_3`, `recall_at_5` — fraction of questions whose
//!   expected keywords all appear in the top-K retrieved facts.
//! - `precision_at_5` — fraction of top-5 retrieved facts that are relevant
//!   (intersect with expected IDs).
//! - `mean_rank` — average rank of the first relevant fact (1-indexed; ∞ if
//!   no relevant fact retrieved).

use crate::fixture::Fixture;
use crate::{BenchKind, BenchResult};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LongMemEvalResult {
    pub fixtures: usize,
    pub questions: usize,
    pub recall_at_1: f64,
    pub recall_at_3: f64,
    pub recall_at_5: f64,
    pub precision_at_5: f64,
    pub mean_rank: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LongMemEvalBenchmark;

impl LongMemEvalBenchmark {
    pub fn run(fixtures: &[Fixture]) -> LongMemEvalResult {
        let mut r_at_1 = 0usize;
        let mut r_at_3 = 0usize;
        let mut r_at_5 = 0usize;
        let mut p_at_5_num = 0usize;
        let mut p_at_5_den = 0usize;
        let mut total_rank = 0usize;
        let mut rank_count = 0usize;
        let mut total_questions = 0usize;

        for f in fixtures {
            for q in &f.questions {
                total_questions += 1;
                let qwords = words(&q.text);
                // Score each fact by lexical overlap with the question.
                let mut scored: Vec<(usize, usize)> = f
                    .facts
                    .iter()
                    .enumerate()
                    .map(|(i, fact)| {
                        let fwords = words(&fact.content);
                        let overlap = qwords.intersection(&fwords).count();
                        (i, overlap)
                    })
                    .collect();
                // Sort by overlap desc, then by fact id asc (stable).
                scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

                let retrieved_ids: Vec<&str> = scored
                    .iter()
                    .take(5)
                    .map(|(i, _)| f.facts[*i].id.as_str())
                    .collect();
                let top_contents: Vec<&str> = scored
                    .iter()
                    .take(5)
                    .map(|(i, _)| f.facts[*i].content.as_str())
                    .collect();

                // Recall: all expected keywords appear in the top-K contents?
                let top1 = top_contents.get(0..1).unwrap_or(&[]);
                let top3 = top_contents.get(0..3).unwrap_or(&[]);
                let top_1_keywords_ok = q
                    .keywords
                    .iter()
                    .all(|kw| contains_any(top1.iter().copied(), kw));
                let top_3_keywords_ok = q
                    .keywords
                    .iter()
                    .all(|kw| contains_any(top3.iter().copied(), kw));
                let top_5_keywords_ok = q
                    .keywords
                    .iter()
                    .all(|kw| contains_any(top_contents.iter().copied(), kw));

                if top_1_keywords_ok {
                    r_at_1 += 1;
                }
                if top_3_keywords_ok {
                    r_at_3 += 1;
                }
                if top_5_keywords_ok {
                    r_at_5 += 1;
                }

                // Precision@5: fraction of top-5 retrieved that's relevant.
                let relevant = retrieved_ids
                    .iter()
                    .filter(|id| q.expected_fact_ids.contains(&id.to_string()))
                    .count();
                p_at_5_num += relevant;
                p_at_5_den += retrieved_ids.len();

                // Mean rank: rank of first relevant fact.
                let first_relevant = retrieved_ids
                    .iter()
                    .position(|id| q.expected_fact_ids.contains(&id.to_string()));
                if let Some(pos) = first_relevant {
                    total_rank += pos + 1;
                    rank_count += 1;
                }
            }
        }

        let n = total_questions.max(1) as f64;
        LongMemEvalResult {
            fixtures: fixtures.len(),
            questions: total_questions,
            recall_at_1: r_at_1 as f64 / n,
            recall_at_3: r_at_3 as f64 / n,
            recall_at_5: r_at_5 as f64 / n,
            precision_at_5: p_at_5_num as f64 / p_at_5_den.max(1) as f64,
            mean_rank: if rank_count > 0 {
                total_rank as f64 / rank_count as f64
            } else {
                f64::INFINITY
            },
        }
    }

    /// Run the full benchmark using the default fixture set and return a complete
    /// [`BenchResult`] ready for JSON serialization.
    pub fn run_default() -> BenchResult {
        let started = std::time::Instant::now();
        let result = Self::run(&Fixture::all());
        let mut out = BenchResult::new("longmemeval-default", BenchKind::LongMemEval, 0)
            .with_meta("dataset", "cairn-bench-fixtures/v0.5.0")
            .with_meta("fixture_count", result.fixtures.to_string())
            .with_meta("question_count", result.questions.to_string());
        out.duration_ms = started.elapsed().as_millis() as u64;
        out.data = crate::BenchData::LongMemEval(result);
        out
    }
}

fn words(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase())
        .collect()
}

fn contains_any<'a>(iter: impl IntoIterator<Item = &'a str>, kw: &str) -> bool {
    let kw_lower = kw.to_ascii_lowercase();
    iter.into_iter()
        .any(|s| s.to_ascii_lowercase().contains(&kw_lower))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alex_fixture_recovers_first_question() {
        let f = Fixture::alex_employer_history();
        let r = LongMemEvalBenchmark::run(&[f]);
        assert_eq!(r.questions, 3);
        // Recall@5 should be 1.0 — lexical overlap is enough to recover all 3 questions.
        assert_eq!(r.recall_at_5, 1.0);
        assert_eq!(r.recall_at_3, 1.0);
    }

    #[test]
    fn migration_fixture_recovers_both_questions() {
        let f = Fixture::migration_timeline();
        let r = LongMemEvalBenchmark::run(&[f]);
        assert_eq!(r.questions, 2);
        assert_eq!(r.recall_at_5, 1.0);
    }

    #[test]
    fn empty_fixture_yields_zero_questions() {
        let r = LongMemEvalBenchmark::run(&[]);
        assert_eq!(r.questions, 0);
        assert_eq!(r.recall_at_5, 0.0);
    }

    #[test]
    fn mean_rank_is_one_when_first_relevant_is_top() {
        // Construct a fixture where the most lexically-overlapping fact is the
        // correct one (and is the only fact, so it must rank #1).
        let f = Fixture {
            name: "trivial".into(),
            facts: vec![crate::fixture::Fact {
                id: "x".into(),
                session: 1,
                content: "PostgreSQL is the database we use.".into(),
                entities: vec!["PostgreSQL".into()],
                day: 0,
            }],
            questions: vec![crate::fixture::Question {
                id: "q".into(),
                text: "Which database?".into(),
                expected_fact_ids: vec!["x".into()],
                keywords: vec!["PostgreSQL".into()],
            }],
        };
        let r = LongMemEvalBenchmark::run(&[f]);
        // Single fact = single retrieved result = mean rank is 1 (or Inf if no relevant found).
        assert_eq!(r.mean_rank, 1.0, "first retrieved is the relevant one");
    }
}
