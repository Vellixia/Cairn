//! 12 — Proactive recall: intent classifier heuristics, auto-inject gate, opt-out flag.

use cairn_proactive::intent::{classify, IntentDecision, IntentFeatures, IntentSignal};

#[test]
fn classify_handles_typical_prompts() {
    // The classifier feeds the auto-inject gate. It must not panic on
    // any input — even empty / unicode / very long prompts.
    for prompt in [
        "what did we decide last week?",
        "show me the README",
        "do not use any memories",
        "ok continue",
        "",
        "🤖",
    ] {
        let _ = classify(prompt, 0.5);
    }
}

#[test]
fn question_with_recall_cue_fires() {
    // The classifier's main signal: a question with a recall cue.
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
    // A "do something" command does not trigger auto-inject. The
    // agent sees no memory unless the user actually asks.
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
    // A stray "?" on a 2-word prompt must not trigger auto-inject.
    // Without this guard, every "ok?" would inject 3 memories.
    let d = classify("ok?", 0.4);
    assert!(matches!(d, IntentDecision::Skip(_)));
}

#[test]
fn features_default_is_empty() {
    // The default features carry no signals — a "no signal" prompt.
    let f = IntentFeatures::default();
    assert!(f.signals.is_empty());
    assert_eq!(f.question_count, 0);
    assert_eq!(f.recall_cue_count, 0);
    assert_eq!(f.file_mention_count, 0);
    assert_eq!(f.word_count, 0);
}

#[test]
fn intent_signal_clone_preserves_variant() {
    // The classifier hands signals across threads.
    let s = IntentSignal::QuestionMarker;
    let s2 = s.clone();
    assert_eq!(s, s2);
}
