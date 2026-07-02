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

use crate::AppState;
use chrono::{DateTime, Duration, Utc};
use std::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};

/// The fixed set of background jobs and their (6-field, seconds-first) cron schedules.
pub const JOBS: &[(&str, &str)] = &[
    ("session-gc", "0 0 2 * * *"),       // daily at 02:00
    ("memory-decay", "0 0 3 * * Sun"),   // weekly, Sunday 03:00
    ("access-log-prune", "0 0 4 1 * *"), // monthly, 1st at 04:00
    ("llm-intelligence", "0 30 3 * * *"), // daily at 03:30 (v0.8.0 Sprint 5)
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
/// history out. Guarded by a plain `Mutex` - jobs tick at most a few times a day, so lock
/// contention is a non-issue.
#[derive(Default)]
pub struct CronHistory(Mutex<Vec<CronRun>>);

impl CronHistory {
    pub fn record(&self, run: CronRun) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
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
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.iter()
            .filter(|r| job.is_none_or(|j| r.job == j))
            .cloned()
            .collect()
    }

    /// The most recent run of `job`, if it has ever run.
    pub fn last_run(&self, job: &str) -> Option<CronRun> {
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.iter().rev().find(|r| r.job == job).cloned()
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
    let started_at = Utc::now();
    let t0 = std::time::Instant::now();
    let (outcome, detail): (&'static str, String) = match job {
        "session-gc" => match state.mem.run_session_gc(state.cfg.session_ttl_days) {
            Ok(n) => ("ok", format!("promoted {n} session-scoped memories to global")),
            Err(e) => ("err", e.to_string()),
        },
        "memory-decay" => match state.mem.run_decay(state.cfg.decay_period_days) {
            Ok(n) => ("ok", format!("decayed confidence on {n} memories")),
            Err(e) => ("err", e.to_string()),
        },
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
                let concepts = state.mem.run_concept_extraction(&state.cfg.llm_consolidation);
                let contradictions = state
                    .mem
                    .run_contradiction_detection(&state.cfg.llm_consolidation);
                let scored = state.mem.run_promotion_scoring(&state.cfg.llm_consolidation);
                match (concepts, contradictions, scored) {
                    (Ok(c), Ok(x), Ok(s)) => (
                        "ok",
                        format!(
                            "extracted concepts on {c} memories, flagged {x} contradictions, \
                             scored {s} promotion candidates"
                        ),
                    ),
                    (c, x, s) => (
                        "err",
                        format!(
                            "concept_extraction={c:?} contradiction_detection={x:?} \
                             promotion_scoring={s:?}"
                        ),
                    ),
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
}
