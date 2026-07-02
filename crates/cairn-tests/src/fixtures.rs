//! Hand-rolled mock data for the `test` crate's business-flow integration tests.
//!
//! Every constructor here is pure (no I/O, no clock dependency other than
//! `Utc::now()` for stable timestamps). Tests should never reach for a real
//! filesystem, network, or HelixDB — that boundary is enforced by *what* this
//! module exports, not by discipline.
//!
//! Conventions:
//! - `mock_*` constructors build a single typed value.
//! - `mock_*_with` constructors take explicit fields for tests that care
//!   about a specific value (e.g. confidence, importance).
//! - All ids are stable, deterministic UUIDs derived from a string so the same
//!   call site always gets the same id (helps across binaries).

use cairn_core::{
    LlmConsolidationConfig, Memory, MemoryKind, MemoryTier, NewMemory, OrgId, RerankConfig,
};
use chrono::{DateTime, Duration, Utc};

/// Stable id derived from a string. Two calls with the same `name` return the
/// same `String` so re-runs are reproducible and cross-binary assertions work.
pub fn stable_id(name: &str) -> String {
    // FNV-1a 64-bit hash, hex-formatted. We don't need a real UUID — we need a
    // stable, unique, log-friendly string.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}-{:04x}-{:04x}-{:04x}-{:012x}", h, 0, 0, 0, 0)
}

/// A single-tenant org id. `OrgId::default()` would work, but explicit is
/// better than implicit when reading test code.
pub fn mock_org() -> OrgId {
    OrgId::default()
}

/// A session id.
pub fn mock_session(name: &str) -> String {
    stable_id(&format!("session::{name}"))
}

/// A memory at a known timestamp `n` minutes before now.
pub fn mock_memory(id_suffix: &str, kind: MemoryKind, tier: MemoryTier, content: &str) -> Memory {
    mock_memory_at(
        id_suffix,
        kind,
        tier,
        content,
        Utc::now() - Duration::minutes(5),
    )
}

/// A memory at an explicit timestamp.
pub fn mock_memory_at(
    id_suffix: &str,
    kind: MemoryKind,
    tier: MemoryTier,
    content: &str,
    when: DateTime<Utc>,
) -> Memory {
    let id = stable_id(&format!("mem::{id_suffix}"));
    Memory {
        id,
        kind,
        tier,
        content: content.to_string(),
        concepts: default_concepts(content),
        files: vec![],
        session_id: Some(mock_session("alpha")),
        importance: 0.5,
        access_count: 0,
        org_id: mock_org(),
        suspicious: false,
        confidence: 0.5,
        pinned: false,
        derived_from: vec![],
        contradicts: vec![],
        supersedes: vec![],
        applies_to: vec![],
        created_at: when,
        updated_at: when,
    }
}

/// A memory with explicit confidence / importance / pinned — for tests that
/// want to exercise the agentmemory reinforcement curve or pinned-skip-decay.
pub fn mock_memory_with(
    id_suffix: &str,
    kind: MemoryKind,
    tier: MemoryTier,
    content: &str,
    importance: f32,
    confidence: f32,
    pinned: bool,
) -> Memory {
    let mut m = mock_memory(id_suffix, kind, tier, content);
    m.importance = importance;
    m.confidence = confidence;
    m.pinned = pinned;
    m
}

/// `NewMemory` for callers that take the input form (e.g. cairn-memory's
/// remember path) rather than the stored form.
pub fn mock_new_memory(content: &str, kind: MemoryKind) -> NewMemory {
    let mut nm = NewMemory::new(content.to_string());
    nm.kind = Some(kind);
    nm.tier = Some(MemoryTier::Working);
    nm.concepts = default_concepts(content);
    nm.importance = Some(0.5);
    nm.confidence = Some(0.5);
    nm
}

/// `NewMemory` for the consolidation / crystallize path: tier is semantic,
/// confidence bumped to 0.7 (semantic facts survive decay longer).
pub fn mock_semantic_fact(content: &str) -> NewMemory {
    let mut nm = NewMemory::new(content.to_string());
    nm.kind = Some(MemoryKind::Fact);
    nm.tier = Some(MemoryTier::Semantic);
    nm.concepts = default_concepts(content);
    nm.confidence = Some(0.7);
    Some(0.5).into_iter().for_each(|_| {}); // (no-op, just for readability)
    nm.importance = Some(0.6);
    nm
}

/// A session with `n` working-tier memories and a linear `derived_from`
/// chain so the memory graph has at least one edge per node.
pub fn mock_session_with_n_memories(n: usize) -> Vec<Memory> {
    let base = Utc::now() - Duration::hours(1);
    let mut out: Vec<Memory> = Vec::with_capacity(n);
    for i in 0..n {
        let kind = match i % 4 {
            0 => MemoryKind::Fact,
            1 => MemoryKind::Decision,
            2 => MemoryKind::Note,
            _ => MemoryKind::Task,
        };
        let content = format!("session memory {i} -- a {kind:?} about the alpha flow");
        let mut m = mock_memory_at(
            &format!("alpha-{i}"),
            kind,
            MemoryTier::Working,
            &content,
            base + Duration::minutes(i as i64),
        );
        if i > 0 {
            m.derived_from = vec![out[i - 1].id.clone()];
        }
        out.push(m);
    }
    out
}

/// `n` memories with a 60% confidence drop, simulating an old + decayed set.
pub fn mock_decayed_memories(n: usize) -> Vec<Memory> {
    (0..n)
        .map(|i| {
            mock_memory_with(
                &format!("decay-{i}"),
                MemoryKind::Note,
                MemoryTier::Working,
                &format!("old note {i}"),
                0.3,
                0.2,
                false,
            )
        })
        .collect()
}

/// A 6-line VTT transcript. Two speakers, three cues each, well-formed.
pub fn mock_transcript_vtt() -> &'static str {
    r#"WEBVTT

00:00:01.000 --> 00:00:04.000
Alice: Hi, this is a test of the ingest pipeline.

00:00:05.000 --> 00:00:08.000
Bob: Sounds good. Let me know if anything breaks.

00:00:09.000 --> 00:00:12.000
Alice: Will do. I'll be on the next call.

00:00:13.000 --> 00:00:16.000
Bob: The cairn memory tier should be set to semantic for this.

00:00:17.000 --> 00:00:20.000
Alice: Agreed, this is a fact we want to keep.

00:00:21.000 --> 00:00:24.000
Bob: Done. End of transcript.
"#
}

/// An SRT transcript (no WEBVTT header, comma decimal separator).
pub fn mock_transcript_srt() -> &'static str {
    r#"1
00:00:01,000 --> 00:00:04,000
Alice: SRT line one.

2
00:00:05,000 --> 00:00:08,000
Bob: SRT line two.

3
00:00:09,000 --> 00:00:12,000
Alice: SRT line three.
"#
}

/// A JSON transcript (the format cairn-ingest accepts in `parse_json`).
pub fn mock_transcript_json() -> &'static str {
    r#"[
      {"start_ms": 1000, "end_ms": 4000, "speaker": "Alice", "text": "json line one"},
      {"start_ms": 5000, "end_ms": 8000, "speaker": "Bob",   "text": "json line two"}
    ]"#
}

/// A small Rust source blob for context-compression tests.
pub fn mock_rust_source() -> &'static str {
    r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn sub(a: i32, b: i32) -> i32 {
    a - b
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn add_works() { assert_eq!(add(1, 2), 3); }
    #[test]
    fn sub_works() { assert_eq!(sub(5, 2), 3); }
}
"#
}

/// Text that contains every redaction category cairn-share is supposed to
/// catch. Tests that the sanitizer finds *all* of them and the resulting
/// text contains none of the raw secrets.
pub fn mock_secret_heavy_text() -> &'static str {
    r#"
Contact: alice@example.com or bob@vellixia.io
Server: 10.0.0.42 (internal) — ssh into it with ssh://root@192.168.1.1
Home path: /home/alice/.ssh/id_rsa leaked via /Users/bob/Documents/key.pem
OpenAI: sk-abcdefghijklmnopqrstuvwxyz0123456789
GitHub: ghp_abcdefghijklmnopqrstuvwxyz0123456789
Slack: xoxb-EXAMPLE-0000000000-notarealslacksecrettoken
JWT: eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJhdXNlciJ9.signature_here
AWS: AKIAIOSFODNN7EXAMPLE
-----BEGIN OPENAI PRIVATE KEY BLOCK-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQ
-----END OPENAI PRIVATE KEY BLOCK-----
"#
}

/// `RerankConfig` with the default (disabled, provider "none") settings.
pub fn mock_rerank_config_default() -> RerankConfig {
    RerankConfig::default()
}

/// `RerankConfig` with a "local" provider and `enabled = true` (LocalReranker
/// path — without the `local` feature it falls back to NullReranker, which
/// the tests assert).
pub fn mock_rerank_config_local() -> RerankConfig {
    RerankConfig {
        enabled: true,
        provider: "local".into(),
        model: Some("Xenova/ms-marco-MiniLM-L-6-v2".into()),
        api_key: None,
        top_k: 5,
        blend_weight: 0.6,
    }
}

/// `LlmConsolidationConfig` with consolidation enabled and a placeholder URL
/// (no API key — tests that hit the consolidator get a network/disabled
/// result rather than a panic).
pub fn mock_llm_consolidation_enabled() -> LlmConsolidationConfig {
    LlmConsolidationConfig {
        enabled: true,
        url: "https://api.openai.com/v1/chat/completions".into(),
        model: "gpt-4o-mini".into(),
        api_key: None,
    }
}

/// `LlmConsolidationConfig` disabled (the production default).
pub fn mock_llm_consolidation_disabled() -> LlmConsolidationConfig {
    LlmConsolidationConfig {
        enabled: false,
        url: String::new(),
        model: String::new(),
        api_key: None,
    }
}

/// Concepts extracted from a piece of content. The split is intentionally
/// naive — tests are about wiring, not about NLP.
fn default_concepts(content: &str) -> Vec<String> {
    content
        .split_whitespace()
        .filter(|w| w.len() > 3)
        .take(8)
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}
