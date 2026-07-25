mod support;

use cairn_domain::IdempotencyKey;
use cairn_project::{AssociateRepository, CreateProject, ProjectService, ProjectTaskError};
use support::Harness;

#[tokio::test]
async fn competing_associations_have_one_committed_winner() {
    let harness = Harness::new().await;
    let service = ProjectService::new(harness.pool.clone());
    let (repository_id, _) = harness.repository_with_worktrees(2).await;
    let first = service
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "First".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    let second = service
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Second".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    let left = ProjectService::new(harness.independent_pool().await);
    let right = ProjectService::new(harness.independent_pool().await);

    let (a, b) = tokio::join!(
        left.associate_repository(AssociateRepository {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: first.id,
            repository_id: repository_id.clone()
        }),
        right.associate_repository(AssociateRepository {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: second.id,
            repository_id: repository_id.clone()
        })
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let failure = a.as_ref().err().or_else(|| b.as_ref().err()).unwrap();
    assert!(matches!(
        failure,
        ProjectTaskError::RepositoryAlreadyAssociated { .. }
    ));

    let associations: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM project_repository_associations WHERE repository_id=?",
    )
    .bind(&repository_id)
    .fetch_one(&harness.pool)
    .await
    .unwrap();
    let events: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events WHERE event_type='project.repository_associated' AND aggregate_id=?")
        .bind(&repository_id).fetch_one(&harness.pool).await.unwrap();
    let registry: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM operation_idempotency WHERE method='project.repository_associate'",
    )
    .fetch_one(&harness.pool)
    .await
    .unwrap();
    assert_eq!((associations.0, events.0, registry.0), (1, 1, 1));
}

#[tokio::test]
async fn identical_concurrent_retry_returns_one_original_result() {
    let harness = Harness::new().await;
    let service = ProjectService::new(harness.pool.clone());
    let (repository_id, _) = harness.repository_with_worktrees(1).await;
    let project = service
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Same".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    let key = IdempotencyKey::new_v7();
    let left = ProjectService::new(harness.independent_pool().await);
    let right = ProjectService::new(harness.independent_pool().await);
    let request = || AssociateRepository {
        idempotency_key: key,
        project_id: project.id,
        repository_id: repository_id.clone(),
    };
    let (a, b) = tokio::join!(
        left.associate_repository(request()),
        right.associate_repository(request())
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a, b);
    assert!(a.created);
    let events: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events WHERE event_type='project.repository_associated'",
    )
    .fetch_one(&harness.pool)
    .await
    .unwrap();
    assert_eq!(events.0, 1);
}
