//! The edge spools: approved events and knowledge commands waiting for the
//! server (`contracts/safe-events.md` §8, `contracts/knowledge-commands.md`
//! §4, `data-model.md` §1.4 and §5).
//!
//! Two tables, one protocol. [`crate::outbox`] already solved per-author
//! claiming, stale-claim reclaim and backoff for Feature 004's sync queue, and
//! §8 of the contract is explicit that the spool reuses those semantics rather
//! than growing a second, subtly different set. What is genuinely new here is
//! everything the outbox never needed: an identity assigned from a durable
//! ordinal, and a capacity policy that may shed a capture event but must never
//! shed a boundary one.
//!
//! # Identity is assigned here, exactly once
//!
//! A hook is a separate short-lived process and cannot hold a counter; the
//! daemon can. So the ordinal comes from `session_event_seq` (events) or
//! `command_seq` (commands), is consumed inside the same transaction that
//! inserts the row, and the identity is derived from it by
//! [`cairn_core::eventid`]. Nothing recomputes it afterwards, so any number of
//! delivery retries carry the same id and the server answers `duplicate`
//! rather than storing a second event (FR-770).
//!
//! **The counter is never `MAX(session_seq)` over the spool.** The spool
//! drains, and sheds rows under the capacity policy below. A counter recovered
//! from the spool restarts at 1 the moment delivery empties a session's rows,
//! re-derives an `event_id` a delivered event already used, and the server —
//! being idempotent on that id — answers `duplicate` and silently discards a
//! real event. That is the failure this module's whole ordinal arrangement
//! exists to make impossible, so there is no statement here that reads an
//! ordinal from either spool table.
//!
//! # The account match is exact
//!
//! Rows stay bound to the account that authored them: a different signed-in
//! account neither delivers them nor sees them. Every claim predicate here
//! says `account_id = ?` and nothing else. A predicate of the shape
//! `account_id IS NULL OR account_id = ?` is how one identity comes to deliver
//! another's work under its own credential, and this repository has introduced
//! and repaired exactly that regression twice (FR-790, FR-864a). `account_id`
//! is `NOT NULL` in schema v8 so there is no NULL to open the door with, and
//! the predicates below do not offer one either.
//!
//! # Capacity, and the one event class that is never shed
//!
//! 50,000 events or 256 MiB, whichever binds first (`data-model.md` §3). On
//! overflow the oldest **capture-class** rows go first, each counted as
//! `spool_overflow_dropped`. When the bound is reached and nothing
//! capture-class is left, the spool does **not** start shedding boundary rows:
//! session open, close, resume and the two compaction events are what every
//! other event is interpreted relative to, so dropping one would not lose an
//! event, it would corrupt the reading of everything still queued. Instead the
//! store enters saturation, refuses new events, and counts each refusal as
//! `spool_saturated_dropped` (FR-785).
//!
//! Saturation is **derived**, never stored: it is the question "is the bound
//! reached and is nothing capture-class left", asked of the table. A column
//! would be a second copy of an answer the rows already give, and it would
//! have to be cleared by whoever drained the spool — which is precisely the
//! bookkeeping a derived predicate cannot forget to do. It clears the moment
//! delivery moves rows out of the undelivered set.

use crate::{rows, tx, Result, Store, StoreError};
use cairn_core::event::{Disposition, EventKind, SafeCanonicalEvent};
use cairn_core::eventid::{command_id, event_id};
use chrono::{DateTime, Utc};
use sqlx::Row;
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Bounds and leases
// ---------------------------------------------------------------------------

/// The `last_error_kind` a row carries when it stopped being retried because
/// its attempt budget ran out, rather than because the server refused it.
///
/// Both land in `refused` — the consequence is identical, and schema v8 has
/// five states, not six — but the *cause* differs, and a person looking at a
/// stuck spool needs to know whether the server said no or was simply never
/// there. A fixed term rather than free text, like every other reason in this
/// feature.
pub const TERMINAL_RETRY_EXHAUSTED: &str = "retry_exhausted";

// ---------------------------------------------------------------------------
// The state machine
// ---------------------------------------------------------------------------
//
// Five states, and their meanings are **not** the outbox's, despite three of
// the names matching. The outbox has no `refused` and no `delivered`, so its
// `failed` had to carry the terminal meaning; here `refused` carries it and
// `failed` is free to mean something the outbox could not express. Reading
// across from `outbox.rs` on the strength of the shared name is the specific
// mistake this comment exists to prevent.
//
// | state       | meaning                                            | claimable |
// |-------------|----------------------------------------------------|-----------|
// | `pending`   | spooled, never attempted, due now                  | yes       |
// | `in_flight` | claimed by a drainer; the lease may go stale        | on expiry |
// | `failed`    | attempted, transport or server-transient, in backoff | when due |
// | `delivered` | the server accepted it, or answered `duplicate`      | no        |
// | `refused`   | terminal and visible: the server refused it permanently, or its attempt budget ran out | never |
//
// - **Transient stays retryable, with bounded backoff and a bounded number of
//   attempts** (FR-784). `failed` is the retrying state, which is the opposite
//   of what the same word means in `outbox.rs`.
// - **Permanent refusals never retry.** No claim predicate here admits
//   `refused`, so the row is retried exactly zero times and stays where a
//   person can see it (FR-772).
// - **A `duplicate` answer is a success.** It is what a retry was *for*: at
//   most one canonical event exists, so the row is marked `delivered`
//   (FR-770, FR-786).
// - **Stale claims are reclaimable.** An `in_flight` row whose lease expired is
//   claimable again, so a drainer that died mid-send strands nothing.
// - **Health distinguishes four conditions**, not two: see [`SpoolBreakdown`].

/// The states a spooled row is still work in (`data-model.md` §5).
///
/// Repeated verbatim in every statement below rather than interpolated from a
/// constant, because these lists are also what SQLite plans the claim and
/// capacity indexes against, and a `format!`-assembled predicate is one
/// refactor away from being assembled from something a caller supplied.
///
/// `delivered` is absent because a delivered row is no longer queued — that is
/// what makes saturation clear itself when the spool drains. `refused` is
/// absent because a permanent refusal has to stay visible (FR-772, FR-784),
/// and a row inside the capacity set is a row the overflow policy may delete:
/// counting refusals toward the bound would let the policy shed the very
/// record the refusal exists to show someone.
const UNDELIVERED: &str = "('pending','in_flight','failed')";

/// Events, and total payload bytes, the spool may hold before the overflow
/// policy runs (`data-model.md` §3, FR-785).
pub const SPOOL_MAX_EVENTS: i64 = 50_000;
/// 256 MiB.
pub const SPOOL_MAX_BYTES: i64 = 256 * 1024 * 1024;

/// How long a claim may sit unacknowledged before another drainer may take it.
///
/// The same sixty seconds [`crate::outbox::CLAIM_TIMEOUT_SECONDS`] uses, and
/// for the same reason: a drainer that dies mid-send leaves rows `in_flight`
/// with nothing left to acknowledge them, and a lease that simply expires is
/// cheaper and harder to get wrong than a liveness protocol. Sixty seconds is
/// comfortably longer than the sync client's twenty-second request timeout, so
/// a drainer merely waiting on a slow server is never overtaken by one that
/// assumed it died.
pub const CLAIM_LEASE_SECONDS: i64 = 60;

/// The first retry waits one second (`contracts/safe-events.md` §8).
pub const BACKOFF_FLOOR_SECONDS: i64 = 1;
/// …and no retry ever waits more than five minutes.
pub const BACKOFF_CEILING_SECONDS: i64 = 300;

/// How many delivery attempts a row gets before it is declared undeliverable
/// (FR-784).
///
/// FR-784 has two clauses joined by "and": retry "MUST be bounded" **and**
/// "MUST back off", and a permanently undeliverable event "MUST become visible
/// rather than being retried forever". Bounded backoff alone satisfies the
/// second and not the first — a row retrying every five minutes until the heat
/// death of the machine has bounded *delay* and unbounded *retry*, and nobody
/// is ever told it is not getting through.
///
/// **This is an attempt budget, not a wall-clock deadline, and the difference
/// is deliberate.** Nothing here reads a timestamp to decide that a row has
/// expired: a laptop closed for a fortnight comes back with every spooled event
/// still deliverable, because the device being off is not the server refusing.
/// FR-783 requires events spooled during an outage to be retried later, and an
/// elapsed-time expiry would quietly turn "later" into "not if you took a
/// holiday".
///
/// The number is chosen by asking how long a budget of this size *approximates*
/// once backoff has reached its five-minute ceiling and a drainer is actually
/// running: 2,016 attempts at five minutes apart is on the order of a week of
/// continuous, actively-retried failure. That is the reasoning behind the size,
/// not a guarantee about elapsed time — a machine that only runs for an hour a
/// day spends the same budget over much longer, which is the correct behaviour
/// and the reason it is counted in attempts.
pub const MAX_DELIVERY_ATTEMPTS: i64 = 2_016;

/// How long to wait before the `attempts`-th delivery attempt of a row.
///
/// Exponential from the floor, clamped at the ceiling. Bounded at both ends on
/// purpose: an unbounded doubling would eventually park a recoverable row for
/// hours after a brief outage, and a flat retry would hammer a server that is
/// down. The shift is clamped before it is taken, so a row with an absurd
/// attempt count saturates at the ceiling rather than overflowing into a
/// nonsensical delay.
pub fn backoff_seconds(attempts: i64) -> i64 {
    let steps = attempts.saturating_sub(1).clamp(0, 62) as u32;
    BACKOFF_FLOOR_SECONDS
        .checked_shl(steps)
        .unwrap_or(BACKOFF_CEILING_SECONDS)
        .min(BACKOFF_CEILING_SECONDS)
}

/// The capacity bound a spool write is measured against.
///
/// A parameter rather than a pair of constants read directly, so the policy
/// can be exercised against a spool of five rows instead of fifty thousand.
/// [`Default`] is the real bound and is what every production caller passes;
/// nothing here reads the constants behind the caller's back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpoolCapacity {
    pub max_events: i64,
    pub max_bytes: i64,
}

impl Default for SpoolCapacity {
    fn default() -> Self {
        Self {
            max_events: SPOOL_MAX_EVENTS,
            max_bytes: SPOOL_MAX_BYTES,
        }
    }
}

// ---------------------------------------------------------------------------
// Command vocabulary
// ---------------------------------------------------------------------------

/// The thirteen knowledge commands migration 8's `command_spool.kind` CHECK
/// admits (`data-model.md` §5).
///
/// A closed type rather than a bare string, so a caller cannot spool a command
/// the drain has no case for and discover it at the CHECK. `verification_run`
/// and `verification_attestation` are two report *shapes*, not two trust
/// levels: both are recorded `remote_attested`, because which route a report
/// arrived on says nothing about which verifier executed (FR-811b/FR-811h).
///
/// This mirrors the CHECK, which remains the authority. When the authoritative
/// command boundary lands it will want the same vocabulary on the wire; at
/// that point this belongs in `cairn-core` beside the event kinds, and this
/// definition should move rather than be duplicated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CommandKind {
    Remember,
    Supersede,
    Reinforce,
    Relate,
    Pin,
    Forget,
    PersonalCreate,
    PersonalForget,
    TeamPropose,
    PatternPromote,
    PatternForget,
    VerificationRun,
    VerificationAttestation,
}

impl CommandKind {
    pub const ALL: &'static [CommandKind] = &[
        CommandKind::Remember,
        CommandKind::Supersede,
        CommandKind::Reinforce,
        CommandKind::Relate,
        CommandKind::Pin,
        CommandKind::Forget,
        CommandKind::PersonalCreate,
        CommandKind::PersonalForget,
        CommandKind::TeamPropose,
        CommandKind::PatternPromote,
        CommandKind::PatternForget,
        CommandKind::VerificationRun,
        CommandKind::VerificationAttestation,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            CommandKind::Remember => "remember",
            CommandKind::Supersede => "supersede",
            CommandKind::Reinforce => "reinforce",
            CommandKind::Relate => "relate",
            CommandKind::Pin => "pin",
            CommandKind::Forget => "forget",
            CommandKind::PersonalCreate => "personal_create",
            CommandKind::PersonalForget => "personal_forget",
            CommandKind::TeamPropose => "team_propose",
            CommandKind::PatternPromote => "pattern_promote",
            CommandKind::PatternForget => "pattern_forget",
            CommandKind::VerificationRun => "verification_run",
            CommandKind::VerificationAttestation => "verification_attestation",
        }
    }
}

impl fmt::Display for CommandKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CommandKind {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        CommandKind::ALL
            .iter()
            .copied()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| format!("unknown command kind: {s}"))
    }
}

/// What a command's ordinal and identity are counted against.
///
/// Not every command has a session. An explicit `cairn remember` from the CLI
/// outside any session genuinely has no author to name, and the shipped code
/// already says so rather than inventing one; a synthetic session row would
/// leave a second active session in the worktree, which is exactly the
/// ambiguity that comment warns about. So a sessionless command is scoped to
/// the store itself, keyed by this store's durable `writer_id`, and its
/// `session_id` column stays NULL (`contracts/knowledge-commands.md` §4.1).
///
/// The kind is part of the identity derivation as well as the counter key,
/// because a session UUID and a writer UUID are different namespaces of key
/// that must not be able to collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandScope {
    Session(Uuid),
    Store(Uuid),
}

impl CommandScope {
    pub fn kind(&self) -> &'static str {
        match self {
            CommandScope::Session(_) => "session",
            CommandScope::Store(_) => "store",
        }
    }

    pub fn key(&self) -> Uuid {
        match self {
            CommandScope::Session(id) | CommandScope::Store(id) => *id,
        }
    }

    /// The `session_id` column's value: a session scope names its session, a
    /// store scope names none.
    pub fn session_id(&self) -> Option<Uuid> {
        match self {
            CommandScope::Session(id) => Some(*id),
            CommandScope::Store(_) => None,
        }
    }
}

/// This store's own writer identity, as the scope of a sessionless command.
///
/// Reads the one `writer_identity` row seeded at migration 7 and never
/// regenerated (D407, FR-490), so a store's sessionless command ordinals are
/// stable across restarts and across reinstalls of the daemon.
pub async fn store_scope(store: &Store) -> Result<CommandScope> {
    let row = sqlx::query("SELECT writer_id FROM writer_identity WHERE id = 1")
        .fetch_one(store.pool())
        .await?;
    Ok(CommandScope::Store(rows::uuid(&row, "writer_id")?))
}

// ---------------------------------------------------------------------------
// Disposition counts
// ---------------------------------------------------------------------------

/// One day's count of one thing happening to one kind of capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispositionCount {
    pub project_id: Uuid,
    pub agent: String,
    pub kind: String,
    pub disposition: Disposition,
    pub day: String,
    pub n: i64,
}

/// The UTC day a count is filed under.
fn today() -> String {
    chrono::Utc::now().date_naive().to_string()
}

/// Count one disposition, creating the day's row if this is its first.
///
/// Counts, not records: a disposition carries no payload content (FR-749d,
/// FR-741), so there is nothing to keep except how often it happened.
pub async fn record_disposition(
    store: &Store,
    project_id: Uuid,
    agent: &str,
    kind: &str,
    disposition: Disposition,
) -> Result<()> {
    let mut conn = store.pool().acquire().await?;
    count_disposition(&mut conn, project_id, agent, kind, disposition).await
}

async fn count_disposition(
    conn: &mut sqlx::SqliteConnection,
    project_id: Uuid,
    agent: &str,
    kind: &str,
    disposition: Disposition,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO capture_disposition_counts
            (project_id, agent, kind, disposition, day, n)
         VALUES (?1, ?2, ?3, ?4, ?5, 1)
         ON CONFLICT (project_id, agent, kind, disposition, day)
         DO UPDATE SET n = n + 1",
    )
    .bind(project_id.to_string())
    .bind(agent)
    .bind(kind)
    .bind(disposition.as_str())
    .bind(today())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Every disposition counted for one project, newest day first.
pub async fn disposition_counts(store: &Store, project_id: Uuid) -> Result<Vec<DispositionCount>> {
    let rs = sqlx::query(
        "SELECT project_id, agent, kind, disposition, day, n
           FROM capture_disposition_counts
          WHERE project_id = ?1
          ORDER BY day DESC, agent, kind, disposition",
    )
    .bind(project_id.to_string())
    .fetch_all(store.pool())
    .await?;
    rs.iter()
        .map(|r| {
            Ok(DispositionCount {
                project_id: rows::uuid(r, "project_id")?,
                agent: r.try_get("agent")?,
                kind: r.try_get("kind")?,
                disposition: rows::enum_val(r, "disposition")?,
                day: r.try_get("day")?,
                n: r.try_get("n")?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The event spool
// ---------------------------------------------------------------------------

/// An approved event on its way into the spool.
///
/// The event's own `event_id` and `session_seq` are **ignored and overwritten**
/// — identity is the store's to assign, inside the transaction, from the
/// durable ordinal. A caller that could choose them could choose a colliding
/// one, be answered `duplicate`, and suppress a genuine event.
///
/// `project_id` and `account_id` are separate from the event because neither
/// crosses the boundary in the event body: the project is routing, and the
/// account is authorization, which is never read from a payload (Principle XI).
#[derive(Debug, Clone)]
pub struct NewEvent {
    pub project_id: Uuid,
    pub account_id: Uuid,
    pub event: SafeCanonicalEvent,
    /// The server instance this row is queued **for** (FR-791), or `None` when
    /// this store has never established one.
    ///
    /// `None` is not a wildcard. It means "queued before there was an instance
    /// to name", and the claim never treats it as matching — the first drain
    /// against an established instance binds it, once. See
    /// [`claim_events`] for that rule and why it is the only safe one.
    pub server_instance_id: Option<Uuid>,
}

/// What the spool did with an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventAdmission {
    /// Spooled, with its identity now fixed forever.
    Spooled {
        event: SafeCanonicalEvent,
        /// How many capture-class rows the overflow policy shed to make room.
        overflow_dropped: u64,
    },
    /// Refused because the spool is saturated: the bound is reached and every
    /// row still queued is boundary-class.
    ///
    /// The kind and the session travel on the refusal rather than into the
    /// counts table, which is keyed by project, agent, kind and day and has no
    /// session column to record one in. The count is still filed; this is the
    /// half of "with its kind and session" that schema v8 has nowhere to put.
    Saturated { session_id: Uuid, kind: EventKind },
}

/// A spooled event handed back to a drainer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpooledEvent {
    pub event_id: Uuid,
    pub session_id: Uuid,
    pub project_id: Uuid,
    pub account_id: Uuid,
    pub session_seq: u64,
    pub kind: EventKind,
    pub event: SafeCanonicalEvent,
    /// Including the attempt this claim just started.
    pub attempts: i64,
}

/// Spool one approved event, assigning its ordinal and identity.
///
/// One transaction does all of it — allocate the ordinal, derive the id,
/// enforce capacity, insert, count `spooled` — because an ordinal consumed
/// without a row would leave a gap, and a row inserted without the ordinal
/// consumed would let the next event re-derive its identity.
///
/// On saturation the transaction is rolled back rather than committed with the
/// event dropped. That matters: the overflow loop may have shed rows before
/// discovering nothing capture-class was left, and shedding rows for an event
/// that is then refused would lose two events instead of one.
pub async fn spool_event(
    store: &Store,
    capacity: SpoolCapacity,
    new: NewEvent,
) -> Result<EventAdmission> {
    let NewEvent {
        project_id,
        account_id,
        mut event,
        server_instance_id,
    } = new;
    let session_id = event.session_id;
    let agent = event.agent.as_str();
    let kind = event.kind;

    let mut tx = tx::begin(store, "spool_event").await?;

    let session_seq = allocate_session_seq(&mut tx, session_id).await?;
    event.session_seq = session_seq;
    event.event_id = event_id(session_id, session_seq);

    let payload =
        serde_json::to_string(&event).map_err(|e| StoreError::Corrupt(format!("event: {e}")))?;
    let payload_bytes = payload.len() as i64;

    let mut overflow_dropped = 0u64;
    let admitted = loop {
        let (rows_now, bytes_now) = undelivered_capacity(&mut tx).await?;
        // `rows_now < max_events` is "there is room for one more"; the byte
        // side has to add the payload because the bound is on total size.
        if rows_now < capacity.max_events && bytes_now + payload_bytes <= capacity.max_bytes {
            break true;
        }
        match shed_oldest_capture_row(&mut tx).await? {
            Some(shed) => {
                count_disposition(
                    &mut tx,
                    shed.project_id,
                    &shed.agent,
                    &shed.kind,
                    Disposition::SpoolOverflowDropped,
                )
                .await?;
                overflow_dropped += 1;
            }
            None => break false,
        }
    };

    if !admitted {
        tx.rollback().await.map_err(StoreError::from)?;
        // Counted outside the rolled-back transaction, because the count is the
        // only trace a saturated store leaves of the event it refused: rolling
        // it back with the shedding would make the loss silent, which is the
        // one thing FR-785 does not allow.
        record_disposition(
            store,
            project_id,
            agent,
            kind.as_str(),
            Disposition::SpoolSaturatedDropped,
        )
        .await?;
        return Ok(EventAdmission::Saturated { session_id, kind });
    }

    let now = rows::now_text();
    sqlx::query(
        "INSERT INTO event_spool
            (event_id, session_id, project_id, account_id, session_seq, kind, payload,
             payload_bytes, boundary_class, state, attempts, claimed_at, next_attempt_at,
             last_error_kind, created_at, server_instance_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', 0, NULL, ?10, NULL, ?10, ?11)",
    )
    .bind(event.event_id.to_string())
    .bind(session_id.to_string())
    .bind(project_id.to_string())
    .bind(account_id.to_string())
    // The ordinal is a u64 on the wire and an INTEGER in SQLite. A session that
    // reached 2^63 events is not a case worth a fallible conversion here.
    .bind(session_seq as i64)
    .bind(kind.as_str())
    .bind(&payload)
    .bind(payload_bytes)
    .bind(event.boundary_class())
    // `next_attempt_at` is set to the insert time rather than left NULL, so the
    // claim predicate compares two timestamps and never has to decide what a
    // missing schedule means.
    .bind(&now)
    .bind(server_instance_id.map(|i| i.to_string()))
    .execute(&mut *tx)
    .await?;

    count_disposition(
        &mut tx,
        project_id,
        agent,
        kind.as_str(),
        Disposition::Spooled,
    )
    .await?;
    tx::commit(tx, "spool_event").await?;

    Ok(EventAdmission::Spooled {
        event,
        overflow_dropped,
    })
}

/// Consume this session's next event ordinal.
///
/// Two statements rather than an upsert with `RETURNING`, because the row may
/// not exist yet and SQLite's `ON CONFLICT DO UPDATE … RETURNING` would have to
/// distinguish the inserted case from the updated one. Both run inside the
/// caller's `BEGIN IMMEDIATE`, which already holds the write lock, so no
/// concurrent writer can interleave between them and two events can never be
/// handed the same ordinal.
async fn allocate_session_seq(conn: &mut sqlx::SqliteConnection, session_id: Uuid) -> Result<u64> {
    sqlx::query(
        "INSERT INTO session_event_seq (session_id, next_seq) VALUES (?1, 1)
         ON CONFLICT (session_id) DO NOTHING",
    )
    .bind(session_id.to_string())
    .execute(&mut *conn)
    .await?;

    let allocated: i64 = sqlx::query_scalar(
        "UPDATE session_event_seq SET next_seq = next_seq + 1
          WHERE session_id = ?1
          RETURNING next_seq - 1",
    )
    .bind(session_id.to_string())
    .fetch_one(&mut *conn)
    .await?;
    Ok(allocated as u64)
}

/// Rows and payload bytes the spool is currently holding as undelivered work.
async fn undelivered_capacity(conn: &mut sqlx::SqliteConnection) -> Result<(i64, i64)> {
    let row = sqlx::query(&format!(
        "SELECT COUNT(*) AS n, COALESCE(SUM(payload_bytes), 0) AS bytes
           FROM event_spool WHERE state IN {UNDELIVERED}"
    ))
    .fetch_one(&mut *conn)
    .await?;
    Ok((row.try_get("n")?, row.try_get("bytes")?))
}

/// What shedding one row needs to know to count it.
struct ShedRow {
    project_id: Uuid,
    agent: String,
    kind: String,
}

/// Delete the oldest capture-class row, or report that there is none.
///
/// `boundary_class = 0` is the whole guard. There is no branch here that could
/// reach a boundary row, so "the overflow policy never sheds a session
/// boundary" is a property of the statement rather than of the caller's
/// discipline.
async fn shed_oldest_capture_row(conn: &mut sqlx::SqliteConnection) -> Result<Option<ShedRow>> {
    let row = sqlx::query(&format!(
        "DELETE FROM event_spool
          WHERE event_id = (
              SELECT event_id FROM event_spool
               WHERE boundary_class = 0 AND state IN {UNDELIVERED}
               ORDER BY created_at, event_id
               LIMIT 1)
          RETURNING project_id, kind, payload"
    ))
    .fetch_optional(&mut *conn)
    .await?;

    let Some(row) = row else { return Ok(None) };
    let payload: String = row.try_get("payload")?;
    let event: SafeCanonicalEvent = serde_json::from_str(&payload)
        .map_err(|e| StoreError::Corrupt(format!("event_spool.payload: {e}")))?;
    Ok(Some(ShedRow {
        project_id: rows::uuid(&row, "project_id")?,
        agent: event.agent.as_str().to_string(),
        kind: row.try_get("kind")?,
    }))
}
/// Bind rows that predate this store ever knowing a server instance.
///
/// **The safe first-binding rule** (FR-791). A row queued before any instance
/// was established carries `NULL`, and `NULL` is not a wildcard — the claim
/// below matches the instance exactly, so such a row is delivered to nobody
/// until it is bound. This is where it is bound: the first drain that runs
/// against an established instance adopts every unbound row of that account,
/// once, inside the claim's own transaction.
///
/// Three properties make it safe, and each is the reason a simpler rule is
/// wrong:
///
/// - **It only ever writes over `NULL`.** A row that already names an instance
///   is untouched, so a second deployment cannot inherit the first's backlog by
///   draining it — which is the whole defect this column exists to close.
/// - **It is scoped to the account** whose drain is running, so one identity's
///   first contact does not bind another identity's unsent work.
/// - **It happens in the claim transaction**, so a row cannot be adopted by one
///   drainer and claimed by another between the two statements.
///
/// The alternative — treating `NULL` as "matches anything" — is the behaviour
/// that existed before this column, restated. It would deliver pre-binding work
/// to whichever server happened to answer first, which for a restored-from-
/// backup deployment at a familiar address is precisely the wrong one.
async fn adopt_unbound_rows(
    tx: &mut sqlx::SqliteConnection,
    table: &str,
    account_id: Uuid,
    server_instance_id: Uuid,
) -> Result<u64> {
    let done = sqlx::query(&format!(
        "UPDATE {table} SET server_instance_id = ?1
          WHERE account_id = ?2 AND server_instance_id IS NULL
            AND state IN {UNDELIVERED}"
    ))
    .bind(server_instance_id.to_string())
    .bind(account_id.to_string())
    .execute(&mut *tx)
    .await?;
    Ok(done.rows_affected())
}

/// Re-key rows queued under a provisional instance id to the reported one.
///
/// **The only rebinding in this module, and it is not a rebinding of identity.**
/// A lane opened against a server below schema 3 is keyed by an id derived from
/// the endpoint, because such a server reports none; when that peer is upgraded
/// in place it begins reporting a real id and the lane re-keys
/// (`sync-namespaces.md` §11a). The rows queued while it could not speak for
/// itself belong to that same server, so they move with the lane.
///
/// This is emphatically **not** a licence to re-key on a URL match. The caller
/// supplies the provisional id it derived from the endpoint and the id the peer
/// has now reported, and only rows carrying the first are touched. A second
/// deployment at the same address reports its own id, and no row of the first
/// carries a provisional id matching it.
pub async fn rebind_provisional_instance(
    store: &Store,
    from_provisional: Uuid,
    to_reported: Uuid,
) -> Result<u64> {
    if from_provisional == to_reported {
        return Ok(0);
    }
    let mut tx = tx::begin(store, "rebind_provisional_instance").await?;
    let mut moved = 0;
    for table in ["event_spool", "command_spool"] {
        let done = sqlx::query(&format!(
            "UPDATE {table} SET server_instance_id = ?2
              WHERE server_instance_id = ?1 AND state IN {UNDELIVERED}"
        ))
        .bind(from_provisional.to_string())
        .bind(to_reported.to_string())
        .execute(&mut *tx)
        .await?;
        moved += done.rows_affected();
    }
    tx::commit(tx, "rebind_provisional_instance").await?;
    Ok(moved)
}

/// Claim up to `limit` deliverable events for this account, oldest first.
///
/// **Two statements in one transaction**, not the single `UPDATE … RETURNING`
/// [`crate::outbox::claim`] uses. A drain first retires every eligible row that
/// has spent its attempt budget, then claims from what is left. They share one
/// `BEGIN IMMEDIATE` because a second drainer arriving between them could claim
/// a row this one had just judged exhausted, and the bound would hold only
/// until two drainers ran at once.
///
/// The bound has to be enforced here and not only where a failure is reported:
/// a drainer that crashes mid-send never calls [`mark_event_failed`], and every
/// claim increments `attempts`, so a crash loop would otherwise retry a row
/// without limit (FR-784).
///
/// Eligible rows are this account's — **exactly** this account's — that are
/// waiting and whose backoff has elapsed, plus rows whose claim has gone stale
/// so an interrupted send returns to the queue rather than stranding forever.
pub async fn claim_events(
    store: &Store,
    account_id: Uuid,
    server_instance_id: Uuid,
    limit: i64,
) -> Result<Vec<SpooledEvent>> {
    let now = chrono::Utc::now();
    let now_text = rows::ts_text(now);
    let stale_before = rows::ts_text(now - chrono::Duration::seconds(CLAIM_LEASE_SECONDS));

    // Terminalizing and claiming share one `BEGIN IMMEDIATE`, so no second
    // drainer can claim a row between this drainer deciding it is exhausted and
    // writing that down. Two statements outside a transaction would leave
    // exactly that window, and the window is the whole defect.
    let mut tx = tx::begin(store, "claim_events").await?;

    // First binding, before anything is terminalized or claimed. Rows queued
    // before this store knew a server instance become this one's; rows already
    // naming an instance are untouched.
    adopt_unbound_rows(&mut tx, "event_spool", account_id, server_instance_id).await?;

    // The bound has to be enforced *here*, not only where a failure is
    // recorded. `mark_event_failed` is never reached by a drainer that dies
    // after claiming, and every claim increments `attempts` — so a process that
    // crashes mid-send, repeatedly, would drive a row's attempts up without
    // limit and never terminalize it. FR-784's bound would hold only for
    // drainers polite enough to report their own failures.
    //
    // Only *eligible* rows are terminalized. A live `in_flight` row at the
    // bound is left alone: its drainer may still be about to succeed, and
    // refusing it out from under a delivery in progress would turn a success
    // into a permanent refusal.
    sqlx::query(
        "UPDATE event_spool
            SET state = 'refused', claimed_at = NULL, next_attempt_at = NULL,
                last_error_kind = ?4
          WHERE account_id = ?2
            -- Scoped to this instance for the same reason the claim is: another
            -- deployment's backlog is not this drainer's to give up on, and
            -- exhausting somebody else's attempts is a durable verdict about
            -- work it was never asked to deliver.
            AND server_instance_id = ?6
            AND attempts >= ?5
            AND ( (state IN ('pending','failed')
                   AND (next_attempt_at IS NULL OR next_attempt_at <= ?1))
               OR (state = 'in_flight'
                   AND (claimed_at IS NULL OR claimed_at < ?3)) )",
    )
    .bind(&now_text)
    .bind(account_id.to_string())
    .bind(&stale_before)
    .bind(TERMINAL_RETRY_EXHAUSTED)
    .bind(MAX_DELIVERY_ATTEMPTS)
    .bind(server_instance_id.to_string())
    .execute(&mut *tx)
    .await?;

    let rs = sqlx::query(
        "UPDATE event_spool
            SET state = 'in_flight', claimed_at = ?1, attempts = attempts + 1,
                -- The previous attempt's reason is history the moment a new
                -- send begins. `last_error_kind` describes why the row is in
                -- the state it is *now*, and now it is in flight.
                --
                -- Leaving it attached broke the status partition at exactly
                -- this transition: a deferred row rests as `pending` carrying
                -- `awaiting_capability`, and re-claiming it produced a row that
                -- was `in_flight` and `deferred` at once — and `waiting` and
                -- `deferred` at once if the claim later went stale. The repair
                -- belongs here rather than in the status query, because a query
                -- taught to ignore a live claim's marker would be working
                -- around a row that says something untrue about itself.
                --
                -- Nothing is lost. Whatever happens next establishes its own
                -- reason: another deferral sets `awaiting_capability` again, a
                -- transport failure sets its own kind, a refusal sets its
                -- refusal kind, and success needs none. Identity is untouched.
                last_error_kind = NULL
          WHERE event_id IN (
              SELECT event_id FROM event_spool
                  -- Exactly equal, and nothing else. Not `IS NULL OR`, not a
                  -- bound parameter that could arrive NULL: a row authored by
                  -- another account is not this drainer's to deliver
                  -- (FR-790, FR-864a).
               WHERE account_id = ?2
                 -- **And the exact server instance** (FR-791). Same rule and
                 -- same reasoning as the account: an endpoint is not an
                 -- identity, so a deployment replaced or restored from backup
                 -- at a familiar address must not inherit the backlog queued
                 -- for its predecessor. `IS NULL` is deliberately not accepted
                 -- here — an unbound row is adopted above, explicitly, or it
                 -- waits.
                 AND server_instance_id = ?6
                 -- Under the bound, checked on the claim itself. The statement
                 -- above has already refused every eligible row at or over it,
                 -- so this is belt and braces — and it is the brace that holds
                 -- if the two ever disagree.
                 AND attempts < ?5
                 AND ( (state IN ('pending','failed')
                        AND (next_attempt_at IS NULL OR next_attempt_at <= ?1))
                       -- A NULL claim stamp on an `in_flight` row is a claim
                       -- already lost, not an open authorization; this NULL is
                       -- about scheduling, which is why it is allowed here and
                       -- never on `account_id`.
                    OR (state = 'in_flight'
                        AND (claimed_at IS NULL OR claimed_at < ?3)) )
               ORDER BY created_at, session_id, session_seq
               LIMIT ?4)
          RETURNING *",
    )
    .bind(&now_text)
    .bind(account_id.to_string())
    .bind(&stale_before)
    .bind(limit)
    .bind(MAX_DELIVERY_ATTEMPTS)
    .bind(server_instance_id.to_string())
    .fetch_all(&mut *tx)
    .await?;

    // Rows are read out before the commit, because `rs` borrows nothing from
    // the transaction and the claim is only real once it is committed.
    let mut claimed = rs
        .iter()
        .map(|r| {
            let created_at: String = r.try_get("created_at")?;
            Ok((created_at, event_row(r)?))
        })
        .collect::<Result<Vec<_>>>()?;
    tx::commit(tx, "claim_events").await?;

    // `RETURNING` promises no order; the queue is oldest-first.
    claimed.sort_by(|a, b| {
        (&a.0, a.1.session_id, a.1.session_seq).cmp(&(&b.0, b.1.session_id, b.1.session_seq))
    });
    Ok(claimed.into_iter().map(|(_, e)| e).collect())
}

fn event_row(r: &sqlx::sqlite::SqliteRow) -> Result<SpooledEvent> {
    let payload: String = r.try_get("payload")?;
    let event: SafeCanonicalEvent = serde_json::from_str(&payload)
        .map_err(|e| StoreError::Corrupt(format!("event_spool.payload: {e}")))?;
    let session_seq: i64 = r.try_get("session_seq")?;
    Ok(SpooledEvent {
        event_id: rows::uuid(r, "event_id")?,
        session_id: rows::uuid(r, "session_id")?,
        project_id: rows::uuid(r, "project_id")?,
        account_id: rows::uuid(r, "account_id")?,
        session_seq: session_seq as u64,
        kind: rows::enum_val(r, "kind")?,
        event,
        attempts: r.try_get("attempts")?,
    })
}

/// The server accepted it.
pub async fn mark_event_delivered(store: &Store, event_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE event_spool
            SET state = 'delivered', claimed_at = NULL, next_attempt_at = NULL
          WHERE event_id = ?1",
    )
    .bind(event_id.to_string())
    .execute(store.pool())
    .await?;
    Ok(())
}

/// A transient failure: schedule the next attempt and let the queue have it
/// back.
///
/// The attempt was already counted when the row was claimed, so the backoff is
/// computed from the count as it now stands — the first failure waits a second,
/// and no failure ever waits more than five minutes.
pub async fn mark_event_failed(store: &Store, event_id: Uuid, error_kind: &str) -> Result<()> {
    let mut tx = tx::begin(store, "mark_event_failed").await?;
    let attempts: i64 = sqlx::query_scalar("SELECT attempts FROM event_spool WHERE event_id = ?1")
        .bind(event_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("spooled event {event_id}")))?;

    if attempts >= MAX_DELIVERY_ATTEMPTS {
        // The attempt budget is spent; this is not a transient failure any more. The row stops
        // being retried and becomes visible, which is what FR-784 asks for; it
        // is not deleted, because a row nobody can see is exactly the silence
        // the requirement forbids.
        sqlx::query(
            "UPDATE event_spool
                SET state = 'refused', claimed_at = NULL, next_attempt_at = NULL,
                    last_error_kind = ?2
              WHERE event_id = ?1",
        )
        .bind(event_id.to_string())
        .bind(TERMINAL_RETRY_EXHAUSTED)
        .execute(&mut *tx)
        .await?;
        return tx::commit(tx, "mark_event_failed").await;
    }

    let next = chrono::Utc::now() + chrono::Duration::seconds(backoff_seconds(attempts));
    sqlx::query(
        "UPDATE event_spool
            SET state = 'failed', claimed_at = NULL, next_attempt_at = ?2, last_error_kind = ?3
          WHERE event_id = ?1",
    )
    .bind(event_id.to_string())
    .bind(rows::ts_text(next))
    .bind(error_kind)
    .execute(&mut *tx)
    .await?;
    tx::commit(tx, "mark_event_failed").await
}

/// How long a deferred row waits before it is probed again.
///
/// A flat interval, not the exponential backoff a transient failure gets, and
/// the difference follows from what is being waited for. A transient failure
/// may clear in a second, so the wait starts short and grows. A capability
/// appears when somebody upgrades a server, which will not happen sooner
/// because Cairn asked twice — so probing fast buys nothing, and probing at the
/// ceiling forever is the bounded traffic FR-788 wants.
pub const DEFERRAL_PROBE_SECONDS: i64 = BACKOFF_CEILING_SECONDS;

/// The `last_error_kind` a deferred row carries while it waits.
pub const DEFERRED_AWAITING_CAPABILITY: &str = "awaiting_capability";

/// The server cannot hold this **yet**. Retryable, and it costs nothing.
///
/// ## Why this is not `mark_event_failed`
///
/// It was, and that was the defect. `attempts` increments when a row is
/// claimed, so routing a deferral through the failure path spent an attempt on
/// every probe — and a store talking to an old server for long enough would
/// drive an upgradeable row to `retry_exhausted` and declare it permanently
/// undeliverable. The row would be terminal because the *server* was old, which
/// is precisely the outcome FR-775's "refuse in a way the client can recognise
/// and defer" exists to avoid.
///
/// So the claim's increment is **refunded**. The same reasoning
/// [`crate::outbox::mark_retryable`] gives for not counting a released claim:
/// `attempts` is a count of futile retries, and a deferral was not one — the
/// server was asked a question it cannot answer yet, and answered honestly.
///
/// The identity does not change. `event_id` was fixed when the row was spooled
/// and is derived from the session and its ordinal, so however long a row waits
/// for an upgrade, the event that eventually lands is the same event and lands
/// exactly once (FR-770).
pub async fn mark_event_deferred(store: &Store, event_id: Uuid, reason: &str) -> Result<()> {
    let next = chrono::Utc::now() + chrono::Duration::seconds(DEFERRAL_PROBE_SECONDS);
    let updated = sqlx::query(
        "UPDATE event_spool
            SET state = 'pending', claimed_at = NULL, next_attempt_at = ?2,
                last_error_kind = ?3,
                attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END
          WHERE event_id = ?1",
    )
    .bind(event_id.to_string())
    .bind(rows::ts_text(next))
    .bind(reason)
    .execute(store.pool())
    .await?;
    if updated.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!("spooled event {event_id}")));
    }
    Ok(())
}

/// The same, for a command. See [`mark_event_deferred`].
pub async fn mark_command_deferred(store: &Store, command_id: Uuid, reason: &str) -> Result<()> {
    let next = chrono::Utc::now() + chrono::Duration::seconds(DEFERRAL_PROBE_SECONDS);
    let updated = sqlx::query(
        "UPDATE command_spool
            SET state = 'pending', claimed_at = NULL, next_attempt_at = ?2,
                last_error_kind = ?3,
                attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END
          WHERE command_id = ?1",
    )
    .bind(command_id.to_string())
    .bind(rows::ts_text(next))
    .bind(reason)
    .execute(store.pool())
    .await?;
    if updated.rows_affected() == 0 {
        return Err(StoreError::NotFound(format!(
            "spooled command {command_id}"
        )));
    }
    Ok(())
}

/// The server rejected it. Permanent, and visible (FR-772, FR-784).
///
/// `next_attempt_at` is cleared rather than pushed out, because a refused row
/// is not waiting for anything: no claim predicate here admits `refused`, so
/// the row is retried zero times and stays where someone can see it.
///
/// The refusal is counted as `rejected_by_server` from the row itself, which is
/// the only path by which a row reaches this state — a locally detected problem
/// stops an event before it is ever spooled.
pub async fn mark_event_refused(store: &Store, event_id: Uuid, error_kind: &str) -> Result<()> {
    let mut tx = tx::begin(store, "mark_event_refused").await?;
    let row = sqlx::query("SELECT project_id, kind, payload FROM event_spool WHERE event_id = ?1")
        .bind(event_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("spooled event {event_id}")))?;
    let payload: String = row.try_get("payload")?;
    let event: SafeCanonicalEvent = serde_json::from_str(&payload)
        .map_err(|e| StoreError::Corrupt(format!("event_spool.payload: {e}")))?;
    let project_id = rows::uuid(&row, "project_id")?;
    let kind: String = row.try_get("kind")?;

    sqlx::query(
        "UPDATE event_spool
            SET state = 'refused', claimed_at = NULL, next_attempt_at = NULL,
                last_error_kind = ?2
          WHERE event_id = ?1",
    )
    .bind(event_id.to_string())
    .bind(error_kind)
    .execute(&mut *tx)
    .await?;
    count_disposition(
        &mut tx,
        project_id,
        event.agent.as_str(),
        &kind,
        Disposition::RejectedByServer,
    )
    .await?;
    tx::commit(tx, "mark_event_refused").await
}

/// Return every claimed event to the queue.
///
/// Called once at daemon start, when nothing is draining yet: an `in_flight`
/// row there belongs to a run that is already gone and would otherwise wait out
/// [`CLAIM_LEASE_SECONDS`] for no reason. The recorded backoff is left alone —
/// a restart is not a reason to retry a row that failed a moment ago.
pub async fn release_event_claims(store: &Store) -> Result<u64> {
    let result = sqlx::query(
        "UPDATE event_spool SET state = 'pending', claimed_at = NULL WHERE state = 'in_flight'",
    )
    .execute(store.pool())
    .await?;
    Ok(result.rows_affected())
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// The one status primitive for either spool.
///
/// **One, deliberately.** There used to be two — a `SpoolStatus` answering "how
/// much has not reached the server" and this answering "why is it not moving" —
/// and they could disagree: `spool_command` enforces a capacity bound that
/// `command_spool_status` did not know about, so it reported `saturated: false`
/// about a spool that was refusing work. Two public functions capable of giving
/// different answers to one question is not a documentation problem, so the
/// second was removed rather than taught to agree.
///
/// The depth question is still answerable — [`undelivered`](Self::undelivered)
/// — it is just derived from these numbers rather than counted separately, so
/// it cannot drift from them.
///
/// The conditions are four fields because they call for four different
/// actions: a spool of 500 waiting rows on a laptop that just came online is
/// healthy, and 500 exhausted ones are not.
///
/// **They are a partition.** `waiting`, `in_flight`, `retrying`, `deferred` and
/// `terminal` are mutually exclusive and, together, cover every row still in
/// the spool; `undelivered()` is the four non-terminal ones. A breakdown whose
/// buckets overlapped would invite the reader to add them up and get a number
/// larger than the table — and would put one row under two headings that call
/// for different actions, which is the whole reason this is not reported as a
/// single depth.
///
/// `terminal_retry_exhausted` is the one exception, and deliberately: it is a
/// *subset* of `terminal` rather than a sixth peer, so it is excluded from the
/// sum and bounded by `terminal` instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpoolBreakdown {
    /// Spooled and due, never attempted or already past its backoff. Waiting on
    /// a drainer, not on a problem.
    pub waiting: i64,
    /// Claimed by a drainer whose lease has not expired. In flight right now.
    pub in_flight: i64,
    /// Attempted, failed transiently, parked until `next_attempt_at`.
    /// Something is wrong and Cairn is still trying.
    pub retrying: i64,
    /// Waiting for a capability the server does not have yet. Retryable and
    /// spending no attempt budget, so this number can sit still for a long time
    /// with nothing wrong — and it is separate from `retrying` because "the
    /// server is old" and "the server is failing" call for different actions.
    pub deferred: i64,
    /// Terminal: the server refused it permanently, or it ran out of its
    /// attempt budget. Cairn has stopped trying and is saying so.
    pub terminal: i64,
    /// Of the terminal rows, those that ran out of their attempt budget rather
    /// than being refused. The consequence is the same and the cause is not:
    /// one means the server said no, the other that it was never reachable.
    pub terminal_retry_exhausted: i64,
    /// Bytes of payload the undelivered rows hold.
    ///
    /// Always zero for commands: the approved data model applies the byte bound
    /// to `event_spool` only, and a command payload is intent rather than a
    /// serialized event. Reporting a number nothing bounds would invite someone
    /// to bound it.
    pub bytes: i64,
    /// New work is being refused: for events, the bound is reached and nothing
    /// capture-class is left to shed; for commands, the bound is reached, full
    /// stop.
    pub saturated: bool,
    /// Undelivered rows bound to a server instance other than the one this
    /// store is now talking to (FR-791).
    ///
    /// Counted and reported rather than silently skipped, because the rows are
    /// intact and invisible otherwise: the claim will not touch them, no
    /// attempt is spent, nothing is refused, and a depth that included them
    /// with no explanation would look like a queue that had simply stopped.
    /// This is the number that says "this work belongs to a different
    /// deployment", which is a thing an operator can act on.
    pub other_instance: i64,
    /// When the oldest undelivered row was created, or `None` when there is
    /// none (FR-792).
    ///
    /// A depth on its own does not say whether anything is wrong: fifty rows
    /// spooled in the last second is a busy minute, and one row spooled last
    /// Tuesday is an outage nobody noticed. The age is what separates them, and
    /// it is the one number a person actually acts on.
    ///
    /// Terminal rows are excluded, deliberately. They are not waiting for
    /// anything, so counting them here would make an unattended permanent
    /// refusal look like an ever-worsening delay and hide a genuine one behind
    /// it.
    pub oldest_at: Option<DateTime<Utc>>,
}

impl SpoolBreakdown {
    /// Rows that have not reached the server, claimed or not.
    ///
    /// The four non-terminal conditions, `deferred` included: a row waiting for
    /// a capability has not reached the server either, and leaving it out would
    /// make a store full of deferred work report an empty queue.
    ///
    /// Derived rather than counted, so it can never disagree with the breakdown
    /// it comes from — and because the conditions are a partition, this is
    /// exactly "everything that is not terminal".
    pub fn undelivered(&self) -> i64 {
        self.waiting + self.in_flight + self.retrying + self.deferred
    }

    /// Rows in a terminal state, which stay visible rather than being deleted.
    pub fn refused(&self) -> i64 {
        self.terminal
    }

    /// Why delivery is not progressing, or `None` when it is (FR-792).
    ///
    /// **The third thing FR-792 asks for**, after the depth and the oldest
    /// entry, and the only one that is a judgement rather than a number. A user
    /// looking at a spool that is not draining needs the reason, and the states
    /// that produce it call for different actions: a saturated store is losing
    /// new work *now*; a retry-exhausted row will never move again without
    /// intervention; a deferred row is fine and merely waiting for a server
    /// upgrade. Collapsing those into "stuck" would tell someone to act on the
    /// harmless one and ignore the harmful one.
    ///
    /// Ordered by severity and the first match wins, because a spool can be
    /// several of these at once and a status line reports one thing. Saturation
    /// leads: it is the only state in which work is being *lost* rather than
    /// delayed.
    ///
    /// An empty spool is never blocked, and neither is one whose rows are all
    /// simply waiting for the next tick — reporting a reason there would make
    /// "blocked" mean "has work", and a signal that is always on is not a
    /// signal. `no_account` is not decidable from the breakdown alone and is
    /// left to the caller, which is the only side that knows whether anybody is
    /// signed in.
    pub fn blocked_reason(&self) -> Option<&'static str> {
        // Ahead of saturation: work bound to another deployment is not merely
        // delayed, it will never move under this one, and no amount of draining
        // will change that. An operator told "the queue is full" would go
        // looking for capacity; the actual news is that this store is pointed at
        // a different server than the one its backlog belongs to (FR-791).
        if self.other_instance > 0 && self.undelivered() == self.other_instance {
            return Some("server_instance_mismatch");
        }
        if self.saturated {
            return Some("saturated");
        }
        if self.terminal_retry_exhausted > 0 {
            return Some("retry_exhausted");
        }
        if self.terminal > self.terminal_retry_exhausted {
            return Some("refused_by_server");
        }
        if self.deferred > 0 {
            return Some("awaiting_capability");
        }
        if self.retrying > 0 {
            return Some("backing_off");
        }
        // Some of the backlog belongs elsewhere and some of it is simply
        // waiting. Reported last, because the deliverable part is moving and a
        // reason implying otherwise would be wrong — but reported, because the
        // rest never will be.
        if self.other_instance > 0 {
            return Some("server_instance_mismatch");
        }
        None
    }
}

/// The event spool's health, broken down by why each row is not moving.
/// One session's spooled events, oldest first.
///
/// Every state, `delivered` included: a token a semantic signal cites stays
/// justified after the event that established it reached the server, because
/// the server still holds it. What does remove a token is the capacity policy
/// shedding the row — and a signal that then cites it is refused, which is
/// exactly the outcome `contracts/extraction.md` §13.3 rule 3 states rather
/// than an accident of the spool draining.
///
/// Ordered by the durable ordinal and never by insertion time, because the
/// ordinal is what "an earlier event" means here (FR-780).
pub async fn session_events(store: &Store, session_id: Uuid) -> Result<Vec<SafeCanonicalEvent>> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT session_seq, payload FROM event_spool
          WHERE session_id = ?1
          ORDER BY session_seq",
    )
    .bind(session_id.to_string())
    .fetch_all(store.pool())
    .await?;

    Ok(rows
        .into_iter()
        // A row whose payload no longer parses is skipped rather than failing
        // the read. It cannot justify a token — nothing can read what it says —
        // and refusing to answer at all would take a whole session's vocabulary
        // down with one unreadable row.
        .filter_map(|(_, payload)| serde_json::from_str::<SafeCanonicalEvent>(&payload).ok())
        .collect())
}

pub async fn event_spool_breakdown(
    store: &Store,
    capacity: SpoolCapacity,
    current_instance: Option<Uuid>,
) -> Result<SpoolBreakdown> {
    breakdown(store, SpoolTable::Events, capacity, current_instance).await
}

/// The command spool's health.
///
/// Saturated here means the queue is full and further commands are being
/// refused — never that anything was dropped, because no command is droppable
/// (see [`spool_command`]).
pub async fn command_spool_breakdown(
    store: &Store,
    capacity: SpoolCapacity,
    current_instance: Option<Uuid>,
) -> Result<SpoolBreakdown> {
    breakdown(store, SpoolTable::Commands, capacity, current_instance).await
}

/// Which spool a status query is about.
///
/// An enum rather than a string, so the table name interpolated into the
/// statements below cannot originate anywhere but here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpoolTable {
    Events,
    Commands,
}

impl SpoolTable {
    fn name(self) -> &'static str {
        match self {
            SpoolTable::Events => "event_spool",
            SpoolTable::Commands => "command_spool",
        }
    }
}

async fn breakdown(
    store: &Store,
    which: SpoolTable,
    capacity: SpoolCapacity,
    // The instance this store is talking to now, when it has one. Only used to
    // count what belongs to a *different* one — the breakdown does not filter
    // by it, because a row stranded by a mismatch is still occupying the spool
    // and still counts toward the bound.
    current_instance: Option<Uuid>,
) -> Result<SpoolBreakdown> {
    let table = which.name();
    // One connection and one clock reading for all of it, so the breakdown is a
    // consistent picture rather than several snapshots of a moving table taken
    // a millisecond apart.
    let mut conn = store.pool().acquire().await?;
    let now = rows::ts_text(chrono::Utc::now());
    let stale_before =
        rows::ts_text(chrono::Utc::now() - chrono::Duration::seconds(CLAIM_LEASE_SECONDS));

    async fn count(
        conn: &mut sqlx::SqliteConnection,
        sql: &str,
        now: &str,
        stale_before: &str,
    ) -> Result<i64> {
        sqlx::query_scalar::<_, i64>(sql)
            .bind(now)
            .bind(stale_before)
            .fetch_one(&mut *conn)
            .await
            .map_err(StoreError::from)
    }

    // **The five conditions are a partition**, and keeping them one takes some
    // care because the underlying states overlap freely: a deferred row is
    // `pending` with a future `next_attempt_at`, which is also exactly what a
    // row waiting out a transient backoff looks like. Counting each condition
    // independently put every deferred row under `retrying` as well — so the
    // five sums exceeded the table, and an old server read as a failing one.
    //
    // `deferred` is therefore tested first and excluded from the other two,
    // which makes the order below load-bearing rather than incidental. It is
    // written as an explicit exclusion rather than left to the reader.
    const NOT_DEFERRED: &str = "(last_error_kind IS NULL                                 OR last_error_kind <> 'awaiting_capability')";

    // Waiting on a drainer, not on a problem. A stale `in_flight` row counts
    // here rather than as in flight: its drainer is gone and the next claim
    // will take it, and calling it in flight would report a dead process as
    // healthy work in progress.
    let waiting = count(
        &mut conn,
        &format!(
            "SELECT COUNT(*) FROM {table}
              WHERE (state = 'pending' AND {NOT_DEFERRED}
                     AND (next_attempt_at IS NULL OR next_attempt_at <= ?1))
                 OR (state = 'in_flight' AND (claimed_at IS NULL OR claimed_at < ?2))"
        ),
        &now,
        &stale_before,
    )
    .await?;
    let in_flight = count(
        &mut conn,
        &format!(
            "SELECT COUNT(*) FROM {table}
              WHERE state = 'in_flight' AND claimed_at IS NOT NULL AND claimed_at >= ?2"
        ),
        &now,
        &stale_before,
    )
    .await?;
    // Something is wrong and Cairn is still trying. A deferral is neither, so
    // it is excluded from both halves.
    let retrying = count(
        &mut conn,
        &format!(
            "SELECT COUNT(*) FROM {table}
              WHERE (state = 'failed' AND {NOT_DEFERRED})
                 OR (state = 'pending' AND {NOT_DEFERRED} AND next_attempt_at > ?1)"
        ),
        &now,
        &stale_before,
    )
    .await?;
    // Waiting on a capability, and **resting** while it waits. A deferral that
    // is due and one still inside its probe interval are both deferrals, so the
    // condition is the marker rather than the schedule — but a row with a send
    // in progress is not resting, and `in_flight` is excluded here rather than
    // left out by luck.
    //
    // A claim clears `last_error_kind`, so an `in_flight` row cannot carry the
    // marker anyway. Excluding the state as well makes the exclusivity
    // structural: if the claim reset were ever lost, this query would still
    // refuse to report a live claim as resting, and the transition tests would
    // fail on the reset rather than on the classification.
    let deferred = count(
        &mut conn,
        &format!(
            "SELECT COUNT(*) FROM {table}
              WHERE state IN ('pending','failed')
                AND last_error_kind = '{DEFERRED_AWAITING_CAPABILITY}'"
        ),
        &now,
        &stale_before,
    )
    .await?;
    let terminal = count(
        &mut conn,
        &format!("SELECT COUNT(*) FROM {table} WHERE state = 'refused'"),
        &now,
        &stale_before,
    )
    .await?;
    let terminal_retry_exhausted = count(
        &mut conn,
        &format!(
            "SELECT COUNT(*) FROM {table}
              WHERE state = 'refused' AND last_error_kind = '{TERMINAL_RETRY_EXHAUSTED}'"
        ),
        &now,
        &stale_before,
    )
    .await?;

    // Every non-terminal condition, `deferred` included: a row waiting for a
    // capability still occupies the spool, so it counts toward the bound. This
    // is the same sum `SpoolBreakdown::undelivered` reports, and it has to be —
    // a capacity check that disagreed with the depth it is checked against
    // would refuse work while reporting room, which is the class of
    // contradiction the single status primitive exists to rule out.
    let queued_now = waiting + in_flight + retrying + deferred;
    let bytes = match which {
        SpoolTable::Events => {
            sqlx::query_scalar::<_, i64>(&format!(
                "SELECT COALESCE(SUM(payload_bytes), 0) FROM {table} WHERE state IN {UNDELIVERED}"
            ))
            .fetch_one(&mut *conn)
            .await?
        }
        SpoolTable::Commands => 0,
    };

    // The event spool is saturated only once nothing capture-class is left to
    // shed; the command spool is saturated the moment it is full, because
    // shedding was never an option for it — it carries no `boundary_class`,
    // since no explicit command is droppable.
    let at_bound = queued_now >= capacity.max_events || bytes >= capacity.max_bytes;
    let saturated = match which {
        SpoolTable::Events => {
            let capture_left: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {table}
                  WHERE boundary_class = 0 AND state IN {UNDELIVERED}"
            ))
            .fetch_one(&mut *conn)
            .await?;
            at_bound && capture_left == 0
        }
        SpoolTable::Commands => at_bound,
    };

    // Bound to a different deployment. `IS NOT NULL` matters: an unbound row is
    // waiting for its first binding, not stranded by a mismatch, and reporting
    // the two together would tell an operator to investigate a store that has
    // simply never synchronized.
    let other_instance: i64 = match current_instance {
        Some(current) => {
            sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {table}
                  WHERE state IN {UNDELIVERED}
                    AND server_instance_id IS NOT NULL
                    AND server_instance_id <> ?1"
            ))
            .bind(current.to_string())
            .fetch_one(&mut *conn)
            .await?
        }
        // Nothing to be mismatched against. A store that has not established an
        // instance cannot say any row belongs to another one.
        None => 0,
    };

    // Oldest by creation, not by `next_attempt_at`: the question FR-792 asks is
    // how long something has been waiting, and a row's backoff moving forward
    // does not make it younger.
    let oldest_at: Option<String> = sqlx::query_scalar(&format!(
        "SELECT MIN(created_at) FROM {table} WHERE state IN {UNDELIVERED}"
    ))
    .fetch_one(&mut *conn)
    .await?;
    let oldest_at = oldest_at.as_deref().and_then(rows::parse_ts);

    Ok(SpoolBreakdown {
        waiting,
        in_flight,
        retrying,
        deferred,
        terminal,
        terminal_retry_exhausted,
        bytes,
        saturated,
        other_instance,
        oldest_at,
    })
}

// ---------------------------------------------------------------------------
// The command spool
// ---------------------------------------------------------------------------

/// A knowledge command on its way into the spool.
///
/// `payload` carries **intent only**. Derived state — a state machine's next
/// state, a supersession decision, a verification authority — is the server's
/// to compute, and a client that could send it could assert it
/// (`contracts/knowledge-commands.md` §3.1, Principle IX). Nothing here
/// inspects the payload, which is exactly why the intent-only rule has to be
/// held by whoever constructs it.
#[derive(Debug, Clone)]
pub struct NewCommand<'a> {
    pub scope: CommandScope,
    /// Absent for a command that belongs to no project — a personal
    /// `cairn remember` issued outside a repository.
    pub project_id: Option<Uuid>,
    pub account_id: Uuid,
    /// The server instance this command is queued **for** (FR-791). `None`
    /// carries the same meaning it does on [`NewEvent`], and the same rule.
    pub server_instance_id: Option<Uuid>,
    pub kind: CommandKind,
    pub payload: &'a serde_json::Value,
}

/// A spooled command handed back to a drainer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpooledCommand {
    pub command_id: Uuid,
    pub scope: CommandScope,
    pub session_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub account_id: Uuid,
    pub command_seq: u64,
    pub kind: CommandKind,
    pub payload: serde_json::Value,
    pub attempts: i64,
}

/// What happened to a command offered to the spool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAdmission {
    /// Queued, with its identity fixed. The caller may tell the user the
    /// command was **accepted for delivery** — not that it is durable
    /// (FR-815a).
    Spooled(SpooledCommand),
    /// Refused because the command spool is full. Nothing was dropped, no
    /// ordinal was consumed, and the caller must **not** report the command as
    /// accepted.
    Saturated { queued: i64 },
}

/// Spool one knowledge command, assigning its ordinal and identity.
///
/// The same durable-ordinal arrangement as [`spool_event`], for the same
/// reason: replay has to be idempotent, so the server answers `duplicate` and
/// applies nothing twice.
///
/// # Capacity: bounded, and nothing is ever shed
///
/// The event spool's overflow policy sheds the oldest capture-class rows. That
/// policy is not merely inappropriate for commands, it is **inexpressible**:
/// `command_spool` has no `boundary_class` column, because there is no such
/// thing as a droppable command. An event is one observation out of a stream
/// Cairn generates; a command is something a person or an agent explicitly
/// asked Cairn to do, and the contract says so — *"a command the server later
/// refuses surfaces to the user; it is not retried forever and it is not
/// silently dropped"* (`contracts/knowledge-commands.md` §4).
///
/// So the shedding half of FR-785 has nothing to operate on here, and the
/// **saturation** half — refuse new work visibly rather than corrupt what is
/// queued — is the whole policy. That is not a new mechanism: it is exactly
/// what the event spool already does once nothing capture-class remains, with
/// the shedding step vacuous because no command was ever droppable.
///
/// The bound is the one `data-model.md` §3 already states, applied to both
/// spools rather than a second number invented for this one. Commands are
/// intent-only payloads issued at human or agent pace, so in ordinary use it
/// will not bind before a prolonged outage exhausts the attempt budget anyway;
/// what it rules out is a scripted loop against an unreachable server growing
/// the store without limit.
///
/// **This is an implementation decision, not a quoted requirement.** §3's
/// bounds table says "Spool capacity" without naming a table, and §4 enumerates
/// what the command spool inherits from the event spool without mentioning
/// capacity at all. Refusing visibly is the reading that keeps FR-785's "the
/// spool MUST be bounded" true without breaking §4's "not silently dropped";
/// leaving it unbounded would satisfy the second and abandon the first.
pub async fn spool_command(
    store: &Store,
    new: NewCommand<'_>,
    capacity: SpoolCapacity,
) -> Result<CommandAdmission> {
    let scope = new.scope;
    let scope_kind = scope.kind();
    let scope_key = scope.key().to_string();
    let payload = serde_json::to_string(new.payload)
        .map_err(|e| StoreError::Corrupt(format!("command payload: {e}")))?;

    let mut tx = tx::begin(store, "spool_command").await?;

    // Checked before the ordinal is allocated, which it can be — unlike an
    // event, whose byte size is not known until its identity has been assigned.
    // A refused command therefore consumes nothing at all: no ordinal, no gap
    // in the scope's sequence, nothing for a later reader to wonder about.
    let queued: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM command_spool WHERE state IN {UNDELIVERED}"
    ))
    .fetch_one(&mut *tx)
    .await?;
    if queued >= capacity.max_events {
        tx::commit(tx, "spool_command").await?;
        return Ok(CommandAdmission::Saturated { queued });
    }

    let command_seq = allocate_command_seq(&mut tx, scope_kind, &scope_key).await?;
    let id = command_id(scope_kind, &scope_key, command_seq);
    let now = rows::now_text();

    sqlx::query(
        "INSERT INTO command_spool
            (command_id, scope_kind, scope_key, session_id, project_id, account_id,
             command_seq, kind, payload, state, attempts, claimed_at, next_attempt_at,
             last_error_kind, created_at, server_instance_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', 0, NULL, ?10, NULL, ?10, ?11)",
    )
    .bind(id.to_string())
    .bind(scope_kind)
    .bind(&scope_key)
    .bind(scope.session_id().map(|s| s.to_string()))
    .bind(new.project_id.map(|p| p.to_string()))
    .bind(new.account_id.to_string())
    .bind(command_seq as i64)
    .bind(new.kind.as_str())
    .bind(&payload)
    .bind(&now)
    .bind(new.server_instance_id.map(|i| i.to_string()))
    .execute(&mut *tx)
    .await?;
    tx::commit(tx, "spool_command").await?;

    Ok(CommandAdmission::Spooled(SpooledCommand {
        command_id: id,
        scope,
        session_id: scope.session_id(),
        project_id: new.project_id,
        account_id: new.account_id,
        command_seq,
        kind: new.kind,
        payload: new.payload.clone(),
        attempts: 0,
    }))
}

/// Consume this scope's next command ordinal. See [`allocate_session_seq`].
async fn allocate_command_seq(
    conn: &mut sqlx::SqliteConnection,
    scope_kind: &str,
    scope_key: &str,
) -> Result<u64> {
    sqlx::query(
        "INSERT INTO command_seq (scope_kind, scope_key, next_seq) VALUES (?1, ?2, 1)
         ON CONFLICT (scope_kind, scope_key) DO NOTHING",
    )
    .bind(scope_kind)
    .bind(scope_key)
    .execute(&mut *conn)
    .await?;

    let allocated: i64 = sqlx::query_scalar(
        "UPDATE command_seq SET next_seq = next_seq + 1
          WHERE scope_kind = ?1 AND scope_key = ?2
          RETURNING next_seq - 1",
    )
    .bind(scope_kind)
    .bind(scope_key)
    .fetch_one(&mut *conn)
    .await?;
    Ok(allocated as u64)
}

/// Claim up to `limit` deliverable commands for this account, in scope order.
///
/// **Ordering is the point, and it is enforced in the predicate, not just the
/// sort.** A supersede queued after its target's creation has to be delivered
/// after it, so a command is claimable only if no lower-numbered command in its
/// own scope is currently un-claimable — held by another drainer, or waiting out
/// a backoff. Sorting alone would not do it: with the head of a scope in flight,
/// an ordered `LIMIT` would happily hand out the row behind it, and the two
/// would race to the server in the wrong order.
///
/// Terminal rows do not hold a scope back. A `delivered` command is done, and a
/// `refused` one has been surfaced to the user (`contracts/knowledge-commands.md`
/// §4) — letting a single refusal wedge every later command in its scope
/// forever would turn one visible failure into a silent dead queue.
///
/// Like [`claim_events`], this is **two statements in one transaction**:
/// retiring rows that have spent their attempt budget, then claiming from what
/// remains. Retiring here also releases a scope rather than wedging it, since
/// the barrier above counts only unsettled rows.
pub async fn claim_commands(
    store: &Store,
    account_id: Uuid,
    server_instance_id: Uuid,
    limit: i64,
) -> Result<Vec<SpooledCommand>> {
    let now = chrono::Utc::now();
    let now_text = rows::ts_text(now);
    let stale_before = rows::ts_text(now - chrono::Duration::seconds(CLAIM_LEASE_SECONDS));

    let mut tx = tx::begin(store, "claim_commands").await?;

    // First binding, on the same terms as the event spool's: only over `NULL`,
    // only this account's, and inside the claim transaction.
    adopt_unbound_rows(&mut tx, "command_spool", account_id, server_instance_id).await?;

    // The same claim-side bound as `claim_events`, for the same reason: a
    // drainer that crashes after claiming never reaches `mark_command_failed`,
    // and every claim increments `attempts`.
    //
    // Terminalizing here also *unblocks* the scope rather than wedging it. The
    // head-of-scope barrier below counts only rows that are still unsettled, so
    // a refused command stops holding back the commands queued behind it —
    // which is what stops one undeliverable instruction turning its whole scope
    // into a silent dead queue.
    sqlx::query(
        "UPDATE command_spool
            SET state = 'refused', claimed_at = NULL, next_attempt_at = NULL,
                last_error_kind = ?4
          WHERE account_id = ?2
            -- This deployment's rows only; see `claim_events` for why giving up
            -- on another's is a verdict nobody asked for.
            AND server_instance_id = ?6
            AND attempts >= ?5
            AND ( (state IN ('pending','failed')
                   AND (next_attempt_at IS NULL OR next_attempt_at <= ?1))
               OR (state = 'in_flight'
                   AND (claimed_at IS NULL OR claimed_at < ?3)) )",
    )
    .bind(&now_text)
    .bind(account_id.to_string())
    .bind(&stale_before)
    .bind(TERMINAL_RETRY_EXHAUSTED)
    .bind(MAX_DELIVERY_ATTEMPTS)
    .bind(server_instance_id.to_string())
    .execute(&mut *tx)
    .await?;

    let rs = sqlx::query(
        "UPDATE command_spool
            SET state = 'in_flight', claimed_at = ?1, attempts = attempts + 1,
                -- The same reset as `claim_events`, for the same reason: a new
                -- attempt supersedes the previous attempt's reason, and a row
                -- carrying a stale one is in two status conditions at once.
                last_error_kind = NULL
          WHERE command_id IN (
              SELECT c.command_id FROM command_spool c
                  -- Exactly equal, as in `claim_events` and for the same
                  -- reason (FR-790, FR-864a).
               WHERE c.account_id = ?2
                 -- **And the exact server instance** (FR-791), never `IS NULL`:
                 -- an unbound row is adopted above or it waits.
                 AND c.server_instance_id = ?6
                 AND c.attempts < ?5
                 AND ( (c.state IN ('pending','failed')
                        AND (c.next_attempt_at IS NULL OR c.next_attempt_at <= ?1))
                    OR (c.state = 'in_flight'
                        AND (c.claimed_at IS NULL OR c.claimed_at < ?3)) )
                 AND c.command_seq < COALESCE((
                       SELECT MIN(b.command_seq) FROM command_spool b
                        WHERE b.scope_kind = c.scope_kind
                          AND b.scope_key = c.scope_key
                          AND ( (b.state = 'in_flight'
                                 AND b.claimed_at IS NOT NULL AND b.claimed_at >= ?3)
                             OR (b.state IN ('pending','failed')
                                 AND b.next_attempt_at IS NOT NULL
                                 AND b.next_attempt_at > ?1) )
                     ), 9223372036854775807)
               ORDER BY c.scope_kind, c.scope_key, c.command_seq
               LIMIT ?4)
          RETURNING *",
    )
    .bind(&now_text)
    .bind(account_id.to_string())
    .bind(&stale_before)
    .bind(limit)
    .bind(MAX_DELIVERY_ATTEMPTS)
    .bind(server_instance_id.to_string())
    .fetch_all(&mut *tx)
    .await?;

    let mut claimed = rs.iter().map(command_row).collect::<Result<Vec<_>>>()?;
    tx::commit(tx, "claim_commands").await?;
    // `RETURNING` promises no order, and this one is a delivery guarantee
    // rather than a presentation choice.
    claimed.sort_by(|a, b| {
        (a.scope.kind(), a.scope.key(), a.command_seq).cmp(&(
            b.scope.kind(),
            b.scope.key(),
            b.command_seq,
        ))
    });
    Ok(claimed)
}

fn command_row(r: &sqlx::sqlite::SqliteRow) -> Result<SpooledCommand> {
    let scope_kind: String = r.try_get("scope_kind")?;
    let scope_key: String = r.try_get("scope_key")?;
    let key = Uuid::parse_str(&scope_key)
        .map_err(|e| StoreError::Corrupt(format!("command_spool.scope_key: {e}")))?;
    let scope = match scope_kind.as_str() {
        "session" => CommandScope::Session(key),
        "store" => CommandScope::Store(key),
        other => {
            return Err(StoreError::Corrupt(format!(
                "command_spool.scope_kind: {other}"
            )))
        }
    };
    let payload: String = r.try_get("payload")?;
    let command_seq: i64 = r.try_get("command_seq")?;
    Ok(SpooledCommand {
        command_id: rows::uuid(r, "command_id")?,
        scope,
        session_id: rows::opt_uuid(r, "session_id")?,
        project_id: rows::opt_uuid(r, "project_id")?,
        account_id: rows::uuid(r, "account_id")?,
        command_seq: command_seq as u64,
        kind: rows::enum_val(r, "kind")?,
        payload: serde_json::from_str(&payload)
            .map_err(|e| StoreError::Corrupt(format!("command_spool.payload: {e}")))?,
        attempts: r.try_get("attempts")?,
    })
}

/// The server accepted it.
pub async fn mark_command_delivered(store: &Store, command_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE command_spool
            SET state = 'delivered', claimed_at = NULL, next_attempt_at = NULL
          WHERE command_id = ?1",
    )
    .bind(command_id.to_string())
    .execute(store.pool())
    .await?;
    Ok(())
}

/// A transient failure. See [`mark_event_failed`].
pub async fn mark_command_failed(store: &Store, command_id: Uuid, error_kind: &str) -> Result<()> {
    let mut tx = tx::begin(store, "mark_command_failed").await?;
    let attempts: i64 =
        sqlx::query_scalar("SELECT attempts FROM command_spool WHERE command_id = ?1")
            .bind(command_id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| StoreError::NotFound(format!("spooled command {command_id}")))?;

    if attempts >= MAX_DELIVERY_ATTEMPTS {
        // Same bound as an event, and for the same reason. A command is the
        // more painful case — it is a user's explicit intent, not a captured
        // observation — which is why it becomes *visible* here rather than
        // being dropped: the whole point of the terminal state is that someone
        // can be told the instruction never landed.
        sqlx::query(
            "UPDATE command_spool
                SET state = 'refused', claimed_at = NULL, next_attempt_at = NULL,
                    last_error_kind = ?2
              WHERE command_id = ?1",
        )
        .bind(command_id.to_string())
        .bind(TERMINAL_RETRY_EXHAUSTED)
        .execute(&mut *tx)
        .await?;
        return tx::commit(tx, "mark_command_failed").await;
    }

    let next = chrono::Utc::now() + chrono::Duration::seconds(backoff_seconds(attempts));
    sqlx::query(
        "UPDATE command_spool
            SET state = 'failed', claimed_at = NULL, next_attempt_at = ?2, last_error_kind = ?3
          WHERE command_id = ?1",
    )
    .bind(command_id.to_string())
    .bind(rows::ts_text(next))
    .bind(error_kind)
    .execute(&mut *tx)
    .await?;
    tx::commit(tx, "mark_command_failed").await
}

/// The server refused it. Permanent, never retried, and surfaced to the user.
pub async fn mark_command_refused(store: &Store, command_id: Uuid, error_kind: &str) -> Result<()> {
    sqlx::query(
        "UPDATE command_spool
            SET state = 'refused', claimed_at = NULL, next_attempt_at = NULL,
                last_error_kind = ?2
          WHERE command_id = ?1",
    )
    .bind(command_id.to_string())
    .bind(error_kind)
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Return every claimed command to the queue. See [`release_event_claims`].
pub async fn release_command_claims(store: &Store) -> Result<u64> {
    let result = sqlx::query(
        "UPDATE command_spool SET state = 'pending', claimed_at = NULL WHERE state = 'in_flight'",
    )
    .execute(store.pool())
    .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_starts_at_one_second_and_never_exceeds_five_minutes() {
        assert_eq!(backoff_seconds(1), 1);
        assert_eq!(backoff_seconds(2), 2);
        assert_eq!(backoff_seconds(3), 4);
        assert_eq!(backoff_seconds(9), 256);
        assert_eq!(backoff_seconds(10), BACKOFF_CEILING_SECONDS);
        for attempts in [0i64, 1, 40, 1_000, i64::MAX] {
            let s = backoff_seconds(attempts);
            assert!(
                (BACKOFF_FLOOR_SECONDS..=BACKOFF_CEILING_SECONDS).contains(&s),
                "attempts={attempts} produced {s}s, outside the stated bounds"
            );
        }
    }

    #[test]
    fn every_command_kind_round_trips_through_its_wire_string() {
        for kind in CommandKind::ALL {
            assert_eq!(CommandKind::from_str(kind.as_str()).unwrap(), *kind);
        }
        assert!(CommandKind::from_str("promote_everything").is_err());
    }

    #[test]
    fn a_store_scope_carries_no_session_and_a_session_scope_carries_its_own() {
        let id = Uuid::now_v7();
        assert_eq!(CommandScope::Store(id).session_id(), None);
        assert_eq!(CommandScope::Session(id).session_id(), Some(id));
        assert_eq!(CommandScope::Store(id).kind(), "store");
        assert_eq!(CommandScope::Session(id).kind(), "session");
    }
}
