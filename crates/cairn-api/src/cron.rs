//! In-process cron scheduler (v0.8.0 Sprint 4).
//!
//! Runs a small fixed set of background jobs on a schedule via `tokio-cron-scheduler`
//! (in-memory job store - no external dependency beyond the process itself, no default
//! features enabled). Every job also has a manual trigger (`POST /api/cron/run/:job` in
//! `lib.rs`) so an operator (or a test) doesn't have to wait for the schedule to fire; both
//! paths go through [`run_job_now`] and land in the same history.
//!
//! **Schedule syntax note:** `tokio-cron-scheduler` uses a 6-field cron expression with a
//! *leading seconds* field (`sec min hour day month weekday`), not the more common 5-field
//! `min hour day month weekday` form - a plain `"0 2 * * *"` would mean "every day at
//! 00:02:00", not 2am.

use crate::{selftune, AppState};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashSet;
use std::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};

/// The fixed set of background jobs and their (6-field, seconds-first) cron schedules.
pub const JOBS: &[(&str, &str)] = &[
    ("session-gc", "0 0 2 * * *"),        // daily at 02:00
    ("memory-decay", "0 0 3 * * Sun"),    // weekly, Sunday 03:00
    ("access-log-prune", "0 0 4 1 * *"),  // monthly, 1st at 04:00
    ("llm-intelligence", "0 30 3 * * *"), // daily at 03:30 (v0.8.0 Sprint 5)
    ("memory-demote", "0 0 4 * * *"),     // daily at 04:00 (v0.8.0 Sprint 8)
    ("tune", "0 0 5 * * Sun"),            // weekly, Sunday 05:00 (v0.8.0 Sprint 9)
];

const MAX_HISTORY_PER_JOB: usize = 10;

/// One completed (or failed) cron run, kept for `GET /api/cron/history`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CronRun {
    pub job: &'static str,
    pub started_at: DateTime<Utc>,
    pub duration_ms: i64,
    pub outcome: &'static str, // "ok" | "err"
    pub detail: String,
}

/// A bounded ring of recent runs, capped per-job so one noisy job can't push another's
/// history out, plus (v0.8.0 Sprint 9) the single-flight bookkeeping for [`run_job_now`].
/// Two independent `Mutex`es - jobs tick at most a few times a day, so lock contention is a
/// non-issue either way.
#[derive(Default)]
pub struct CronHistory {
    runs: Mutex<Vec<CronRun>>,
    running: Mutex<HashSet<&'static str>>,
}

impl CronHistory {
    pub fn record(&self, run: CronRun) {
        let mut g = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        g.push(run);
        for name in JOBS.iter().map(|(n, _)| *n) {
            while g.iter().filter(|r| r.job == name).count() > MAX_HISTORY_PER_JOB {
                if let Some(pos) = g.iter().position(|r| r.job == name) {
                    g.remove(pos);
                }
            }
        }
    }

    /// Every kept run, optionally filtered to one job, newest last.
    pub fn recent(&self, job: Option<&str>) -> Vec<CronRun> {
        let g = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        g.iter()
            .filter(|r| job.is_none_or(|j| r.job == j))
            .cloned()
            .collect()
    }

    /// The most recent run of `job`, if it has ever run.
    pub fn last_run(&self, job: &str) -> Option<CronRun> {
        let g = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        g.iter().rev().find(|r| r.job == job).cloned()
    }

    /// v0.8.0 Sprint 9 single-flight guard: reserves `job`'s running slot. Returns `true` if it
    /// wasn't already taken (the caller now owns it and must release it via [`Self::finish`]),
    /// `false` if another run of the same job is already in flight.
    fn try_start(&self, job: &'static str) -> bool {
        let mut g = self.running.lock().unwrap_or_else(|e| e.into_inner());
        g.insert(job)
    }

    /// Release a single-flight slot acquired via [`Self::try_start`].
    fn finish(&self, job: &str) {
        let mut g = self.running.lock().unwrap_or_else(|e| e.into_inner());
        g.remove(job);
    }

    /// `true` while `job` has an in-flight run, for `GET /api/cron/health`.
    pub fn is_running(&self, job: &str) -> bool {
        let g = self.running.lock().unwrap_or_else(|e| e.into_inner());
        g.contains(job)
    }

    /// v0.8.0 Sprint 9: `true` once `job` has gone more than twice its own nominal period
    /// (see [`nominal_period`]) since its last recorded run - long enough overdue that the
    /// scheduler having silently died is a more likely explanation than this one tick just
    /// running late. A job that has never run at all is never stale; there's nothing wrong
    /// with a freshly started server waiting for its first scheduled tick.
    pub fn is_stale(&self, job: &str, now: DateTime<Utc>) -> bool {
        match self.last_run(job) {
            Some(run) => now - run.started_at > nominal_period(job) * 2,
            None => false,
        }
    }
}

/// The interval each job is expected to run at, used only for [`CronHistory::is_stale`] - not
/// consulted for scheduling itself (`tokio_cron_scheduler` owns that, from `JOBS`' own cron
/// strings). Jobs not listed here are assumed daily, which covers the other three of today's
/// six real jobs (`session-gc`, `llm-intelligence`, `memory-demote`).
fn nominal_period(job: &str) -> Duration {
    match job {
        "memory-decay" | "tune" => Duration::days(7),
        "access-log-prune" => Duration::days(31),
        _ => Duration::days(1),
    }
}

/// RAII release for [`CronHistory::try_start`] - guarantees the slot is freed even if a job
/// body were to panic, so one bad run can never wedge a job as "running" forever.
struct RunningGuard<'a> {
    history: &'a CronHistory,
    job: &'static str,
}

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.history.finish(self.job);
    }
}

/// Run one named job synchronously and record the outcome. Shared by the scheduler's own
/// ticks and the manual-trigger HTTP handler, so both paths behave identically and both show
/// up in the same history. Returns `Err` only for an unrecognized job name - a job's own
/// failure is captured as an `"err"`-outcome `CronRun`, not a Rust `Err`, so a bad run never
/// panics the scheduler.
pub fn run_job_now(state: &AppState, job: &'static str) -> Result<CronRun, String> {
    if !JOBS.iter().any(|(name, _)| *name == job) {
        return Err(format!("unknown job: {job}"));
    }
    // v0.8.0 Sprint 9: single-flight - a scheduler tick landing on top of a still-running
    // previous tick (or a manual `POST /api/cron/run/:job` racing the scheduler on another
    // thread) would otherwise run the same job's body twice concurrently. Skip instead, but
    // still record it so it's visible in `GET /api/cron/history` under its own outcome rather
    // than being silently swallowed or confused with a real success.
    if !state.cron_history.try_start(job) {
        let run = CronRun {
            job,
            started_at: Utc::now(),
            duration_ms: 0,
            outcome: "skipped",
            detail: "already running - single-flight guard".to_string(),
        };
        state.cron_history.record(run.clone());
        crate::events::publish_cron(&state.events, run.job, run.outcome);
        tracing::warn!(
            job = job,
            "cron tick skipped - previous run still in flight"
        );
        return Ok(run);
    }
    let _guard = RunningGuard {
        history: &state.cron_history,
        job,
    };
    let started_at = Utc::now();
    let t0 = std::time::Instant::now();
    let (outcome, detail): (&'static str, String) = match job {
        "session-gc" => match state.mem.run_session_gc(state.cfg.session_ttl_days) {
            Ok(n) => (
                "ok",
                format!("promoted {n} session-scoped memories to global"),
            ),
            Err(e) => ("err", e.to_string()),
        },
        "memory-decay" => {
            // v0.8.0 Sprint 9: memory hygiene rides along on the same daily tick as decay
            // rather than getting its own job - all three are cheap full-corpus scans over
            // the same data, and there's no benefit to a separate schedule for them.
            let decayed = state.mem.run_decay(state.cfg.decay_period_days);
            let deduped = state.mem.run_dedup_sweep();
            let capped = state
                .mem
                .run_working_tier_cap(state.cfg.max_working_per_project);
            match (decayed, deduped, capped) {
                (Ok(d), Ok(x), Ok(c)) => (
                    "ok",
                    format!(
                        "decayed confidence on {d} memories, deduped {x}, \
                         capped {c} over-quota working memories"
                    ),
                ),
                (d, x, c) => (
                    "err",
                    format!("decay={d:?} dedup_sweep={x:?} working_tier_cap={c:?}"),
                ),
            }
        }
        "access-log-prune" => {
            let cutoff = Utc::now() - Duration::days(state.cfg.access_log_retention_days as i64);
            match state.store.prune_access_log_before(cutoff) {
                Ok(n) => (
                    "ok",
                    format!(
                        "pruned {n} access_log rows older than {} days",
                        state.cfg.access_log_retention_days
                    ),
                ),
                Err(e) => ("err", e.to_string()),
            }
        }
        "llm-intelligence" => {
            if !state.cfg.llm_consolidation.enabled {
                (
                    "ok",
                    "skipped - CAIRN_LLM_CONSOLIDATION disabled".to_string(),
                )
            } else {
                let budget = state.cfg.llm_daily_budget;
                let concepts = state
                    .mem
                    .run_concept_extraction(&state.cfg.llm_consolidation, budget);
                let contradictions = state
                    .mem
                    .run_contradiction_detection(&state.cfg.llm_consolidation, budget);
                let scored = state
                    .mem
                    .run_promotion_scoring(&state.cfg.llm_consolidation, budget);
                // v0.8.0 Sprint 8: full-auto promotion runs last, on the scores this same tick
                // just computed - a memory that clears CAIRN_PROMOTE_THRESHOLD is promoted
                // immediately rather than waiting for a human to notice it in the candidates list.
                // v0.8.0 Sprint 9: the threshold itself may have been nudged by the `tune` job
                // since the server started - `effective_promote_threshold` returns that override
                // when one exists, or `state.cfg.promote_threshold` unchanged otherwise.
                let threshold = selftune::effective_promote_threshold(state.cfg.promote_threshold);
                let promoted = state.mem.run_auto_promote(threshold);
                match (concepts, contradictions, scored, promoted) {
                    (Ok(c), Ok(x), Ok(s), Ok(p)) => (
                        "ok",
                        format!(
                            "extracted concepts on {c} memories, resolved/flagged {x} \
                             contradictions, scored {s} promotion candidates, auto-promoted {p}"
                        ),
                    ),
                    (c, x, s, p) => (
                        "err",
                        format!(
                            "concept_extraction={c:?} contradiction_detection={x:?} \
                             promotion_scoring={s:?} auto_promote={p:?}"
                        ),
                    ),
                }
            }
        }
        "memory-demote" => match state.mem.run_auto_demote(state.cfg.demote_idle_days) {
            Ok(n) => ("ok", format!("demoted {n} stale auto-promoted memories")),
            Err(e) => ("err", e.to_string()),
        },
        "tune" => {
            if !state.cfg.selftune {
                ("ok", "skipped - CAIRN_SELFTUNE disabled".to_string())
            } else {
                let tracker = state
                    .mem
                    .followup_tracker()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let queries = tracker.queries;
                let rate = tracker.followup_rate();
                drop(tracker);
                if queries < selftune::MIN_QUERIES_FOR_TUNING {
                    (
                        "ok",
                        format!(
                            "skipped - only {queries} queries observed, want at least {}",
                            selftune::MIN_QUERIES_FOR_TUNING
                        ),
                    )
                } else {
                    let (old, new) = selftune::tune(state.cfg.promote_threshold, rate);
                    if (new - old).abs() > f32::EPSILON {
                        (
                            "ok",
                            format!(
                                "followup_rate={rate:.3} over {queries} queries, \
                                 promote_threshold {old:.2} -> {new:.2}"
                            ),
                        )
                    } else {
                        (
                            "ok",
                            format!(
                                "followup_rate={rate:.3} over {queries} queries, \
                                 promote_threshold unchanged at {old:.2}"
                            ),
                        )
                    }
                }
            }
        }
        // Unreachable: the JOBS membership check above already rejected anything else.
        other => ("err", format!("unknown job: {other}")),
    };
    let run = CronRun {
        job,
        started_at,
        duration_ms: t0.elapsed().as_millis() as i64,
        outcome,
        detail,
    };
    state.cron_history.record(run.clone());
    crate::events::publish_cron(&state.events, run.job, run.outcome);
    tracing::info!(
        job = job,
        outcome = outcome,
        duration_ms = run.duration_ms,
        "cron job ran"
    );
    Ok(run)
}

/// Build and start the scheduler. Returns the `JobScheduler` handle - the caller must keep it
/// alive for the process lifetime (dropping it stops the jobs). A no-op (empty, never-started
/// scheduler) when `Config::cron_enabled` is `false`.
pub async fn start(state: AppState) -> Result<JobScheduler, JobSchedulerError> {
    let sched = JobScheduler::new().await?;
    if !state.cfg.cron_enabled {
        tracing::info!("cron scheduler disabled (CAIRN_CRON_ENABLED=false)");
        return Ok(sched);
    }
    for (name, schedule) in JOBS {
        let name = *name;
        let state = state.clone();
        let job = Job::new_async(*schedule, move |_uuid, _lock| {
            let state = state.clone();
            Box::pin(async move {
                // `run_job_now` is synchronous and scans the whole memory corpus - run it on
                // a blocking thread so a slow job never stalls the scheduler's own executor.
                let state = state.clone();
                let _ = tokio::task::spawn_blocking(move || run_job_now(&state, name)).await;
            })
        })?;
        sched.add(job).await?;
    }
    sched.start().await?;
    tracing::info!(jobs = JOBS.len(), "cron scheduler started");
    Ok(sched)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(job: &'static str, outcome: &'static str) -> CronRun {
        CronRun {
            job,
            started_at: Utc::now(),
            duration_ms: 1,
            outcome,
            detail: "test".into(),
        }
    }

    #[test]
    fn history_caps_entries_per_job_independently() {
        let h = CronHistory::default();
        for _ in 0..(MAX_HISTORY_PER_JOB + 5) {
            h.record(run("session-gc", "ok"));
        }
        h.record(run("memory-decay", "ok"));

        assert_eq!(h.recent(Some("session-gc")).len(), MAX_HISTORY_PER_JOB);
        assert_eq!(h.recent(Some("memory-decay")).len(), 1);
        assert_eq!(h.recent(None).len(), MAX_HISTORY_PER_JOB + 1);
    }

    #[test]
    fn last_run_returns_the_most_recent_for_that_job() {
        let h = CronHistory::default();
        h.record(run("session-gc", "ok"));
        h.record(run("memory-decay", "err"));
        h.record(run("session-gc", "err"));

        let last = h.last_run("session-gc").expect("has a run");
        assert_eq!(last.outcome, "err");
        assert!(h.last_run("access-log-prune").is_none());
    }

    #[test]
    fn single_flight_guard_blocks_a_second_start_until_the_first_finishes() {
        let h = CronHistory::default();
        assert!(h.try_start("session-gc"), "first start succeeds");
        assert!(
            !h.try_start("session-gc"),
            "second concurrent start is blocked"
        );
        assert!(h.is_running("session-gc"));

        h.finish("session-gc");
        assert!(!h.is_running("session-gc"));
        assert!(
            h.try_start("session-gc"),
            "start succeeds again once finished"
        );
    }

    #[test]
    fn single_flight_guard_is_independent_per_job() {
        let h = CronHistory::default();
        assert!(h.try_start("session-gc"));
        assert!(
            h.try_start("memory-decay"),
            "a different job is never blocked"
        );
    }

    #[test]
    fn running_guard_releases_the_slot_on_drop() {
        let h = CronHistory::default();
        {
            // Mirrors `run_job_now`'s own sequence: `try_start` acquires, the guard only ever
            // releases - constructing a `RunningGuard` alone does not mark anything running.
            assert!(h.try_start("session-gc"));
            let _guard = RunningGuard {
                history: &h,
                job: "session-gc",
            };
            assert!(h.is_running("session-gc"));
        }
        assert!(
            !h.is_running("session-gc"),
            "slot released once the guard drops"
        );
    }

    #[test]
    fn is_stale_is_false_for_a_job_that_has_never_run() {
        let h = CronHistory::default();
        assert!(!h.is_stale("session-gc", Utc::now()));
    }

    #[test]
    fn is_stale_flags_a_daily_job_gone_quiet_for_days() {
        let h = CronHistory::default();
        h.record(CronRun {
            started_at: Utc::now() - Duration::days(10),
            ..run("session-gc", "ok")
        });
        assert!(h.is_stale("session-gc", Utc::now()));
    }

    #[test]
    fn is_stale_is_false_for_a_weekly_job_run_yesterday() {
        let h = CronHistory::default();
        h.record(CronRun {
            started_at: Utc::now() - Duration::days(1),
            ..run("memory-decay", "ok")
        });
        assert!(
            !h.is_stale("memory-decay", Utc::now()),
            "well within its weekly nominal period"
        );
    }

    #[test]
    fn run_job_now_skips_and_records_when_already_running() {
        let Some((state, _dir)) = crate::tests::test_state() else {
            return;
        };
        assert!(state.cron_history.try_start("session-gc"));

        let run = run_job_now(&state, "session-gc").expect("known job");
        assert_eq!(run.outcome, "skipped");

        state.cron_history.finish("session-gc");
    }

    #[test]
    fn run_job_now_publishes_a_cron_event() {
        let Some((state, _dir)) = crate::tests::test_state() else {
            return;
        };
        let mut rx = state.events.subscribe();
        run_job_now(&state, "session-gc").expect("known job");
        let ev = rx
            .try_recv()
            .expect("run_job_now should publish a cron event");
        assert_eq!(ev.kind, crate::events::KIND_CRON);
        assert_eq!(ev.data["job"], "session-gc");
        assert_eq!(ev.data["outcome"], "ok");
    }
}
