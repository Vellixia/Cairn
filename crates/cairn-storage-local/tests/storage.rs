//! T015: storage guarantees — migrations, append-only triggers, idempotent
//! append (dedup returns prior result, projection skipped), atomicity,
//! serialized concurrent appends, live-session uniqueness.

use cairn_storage_local::{events as ev, NewEvent, WorktreeWriters};
use sqlx::SqlitePool;

async fn test_pool() -> (tempfile::TempDir, SqlitePool) {
    let dir = tempfile::TempDir::new().unwrap();
    let pool = cairn_storage_local::open_pool_at(&dir.path().join("cairn.db"))
        .await
        .unwrap();
    (dir, pool)
}

fn event(key: &str) -> NewEvent {
    NewEvent {
        id: uuid::Uuid::now_v7().to_string(),
        idempotency_key: key.to_string(),
        event_type: "repository.registered".into(),
        repository_id: None,
        worktree_id: None,
        session_id: None,
        snapshot_id: None,
        payload: serde_json::json!({"k": key}),
        recorded_at: "2026-07-16T00:00:00.000Z".into(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn migrates_from_empty_and_seeds_meta() {
    let (_dir, pool) = test_pool().await;
    let (v,): (String,) = sqlx::query_as("SELECT value FROM meta WHERE key = 'fp_schema_version'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(v, "1");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_reject_update_and_delete() {
    let (_dir, pool) = test_pool().await;
    let writers = WorktreeWriters::new();
    ev::append_with_projection(
        &pool,
        &writers,
        "w1",
        event("k1"),
        Box::new(|_| Box::pin(async { Ok(()) })),
    )
    .await
    .unwrap();

    let upd = sqlx::query("UPDATE events SET event_type = 'x' WHERE idempotency_key = 'k1'")
        .execute(&pool)
        .await;
    assert!(upd.is_err(), "UPDATE on events must be rejected by trigger");
    let del = sqlx::query("DELETE FROM events WHERE idempotency_key = 'k1'")
        .execute(&pool)
        .await;
    assert!(del.is_err(), "DELETE on events must be rejected by trigger");
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshots_reject_update_and_delete() {
    let (_dir, pool) = test_pool().await;
    seed_repo_worktree(&pool).await;
    sqlx::query(
        "INSERT INTO snapshots (id, worktree_id, branch, head_commit, staged_fp, unstaged_fp, \
         untracked_fp, snapshot_fp, fp_schema_version, created_at) \
         VALUES ('s1','w1','main','h','a','b','c','fp',1,'t')",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(sqlx::query("UPDATE snapshots SET branch='x' WHERE id='s1'")
        .execute(&pool)
        .await
        .is_err());
    assert!(sqlx::query("DELETE FROM snapshots WHERE id='s1'")
        .execute(&pool)
        .await
        .is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_idempotency_key_dedupes_and_skips_projection() {
    let (_dir, pool) = test_pool().await;
    let writers = WorktreeWriters::new();

    sqlx::query("CREATE TABLE probe (n INTEGER)")
        .execute(&pool)
        .await
        .unwrap();

    let projection = || -> ev::TxnFn<()> {
        Box::new(|conn| {
            Box::pin(async move {
                sqlx::query("INSERT INTO probe (n) VALUES (1)")
                    .execute(&mut *conn)
                    .await
                    .map_err(cairn_storage_local::StorageError::from)?;
                Ok(())
            })
        })
    };

    let first = ev::append_with_projection(&pool, &writers, "w1", event("dup"), projection())
        .await
        .unwrap();
    let second = ev::append_with_projection(&pool, &writers, "w1", event("dup"), projection())
        .await
        .unwrap();

    assert!(!first.deduplicated);
    assert!(second.deduplicated);
    assert_eq!(
        first.seq, second.seq,
        "dedup returns the prior event's result"
    );

    let (rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events WHERE idempotency_key='dup'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    let (probes,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM probe")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(probes, 1, "projection must not re-run on dedup");
}

#[tokio::test(flavor = "multi_thread")]
async fn projection_failure_rolls_back_event_append() {
    let (_dir, pool) = test_pool().await;
    let writers = WorktreeWriters::new();
    let result = ev::append_with_projection(
        &pool,
        &writers,
        "w1",
        event("rollback"),
        Box::new(|_| {
            Box::pin(async {
                Err(cairn_storage_local::StorageError::Conflict(
                    "injected".into(),
                ))
            })
        }),
    )
    .await;
    assert!(result.is_err());
    let (rows,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM events WHERE idempotency_key='rollback'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 0, "event append must roll back with its projection");
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_appends_serialize_in_seq_order() {
    let (_dir, pool) = test_pool().await;
    let writers = std::sync::Arc::new(WorktreeWriters::new());
    sqlx::query("CREATE TABLE ordering (seq INTEGER, tag INTEGER)")
        .execute(&pool)
        .await
        .unwrap();

    let mut handles = Vec::new();
    for i in 0..20 {
        let pool = pool.clone();
        let writers = writers.clone();
        handles.push(tokio::spawn(async move {
            let e = event(&format!("conc-{i}"));
            ev::append_with_projection(
                &pool,
                &writers,
                "w1",
                e,
                Box::new(move |conn| {
                    Box::pin(async move {
                        // Record the max seq visible inside the txn: with a
                        // single writer this must be strictly increasing.
                        let (max_seq,): (i64,) =
                            sqlx::query_as("SELECT COALESCE(MAX(seq),0) FROM events")
                                .fetch_one(&mut *conn)
                                .await
                                .map_err(cairn_storage_local::StorageError::from)?;
                        sqlx::query("INSERT INTO ordering (seq, tag) VALUES (?, ?)")
                            .bind(max_seq)
                            .bind(i)
                            .execute(&mut *conn)
                            .await
                            .map_err(cairn_storage_local::StorageError::from)?;
                        Ok(())
                    })
                }),
            )
            .await
            .unwrap();
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    let rows: Vec<(i64,)> = sqlx::query_as("SELECT seq FROM ordering ORDER BY rowid")
        .fetch_all(&pool)
        .await
        .unwrap();
    let mut prev = 0;
    for (s,) in rows {
        assert!(
            s > prev || prev == 0,
            "projection order must follow seq order"
        );
        prev = s;
    }
    let events = ev::list_events(&pool, None, None, None, None, 100)
        .await
        .unwrap();
    assert_eq!(events.len(), 20);
    assert!(events.windows(2).all(|w| w[0].seq < w[1].seq));
}

#[tokio::test(flavor = "multi_thread")]
async fn partial_unique_index_blocks_second_live_session() {
    let (_dir, pool) = test_pool().await;
    seed_repo_worktree(&pool).await;
    sqlx::query(
        "INSERT INTO snapshots (id, worktree_id, branch, head_commit, staged_fp, unstaged_fp, \
         untracked_fp, snapshot_fp, fp_schema_version, created_at) \
         VALUES ('s1','w1','main','h','a','b','c','fp',1,'t')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let insert = |id: &'static str, state: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO sessions (id, repository_id, worktree_id, local_user, agent_type, \
                 agent_instance_id, agent_pid, resume_token_hash, lease_expires_at, state, \
                 start_snapshot_id, current_snapshot_id, started_at, ended_at, \
                 last_heartbeat_at, recovering_since) \
                 VALUES (?, 'r1', 'w1', 'u', 'agent', 'inst-1', NULL, 'h', 't', ?, 's1', 's1', \
                 't', NULL, 't', NULL)",
            )
            .bind(id)
            .bind(state)
            .execute(&pool)
            .await
        }
    };

    insert("sess1", "active").await.unwrap();
    assert!(
        insert("sess2", "active").await.is_err(),
        "second live session must be blocked"
    );
    assert!(
        insert("sess3", "recovering").await.is_err(),
        "recovering also counts as live"
    );
    // Terminal states do not collide.
    insert("sess4", "stopped").await.unwrap();
}

async fn seed_repo_worktree(pool: &SqlitePool) {
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
}
