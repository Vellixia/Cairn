//! Local business-flow integration tests.
//!
//! This crate is a workspace member that exists only to host hermetic
//! integration tests. Every `tests/<topic>.rs` file is an independent
//! `cargo test` binary, run by `cargo test -p cairn-tests` (or
//! `cargo test --workspace`).
//!
//! ## Hard boundary
//!
//! - No network calls.
//! - No live database / docker / external services.
//! - Every test calls a real Cairn crate function against a real
//!   `cairn_store::Store::open_in_memory()` instance. Hand-coded JSON
//!   literals and re-implementations of functions already in the crate
//!   are explicitly rejected - a tautological test that never touches
//!   a cairn crate will pass even when the production code is broken.
//!
//! ## Coverage
//!
//! 17 test files, 134 tests:
//! - `01_memory_tiers`: followup/gotcha trackers, activity heatmap,
//!   architecture report, serde round-trip for `NewMemory`.
//! - `04_rerank`: NullReranker, `from_config` fallback chain, alpha-blend
//!   redaction, end-to-end `MemoryEngine::hybrid_search_with_rerank`.
//! - `05_guardrails`: real `Guard::{verify_edit, set_anchor, anchor}` against
//!   the in-memory store (clean / large-deletion / suspicious-anchor paths).
//! - `06_shell_profiles`: `cairn_shell::{compress_output, find_match, REGISTRY}`.
//! - `07_share`: `cairn_share::Sanitizer` (all secret kinds, sensitivity).
//! - `08_pack_registry`: `cairn_pack` Ed25519 sign/verify, `cairn_registry::TrustScope`.
//! - `09_session`: `cairn_session::SessionStore` save/load + drift lifecycle.
//! - `10_sync_crypto`: `cairn_sync::{GCounter, ORSet, VectorClock}` + AEAD crypto round-trip.
//! - `12_proactive`: `cairn_proactive::intent::classify`.
//! - `13_ingest`: `cairn_ingest::{parse_vtt, parse_srt, parse_json}`.
//! - `16_config`: `Config::resolve(None)` env-driven, `OrgId` validation.
//! - `17_workspace_invariants`: workspace member list, tilde constraints.
//! - `18_context_engine`: real `ContextEngine` over `Store::open_in_memory`
//!   - Full / Cached / Diff / Outline / anti-inflation / auto-delta.
//! - `19_memory_engine`: real `MemoryEngine` - remember dedup, recall
//!   ranking, hybrid_search, gotcha promotion, crystallize, consolidate.
//! - `20_assembler`: real `Assembler::assemble` - budget enforcement,
//!   dropped items, JSON shape.
//! - `22_mcp_dispatch`: real `McpServer::dispatch` - tool list,
//!   remember/recall round-trip, assemble, sanitize, unknown-tool error.
//! - `23_api_envelope`: real `cairn_api::router` mounted via
//!   `tower::ServiceExt::oneshot` - /api/health, /api/capabilities,
//!   /api/openapi.json, /api/stats (auth-gated), 404 envelope.
//!
//! Add a new flow by dropping a `tests/<NN>_<topic>.rs` file. Cargo
//! discovers it automatically. Tests must exercise a real Cairn crate
//! API; pure JSON-shape asserts are not accepted.

pub mod fixtures;
pub mod mock_store;
