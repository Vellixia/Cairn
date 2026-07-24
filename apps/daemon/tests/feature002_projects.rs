//! T054: independent Feature 002 US1 acceptance through real daemon IPC.

mod support;

use cairn_events::replay::{live_project_projections, rebuild_project_projections};
use cairn_protocol::{methods, ErrorCode};
use fixtures_repositories::FixtureRepo;
use serde_json::json;
use support::TestDaemon;
use uuid::Uuid;

fn key() -> String {
    Uuid::now_v7().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn project_and_repository_association_acceptance() {
    let daemon = TestDaemon::start().await;
    let repository = FixtureRepo::new().unwrap();
    let registered = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": repository.root().to_string_lossy()}),
        )
        .await
        .unwrap();
    let repository_id = registered["repository"]["repository_id"]
        .as_str()
        .unwrap()
        .to_string();

    let linked = repository.add_linked_worktree("linked-before").unwrap();
    let linked_registered = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": linked.to_string_lossy()}),
        )
        .await
        .unwrap();
    assert_eq!(
        linked_registered["repository"]["repository_id"],
        json!(repository_id)
    );

    let first_create_key = key();
    let first = daemon
        .call(
            methods::PROJECT_CREATE,
            &json!({"idempotency_key":first_create_key,"name":"Duplicate","description":"first"}),
        )
        .await
        .unwrap();
    let second = daemon
        .call(
            methods::PROJECT_CREATE,
            &json!({"idempotency_key":key(),"name":"Duplicate","description":"second"}),
        )
        .await
        .unwrap();
    let first_id = first["project"]["project_id"].as_str().unwrap().to_string();
    let second_id = second["project"]["project_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(first_id, second_id);

    let method_conflict = daemon
        .call(
            methods::PROJECT_UPDATE,
            &json!({"idempotency_key":first_create_key,"project_id":first_id,"name":"Wrong reuse","description":null,"clear_description":false,"status":null}),
        )
        .await
        .unwrap_err();
    assert_eq!(method_conflict.code, ErrorCode::IdempotencyConflict);
    assert_eq!(
        serde_json::to_value(method_conflict.data.unwrap()).unwrap()["reason"],
        json!("method_mismatch")
    );

    let listed = daemon
        .call(methods::PROJECT_LIST, &json!({"limit":50}))
        .await
        .unwrap();
    let listed_ids: Vec<_> = listed["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|project| project["project_id"].as_str().unwrap())
        .collect();
    assert_eq!(listed_ids.len(), 2);
    assert!(listed_ids.windows(2).all(|ids| ids[0] < ids[1]));

    let association_key = key();
    let associated = daemon
        .call(
            methods::PROJECT_REPOSITORY_ASSOCIATE,
            &json!({"idempotency_key":association_key,"project_id":first_id,"repository_id":repository_id}),
        )
        .await
        .unwrap();
    assert_eq!(associated["created"], json!(true));
    let retry = daemon
        .call(
            methods::PROJECT_REPOSITORY_ASSOCIATE,
            &json!({"idempotency_key":association_key,"project_id":first_id,"repository_id":repository_id}),
        )
        .await
        .unwrap();
    assert_eq!(retry, associated);
    let distinct_retry = daemon
        .call(
            methods::PROJECT_REPOSITORY_ASSOCIATE,
            &json!({"idempotency_key":key(),"project_id":first_id,"repository_id":repository_id}),
        )
        .await
        .unwrap();
    assert_eq!(distinct_retry["created"], json!(false));

    let linked_after = repository.add_linked_worktree("linked-after").unwrap();
    daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": linked_after.to_string_lossy()}),
        )
        .await
        .unwrap();
    let pool = cairn_storage_local::open_pool_at(&daemon.db_path())
        .await
        .unwrap();
    let inherited: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM worktrees w JOIN project_repository_associations a ON a.repository_id=w.repository_id WHERE w.repository_id=? AND a.project_id=?",
    )
    .bind(&repository_id)
    .bind(&first_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(inherited.0, 3);

    let moved = repository.fixture_dir().join("repo-moved");
    std::fs::rename(repository.root(), &moved).unwrap();
    let moved_registration = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": moved.to_string_lossy()}),
        )
        .await
        .unwrap();
    assert_eq!(
        moved_registration["repository"]["repository_id"],
        json!(repository_id)
    );
    assert_eq!(
        moved_registration["worktree"]["worktree_id"],
        registered["worktree"]["worktree_id"]
    );

    let event_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events WHERE event_type='project.repository_associated' AND aggregate_id=?",
    )
    .bind(&repository_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(event_count.0, 1);

    let conflict = daemon
        .call(
            methods::PROJECT_REPOSITORY_ASSOCIATE,
            &json!({"idempotency_key":key(),"project_id":second_id,"repository_id":repository_id}),
        )
        .await
        .unwrap_err();
    assert_eq!(conflict.code, ErrorCode::RepositoryProjectConflict);

    daemon
        .call(
            methods::PROJECT_UPDATE,
            &json!({"idempotency_key":key(),"project_id":first_id,"name":null,"description":null,"clear_description":false,"status":"archived"}),
        )
        .await
        .unwrap();
    let other_repository = FixtureRepo::new().unwrap();
    let other_registered = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path":other_repository.root().to_string_lossy()}),
        )
        .await
        .unwrap();
    let other_repository_id = other_registered["repository"]["repository_id"]
        .as_str()
        .unwrap();
    let archived = daemon
        .call(
            methods::PROJECT_REPOSITORY_ASSOCIATE,
            &json!({"idempotency_key":key(),"project_id":first_id,"repository_id":other_repository_id}),
        )
        .await
        .unwrap_err();
    assert_eq!(archived.code, ErrorCode::ProjectArchived);

    daemon
        .call(
            methods::PROJECT_UPDATE,
            &json!({"idempotency_key":key(),"project_id":first_id,"name":null,"description":null,"clear_description":false,"status":"active"}),
        )
        .await
        .unwrap();
    let resumed = daemon
        .call(
            methods::PROJECT_REPOSITORY_ASSOCIATE,
            &json!({"idempotency_key":key(),"project_id":first_id,"repository_id":other_repository_id}),
        )
        .await
        .unwrap();
    assert_eq!(resumed["created"], json!(true));

    assert_eq!(
        rebuild_project_projections(&pool).await.unwrap(),
        live_project_projections(&pool).await.unwrap()
    );
    assert!(!methods::ALL_METHODS.iter().any(|method| {
        method.contains("repository_transfer") || method.contains("repository_remove")
    }));
    daemon.stop().await;
}
