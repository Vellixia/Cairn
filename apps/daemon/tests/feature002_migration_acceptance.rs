use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use cairn_domain::{IdempotencyKey, SessionId};
use cairn_project::{AssociateRepository, CreateProject, CreateTask, ProjectService, TaskService};
use cairn_session::{BindSession, SessionConfig, SessionService};
use cairn_storage_local::writer::WorktreeWriters;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;

const PRODUCER_SHA: &str = "4a06c4125715bb4b78b54e49c81eccd82100a7b7";

fn fixture_dir() -> PathBuf {
    std::env::var_os("CAIRN_FEATURE001_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/databases"))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn contract(goal: &str) -> cairn_domain::GoalContractV1 {
    cairn_domain::GoalContractV1::new(
        goal.into(),
        vec!["migrated".into()],
        vec![],
        vec!["replays".into()],
        vec!["offline".into()],
    )
    .unwrap()
}

type LegacyEvent = (
    i64,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    String,
);

fn event_hash(row: &LegacyEvent) -> String {
    let mut canonical = BTreeMap::new();
    canonical.insert("event_type", json!(row.3));
    canonical.insert("id", json!(row.1));
    canonical.insert("idempotency_key", json!(row.2));
    canonical.insert("payload", json!(row.8));
    canonical.insert("recorded_at", json!(row.9));
    canonical.insert("repository_id", json!(row.4));
    canonical.insert("seq", json!(row.0));
    canonical.insert("session_id", json!(row.6));
    canonical.insert("snapshot_id", json!(row.7));
    canonical.insert("worktree_id", json!(row.5));
    sha256(&serde_json::to_vec(&canonical).unwrap())
}

#[tokio::test]
async fn populated_feature001_fixture_migrates_reopens_mutates_and_replays_without_loss() {
    let fixture_path = fixture_dir().join("feature-001-v1.sqlite3");
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(fixture_dir().join("feature-001-v1.manifest.json")).unwrap(),
    )
    .unwrap();
    let fixture_bytes = std::fs::read(&fixture_path).unwrap();
    assert_eq!(manifest["producer"]["commit_sha"], PRODUCER_SHA);
    assert_eq!(manifest["producer"]["maximum_migration_version"], 1);
    assert_eq!(manifest["producer"]["feature_002_code_used"], false);
    assert_eq!(manifest["fixture"]["sha256"], sha256(&fixture_bytes));

    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("migrated.sqlite3");
    std::fs::copy(&fixture_path, &copy).unwrap();
    let pool = cairn_storage_local::open_pool_at(&copy).await.unwrap();

    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA quick_check")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "ok"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pragma_foreign_key_check")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    for table in [
        "repositories",
        "worktrees",
        "snapshots",
        "sessions",
        "events",
    ] {
        let count = sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(manifest["table_counts"][table], count, "historical {table}");
    }

    // `meta` is the one historical table the migration is allowed to extend, and only
    // with the declared local schema-version marker.
    let meta_rows: Vec<(String, String)> =
        sqlx::query_as("SELECT key,value FROM meta ORDER BY key")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        meta_rows,
        vec![
            ("fp_schema_version".to_string(), "1".to_string()),
            ("local_schema_version".to_string(), "2".to_string()),
        ],
        "migration must preserve historical meta rows and add only the schema-version marker"
    );
    let legacy_events: Vec<LegacyEvent> = sqlx::query_as(
        "SELECT seq,id,idempotency_key,event_type,repository_id,worktree_id,session_id,snapshot_id,payload,recorded_at FROM events WHERE seq<=18 ORDER BY seq",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let hashes = legacy_events.iter().map(event_hash).collect::<Vec<_>>();
    let expected_hashes = manifest["ordered_event_row_hashes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["sha256"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(hashes, expected_hashes);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sessions WHERE binding_mode='local_unbound'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        4
    );

    // Every historical session identifier, lifecycle state, timestamp, lease, snapshot
    // reference, and resume-token hash must survive field-for-field.
    let session_rows = sqlx::query(
        "SELECT id,repository_id,worktree_id,local_user,agent_type,agent_instance_id,agent_pid,resume_token_hash,lease_expires_at,state,start_snapshot_id,current_snapshot_id,started_at,ended_at,last_heartbeat_at,recovering_since,binding_mode FROM sessions ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let mut live_sessions = Vec::new();
    for row in &session_rows {
        assert_eq!(
            row.get::<String, _>("binding_mode"),
            "local_unbound",
            "every migrated session must be explicitly local_unbound"
        );
        let mut fields = BTreeMap::new();
        for column in [
            "id",
            "repository_id",
            "worktree_id",
            "local_user",
            "agent_type",
            "agent_instance_id",
            "resume_token_hash",
            "lease_expires_at",
            "state",
            "start_snapshot_id",
            "current_snapshot_id",
            "started_at",
            "ended_at",
            "last_heartbeat_at",
            "recovering_since",
        ] {
            fields.insert(
                column.to_string(),
                json!(row.get::<Option<String>, _>(column)),
            );
        }
        fields.insert(
            "agent_pid".into(),
            json!(row.get::<Option<i64>, _>("agent_pid")),
        );
        live_sessions.push(fields);
    }
    let mut expected_sessions = manifest["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| {
            session
                .as_object()
                .unwrap()
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<BTreeMap<String, Value>>()
        })
        .collect::<Vec<_>>();
    expected_sessions.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    assert_eq!(
        live_sessions, expected_sessions,
        "migration changed a stable historical session field"
    );

    for table in [
        "projects",
        "project_repository_associations",
        "tasks",
        "task_revisions",
        "session_bindings",
        "operation_idempotency",
        "event_aggregate_heads",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&pool)
                .await
                .unwrap(),
            0,
            "migration fabricated {table}"
        );
    }

    pool.close().await;
    let pool = cairn_storage_local::open_pool_at(&copy).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE version=2")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1,
        "second open must version-gate migration 0002"
    );

    let active = &manifest["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["state"] == "active")
        .unwrap();
    let repository_id = active["repository_id"].as_str().unwrap().to_string();
    let session_id = SessionId::from_str(active["id"].as_str().unwrap()).unwrap();
    let projects = ProjectService::new(pool.clone());
    let project = projects
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Migrated project".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    projects
        .associate_repository(AssociateRepository {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            repository_id,
        })
        .await
        .unwrap();
    let task = TaskService::new(pool.clone())
        .create(CreateTask {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            title: "Migrated task".into(),
            goal_contract: contract("migrated revision one"),
        })
        .await
        .unwrap();
    SessionService::new(
        pool.clone(),
        Arc::new(WorktreeWriters::new()),
        SessionConfig::from_env(),
    )
    .bind(BindSession {
        idempotency_key: IdempotencyKey::new_v7(),
        session_id,
        project_id: project.id,
        task_revision_id: task.revision.id,
    })
    .await
    .unwrap();
    cairn_events::replay::verify_mixed_projections(&pool)
        .await
        .unwrap();
    pool.close().await;

    let pool = cairn_storage_local::open_pool_at(&copy).await.unwrap();
    assert_eq!(
        cairn_storage_local::session_bindings::mode(&pool, &session_id.to_string())
            .await
            .unwrap()
            .as_deref(),
        Some("project_bound")
    );
    cairn_events::replay::verify_mixed_projections(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn representative_larger_v1_fixture_preserves_every_added_row_during_real_migration() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("large-v1.sqlite3");
    std::fs::copy(fixture_dir().join("feature-001-v1.sqlite3"), &copy).unwrap();
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", copy.display()))
        .unwrap()
        .foreign_keys(true);
    let raw = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    for index in 0..32 {
        let repository_id = format!("large-repository-{index:03}");
        let worktree_id = format!("large-worktree-{index:03}");
        let snapshot_id = format!("large-snapshot-{index:03}");
        let session_id = format!("large-session-{index:03}");
        let now = format!("2026-07-22T02:{index:02}:00.000Z");
        sqlx::query(
            "INSERT INTO repositories(id,repo_uuid,canonical_path,registered_at) VALUES(?,?,?,?)",
        )
        .bind(&repository_id)
        .bind(format!("large-repo-uuid-{index:03}"))
        .bind(format!("/large/{index:03}"))
        .bind(&now)
        .execute(&raw)
        .await
        .unwrap();
        sqlx::query("INSERT INTO worktrees(id,repository_id,worktree_uuid,path,is_main,registered_at) VALUES(?,?,?,?,1,?)")
            .bind(&worktree_id)
            .bind(&repository_id)
            .bind(format!("large-worktree-uuid-{index:03}"))
            .bind(format!("/large/{index:03}"))
            .bind(&now)
            .execute(&raw)
            .await
            .unwrap();
        sqlx::query("INSERT INTO snapshots(id,worktree_id,branch,head_commit,staged_fp,unstaged_fp,untracked_fp,snapshot_fp,fp_schema_version,created_at) VALUES(?,?,'main','head','s','u','n',?,1,?)")
            .bind(&snapshot_id)
            .bind(&worktree_id)
            .bind(format!("large-fingerprint-{index:03}"))
            .bind(&now)
            .execute(&raw)
            .await
            .unwrap();
        sqlx::query("INSERT INTO sessions(id,repository_id,worktree_id,local_user,agent_type,agent_instance_id,resume_token_hash,lease_expires_at,state,start_snapshot_id,current_snapshot_id,started_at,last_heartbeat_at) VALUES(?,?,?,'local','fixture',?,'hash',?,'stopped',?,?,?,?)")
            .bind(&session_id)
            .bind(&repository_id)
            .bind(&worktree_id)
            .bind(format!("large-agent-{index:03}"))
            .bind(&now)
            .bind(&snapshot_id)
            .bind(&snapshot_id)
            .bind(&now)
            .bind(&now)
            .execute(&raw)
            .await
            .unwrap();
    }
    let before = [
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM repositories")
            .fetch_one(&raw)
            .await
            .unwrap(),
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM worktrees")
            .fetch_one(&raw)
            .await
            .unwrap(),
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM snapshots")
            .fetch_one(&raw)
            .await
            .unwrap(),
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions")
            .fetch_one(&raw)
            .await
            .unwrap(),
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events")
            .fetch_one(&raw)
            .await
            .unwrap(),
    ];
    raw.close().await;
    let migrated = cairn_storage_local::open_pool_at(&copy).await.unwrap();
    let after = [
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM repositories")
            .fetch_one(&migrated)
            .await
            .unwrap(),
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM worktrees")
            .fetch_one(&migrated)
            .await
            .unwrap(),
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM snapshots")
            .fetch_one(&migrated)
            .await
            .unwrap(),
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions")
            .fetch_one(&migrated)
            .await
            .unwrap(),
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events")
            .fetch_one(&migrated)
            .await
            .unwrap(),
    ];
    assert_eq!(after, before);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sessions WHERE binding_mode='local_unbound'"
        )
        .fetch_one(&migrated)
        .await
        .unwrap(),
        before[3]
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA quick_check")
            .fetch_one(&migrated)
            .await
            .unwrap(),
        "ok"
    );
}
