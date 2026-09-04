//! The capacity policy, driven against a real store with a bound small enough
//! to reach (T095, FR-785, SC-752, SC-753).
//!
//! # Why this is a store-level test and not an end-to-end one
//!
//! The bound is fifty thousand events or 256 MiB, whichever binds first. A
//! test that drove it through hooks would have to produce fifty thousand real
//! captures to observe a single shed, and would then be measuring the hook
//! path rather than the policy. [`SpoolCapacity`] exists precisely so the
//! policy can be exercised against a spool of five rows — its fields are
//! public and every production caller passes [`SpoolCapacity::default`], so a
//! small bound here changes the numbers and nothing else.
//!
//! # What is already proved in `crates/cairn-store/tests/feature005_spool.rs`
//!
//! That file already holds the single-shed case
//! (`overflow_sheds_the_oldest_capture_row_and_counts_the_drop`), the byte
//! bound binding independently of the row bound, saturation refusing rather
//! than shedding a boundary row, and saturation clearing on delivery. None of
//! it is restated here. What this file adds is the four things that file does
//! not reach:
//!
//! 1. **Shedding several rows at once, in strict age order.** One shed cannot
//!    distinguish "oldest first" from "any one row"; a run of three can.
//! 2. **A boundary row that is strictly the *oldest* thing in the spool.** In
//!    the store test the boundary rows are older than the capture row, so the
//!    policy is never asked to skip past an old boundary to reach a younger
//!    capture row — which is the exact case FR-785's "even when it is the
//!    oldest" clause is about.
//! 3. **What a drop record actually contains.** A count is only privacy-safe
//!    if the row it writes carries no part of what it dropped, and nothing
//!    asserts that today.
//! 4. **The command spool at its bound.** `command_spool` has no
//!    `boundary_class` and no shed path at all, and there is no test anywhere
//!    that it refuses rather than deletes.

use cairn_core::event::{
    CompactionTrigger, Disposition, EventAgent, EventContent, EventKind, FileIdentity, OpenTrigger,
    SafeCanonicalEvent,
};
use cairn_e2e::feature005::Local;
use cairn_store::outbox::SyncPolicy;
use cairn_store::repo::{start_session, StartSession};
use cairn_store::spool::{
    self, CommandAdmission, CommandKind, CommandScope, EventAdmission, NewCommand, NewEvent,
    SpoolCapacity,
};
use cairn_store::Store;
use chrono::{Duration, Utc};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

/// A store with a project, a session and an account, ready to spool into.
struct Fixture {
    local: Local,
    session: Uuid,
    account: Uuid,
}

async fn fixture() -> Fixture {
    let local = Local::new().await;
    let session = start_session(
        &local.store,
        StartSession {
            project_id: local.project,
            user_id: Uuid::now_v7(),
            agent: "claude_code",
            agent_session_key: "capacity-session",
            branch: "main",
            commit_sha: None,
            worktree_path: "/fixture/capacity",
            task_id: None,
            daemon_run_id: Uuid::now_v7(),
            // Unlinked: nothing here is about synchronization, and a linked
            // project would queue outbox work this test would then have to
            // explain away.
            policy: SyncPolicy {
                linked: false,
                server_project_id: None,
            },
        },
    )
    .await
    .expect("a session to spool events against")
    .id;
    Fixture {
        local,
        session,
        account: Uuid::now_v7(),
    }
}

impl Fixture {
    fn store(&self) -> &Store {
        &self.local.store
    }

    async fn spool(&self, capacity: SpoolCapacity, event: SafeCanonicalEvent) -> EventAdmission {
        spool::spool_event(
            self.store(),
            capacity,
            NewEvent {
                project_id: self.local.project,
                account_id: self.account,
                event,
            },
        )
        .await
        .expect("spool_event")
    }

    /// Spool and unwrap, for the rows a test is building a fixture out of
    /// rather than making an assertion about.
    async fn admit(&self, capacity: SpoolCapacity, event: SafeCanonicalEvent) -> Uuid {
        match self.spool(capacity, event).await {
            EventAdmission::Spooled { event, .. } => event.event_id,
            EventAdmission::Saturated { kind, .. } => {
                panic!("the fixture could not be built: {kind} was refused as saturated")
            }
        }
    }

    async fn scalar(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sql)
            .fetch_one(self.store().pool())
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// Every undelivered event id, oldest first — the order the shed walks.
    async fn queue_oldest_first(&self) -> Vec<Uuid> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT event_id FROM event_spool
              WHERE state IN ('pending','in_flight','failed')
              ORDER BY created_at, event_id",
        )
        .fetch_all(self.store().pool())
        .await
        .expect("queue");
        ids.iter().map(|s| s.parse().expect("uuid")).collect()
    }

    async fn disposition_total(&self, disposition: Disposition) -> i64 {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(n), 0) FROM capture_disposition_counts WHERE disposition = ?1",
        )
        .bind(disposition.as_str())
        .fetch_one(self.store().pool())
        .await
        .expect("disposition total")
    }

    /// Pin a row's creation time, so age order is a fact of the fixture rather
    /// than of how fast the machine ran.
    ///
    /// `created_at` is what the shed orders on, so writing it directly is
    /// setting the input the policy reads — not reaching around it. Two rows
    /// spooled inside the same clock tick would otherwise be separated only by
    /// the `event_id` tiebreak, and a test whose age order came from a UUID
    /// comparison would pass whatever the policy did.
    async fn age(&self, event_id: Uuid, seconds_ago: i64) {
        let when = (Utc::now() - Duration::seconds(seconds_ago)).to_rfc3339();
        sqlx::query("UPDATE event_spool SET created_at = ?1 WHERE event_id = ?2")
            .bind(when)
            .bind(event_id.to_string())
            .execute(self.store().pool())
            .await
            .expect("age a row");
    }

    /// Every column of every row of `table`, concatenated.
    ///
    /// Built from `pragma_table_info` rather than from a hand-written column
    /// list, so a column added to the table later is included automatically. A
    /// privacy assertion written against a fixed list would silently stop
    /// covering the one new column somebody put content in.
    async fn every_value_in(&self, table: &str) -> String {
        let cols: Vec<String> =
            sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .fetch_all(self.store().pool())
                .await
                .expect("columns");
        assert!(!cols.is_empty(), "{table} has no columns");
        let expr = cols
            .iter()
            .map(|c| format!("COALESCE(CAST(\"{c}\" AS TEXT), '')"))
            .collect::<Vec<_>>()
            .join(" || '\u{1f}' || ");
        let rows: Vec<String> = sqlx::query_scalar(&format!("SELECT {expr} FROM \"{table}\""))
            .fetch_all(self.store().pool())
            .await
            .expect("rows");
        rows.join("\n")
    }

    async fn columns_of(&self, table: &str) -> Vec<String> {
        let mut cols: Vec<String> =
            sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .fetch_all(self.store().pool())
                .await
                .expect("columns");
        cols.sort();
        cols
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// A capture-class event — the class the overflow policy may shed.
///
/// `marker` travels inside the payload, so a test can ask afterwards whether
/// any trace of a shed event survived anywhere.
fn capture_event(session: Uuid, marker: &str) -> SafeCanonicalEvent {
    SafeCanonicalEvent {
        // Both placeholders: the store assigns identity and ordinal, and a
        // test that pre-filled them would not notice if it stopped.
        event_id: Uuid::nil(),
        session_seq: 0,
        contract_version: cairn_core::event::CONTRACT_VERSION,
        kind: EventKind::FileRead,
        agent: EventAgent::ClaudeCode,
        vendor_event: None,
        session_id: session,
        occurred_at: Utc::now(),
        content: Some(EventContent::File {
            repo_file: Some(format!("src/{marker}.rs")),
            repo_file_from: None,
            change_kind: None,
            file_identity: FileIdentity::Present,
        }),
    }
}

/// A boundary-class event: a compaction. Never shed.
fn compacted(session: Uuid) -> SafeCanonicalEvent {
    SafeCanonicalEvent {
        event_id: Uuid::nil(),
        session_seq: 0,
        contract_version: cairn_core::event::CONTRACT_VERSION,
        kind: EventKind::ContextCompacted,
        agent: EventAgent::ClaudeCode,
        vendor_event: None,
        session_id: session,
        occurred_at: Utc::now(),
        content: Some(EventContent::Compaction {
            compaction_trigger: CompactionTrigger::Auto,
        }),
    }
}

/// The other boundary class: a session opening.
fn session_opened(session: Uuid) -> SafeCanonicalEvent {
    SafeCanonicalEvent {
        event_id: Uuid::nil(),
        session_seq: 0,
        contract_version: cairn_core::event::CONTRACT_VERSION,
        kind: EventKind::SessionOpened,
        agent: EventAgent::ClaudeCode,
        vendor_event: None,
        session_id: session,
        occurred_at: Utc::now(),
        content: Some(EventContent::SessionOpen {
            open_trigger: OpenTrigger::Startup,
        }),
    }
}

fn marker() -> String {
    format!("shed_marker_{}", Uuid::now_v7().simple())
}

// ---------------------------------------------------------------------------
// 1. Shedding is oldest-first, several at a time, and every shed is counted
// ---------------------------------------------------------------------------

/// The byte bound forces three rows out at once, and it is the three oldest.
///
/// One shed proves nothing about ordering — with five rows queued, deleting
/// "some row" and deleting "the oldest row" are indistinguishable in a single
/// step. Three consecutive sheds inside one admission are distinguishable:
/// there are sixty orderings of five rows and only one of them is age order.
///
/// The bound used here is measured from the store rather than assumed. Every
/// event in this test serializes to the same number of bytes, and the test
/// asserts that before relying on it — a payload whose length varied would
/// make the arithmetic below quietly wrong rather than loudly wrong.
///
/// **Falsified by** dropping the `ORDER BY created_at` from
/// `shed_oldest_capture_row`, by shedding newest-first, or by counting one
/// `spool_overflow_dropped` per admission instead of per row.
#[tokio::test]
async fn overflow_sheds_the_oldest_rows_first_and_counts_every_one_of_them() {
    let f = fixture().await;
    let roomy = SpoolCapacity {
        max_events: 1_000,
        max_bytes: i64::MAX,
    };

    // Five capture rows, aged so that the queue order is unambiguous: `first`
    // is fifty seconds old, `fifth` is ten.
    let mut queued = Vec::new();
    for n in 0..5 {
        let id = f.admit(roomy, capture_event(f.session, &marker())).await;
        f.age(id, 50 - n * 10).await;
        queued.push(id);
    }
    assert_eq!(
        f.queue_oldest_first().await,
        queued,
        "the fixture's own age order is not what the queue reports"
    );

    let sizes: Vec<i64> = sqlx::query_scalar("SELECT DISTINCT payload_bytes FROM event_spool")
        .fetch_all(f.store().pool())
        .await
        .expect("payload sizes");
    assert_eq!(
        sizes.len(),
        1,
        "these five events are supposed to be the same size; the byte \
         arithmetic below is only valid if they are: {sizes:?}"
    );
    let unit = sizes[0];

    // Room for three rows' worth of bytes. Admitting a sixth therefore has to
    // get the spool down to two rows before it fits, which is three sheds.
    let tight = SpoolCapacity {
        max_events: 1_000,
        max_bytes: unit * 3,
    };
    let admission = f.spool(tight, capture_event(f.session, &marker())).await;
    let EventAdmission::Spooled {
        event: sixth,
        overflow_dropped,
    } = admission
    else {
        panic!("the byte bound refused an event it had room for after shedding");
    };
    assert_eq!(
        overflow_dropped, 3,
        "the admission reported the wrong number of shed rows"
    );

    // The survivors, and their order. Naming the exact expected list rather
    // than checking a count: a policy that shed three arbitrary rows would
    // pass a count assertion and fail this one.
    assert_eq!(
        f.queue_oldest_first().await,
        vec![queued[3], queued[4], sixth.event_id],
        "overflow shed rows that were not the oldest three"
    );
    assert_eq!(
        f.disposition_total(Disposition::SpoolOverflowDropped).await,
        3,
        "the shed count is per row, not per admission: three rows left and the \
         counter has to say three"
    );
    assert_eq!(
        f.disposition_total(Disposition::SpoolSaturatedDropped)
            .await,
        0,
        "shedding to make room is not saturation, and conflating the two would \
         make a healthy busy spool look like a refusing one"
    );
}

// ---------------------------------------------------------------------------
// 2. A boundary row is never shed, not even the oldest one
// ---------------------------------------------------------------------------

/// The oldest row in the spool is a session open, and the policy walks past it.
///
/// `crates/cairn-store/tests/feature005_spool.rs` already proves a boundary row
/// survives; in that fixture the boundary rows happen to be the oldest and the
/// capture row the newest, so "shed the oldest" and "shed the oldest capture"
/// give the same answer and the test cannot tell them apart. Here they give
/// different answers at every step: the two oldest rows are boundary, and the
/// policy has to skip both of them, every time, to find something it may drop.
///
/// **Falsified by** shedding on age alone — dropping the `boundary_class = 0`
/// filter from `shed_oldest_capture_row` and ordering only by `created_at`
/// takes the session open first here, and passes the store-level test, where
/// the oldest row happens to be one the policy may drop anyway.
///
/// Demoting the filter to an *ordering preference* (`ORDER BY boundary_class,
/// created_at`) is a different defect and this test does not catch it: with a
/// capture row always available, a preference and a guard choose the same row.
/// That one is caught by
/// [`a_boundary_only_spool_at_its_bound_refuses_new_work_and_disturbs_nothing_queued`],
/// which is the case where nothing capture-class is left to prefer.
#[tokio::test]
async fn a_boundary_row_is_never_shed_even_when_it_is_the_oldest_row_in_the_spool() {
    let f = fixture().await;
    let roomy = SpoolCapacity {
        max_events: 1_000,
        max_bytes: i64::MAX,
    };

    // Oldest to newest: open, compaction, then three capture rows.
    let opened = f.admit(roomy, session_opened(f.session)).await;
    f.age(opened, 90).await;
    let compaction = f.admit(roomy, compacted(f.session)).await;
    f.age(compaction, 80).await;
    let mut captures = Vec::new();
    for n in 0..3 {
        let id = f.admit(roomy, capture_event(f.session, &marker())).await;
        f.age(id, 70 - n * 10).await;
        captures.push(id);
    }
    assert_eq!(
        f.queue_oldest_first().await,
        vec![opened, compaction, captures[0], captures[1], captures[2]],
        "the fixture is not aged the way this test needs it"
    );

    // Five rows at a bound of five. Each new event must make room, and each
    // time the two oldest rows are the ones it may not touch.
    let tight = SpoolCapacity {
        max_events: 5,
        max_bytes: i64::MAX,
    };
    let mut admitted = Vec::new();
    for n in 0..3 {
        let admission = f.spool(tight, capture_event(f.session, &marker())).await;
        let EventAdmission::Spooled {
            event,
            overflow_dropped,
        } = admission
        else {
            panic!("a capture row was still available to shed and the spool refused anyway");
        };
        assert_eq!(overflow_dropped, 1, "round {n} shed the wrong count");
        f.age(event.event_id, 5 - n).await;
        admitted.push(event.event_id);
    }

    // Every original capture row is gone, in age order; both boundary rows are
    // untouched and still the oldest things in the queue.
    assert_eq!(
        f.queue_oldest_first().await,
        vec![opened, compaction, admitted[0], admitted[1], admitted[2]],
        "the policy either kept a capture row it should have shed or shed a \
         boundary row it may never shed"
    );
    assert_eq!(
        f.scalar("SELECT COUNT(*) FROM event_spool WHERE boundary_class = 1")
            .await,
        2,
        "a boundary row was shed"
    );
}

// ---------------------------------------------------------------------------
// 3. A fully boundary-class spool saturates, and corrupts nothing
// ---------------------------------------------------------------------------

/// At the bound with nothing left to shed, new work is refused and the queue is
/// left exactly as it was.
///
/// The refusal itself is already covered at the store level. What is not is the
/// second half of FR-785's promise — *"nothing already queued is corrupted"*.
/// `spool_event` sheds inside the transaction it may then roll back, so a
/// saturating write is the one write in this module that can touch rows it does
/// not go on to keep. This asserts the rollback holds by comparing every column
/// of every queued row before and after a run of refusals.
///
/// **Falsified by** committing the transaction on the saturation path instead
/// of rolling it back: the rows shed before the policy discovered nothing
/// capture-class remained would stay deleted, and the store would have lost
/// several events in order to refuse one.
#[tokio::test]
async fn a_boundary_only_spool_at_its_bound_refuses_new_work_and_disturbs_nothing_queued() {
    let f = fixture().await;
    let tight = SpoolCapacity {
        max_events: 3,
        max_bytes: i64::MAX,
    };

    // Two boundary rows and one capture row: at the bound, but not saturated,
    // because there is still something droppable.
    f.admit(tight, session_opened(f.session)).await;
    f.admit(tight, compacted(f.session)).await;
    f.admit(tight, capture_event(f.session, &marker())).await;

    // The next boundary event sheds that last capture row. Now the spool is at
    // its bound and every row in it is one the policy may not touch.
    f.admit(tight, compacted(f.session)).await;
    let before = f.every_value_in("event_spool").await;
    let breakdown = spool::event_spool_breakdown(f.store(), tight)
        .await
        .expect("breakdown");
    assert!(
        breakdown.saturated,
        "a full spool with nothing capture-class left is saturated by definition"
    );

    // Four refusals: three capture and one boundary. Saturation is not
    // selective — admitting a boundary row here would put the spool over its
    // bound with nothing to shed next time.
    let mut refusals = 0;
    for n in 0..4 {
        let event = if n == 3 {
            compacted(f.session)
        } else {
            capture_event(f.session, &marker())
        };
        match f.spool(tight, event).await {
            EventAdmission::Saturated { session_id, kind } => {
                assert_eq!(
                    session_id, f.session,
                    "the refusal did not name its session"
                );
                assert!(
                    matches!(kind, EventKind::FileRead | EventKind::ContextCompacted),
                    "the refusal did not name the kind it refused: {kind}"
                );
                refusals += 1;
            }
            other => panic!("a saturated spool admitted an event: {other:?}"),
        }
    }
    assert_eq!(refusals, 4);
    assert_eq!(
        f.disposition_total(Disposition::SpoolSaturatedDropped)
            .await,
        4,
        "every refusal is counted: the count is the only trace a saturated \
         store leaves of the event it could not take"
    );

    assert_eq!(
        f.every_value_in("event_spool").await,
        before,
        "refusing new work changed a row that was already queued — the \
         saturation rollback is not holding"
    );
}

/// A refusal puts back every row it shed on the way to discovering it had to
/// refuse.
///
/// This is the case the rollback in `spool_event` exists for, and it is
/// narrower than it looks. When the spool is *already* all boundary-class, the
/// shed loop finds nothing on its first pass and refuses having deleted
/// nothing — so committing instead of rolling back is indistinguishable from
/// rolling back, and the test above cannot see the difference. The rollback is
/// only load-bearing when the loop sheds real rows and *then* runs out: the
/// bound is consumed by rows the policy may not touch, so no amount of
/// shedding will ever make room.
///
/// A byte bound exactly equal to what the boundary rows already occupy is that
/// shape. Every capture row is shed, the event still does not fit, and the
/// store has to end the write holding what it held before — otherwise it has
/// destroyed two events in order to refuse a third, which is the arithmetic
/// FR-785 is least willing to accept.
///
/// **Falsified by** committing the transaction on the saturation path instead
/// of rolling it back.
#[tokio::test]
async fn a_refusal_puts_back_every_row_it_shed_before_it_discovered_it_had_to_refuse() {
    let f = fixture().await;
    let roomy = SpoolCapacity {
        max_events: 1_000,
        max_bytes: i64::MAX,
    };

    // Two boundary rows, then two capture rows the policy is allowed to shed.
    f.admit(roomy, session_opened(f.session)).await;
    f.admit(roomy, compacted(f.session)).await;
    let shed_marker = marker();
    f.admit(roomy, capture_event(f.session, &shed_marker)).await;
    f.admit(roomy, capture_event(f.session, &marker())).await;

    // A bound already fully consumed by rows that may never be shed. Shedding
    // both capture rows still does not make room, because the boundary rows
    // alone are the whole budget.
    let boundary_bytes = f
        .scalar("SELECT COALESCE(SUM(payload_bytes), 0) FROM event_spool WHERE boundary_class = 1")
        .await;
    let impossible = SpoolCapacity {
        max_events: 1_000,
        max_bytes: boundary_bytes,
    };

    let before = f.every_value_in("event_spool").await;
    assert!(
        before.contains(&shed_marker),
        "the fixture's capture rows are not in the spool, so their survival \
         would prove nothing"
    );

    let admission = f
        .spool(impossible, capture_event(f.session, &marker()))
        .await;
    assert!(
        matches!(admission, EventAdmission::Saturated { .. }),
        "a bound that cannot be met even after shedding everything droppable \
         has to refuse, not admit: {admission:?}"
    );

    assert_eq!(
        f.every_value_in("event_spool").await,
        before,
        "the refusal kept the rows it shed on the way to refusing: two events \
         were destroyed so that a third could be turned away"
    );
    assert_eq!(
        f.disposition_total(Disposition::SpoolSaturatedDropped)
            .await,
        1,
        "one event was refused, so exactly one refusal is counted"
    );
    // The shed counts are rolled back with the rows. A store that reported two
    // overflow drops while holding both rows would be describing a spool that
    // does not exist.
    assert_eq!(
        f.disposition_total(Disposition::SpoolOverflowDropped).await,
        0,
        "rows that came back were counted as dropped"
    );
}

// ---------------------------------------------------------------------------
// 4. A drop record is a count and nothing else
// ---------------------------------------------------------------------------

/// Neither kind of drop record carries any part of what it dropped.
///
/// This is FR-749d and FR-741 applied to the one place in the feature where
/// Cairn writes a record *about* an event it is discarding. A drop record that
/// quoted the path it dropped would put the material on the durable side of a
/// boundary the discard exists to respect, and it would do so in the table
/// health reporting reads out loud.
///
/// Asserted positively, in two directions:
///
/// - the table's column set is named in full, so a column capable of holding
///   content cannot be added without this test failing; and
/// - a marker that provably *was* in the spooled payload is looked for across
///   every column of every row of the counts table afterwards.
///
/// The "provably was" half matters: without it a marker that never made it into
/// the store in the first place would make the absence assertion vacuous.
///
/// **Falsified by** giving `record_disposition` a `detail`, `sample` or
/// `last_payload` column, or by counting a drop with the event's `repo_file` as
/// part of the key.
#[tokio::test]
async fn a_drop_record_carries_counts_and_nothing_of_what_was_dropped() {
    let f = fixture().await;
    let roomy = SpoolCapacity {
        max_events: 1_000,
        max_bytes: i64::MAX,
    };

    // The exact columns. Anything else is a place content could live.
    assert_eq!(
        f.columns_of("capture_disposition_counts").await,
        vec!["agent", "day", "disposition", "kind", "n", "project_id"],
        "a column was added to the disposition counts; a drop record is a \
         count of what happened, and every one of these six names is either an \
         identifier or a number (FR-749d)"
    );

    // A shed row, with a marker inside its payload.
    let shed_marker = marker();
    let shed = f.admit(roomy, capture_event(f.session, &shed_marker)).await;
    f.age(shed, 60).await;
    assert!(
        f.every_value_in("event_spool").await.contains(&shed_marker),
        "the marker never reached the spool, so its later absence would prove \
         nothing"
    );

    // A boundary row, so the next capture event has to shed rather than be
    // refused, and then a second so the spool becomes saturated.
    let tight = SpoolCapacity {
        max_events: 2,
        max_bytes: i64::MAX,
    };
    f.admit(tight, session_opened(f.session)).await;

    let overflow_marker = marker();
    let admission = f
        .spool(tight, capture_event(f.session, &overflow_marker))
        .await;
    assert!(
        matches!(
            admission,
            EventAdmission::Spooled {
                overflow_dropped: 1,
                ..
            }
        ),
        "expected the aged capture row to be shed, got {admission:?}"
    );

    // Now the spool holds one boundary row and one capture row at a bound of
    // two; one more boundary event sheds the capture row, and the one after
    // that is refused.
    f.admit(tight, compacted(f.session)).await;
    let saturated_marker = marker();
    assert!(
        matches!(
            f.spool(tight, capture_event(f.session, &saturated_marker))
                .await,
            EventAdmission::Saturated { .. }
        ),
        "the fixture needs a saturating refusal here"
    );

    let counts = f.every_value_in("capture_disposition_counts").await;
    assert!(
        counts.contains("spool_overflow_dropped") && counts.contains("spool_saturated_dropped"),
        "both drops have to have been recorded for their contents to be worth \
         checking: {counts}"
    );
    for (what, leaked) in [
        ("the shed row's payload", &shed_marker),
        ("the admitted row's payload", &overflow_marker),
        ("the refused event's payload", &saturated_marker),
    ] {
        assert!(
            !counts.contains(leaked.as_str()),
            "a disposition count carries {what}: {counts}"
        );
    }
    // The whole content, not just the marker: a record that stored the kind's
    // structured body under a different name would not contain the marker
    // string on its own.
    for forbidden in ["src/", ".rs", "repo_file", "payload", "content"] {
        assert!(
            !counts.contains(forbidden),
            "a disposition count carries `{forbidden}`, which is payload shape \
             rather than a count: {counts}"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. The command spool refuses; it never sheds
// ---------------------------------------------------------------------------

fn command_payload() -> serde_json::Value {
    serde_json::json!({ "content": "an intent, and nothing derived" })
}

async fn spool_one(f: &Fixture, capacity: SpoolCapacity, scope: CommandScope) -> CommandAdmission {
    let payload = command_payload();
    spool::spool_command(
        f.store(),
        NewCommand {
            scope,
            project_id: Some(f.local.project),
            account_id: f.account,
            kind: CommandKind::Remember,
            payload: &payload,
        },
        capacity,
    )
    .await
    .expect("spool_command")
}

/// At its bound the command spool answers `Saturated` and drops nothing.
///
/// An event is one observation out of a stream Cairn generates on its own; a
/// command is something a person explicitly asked Cairn to do. The contract
/// says a command "is not retried forever and it is not silently dropped"
/// (`contracts/knowledge-commands.md` §4), so the shedding half of FR-785 has
/// nothing to operate on here — and the schema says so too: `command_spool`
/// has no `boundary_class`, because there is no such thing as a droppable
/// command. That absence is asserted rather than assumed, because a column
/// added later is how the shed path would become expressible.
///
/// **Falsified by** giving the command spool an overflow policy of any kind, or
/// by reporting the refused command to the caller as accepted.
#[tokio::test]
async fn the_command_spool_refuses_at_its_bound_and_sheds_nothing() {
    let f = fixture().await;
    assert!(
        !f.columns_of("command_spool")
            .await
            .contains(&"boundary_class".to_string()),
        "`command_spool` grew a `boundary_class`: the only reason to classify a \
         command is to decide which ones may be dropped, and none may"
    );

    let tight = SpoolCapacity {
        max_events: 3,
        max_bytes: i64::MAX,
    };
    let scope = CommandScope::Session(f.session);
    let mut queued = Vec::new();
    for _ in 0..3 {
        match spool_one(&f, tight, scope).await {
            CommandAdmission::Spooled(c) => queued.push(c),
            CommandAdmission::Saturated { queued } => {
                panic!("the command spool refused below its bound at {queued}")
            }
        }
    }
    assert_eq!(
        queued.iter().map(|c| c.command_seq).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "ordinals are consumed in order"
    );

    let before = f.every_value_in("command_spool").await;
    for _ in 0..3 {
        match spool_one(&f, tight, scope).await {
            CommandAdmission::Saturated { queued } => assert_eq!(
                queued, 3,
                "the refusal has to say how deep the queue actually is, or it \
                 tells the user nothing they can act on"
            ),
            CommandAdmission::Spooled(c) => {
                panic!("a full command spool accepted command {}", c.command_id)
            }
        }
    }
    assert_eq!(
        f.every_value_in("command_spool").await,
        before,
        "a refusal changed the queue: no explicit command is droppable"
    );
    assert!(
        spool::command_spool_breakdown(f.store(), tight)
            .await
            .expect("breakdown")
            .saturated,
        "a command spool that is refusing work has to report itself saturated"
    );

    // Nothing was counted as dropped, because nothing was. The command spool
    // has no shed path, and a refusal is not a drop.
    assert_eq!(
        f.disposition_total(Disposition::SpoolOverflowDropped).await,
        0
    );
}

/// Command saturation clears on delivery, with nobody clearing a flag.
///
/// Saturation is derived from the rows rather than stored, so draining one row
/// is the whole recovery. Asserted for commands specifically: the event side
/// has this test at the store level and the command side does not, and the two
/// go through different branches of `breakdown`.
///
/// The refused commands also consumed no ordinal — the next accepted command is
/// number four, not number seven — which is what makes a refusal recoverable
/// rather than a permanent hole in the scope's sequence.
///
/// **Falsified by** storing saturation as a column, or by allocating the
/// ordinal before the capacity check in `spool_command`.
#[tokio::test]
async fn command_saturation_clears_when_delivery_drains_a_row() {
    let f = fixture().await;
    let tight = SpoolCapacity {
        max_events: 2,
        max_bytes: i64::MAX,
    };
    let scope = CommandScope::Session(f.session);

    let first = match spool_one(&f, tight, scope).await {
        CommandAdmission::Spooled(c) => c,
        other => panic!("expected a queued command, got {other:?}"),
    };
    let _second = spool_one(&f, tight, scope).await;
    assert!(matches!(
        spool_one(&f, tight, scope).await,
        CommandAdmission::Saturated { .. }
    ));

    spool::mark_command_delivered(f.store(), first.command_id)
        .await
        .expect("delivered");
    assert!(
        !spool::command_spool_breakdown(f.store(), tight)
            .await
            .expect("breakdown")
            .saturated,
        "delivery drained a row and saturation did not clear on its own"
    );

    match spool_one(&f, tight, scope).await {
        CommandAdmission::Spooled(c) => assert_eq!(
            c.command_seq, 3,
            "the refused command consumed an ordinal, leaving a gap in the \
             scope's sequence that no reader can explain"
        ),
        other => panic!("the spool had room and refused anyway: {other:?}"),
    }
}
