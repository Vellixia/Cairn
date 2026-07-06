//! Self-tuning promotion threshold (v0.8.0 Sprint 9, the `tune` cron job).
//!
//! `Config.promote_threshold` comes from `CAIRN_PROMOTE_THRESHOLD` and is otherwise fixed for
//! the life of the process. This module holds a small process-wide override that a weekly job
//! nudges based on `cairn_memory::FollowupTracker::followup_rate()` - the fraction of recalls
//! that got immediately re-queried with a disjoint result set, a proxy for "retrieval (and by
//! extension, whatever's been promoted into globally-recalled scope) isn't satisfying queries."
//! Gated behind `Config.selftune`; when it's off, [`tune`] is simply never called and
//! [`effective_promote_threshold`] always returns the configured value unchanged.
//!
//! Deliberately in-memory only, matching the Sprint 9 LLM budget tracker's precedent - it
//! resets to `Config.promote_threshold` on every server restart rather than being persisted.
//! A restart just means "resume from the configured baseline," never lost data or a crash.

use std::sync::{Mutex, OnceLock};

static OVERRIDE: OnceLock<Mutex<Option<f32>>> = OnceLock::new();

/// The threshold `run_auto_promote` should actually use this tick: the self-tuned override if
/// [`tune`] has ever set one, otherwise `configured` (`Config.promote_threshold`) unchanged.
pub fn effective_promote_threshold(configured: f32) -> f32 {
    let cell = OVERRIDE.get_or_init(|| Mutex::new(None));
    cell.lock()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or(configured)
}

const MIN_THRESHOLD: f32 = 0.5;
const MAX_THRESHOLD: f32 = 0.95;
const MAX_WEEKLY_STEP: f32 = 0.05;

/// A followup rate above this is read as "promotion is currently too generous" (raise the bar);
/// below this, as "promotion could afford to be a little more generous" (lower it slightly).
/// Anything in between is left alone - there's no strong enough signal to act on.
const HIGH_FOLLOWUP_RATE: f64 = 0.3;
const LOW_FOLLOWUP_RATE: f64 = 0.1;

/// Below this many observed queries, `FollowupTracker::followup_rate()` is noise, not signal -
/// most importantly, a brand-new tracker reports a rate of exactly `0.0` for "no data yet" in
/// the same way it would for "genuinely excellent recall," and those two must never be read the
/// same way (the first tells you nothing; the second says "lower the threshold"). The caller
/// (the `tune` cron job) is expected to check `FollowupTracker::queries` against this and skip
/// calling [`tune`] entirely below it.
pub const MIN_QUERIES_FOR_TUNING: u64 = 20;

/// The `tune` cron job body: nudge the effective threshold by at most `MAX_WEEKLY_STEP`,
/// clamped to `[MIN_THRESHOLD, MAX_THRESHOLD]`, based on `followup_rate`. Returns `(old, new)`
/// so the caller can log a human-readable line only when something actually changed.
pub fn tune(configured: f32, followup_rate: f64) -> (f32, f32) {
    let old = effective_promote_threshold(configured);
    let step = if followup_rate > HIGH_FOLLOWUP_RATE {
        MAX_WEEKLY_STEP
    } else if followup_rate < LOW_FOLLOWUP_RATE {
        -MAX_WEEKLY_STEP
    } else {
        0.0
    };
    let new = (old + step).clamp(MIN_THRESHOLD, MAX_THRESHOLD);
    if new != old {
        let cell = OVERRIDE.get_or_init(|| Mutex::new(None));
        *cell.lock().unwrap_or_else(|e| e.into_inner()) = Some(new);
    }
    (old, new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// `OVERRIDE` is a single process-wide static; serialize tests that touch it and always
    /// reset it afterwards so one test's tuning never leaks into another's assertions.
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn with_reset<T>(f: impl FnOnce() -> T) -> T {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cell = OVERRIDE.get_or_init(|| Mutex::new(None));
        *cell.lock().unwrap() = None;
        let result = f();
        *OVERRIDE.get().unwrap().lock().unwrap() = None;
        result
    }

    #[test]
    fn effective_threshold_is_the_configured_value_with_no_override() {
        with_reset(|| {
            assert_eq!(effective_promote_threshold(0.85), 0.85);
        });
    }

    #[test]
    fn high_followup_rate_raises_the_threshold() {
        with_reset(|| {
            let (old, new) = tune(0.85, 0.5);
            assert_eq!(old, 0.85);
            assert!((new - 0.90).abs() < 1e-6, "expected 0.90, got {new}");
            assert_eq!(effective_promote_threshold(0.85), new);
        });
    }

    #[test]
    fn low_followup_rate_lowers_the_threshold() {
        with_reset(|| {
            let (old, new) = tune(0.85, 0.02);
            assert_eq!(old, 0.85);
            assert!((new - 0.80).abs() < 1e-6, "expected 0.80, got {new}");
        });
    }

    #[test]
    fn mid_range_followup_rate_leaves_threshold_unchanged() {
        with_reset(|| {
            let (old, new) = tune(0.85, 0.2);
            assert_eq!(old, new);
        });
    }

    #[test]
    fn threshold_never_climbs_above_the_max() {
        with_reset(|| {
            let mut current = 0.85;
            for _ in 0..20 {
                let (_, new) = tune(current, 1.0);
                current = new;
            }
            assert!(current <= MAX_THRESHOLD);
        });
    }

    #[test]
    fn threshold_never_drops_below_the_min() {
        with_reset(|| {
            let mut current = 0.85;
            for _ in 0..20 {
                let (_, new) = tune(current, 0.0);
                current = new;
            }
            assert!(current >= MIN_THRESHOLD);
        });
    }

    #[test]
    fn tuning_is_cumulative_across_calls_via_the_override() {
        with_reset(|| {
            let (_, first) = tune(0.85, 0.5);
            // Second call ignores the (now-stale) `configured` argument in favor of the
            // override `tune` itself just set - mirrors calling the job again a week later
            // without a restart in between.
            let (old_second, second) = tune(0.85, 0.5);
            assert_eq!(old_second, first);
            assert!(second > first || second == MAX_THRESHOLD);
        });
    }
}
