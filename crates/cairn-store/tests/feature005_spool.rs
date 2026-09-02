//! T022 — the edge spools' load-bearing properties.
//!
//! Each test below is written to fail if one specific protection is removed,
//! rather than to exercise the happy path: a durable ordinal that a drain
//! cannot reset, a claim that is bound to exactly one account, a capacity
//! policy that would rather refuse new work than corrupt queued work, and a
//! command queue that delivers a scope in the order it was issued.
//!
//! A file-backed store throughout, not `:memory:`. The ordinal and claim
//! properties are about two connections contending for the same rows, and a
//! single shared in-memory connection cannot contend with itself.

use cairn_core::domain::{ApplicabilityFact, MemoryType, TeamState};
use cairn_core::event::{
    CompactionTrigger, EventAgent, EventContent, EventKind, FileIdentity, OpenTrigger,
    SafeCanonicalEvent,
};
use cairn_core::eventid;
use cairn_store::global::{
    create_personal, get_personal, merge_synced_personal, merge_synced_team, NewPersonalKnowledge,
    SyncedPersonalKnowledge, SyncedTeamKnowledge,
};
use cairn_store::outbox::SyncPolicy;
use cairn_store::repo::{ensure_project, start_session, StartSession};
use cairn_store::spool::{
    self, CommandKind, CommandScope, EventAdmission, NewCommand, NewEvent, SpoolCapacity,
    BACKOFF_CEILING_SECONDS, CLAIM_LEASE_SECONDS,
};
use cairn_store::Store;
use chrono::Utc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    project_id: Uuid,
    session_id: Uuid,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(&dir.path().join("spool.sqlite3"))
        .await
        .expect("store");
    let project = ensure_project(&store, "/tmp/spool/.git", "spool", None)
        .await
        .expect("project");
    let session = start_session(
        &store,
        StartSession {
            project_id: project.id,
            user_id: Uuid::now_v7(),
            agent: "claude_code",
            agent_session_key: "spool-session",
            branch: "main",
            commit_sha: None,
            worktree_path: "/tmp/spool",
            task_id: None,
            daemon_run_id: Uuid::now_v7(),
            policy: SyncPolicy {
                linked: false,
                server_project_id: None,
            },
        },
    )
    .await
    .expect("session");
    Fixture {
        _dir: dir,
        project_id: project.id,
        session_id: session.id,
        store,
    }
}

/// A capture-class event: the class the overflow policy is allowed to shed.
fn capture_event(session_id: Uuid) -> SafeCanonicalEvent {
    SafeCanonicalEvent {
        // Both of these are placeholders. The spool assigns the real values,
        // and a test that pre-filled them would not notice if it stopped.
        event_id: Uuid::nil(),
        session_seq: 0,
        contract_version: 1,
        kind: EventKind::FileRead,
        agent: EventAgent::ClaudeCode,
        vendor_event: None,
        session_id,
        occurred_at: Utc::now(),
        content: Some(EventContent::File {
            repo_file: Some("src/lib.rs".to_string()),
            repo_file_from: None,
            change_kind: None,
            file_identity: FileIdentity::Present,
        }),
    }
}

/// A boundary-class event: the class the overflow policy must never shed.
fn boundary_event(session_id: Uuid) -> SafeCanonicalEvent {
    SafeCanonicalEvent {
        event_id: Uuid::nil(),
        session_seq: 0,
        contract_version: 1,
        kind: EventKind::ContextCompacted,
        agent: EventAgent::ClaudeCode,
        vendor_event: None,
        session_id,
        occurred_at: Utc::now(),
        content: Some(EventContent::Compaction {
            compaction_trigger: CompactionTrigger::Auto,
        }),
    }
}

fn session_opened(session_id: Uuid) -> SafeCanonicalEvent {
    SafeCanonicalEvent {
        event_id: Uuid::nil(),
        session_seq: 0,
        contract_version: 1,
        kind: EventKind::SessionOpened,
        agent: EventAgent::ClaudeCode,
        vendor_event: None,
        session_id,
        occurred_at: Utc::now(),
        content: Some(EventContent::SessionOpen {
            open_trigger: OpenTrigger::Startup,
        }),
    }
}

async fn spool(f: &Fixture, account: Uuid, event: SafeCanonicalEvent) -> EventAdmission {
    spool_with(f, SpoolCapacity::default(), account, event).await
}

async fn spool_with(
    f: &Fixture,
    capacity: SpoolCapacity,
    account: Uuid,
    event: SafeCanonicalEvent,
) -> EventAdmission {
    spool::spool_event(
        &f.store,
        capacity,
        NewEvent {
            project_id: f.project_id,
            account_id: account,
            event,
        },
    )
    .await
    .expect("spool_event")
}

fn spooled(admission: EventAdmission) -> SafeCanonicalEvent {
    match admission {
        EventAdmission::Spooled { event, .. } => event,
        EventAdmission::Saturated { session_id, kind } => {
            panic!("expected the event to be spooled, got saturation on {kind} in {session_id}")
        }
    }
}

async fn scalar(f: &Fixture, sql: &str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(f.store.pool())
        .await
        .expect("scalar")
}

async fn disposition_total(f: &Fixture, disposition: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(n), 0) FROM capture_disposition_counts WHERE disposition = ?1",
    )
    .bind(disposition)
    .fetch_one(f.store.pool())
    .await
    .expect("disposition total")
}

/// A row's current state, read from the table rather than inferred.
async fn state_of(f: &Fixture, event_id: Uuid) -> String {
    sqlx::query_scalar("SELECT state FROM event_spool WHERE event_id = ?1")
        .bind(event_id.to_string())
        .fetch_one(f.store.pool())
        .await
        .expect("state")
}

/// Put a row's attempt count where a test needs it, rather than driving it
/// there through two thousand real delivery attempts.
async fn set_attempts(f: &Fixture, event_id: Uuid, attempts: i64) {
    sqlx::query("UPDATE event_spool SET attempts = ?1 WHERE event_id = ?2")
        .bind(attempts)
        .bind(event_id.to_string())
        .execute(f.store.pool())
        .await
        .expect("set attempts");
}

/// Move a row's backoff into the past so the next claim is due now.
async fn expire_backoff(f: &Fixture, event_id: Uuid) {
    sqlx::query("UPDATE event_spool SET next_attempt_at = ?1 WHERE event_id = ?2")
        .bind((Utc::now() - chrono::Duration::hours(1)).to_rfc3339())
        .bind(event_id.to_string())
        .execute(f.store.pool())
        .await
        .expect("expire backoff");
}

/// Age a claim so the lease has expired, without waiting a minute for it.
async fn age_claim(f: &Fixture, event_id: Uuid) {
    let stale = Utc::now() - chrono::Duration::seconds(CLAIM_LEASE_SECONDS + 5);
    sqlx::query("UPDATE event_spool SET claimed_at = ?1 WHERE event_id = ?2")
        .bind(stale.to_rfc3339())
        .bind(event_id.to_string())
        .execute(f.store.pool())
        .await
        .expect("age claim");
}

// ---------------------------------------------------------------------------
// Durable ordinals
// ---------------------------------------------------------------------------

/// Twenty-four writers on one session receive twenty-four distinct ordinals and
/// twenty-four distinct identities, with no hole and no repeat.
///
/// Falsified by allocating the ordinal outside the inserting transaction, which
/// is where a check-then-act would hand two events the same number and, through
/// the UUIDv5 derivation, the same `event_id`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writers_on_one_session_each_get_their_own_ordinal() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let (project_id, session_id) = (f.project_id, f.session_id);

    let mut writers = Vec::new();
    for _ in 0..24 {
        let store = f.store.clone();
        writers.push(tokio::spawn(async move {
            let admission = spool::spool_event(
                &store,
                SpoolCapacity::default(),
                NewEvent {
                    project_id,
                    account_id: account,
                    event: capture_event(session_id),
                },
            )
            .await
            .expect("spool_event");
            match admission {
                EventAdmission::Spooled { event, .. } => (event.session_seq, event.event_id),
                other => panic!("expected a spooled event, got {other:?}"),
            }
        }));
    }

    let mut assigned = Vec::new();
    for w in writers {
        assigned.push(w.await.expect("writer finishes"));
    }
    let mut seqs: Vec<u64> = assigned.iter().map(|(s, _)| *s).collect();
    seqs.sort_unstable();
    assert_eq!(
        seqs,
        (1..=24u64).collect::<Vec<_>>(),
        "the ordinals are not the gapless run 1..=24"
    );

    let ids: std::collections::HashSet<Uuid> = assigned.iter().map(|(_, id)| *id).collect();
    assert_eq!(ids.len(), 24, "two events were given one identity");
    for (seq, id) in &assigned {
        assert_eq!(
            *id,
            eventid::event_id(session_id, *seq),
            "the stored identity is not the derivation of its own ordinal"
        );
    }
    assert_eq!(scalar(&f, "SELECT COUNT(*) FROM event_spool").await, 24);
}

/// The ordinal survives the spool draining completely.
///
/// This is the whole reason `session_event_seq` exists. Falsified by deriving
/// the counter from `MAX(session_seq)` over `event_spool`: with the session's
/// rows delivered and pruned, the next event would be numbered 1 again and
/// would re-derive an `event_id` the server has already stored, which the
/// server answers `duplicate` — losing a real event in silence.
#[tokio::test]
async fn the_ordinal_does_not_reset_when_the_spool_drains() {
    let f = fixture().await;
    let account = Uuid::now_v7();

    let mut delivered = Vec::new();
    for _ in 0..3 {
        let event = spooled(spool(&f, account, capture_event(f.session_id)).await);
        spool::mark_event_delivered(&f.store, event.event_id)
            .await
            .expect("delivered");
        delivered.push(event.event_id);
    }
    // A drain that prunes what it delivered, which is the state the counter has
    // to be independent of.
    sqlx::query("DELETE FROM event_spool WHERE state = 'delivered'")
        .execute(f.store.pool())
        .await
        .expect("prune");
    assert_eq!(scalar(&f, "SELECT COUNT(*) FROM event_spool").await, 0);

    let next = spooled(spool(&f, account, capture_event(f.session_id)).await);
    assert_eq!(next.session_seq, 4, "the ordinal restarted after a drain");
    assert!(
        !delivered.contains(&next.event_id),
        "the ordinal reset re-derived an identity a delivered event already used"
    );
}

/// A saturated refusal consumes no ordinal, so the run stays gapless.
#[tokio::test]
async fn a_refused_event_leaves_the_ordinal_unconsumed() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let tight = SpoolCapacity {
        max_events: 1,
        max_bytes: i64::MAX,
    };
    let first = spooled(spool_with(&f, tight, account, boundary_event(f.session_id)).await);
    assert_eq!(first.session_seq, 1);

    let refused = spool_with(&f, tight, account, capture_event(f.session_id)).await;
    assert!(matches!(refused, EventAdmission::Saturated { .. }));

    spool::mark_event_delivered(&f.store, first.event_id)
        .await
        .expect("delivered");
    let next = spooled(spool_with(&f, tight, account, capture_event(f.session_id)).await);
    assert_eq!(
        next.session_seq, 2,
        "a refusal consumed an ordinal it never used"
    );
}

// ---------------------------------------------------------------------------
// Claims
// ---------------------------------------------------------------------------

/// A drainer sees exactly its own account's rows, and never another's.
///
/// Falsified by any `account_id IS NULL OR account_id = ?` predicate — the
/// regression this repository has introduced and repaired twice (FR-790,
/// FR-864a).
#[tokio::test]
async fn a_claim_returns_only_the_signed_in_accounts_own_rows() {
    let f = fixture().await;
    let mine = Uuid::now_v7();
    let theirs = Uuid::now_v7();

    let a = spooled(spool(&f, mine, capture_event(f.session_id)).await);
    let b = spooled(spool(&f, theirs, capture_event(f.session_id)).await);

    let claimed = spool::claim_events(&f.store, mine, 10)
        .await
        .expect("claim");
    assert_eq!(claimed.len(), 1, "claimed something that was not mine");
    assert_eq!(claimed[0].event_id, a.event_id);

    let state: String = sqlx::query_scalar("SELECT state FROM event_spool WHERE event_id = ?1")
        .bind(b.event_id.to_string())
        .fetch_one(f.store.pool())
        .await
        .expect("state");
    assert_eq!(
        state, "pending",
        "another account's row was moved by a claim it was not eligible for"
    );
}

/// The other account's row is still not deliverable when it is the only row
/// left and its claim has long since expired.
///
/// Separate from the test above because a predicate can be exactly right about
/// the pending case and still fall open on the stale-reclaim branch, which is
/// its own `OR` arm.
#[tokio::test]
async fn a_stale_row_belonging_to_another_account_is_still_not_claimable() {
    let f = fixture().await;
    let theirs = Uuid::now_v7();
    let mine = Uuid::now_v7();
    let row = spooled(spool(&f, theirs, capture_event(f.session_id)).await);

    spool::claim_events(&f.store, theirs, 10)
        .await
        .expect("their claim");
    age_claim(&f, row.event_id).await;

    let mine_claimed = spool::claim_events(&f.store, mine, 10)
        .await
        .expect("claim");
    assert!(
        mine_claimed.is_empty(),
        "an expired lease made another account's row deliverable under my credential"
    );
}

/// A claimed row is invisible to a second drainer until its lease expires, and
/// then returns to the queue.
#[tokio::test]
async fn an_abandoned_claim_is_reclaimed_once_its_lease_expires() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let row = spooled(spool(&f, account, capture_event(f.session_id)).await);

    let first = spool::claim_events(&f.store, account, 10)
        .await
        .expect("claim");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].attempts, 1);

    assert!(
        spool::claim_events(&f.store, account, 10)
            .await
            .expect("claim")
            .is_empty(),
        "a live claim was handed to a second drainer"
    );

    age_claim(&f, row.event_id).await;
    let reclaimed = spool::claim_events(&f.store, account, 10)
        .await
        .expect("claim");
    assert_eq!(reclaimed.len(), 1, "an abandoned claim stranded its row");
    assert_eq!(
        reclaimed[0].attempts, 2,
        "the reclaim did not count as an attempt"
    );
}

/// Two drainers racing for the same queue split it; no row is delivered twice.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_drainers_never_claim_the_same_row() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    for _ in 0..40 {
        spool(&f, account, capture_event(f.session_id)).await;
    }

    let mut drainers = Vec::new();
    for _ in 0..4 {
        let store = f.store.clone();
        drainers.push(tokio::spawn(async move {
            let mut got = Vec::new();
            for _ in 0..5 {
                let batch = spool::claim_events(&store, account, 7)
                    .await
                    .expect("claim");
                got.extend(batch.into_iter().map(|e| e.event_id));
            }
            got
        }));
    }
    let mut claimed = Vec::new();
    for d in drainers {
        claimed.extend(d.await.expect("drainer finishes"));
    }
    let distinct: std::collections::HashSet<_> = claimed.iter().copied().collect();
    assert_eq!(
        claimed.len(),
        distinct.len(),
        "a row was claimed by two drainers at once"
    );
    assert_eq!(distinct.len(), 40, "every row must be claimed exactly once");
}

// ---------------------------------------------------------------------------
// Backoff and refusal
// ---------------------------------------------------------------------------

/// A transient failure parks the row for its backoff and no longer.
#[tokio::test]
async fn a_failed_row_waits_out_its_backoff_before_it_is_claimable_again() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let row = spooled(spool(&f, account, capture_event(f.session_id)).await);
    spool::claim_events(&f.store, account, 10)
        .await
        .expect("claim");
    spool::mark_event_failed(&f.store, row.event_id, "unreachable")
        .await
        .expect("failed");

    assert!(
        spool::claim_events(&f.store, account, 10)
            .await
            .expect("claim")
            .is_empty(),
        "a row in backoff was claimed immediately, so the backoff is not enforced"
    );

    sqlx::query("UPDATE event_spool SET next_attempt_at = ?1 WHERE event_id = ?2")
        .bind((Utc::now() - chrono::Duration::seconds(5)).to_rfc3339())
        .bind(row.event_id.to_string())
        .execute(f.store.pool())
        .await
        .expect("elapse the backoff");
    let again = spool::claim_events(&f.store, account, 10)
        .await
        .expect("claim");
    assert_eq!(again.len(), 1, "an elapsed backoff did not return the row");
    assert_eq!(again[0].attempts, 2);
}

/// The backoff is bounded at both ends: one second at the first retry, five
/// minutes however many failures accumulate.
#[tokio::test]
async fn the_backoff_is_bounded_between_one_second_and_five_minutes() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let row = spooled(spool(&f, account, capture_event(f.session_id)).await);
    spool::claim_events(&f.store, account, 10)
        .await
        .expect("claim");

    let before = Utc::now();
    spool::mark_event_failed(&f.store, row.event_id, "unreachable")
        .await
        .expect("failed");
    let first: String =
        sqlx::query_scalar("SELECT next_attempt_at FROM event_spool WHERE event_id = ?1")
            .bind(row.event_id.to_string())
            .fetch_one(f.store.pool())
            .await
            .expect("next_attempt_at");
    let waited = (chrono::DateTime::parse_from_rfc3339(&first)
        .expect("rfc3339")
        .with_timezone(&Utc)
        - before)
        .num_milliseconds();
    assert!(
        (900..2_500).contains(&waited),
        "the first retry waited {waited}ms, which is not the stated one second"
    );

    sqlx::query("UPDATE event_spool SET attempts = 30 WHERE event_id = ?1")
        .bind(row.event_id.to_string())
        .execute(f.store.pool())
        .await
        .expect("many attempts");
    let before = Utc::now();
    spool::mark_event_failed(&f.store, row.event_id, "unreachable")
        .await
        .expect("failed");
    let capped: String =
        sqlx::query_scalar("SELECT next_attempt_at FROM event_spool WHERE event_id = ?1")
            .bind(row.event_id.to_string())
            .fetch_one(f.store.pool())
            .await
            .expect("next_attempt_at");
    let waited = (chrono::DateTime::parse_from_rfc3339(&capped)
        .expect("rfc3339")
        .with_timezone(&Utc)
        - before)
        .num_seconds();
    assert!(
        (BACKOFF_CEILING_SECONDS - 2..=BACKOFF_CEILING_SECONDS + 2).contains(&waited),
        "thirty failures produced a {waited}s wait rather than the five-minute ceiling"
    );
}

/// A rejection is permanent: the row moves to `refused`, is never claimed
/// again, and stays where someone can see it (FR-772, FR-784).
#[tokio::test]
async fn a_rejected_event_is_refused_permanently_and_stays_visible() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let row = spooled(spool(&f, account, capture_event(f.session_id)).await);
    spool::claim_events(&f.store, account, 10)
        .await
        .expect("claim");
    spool::mark_event_refused(&f.store, row.event_id, "event_id_mismatch")
        .await
        .expect("refused");

    age_claim(&f, row.event_id).await;
    assert!(
        spool::claim_events(&f.store, account, 10)
            .await
            .expect("claim")
            .is_empty(),
        "a permanently refused row was retried"
    );
    let status = spool::event_spool_status(&f.store, SpoolCapacity::default())
        .await
        .expect("status");
    assert_eq!(status.refused, 1, "the refusal is not reportable");
    assert_eq!(
        status.undelivered, 0,
        "a refused row is still counted as queued work"
    );
    assert_eq!(disposition_total(&f, "rejected_by_server").await, 1);
}

/// Every claim is returned to the queue at daemon start, without waiting out a
/// lease belonging to a run that is already gone.
#[tokio::test]
async fn releasing_claims_returns_in_flight_rows_to_the_queue() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    for _ in 0..3 {
        spool(&f, account, capture_event(f.session_id)).await;
    }
    spool::claim_events(&f.store, account, 10)
        .await
        .expect("claim");
    assert_eq!(
        spool::release_event_claims(&f.store)
            .await
            .expect("release"),
        3
    );
    assert_eq!(
        spool::claim_events(&f.store, account, 10)
            .await
            .expect("claim")
            .len(),
        3
    );
}

// ---------------------------------------------------------------------------
// Capacity
// ---------------------------------------------------------------------------

/// Overflow sheds the oldest capture-class row and counts it.
#[tokio::test]
async fn overflow_sheds_the_oldest_capture_row_and_counts_the_drop() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let tight = SpoolCapacity {
        max_events: 3,
        max_bytes: i64::MAX,
    };
    let mut spooled_ids = Vec::new();
    for _ in 0..3 {
        spooled_ids.push(
            spooled(spool_with(&f, tight, account, capture_event(f.session_id)).await).event_id,
        );
    }

    let admission = spool_with(&f, tight, account, capture_event(f.session_id)).await;
    match admission {
        EventAdmission::Spooled {
            overflow_dropped, ..
        } => assert_eq!(
            overflow_dropped, 1,
            "the fourth event shed the wrong number"
        ),
        other => panic!("expected the fourth event to be admitted, got {other:?}"),
    }

    assert_eq!(scalar(&f, "SELECT COUNT(*) FROM event_spool").await, 3);
    let oldest_present: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM event_spool WHERE event_id = ?1")
            .bind(spooled_ids[0].to_string())
            .fetch_one(f.store.pool())
            .await
            .expect("count");
    assert_eq!(
        oldest_present, 0,
        "overflow shed something other than the oldest row"
    );
    assert_eq!(disposition_total(&f, "spool_overflow_dropped").await, 1);
}

/// The byte bound binds independently of the row bound.
#[tokio::test]
async fn the_byte_bound_sheds_even_when_the_row_count_is_far_below_its_bound() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    spool(&f, account, capture_event(f.session_id)).await;
    let unit = scalar(&f, "SELECT payload_bytes FROM event_spool").await;

    let byte_bound = SpoolCapacity {
        max_events: 10_000,
        max_bytes: unit * 2 + 8,
    };
    spool_with(&f, byte_bound, account, capture_event(f.session_id)).await;
    let third = spool_with(&f, byte_bound, account, capture_event(f.session_id)).await;
    match third {
        EventAdmission::Spooled {
            overflow_dropped, ..
        } => assert_eq!(overflow_dropped, 1),
        other => panic!("expected admission after shedding, got {other:?}"),
    }
    assert_eq!(
        scalar(&f, "SELECT COUNT(*) FROM event_spool").await,
        2,
        "the byte bound did not bind while the row count still had room"
    );
}

/// A boundary row is never shed, and when nothing else is left the spool
/// refuses new work instead (FR-785).
///
/// This is the property the whole capacity policy is arranged around: session
/// open, close and compaction rows are what every other event is interpreted
/// relative to, so shedding one would not lose an event, it would corrupt the
/// reading of everything still queued.
#[tokio::test]
async fn saturation_refuses_new_events_rather_than_shedding_a_boundary_row() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let tight = SpoolCapacity {
        max_events: 3,
        max_bytes: i64::MAX,
    };

    let mut boundary_ids = Vec::new();
    boundary_ids
        .push(spooled(spool_with(&f, tight, account, session_opened(f.session_id)).await).event_id);
    boundary_ids
        .push(spooled(spool_with(&f, tight, account, boundary_event(f.session_id)).await).event_id);
    spool_with(&f, tight, account, capture_event(f.session_id)).await;
    assert!(
        !spool::event_spool_status(&f.store, tight)
            .await
            .expect("status")
            .saturated,
        "a spool with a capture row left is not saturated"
    );

    // The third boundary row sheds the last capture row rather than being
    // refused: shedding capture is still the first resort.
    boundary_ids
        .push(spooled(spool_with(&f, tight, account, boundary_event(f.session_id)).await).event_id);
    assert_eq!(
        scalar(
            &f,
            "SELECT COUNT(*) FROM event_spool WHERE boundary_class = 1"
        )
        .await,
        3
    );
    assert_eq!(disposition_total(&f, "spool_overflow_dropped").await, 1);

    let refused = spool_with(&f, tight, account, capture_event(f.session_id)).await;
    match refused {
        EventAdmission::Saturated { session_id, kind } => {
            assert_eq!(
                session_id, f.session_id,
                "the refusal did not name its session"
            );
            assert_eq!(
                kind,
                EventKind::FileRead,
                "the refusal did not name its kind"
            );
        }
        other => panic!("expected saturation, got {other:?}"),
    }
    assert_eq!(disposition_total(&f, "spool_saturated_dropped").await, 1);
    assert_eq!(
        scalar(
            &f,
            "SELECT COUNT(*) FROM event_spool WHERE boundary_class = 1"
        )
        .await,
        3,
        "saturation shed a boundary row"
    );
    for id in &boundary_ids {
        let present: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM event_spool WHERE event_id = ?1")
                .bind(id.to_string())
                .fetch_one(f.store.pool())
                .await
                .expect("count");
        assert_eq!(present, 1, "a boundary row was dropped");
    }
    assert!(
        spool::event_spool_status(&f.store, tight)
            .await
            .expect("status")
            .saturated
    );

    // A boundary event is refused too. Saturation is not selective — it is the
    // store declining new work, and admitting a boundary row here would put the
    // spool over the bound with nothing left to shed the next time.
    assert!(matches!(
        spool_with(&f, tight, account, boundary_event(f.session_id)).await,
        EventAdmission::Saturated { .. }
    ));
    assert_eq!(disposition_total(&f, "spool_saturated_dropped").await, 2);
}

/// Saturation clears as soon as delivery drains a row — with nobody clearing a
/// flag, because there is no flag.
#[tokio::test]
async fn saturation_clears_when_delivery_drains_the_spool() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let tight = SpoolCapacity {
        max_events: 2,
        max_bytes: i64::MAX,
    };
    let first = spooled(spool_with(&f, tight, account, session_opened(f.session_id)).await);
    spool_with(&f, tight, account, boundary_event(f.session_id)).await;
    assert!(matches!(
        spool_with(&f, tight, account, capture_event(f.session_id)).await,
        EventAdmission::Saturated { .. }
    ));

    spool::mark_event_delivered(&f.store, first.event_id)
        .await
        .expect("delivered");
    assert!(
        !spool::event_spool_status(&f.store, tight)
            .await
            .expect("status")
            .saturated,
        "delivery drained a row and saturation did not clear"
    );
    assert!(matches!(
        spool_with(&f, tight, account, capture_event(f.session_id)).await,
        EventAdmission::Spooled { .. }
    ));
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// A bound no test that is not about the bound will ever reach.
fn roomy() -> SpoolCapacity {
    SpoolCapacity {
        max_events: 10_000,
        max_bytes: i64::MAX,
    }
}

/// Unwrap an admission that is expected to have been queued.
fn queued(admission: spool::CommandAdmission) -> spool::SpooledCommand {
    match admission {
        spool::CommandAdmission::Spooled(c) => c,
        spool::CommandAdmission::Saturated { queued } => {
            panic!("the command spool refused at {queued} queued rows")
        }
    }
}

async fn spool_command(f: &Fixture, scope: CommandScope, account: Uuid, kind: CommandKind) -> Uuid {
    let payload = serde_json::json!({ "content": "an intent, and nothing derived" });
    queued(
        spool::spool_command(
            &f.store,
            NewCommand {
                scope,
                project_id: Some(f.project_id),
                account_id: account,
                kind,
                payload: &payload,
            },
            roomy(),
        )
        .await
        .expect("spool_command"),
    )
    .command_id
}

/// Commands drain in `command_seq` order within their scope.
#[tokio::test]
async fn commands_drain_in_sequence_order_within_their_scope() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let scope = CommandScope::Session(f.session_id);
    for _ in 0..3 {
        spool_command(&f, scope, account, CommandKind::Remember).await;
    }

    let claimed = spool::claim_commands(&f.store, account, 10)
        .await
        .expect("claim");
    assert_eq!(
        claimed.iter().map(|c| c.command_seq).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "commands did not drain in the order they were issued"
    );
    for c in &claimed {
        assert_eq!(
            c.command_id,
            eventid::command_id("session", &f.session_id.to_string(), c.command_seq)
        );
    }
}

/// A command in flight holds back everything behind it in its own scope.
///
/// Falsified by relying on `ORDER BY` alone: with the head of the scope
/// claimed, an ordered `LIMIT` would happily hand out the next row, and a
/// supersede would race its target to the server.
#[tokio::test]
async fn a_command_in_flight_blocks_the_rest_of_its_scope_until_it_settles() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let scope = CommandScope::Session(f.session_id);
    for _ in 0..3 {
        spool_command(&f, scope, account, CommandKind::Remember).await;
    }

    let head = spool::claim_commands(&f.store, account, 1)
        .await
        .expect("claim");
    assert_eq!(head.len(), 1);
    assert_eq!(head[0].command_seq, 1);

    assert!(
        spool::claim_commands(&f.store, account, 10)
            .await
            .expect("claim")
            .is_empty(),
        "the tail of a scope was claimed while its head was still in flight"
    );

    spool::mark_command_delivered(&f.store, head[0].command_id)
        .await
        .expect("delivered");
    let rest = spool::claim_commands(&f.store, account, 10)
        .await
        .expect("claim");
    assert_eq!(
        rest.iter().map(|c| c.command_seq).collect::<Vec<_>>(),
        vec![2, 3]
    );
}

/// A command waiting out a backoff holds its scope too — otherwise the retry
/// of a failed command would arrive after the command that followed it.
#[tokio::test]
async fn a_command_in_backoff_holds_its_scope() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let scope = CommandScope::Session(f.session_id);
    for _ in 0..2 {
        spool_command(&f, scope, account, CommandKind::Remember).await;
    }
    let head = spool::claim_commands(&f.store, account, 1)
        .await
        .expect("claim");
    spool::mark_command_failed(&f.store, head[0].command_id, "unreachable")
        .await
        .expect("failed");

    assert!(
        spool::claim_commands(&f.store, account, 10)
            .await
            .expect("claim")
            .is_empty(),
        "a command overtook the failed command it was queued behind"
    );
}

/// Two scopes are independent: one blocked head does not stall the other.
#[tokio::test]
async fn one_blocked_scope_does_not_stall_another() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let session_scope = CommandScope::Session(f.session_id);
    let store_scope = spool::store_scope(&f.store).await.expect("store scope");

    spool_command(&f, session_scope, account, CommandKind::Remember).await;
    spool_command(&f, session_scope, account, CommandKind::Supersede).await;
    spool_command(&f, store_scope, account, CommandKind::PersonalCreate).await;

    let head = spool::claim_commands(&f.store, account, 1)
        .await
        .expect("claim");
    assert_eq!(head.len(), 1);
    let blocked_scope = head[0].scope;

    let next = spool::claim_commands(&f.store, account, 10)
        .await
        .expect("claim");
    assert!(
        next.iter().all(|c| c.scope != blocked_scope),
        "a claim reached past a blocked scope's head"
    );
    assert!(
        !next.is_empty(),
        "the other scope was stalled by an unrelated scope's in-flight head"
    );
}

/// A sessionless command is a real command: store-scoped, with no session and
/// no invented one (`contracts/knowledge-commands.md` §4.1).
#[tokio::test]
async fn a_sessionless_command_is_scoped_to_the_store_and_names_no_session() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let scope = spool::store_scope(&f.store).await.expect("store scope");
    let writer = match scope {
        CommandScope::Store(w) => w,
        other => panic!("expected a store scope, got {other:?}"),
    };

    let payload = serde_json::json!({ "content": "remembered outside any session" });
    let command = queued(
        spool::spool_command(
            &f.store,
            NewCommand {
                scope,
                project_id: None,
                account_id: account,
                kind: CommandKind::PersonalCreate,
                payload: &payload,
            },
            roomy(),
        )
        .await
        .expect("spool_command"),
    );

    assert_eq!(command.session_id, None);
    assert_eq!(command.project_id, None);
    assert_eq!(
        command.command_id,
        eventid::command_id("store", &writer.to_string(), 1)
    );

    let (session_id, project_id): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT session_id, project_id FROM command_spool WHERE command_id = ?1")
            .bind(command.command_id.to_string())
            .fetch_one(f.store.pool())
            .await
            .expect("row");
    assert_eq!(session_id, None, "a session was invented for a CLI command");
    assert_eq!(project_id, None);

    // The store's own writer identity is stable, so a second sessionless
    // command continues the same run rather than starting a new one.
    let second = queued(
        spool::spool_command(
            &f.store,
            NewCommand {
                scope: spool::store_scope(&f.store).await.expect("store scope"),
                project_id: None,
                account_id: account,
                kind: CommandKind::PersonalForget,
                payload: &payload,
            },
            roomy(),
        )
        .await
        .expect("spool_command"),
    );
    assert_eq!(second.command_seq, 2);
}

/// Commands are account-bound exactly as events are.
#[tokio::test]
async fn a_command_claim_returns_only_the_signed_in_accounts_own_rows() {
    let f = fixture().await;
    let mine = Uuid::now_v7();
    let theirs = Uuid::now_v7();
    spool_command(
        &f,
        CommandScope::Session(f.session_id),
        theirs,
        CommandKind::Remember,
    )
    .await;

    assert!(
        spool::claim_commands(&f.store, mine, 10)
            .await
            .expect("claim")
            .is_empty(),
        "another account's command was claimed under my credential"
    );
    assert_eq!(
        spool::claim_commands(&f.store, theirs, 10)
            .await
            .expect("claim")
            .len(),
        1
    );
}

/// A refused command is never retried, and is still counted as a refusal.
#[tokio::test]
async fn a_refused_command_is_never_retried_and_does_not_wedge_its_scope() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let scope = CommandScope::Session(f.session_id);
    let first = spool_command(&f, scope, account, CommandKind::Remember).await;
    spool_command(&f, scope, account, CommandKind::Supersede).await;

    spool::claim_commands(&f.store, account, 1)
        .await
        .expect("claim");
    spool::mark_command_refused(&f.store, first, "not_permitted")
        .await
        .expect("refused");

    let status = spool::command_spool_status(&f.store).await.expect("status");
    assert_eq!(status.refused, 1);
    let next = spool::claim_commands(&f.store, account, 10)
        .await
        .expect("claim");
    assert_eq!(
        next.iter().map(|c| c.command_seq).collect::<Vec<_>>(),
        vec![2],
        "a refusal either wedged the scope or was itself retried"
    );
}

// ---------------------------------------------------------------------------
// T021 — the local replicas are a cache, and the server wins (FR-712a)
// ---------------------------------------------------------------------------

fn synced_personal(id: Uuid, owner: Uuid, content: &str) -> SyncedPersonalKnowledge {
    SyncedPersonalKnowledge {
        id,
        owner_user_id: owner,
        knowledge_type: MemoryType::Fact,
        content: content.to_string(),
        topic_key: Some("retry.budget".to_string()),
        value_key: Some("three".to_string()),
        applicability: Vec::new(),
        writer_id: Uuid::now_v7(),
        writer_seq: 1,
        created_at: Utc::now(),
        superseded_by_id: None,
        forgotten_at: None,
    }
}

/// A server-side content correction reaches the cached copy.
///
/// Falsified by the insert-once rule this replaced: with content written only
/// at first insert, the device would go on recalling the uncorrected text for
/// as long as the id survives.
#[tokio::test]
async fn a_server_side_correction_replaces_the_cached_personal_copy() {
    let f = fixture().await;
    let owner = Uuid::now_v7();
    let id = Uuid::now_v7();
    merge_synced_personal(&f.store, synced_personal(id, owner, "the original claim"))
        .await
        .expect("first");

    let mut corrected = synced_personal(id, owner, "the corrected claim");
    corrected.value_key = Some("four".to_string());
    corrected.applicability = vec![ApplicabilityFact {
        kind: cairn_core::domain::ApplicabilityKind::Language,
        value: "rust".to_string(),
    }];
    let merged = merge_synced_personal(&f.store, corrected)
        .await
        .expect("second");
    assert_eq!(merged.content, "the corrected claim");
    assert_eq!(merged.value_key.as_deref(), Some("four"));

    let facts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM personal_knowledge_applicability WHERE personal_id = ?1",
    )
    .bind(id.to_string())
    .fetch_one(f.store.pool())
    .await
    .expect("count");
    assert_eq!(facts, 1, "the refresh did not carry applicability");
}

/// The tombstone still wins over whatever content arrives with it.
#[tokio::test]
async fn a_tombstone_still_clears_content_under_server_wins() {
    let f = fixture().await;
    let owner = Uuid::now_v7();
    let id = Uuid::now_v7();
    merge_synced_personal(&f.store, synced_personal(id, owner, "here for now"))
        .await
        .expect("first");
    let mut tombstone = synced_personal(id, owner, "still here, apparently");
    tombstone.forgotten_at = Some(Utc::now());
    let merged = merge_synced_personal(&f.store, tombstone)
        .await
        .expect("tombstone");
    assert_eq!(merged.content, "");
    assert!(merged.forgotten_at.is_some());
}

/// A record the server has never sent is not touched by any refresh.
#[tokio::test]
async fn a_local_only_personal_record_survives_a_refresh_of_another_row() {
    let f = fixture().await;
    let owner = Uuid::now_v7();
    let local = create_personal(
        &f.store,
        NewPersonalKnowledge::direct(
            owner,
            MemoryType::Fact,
            "written here and never pushed",
            Some("local.only"),
            Some("kept"),
            Vec::new(),
        ),
        &[],
    )
    .await
    .expect("create_personal");

    let id = Uuid::now_v7();
    merge_synced_personal(&f.store, synced_personal(id, owner, "from the server"))
        .await
        .expect("first");
    merge_synced_personal(&f.store, synced_personal(id, owner, "corrected"))
        .await
        .expect("second");

    let still = get_personal(&f.store, local.record.id, owner)
        .await
        .expect("local record");
    assert_eq!(still.content, "written here and never pushed");
    assert!(
        still.origin_digest.is_none() || still.origin_digest == local.record.origin_digest,
        "a refresh disturbed a local record's own provenance"
    );
}

fn synced_team(id: Uuid, content: &str, state: TeamState) -> SyncedTeamKnowledge {
    SyncedTeamKnowledge {
        id,
        knowledge_type: MemoryType::Convention,
        content: content.to_string(),
        topic_key: Some("vcs.merge_style".to_string()),
        value_key: Some("squash".to_string()),
        applicability: Vec::new(),
        state,
        proposed_by_user_id: Uuid::now_v7(),
        ratified_by_user_id: if state == TeamState::Authoritative {
            Some(Uuid::now_v7())
        } else {
            None
        },
        ratified_at: if state == TeamState::Authoritative {
            Some(Utc::now())
        } else {
            None
        },
        writer_id: Uuid::now_v7(),
        writer_seq: 1,
        created_at: Utc::now(),
        superseded_by_id: None,
        retired_by_user_id: None,
        retired_at: None,
    }
}

/// The server's state wins even when it does not advance, and its content wins
/// too (FR-712a).
///
/// Falsified by the monotonic clamp this replaced, which silently discarded any
/// transition the local row had already passed — including a legitimate
/// reversal the server made.
#[tokio::test]
async fn a_team_state_that_did_not_advance_is_still_accepted_from_the_server() {
    let f = fixture().await;
    let instance = Uuid::now_v7();
    let id = Uuid::now_v7();
    let merged = merge_synced_team(
        &f.store,
        instance,
        synced_team(id, "we squash-merge", TeamState::Authoritative),
    )
    .await
    .expect("first");
    assert_eq!(merged.state, TeamState::Authoritative);

    let reversed = merge_synced_team(
        &f.store,
        instance,
        synced_team(
            id,
            "we squash-merge, with an exception",
            TeamState::Proposed,
        ),
    )
    .await
    .expect("second");
    assert_eq!(
        reversed.state,
        TeamState::Proposed,
        "the cache overruled the server about its own record's state"
    );
    assert_eq!(reversed.content, "we squash-merge, with an exception");
    assert_eq!(
        scalar(&f, "SELECT COUNT(*) FROM team_knowledge").await,
        1,
        "the refresh inserted a second copy instead of refreshing"
    );
}

// ---------------------------------------------------------------------------
// The state machine, stated as tests (FR-772, FR-784)
// ---------------------------------------------------------------------------

/// A transient failure is retryable, and this file says so deliberately.
///
/// `failed` means the **opposite** here of what it means in `outbox.rs`, where
/// it is the terminal state. The outbox has no `refused`, so its `failed` had
/// to carry the permanent meaning; schema v8 gives the spool a `refused` of its
/// own, which frees `failed` to mean "attempted, transient, waiting out its
/// backoff". Reading across from the outbox on the strength of the shared name
/// is the mistake this test exists to catch.
#[tokio::test]
async fn a_transient_failure_is_retryable_and_a_refusal_is_not() {
    let f = fixture().await;
    let account = Uuid::now_v7();

    let transient = spooled(spool(&f, account, capture_event(f.session_id)).await).event_id;
    let permanent = spooled(spool(&f, account, capture_event(f.session_id)).await).event_id;

    spool::claim_events(&f.store, account, 10)
        .await
        .expect("claim");
    spool::mark_event_failed(&f.store, transient, "connection_refused")
        .await
        .expect("mark failed");
    spool::mark_event_refused(&f.store, permanent, "repo_file_absolute")
        .await
        .expect("mark refused");

    // The transient row is claimable again once its backoff elapses.
    expire_backoff(&f, transient).await;
    let again = spool::claim_events(&f.store, account, 10)
        .await
        .expect("claim");
    let ids: Vec<Uuid> = again.iter().map(|e| e.event_id).collect();
    assert!(
        ids.contains(&transient),
        "a transient failure was not retried"
    );
    assert!(
        !ids.contains(&permanent),
        "a permanent refusal was retried; no claim predicate may admit `refused`"
    );
}

/// Retry is bounded, not merely backed off (FR-784).
///
/// FR-784 joins two clauses with "and": retry "MUST be bounded" **and** "MUST
/// back off", and a permanently undeliverable event "MUST become visible rather
/// than being retried forever". A row retrying every five minutes indefinitely
/// has bounded delay and unbounded retry, and satisfies only one of them.
#[tokio::test]
async fn retry_stops_at_the_bound_and_the_row_becomes_visible() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let id = spooled(spool(&f, account, capture_event(f.session_id)).await).event_id;

    // One attempt short of the bound: still retrying.
    set_attempts(&f, id, spool::MAX_DELIVERY_ATTEMPTS - 1).await;
    spool::mark_event_failed(&f.store, id, "connection_refused")
        .await
        .expect("mark failed");
    assert_eq!(state_of(&f, id).await, "failed");

    // At the bound: terminal, and visible rather than deleted.
    set_attempts(&f, id, spool::MAX_DELIVERY_ATTEMPTS).await;
    spool::mark_event_failed(&f.store, id, "connection_refused")
        .await
        .expect("mark failed");
    assert_eq!(
        state_of(&f, id).await,
        "refused",
        "a row went on being retried past its retry window"
    );
    assert_eq!(
        scalar(
            &f,
            &format!("SELECT COUNT(*) FROM event_spool WHERE event_id = '{id}'")
        )
        .await,
        1,
        "an undeliverable row was deleted instead of surfaced"
    );

    // And it never becomes claimable again.
    expire_backoff(&f, id).await;
    let claimed = spool::claim_events(&f.store, account, 10)
        .await
        .expect("claim");
    assert!(claimed.is_empty(), "an exhausted row was claimed again");
}

/// The cause survives, even though the state does not distinguish it.
///
/// Exhaustion and refusal are both `refused` because schema v8 has five states
/// and the *consequence* is identical. The cause is not, and a person looking
/// at a stuck spool needs to know whether the server said no or was never
/// reachable.
#[tokio::test]
async fn an_exhausted_row_is_distinguishable_from_a_server_refusal() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let exhausted = spooled(spool(&f, account, capture_event(f.session_id)).await).event_id;
    let refused = spooled(spool(&f, account, capture_event(f.session_id)).await).event_id;

    set_attempts(&f, exhausted, spool::MAX_DELIVERY_ATTEMPTS).await;
    spool::mark_event_failed(&f.store, exhausted, "connection_refused")
        .await
        .expect("mark failed");
    spool::mark_event_refused(&f.store, refused, "repo_file_absolute")
        .await
        .expect("mark refused");

    let breakdown = spool::event_spool_breakdown(&f.store, SpoolCapacity::default())
        .await
        .expect("breakdown");
    assert_eq!(breakdown.terminal, 2);
    assert_eq!(
        breakdown.terminal_retry_exhausted, 1,
        "the two terminal causes were reported as one"
    );
}

/// A `duplicate` answer is a success, because it is what the retry was for.
#[tokio::test]
async fn a_duplicate_answer_settles_the_row_as_delivered() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let id = spooled(spool(&f, account, capture_event(f.session_id)).await).event_id;
    spool::claim_events(&f.store, account, 10)
        .await
        .expect("claim");
    // The drainer maps `accepted` and `duplicate` alike: at most one canonical
    // event exists either way (FR-770, FR-786).
    spool::mark_event_delivered(&f.store, id)
        .await
        .expect("delivered");
    assert_eq!(state_of(&f, id).await, "delivered");

    let breakdown = spool::event_spool_breakdown(&f.store, SpoolCapacity::default())
        .await
        .expect("breakdown");
    assert_eq!(
        breakdown.waiting + breakdown.retrying + breakdown.in_flight,
        0
    );
    assert_eq!(breakdown.terminal, 0, "a success was reported as terminal");
}

/// Health tells waiting, retrying, in flight and terminal apart.
///
/// One "undelivered" number reports four different situations as one problem. A
/// spool of waiting rows on a laptop that just came online is healthy; the same
/// count of exhausted rows is not.
#[tokio::test]
async fn health_distinguishes_waiting_retrying_in_flight_and_terminal() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    // Four rows, driven into the four conditions.
    for _ in 0..4 {
        spooled(spool(&f, account, capture_event(f.session_id)).await);
    }
    let claimed = spool::claim_events(&f.store, account, 10)
        .await
        .expect("claim");
    assert_eq!(claimed.len(), 4);

    spool::mark_event_failed(&f.store, claimed[0].event_id, "connection_refused")
        .await
        .expect("failed");
    spool::mark_event_refused(&f.store, claimed[1].event_id, "repo_file_absolute")
        .await
        .expect("refused");
    // The remaining two are still claimed. Releasing every claim and taking one
    // back leaves exactly one waiting and one in flight.
    spool::release_event_claims(&f.store)
        .await
        .expect("release");
    let reclaimed = spool::claim_events(&f.store, account, 1)
        .await
        .expect("reclaim");
    assert_eq!(reclaimed.len(), 1);

    let b = spool::event_spool_breakdown(&f.store, SpoolCapacity::default())
        .await
        .expect("breakdown");
    assert_eq!(b.waiting, 1, "{b:?}");
    assert_eq!(b.in_flight, 1, "{b:?}");
    assert_eq!(b.retrying, 1, "{b:?}");
    assert_eq!(b.terminal, 1, "{b:?}");
    assert!(!b.saturated);
}

// ---------------------------------------------------------------------------
// Command-spool capacity (FR-785 ∧ knowledge-commands.md §4)
// ---------------------------------------------------------------------------

/// A full command spool refuses new commands and drops none.
///
/// The event spool sheds its oldest capture-class rows. That is inexpressible
/// for commands — `command_spool` has no `boundary_class`, because no command
/// is droppable — so the shedding half of FR-785 has nothing to operate on and
/// the saturation half is the whole policy.
#[tokio::test]
async fn a_full_command_spool_refuses_rather_than_discarding_earlier_intent() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let tiny = SpoolCapacity {
        max_events: 2,
        max_bytes: i64::MAX,
    };
    let payload = serde_json::json!({ "content": "an explicit instruction" });
    let queue = |kind| {
        spool::spool_command(
            &f.store,
            NewCommand {
                scope: CommandScope::Session(f.session_id),
                project_id: Some(f.project_id),
                account_id: account,
                kind,
                payload: &payload,
            },
            tiny,
        )
    };

    let first = queued(queue(CommandKind::Remember).await.expect("first"));
    let second = queued(queue(CommandKind::Supersede).await.expect("second"));

    match queue(CommandKind::Forget).await.expect("third") {
        spool::CommandAdmission::Saturated { queued } => assert_eq!(queued, 2),
        spool::CommandAdmission::Spooled(_) => {
            panic!("the command spool accepted work past its bound")
        }
    }

    // Nothing earlier was discarded to make room. That is the whole difference
    // from the event policy: a command is something a person asked for.
    assert_eq!(
        scalar(&f, "SELECT COUNT(*) FROM command_spool").await,
        2,
        "an explicit command was dropped to admit a newer one"
    );
    for id in [first.command_id, second.command_id] {
        assert_eq!(
            scalar(
                &f,
                &format!("SELECT COUNT(*) FROM command_spool WHERE command_id = '{id}'")
            )
            .await,
            1
        );
    }
}

/// A refused command consumes no ordinal, so the scope's sequence has no gap.
#[tokio::test]
async fn a_refused_command_leaves_the_scope_sequence_intact() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let tiny = SpoolCapacity {
        max_events: 1,
        max_bytes: i64::MAX,
    };
    let payload = serde_json::json!({ "content": "an explicit instruction" });
    let queue = |kind, capacity| {
        spool::spool_command(
            &f.store,
            NewCommand {
                scope: CommandScope::Session(f.session_id),
                project_id: Some(f.project_id),
                account_id: account,
                kind,
                payload: &payload,
            },
            capacity,
        )
    };

    let first = queued(queue(CommandKind::Remember, tiny).await.expect("first"));
    assert_eq!(first.command_seq, 1);

    // Refused: the ordinal is checked before it is allocated, so nothing is
    // consumed and no later reader has to wonder about a missing number.
    assert!(matches!(
        queue(CommandKind::Supersede, tiny).await.expect("second"),
        spool::CommandAdmission::Saturated { .. }
    ));

    // Room again, and the next command is 2 rather than 3.
    let next = queued(queue(CommandKind::Supersede, roomy()).await.expect("third"));
    assert_eq!(
        next.command_seq, 2,
        "a refused command burned an ordinal and left a gap in the scope"
    );
}

/// Saturation is visible in health, not merely returned to one caller.
#[tokio::test]
async fn a_saturated_command_spool_is_reported_as_saturated() {
    let f = fixture().await;
    let account = Uuid::now_v7();
    let one = SpoolCapacity {
        max_events: 1,
        max_bytes: i64::MAX,
    };
    let payload = serde_json::json!({ "content": "an explicit instruction" });
    queued(
        spool::spool_command(
            &f.store,
            NewCommand {
                scope: CommandScope::Session(f.session_id),
                project_id: Some(f.project_id),
                account_id: account,
                kind: CommandKind::Remember,
                payload: &payload,
            },
            one,
        )
        .await
        .expect("first"),
    );

    let b = spool::command_spool_breakdown(&f.store, one)
        .await
        .expect("breakdown");
    assert!(
        b.saturated,
        "a full command spool did not report saturation"
    );
    assert_eq!(b.waiting, 1);

    // And it clears when the queue drains, with no flag to reset.
    let b_roomy = spool::command_spool_breakdown(&f.store, roomy())
        .await
        .expect("breakdown");
    assert!(!b_roomy.saturated);
}
