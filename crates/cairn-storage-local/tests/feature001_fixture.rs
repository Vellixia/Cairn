use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

const PRODUCER_SHA: &str = "4a06c4125715bb4b78b54e49c81eccd82100a7b7";

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/databases")
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn contains_forbidden_key(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key("resume_token") || object.values().any(contains_forbidden_key)
        }
        Value::Array(values) => values.iter().any(contains_forbidden_key),
        _ => false,
    }
}

#[tokio::test]
async fn frozen_feature001_fixture_matches_provenance_manifest() {
    let dir = fixture_dir();
    let db_path = dir.join("feature-001-v1.sqlite3");
    let manifest_path = dir.join("feature-001-v1.manifest.json");
    let manifest_bytes = std::fs::read(&manifest_path).expect("fixture manifest");
    let manifest: Value = serde_json::from_slice(&manifest_bytes).expect("valid manifest JSON");
    let db_bytes = std::fs::read(&db_path).expect("fixture database");

    assert_eq!(manifest["producer"]["commit_sha"], PRODUCER_SHA);
    assert_eq!(manifest["producer"]["clean_checkout"], true);
    assert_eq!(manifest["producer"]["maximum_migration_version"], 1);
    assert_eq!(manifest["producer"]["feature_002_code_used"], false);
    assert_eq!(manifest["fixture"]["sha256"], sha256(&db_bytes));
    assert_eq!(manifest["fixture"]["size_bytes"], db_bytes.len());
    assert!(!contains_forbidden_key(&manifest));

    let migration_0001 = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations/0001_init.sql");
    assert_eq!(
        manifest["schema"]["migration_0001_sha256"],
        sha256(&std::fs::read(migration_0001).expect("migration 0001"))
    );

    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))
        .expect("sqlite options")
        .read_only(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("open fixture read-only");

    let (quick_check,): (String,) = sqlx::query_as("PRAGMA quick_check")
        .fetch_one(&pool)
        .await
        .expect("quick_check");
    assert_eq!(quick_check, "ok");

    let (migration_version,): (i64,) =
        sqlx::query_as("SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .expect("migration version");
    assert_eq!(migration_version, 1, "fixture must predate Feature 002");

    let forbidden_objects = [
        "projects",
        "project_repository_associations",
        "tasks",
        "task_revisions",
        "session_bindings",
        "operation_idempotency",
        "event_aggregate_heads",
    ];
    for name in forbidden_objects {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE name = ?")
            .bind(name)
            .fetch_one(&pool)
            .await
            .expect("schema object lookup");
        assert_eq!(count, 0, "Feature 002 object {name} present in v1 fixture");
    }
    let (binding_columns,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'binding_mode'",
    )
    .fetch_one(&pool)
    .await
    .expect("session columns");
    assert_eq!(binding_columns, 0, "Feature 002 session column present");

    for table in [
        "_sqlx_migrations",
        "events",
        "meta",
        "repositories",
        "sessions",
        "snapshots",
        "worktrees",
    ] {
        let query = format!("SELECT COUNT(*) FROM {table}");
        let (actual,): (i64,) = sqlx::query_as(&query)
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|error| panic!("count {table}: {error}"));
        assert_eq!(manifest["table_counts"][table], actual);
    }

    let state_rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT state, COUNT(*) FROM sessions GROUP BY state ORDER BY state")
            .fetch_all(&pool)
            .await
            .expect("session state counts");
    let state_counts: BTreeMap<String, i64> = state_rows.into_iter().collect();
    assert_eq!(state_counts.get("active"), Some(&1));
    assert_eq!(state_counts.get("recovering"), Some(&1));
    assert_eq!(state_counts.get("stopped"), Some(&1));
    assert_eq!(state_counts.get("interrupted"), Some(&1));

    type SessionTuple = (
        String,
        String,
        String,
        String,
        String,
        String,
        Option<i64>,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
    );
    let sessions: Vec<SessionTuple> = sqlx::query_as(
        "SELECT id, repository_id, worktree_id, local_user, agent_type, agent_instance_id, \
         agent_pid, resume_token_hash, lease_expires_at, state, start_snapshot_id, \
         current_snapshot_id, started_at, ended_at, last_heartbeat_at, recovering_since \
         FROM sessions ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("session manifest rows");
    let session_values: Vec<Value> = sessions
        .into_iter()
        .map(|row| {
            json!({
                "id": row.0,
                "repository_id": row.1,
                "worktree_id": row.2,
                "local_user": row.3,
                "agent_type": row.4,
                "agent_instance_id": row.5,
                "agent_pid": row.6,
                "resume_token_hash": row.7,
                "lease_expires_at": row.8,
                "state": row.9,
                "start_snapshot_id": row.10,
                "current_snapshot_id": row.11,
                "started_at": row.12,
                "ended_at": row.13,
                "last_heartbeat_at": row.14,
                "recovering_since": row.15,
            })
        })
        .collect();
    assert_eq!(manifest["sessions"], Value::Array(session_values));

    type EventTuple = (
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
    let events: Vec<EventTuple> = sqlx::query_as(
        "SELECT seq, id, idempotency_key, event_type, repository_id, worktree_id, \
         session_id, snapshot_id, payload, recorded_at FROM events ORDER BY seq",
    )
    .fetch_all(&pool)
    .await
    .expect("ordered events");
    let event_hashes: Vec<Value> = events
        .into_iter()
        .map(|row| {
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
            json!({
                "seq": row.0,
                "sha256": sha256(&serde_json::to_vec(&canonical).expect("canonical event")),
            })
        })
        .collect();
    assert_eq!(
        manifest["ordered_event_row_hashes"],
        Value::Array(event_hashes.clone())
    );
    assert_eq!(
        manifest["ordered_event_hash_manifest_sha256"],
        sha256(&serde_json::to_vec(&event_hashes).expect("event hash manifest"))
    );

    let event_types: BTreeSet<String> = sqlx::query_scalar("SELECT event_type FROM events")
        .fetch_all(&pool)
        .await
        .expect("event types")
        .into_iter()
        .collect();
    for forbidden in [
        "project.created",
        "project.updated",
        "project.repository_associated",
        "task.created",
        "task.revision_created",
        "session.bound",
    ] {
        assert!(!event_types.contains(forbidden));
    }
}
