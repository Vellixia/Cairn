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

use crate::extract::{self, CandidateProposal};
use cairn_core::domain::{KnowledgeDomain, MemoryType, RelationBasis, RelationKind};
use cairn_core::event::SafeCanonicalEvent;
use cairn_core::eventid::{candidate_id, corroboration_id, proposal_digest, refusal_id};
use cairn_core::knowledge::{
    normalize_candidate_keys, normalize_content, normalize_relation_endpoints, value_keys_agree,
};
use cairn_core::validate::validate_candidate_content;
use sqlx::PgPool;
use std::collections::BTreeSet;
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
/// What a run records before its extractor has spoken.
///
/// Overwritten at close with the kind that actually ran, so a run row never
/// claims an extractor it did not use.
const EXTRACTOR_KIND: &str = "pending";

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

/// What a pass produced: the events it consolidated, or why it produced nothing.
type PassOutcome = Result<Vec<Uuid>, String>;

// ---------------------------------------------------------------------------
// Governance and persistence (T053–T056, `contracts/consolidation.md` §5, §7)
// ---------------------------------------------------------------------------

/// The refusal vocabulary (§9), distinct from the event-rejection one.
///
/// Spelled here as constants rather than inline strings so a reason cannot be
/// invented at a call site: FR-804a requires a fixed, enumerated vocabulary,
/// and a health report that had to guess whether `privacy` and
/// `privacy_refused` were the same reason could not count either.
mod refusal {
    // Every term §9 fixes is named here, including the two this build does not
    // currently emit. The vocabulary is a property of the contract and not of
    // the implementation: a reader of a refusal row, and a health report
    // counting reasons, must recognise every term the specification allows, and
    // trimming the list to what one build happens to produce would turn an
    // enumeration into a description of today's code.
    //
    // `conflicts_with_existing` in particular is unemitted on purpose. A
    // conflict is recorded and never resolved (FR-799), so a candidate that
    // disagrees with existing knowledge is persisted alongside it rather than
    // refused — but a future resolver that must refuse one has the term
    // waiting, spelled the way the contract spells it.
    #![allow(dead_code)]

    pub const KEY_NORMALIZATION_FAILED: &str = "key_normalization_failed";
    pub const KEY_NOT_DERIVABLE: &str = "key_not_derivable";
    pub const PRIVACY_REFUSED: &str = "privacy_refused";
    pub const UNVERIFIABLE_SOURCE: &str = "unverifiable_source";
    pub const CONFLICTS_WITH_EXISTING: &str = "conflicts_with_existing";
    pub const BOUND_EXCEEDED: &str = "bound_exceeded";
    pub const EXTRACTOR_MALFORMED_OUTPUT: &str = "extractor_malformed_output";

    /// Every term, so a test can hold the vocabulary to its stated size.
    pub const ALL: &[&str] = &[
        KEY_NORMALIZATION_FAILED,
        KEY_NOT_DERIVABLE,
        PRIVACY_REFUSED,
        UNVERIFIABLE_SOURCE,
        "domain_unresolvable",
        "scope_unresolvable",
        CONFLICTS_WITH_EXISTING,
        BOUND_EXCEEDED,
        EXTRACTOR_MALFORMED_OUTPUT,
    ];
}

/// How many existing records a full-text narrowing may return.
///
/// The text index narrows; exact normalized-content equality decides. A wider
/// result would not make the decision better, because the decision is not a
/// similarity judgement — two contents are equal or they are unrelated.
const TEXT_MATCH_LIMIT: i64 = 20;

const READ_BATCH_EVENTS: &str = "\
SELECT event_id, account_id, agent, kind, vendor_event, session_seq, contract_version,
       content, occurred_at
  FROM safe_events
 WHERE project_id = $1 AND session_id = $2 AND event_id = ANY($3)
 ORDER BY session_seq";

/// The project's recent sessions, for the rules that need more than one.
///
/// Scoped to the batch's project and nothing wider. FR-805a1 confines an
/// extraction request to one project and one account context, and this read is
/// the one place a cross-project corpus could otherwise leak in.
const READ_PROJECT_EVENTS: &str = "\
SELECT session_id, event_id, agent, kind, vendor_event, session_seq, contract_version,
       content, occurred_at
  FROM safe_events
 WHERE project_id = $1
   AND session_id IN (
         SELECT session_id FROM safe_events
          WHERE project_id = $1
          GROUP BY session_id
          ORDER BY MAX(received_at) DESC
          LIMIT $2)
 ORDER BY session_id, session_seq
 LIMIT $3";

/// Which of the cited events exist in this project and this session.
///
/// Gate 1 for a **session rule**, which is what an extractor produces. The
/// predicate carries both, so a proposal citing an event from anywhere else
/// gets an empty row back and is refused rather than persisted with a citation
/// nobody can follow.
const VERIFY_SESSION_SOURCES: &str = "\
SELECT event_id FROM safe_events
 WHERE project_id = $1 AND session_id = $2 AND event_id = ANY($3)";

/// Which of the cited events exist in this project at all.
///
/// Gate 1 for a **project rule**. R3, R5, R6 and R8 exist precisely because
/// their evidence spans sessions (`contracts/extraction.md` §4.0), so verifying
/// them against one session would refuse every candidate they can produce — and
/// silently, since a refusal is a normal outcome. The scope that matters is
/// still the one FR-805a1 names: one project, and the aggregator was handed
/// exactly this project's sessions.
///
/// No extractor reaches this predicate. A project rule is Cairn's own
/// deterministic function, which is the reason the rules that most resemble
/// policy claims about a project are the ones no extractor influences at all.
const VERIFY_PROJECT_SOURCES: &str = "\
SELECT event_id FROM safe_events
 WHERE project_id = $1 AND event_id = ANY($2)";

/// Existing project knowledge under one subject.
///
/// Corroboration endpoints are excluded: they are the evidence behind a
/// reinforcement count, never independent knowledge, and matching against one
/// would let a reinforcement reinforce itself (FR-798a).
const PROJECT_BY_TOPIC: &str = "\
SELECT id, value_key, content FROM memories
 WHERE project_id = $1 AND topic_key = $2
   AND deleted_at IS NULL AND state <> 'superseded'
   AND origin_kind IS DISTINCT FROM 'corroboration'";

const PROJECT_BY_TEXT: &str = "\
SELECT id, content FROM memories
 WHERE project_id = $1
   AND deleted_at IS NULL AND state <> 'superseded'
   AND origin_kind IS DISTINCT FROM 'corroboration'
   AND to_tsvector('english', content) @@ plainto_tsquery('english', $2)
 LIMIT $3";

const PERSONAL_BY_TOPIC: &str = "\
SELECT id, value_key, content FROM personal_knowledge
 WHERE owner_user_id = $1 AND topic_key = $2 AND forgotten_at IS NULL";

const PERSONAL_BY_TEXT: &str = "\
SELECT id, content FROM personal_knowledge
 WHERE owner_user_id = $1 AND forgotten_at IS NULL
   AND to_tsvector('english', content) @@ plainto_tsquery('english', $2)
 LIMIT $3";

const TEAM_BY_TOPIC: &str = "\
SELECT id, value_key, content FROM team_knowledge
 WHERE topic_key = $1 AND state = 'authoritative'";

const TEAM_BY_TEXT: &str = "\
SELECT id, content FROM team_knowledge
 WHERE state = 'authoritative'
   AND to_tsvector('english', content) @@ plainto_tsquery('english', $1)
 LIMIT $2";

const INSERT_MEMORY: &str = "\
INSERT INTO memories
    (id, project_id, type, scope, scope_key, content, state, origin_session_id,
     observation_ids, evidence_count, topic_key, value_key, origin_kind, updated_at)
VALUES ($1, $2, $3, 'project', $4, $5, 'active', $6, '[]'::jsonb, 0, $7, $8, $9, now())
ON CONFLICT (id) DO NOTHING";

const INSERT_RELATION: &str = "\
INSERT INTO memory_relations
    (from_memory_id, to_memory_id, kind, project_id, decided_by_session, basis)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (from_memory_id, to_memory_id, kind) DO NOTHING";

const BUMP_REINFORCEMENT: &str = "\
UPDATE memories SET reinforcement_count = reinforcement_count + 1, updated_at = now()
 WHERE id = $1";

const INSERT_CANDIDATE: &str = "\
INSERT INTO knowledge_candidates
    (candidate_id, run_id, project_id, proposed_kind, proposed_domain, topic_key, value_key,
     content, decision, refusal_reason, result_ref_kind, result_domain, result_knowledge_id)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
ON CONFLICT (candidate_id) DO NOTHING";

const INSERT_SOURCE_EVENT: &str = "\
INSERT INTO candidate_source_events (candidate_id, event_id)
VALUES ($1, $2)
ON CONFLICT (candidate_id, event_id) DO NOTHING";

const COUNT_RUN: &str = "\
UPDATE consolidation_runs
   SET candidates_proposed = $2, candidates_accepted = $3, candidates_refused = $4,
       extractor_kind = $5
 WHERE run_id = $1";

/// What one pass reads about the project's existing knowledge for one subject.
struct Existing {
    id: Uuid,
    value_key: Option<String>,
}

/// What Cairn decided about one proposal.
///
/// Named after the values `knowledge_candidates.decision` accepts, so the
/// column and the code cannot drift into describing the same outcome
/// differently.
enum Verdict {
    /// New durable knowledge.
    Accepted { knowledge_id: Uuid },
    /// The same subject and value already exist; a corroboration endpoint
    /// reinforces them.
    Reinforced { knowledge_id: Uuid },
    /// The claim is already stated by knowledge this account can read.
    Duplicate,
    /// New knowledge, and a recorded disagreement with what was already held.
    Conflicted { knowledge_id: Uuid },
    /// Nothing persisted, and the reason why.
    Refused { reason: &'static str },
}

impl Verdict {
    fn decision(&self) -> &'static str {
        match self {
            Verdict::Accepted { .. } => "accepted",
            Verdict::Reinforced { .. } => "reinforced",
            Verdict::Duplicate => "duplicate",
            Verdict::Conflicted { .. } => "conflicted",
            Verdict::Refused { .. } => "refused",
        }
    }

    fn knowledge_id(&self) -> Option<Uuid> {
        match self {
            Verdict::Accepted { knowledge_id }
            | Verdict::Reinforced { knowledge_id }
            | Verdict::Conflicted { knowledge_id } => Some(*knowledge_id),
            Verdict::Duplicate | Verdict::Refused { .. } => None,
        }
    }

    fn refusal(&self) -> Option<&'static str> {
        match self {
            Verdict::Refused { reason } => Some(reason),
            _ => None,
        }
    }

    /// Whether this proposal produced a durable effect.
    fn is_durable(&self) -> bool {
        matches!(
            self,
            Verdict::Accepted { .. } | Verdict::Reinforced { .. } | Verdict::Conflicted { .. }
        )
    }
}

/// One proposal, with the session that produced it.
///
/// A session rule's relation records that session; a project rule's has no
/// single session and records the nil UUID, which the codebase already uses for
/// an unattributed act. Inventing a session would misattribute a relation to
/// work that did not decide it (§5 gate 6a).
///
/// **The same value names the candidate.** §7 derives identity from the
/// project, the session and the normalized keys, and a project rule has no
/// session to put there. Using the pass's session instead would give one
/// project-wide claim a different identity in every session that triggered a
/// pass: the record itself would not duplicate, because gate 6 finds it and
/// reinforces — but it would gain a *fresh* corroboration endpoint and another
/// increment on every pass, forever. The count would stop meaning "independent
/// confirmations" and start meaning "passes since this rule began firing",
/// which is a number nobody asked for and everybody would read as the other
/// one.
struct Attributed {
    proposal: CandidateProposal,
    decided_by_session: Uuid,
}

/// Consolidate one claimed batch: extract, govern, persist — one transaction.
///
/// The transaction boundary is FR-808's: a failed batch persists nothing, so a
/// partial candidate cannot exist. The `attempts` increment is deliberately
/// outside it — the claim already committed it — so a rollback here leaves the
/// counter where it was raised and the batch is not retried forever.
///
/// A pass that proposes nothing still consolidates its events. §8 is explicit
/// that a run with zero candidates is still a run, and leaving the events
/// `pending` would report outstanding work where there is none.
async fn consolidate_batch(pool: &PgPool, claim: &Claim) -> PassOutcome {
    match run_pass(pool, claim).await {
        Ok(()) => Ok(claim.events.clone()),
        Err(e) => Err(e.to_string()),
    }
}

async fn run_pass(pool: &PgPool, claim: &Claim) -> Result<(), sqlx::Error> {
    let batch = read_batch_events(pool, claim).await?;
    if batch.is_empty() {
        // A pass with nothing readable is still a pass, and its run row should
        // say which extractor did not find anything rather than keeping the
        // placeholder it was opened with. "Consolidation found nothing" and
        // "consolidation never happened" are two states a health report must
        // not conflate (§8).
        let (extractor, _) = extract::select_extractor(None);
        sqlx::query(COUNT_RUN)
            .bind(claim.run_id)
            .bind(0_i32)
            .bind(0_i32)
            .bind(0_i32)
            .bind(extractor.kind())
            .execute(pool)
            .await?;
        return Ok(());
    }
    // The owner comes from the account the server bound when it accepted the
    // events, never from event content and never from extractor output. This is
    // how a process with no authenticated caller satisfies the rule that
    // identity is established rather than asserted (FR-810a, Principle XI).
    let bound_account = batch[0].account_id;
    let events: Vec<SafeCanonicalEvent> = batch.iter().map(|e| e.event.clone()).collect();

    let (extractor, blocker) = extract::select_extractor(None);
    if let Some(blocker) = blocker {
        // Reported rather than silent, and the baseline runs. Where compliance
        // cannot be established the privacy contract is not traded for
        // extraction quality (FR-805e).
        tracing::debug!(
            blocker = blocker.reason(),
            "consolidation is running the deterministic baseline"
        );
    }

    let mut proposed: Vec<Attributed> = Vec::new();
    let mut refusals: Vec<(&'static str, String)> = Vec::new();

    let scoped = extract::ExtractionInput::new(
        extract::ProjectRef(claim.project_id),
        extract::AccountRef(bound_account),
        extract::SessionRef(claim.session_id),
        events.clone(),
    );

    // The scoping governance works under is the scoping the request declares,
    // read back from it rather than carried alongside in a second variable. One
    // of the two could otherwise be changed without the other, and the pair
    // being equal is the whole of FR-805a1.
    let (project, account) = match &scoped {
        Ok(input) => (input.project(), input.account()),
        Err(_) => (
            extract::ProjectRef(claim.project_id),
            extract::AccountRef(bound_account),
        ),
    };
    let account_id = account.0;

    match scoped {
        Ok(input) => match extractor.extract(&input) {
            Ok(list) => proposed.extend(list.into_iter().map(|proposal| Attributed {
                proposal,
                decided_by_session: claim.session_id,
            })),
            Err(e) => refusals.push((e.reason(), String::new())),
        },
        Err(e) => refusals.push((e.reason(), String::new())),
    }

    // The project rules run in the same pass, over the same project, under the
    // same scoping — but not through the extractor. A session-scoped
    // `ExtractionInput` cannot see across sessions, and widening it is not
    // available (FR-805a1), so the rules that most resemble policy claims about
    // a project are the ones no extractor influences at all.
    let project_sessions = read_project_events(pool, project.0).await?;
    proposed.extend(
        extract::aggregate(project, &project_sessions)
            .into_iter()
            .map(|proposal| Attributed {
                proposal,
                // A project rule has no single session. The nil UUID is the
                // codebase's existing name for an unattributed act.
                decided_by_session: Uuid::nil(),
            }),
    );

    // Everything Cairn's own rules can derive from exactly this evidence, which
    // is what gate 5a re-derives a proposal's keys against.
    let mut derivable =
        extract::rederive_session_keys(extract::SessionRef(claim.session_id), &events);
    derivable.extend(extract::rederive_project_keys(project, &project_sessions));

    let verified = verify_sources(pool, claim, &proposed).await?;

    let mut tx = pool.begin().await?;
    let mut accepted = 0i32;
    let mut refused = refusals.len() as i32;
    let proposed_count = (proposed.len() + refusals.len()) as i32;
    let mut settled: BTreeSet<(String, String)> = BTreeSet::new();

    for (reason, digest_of) in &refusals {
        record_refusal(&mut tx, claim, reason, digest_of, None).await?;
    }

    for attributed in &proposed {
        let verdict = govern(
            &mut tx,
            claim,
            account_id,
            attributed,
            &verified,
            &derivable,
            &mut settled,
        )
        .await?;
        if verdict.is_durable() {
            accepted += 1;
        }
        if verdict.refusal().is_some() {
            refused += 1;
        }
        record_candidate(&mut tx, claim, attributed, &verdict, &verified).await?;
    }

    sqlx::query(COUNT_RUN)
        .bind(claim.run_id)
        .bind(proposed_count)
        .bind(accepted)
        .bind(refused)
        .bind(extractor.kind())
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Every gate, in the fixed order §5 states, for one proposal.
///
/// Failing any gate refuses the candidate and persists nothing for it. The
/// order is not an implementation detail: gate 2 must run before gate 6 because
/// identity is the normalized key, and gate 5a must run before gate 6 because
/// otherwise the extractor would choose which existing record gets reinforced —
/// a well-formed proposal whose keys happen to match a high-value record would
/// produce a durable reinforcement a null extractor would not, which is
/// precisely the difference SC-742 measures.
#[allow(clippy::too_many_arguments)]
async fn govern(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    claim: &Claim,
    account_id: Uuid,
    attributed: &Attributed,
    verified: &Verified,
    derivable: &BTreeSet<(String, String)>,
    settled: &mut BTreeSet<(String, String)>,
) -> Result<Verdict, sqlx::Error> {
    let proposal = &attributed.proposal;

    // Gate 0 — shape. Not one of the ten, and deliberately before them: a
    // proposal with no content or no citation has nothing for a gate to judge.
    if proposal.check_shape().is_err() {
        return Ok(Verdict::Refused {
            reason: refusal::EXTRACTOR_MALFORMED_OUTPUT,
        });
    }

    // Gate 1 — source verification (FR-805c).
    if proposal.source_event_ids.is_empty()
        || !proposal
            .source_event_ids
            .iter()
            .all(|id| verified.accepts(attributed, id))
    {
        return Ok(Verdict::Refused {
            reason: refusal::UNVERIFIABLE_SOURCE,
        });
    }

    // Gate 2 — key normalization. Refused, never repaired: a repaired key
    // silently changes which existing knowledge the candidate collides with
    // (FR-796b).
    let Ok((Some(topic_key), Some(value_key))) =
        normalize_candidate_keys(Some(&proposal.topic_key), Some(&proposal.value_key))
    else {
        return Ok(Verdict::Refused {
            reason: refusal::KEY_NORMALIZATION_FAILED,
        });
    };

    // Gate 3 — privacy, through the same single implementation that governs any
    // other content (FR-759, FR-760).
    if validate_candidate_content(&proposal.content, Some(&topic_key), Some(&value_key), &[])
        .is_err()
    {
        return Ok(Verdict::Refused {
            reason: refusal::PRIVACY_REFUSED,
        });
    }

    // Gates 4 and 5 — domain, scope and ownership, resolved by Cairn.
    //
    // A batch is one session's events in one project, and project-session
    // activity establishes knowledge about that project and nothing else. So
    // the resolved domain is always `project`, the scope is always the project,
    // and `proposed_domain` is recorded as the advisory value it is — an
    // extractor claiming `personal` or `team` changes nothing, which is the
    // outcome SC-742 requires. Ownership therefore never leaves the project
    // domain either; the account binding is still read from the events rather
    // than from the proposal, because the rule that identity is established and
    // not asserted has to hold in the code and not only in the outcome.
    let _resolved_owner = account_id;

    // Gate 5a — key ↔ evidence correspondence. Cairn must be able to re-derive
    // the proposed pair with its own rules from the cited events.
    let rederived = derivable.contains(&(proposal.topic_key.clone(), proposal.value_key.clone()))
        || derivable.iter().any(|(t, v)| {
            normalize_candidate_keys(Some(t), Some(v))
                .map(|(t, v)| (t, v) == (Some(topic_key.clone()), Some(value_key.clone())))
                .unwrap_or(false)
        });
    if !rederived {
        return Ok(Verdict::Refused {
            reason: refusal::KEY_NOT_DERIVABLE,
        });
    }

    // Two proposals with one normalized identity are one candidate. Without
    // this the second would collide on `UNIQUE (run_id, topic_key, value_key)`
    // and fail the whole transaction, taking the batch with it.
    if !settled.insert((topic_key.clone(), value_key.clone())) {
        return Ok(Verdict::Duplicate);
    }

    // Gate 6 — duplicate and reinforcement, on normalized-key identity.
    //
    // **A candidate never reinforces itself.** A record this candidate already
    // created carries this candidate's own deterministic id, and a re-executed
    // batch derives the same proposal again — so without this exclusion a
    // session that crashed after its governance transaction committed would be
    // reclaimed, find its own creation, and corroborate it: a second record, a
    // second relation and an increment that no second occurrence produced.
    // Re-execution after an abandoned claim is expected; a second durable
    // effect from it is the thing SC-703 and SC-739 forbid.
    //
    // The asymmetry is worth naming, because it is why the defect was invisible
    // on half the paths: a *reinforcing* session's reclaim was already safe,
    // since its corroboration endpoint is excluded from this lookup by
    // `origin_kind`. Only a creating session's reclaim was exposed.
    let own = candidate_id(
        claim.project_id,
        attributed.decided_by_session,
        Some(&topic_key),
        Some(&value_key),
    );
    let existing = project_by_topic(tx, claim.project_id, &topic_key).await?;
    let exact = existing
        .iter()
        .find(|row| row.id != own && matches_value_key(row.value_key.as_deref(), &value_key));

    if let Some(row) = exact {
        let knowledge_id = reinforce(tx, claim, attributed, &topic_key, &value_key, row).await?;
        return Ok(Verdict::Reinforced { knowledge_id });
    }

    // The same subject and value in a domain consolidation may not write to.
    // Personal and team knowledge are not project knowledge and the record
    // would be legible to different people, but the *claim* is already held and
    // already maintained; creating a second place to maintain it is what an
    // automatic process should not do unasked. An explicit `cairn_remember`
    // still can, which is the difference between a decision somebody took and
    // one nobody did.
    if subject_elsewhere(tx, account_id, &topic_key)
        .await?
        .iter()
        .any(|(_, stored, _)| matches_value_key(stored.as_deref(), &value_key))
    {
        return Ok(Verdict::Duplicate);
    }

    // A claim this account can already read, stated under a different key.
    // Exact normalized-content equality decides; the text index only narrows
    // the search. That is syntactic, not a similarity judgement — two contents
    // are equal or they are unrelated, and there is no third answer for an
    // automatic decision to get wrong (FR-796c, FR-806).
    if already_stated(tx, claim.project_id, account_id, &proposal.content).await? {
        return Ok(Verdict::Duplicate);
    }

    // Gates 8, 9 and 10 have nothing to do here and that is the point: this
    // path creates a `project` record in the `active` state with no
    // verification, no supersession and no team state. Consolidation may not
    // supersede anything (FR-800), may not assert verification (FR-811), and
    // may not ratify team guidance (FR-809) — and it cannot, because there is
    // no branch here that writes any of them.
    let knowledge_id = create(tx, claim, attributed, &topic_key, &value_key).await?;

    // Gate 7 — conflict. Same subject, overlapping scope, a different value.
    // Recorded, never resolved (FR-799). Every record here is project-scoped,
    // so the scopes overlap by construction.
    let mut conflicted = false;
    for row in &existing {
        if matches_value_key(row.value_key.as_deref(), &value_key) {
            continue;
        }
        let (from, to) =
            normalize_relation_endpoints(RelationKind::ConflictsWith, knowledge_id, row.id);
        sqlx::query(INSERT_RELATION)
            .bind(from)
            .bind(to)
            .bind(RelationKind::ConflictsWith.as_str())
            .bind(claim.project_id)
            .bind(attributed.decided_by_session)
            .bind(RelationBasis::DeterministicRule.as_str())
            .execute(&mut **tx)
            .await?;
        conflicted = true;
    }

    Ok(if conflicted {
        Verdict::Conflicted { knowledge_id }
    } else {
        Verdict::Accepted { knowledge_id }
    })
}

/// Whether a stored value key and a canonical one name the same value.
///
/// Through `value_keys_agree`, not string equality, because a record written
/// before the separator-folding rule carries its key in the old spelling and is
/// the same value (T015). Comparing literally would make a legacy record and
/// its own canonical form read as two values, and the candidate would conflict
/// with itself.
fn matches_value_key(stored: Option<&str>, canonical: &str) -> bool {
    match stored {
        Some(stored) => value_keys_agree(stored, canonical),
        None => false,
    }
}

/// Persist a new project memory, deterministically named.
///
/// The record's id **is** the candidate id, so a re-executed batch upserts the
/// same row rather than adding a second. The identity is derived from the
/// project, the session and the normalized keys and deliberately not from the
/// event set, which is not stable across a reclaim (FR-798c).
async fn create(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    claim: &Claim,
    attributed: &Attributed,
    topic_key: &str,
    value_key: &str,
) -> Result<Uuid, sqlx::Error> {
    let id = candidate_id(
        claim.project_id,
        attributed.decided_by_session,
        Some(topic_key),
        Some(value_key),
    );
    // `ON CONFLICT DO NOTHING`, so a re-executed batch that already created
    // this record writes nothing rather than a second copy of it.
    sqlx::query(INSERT_MEMORY)
        .bind(id)
        .bind(claim.project_id)
        .bind(attributed.proposal.kind.as_str())
        .bind(claim.project_id.to_string())
        .bind(&attributed.proposal.content)
        .bind(attributed.decided_by_session)
        .bind(topic_key)
        .bind(value_key)
        // Distinguishes knowledge nobody asked for from knowledge somebody
        // created (FR-816).
        .bind("consolidated")
        .execute(&mut **tx)
        .await?;
    Ok(id)
}

/// Reinforce an existing record from a persisted corroboration endpoint.
///
/// Reinforcement is a relation between two durable records, so the candidate
/// needs an endpoint to reinforce *from* (FR-798a). The endpoint is a row in
/// the same table as the knowledge it corroborates, marked `corroboration` —
/// one marker that carries every rule the requirement needs: recall excludes
/// it, counts exclude it, and its identity is deterministic, so a re-executed
/// batch upserts the same row instead of adding a second.
///
/// The reinforcement count is bumped **only when the relation was actually
/// inserted**. Incrementing unconditionally would make a re-execution visibly
/// different from a single run, which is exactly what SC-703 forbids; the
/// relation's primary key is what makes the guard exact.
async fn reinforce(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    claim: &Claim,
    attributed: &Attributed,
    topic_key: &str,
    value_key: &str,
    existing: &Existing,
) -> Result<Uuid, sqlx::Error> {
    let endpoint = corroboration_id(
        claim.project_id,
        attributed.decided_by_session,
        Some(topic_key),
        Some(value_key),
    );
    sqlx::query(INSERT_MEMORY)
        .bind(endpoint)
        .bind(claim.project_id)
        .bind(attributed.proposal.kind.as_str())
        .bind(claim.project_id.to_string())
        .bind(&attributed.proposal.content)
        .bind(attributed.decided_by_session)
        .bind(topic_key)
        .bind(value_key)
        .bind("corroboration")
        .execute(&mut **tx)
        .await?;

    let inserted = sqlx::query(INSERT_RELATION)
        .bind(endpoint)
        .bind(existing.id)
        .bind(RelationKind::Reinforces.as_str())
        .bind(claim.project_id)
        .bind(attributed.decided_by_session)
        // The one basis only consolidation can produce, so an inferred
        // reinforcement stays distinguishable from one a human asked for
        // (FR-801a, FR-802).
        .bind(RelationBasis::ConsolidationReinforcement.as_str())
        .execute(&mut **tx)
        .await?
        .rows_affected();

    if inserted == 1 {
        sqlx::query(BUMP_REINFORCEMENT)
            .bind(existing.id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(existing.id)
}

/// Whether knowledge this account can read already states this claim.
///
/// All three domains, because the question is about the reader and not about
/// the storage: a project record restating what the account already holds
/// personally, or what the team already holds authoritatively, adds a second
/// place to maintain one claim and no new reader for it. Consolidation, which
/// nobody asked for, should not create one; an explicit `cairn_remember` still
/// can.
///
/// The three full-text indexes narrow the search and the normalized-content
/// comparison decides it. No embeddings and no similarity model: FR-796c is
/// explicit that identity is a deterministic syntactic function.
async fn already_stated(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    project_id: Uuid,
    account_id: Uuid,
    content: &str,
) -> Result<bool, sqlx::Error> {
    let wanted = normalize_content(content);
    if wanted.is_empty() {
        return Ok(false);
    }

    let project: Vec<(Uuid, String)> = sqlx::query_as(PROJECT_BY_TEXT)
        .bind(project_id)
        .bind(content)
        .bind(TEXT_MATCH_LIMIT)
        .fetch_all(&mut **tx)
        .await?;
    let personal: Vec<(Uuid, String)> = sqlx::query_as(PERSONAL_BY_TEXT)
        .bind(account_id)
        .bind(content)
        .bind(TEXT_MATCH_LIMIT)
        .fetch_all(&mut **tx)
        .await?;
    let team: Vec<(Uuid, String)> = sqlx::query_as(TEAM_BY_TEXT)
        .bind(content)
        .bind(TEXT_MATCH_LIMIT)
        .fetch_all(&mut **tx)
        .await?;

    Ok(project
        .into_iter()
        .chain(personal)
        .chain(team)
        .any(|(_, found)| normalize_content(&found) == wanted))
}

/// Existing project knowledge under one subject, and the same subject in the
/// domains this account can read.
///
/// The exact normalized-key path, in all three domains (T055, FR-806). The
/// project rows drive reinforcement and conflict; the personal and team rows
/// are read for the same subject so a later resolver has one place to consult
/// rather than three, and so the indexes this feature added are exercised by
/// the code that needed them.
async fn project_by_topic(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    project_id: Uuid,
    topic_key: &str,
) -> Result<Vec<Existing>, sqlx::Error> {
    let rows: Vec<(Uuid, Option<String>, String)> = sqlx::query_as(PROJECT_BY_TOPIC)
        .bind(project_id)
        .bind(topic_key)
        .fetch_all(&mut **tx)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(id, value_key, _content)| Existing { id, value_key })
        .collect())
}

/// The same subject in the personal and team domains, for a caller that needs
/// to know whether a claim is already governed somewhere Cairn may not write.
///
/// Consolidation may not create personal knowledge for another user and may not
/// ratify team guidance, so this is a read and never a write path.
async fn subject_elsewhere(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: Uuid,
    topic_key: &str,
) -> Result<Vec<(Uuid, Option<String>, String)>, sqlx::Error> {
    let mut found: Vec<(Uuid, Option<String>, String)> = sqlx::query_as(PERSONAL_BY_TOPIC)
        .bind(account_id)
        .bind(topic_key)
        .fetch_all(&mut **tx)
        .await?;
    let team: Vec<(Uuid, Option<String>, String)> = sqlx::query_as(TEAM_BY_TOPIC)
        .bind(topic_key)
        .fetch_all(&mut **tx)
        .await?;
    found.extend(team);
    Ok(found)
}

/// Record what the extractor proposed and what Cairn decided about it.
///
/// Both halves, because "why is this knowledge here" should have an answer that
/// is not a guess. The keys stored are the **normalized** ones and never the
/// proposed ones: the normalized form is what reconciliation matched on, and
/// keeping the proposal would leave two rows disagreeing about what one subject
/// is called.
async fn record_candidate(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    claim: &Claim,
    attributed: &Attributed,
    verdict: &Verdict,
    verified: &Verified,
) -> Result<(), sqlx::Error> {
    let proposal = &attributed.proposal;

    if let Some(reason) = verdict.refusal() {
        return record_refusal(tx, claim, reason, &proposal.content, Some(proposal)).await;
    }

    let (topic_key, value_key) =
        match normalize_candidate_keys(Some(&proposal.topic_key), Some(&proposal.value_key)) {
            Ok((t, v)) => (t, v),
            // Unreachable: a proposal whose keys do not normalize was refused at
            // gate 2 and returned above. Recording it as a refusal rather than
            // panicking keeps a future reordering of the gates from taking the
            // whole batch down with it.
            Err(_) => {
                return record_refusal(
                    tx,
                    claim,
                    refusal::KEY_NORMALIZATION_FAILED,
                    &proposal.content,
                    Some(proposal),
                )
                .await
            }
        };

    let id = candidate_id(
        claim.project_id,
        attributed.decided_by_session,
        topic_key.as_deref(),
        value_key.as_deref(),
    );
    sqlx::query(INSERT_CANDIDATE)
        .bind(id)
        .bind(claim.run_id)
        .bind(proposal.kind.as_str())
        // Advisory, and recorded as proposed so the difference between what the
        // extractor wanted and what Cairn resolved stays visible.
        .bind(proposal.proposed_domain.as_str())
        .bind(&topic_key)
        .bind(&value_key)
        .bind(&proposal.content)
        .bind(verdict.decision())
        .bind(Option::<String>::None)
        .bind(verdict.knowledge_id().map(|_| "knowledge"))
        .bind(verdict.knowledge_id().map(|_| "project"))
        .bind(verdict.knowledge_id())
        .execute(&mut **tx)
        .await?;

    // Evidence is additive: a re-execution that saw more events adds rows here
    // without changing which candidate they belong to (FR-798c).
    for event_id in proposal
        .source_event_ids
        .iter()
        .filter(|e| verified.accepts(attributed, e))
    {
        sqlx::query(INSERT_SOURCE_EVENT)
            .bind(id)
            .bind(event_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Record a refusal, carrying the reason and never the content.
///
/// The identity is derived from the reason and a **digest** of the proposal
/// rather than from the key pair, so several distinct malformed proposals in
/// one session record several distinct refusals. Deriving them from the keys
/// would collapse every `key_normalization_failed` in a session onto one row
/// and undercount refusals, which FR-807 and SC-705 depend on being accurate.
///
/// `content` is written as the empty string. The column is `NOT NULL` and a
/// refusal record must not carry the material that caused it (FR-741, SC-705);
/// an empty string satisfies the constraint and carries nothing.
async fn record_refusal(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    claim: &Claim,
    reason: &str,
    digest_of: &str,
    proposal: Option<&CandidateProposal>,
) -> Result<(), sqlx::Error> {
    let id = refusal_id(
        claim.project_id,
        claim.session_id,
        reason,
        &proposal_digest(digest_of),
    );
    sqlx::query(INSERT_CANDIDATE)
        .bind(id)
        .bind(claim.run_id)
        .bind(proposal.map_or(MemoryType::Fact.as_str(), |p| p.kind.as_str()))
        .bind(proposal.map_or(KnowledgeDomain::Project.as_str(), |p| {
            p.proposed_domain.as_str()
        }))
        // No keys. A refused candidate has none Cairn is willing to stand
        // behind, and a privacy refusal's keys would name the subject of the
        // content the refusal exists to keep out.
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind("")
        .bind("refused")
        .bind(reason)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<Uuid>::None)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Which cited event ids exist in this project and session (gate 1).
///
/// One query for the union of everything cited, rather than one per proposal: a
/// pass proposes at most a few dozen candidates and the answer is the same set
/// for all of them.
async fn verify_sources(
    pool: &PgPool,
    claim: &Claim,
    proposed: &[Attributed],
) -> Result<Verified, sqlx::Error> {
    let cited = |session_scoped: bool| -> Vec<Uuid> {
        proposed
            .iter()
            .filter(|a| (a.decided_by_session != Uuid::nil()) == session_scoped)
            .flat_map(|a| a.proposal.source_event_ids.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    };

    let in_session = cited(true);
    let session = if in_session.is_empty() {
        BTreeSet::new()
    } else {
        sqlx::query_scalar(VERIFY_SESSION_SOURCES)
            .bind(claim.project_id)
            .bind(claim.session_id)
            .bind(&in_session)
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect()
    };

    let in_project = cited(false);
    let project = if in_project.is_empty() {
        BTreeSet::new()
    } else {
        sqlx::query_scalar(VERIFY_PROJECT_SOURCES)
            .bind(claim.project_id)
            .bind(&in_project)
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect()
    };

    Ok(Verified { session, project })
}

/// The citations gate 1 accepted, by the scope each rule tier is verified in.
///
/// Two sets rather than one, because the two tiers are answerable for different
/// things: a session rule may cite only what it was handed, and a project rule
/// may cite anything in the project it aggregated over. Merging them would let
/// a session-rule proposal borrow a project-rule citation's verification, which
/// is the widening gate 1 exists to prevent.
struct Verified {
    session: BTreeSet<Uuid>,
    project: BTreeSet<Uuid>,
}

impl Verified {
    fn accepts(&self, attributed: &Attributed, event_id: &Uuid) -> bool {
        if attributed.decided_by_session == Uuid::nil() {
            self.project.contains(event_id)
        } else {
            self.session.contains(event_id)
        }
    }
}

/// One `safe_events` row as the database hands it back.
///
/// Named rather than repeated at two call sites: the tuple is wide because the
/// event is reassembled from its columns, and two spellings of the same wide
/// tuple is two places for a column to be added to only one of.
type EventRow = (
    Uuid,
    Uuid,
    String,
    String,
    Option<String>,
    i64,
    i32,
    Option<serde_json::Value>,
    chrono::DateTime<chrono::Utc>,
);

/// One row of `safe_events`, rebuilt into the event it was accepted as.
struct BatchEvent {
    account_id: Uuid,
    event: SafeCanonicalEvent,
}

async fn read_batch_events(pool: &PgPool, claim: &Claim) -> Result<Vec<BatchEvent>, sqlx::Error> {
    let rows: Vec<EventRow> = sqlx::query_as(READ_BATCH_EVENTS)
        .bind(claim.project_id)
        .bind(claim.session_id)
        .bind(&claim.events)
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .filter_map(
            |(event_id, account_id, agent, kind, vendor_event, seq, version, content, at)| {
                rebuild(
                    event_id,
                    claim.session_id,
                    agent,
                    kind,
                    vendor_event,
                    seq,
                    version,
                    content,
                    at,
                )
                .map(|event| BatchEvent { account_id, event })
            },
        )
        .collect())
}

async fn read_project_events(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<extract::SessionEvents>, sqlx::Error> {
    let rows: Vec<EventRow> = sqlx::query_as(READ_PROJECT_EVENTS)
        .bind(project_id)
        .bind(extract::AGGREGATE_MAX_SESSIONS)
        .bind(extract::AGGREGATE_MAX_EVENTS)
        .fetch_all(pool)
        .await?;

    // A read that hit its cap may have stopped in the middle of a session, and
    // a half-read session is worse than an unread one: R5 matches an *identical
    // ordered sequence*, so a truncated one is a procedure that never happened.
    // The last session is therefore dropped whenever the cap was reached, which
    // is the same judgement the rule already makes about a sequence too long to
    // render — half a procedure is a wrong procedure.
    let truncated = rows.len() as i64 == extract::AGGREGATE_MAX_EVENTS;

    let mut sessions: Vec<extract::SessionEvents> = Vec::new();
    for (session_id, event_id, agent, kind, vendor_event, seq, version, content, at) in rows {
        let Some(event) = rebuild(
            event_id,
            session_id,
            agent,
            kind,
            vendor_event,
            seq,
            version,
            content,
            at,
        ) else {
            continue;
        };
        match sessions.last_mut() {
            Some(last) if last.session_ref.0 == session_id => last.events.push(event),
            _ => sessions.push(extract::SessionEvents {
                session_ref: extract::SessionRef(session_id),
                events: vec![event],
            }),
        }
    }
    if truncated {
        sessions.pop();
    }
    Ok(sessions)
}

/// Rebuild a stored row into the closed event type it was accepted as.
///
/// Through the type's own deserializer rather than field by field, so a row
/// that could not be an accepted event — an unknown kind, content that does not
/// belong to it — is skipped rather than reconstructed into something the
/// schema would have refused. Skipping is right: an event nothing can parse is
/// evidence for nothing, and consolidating around it would put a claim in
/// durable memory that its own provenance cannot explain.
#[allow(clippy::too_many_arguments)]
fn rebuild(
    event_id: Uuid,
    session_id: Uuid,
    agent: String,
    kind: String,
    vendor_event: Option<String>,
    session_seq: i64,
    contract_version: i32,
    content: Option<serde_json::Value>,
    occurred_at: chrono::DateTime<chrono::Utc>,
) -> Option<SafeCanonicalEvent> {
    let mut value = serde_json::json!({
        "event_id": event_id,
        "contract_version": contract_version,
        "kind": kind,
        "agent": agent,
        "vendor_event": vendor_event,
        "session_id": session_id,
        "session_seq": session_seq,
        "occurred_at": occurred_at,
    });
    if let Some(content) = content {
        if !content.is_null() {
            value["content"] = content;
        }
    }
    serde_json::from_value(value).ok()
}

// ---------------------------------------------------------------------------
// Observability (T056, §8, FR-793c, FR-807, FR-813, SC-748)
// ---------------------------------------------------------------------------

/// What a health report can say about consolidation at any moment.
///
/// Readable mid-pass and immediately after a restart, because every field is a
/// committed row rather than worker state (SC-748). A backlog is never reported
/// to a client as an ingestion failure and never applies back-pressure to
/// ingest: nothing in ingest reads any of this (FR-814).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConsolidationHealth {
    /// Events accepted and not yet consolidated.
    pub backlog_depth: i64,
    /// When the oldest of them was enqueued, or absent when there is no
    /// backlog. Absent is a different answer from zero and is reported as one.
    pub oldest_enqueued_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Events that exhausted their attempts. Never retried automatically, and
    /// visible here so that is a fact somebody can see rather than infer.
    pub failed_events: i64,
    /// Passes that ran and passes that produced nothing, since the server
    /// started serving this database.
    pub runs_finished: i64,
    pub runs_failed: i64,
    /// Candidates the runs proposed, accepted and refused.
    pub candidates_proposed: i64,
    pub candidates_accepted: i64,
    pub candidates_refused: i64,
}

const HEALTH: &str = "\
SELECT
  (SELECT count(*) FROM consolidation_work WHERE state = 'pending'),
  (SELECT min(enqueued_at) FROM consolidation_work WHERE state = 'pending'),
  (SELECT count(*) FROM consolidation_work WHERE state = 'failed'),
  (SELECT count(*) FROM consolidation_runs WHERE state = 'finished'),
  (SELECT count(*) FROM consolidation_runs WHERE state = 'failed'),
  (SELECT coalesce(sum(candidates_proposed), 0) FROM consolidation_runs),
  (SELECT coalesce(sum(candidates_accepted), 0) FROM consolidation_runs),
  (SELECT coalesce(sum(candidates_refused), 0) FROM consolidation_runs)";

/// Read consolidation's health, whether or not a pass is running.
///
/// Deliberately not guarded on a lease or a worker: the question "how far
/// behind is consolidation" must be answerable while a pass holds one, which is
/// exactly when somebody asks it.
pub async fn health(pool: &PgPool) -> Result<ConsolidationHealth, sqlx::Error> {
    let row: (
        i64,
        Option<chrono::DateTime<chrono::Utc>>,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(HEALTH).fetch_one(pool).await?;
    Ok(ConsolidationHealth {
        backlog_depth: row.0,
        oldest_enqueued_at: row.1,
        failed_events: row.2,
        runs_finished: row.3,
        runs_failed: row.4,
        candidates_proposed: row.5,
        candidates_accepted: row.6,
        candidates_refused: row.7,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refusal_vocabulary_is_exactly_the_one_the_contract_fixes() {
        // FR-804a requires a fixed, enumerated vocabulary distinct from the
        // event-rejection one. Nine terms, `contracts/consolidation.md` §9.
        assert_eq!(refusal::ALL.len(), 9);
        let unique: std::collections::BTreeSet<&&str> = refusal::ALL.iter().collect();
        assert_eq!(unique.len(), 9, "two refusal reasons share a name");
        for expected in [
            refusal::KEY_NORMALIZATION_FAILED,
            refusal::KEY_NOT_DERIVABLE,
            refusal::PRIVACY_REFUSED,
            refusal::UNVERIFIABLE_SOURCE,
            refusal::CONFLICTS_WITH_EXISTING,
            refusal::BOUND_EXCEEDED,
            refusal::EXTRACTOR_MALFORMED_OUTPUT,
        ] {
            assert!(
                refusal::ALL.contains(&expected),
                "{expected} is not in the vocabulary"
            );
        }
        // An extraction error reports under the same vocabulary, so a run's
        // refusal counts cannot be split across two spellings of one reason.
        for e in [
            crate::extract::ExtractError::BoundExceeded,
            crate::extract::ExtractError::MalformedOutput,
        ] {
            assert!(
                refusal::ALL.contains(&e.reason()),
                "{} is not in the vocabulary",
                e.reason()
            );
        }
    }

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
