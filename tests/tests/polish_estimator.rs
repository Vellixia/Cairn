//! T075 — the token estimator's error on real briefings (FR-029, D8).
//!
//! Cairn's budget is denominated in its own estimated tokens; the guarantee is
//! that the estimator is *conservative*, so the estimated budget is a safe
//! upper bound. This measures that on briefings the product actually produces.
//!
//! The comparison is against a **real BPE tokenizer** (`cl100k_base`, the
//! encoding GPT-4-class models use), not a rule of thumb. Cairn still claims
//! compliance only against its own estimator — this measures how far that
//! estimator sits from a real one, and in which direction.

use cairn_core::budget::estimate;
use cairn_e2e::Sandbox;
use serde_json::json;

/// Kept as a secondary reference alongside the real tokenizer.
const REFERENCE_CHARS_PER_TOKEN: f64 = 4.0;

#[test]
fn the_estimator_over_counts_on_real_briefings() {
    let s = Sandbox::new();

    s.hook(
        "SessionStart",
        json!({ "session_id": "est", "source": "startup" }),
    );
    s.write_file("src/limiter.rs", "pub fn limit() {}\n");
    s.hook(
        "PostToolUse",
        json!({ "session_id": "est", "tool_name": "Edit",
                "tool_input": { "file_path": "src/limiter.rs" } }),
    );
    s.hook(
        "PostToolUse",
        json!({ "session_id": "est", "tool_name": "Bash",
                "tool_input": { "command": "cargo test --workspace" },
                "tool_response": { "exit_code": 101 } }),
    );
    for i in 0..8 {
        s.must(&[
            "memory",
            "add",
            "--type",
            "convention",
            "--scope",
            "project",
            &format!("Convention {i}: errors are returned, never logged and swallowed"),
        ]);
    }
    s.hook(
        "SessionEnd",
        json!({ "session_id": "est", "reason": "clear" }),
    );

    let rendered = s.cairn(&["context"]).stdout;
    assert!(!rendered.is_empty(), "a briefing should have been produced");

    let estimated = estimate(&rendered);

    // A real BPE tokenizer, not an approximation.
    let bpe = tiktoken_rs::cl100k_base().expect("cl100k_base");
    let actual = bpe.encode_with_special_tokens(&rendered).len();
    let rule_of_thumb =
        (rendered.chars().count() as f64 / REFERENCE_CHARS_PER_TOKEN).ceil() as usize;

    let error = (estimated as f64 - actual as f64) / actual as f64 * 100.0;
    println!(
        "T075: briefing of {} chars\n  \
         Cairn estimate      {estimated}\n  \
         cl100k_base actual  {actual}\n  \
         4-chars rule        {rule_of_thumb}\n  \
         error vs real tokenizer: {error:+.1}% (conservative when positive)",
        rendered.chars().count()
    );

    assert!(
        estimated >= actual,
        "the estimator under-counted a real tokenizer ({estimated} < {actual}); \
         the budget would stop being a safe upper bound"
    );

    // The reported budget accounting must itself be honest.
    let payload = s.json(&["context"]);
    let spent = payload["estimated_tokens"].as_u64().unwrap();
    let budget = payload["budget"].as_u64().unwrap();
    assert!(spent <= budget, "{spent} of {budget}");
    println!("T075: briefing reported {spent} of {budget} estimated tokens");
}
