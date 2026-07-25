//! T017: projections rebuilt from the event log equal live projections
//! (constitution: event replay).

use cairn_events::{replay, EventBuilder, SessionStartedPayload, StateChangedPayload};
use cairn_storage_local::{events as ev, WorktreeWriters};
use sqlx::SqlitePool;

async fn test_pool() -> (tempfile::TempDir, SqlitePool) {
    let dir = tempfile::TempDir::new().unwrap();
    let pool = cairn_storage_local::open_pool_at(&dir.path().join("cairn.db"))
        .await
        .unwrap();
    seed(&pool).await;
    (dir, pool)
}

async fn seed(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO repositories (id, repo_uuid, canonical_path, registered_at) \
         VALUES ('r1','ru1','/tmp/x','t')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO worktrees (id, repository_id, worktree_uuid, path, is_main, registered_at) \
         VALUES ('w1','r1','wu1','/tmp/x',1,'t')",
    )
    .execute(pool)
    .await
    .unwrap();
    for id in ["s1", "s2", "s3"] {
        sqlx::query(
            "INSERT INTO snapshots (id, worktree_id, branch, head_commit, staged_fp, unstaged_fp, \
             untracked_fp, snapshot_fp, fp_schema_version, created_at) \
             VALUES (?, 'w1', 'main', 'h', 'a', 'b', 'c', ?, 1, 't')",
        )
        .bind(id)
        .bind(format!("fp-{id}"))
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn append(pool: &SqlitePool, writers: &WorktreeWriters, e: cairn_storage_local::NewEvent) {
    ev::append_with_projection(
        pool,
        writers,
        "w1",
        e,
        Box::new(|_| Box::pin(async { Ok(()) })),
    )
    .await
    .unwrap();
}

fn insert_session_row(
    pool: &SqlitePool,
    id: &str,
    state: &str,
    current: &str,
) -> impl std::future::Future<Output = ()> {
    let pool = pool.clone();
    let (id, state, current) = (id.to_string(), state.to_string(), current.to_string());
    async move {
        sqlx::query(
            "INSERT INTO sessions (id, repository_id, worktree_id, local_user, agent_type, \
             agent_instance_id, agent_pid, resume_token_hash, lease_expires_at, state, \
             start_snapshot_id, current_snapshot_id, started_at, ended_at, last_heartbeat_at, \
             recovering_since) \
             VALUES (?, 'r1', 'w1', 'u', 'agent', ?, NULL, 'h', 't', ?, 's1', ?, 't', NULL, 't', NULL)",
        )
        .bind(&id)
        .bind(format!("inst-{id}"))
        .bind(&state)
        .bind(&current)
        .execute(&pool)
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn replayed_projections_match_live_projections() {
    let (_dir, pool) = test_pool().await;
    let writers = WorktreeWriters::new();

    // Live projection rows (what the daemon would maintain).
    insert_session_row(&pool, "sess-a", "stopped", "s2").await;
    insert_session_row(&pool, "sess-b", "active", "s3").await;

    // Event history telling the same story.
    let payload_a = SessionStartedPayload {
        agent_type: "agent".into(),
        agent_instance_id: "inst-sess-a".into(),
        start_snapshot_id: "s1".into(),
        local_user: "u".into(),
    };
    append(
        &pool,
        &writers,
        EventBuilder::session_started("r1", "w1", "sess-a", &payload_a),
    )
    .await;

    let payload_b = SessionStartedPayload {
        agent_type: "agent".into(),
        agent_instance_id: "inst-sess-b".into(),
        start_snapshot_id: "s1".into(),
        local_user: "u".into(),
    };
    append(
        &pool,
        &writers,
        EventBuilder::session_started("r1", "w1", "sess-b", &payload_b),
    )
    .await;

    // Worktree state change to s3 — applies to live sessions only.
    append(
        &pool,
        &writers,
        EventBuilder::repository_state_changed(
            "r1",
            "w1",
            &StateChangedPayload {
                worktree_id: "w1".into(),
                from_snapshot_id: Some("s1".into()),
                to_snapshot_id: "s3".into(),
            },
        ),
    )
    .await;

    // sess-a stopped with final snapshot s2.
    append(
        &pool,
        &writers,
        EventBuilder::session_stopped("r1", "w1", "sess-a", "s2"),
    )
    .await;

    let rebuilt = replay::rebuild_sessions(&pool).await.unwrap();
    let live = replay::live_sessions(&pool).await.unwrap();

    assert_eq!(rebuilt.len(), live.len());
    for (id, live_proj) in &live {
        let re = rebuilt
            .get(id)
            .unwrap_or_else(|| panic!("missing rebuilt session {id}"));
        assert_eq!(re, live_proj, "replayed projection diverged for {id}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn interrupted_and_recovered_replay_correctly() {
    let (_dir, pool) = test_pool().await;
    let writers = WorktreeWriters::new();

    insert_session_row(&pool, "sess-x", "interrupted", "s1").await;
    insert_session_row(&pool, "sess-y", "active", "s2").await;
    insert_session_row(&pool, "sess-z", "interrupted", "s1").await;

    let mk = |sid: &str| SessionStartedPayload {
        agent_type: "agent".into(),
        agent_instance_id: format!("inst-sess-{sid}"),
        start_snapshot_id: "s1".into(),
        local_user: "u".into(),
    };
    append(
        &pool,
        &writers,
        EventBuilder::session_started("r1", "w1", "sess-x", &mk("x")),
    )
    .await;
    append(
        &pool,
        &writers,
        EventBuilder::session_started("r1", "w1", "sess-y", &mk("y")),
    )
    .await;
    append(
        &pool,
        &writers,
        EventBuilder::session_started("r1", "w1", "sess-z", &mk("z")),
    )
    .await;
    append(
        &pool,
        &writers,
        EventBuilder::session_watcher_start_failed(
            "r1",
            "w1",
            "sess-x",
            cairn_domain::WatcherStartStage::Install,
        ),
    )
    .await;
    let reconcile_failure = EventBuilder::session_watcher_start_failed(
        "r1",
        "w1",
        "sess-z",
        cairn_domain::WatcherStartStage::Reconcile,
    );
    assert_eq!(
        reconcile_failure.payload,
        serde_json::json!({
            "reason": "watcher_start_failed",
            "watcher_stage": "reconcile",
        }),
        "watcher interruption evidence is bounded to stable replay fields"
    );
    append(&pool, &writers, reconcile_failure).await;
    append(
        &pool,
        &writers,
        EventBuilder::session_recovered("r1", "w1", "sess-y", "s2"),
    )
    .await;

    let rebuilt = replay::rebuild_sessions(&pool).await.unwrap();
    let live = replay::live_sessions(&pool).await.unwrap();
    assert_eq!(rebuilt, live);
}
