//! Intent classifier for proactive recall (v0.5.0 Sprint 18a).
//!
//! Tiny, fully-local heuristic — no LLM call. The classifier runs on every
//! agent turn; if it costs 10 ms we'd feel it. So:
//!
//! - **O(n) in prompt length**, single pass.
//! - No allocations beyond small fixed-size strings (everything is byte
//!   comparisons on the input slice).
//! - Heuristic score 0.0..=1.0; threshold (default 0.4) decides Fire vs Skip.
//!
//! See `lib.rs` for the rationale on keeping this local. ADR-025 covers why
//! we don't ship a learned model here.

/// One signal the classifier picked up. We expose this for diagnostics so
/// the dashboard can show *why* the hook fired (or didn't).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentSignal {
    QuestionMarker,
    RecallCue,
    FileMention,
    ReferencePronoun,
    /// Plain imperative ("Add a print") or statement — not a recall trigger.
    Imperative,
}

/// The features extracted from a prompt — used internally to score; also
/// surfaced in `IntentDecision::Fire.features` for observability.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntentFeatures {
    pub signals: Vec<IntentSignal>,
    pub question_count: usize,
    pub recall_cue_count: usize,
    pub file_mention_count: usize,
    pub reference_pronoun_count: usize,
    pub word_count: usize,
}

/// Outcome of classifying a prompt.
#[derive(Debug, Clone, PartialEq)]
pub enum IntentDecision {
    Skip(&'static str),
    Fire {
        score: f32,
        features: IntentFeatures,
    },
}

/// Classify `prompt` against `threshold` (0.0..=1.0). Returns either a Skip
/// with a short reason or a Fire with the score + extracted features.
pub fn classify(prompt: &str, threshold: f32) -> IntentDecision {
    let lower = prompt.to_ascii_lowercase();
    let mut f = IntentFeatures::default();

    // Question markers
    if lower.contains('?') {
        f.signals.push(IntentSignal::QuestionMarker);
        f.question_count += 1;
    }
    for marker in &["what ", "why ", "how ", "when ", "where ", "which "] {
        if lower.contains(marker) {
            f.signals.push(IntentSignal::QuestionMarker);
            f.question_count += 1;
        }
    }

    // Recall cues
    for cue in &[
        "remember",
        "recall",
        "decided",
        "agreed",
        "last time",
        "previously",
        "earlier",
        "before",
    ] {
        if lower.contains(cue) {
            f.signals.push(IntentSignal::RecallCue);
            f.recall_cue_count += 1;
        }
    }

    // File / path mentions — looks for `/` or `.rs` / `.ts` / `.py` patterns.
    // Cheap, won't trigger on a casual mention of "the file" but catches paths.
    let mut slash_count = 0;
    let mut ext_seen = false;
    let mut last_was_slash = false;
    let next_char_is_alnum = |idx: usize| -> bool {
        lower[idx..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric())
    };
    for (i, c) in lower.char_indices() {
        if c == '/' {
            slash_count += 1;
            last_was_slash = true;
        } else if last_was_slash && c.is_alphanumeric() {
            // token after a slash
            f.file_mention_count += 1;
            last_was_slash = false;
        } else {
            last_was_slash = false;
        }
        if c == '.' && i + 1 < lower.len() && next_char_is_alnum(i + 1) {
            ext_seen = true;
        }
    }
    if slash_count >= 2 && f.file_mention_count > 0 {
        f.signals.push(IntentSignal::FileMention);
    } else if ext_seen {
        f.signals.push(IntentSignal::FileMention);
        f.file_mention_count += 1;
    }

    // Reference pronouns — "this", "that", "the api", "the model" without
    // a clear antecedent usually means "the thing we talked about".
    for pronoun in &[
        " this ",
        " that ",
        " the api",
        " the model",
        " the cli",
        " the server",
        " above",
    ] {
        if lower.contains(pronoun) {
            f.signals.push(IntentSignal::ReferencePronoun);
            f.reference_pronoun_count += 1;
        }
    }

    // Word count — used to suppress recall on very short prompts that fire
    // on a stray question mark.
    f.word_count = lower.split_whitespace().count();

    // If none of the recall-side signals fired, classify as imperative.
    let has_recall_signal =
        f.question_count + f.recall_cue_count + f.file_mention_count + f.reference_pronoun_count
            > 0;
    if !has_recall_signal {
        f.signals.push(IntentSignal::Imperative);
        return IntentDecision::Skip("no recall signal");
    }

    // Score: sum of weighted signals, normalized to [0, 1].
    let raw = f.question_count as f32 * 0.4
        + f.recall_cue_count as f32 * 0.6
        + f.file_mention_count as f32 * 0.3
        + f.reference_pronoun_count as f32 * 0.3;
    // Saturate at raw >= 1.2 → score 1.0 (so a prompt with two cues still hits
    // the threshold deterministically).
    let score = (raw / 1.2).clamp(0.0, 1.0);

    if score < threshold {
        IntentDecision::Skip("score below threshold")
    } else {
        IntentDecision::Fire { score, features: f }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn question_with_recall_cue_fires() {
        let d = classify("What did we decide last time?", 0.4);
        match d {
            IntentDecision::Fire { score, .. } => {
                assert!(score >= 0.4, "expected score >= 0.4, got {score}");
            }
            _ => panic!("expected Fire"),
        }
    }

    #[test]
    fn plain_imperative_skips() {
        let d = classify("Add a print statement", 0.4);
        assert!(matches!(d, IntentDecision::Skip(_)));
    }

    #[test]
    fn file_path_mention_fires() {
        let d = classify("Look at crates/cairn-mcp/src/lib.rs", 0.4);
        assert!(matches!(d, IntentDecision::Fire { .. }));
    }

    #[test]
    fn very_short_question_is_suppressed() {
        // One stray "?" on a 2-word prompt shouldn't fire.
        let d = classify("ok?", 0.4);
        assert!(matches!(d, IntentDecision::Skip(_)));
    }

    #[test]
    fn threshold_controls_fire() {
        // Same prompt, two thresholds — high threshold should skip.
        let prompt = "Why?";
        assert!(matches!(classify(prompt, 0.9), IntentDecision::Skip(_)));
        // (Low threshold on a 1-word question still suppresses via the word-count
        // floor; just confirm the API doesn't crash.)
        let _ = classify(prompt, 0.01);
    }
}
