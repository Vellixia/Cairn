mod support;

use std::path::Path;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use support::{feature001_fixture_path, TestDatabase};

async fn raw_pool(path: &Path) -> SqlitePool {
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .unwrap()
        .create_if_missing(false)
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap()
}

async fn rows(pool: &SqlitePool, query: &str) -> Vec<String> {
    sqlx::query_as::<_, (String,)>(query)
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|(value,)| value)
        .collect()
}

#[tokio::test]
async fn populated_feature001_fixture_migrates_with_field_for_field_history() {
    let source = raw_pool(&feature001_fixture_path()).await;
    let repositories = rows(&source, "SELECT json_array(id,repo_uuid,canonical_path,default_remote_name,default_remote_url,copied_from_repository_id,registered_at) FROM repositories ORDER BY id").await;
    let worktrees = rows(&source, "SELECT json_array(id,repository_id,worktree_uuid,path,is_main,registered_at) FROM worktrees ORDER BY id").await;
    let snapshots = rows(&source, "SELECT json_array(id,worktree_id,branch,head_commit,staged_fp,unstaged_fp,untracked_fp,snapshot_fp,fp_schema_version,created_at) FROM snapshots ORDER BY id").await;
    let sessions = rows(&source, "SELECT json_array(id,repository_id,worktree_id,local_user,agent_type,agent_instance_id,agent_pid,resume_token_hash,lease_expires_at,state,start_snapshot_id,current_snapshot_id,started_at,ended_at,last_heartbeat_at,recovering_since) FROM sessions ORDER BY id").await;
    let events = rows(&source, "SELECT json_array(seq,id,idempotency_key,event_type,repository_id,worktree_id,session_id,snapshot_id,payload,recorded_at) FROM events ORDER BY seq").await;
    source.close().await;

    let migrated = TestDatabase::from_feature001_fixture().await;
    assert_eq!(repositories, rows(&migrated.pool, "SELECT json_array(id,repo_uuid,canonical_path,default_remote_name,default_remote_url,copied_from_repository_id,registered_at) FROM repositories ORDER BY id").await);
    assert_eq!(worktrees, rows(&migrated.pool, "SELECT json_array(id,repository_id,worktree_uuid,path,is_main,registered_at) FROM worktrees ORDER BY id").await);
    assert_eq!(snapshots, rows(&migrated.pool, "SELECT json_array(id,worktree_id,branch,head_commit,staged_fp,unstaged_fp,untracked_fp,snapshot_fp,fp_schema_version,created_at) FROM snapshots ORDER BY id").await);
    assert_eq!(sessions, rows(&migrated.pool, "SELECT json_array(id,repository_id,worktree_id,local_user,agent_type,agent_instance_id,agent_pid,resume_token_hash,lease_expires_at,state,start_snapshot_id,current_snapshot_id,started_at,ended_at,last_heartbeat_at,recovering_since) FROM sessions ORDER BY id").await);
    assert_eq!(events, rows(&migrated.pool, "SELECT json_array(seq,id,idempotency_key,event_type,repository_id,worktree_id,session_id,snapshot_id,payload,recorded_at) FROM events ORDER BY seq").await);
    let (foreign_key_errors,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM pragma_foreign_key_check")
            .fetch_one(&migrated.pool)
            .await
            .unwrap();
    assert_eq!(foreign_key_errors, 0);
}
