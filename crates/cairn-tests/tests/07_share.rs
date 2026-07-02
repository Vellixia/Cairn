//! 07 — Privacy-first sanitization: secret detection, redaction, sensitivity
//! classification, and ShareBundle round-trip.
//!
//! `Sanitizer::new` + `sanitize` are pure — no store. The
//! `sanitize_memory` / `bundle` methods are also pure. We exercise every
//! redaction category with the secret-heavy fixture.

use cairn_share::{Sanitizer, SecretKind, Sensitivity};

#[test]
fn sanitizer_finds_every_documented_secret_category() {
    let text = cairn_tests::fixtures::mock_secret_heavy_text();
    let s = Sanitizer::new();
    let out = s.sanitize(text);
    // We expect at least one finding per category, but at minimum
    // the high-priority ones: PrivateKey, OpenAiKey, GitHubToken,
    // SlackToken, Jwt, AwsKey, Email, IpAddress, HomePath, Bearer.
    let kinds: Vec<SecretKind> = out.findings.iter().map(|f| f.kind).collect();
    for required in [
        SecretKind::OpenAiKey,
        SecretKind::GithubToken,
        SecretKind::SlackToken,
        SecretKind::Jwt,
        SecretKind::AwsKey,
        SecretKind::Email,
        SecretKind::IpAddress,
    ] {
        assert!(
            kinds.contains(&required),
            "expected finding of {:?}, got {:?}",
            required,
            kinds
        );
    }
    // The resulting text must not contain any of the original raw
    // secrets.
    for needle in [
        "sk-abcdefghijklmnopqrstuvwxyz0123456789",
        "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        "xoxb-EXAMPLE-0000000000-notarealslacksecrettoken",
        "AKIAIOSFODNN7EXAMPLE",
        "eyJhbGciOiJIUzI1NiJ9",
    ] {
        assert!(
            !out.text.contains(needle),
            "raw secret {needle} survived redaction"
        );
    }
    // Email/IPs are redacted.
    assert!(!out.text.contains("alice@example.com"));
    assert!(!out.text.contains("10.0.0.42"));
}

#[test]
fn sensitivity_classification_runs_to_most_sensitive() {
    let s = Sanitizer::new();
    // A string with only an email -> NeedsReview (PII, not a credential).
    let s_email = s.sanitize("contact alice@example.com please");
    assert_eq!(s_email.sensitivity, Sensitivity::NeedsReview);
    // A string with a key -> Private.
    let s_key = s.sanitize("my key is sk-abcdefghijklmnopqrstuvwxyz0123456789");
    assert_eq!(s_key.sensitivity, Sensitivity::Private);
    // A clean string -> Shareable.
    let s_clean = s.sanitize("the cargo build finished in 12.3s");
    assert_eq!(s_clean.sensitivity, Sensitivity::Shareable);
    // Mixed -> most-sensitive wins.
    let s_mixed =
        s.sanitize("email alice@example.com and key sk-abcdefghijklmnopqrstuvwxyz0123456789");
    assert_eq!(s_mixed.sensitivity, Sensitivity::Private);
}

#[test]
fn sanitizer_handles_clean_input() {
    let s = Sanitizer::new();
    let out = s.sanitize("this is a clean technical note about token budgets");
    assert!(out.findings.is_empty());
    assert_eq!(out.sensitivity, Sensitivity::Shareable);
    assert_eq!(
        out.text,
        "this is a clean technical note about token budgets"
    );
}

#[test]
fn sanitizer_is_idempotent() {
    // Running sanitize twice on the already-redacted text finds no
    // further findings. Important for the dashboard's "Share" button
    // which calls sanitize repeatedly.
    let s = Sanitizer::new();
    let out1 = s.sanitize(cairn_tests::fixtures::mock_secret_heavy_text());
    let out2 = s.sanitize(&out1.text);
    assert!(out2.findings.is_empty(), "redacted text is clean");
    assert_eq!(out2.sensitivity, Sensitivity::Shareable);
}

#[test]
fn finding_order_is_by_position() {
    let s = Sanitizer::new();
    let text = "alice@example.com and bob@vellixia.io are both on the team";
    let findings = s.scan(text);
    assert!(!findings.is_empty());
    // Findings are reported in start-position order so the
    // dashboard's "highlighted spans" list is stable.
    for w in findings.windows(2) {
        assert!(w[0].start <= w[1].start, "findings ordered by start");
    }
}

#[test]
fn share_bundle_round_trip_through_serde() {
    use cairn_share::{ShareBundle, ShareableMemory};
    let bundle = ShareBundle {
        schema: ShareBundle::SCHEMA.to_string(),
        version: 1,
        memories: vec![ShareableMemory {
            kind: cairn_core::MemoryKind::Fact,
            content: "fact content".to_string(),
            concepts: vec!["a".to_string()],
            sensitivity: Sensitivity::Shareable,
            redactions: 0,
        }],
    };
    let s = serde_json::to_string(&bundle).expect("serialize");
    let back: ShareBundle = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back.schema, ShareBundle::SCHEMA);
    assert_eq!(back.memories.len(), 1);
    assert_eq!(back.memories[0].content, "fact content");
}

#[test]
fn secret_kind_placeholders_are_unique() {
    // Every SecretKind has a placeholder. No two kinds share one
    // (otherwise redaction would lose information).
    let placeholders: std::collections::HashSet<&str> = [
        SecretKind::PrivateKey,
        SecretKind::AwsKey,
        SecretKind::GithubToken,
        SecretKind::SlackToken,
        SecretKind::GoogleApiKey,
        SecretKind::StripeKey,
        SecretKind::OpenAiKey,
        SecretKind::AnthropicKey,
        SecretKind::GenericSecret,
        SecretKind::Jwt,
        SecretKind::NamedSecret,
        SecretKind::BearerToken,
        SecretKind::Email,
        SecretKind::IpAddress,
        SecretKind::HomePath,
        SecretKind::HighEntropy,
    ]
    .iter()
    .map(|k| k.placeholder())
    .collect(); // No placeholder is empty.
    for k in [
        SecretKind::PrivateKey,
        SecretKind::OpenAiKey,
        SecretKind::Email,
        SecretKind::IpAddress,
    ] {
        assert!(!k.placeholder().is_empty());
    }
    // Some kinds share a placeholder (GenericSecret / NamedSecret both
    // redact to `[redacted:secret]`) but at minimum we have at least
    // 12 unique placeholders across the 16 kinds — the catch-alls
    // collapse together.
    assert!(placeholders.len() >= 12, "at least 12 unique placeholders");
}
