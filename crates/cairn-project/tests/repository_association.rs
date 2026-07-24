mod support;

use cairn_domain::{IdempotencyKey, ProjectStatus};
use cairn_project::{
    AssociateRepository, CreateProject, ProjectService, ProjectTaskError, UpdateProject,
};
use support::Harness;

async fn create(service: &ProjectService, name: &str) -> cairn_domain::Project {
    service
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: name.into(),
            description: None,
        })
        .await
        .unwrap()
        .project
}

#[tokio::test]
async fn association_uses_repository_identity_and_all_worktrees_inherit_it() {
    let harness = Harness::new().await;
    let service = ProjectService::new(harness.pool.clone());
    let (repository_id, worktree_ids) = harness.repository_with_worktrees(2).await;
    let project = create(&service, "Identity").await;

    let key = IdempotencyKey::new_v7();
    let first = service
        .associate_repository(AssociateRepository {
            idempotency_key: key,
            project_id: project.id,
            repository_id: repository_id.clone(),
        })
        .await
        .unwrap();
    assert!(first.created);
    let retry = service
        .associate_repository(AssociateRepository {
            idempotency_key: key,
            project_id: project.id,
            repository_id: repository_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(retry, first);

    let distinct_idempotency_key = IdempotencyKey::new_v7();
    let distinct_key = service
        .associate_repository(AssociateRepository {
            idempotency_key: distinct_idempotency_key,
            project_id: project.id,
            repository_id: repository_id.clone(),
        })
        .await
        .unwrap();
    assert!(!distinct_key.created);
    assert_eq!(distinct_key.association, first.association);
    let distinct_retry = service
        .associate_repository(AssociateRepository {
            idempotency_key: distinct_idempotency_key,
            project_id: project.id,
            repository_id: repository_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(distinct_retry, distinct_key);

    for worktree_id in &worktree_ids {
        assert_eq!(
            service
                .project_for_worktree(worktree_id)
                .await
                .unwrap()
                .unwrap()
                .id,
            project.id
        );
    }

    cairn_storage_local::repos::update_canonical_path(
        &harness.pool,
        &repository_id,
        "/moved/repository",
    )
    .await
    .unwrap();
    cairn_storage_local::worktrees::update_path(
        &harness.pool,
        &worktree_ids[0],
        "/moved/repository",
    )
    .await
    .unwrap();
    assert_eq!(
        service
            .association_for_repository(&repository_id)
            .await
            .unwrap()
            .unwrap(),
        first.association
    );
}

#[tokio::test]
async fn conflicting_association_and_archived_mutation_are_rejected() {
    let harness = Harness::new().await;
    let service = ProjectService::new(harness.pool.clone());
    let (repository_id, _) = harness.repository_with_worktrees(1).await;
    let (other_repository_id, _) = harness.repository_with_worktrees(1).await;
    let first = create(&service, "First").await;
    let second = create(&service, "Second").await;

    service
        .associate_repository(AssociateRepository {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: first.id,
            repository_id: repository_id.clone(),
        })
        .await
        .unwrap();
    let conflict = service
        .associate_repository(AssociateRepository {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: second.id,
            repository_id: repository_id.clone(),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        conflict,
        ProjectTaskError::RepositoryAlreadyAssociated {
            repository_id: ref id,
            existing_project_id,
            requested_project_id,
        } if id == &repository_id && existing_project_id == first.id && requested_project_id == second.id
    ));

    service
        .update(UpdateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: first.id,
            name: None,
            description: None,
            clear_description: false,
            status: Some(ProjectStatus::Archived),
        })
        .await
        .unwrap();
    let archived = service
        .associate_repository(AssociateRepository {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: first.id,
            repository_id: other_repository_id,
        })
        .await
        .unwrap_err();
    assert!(
        matches!(archived, ProjectTaskError::ProjectArchived { project_id } if project_id == first.id)
    );
}
