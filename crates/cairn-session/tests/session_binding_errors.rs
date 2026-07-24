mod support;

use cairn_domain::{
    EventId, IdempotencyKey, ProjectId, ProjectStatus, SessionId, SessionState, TaskRevisionId,
    Timestamp,
};
use cairn_project::{CreateProject, CreateTask, ProjectService, TaskService, UpdateProject};
use cairn_session::{BindSession, SessionBindingError};
use cairn_storage_local::records::{RepositoryRow, WorktreeRow};
use cairn_storage_local::{operation_idempotency, session_bindings, sessions};
use support::{contract, Harness};

#[tokio::test]
async fn relationship_and_archive_failures_are_typed_and_atomic() {
    let harness = Harness::new().await;
    let context = harness.context(SessionState::Stopped).await;
    let projects = ProjectService::new(harness.pool.clone());
    projects
        .update(UpdateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: context.project_id,
            name: None,
            description: None,
            clear_description: false,
            status: Some(ProjectStatus::Archived),
        })
        .await
        .unwrap();
    let key = IdempotencyKey::new_v7();
    let error = harness
        .sessions()
        .bind(BindSession {
            idempotency_key: key,
            session_id: context.session.id.parse().unwrap(),
            project_id: context.project_id,
            task_revision_id: context.revision_id,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, SessionBindingError::ProjectArchived { .. }));
    assert!(session_bindings::get(&harness.pool, &context.session.id)
        .await
        .unwrap()
        .is_none());
    assert!(operation_idempotency::get(&harness.pool, &key.to_string())
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        sessions::get_by_id(&harness.pool, &context.session.id)
            .await
            .unwrap()
            .unwrap()
            .binding_mode,
        "local_unbound"
    );
}

#[tokio::test]
async fn mismatches_and_conflicting_rebind_never_replace_the_first_binding() {
    let harness = Harness::new().await;
    let context = harness.context(SessionState::Recovering).await;
    let other_project = ProjectService::new(harness.pool.clone())
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Other".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    let other_revision = TaskService::new(harness.pool.clone())
        .create(CreateTask {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: other_project.id,
            title: "Other task".into(),
            goal_contract: contract("other"),
        })
        .await
        .unwrap()
        .revision;

    let not_associated = harness
        .sessions()
        .bind(BindSession {
            idempotency_key: IdempotencyKey::new_v7(),
            session_id: context.session.id.parse().unwrap(),
            project_id: other_project.id,
            task_revision_id: other_revision.id,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        not_associated,
        SessionBindingError::RepositoryNotAssociated { .. }
    ));

    let mismatch = harness
        .sessions()
        .bind(BindSession {
            idempotency_key: IdempotencyKey::new_v7(),
            session_id: context.session.id.parse().unwrap(),
            project_id: context.project_id,
            task_revision_id: other_revision.id,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        mismatch,
        SessionBindingError::TaskRevisionProjectMismatch { .. }
    ));

    let first_key = IdempotencyKey::new_v7();
    let first = harness
        .sessions()
        .bind(BindSession {
            idempotency_key: first_key,
            session_id: context.session.id.parse().unwrap(),
            project_id: context.project_id,
            task_revision_id: context.revision_id,
        })
        .await
        .unwrap();
    let retry = harness
        .sessions()
        .bind(BindSession {
            idempotency_key: first_key,
            session_id: context.session.id.parse().unwrap(),
            project_id: context.project_id,
            task_revision_id: context.revision_id,
        })
        .await
        .unwrap();
    assert_eq!(retry, first);

    let conflict = harness
        .sessions()
        .bind(BindSession {
            idempotency_key: IdempotencyKey::new_v7(),
            session_id: context.session.id.parse().unwrap(),
            project_id: other_project.id,
            task_revision_id: other_revision.id,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        conflict,
        SessionBindingError::SessionAlreadyBound { .. }
    ));
    assert_eq!(
        session_bindings::get(&harness.pool, &context.session.id)
            .await
            .unwrap()
            .unwrap()
            .task_revision_id,
        context.revision_id.to_string()
    );
}

#[tokio::test]
async fn missing_ids_worktree_ownership_and_cross_method_raw_keys_fail_closed() {
    let harness = Harness::new().await;
    let context = harness.context(SessionState::Active).await;
    let service = harness.sessions();
    assert!(matches!(
        service
            .bind(BindSession {
                idempotency_key: IdempotencyKey::new_v7(),
                session_id: SessionId::new_v7(),
                project_id: context.project_id,
                task_revision_id: context.revision_id,
            })
            .await
            .unwrap_err(),
        SessionBindingError::SessionNotFound { .. }
    ));
    assert!(matches!(
        service
            .bind(BindSession {
                idempotency_key: IdempotencyKey::new_v7(),
                session_id: context.session.id.parse().unwrap(),
                project_id: ProjectId::new_v7(),
                task_revision_id: context.revision_id,
            })
            .await
            .unwrap_err(),
        SessionBindingError::ProjectNotFound { .. }
    ));
    assert!(matches!(
        service
            .bind(BindSession {
                idempotency_key: IdempotencyKey::new_v7(),
                session_id: context.session.id.parse().unwrap(),
                project_id: context.project_id,
                task_revision_id: TaskRevisionId::new_v7(),
            })
            .await
            .unwrap_err(),
        SessionBindingError::TaskRevisionNotFound { .. }
    ));

    let now = Timestamp::now().to_rfc3339();
    let other_repository = EventId::new_v7().to_string();
    let other_worktree = EventId::new_v7().to_string();
    cairn_storage_local::repos::insert(
        &harness.pool,
        &RepositoryRow {
            id: other_repository.clone(),
            repo_uuid: EventId::new_v7().to_string(),
            canonical_path: "/other".into(),
            default_remote_name: None,
            default_remote_url: None,
            copied_from_repository_id: None,
            registered_at: now.clone(),
        },
    )
    .await
    .unwrap();
    cairn_storage_local::worktrees::insert(
        &harness.pool,
        &WorktreeRow {
            id: other_worktree.clone(),
            repository_id: other_repository,
            worktree_uuid: EventId::new_v7().to_string(),
            path: "/other".into(),
            is_main: 1,
            registered_at: now,
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE sessions SET worktree_id=? WHERE id=?")
        .bind(other_worktree)
        .bind(&context.session.id)
        .execute(&harness.pool)
        .await
        .unwrap();
    assert!(matches!(
        service
            .bind(BindSession {
                idempotency_key: IdempotencyKey::new_v7(),
                session_id: context.session.id.parse().unwrap(),
                project_id: context.project_id,
                task_revision_id: context.revision_id,
            })
            .await
            .unwrap_err(),
        SessionBindingError::RepositoryNotAssociated { .. }
    ));

    let raw_key = IdempotencyKey::new_v7();
    ProjectService::new(harness.pool.clone())
        .create(CreateProject {
            idempotency_key: raw_key,
            name: "Own the key".into(),
            description: None,
        })
        .await
        .unwrap();
    assert!(matches!(
        service
            .bind(BindSession {
                idempotency_key: raw_key,
                session_id: context.session.id.parse().unwrap(),
                project_id: context.project_id,
                task_revision_id: context.revision_id,
            })
            .await
            .unwrap_err(),
        SessionBindingError::IdempotencyConflict { .. }
    ));
}
