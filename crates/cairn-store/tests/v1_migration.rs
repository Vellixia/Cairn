use cairn_store::{transfer, Store};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::fs;

async fn count(store: &Store, table: &str) -> i64 {
    sqlx::query(&format!("SELECT COUNT(*) AS count FROM {table}"))
        .fetch_one(store.pool())
        .await
        .unwrap()
        .get("count")
}

#[tokio::test]
async fn tombstone_summary_mismatch_rejects_before_target_mutation() {
    let source = Store::open_memory().await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let snapshot = dir.path().join("summary.sqlite");
    let mut manifest = transfer::export_snapshot(&source, &snapshot).await.unwrap();
    manifest.tombstones.clear();
    let target = Store::open_memory().await.unwrap();
    assert!(transfer::import_snapshot(&target, &manifest, &snapshot)
        .await
        .is_err());
    assert_eq!(count(&target, "projects").await, 0);
}

#[tokio::test]
async fn local_only_summary_mismatch_rejects_before_target_mutation() {
    let source = Store::open_memory().await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let snapshot = dir.path().join("summary.sqlite");
    let mut manifest = transfer::export_snapshot(&source, &snapshot).await.unwrap();
    manifest.local_only.clear();
    let target = Store::open_memory().await.unwrap();
    assert!(transfer::import_snapshot(&target, &manifest, &snapshot)
        .await
        .is_err());
    assert_eq!(count(&target, "projects").await, 0);
}

#[tokio::test]
async fn manifest_names_every_preservation_lane_and_restore_is_idempotent() {
    let source = Store::open_memory().await.unwrap();
    let project = uuid::Uuid::now_v7();
    let memory = uuid::Uuid::now_v7();
    let user = uuid::Uuid::now_v7();
    let session = uuid::Uuid::now_v7();
    let at = "2026-09-15T00:00:00Z";
    sqlx::query("INSERT INTO users (id, email, display_name, created_at) VALUES (?1, ?2, ?3, ?4)")
        .bind(user)
        .bind("migration@example.test")
        .bind("Migration test")
        .bind(at)
        .execute(source.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO projects (id, name, git_common_dir, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)")
        .bind(project)
        .bind("v1 transfer")
        .bind("/tmp/v1-transfer")
        .bind(at)
        .execute(source.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO sessions (id, project_id, user_id, agent, branch, worktree_path, agent_session_key, status, started_at, last_event_at, daemon_run_id) VALUES (?1, ?2, ?3, 'test', 'main', '/tmp/v1-transfer', 'migration-test', 'completed', ?4, ?4, 'run')")
        .bind(session)
        .bind(project)
        .bind(user)
        .bind(at)
        .execute(source.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO memories (id, project_id, type, scope, scope_key, content, state, origin_session_id, created_at, updated_at, deleted_at) VALUES (?1, ?2, 'fact', 'project', '', 'preserve me', 'active', ?3, ?4, ?4, NULL)")
        .bind(memory)
        .bind(project)
        .bind(session)
        .bind(at)
        .execute(source.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE memories SET deleted_at = ?1 WHERE id = ?2")
        .bind("2026-09-15T00:01:00Z")
        .bind(memory)
        .execute(source.pool())
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let snapshot = dir.path().join("backup.sqlite");
    let manifest = transfer::export_snapshot(&source, &snapshot).await.unwrap();
    let manifest_path = dir.path().join("manifest.json");
    transfer::write_manifest(&manifest, &manifest_path).unwrap();
    assert_eq!(transfer::read_manifest(&manifest_path).unwrap(), manifest);

    assert_eq!(manifest.version, transfer::MANIFEST_VERSION);
    assert!(manifest
        .lanes
        .values()
        .any(|tables| tables.contains_key("projects")));
    assert!(manifest
        .lanes
        .values()
        .any(|tables| tables.contains_key("memories")));
    assert!(manifest
        .lanes
        .values()
        .any(|tables| tables.contains_key("retained_local")));
    assert_eq!(manifest.snapshot, "backup.sqlite");
    assert_eq!(manifest.tombstones["memories"], 1);
    assert_eq!(manifest.local_only["memories"], 0);

    #[cfg(unix)]
    {
        let dangling = dir.path().join("dangling.sqlite");
        std::os::unix::fs::symlink("does-not-exist", &dangling).unwrap();
        assert!(transfer::export_snapshot(&source, &dangling).await.is_err());
    }
    assert!(snapshot.exists());

    let target = Store::open_memory().await.unwrap();
    let mut lane_mismatch = manifest.clone();
    lane_mismatch.lanes.clear();
    assert!(
        transfer::import_snapshot(&target, &lane_mismatch, &snapshot)
            .await
            .is_err()
    );
    assert_eq!(count(&target, "projects").await, 0);
    // Simulate a crash/failure after the ownership table copied but before a
    // canonical record can copy. Retrying the original snapshot must retain
    // the already copied project and continue with the memory.
    let interrupted = dir.path().join("interrupted.sqlite");
    fs::copy(&snapshot, &interrupted).unwrap();
    let corrupt = sqlx::SqlitePool::connect(&format!("sqlite://{}", interrupted.display()))
        .await
        .unwrap();
    sqlx::query("ALTER TABLE memories ADD COLUMN future_column TEXT")
        .execute(&corrupt)
        .await
        .unwrap();
    corrupt.close().await;
    assert!(transfer::import_snapshot(&target, &manifest, &interrupted)
        .await
        .is_err());
    assert_eq!(
        count(&target, "projects").await,
        0,
        "integrity rejection must not mutate target"
    );

    // A schema-changing snapshot with a matching digest reaches preflight;
    // column mismatch remains atomic too.
    let mut changed_manifest = manifest.clone();
    let bytes = fs::read(&interrupted).unwrap();
    changed_manifest.snapshot_size = bytes.len() as u64;
    changed_manifest.snapshot_sha256 = format!("{:x}", Sha256::digest(bytes));
    let error = transfer::import_snapshot(&target, &changed_manifest, &interrupted)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("incompatible_snapshot_schema"));
    assert_eq!(
        count(&target, "projects").await,
        0,
        "preflight must be atomic"
    );

    let first = transfer::import_snapshot(&target, &manifest, &snapshot)
        .await
        .unwrap();
    let second = transfer::import_snapshot(&target, &manifest, &snapshot)
        .await
        .unwrap();
    assert!(first.accepted > 0);
    assert_eq!(second.accepted, 0, "second import: {second:?}");
    assert_eq!(second.rejected, 0);
    assert!(
        second.retained > 0,
        "retry must name duplicate records retained"
    );
    assert_eq!(count(&target, "memories").await, 1);
    let violations: i64 = sqlx::query_scalar("SELECT count(*) FROM pragma_foreign_key_check")
        .fetch_one(target.pool())
        .await
        .unwrap();
    assert_eq!(violations, 0, "restore must preserve relations");
}

#[tokio::test]
async fn divergent_stable_id_rejects_before_dependents_attach() {
    let source = Store::open_memory().await.unwrap();
    let user = uuid::Uuid::now_v7();
    let project = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO users (id, email, display_name, created_at) VALUES (?1, ?2, ?3, ?4)")
        .bind(user)
        .bind("source@example.test")
        .bind("Source")
        .bind("2026-09-15T00:00:00Z")
        .execute(source.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO projects (id, name, git_common_dir, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)")
        .bind(project).bind("must not attach").bind("/tmp/divergent").bind("2026-09-15T00:00:00Z")
        .execute(source.pool()).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let snapshot = dir.path().join("divergent.sqlite");
    let manifest = transfer::export_snapshot(&source, &snapshot).await.unwrap();
    let target = Store::open_memory().await.unwrap();
    sqlx::query("INSERT INTO users (id, email, display_name, created_at) VALUES (?1, ?2, ?3, ?4)")
        .bind(user)
        .bind("target@example.test")
        .bind("Target")
        .bind("2026-09-15T00:00:00Z")
        .execute(target.pool())
        .await
        .unwrap();
    let error = transfer::import_snapshot(&target, &manifest, &snapshot)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("divergent_stable_id"), "{error}");
    assert_eq!(
        count(&target, "projects").await,
        0,
        "preflight must not attach dependents"
    );
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = ?1")
        .bind(user)
        .fetch_one(target.pool())
        .await
        .unwrap();
    assert_eq!(email, "target@example.test");
}
