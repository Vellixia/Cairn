//! Context ledger (P2.5). Tracks token utilisation against a configurable context
//! window to produce a "pressure" gauge for the dashboard.
//!
//! Each `record()` call adds (or updates) one entry. `pressure()` returns a snapshot
//! with: raw utilization, pinned/stale penalties, recommendation action, and the
//! top eviction candidates ranked by `phi`.
//!
//! Pinned entries are excluded from the eviction candidate list - the agent has
//! explicitly marked them as important.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Default context window (128k tokens, ~Claude Sonnet).
pub const DEFAULT_WINDOW_SIZE: usize = 128_000;

/// Pinned entries add this penalty to effective utilization (each).
const PINNED_PENALTY: f64 = 0.02;

/// Stale (unused for >24h) entries add this penalty each.
const STALE_PENALTY: f64 = 0.01;

/// How many hours before an entry counts as "stale".
const STALE_HOURS: i64 = 24;

/// One tracked entry in the context ledger.
#[derive(Debug, Clone, Serialize)]
pub struct LedgerEntry {
    pub path: String,
    pub mode: String,
    pub original_tokens: usize,
    pub sent_tokens: usize,
    pub timestamp: DateTime<Utc>,
    pub access_count: u32,
    pub phi: f64,
    pub pinned: bool,
}

/// Recommended action based on context pressure.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum PressureAction {
    NoAction,
    SuggestCompression,
    ForceCompression,
    EvictLeastRelevant,
}

/// Current pressure state.
#[derive(Debug, Clone, Serialize)]
pub struct ContextPressure {
    pub utilization: f64,
    pub remaining_tokens: usize,
    pub entries_count: usize,
    pub recommendation: PressureAction,
    pub eviction_candidates: Vec<LedgerEntry>,
    pub window_size: usize,
}

/// Tracks entries in the context window and computes pressure.
#[derive(Debug)]
pub struct ContextLedger {
    window_size: usize,
    entries: Vec<LedgerEntry>,
    pub total_tokens_sent: usize,
}

impl ContextLedger {
    pub fn new() -> Self {
        Self {
            window_size: DEFAULT_WINDOW_SIZE,
            entries: Vec::new(),
            total_tokens_sent: 0,
        }
    }

    pub fn with_window_size(window_size: usize) -> Self {
        Self {
            window_size,
            entries: Vec::new(),
            total_tokens_sent: 0,
        }
    }

    /// Record one context entry. Updates an existing entry (matched by path) or appends
    /// a new one. `pinned` marks the entry as exempt from eviction.
    pub fn record(
        &mut self,
        path: String,
        mode: String,
        original_tokens: usize,
        sent_tokens: usize,
        pinned: bool,
    ) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.path == path) {
            existing.mode = mode;
            existing.original_tokens = original_tokens;
            existing.sent_tokens = sent_tokens;
            existing.timestamp = Utc::now();
            existing.access_count += 1;
            existing.pinned = pinned;
            existing.phi = compute_phi(existing);
        } else {
            let mut entry = LedgerEntry {
                path,
                mode,
                original_tokens,
                sent_tokens,
                timestamp: Utc::now(),
                access_count: 1,
                phi: 0.5,
                pinned,
            };
            entry.phi = compute_phi(&entry);
            self.entries.push(entry);
        }
        self.total_tokens_sent = self.entries.iter().map(|e| e.sent_tokens).sum();
    }

    /// Total unique paths tracked.
    pub fn entries_count(&self) -> usize {
        self.entries.len()
    }

    /// Compute current context pressure.
    pub fn pressure(&self) -> ContextPressure {
        let now = Utc::now();
        let raw_utilization = if self.window_size > 0 {
            self.total_tokens_sent as f64 / self.window_size as f64
        } else {
            0.0
        };

        let pinned_count = self.entries.iter().filter(|e| e.pinned).count() as f64;
        let stale_count = self
            .entries
            .iter()
            .filter(|e| {
                let age = (now - e.timestamp).num_hours();
                age > STALE_HOURS && !e.pinned
            })
            .count() as f64;

        let effective =
            (raw_utilization + pinned_count * PINNED_PENALTY + stale_count * STALE_PENALTY)
                .min(1.0);

        let recommendation = if effective > 0.90 {
            PressureAction::EvictLeastRelevant
        } else if effective > 0.75 {
            PressureAction::ForceCompression
        } else if effective > 0.50 {
            PressureAction::SuggestCompression
        } else {
            PressureAction::NoAction
        };

        // Candidates for eviction: non-pinned, sorted by phi ascending (low phi = evict first)
        let mut candidates: Vec<LedgerEntry> =
            self.entries.iter().filter(|e| !e.pinned).cloned().collect();
        candidates.sort_by(|a, b| {
            a.phi
                .partial_cmp(&b.phi)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates.truncate(5);

        ContextPressure {
            utilization: effective,
            remaining_tokens: self.window_size.saturating_sub(self.total_tokens_sent),
            entries_count: self.entries.len(),
            recommendation,
            eviction_candidates: candidates,
            window_size: self.window_size,
        }
    }
}

impl Default for ContextLedger {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute phi value for an entry. Lightweight version: token cost weighted + access
/// history boost + small base relevance. Higher phi = more relevant (don't evict).
fn compute_phi(entry: &LedgerEntry) -> f64 {
    let cost_factor = (entry.sent_tokens as f64 / 1000.0_f64.max(1.0)).min(1.0);
    let access_factor = (entry.access_count as f64 * 0.1).min(1.0);
    // High token cost = lower phi (good eviction candidate).
    // High access count = higher phi (keep).
    let phi = (1.0 - cost_factor * 0.5) * 0.5 + access_factor * 0.5;
    phi.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pressure_at_quarter() {
        let mut ledger = ContextLedger::with_window_size(128_000);
        ledger.record("src/lib.rs".into(), "full".into(), 32000, 32000, false);
        let p = ledger.pressure();
        // 32000 / 128000 = 0.25
        assert!((p.utilization - 0.25).abs() < 0.01);
        assert_eq!(p.recommendation, PressureAction::NoAction);
    }

    #[test]
    fn test_high_pressure_recommends_eviction() {
        let mut ledger = ContextLedger::with_window_size(1000);
        for i in 0..10 {
            ledger.record(format!("file{}.rs", i), "full".into(), 100, 100, false);
        }
        let p = ledger.pressure();
        assert!(p.utilization > 0.9);
        assert_eq!(p.recommendation, PressureAction::EvictLeastRelevant);
    }

    #[test]
    fn test_pinned_entries_excluded_from_eviction() {
        let mut ledger = ContextLedger::with_window_size(1000);
        ledger.record("pinned.rs".into(), "full".into(), 100, 100, true);
        ledger.record("normal.rs".into(), "full".into(), 100, 100, false);
        let candidates = ledger.pressure().eviction_candidates;
        assert!(!candidates.iter().any(|e| e.pinned));
    }

    #[test]
    fn test_phi_ranks_eviction_candidates() {
        let mut ledger = ContextLedger::with_window_size(2000);
        // Big unused file (low phi)
        ledger.record("big_unused.rs".into(), "full".into(), 500, 500, false);
        // Small used file (high phi via repeated access)
        ledger.record("small_used.rs".into(), "full".into(), 10, 10, false);
        ledger.record("small_used.rs".into(), "full".into(), 10, 10, false);
        ledger.record("small_used.rs".into(), "full".into(), 10, 10, false);
        let candidates = ledger.pressure().eviction_candidates;
        assert_eq!(candidates[0].path, "big_unused.rs");
    }

    #[test]
    fn test_record_updates_existing() {
        let mut ledger = ContextLedger::with_window_size(1000);
        ledger.record("foo.rs".into(), "full".into(), 100, 100, false);
        ledger.record("foo.rs".into(), "full".into(), 100, 100, false);
        assert_eq!(ledger.entries_count(), 1);
    }

    #[test]
    fn test_force_compression_threshold() {
        let mut ledger = ContextLedger::with_window_size(100);
        ledger.record("a.rs".into(), "full".into(), 80, 80, false);
        let p = ledger.pressure();
        // 0.80 raw, no stale/pinned yet, > 0.75
        assert_eq!(p.recommendation, PressureAction::ForceCompression);
    }

    #[test]
    fn test_suggest_compression_threshold() {
        let mut ledger = ContextLedger::with_window_size(100);
        ledger.record("a.rs".into(), "full".into(), 60, 60, false);
        let p = ledger.pressure();
        assert_eq!(p.recommendation, PressureAction::SuggestCompression);
    }

    #[test]
    fn test_zero_window_safe() {
        let mut ledger = ContextLedger::with_window_size(0);
        ledger.record("a.rs".into(), "full".into(), 100, 100, false);
        let p = ledger.pressure();
        // 0 window → 0 utilization (safe default, not panic)
        assert_eq!(p.utilization, 0.0);
    }
}
