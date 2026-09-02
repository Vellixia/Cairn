//! Consolidation's claim machinery (`contracts/consolidation.md` §4, §4.1, §6;
//! FR-793a–d, FR-808).
//!
//! One in-process Tokio task turning accepted safe events into knowledge,
//! inside the server that already exists. This module is the part of that which
//! decides *what a pass works on and what happens to it afterwards*: electing a
//! session, counting its attempt, holding and releasing the lease. The
//! extraction, governance and persistence a pass performs are US1's
//! (`consolidate_batch` below is the seam), and nothing here depends on them.
//!
//! # A group is not lockable; a row is
//!
//! A pass consolidates one *session's* events, and the obvious way to claim
//! that is `GROUP BY session_id … FOR UPDATE SKIP LOCKED`. PostgreSQL refuses
//! it outright — `ERROR: 0A000: FOR UPDATE is not allowed with GROUP BY clause`
//! — because a locking clause needs rows it can identify with individual table
//! rows, and an aggregate has none. So the group is given a row:
//! `consolidation_session`, one per session, and that row is what gets locked
//! and leased. No statement in this module aggregates, so `0A000` cannot arise.
//!
//! # The attempt is counted before the work runs
//!
//! `attempts` increments in the **claim** transaction, once per event the pass
//! is about to process — not on failure. A worker that dies mid-pass has
//! therefore already consumed the attempt, and the reclaim resumes from that
//! count. Counting on failure instead lets a process that crashes before it can
//! record anything retry the same event forever; the local spool shipped that
//! defect and had to be repaired (`crates/cairn-store/src/spool.rs`, "every
//! claim increments `attempts`, so a crash loop would otherwise retry a row"),
//! and this is the same rule in the same shape.
//!
//! # Five attempts means five that actually ran
//!
//! Claim selection requires `attempts < 5`, so a sixth pass never starts. The
//! failure sweep runs at **close**, after the pass, and after the successful
//! event ids have already moved to `done` — both placements matter. A sweep at
//! claim time would stop the fifth attempt from running at all; a sweep before
//! the success update would file a fifth attempt that succeeded as `failed`.
//!
//! # Nothing strands a session
//!
//! The close statement chooses `pending` or `done` with a `CASE` rather than
//! guarding on `NOT EXISTS`. The guard form matched no row when work remained,
//! which left the session `claimed` with a live lease and nothing re-elected it
//! until the lease expired — a five-minute stall after every full batch. A
//! session whose remaining work has all `failed` reaches `done`, because the
//! `EXISTS` looks only for `pending`.

use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::Semaphore;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// The bounds (§6)
// ---------------------------------------------------------------------------

/// Events one pass claims from one session.
pub const BATCH_EVENTS: i64 = 200;

/// How long a claim owns its session before another worker may reclaim it.
pub const CLAIM_LEASE: Duration = Duration::from_secs(5 * 60);

/// The pause between passes.
///
/// The contract names one number, and it is used for an idle poll as well as
/// for the gap between batches: inventing a second, longer interval for an
/// empty queue would be a bound nobody stated, and a slower one would delay
/// every session's first pass by however long it was.
pub const BATCH_YIELD: Duration = Duration::from_millis(100);

/// Passes an event gets before it is retired unconsolidated.
pub const MAX_ATTEMPTS: i32 = 5;

/// The server schema that brings `consolidation_session` and its neighbours.
///
/// Named rather than compared against [`crate::db::SCHEMA_VERSION`]: the two
/// will diverge at the next migration, and what consolidation needs is these
/// tables, not the newest schema this build happens to carry.
pub const REQUIRED_SCHEMA: i64 = 4;

/// The most connections consolidation may hold, however large the pool is.
const MAX_POOL_SHARE: u32 = 2;

/// The fraction of the pool consolidation may hold.
const POOL_SHARE_DIVISOR: u32 = 5;

/// What `last_error` records for an event retired without a reported error.
///
/// A fifth attempt whose worker died has no error to quote — the pass never got
/// far enough to produce one — but a `failed` row that says nothing is a row
/// system health cannot explain. A fixed term, like every other reason in this
/// feature, never free text derived from content.
const ATTEMPTS_EXHAUSTED: &str = "attempts_exhausted";

/// What produced this pass's candidates.
///
/// `none` while US1's extractor (T052) is unwritten. Calling it `deterministic`
/// would claim the R1–R8 baseline had run, and a run report that overstates
/// what examined the events is worse than one that admits nothing did.
const EXTRACTOR_KIND: &str = "none";

/// How many of the server's connections consolidation may hold (FR-793a1).
///
/// A **fraction with a floor**, not a fixed two. `max_connections` is
/// operator-set and the end-to-end suite deliberately runs servers with small
/// pools; a fixed two would take two thirds of a pool of three, which is
/// exactly the starvation this bound exists to prevent. Integer division also
/// states the rest of the rule without a second branch: below five connections
/// the share is zero and consolidation does not run at all, because request
/// serving is what the process is for and a backlog must never be reported to a
/// client as an ingestion failure (FR-814).
pub fn pool_share(max_connections: u32) -> u32 {
    MAX_POOL_SHARE.min(max_connections / POOL_SHARE_DIVISOR)
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

/// Elect and claim exactly one session, oldest first, reclaiming a dead lease.
///
/// `LIMIT 1` over a table with one row per session is what makes "one session
/// per pass" structural rather than conventional. `SKIP LOCKED` excludes
/// concurrent claimers for the length of the claiming transaction, and the
/// `state` flip commits inside that window, so afterwards the election
/// predicate itself excludes them. `OFFSET` is deliberately absent: rows it
/// skipped would still be locked.
/// How many pending events make a session eligible on volume alone
/// (`contracts/consolidation.md` §3).
///
/// The same number as the batch, which is not a coincidence: the threshold is
/// "there is a full batch to do".
pub const ELIGIBLE_PENDING_COUNT: i64 = BATCH_EVENTS;

/// How old the oldest pending event must be for a still-open session to be
/// eligible on age (`contracts/consolidation.md` §3).
///
/// This is the condition that makes a long-running session produce knowledge
/// before it ends. Without it a session that stays open all afternoon and never
/// reaches 200 events would consolidate nothing until it closed.
pub const ELIGIBLE_AGE: Duration = Duration::from_secs(10 * 60);

const ELECT: &str = "\
UPDATE consolidation_session s
   SET state = 'claimed', claimed_by = $1,
       claim_expires_at = now() + make_interval(secs => $2),
       -- The latch, set on the first election and left alone afterwards.
       -- `COALESCE` rather than `now()` so a generation that has already begun
       -- keeps the moment it became eligible, which is what a report of
       -- \"how long has this been consolidating\" needs.
       eligible_since = COALESCE(s.eligible_since, now())
 WHERE (s.project_id, s.session_id) = (
         SELECT c.project_id, c.session_id
           FROM consolidation_session c
          WHERE (c.state = 'pending'
                 OR (c.state = 'claimed' AND c.claim_expires_at < now()))
            AND (
                 -- Already latched: this generation began consolidating and
                 -- stays eligible until its work is finished. This is what
                 -- makes the tail of a partial batch re-electable at once
                 -- rather than waiting for a threshold it can no longer meet
                 -- (see section 3 of the consolidation contract).
                 c.eligible_since IS NOT NULL
                 -- The session has closed. Read from the server's own
                 -- `sessions` row, which sync maintains; no new client
                 -- assertion is introduced for this.
                 OR EXISTS (SELECT 1 FROM sessions ss
                             WHERE ss.id = c.session_id
                               AND (ss.ended_at IS NOT NULL OR ss.status <> 'active'))
                 -- A full batch has accumulated.
                 OR (SELECT COUNT(*) FROM consolidation_work w
                      WHERE w.project_id = c.project_id
                        AND w.session_id = c.session_id
                        AND w.state = 'pending'
                        AND w.attempts < $4) >= $5
                 -- The oldest pending event has waited long enough.
                 OR c.oldest_enqueued_at <= now() - make_interval(secs => $6)
                )
          ORDER BY c.oldest_enqueued_at
          FOR UPDATE SKIP LOCKED
          LIMIT 1)
RETURNING s.project_id, s.session_id";

/// The claimed session's work, in the order it happened.
///
/// `ORDER BY session_seq`, never `event_id`: the id is a UUIDv5 derived from
/// the session and an ordinal, so ordering by it would shuffle a session's
/// history and make every rule that reads a sequence of acts meaningless.
const READ_BATCH: &str = "\
SELECT event_id
  FROM consolidation_work
 WHERE project_id = $1 AND session_id = $2 AND state = 'pending'
   AND attempts < $3
 ORDER BY session_seq
 LIMIT $4";

const COUNT_ATTEMPT: &str = "\
UPDATE consolidation_work SET attempts = attempts + 1
 WHERE project_id = $1 AND session_id = $2 AND event_id = ANY($3)";

/// A run exists from the moment a pass claims, so backlog and progress are
/// reportable mid-pass and immediately after a restart (FR-793c).
///
/// The candidate counters stay NULL here: this pass has no extractor, and US1
/// (T054, T056) fills them in when there is something to count. A run left
/// `running` by a worker that died is not tidied away, because "a pass started
/// and never finished" is a fact a health report needs.
const OPEN_RUN: &str = "\
INSERT INTO consolidation_runs
       (run_id, project_id, session_id, started_at, events_claimed, extractor_kind, state)
VALUES ($1, $2, $3, now(), $4, $5, 'running')";

/// Success wins, including success on the fifth attempt.
const CLOSE_CONSOLIDATED: &str = "\
UPDATE consolidation_work SET state = 'done'
 WHERE project_id = $1 AND session_id = $2 AND state = 'pending'
   AND event_id = ANY($3)";

/// Only unsuccessful fifth attempts remain `pending` here, and they retire.
///
/// Not restricted to this pass's batch, deliberately: an event whose fifth
/// attempt was consumed by a worker that then died is not in any later batch —
/// claim selection excludes it — so a batch-scoped sweep would leave it
/// `pending` forever and its session electable forever with it.
const CLOSE_EXHAUSTED: &str = "\
UPDATE consolidation_work SET state = 'failed', last_error = $3
 WHERE project_id = $1 AND session_id = $2 AND state = 'pending'
   AND attempts >= $4";

/// Release the lease and set the next state, in one statement.
///
/// A `CASE`, not a guard. `… SET state='done' … AND NOT EXISTS (pending)`
/// matched no row when work remained, so the session kept its lease and waited
/// out the full five minutes before anything re-elected it; a session with 500
/// pending events drained at 200 per five minutes for no reason. Guarded on
/// `claimed_by` so a worker whose lease already expired and was reclaimed by
/// somebody else cannot release a claim it no longer holds.
const CLOSE_SESSION: &str = "\
UPDATE consolidation_session
   SET state = CASE WHEN EXISTS (SELECT 1 FROM consolidation_work
                                  WHERE project_id = $1 AND session_id = $2
                                    AND state = 'pending')
                    THEN 'pending'
                    ELSE 'done'
               END,
       -- The latch survives a partial batch and is dropped when the generation
       -- finishes. That asymmetry is the whole mechanism: 205 events trigger at
       -- 200, this pass takes 200, and the remaining five stay eligible because
       -- the latch is still set — they do not have to satisfy a threshold they
       -- can no longer reach. Once nothing is pending the generation is over,
       -- and the next one must earn its own eligibility.
       eligible_since = CASE WHEN EXISTS (SELECT 1 FROM consolidation_work
                                           WHERE project_id = $1 AND session_id = $2
                                             AND state = 'pending')
                             THEN eligible_since
                             ELSE NULL
                        END,
       claimed_by = NULL, claim_expires_at = NULL
 WHERE project_id = $1 AND session_id = $2 AND claimed_by = $3";

const FINISH_RUN: &str = "\
UPDATE consolidation_runs SET finished_at = now(), state = $2 WHERE run_id = $1";

/// Push the lease out while a pass is still running.
///
/// Guarded on `claimed_by` for the same reason the close is: a worker whose
/// lease expired and was reclaimed must not be able to take it back mid-pass
/// and leave two workers each believing they own the session.
const HEARTBEAT: &str = "\
UPDATE consolidation_session
   SET claim_expires_at = now() + make_interval(secs => $3)
 WHERE project_id = $1 AND session_id = $2 AND claimed_by = $4";

// ---------------------------------------------------------------------------
// The worker
// ---------------------------------------------------------------------------

/// One session claimed for one pass.
struct Claim {
    project_id: Uuid,
    session_id: Uuid,
    run_id: Uuid,
    /// The events whose attempt this claim has already counted.
    events: Vec<Uuid>,
}

/// The single in-process consolidation task.
pub struct Consolidator {
    pool: PgPool,
    /// Consolidation's share of the pool it shares with request serving.
    ///
    /// A semaphore rather than a second pool: a second pool would add
    /// connections on top of `max_connections` instead of taking a slice of it,
    /// which is the opposite of the bound. Holding at most `share` permits
    /// leaves the rest of the pool available to requests at all times, which is
    /// how FR-814's prohibition on back-pressure is met by limits rather than
    /// by intention.
    budget: Semaphore,
    /// This process's claim identity.
    ///
    /// Unique per process, because the close and heartbeat statements are
    /// guarded on it: two workers sharing a name could release each other's
    /// leases.
    worker: String,
}

impl Consolidator {
    pub fn new(pool: PgPool, share: u32) -> Self {
        Self {
            pool,
            budget: Semaphore::new(share as usize),
            worker: format!("consolidator-{}", Uuid::now_v7()),
        }
    }

    /// Claim, consolidate, close — forever, yielding between passes.
    ///
    /// A failed pass is logged and the loop continues. It must not end the
    /// task: the failure may be one session's, and a server that stopped
    /// consolidating on the first error would accumulate a backlog silently for
    /// as long as it stayed up.
    pub async fn run(self) {
        tracing::info!(
            worker = %self.worker,
            batch = BATCH_EVENTS,
            lease_secs = CLAIM_LEASE.as_secs(),
            max_attempts = MAX_ATTEMPTS,
            "consolidation started"
        );
        loop {
            if let Err(e) = self.pass().await {
                tracing::warn!(worker = %self.worker, error = %e, "a consolidation pass failed");
            }
            tokio::time::sleep(BATCH_YIELD).await;
        }
    }

    /// One pass. `Ok(false)` when there was nothing to elect.
    async fn pass(&self) -> Result<bool, sqlx::Error> {
        let Some(claim) = self.claim().await? else {
            return Ok(false);
        };

        // The pass proper, outside the claim transaction. Its own transaction,
        // so a failure persists nothing and a partial candidate cannot exist
        // (FR-808) — and separate from the claim's, so the `attempts` increment
        // survives that rollback and the counter stays monotonic.
        //
        // The heartbeat runs beside it rather than after it: a pass longer than
        // the lease would otherwise be reclaimed out from under itself.
        let outcome = tokio::select! {
            outcome = consolidate_batch(&self.pool, &claim) => outcome,
            never = self.heartbeat(&claim) => match never {},
        };

        self.close(&claim, outcome).await?;
        Ok(true)
    }

    /// Elect one session, read its batch, and count the attempt — one
    /// transaction (§4).
    async fn claim(&self) -> Result<Option<Claim>, sqlx::Error> {
        let _permit = self.permit().await;
        let mut tx = self.pool.begin().await?;

        let elected: Option<(Uuid, Uuid)> = sqlx::query_as(ELECT)
            .bind(&self.worker)
            .bind(CLAIM_LEASE.as_secs_f64())
            // `$3` is unused by ELECT and kept only so the parameter numbering
            // matches the rest of this module's statements, where `$3` is the
            // attempt ceiling. Binding it here keeps the eligibility bounds at
            // the same indices they have everywhere else.
            .bind(MAX_ATTEMPTS)
            .bind(MAX_ATTEMPTS)
            .bind(ELIGIBLE_PENDING_COUNT)
            .bind(ELIGIBLE_AGE.as_secs_f64())
            .fetch_optional(&mut *tx)
            .await?;
        let Some((project_id, session_id)) = elected else {
            // Nothing to claim. Rolled back rather than committed because the
            // transaction wrote nothing and a row lock held a moment longer is
            // a row another worker cannot elect.
            tx.rollback().await?;
            return Ok(None);
        };

        let events: Vec<Uuid> = sqlx::query_scalar(READ_BATCH)
            .bind(project_id)
            .bind(session_id)
            .bind(MAX_ATTEMPTS)
            .bind(BATCH_EVENTS)
            .fetch_all(&mut *tx)
            .await?;

        // Before the work runs, in the same transaction as the claim. This is
        // the line that makes a crash mid-pass consume its attempt.
        sqlx::query(COUNT_ATTEMPT)
            .bind(project_id)
            .bind(session_id)
            .bind(&events)
            .execute(&mut *tx)
            .await?;

        let run_id = Uuid::now_v7();
        sqlx::query(OPEN_RUN)
            .bind(run_id)
            .bind(project_id)
            .bind(session_id)
            .bind(events.len() as i32)
            .bind(EXTRACTOR_KIND)
            .execute(&mut *tx)
            .await?;

        // Committed, not held: every piece of claim state is a durable row
        // rather than a lock, which is what makes the arrangement survive a
        // backend crash or a server restart. Advisory locks were rejected for
        // exactly that — they vanish, and they cannot express a lease.
        tx.commit().await?;

        Ok(Some(Claim {
            project_id,
            session_id,
            run_id,
            events,
        }))
    }

    /// Mark successes, retire exhausted work, release the lease — one
    /// transaction, in this order (§4.1).
    async fn close(&self, claim: &Claim, outcome: PassOutcome) -> Result<(), sqlx::Error> {
        let (consolidated, last_error, run_state) = match &outcome {
            Ok(ids) => (ids.as_slice(), ATTEMPTS_EXHAUSTED.to_string(), "finished"),
            Err(e) => ([].as_slice(), e.clone(), "failed"),
        };

        let _permit = self.permit().await;
        let mut tx = self.pool.begin().await?;

        sqlx::query(CLOSE_CONSOLIDATED)
            .bind(claim.project_id)
            .bind(claim.session_id)
            .bind(consolidated)
            .execute(&mut *tx)
            .await?;

        sqlx::query(CLOSE_EXHAUSTED)
            .bind(claim.project_id)
            .bind(claim.session_id)
            .bind(&last_error)
            .bind(MAX_ATTEMPTS)
            .execute(&mut *tx)
            .await?;

        sqlx::query(CLOSE_SESSION)
            .bind(claim.project_id)
            .bind(claim.session_id)
            .bind(&self.worker)
            .execute(&mut *tx)
            .await?;

        sqlx::query(FINISH_RUN)
            .bind(claim.run_id)
            .bind(run_state)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        if let Err(e) = &outcome {
            tracing::warn!(
                session = %claim.session_id,
                events = claim.events.len(),
                error = %e,
                "a consolidation batch produced nothing and its attempt is spent"
            );
        }
        Ok(())
    }

    /// Extend the lease for as long as the pass runs. Never returns.
    async fn heartbeat(&self, claim: &Claim) -> std::convert::Infallible {
        loop {
            // A third of the lease, so two consecutive failures still leave
            // time to recover before another worker may legitimately reclaim.
            tokio::time::sleep(CLAIM_LEASE / 3).await;
            let _permit = self.permit().await;
            let extended = sqlx::query(HEARTBEAT)
                .bind(claim.project_id)
                .bind(claim.session_id)
                .bind(CLAIM_LEASE.as_secs_f64())
                .bind(&self.worker)
                .execute(&self.pool)
                .await;
            match extended {
                Ok(done) if done.rows_affected() == 0 => tracing::warn!(
                    session = %claim.session_id,
                    "this pass no longer holds its lease; another worker has the session"
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "a consolidation heartbeat failed"),
            }
        }
    }

    /// One of consolidation's connections, waited for rather than taken.
    async fn permit(&self) -> tokio::sync::SemaphorePermit<'_> {
        self.budget
            .acquire()
            .await
            .expect("consolidation's connection budget is never closed")
    }
}

// ---------------------------------------------------------------------------
// The extraction seam — US1's
// ---------------------------------------------------------------------------

/// What a pass produced: the events it consolidated, or why it produced nothing.
type PassOutcome = Result<Vec<Uuid>, String>;

/// Consolidate one claimed batch — **the seam US1 fills in** (T052 extraction,
/// T053 governance, T054 persistence).
///
/// There is no extractor yet, so this is a deterministic no-op: it consolidates
/// every event it was handed and produces no knowledge. That is a real outcome
/// rather than a stub's evasion — §8 is explicit that a run with zero
/// candidates is still a run, and "consolidation found nothing" and
/// "consolidation never happened" are two states a health report must not
/// conflate. Draining is the honest behaviour of a pass with nothing to propose;
/// leaving the events `pending` instead would grow a backlog that says work is
/// outstanding when none is.
///
/// Whatever replaces this must run extraction, governance and persistence in
/// **one transaction** (FR-808): a failed batch persists nothing, so a partial
/// candidate cannot exist. The `attempts` increment is deliberately not in that
/// transaction — it is already committed by the claim — so a rollback here
/// leaves the counter where it was raised.
async fn consolidate_batch(_pool: &PgPool, claim: &Claim) -> PassOutcome {
    Ok(claim.events.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pool_share_is_a_fraction_with_a_floor_and_a_ceiling() {
        // Below five connections consolidation does not run at all: a fixed
        // share of two would take two thirds of a pool of three, and the
        // end-to-end suite runs servers exactly that small.
        assert_eq!(pool_share(0), 0);
        assert_eq!(pool_share(4), 0);
        // Five is the smallest pool that earns a share, and it earns one.
        assert_eq!(pool_share(5), 1);
        assert_eq!(pool_share(9), 1);
        // The ceiling holds however large the pool is, so request serving keeps
        // everything above two.
        assert_eq!(pool_share(10), 2);
        assert_eq!(pool_share(100), 2);
    }
}
