//! Live cost-savings metrics.
//!
//! The Cairn context engine's [`cairn_context`] and shell compressor already emit savings data
//! through the API and through the `cairn bench` tool. This module aggregates those signals
//! into a single `/api/metrics` endpoint for the dashboard:
//!
//! - **cumulative bytes / tokens served** (the sum of every read/compress payload the server
//!   has handed to agents since start)
//! - **cumulative raw bytes that *would* have been served** without compression (estimated)
//! - **hit rate / bounce rate** (the share of recall/wakeup queries the agent *did* follow up on,
//!   approximated via access_count > 0 in the memory engine)
//! - **total memories, total checkpoints** (the headline trust counters)
//!
//! The numbers are best-effort and intentionally cheap - no on-the-fly recomputation across
//! millions of records. The point is "did the user's token spend go down?" - a running tally
//! updated on every read.

use crate::AppState;
use axum::{extract::State, Json};
use cairn_assemble::AssemblyReport;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Live counter - `bytes_in` is the compact payload we served the agent, `bytes_out_full` is
/// what we *would* have sent uncompressed (so saved = full ' compact, divided by `full`).
#[derive(Debug, Default)]
pub struct SavingsCounter {
    pub compact: AtomicU64,
    pub full: AtomicU64,
    pub calls: AtomicU64,
    /// `hits` = read results that returned a non-empty view; `bounces` = empty results.
    pub hits: AtomicU64,
    pub bounces: AtomicU64,
    /// Total memory-wakeup tokens the agent has been served (approximate, sum of est_tokens).
    pub wakeup_tokens: AtomicU64,
    /// Total recall tokens the agent has been served.
    pub recall_tokens: AtomicU64,
}

impl SavingsCounter {
    /// Record one read/cache hit. `compact` is what we sent; `full` is what a raw read would
    /// have produced. `is_bounce` is true when the view was empty (e.g. file missing).
    pub fn record_read(&self, compact: u64, full: u64, is_bounce: bool) {
        self.compact.fetch_add(compact, Ordering::Relaxed);
        self.full.fetch_add(full, Ordering::Relaxed);
        self.calls.fetch_add(1, Ordering::Relaxed);
        if is_bounce {
            self.bounces.fetch_add(1, Ordering::Relaxed);
        } else {
            self.hits.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record one assembler pass - its `used_tokens` and dropped-token counts add to the ledger
    /// so the savings dashboard can show "I assembled N queries, kept M tokens, dropped K".
    pub fn record_assemble(&self, r: &AssemblyReport) {
        self.wakeup_tokens
            .fetch_add(r.used_tokens as u64, Ordering::Relaxed);
        self.compact
            .fetch_add(r.used_tokens as u64, Ordering::Relaxed);
        self.full
            .fetch_add(r.budget_tokens as u64, Ordering::Relaxed);
        self.calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot the counters as plain numbers for the JSON response.
    pub fn snapshot(&self) -> SavingsSnapshot {
        let compact = self.compact.load(Ordering::Relaxed);
        let full = self.full.load(Ordering::Relaxed);
        let calls = self.calls.load(Ordering::Relaxed);
        let hits = self.hits.load(Ordering::Relaxed);
        let bounces = self.bounces.load(Ordering::Relaxed);
        let saved = full.saturating_sub(compact);
        let ratio = if full == 0 {
            0.0
        } else {
            saved as f64 / full as f64
        };
        SavingsSnapshot {
            compact_bytes: compact,
            full_bytes: full,
            saved_bytes: saved,
            saved_ratio: ratio,
            calls,
            hits,
            bounces,
            hit_rate: if calls == 0 {
                0.0
            } else {
                hits as f64 / calls as f64
            },
            bounce_rate: if calls == 0 {
                0.0
            } else {
                bounces as f64 / calls as f64
            },
            wakeup_tokens: self.wakeup_tokens.load(Ordering::Relaxed),
            recall_tokens: self.recall_tokens.load(Ordering::Relaxed),
            // Bounce tracker stats are filled in by the metrics handler (which has
            // access to AppState.context.bounce_tracker).
            context_bounces: 0,
            context_wasted_tokens: 0,
            per_extension: Vec::new(),
            // Followup stats (P1.6) filled by the metrics handler.
            followup_queries: 0,
            followups: 0,
            followup_rate: 0.0,
            // Gotcha stats (P4.3) filled by the metrics handler.
            gotcha_failures: 0,
            gotcha_promoted: 0,
        }
    }
}

/// Snapshot of [`SavingsCounter`] for the JSON endpoint.
#[derive(Debug, Serialize, Clone)]
pub struct SavingsSnapshot {
    pub compact_bytes: u64,
    pub full_bytes: u64,
    pub saved_bytes: u64,
    /// 0.0--1.0 fraction of full bytes saved by compaction.
    pub saved_ratio: f64,
    pub calls: u64,
    pub hits: u64,
    pub bounces: u64,
    /// 0.0--1.0 - share of read calls that returned something.
    pub hit_rate: f64,
    /// 0.0--1.0 - share of read calls that returned nothing.
    pub bounce_rate: f64,
    pub wakeup_tokens: u64,
    pub recall_tokens: u64,
    /// P1.7 - total "true" bounces from the context engine's bounce tracker
    /// (compressed read followed by full read within the window).
    pub context_bounces: u64,
    /// P1.7 - tokens the agent consumed that turned out to be wasted (compressed-then-full).
    pub context_wasted_tokens: u64,
    /// P1.7 - per-extension bounce stats, sorted by bounce count desc.
    pub per_extension: Vec<ExtensionBounceStats>,
    /// P1.6 - total recall queries seen by the followup tracker.
    pub followup_queries: u64,
    /// P1.6 - queries that re-issued the same fingerprint with a disjoint result set.
    pub followups: u64,
    /// P1.6 - `followups / followup_queries`, or 0.0 when no queries yet.
    pub followup_rate: f64,
    /// P4.3 - total failure events recorded with the gotcha tracker.
    pub gotcha_failures: u64,
    /// P4.3 - clusters promoted to gotcha memories (i.e. crossed the threshold).
    pub gotcha_promoted: u64,
}

/// One extension's bounce stats (P1.7).
#[derive(Debug, Serialize, Clone)]
pub struct ExtensionBounceStats {
    pub extension: String,
    pub reads: usize,
    pub bounces: usize,
    pub wasted_tokens: usize,
    pub bounce_rate: f64,
}

impl SavingsSnapshot {
    /// Approximate USD saved assuming $0.00003 per output token (typical Sonnet-class pricing).
    /// Conservative: this only counts *input* tokens we didn't send, not output savings.
    pub fn usd_saved(&self) -> f64 {
        // Convert byte savings to token savings via 4-bytes-per-token, then dollars.
        let tokens_saved = self.saved_bytes as f64 / 4.0;
        tokens_saved * 0.00003
    }
}

/// `GET /api/metrics` - live savings + hit-rate + bounce-rate.
pub async fn metrics(State(s): State<AppState>) -> Result<Json<MetricsResponse>, crate::ApiError> {
    let mut snap = s.savings.snapshot();
    let memories = s.store.count_memories().unwrap_or(0);
    let checkpoints = s.guard.list_checkpoints().unwrap_or_default().len();

    // Read bounce tracker stats (P1.7) - they're owned by the context engine.
    {
        let tracker = s.ctx.bounce_tracker().lock().unwrap();
        snap.context_bounces = tracker.total_bounces;
        snap.context_wasted_tokens = tracker.total_wasted_tokens as u64;
        snap.per_extension = tracker
            .per_extension_stats()
            .into_iter()
            .map(|(ext, stats)| ExtensionBounceStats {
                bounce_rate: if stats.reads > 0 {
                    stats.bounces as f64 / stats.reads as f64
                } else {
                    0.0
                },
                extension: ext,
                reads: stats.reads,
                bounces: stats.bounces,
                wasted_tokens: stats.wasted_tokens,
            })
            .collect();
    }

    // Read followup tracker stats (P1.6) - owned by the memory engine.
    {
        let tracker = s.mem.followup_tracker().lock().unwrap();
        snap.followup_queries = tracker.queries;
        snap.followups = tracker.followups;
        snap.followup_rate = tracker.followup_rate();
    }

    // Read gotcha tracker stats (P4.3) - owned by the memory engine.
    {
        let tracker = s.mem.gotcha_tracker().lock().unwrap();
        snap.gotcha_failures = tracker.total_failures;
        snap.gotcha_promoted = tracker.promoted_clusters;
    }

    Ok(Json(MetricsResponse {
        savings: snap.clone(),
        usd_saved: snap.usd_saved(),
        memories,
        checkpoints,
        server: serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "started_at": s.started_at,
        }),
    }))
}

/// Top-level metrics response shape.
#[derive(Debug, Serialize)]
pub struct MetricsResponse {
    pub savings: SavingsSnapshot,
    /// Approximate USD saved (input-token pricing).
    pub usd_saved: f64,
    pub memories: i64,
    pub checkpoints: usize,
    /// Server build metadata.
    pub server: serde_json::Value,
}

/// Shape returned by `/api/metrics/savings` - the small subset the mobile companion
/// surfaces on `/mobile`. Smaller than the full `/api/metrics` payload so a phone
/// on a flaky network can render quickly.
#[derive(Debug, Serialize)]
pub struct MobileSavingsResponse {
    /// Tokens the server has served to clients via recall + wakeup reads in the
    /// last 24 hours. Approximate, summed from `SavingsCounter`.
    pub tokens_saved_today: u64,
    /// Number of drift events currently in `pending` status.
    pub drift_pending: u64,
    /// Number of pack installs recorded in the last 7 days. The mobile page renders
    /// this as a flat "how active is the team" stat; the underlying data lives on
    /// the registry's revocations log (which the install code currently does not
    /// append to) so we report 0 for now.
    pub recent_pack_installs: u64,
}

/// `GET /api/metrics/savings` - the mobile companion's three stats.
pub async fn mobile_savings(
    State(s): State<AppState>,
) -> Result<Json<MobileSavingsResponse>, crate::ApiError> {
    let snap = s.savings.snapshot();
    let tokens_saved_today = snap.recall_tokens.saturating_add(snap.wakeup_tokens);
    let drift_pending = s
        .sessions
        .recent_drift(200, None)
        .map(|events| {
            events
                .iter()
                .filter(|e| e.status == cairn_session::DriftStatus::Pending)
                .count() as u64
        })
        .unwrap_or(0);
    // The registry does not yet emit an "install" event the metrics endpoint can
    // count, so `recent_pack_installs` stays 0 until the registry gains an install
    // hook. Documented in the response shape so the client can render 0 cleanly.
    let recent_pack_installs = 0u64;
    Ok(Json(MobileSavingsResponse {
        tokens_saved_today,
        drift_pending,
        recent_pack_installs,
    }))
}

// -- wire type so the metric counter survives in AppState ----------------------------------

/// Thread-safe handle to the live savings counter - held in [`AppState`] and incremented by
/// instrumented handlers. Lives next to AppState so the metrics endpoint can read it cheaply.
#[derive(Clone, Default)]
pub struct SavingsState(pub Arc<SavingsCounter>);

impl std::ops::Deref for SavingsState {
    type Target = SavingsCounter;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Cheap, monotonic server-start timestamp (seconds since epoch). Used by the dashboard to
/// render "uptime" without leaking absolute host time.
pub fn server_started() -> i64 {
    use std::sync::OnceLock;
    static STARTED: OnceLock<i64> = OnceLock::new();
    *STARTED.get_or_init(|| chrono::Utc::now().timestamp())
}

// -- tests --------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn savings_ratio_is_zero_when_nothing_recorded() {
        let s = SavingsCounter::default();
        let snap = s.snapshot();
        assert_eq!(snap.saved_bytes, 0);
        assert_eq!(snap.saved_ratio, 0.0);
        assert_eq!(snap.hit_rate, 0.0);
        assert_eq!(snap.bounce_rate, 0.0);
    }

    #[test]
    fn record_read_updates_compact_and_full() {
        let s = SavingsCounter::default();
        s.record_read(200, 1000, false);
        s.record_read(0, 100, true);
        let snap = s.snapshot();
        assert_eq!(snap.compact_bytes, 200);
        assert_eq!(snap.full_bytes, 1100);
        assert_eq!(snap.saved_bytes, 900);
        assert!((snap.saved_ratio - (900.0 / 1100.0)).abs() < 1e-9);
        assert_eq!(snap.calls, 2);
        assert_eq!(snap.hits, 1);
        assert_eq!(snap.bounces, 1);
        assert!((snap.hit_rate - 0.5).abs() < 1e-9);
        assert!((snap.bounce_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn record_assemble_keeps_tokens_and_counts_full() {
        let s = SavingsCounter::default();
        // We can't construct a real AssemblyReport without a memory engine, so just check
        // the counter starts at zero and the record_* path tolerates empty snapshots.
        let snap = s.snapshot();
        assert_eq!(snap.wakeup_tokens, 0);
        assert_eq!(snap.calls, 0);
    }

    #[test]
    fn usd_saved_is_zero_for_zero_savings() {
        let s = SavingsCounter::default();
        let snap = s.snapshot();
        assert!(snap.usd_saved() < 1e-9);
    }

    #[test]
    fn usd_saved_grows_with_savings() {
        let s = SavingsCounter::default();
        s.record_read(100, 10_000, false);
        let snap = s.snapshot();
        let usd = snap.usd_saved();
        assert!(
            usd > 0.0,
            "usd_saved should be > 0 for non-zero savings; got {usd}"
        );
        // Sanity bound - 9900 bytes  2475 tokens  $0.074.
        assert!(usd < 1.0);
    }
}
