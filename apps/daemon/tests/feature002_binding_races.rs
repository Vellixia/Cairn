//! T134: barrier-controlled binding/lifecycle race coverage.
//!
//! Both required scenarios bind an existing session while a watcher stage is
//! deterministically parked: once while watcher reconciliation is paused, and once
//! while recovery/reattach is paused. Coordination uses `WatcherTestControls`
//! barriers only — never a correctness sleep.

mod support;

use cairn_protocol::*;
use cairn_storage_local::{events, open_pool_at, session_bindings, sessions};
use fixtures_repositories::FixtureRepo;
use sqlx::SqlitePool;
use support::binding::BindingFixture;
use support::TestDaemon;

/// Registry method discriminator for binding, as stored by `cairn-session`
/// (the storage-level name, not the transport method constant).
const REGISTRY_METHOD_BIND: &str = "session.bind";

/// Rows in the global operation-idempotency registry for one method.
async fn registry_rows(pool: &SqlitePool, method: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM operation_idempotency WHERE method=?")
        .bind(method)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Aggregate sequences that skip a value — a partial or duplicated append.
async fn aggregate_sequence_gaps(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM (SELECT aggregate_seq, LAG(aggregate_seq) OVER(PARTITION BY aggregate_type,aggregate_id ORDER BY aggregate_seq) AS previous FROM events WHERE aggregate_seq IS NOT NULL) WHERE previous IS NOT NULL AND aggregate_seq<>previous+1",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// `event_aggregate_heads` state for one aggregate, paired with what `events`
/// actually committed for the same aggregate. `allocate_aggregate_seq` bumps
/// `last_seq` through `ON CONFLICT DO UPDATE`, so a head can outrun its events if a
/// transaction allocates and then rolls back — these fields make that visible.
struct AggregateHead {
    /// Rows in `event_aggregate_heads` for the aggregate; exactly 1 is healthy.
    rows: i64,
    /// `event_aggregate_heads.last_seq`.
    last_seq: i64,
    /// `MAX(events.aggregate_seq)` for the same aggregate.
    max_committed_seq: i64,
    /// Committed scoped events for the aggregate; equals `last_seq` when the
    /// sequence is contiguous with no gap and no duplicate.
    committed_events: i64,
}

async fn aggregate_head(
    pool: &SqlitePool,
    aggregate_type: &str,
    aggregate_id: &str,
) -> AggregateHead {
    let rows = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM event_aggregate_heads WHERE aggregate_type=? AND aggregate_id=?",
    )
    .bind(aggregate_type)
    .bind(aggregate_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let last_seq = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(last_seq), 0) FROM event_aggregate_heads WHERE aggregate_type=? AND aggregate_id=?",
    )
    .bind(aggregate_type)
    .bind(aggregate_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let max_committed_seq = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(aggregate_seq), 0) FROM events WHERE aggregate_type=? AND aggregate_id=? AND aggregate_seq IS NOT NULL",
    )
    .bind(aggregate_type)
    .bind(aggregate_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let committed_events = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM events WHERE aggregate_type=? AND aggregate_id=? AND aggregate_seq IS NOT NULL",
    )
    .bind(aggregate_type)
    .bind(aggregate_id)
    .fetch_one(pool)
    .await
    .unwrap();
    AggregateHead {
        rows,
        last_seq,
        max_committed_seq,
        committed_events,
    }
}

/// Assert the session aggregate head is single, exact, and contiguous.
fn assert_head_tracks_committed_events(head: &AggregateHead, label: &str) {
    assert_eq!(
        head.rows, 1,
        "{label}: exactly one aggregate-head row must exist for the session aggregate"
    );
    assert!(
        head.last_seq <= head.max_committed_seq,
        "{label}: aggregate head is over-advanced ({} > {}) — a rolled-back allocation survived",
        head.last_seq,
        head.max_committed_seq
    );
    assert!(
        head.last_seq >= head.max_committed_seq,
        "{label}: aggregate head is behind committed events ({} < {})",
        head.last_seq,
        head.max_committed_seq
    );
    assert_eq!(
        head.committed_events, head.last_seq,
        "{label}: session aggregate sequence must be contiguous 1..=last_seq with no gap or duplicate"
    );
    assert!(
        head.last_seq >= 1,
        "{label}: the binding must have advanced the session aggregate at least once"
    );
}

/// Aggregates anywhere in the ledger whose head row is missing, duplicated, or
/// disagreeing with committed events, plus orphan heads with no events at all.
/// Zero is the only healthy value.
async fn inconsistent_aggregate_heads(pool: &SqlitePool) -> i64 {
    let disagreeing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM (SELECT e.aggregate_type AS t, e.aggregate_id AS i, MAX(e.aggregate_seq) AS max_seq, (SELECT COUNT(*) FROM event_aggregate_heads h WHERE h.aggregate_type=e.aggregate_type AND h.aggregate_id=e.aggregate_id) AS head_rows, (SELECT MAX(h.last_seq) FROM event_aggregate_heads h WHERE h.aggregate_type=e.aggregate_type AND h.aggregate_id=e.aggregate_id) AS head_seq FROM events e WHERE e.aggregate_seq IS NOT NULL GROUP BY e.aggregate_type, e.aggregate_id) WHERE head_rows<>1 OR head_seq IS NULL OR head_seq<>max_seq",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let orphaned = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM event_aggregate_heads h WHERE NOT EXISTS (SELECT 1 FROM events e WHERE e.aggregate_type=h.aggregate_type AND e.aggregate_id=h.aggregate_id AND e.aggregate_seq IS NOT NULL)",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    disagreeing + orphaned
}

/// Global sequence of the single `session.bound` event for `session_id`.
async fn bound_event_seq(pool: &SqlitePool, session_id: &str) -> i64 {
    let bound: Vec<_> = events::list_events(pool, None, None, Some(session_id), None, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == "session.bound")
        .collect();
    assert_eq!(bound.len(), 1, "exactly one session.bound event");
    bound[0].seq
}

#[tokio::test(flavor = "multi_thread")]
async fn bind_during_paused_reattach_reconciliation_survives_watcher_failure() {
    let daemon = TestDaemon::start().await;
    let fixture = BindingFixture::create(&daemon).await;
    let (dir, config) = daemon.stop().await;
    let daemon = TestDaemon::start_with(dir, config).await;
    let pool = open_pool_at(&daemon.db_path()).await.unwrap();
    assert_eq!(
        sessions::get_by_id(&pool, &fixture.session_id.to_string())
            .await
            .unwrap()
            .unwrap()
            .state,
        "recovering"
    );

    let controls = daemon.watcher_controls();
    controls.pause_before_reconcile();
    let mut client = daemon.client().await;
    let reattach_params = SessionReattachParams {
        session_id: fixture.session_id,
        agent_instance_id: fixture.agent_instance_id,
        resume_token: fixture.resume_token.clone(),
    };
    let reattach = tokio::spawn(async move {
        client
            .call(methods::SESSION_REATTACH, &reattach_params)
            .await
    });
    controls.wait_before_reconcile().await;
    assert_eq!(
        sessions::get_by_id(&pool, &fixture.session_id.to_string())
            .await
            .unwrap()
            .unwrap()
            .state,
        "active",
        "only reattach, not binding, performs the recovering-to-active transition"
    );

    let bound: SessionBindResult = serde_json::from_value(
        daemon
            .call(
                methods::SESSION_BIND,
                &fixture.bind_params(IdempotencyKey::new_v7()),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(bound.created);
    assert_eq!(
        sessions::get_by_id(&pool, &fixture.session_id.to_string())
            .await
            .unwrap()
            .unwrap()
            .state,
        "active",
        "binding must not perform a lifecycle transition"
    );
    controls.force_reconcile_failure();
    controls.release_reconcile();
    let error = reattach.await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::WatcherStartFailed);

    let final_session = sessions::get_by_id(&pool, &fixture.session_id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(final_session.state, "interrupted");
    assert_eq!(final_session.binding_mode, "project_bound");
    assert!(
        session_bindings::get(&pool, &fixture.session_id.to_string())
            .await
            .unwrap()
            .is_some()
    );
    let relevant: Vec<_> = events::list_events(
        &pool,
        None,
        None,
        Some(&fixture.session_id.to_string()),
        None,
        100,
    )
    .await
    .unwrap()
    .into_iter()
    .filter(|event| {
        matches!(
            event.event_type.as_str(),
            "session.recovered" | "session.bound" | "session.interrupted"
        )
    })
    .map(|event| event.event_type)
    .collect();
    assert_eq!(
        relevant,
        vec!["session.recovered", "session.bound", "session.interrupted"]
    );
    let (binding_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM session_bindings WHERE session_id=?")
            .bind(fixture.session_id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(binding_count, 1);
    assert_eq!(
        registry_rows(&pool, REGISTRY_METHOD_BIND).await,
        1,
        "the race must leave exactly one binding registry record"
    );
    assert_eq!(
        aggregate_sequence_gaps(&pool).await,
        0,
        "no partial aggregate sequence survived the race"
    );
    let head = aggregate_head(&pool, "session", &fixture.session_id.to_string()).await;
    assert_head_tracks_committed_events(&head, "paused reattach");
    assert_eq!(
        inconsistent_aggregate_heads(&pool).await,
        0,
        "no duplicate or partial aggregate-head state may survive the race"
    );
    pool.close().await;
    drop(fixture);
    daemon.stop().await;
}

/// The second required T134 scenario: bind an existing active session while a
/// watcher installation/reconciliation for another worktree is parked at its
/// barrier. Binding and watcher lifecycle are independent dimensions, so the bind
/// must commit exactly once, in deterministic global order, without touching the
/// lifecycle state of either session.
#[tokio::test(flavor = "multi_thread")]
async fn bind_while_watcher_reconciliation_is_paused_keeps_lifecycle_and_binding_independent() {
    let daemon = TestDaemon::start().await;
    let fixture = BindingFixture::create(&daemon).await;
    let pool = open_pool_at(&daemon.db_path()).await.unwrap();
    assert_eq!(
        sessions::get_by_id(&pool, &fixture.session_id.to_string())
            .await
            .unwrap()
            .unwrap()
            .state,
        "active"
    );

    // A second, unassociated repository is eligible for an explicit bootstrap
    // start, and its fresh worktree drives a real watcher install + reconcile.
    let other_repository = FixtureRepo::new().unwrap();
    daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &RegisterParams {
                path: other_repository.root().to_string_lossy().to_string(),
            },
        )
        .await
        .unwrap();

    let controls = daemon.watcher_controls();
    controls.pause_before_reconcile();
    let mut client = daemon.client().await;
    let bootstrap_agent = AgentInstanceId(uuid::Uuid::now_v7());
    let start_params = SessionStartParams {
        path: Some(other_repository.root().to_string_lossy().to_string()),
        repository_id: None,
        agent_type: "watcher-race-bootstrap".into(),
        agent_instance_id: bootstrap_agent,
        agent_pid: None,
        scope: Some(SessionScopeDto::LocalUnbound),
    };
    let start =
        tokio::spawn(async move { client.call(methods::SESSION_START, &start_params).await });

    // Barrier: reconciliation for the second worktree is now parked.
    controls.wait_before_reconcile().await;
    assert!(
        !start.is_finished(),
        "reconciliation is a success boundary, so the start must still be in flight"
    );

    // Bind the first session while that reconciliation is held.
    let bound: SessionBindResult = serde_json::from_value(
        daemon
            .call(
                methods::SESSION_BIND,
                &fixture.bind_params(IdempotencyKey::new_v7()),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(bound.created);
    assert_eq!(bound.session_id, fixture.session_id);
    assert_eq!(
        sessions::get_by_id(&pool, &fixture.session_id.to_string())
            .await
            .unwrap()
            .unwrap()
            .state,
        "active",
        "binding must not perform a lifecycle transition"
    );
    let bound_seq = bound_event_seq(&pool, &fixture.session_id.to_string()).await;
    assert!(
        !start.is_finished(),
        "the binding committed while reconciliation was still parked"
    );

    controls.release_reconcile();
    let started: SessionStartResult =
        serde_json::from_value(start.await.unwrap().unwrap()).unwrap();
    assert_eq!(
        started.session.scope,
        SessionScopeDto::LocalUnbound,
        "a concurrent lifecycle start never inherits another session's binding"
    );

    // Final state: independent dimensions, one binding, deterministic order.
    let first = sessions::get_by_id(&pool, &fixture.session_id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.state, "active");
    assert_eq!(first.binding_mode, "project_bound");
    let second = sessions::get_by_id(&pool, &started.session.session_id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.state, "active");
    assert_eq!(
        second.binding_mode, "local_unbound",
        "watcher lifecycle events never implicitly change binding mode"
    );
    let binding = session_bindings::get(&pool, &fixture.session_id.to_string())
        .await
        .unwrap()
        .expect("committed binding");
    assert_eq!(binding.task_revision_id, fixture.revision_id.to_string());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM session_bindings")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1,
        "exactly one binding projection exists"
    );
    assert_eq!(registry_rows(&pool, REGISTRY_METHOD_BIND).await, 1);
    assert_eq!(aggregate_sequence_gaps(&pool).await, 0);
    let head = aggregate_head(&pool, "session", &fixture.session_id.to_string()).await;
    assert_head_tracks_committed_events(&head, "paused watcher reconciliation");
    let concurrent_head =
        aggregate_head(&pool, "session", &started.session.session_id.to_string()).await;
    assert!(
        concurrent_head.last_seq <= concurrent_head.max_committed_seq,
        "the concurrently started session's head is over-advanced"
    );
    assert!(
        concurrent_head.last_seq >= concurrent_head.max_committed_seq,
        "the concurrently started session's head is behind its committed events"
    );
    assert_eq!(
        inconsistent_aggregate_heads(&pool).await,
        0,
        "no duplicate or partial aggregate-head state may survive the race"
    );

    let second_started_seq = events::list_events(
        &pool,
        None,
        None,
        Some(&started.session.session_id.to_string()),
        None,
        100,
    )
    .await
    .unwrap()
    .into_iter()
    .find(|event| event.event_type == "session.started")
    .expect("second session.started")
    .seq;
    // The start transaction commits before the watcher install/reconcile barriers,
    // so `session.started` is already in the ledger when the bind lands. The global
    // sequence must therefore record started-then-bound, matching real commit order
    // rather than the order in which the two requests were issued.
    assert!(
        second_started_seq < bound_seq,
        "the global ledger order must match the observed commit order"
    );
    assert_eq!(
        events::list_events(&pool, None, None, None, None, 1_000)
            .await
            .unwrap()
            .windows(2)
            .filter(|pair| pair[0].seq >= pair[1].seq)
            .count(),
        0,
        "the global sequence must remain strictly increasing across the race"
    );

    pool.close().await;
    drop(other_repository);
    drop(fixture);
    daemon.stop().await;
}
