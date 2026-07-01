//! Benchmark harness + fixtures (v0.5.0 Sprint 16).
//!
//! Three classes of benchmark live here:
//!
//! - **LongMemEval / LoCoMo** - synthetic recall tasks. We hand-build a small
//!   fixture set that captures the *shape* of those benchmarks (multi-session
//!   memories with entity-resolution and temporal questions). The numbers we
//!   publish in `docs/testing/benchmarks.md` are from this fixture, not the full
//!   external benchmark (we don't redistribute that data here).
//!
//! - **Task-success horizon** - runs a synthetic task pipeline at increasing
//!   horizons (10 / 25 / 50 / 100 steps) and measures how often Cairn's
//!   `assemble` produces a context that still includes the relevant memory
//!   before the context is over-full.
//!
//! - **Smart memory retention** - measures how Cairn's `confidence` field +
//!   `pin` + `crystallize` together preserve "important" memories across many
//!   `reinforce` cycles vs. naive LRU eviction.
//!
//! All three produce a [`BenchResult`] that the harness can serialize to JSON
//! for CI ingestion and `docs/testing/benchmarks.md` rendering. See ADR-023 for the
//! methodology choices (why we hand-build fixtures rather than redistribute the
//! external benchmarks verbatim).

pub mod fixture;
pub mod horizon;
pub mod longmemeval;
pub mod retention;

pub use fixture::Fixture;
pub use horizon::{HorizonBenchmark, HorizonStep};
pub use longmemeval::{LongMemEvalBenchmark, LongMemEvalResult};
pub use retention::{RetentionBenchmark, RetentionResult};

use serde::{Deserialize, Serialize};

/// Top-level result type. Each benchmark returns one of these with its own `kind`.
/// The harness writes JSON of this shape to `target/benchmarks/<name>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResult {
    pub name: String,
    pub kind: BenchKind,
    /// Wall-clock duration of the benchmark, in milliseconds.
    pub duration_ms: u64,
    /// Free-form metadata (commit hash, host OS, dataset version, etc.). Helps
    /// reproduce and compare across runs.
    pub meta: BTreeMap<String, String>,
    /// The kind-specific payload.
    pub data: BenchData,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BenchKind {
    LongMemEval,
    Horizon,
    Retention,
}

/// Tagged payload - exactly one variant per benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BenchData {
    LongMemEval(LongMemEvalResult),
    Horizon(horizon::HorizonOutput),
    Retention(retention::RetentionOutput),
}

impl BenchResult {
    pub fn new(name: impl Into<String>, kind: BenchKind, duration_ms: u64) -> Self {
        Self {
            name: name.into(),
            kind,
            duration_ms,
            meta: BTreeMap::new(),
            data: match kind {
                BenchKind::LongMemEval => BenchData::LongMemEval(LongMemEvalResult::default()),
                BenchKind::Horizon => BenchData::Horizon(horizon::HorizonOutput::default()),
                BenchKind::Retention => BenchData::Retention(retention::RetentionOutput::default()),
            },
        }
    }

    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.meta.insert(key.into(), value.into());
        self
    }
}

use std::collections::BTreeMap;
