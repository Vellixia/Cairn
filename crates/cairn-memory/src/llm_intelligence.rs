//! LLM-driven background intelligence (v0.8.0 Sprint 5): concept extraction, contradiction
//! detection, and promotion-candidate scoring. Runs inside the `llm-intelligence` cron job
//! (`crates/cairn-api/src/cron.rs`), gated end-to-end by `LlmConsolidationConfig::enabled` -
//! every function here is a safe, cheap no-op when the LLM is disabled.
//!
//! Reuses `llm_consolidator::chat_with_config` for the HTTP call (no second client) and
//! follows this crate's existing tag-based output parsing convention (`parse_facts` etc. in
//! `llm_consolidator.rs`) rather than asking the LLM for strict JSON - a model that drifts
//! from the format silently drops just that one item instead of failing the whole batch.

use crate::llm_consolidator::chat_with_config;
use cairn_core::{LlmConsolidationConfig, MemoryKind};

/// Ask the LLM to extract up to 5 key concepts from `content`. Returns an empty vec on any
/// failure (network error, empty/malformed response) - concept extraction is a nice-to-have
/// enrichment, never worth failing the whole cron run over.
pub(crate) fn extract_concepts_via_llm(cfg: &LlmConsolidationConfig, content: &str) -> Vec<String> {
    let prompt = format!(
        "Extract up to 5 key concepts (single words or short phrases) from this text. \
         Output each as <concept>the concept</concept> and nothing else.\n\nText: {content}"
    );
    match chat_with_config(cfg, &prompt) {
        Ok(text) => parse_concepts(&text),
        Err(e) => {
            tracing::warn!(error = %e, "concept extraction LLM call failed");
            Vec::new()
        }
    }
}

/// Parse `<concept>...</concept>` tags. Lenient, same style as `LlmConsolidator::parse_facts`.
pub(crate) fn parse_concepts(text: &str) -> Vec<String> {
    text.split("<concept>")
        .skip(1)
        .filter_map(|chunk| chunk.split("</concept>").next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(5)
        .collect()
}

/// Ask the LLM whether `statement` contradicts `candidate`. Defaults to `false` (not a
/// contradiction) on any failure - a missed contradiction is far cheaper than a false positive
/// that wrongly flags a good memory `suspicious`.
pub(crate) fn check_contradiction_via_llm(
    cfg: &LlmConsolidationConfig,
    statement: &str,
    candidate: &str,
) -> bool {
    let prompt = format!(
        "Do these two statements directly contradict each other (not just discuss the same \
         topic)? Answer with exactly one word, YES or NO.\n\nStatement A: {statement}\n\
         Statement B: {candidate}"
    );
    match chat_with_config(cfg, &prompt) {
        Ok(text) => parse_yes_no(&text),
        Err(e) => {
            tracing::warn!(error = %e, "contradiction-detection LLM call failed");
            false
        }
    }
}

/// `true` only if the response's first word is (case-insensitively) "yes".
pub(crate) fn parse_yes_no(text: &str) -> bool {
    text.split_whitespace().next().is_some_and(|w| {
        w.trim_matches(|c: char| !c.is_alphanumeric())
            .eq_ignore_ascii_case("yes")
    })
}

/// Ask the LLM for a 2-3 sentence summary of what a session accomplished, from its memory
/// contents. Returns `None` on any failure or empty response.
pub(crate) fn summarize_session_via_llm(
    cfg: &LlmConsolidationConfig,
    contents: &[String],
) -> Option<String> {
    let joined = contents
        .iter()
        .map(|c| format!("- {c}"))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "Summarize in 2-3 sentences what was accomplished in this session, based on these \
         notes. Output the summary wrapped in <summary></summary> and nothing else.\n\n{joined}"
    );
    match chat_with_config(cfg, &prompt) {
        Ok(text) => {
            let summary = text
                .split("<summary>")
                .nth(1)
                .and_then(|s| s.split("</summary>").next())
                .unwrap_or(text.trim())
                .trim()
                .to_string();
            if summary.is_empty() {
                None
            } else {
                Some(summary)
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "session-summary LLM call failed");
            None
        }
    }
}

/// Prior weight for how universally applicable a memory's `kind` tends to be - a `Fact` or
/// reusable `Gotcha` travels well across projects; a `Task`/`Preference` is inherently
/// project- or person-specific and should never be promoted regardless of other signals.
pub(crate) fn kind_prior_score(kind: MemoryKind) -> f32 {
    match kind {
        MemoryKind::Fact => 0.9,
        MemoryKind::Gotcha => 0.7,
        MemoryKind::Decision => 0.6,
        MemoryKind::Note => 0.3,
        MemoryKind::Task | MemoryKind::Preference => 0.0,
    }
}

/// Combine the kind prior with the cross-project access signal into a fast, LLM-free score.
/// `cross_project_hits` saturates at 3 hits (`/3.0` capped to `1.0`) - a memory doesn't need to
/// be accessed from every other project to prove it's broadly useful.
///
/// Bug R1 fix: the old weights (kind_prior * 0.4 + cross_score * 0.6) made the ceiling
/// 0.36 when cross_project_hits was always 0 (Project-scoped memories are only recalled
/// under their own project — scope isolation makes cross-project access structurally
/// impossible). The new weights (kind_prior * 0.8 + cross_score * 0.2) let a high
/// kind_prior alone reach the candidate band: Fact = 0.72, Gotcha = 0.56, Decision = 0.48.
/// Cross-project hits still provide a boost (up to +0.2) when the signal exists.
pub(crate) fn fast_promotion_score(kind: MemoryKind, cross_project_hits: i64) -> f32 {
    let cross_score = (cross_project_hits as f32 / 3.0).min(1.0);
    kind_prior_score(kind) * 0.8 + cross_score * 0.2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_concepts_extracts_tagged_items_and_caps_at_five() {
        let text = "<concept>rust</concept> blah <concept>ownership</concept>\
                    <concept>borrow checker</concept><concept>lifetimes</concept>\
                    <concept>traits</concept><concept>generics</concept>";
        let concepts = parse_concepts(text);
        assert_eq!(concepts.len(), 5, "capped at 5 even though 6 tags present");
        assert_eq!(concepts[0], "rust");
        assert_eq!(concepts[1], "ownership");
    }

    #[test]
    fn parse_concepts_ignores_malformed_or_empty_tags() {
        assert!(parse_concepts("no tags here").is_empty());
        assert!(parse_concepts("<concept></concept>").is_empty());
    }

    #[test]
    fn parse_yes_no_matches_leading_word_case_insensitively() {
        assert!(parse_yes_no("YES"));
        assert!(parse_yes_no("yes, because..."));
        assert!(parse_yes_no("Yes."));
        assert!(!parse_yes_no("NO"));
        assert!(!parse_yes_no("no, these are unrelated"));
        assert!(!parse_yes_no(""));
    }

    #[test]
    fn kind_prior_score_zeroes_out_task_and_preference() {
        assert_eq!(kind_prior_score(MemoryKind::Task), 0.0);
        assert_eq!(kind_prior_score(MemoryKind::Preference), 0.0);
        assert!(kind_prior_score(MemoryKind::Fact) > kind_prior_score(MemoryKind::Note));
    }

    #[test]
    fn fast_promotion_score_saturates_cross_project_hits_at_three() {
        let at_three = fast_promotion_score(MemoryKind::Fact, 3);
        let at_ten = fast_promotion_score(MemoryKind::Fact, 10);
        assert_eq!(at_three, at_ten, "cross-project signal saturates at 3 hits");
        assert!(
            fast_promotion_score(MemoryKind::Task, 10) < at_three,
            "task kind never scores high"
        );
    }
}
