mod support;

use cairn_domain::{
    EventId, IdempotencyKey, ProjectId, ProjectStatus, SessionBindingMode, TaskRevisionId,
};
use cairn_project::{ProjectService, UpdateProject};
use cairn_session::{SessionError, StartOutcome};
use cairn_storage_local::{events, session_bindings, sessions};
use support::Harness;

#[tokio::test]
async fn bootstrap_unbound_and_valid_bound_start_follow_constitution_and_event_order() {
    let bootstrap = Harness::new().await;
    let context = bootstrap.start_context().await;
    let unbound = bootstrap
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "bootstrap-agent",
            &EventId::new_v7().to_string(),
            None,
            &context.snapshot,
            SessionBindingMode::LocalUnbound,
        )
        .await
        .unwrap();
    assert_eq!(unbound.outcome, StartOutcome::Created);
    assert_eq!(unbound.session.binding_mode, "local_unbound");

    let bound_harness = Harness::new().await;
    let context = bound_harness.start_context().await;
    let (project_id, _task_id, revision_id) = bound_harness.add_project_scope(&context).await;
    let agent_instance_id = EventId::new_v7().to_string();
    let bound = bound_harness
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "bound-agent",
            &agent_instance_id,
            None,
            &context.snapshot,
            SessionBindingMode::ProjectBound {
                project_id,
                task_revision_id: revision_id,
            },
        )
        .await
        .unwrap();
    assert_eq!(bound.session.binding_mode, "project_bound");
    let binding = session_bindings::get(&bound_harness.pool, &bound.session.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding.project_id, project_id.to_string());
    assert_eq!(binding.task_revision_id, revision_id.to_string());
    let types: Vec<_> = events::list_events(
        &bound_harness.pool,
        None,
        None,
        Some(&bound.session.id),
        None,
        10,
    )
    .await
    .unwrap()
    .into_iter()
    .map(|event| event.event_type)
    .collect();
    assert_eq!(types, vec!["session.started", "session.bound"]);
}

#[tokio::test]
async fn unbound_start_is_rejected_once_selectable_scope_exists_without_partial_state() {
    let harness = Harness::new().await;
    let context = harness.start_context().await;
    let (project_id, _, _) = harness.add_project_scope(&context).await;
    let error = harness
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "unbound-agent",
            &EventId::new_v7().to_string(),
            None,
            &context.snapshot,
            SessionBindingMode::LocalUnbound,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SessionError::ProjectScopeRequired {
            project_id: actual,
            ..
        } if actual == project_id
    ));
    assert!(
        sessions::list(&harness.pool, Some(&context.repository_id), None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn associated_project_without_a_task_revision_still_allows_bootstrap_unbound_start() {
    let harness = Harness::new().await;
    let context = harness.start_context().await;
    let project = ProjectService::new(harness.pool.clone())
        .create(cairn_project::CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Empty project".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    ProjectService::new(harness.pool.clone())
        .associate_repository(cairn_project::AssociateRepository {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            repository_id: context.repository_id.clone(),
        })
        .await
        .unwrap();

    let started = harness
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "bootstrap-agent",
            &EventId::new_v7().to_string(),
            None,
            &context.snapshot,
            SessionBindingMode::LocalUnbound,
        )
        .await
        .unwrap();
    assert_eq!(started.session.binding_mode, "local_unbound");
}

#[tokio::test]
async fn bound_start_rejects_missing_archived_and_mismatched_scope_without_partial_state() {
    let harness = Harness::new().await;
    let context = harness.start_context().await;
    let missing_project = ProjectId::new_v7();
    let missing_revision = TaskRevisionId::new_v7();
    let error = harness
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "missing-project",
            &EventId::new_v7().to_string(),
            None,
            &context.snapshot,
            SessionBindingMode::ProjectBound {
                project_id: missing_project,
                task_revision_id: missing_revision,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SessionError::ProjectNotFound { project_id } if project_id == missing_project
    ));

    let (project_id, _, revision_id) = harness.add_project_scope(&context).await;
    ProjectService::new(harness.pool.clone())
        .update(UpdateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id,
            name: None,
            description: None,
            clear_description: false,
            status: Some(ProjectStatus::Archived),
        })
        .await
        .unwrap();
    let error = harness
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "archived-project",
            &EventId::new_v7().to_string(),
            None,
            &context.snapshot,
            SessionBindingMode::ProjectBound {
                project_id,
                task_revision_id: revision_id,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SessionError::ProjectArchived { project_id: actual } if actual == project_id
    ));

    let other = harness.start_context().await;
    let (other_project, _, other_revision) = harness.add_project_scope(&other).await;
    let error = harness
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "repository-mismatch",
            &EventId::new_v7().to_string(),
            None,
            &context.snapshot,
            SessionBindingMode::ProjectBound {
                project_id: other_project,
                task_revision_id: other_revision,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SessionError::RepositoryNotAssociated { .. }
    ));

    ProjectService::new(harness.pool.clone())
        .update(UpdateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id,
            name: None,
            description: None,
            clear_description: false,
            status: Some(ProjectStatus::Active),
        })
        .await
        .unwrap();
    let error = harness
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "revision-mismatch",
            &EventId::new_v7().to_string(),
            None,
            &context.snapshot,
            SessionBindingMode::ProjectBound {
                project_id,
                task_revision_id: other_revision,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SessionError::TaskRevisionProjectMismatch { .. }
    ));

    assert!(
        sessions::list(&harness.pool, Some(&context.repository_id), None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn archived_project_association_does_not_block_a_new_local_bootstrap_session() {
    let harness = Harness::new().await;
    let context = harness.start_context().await;
    let (project_id, _, _) = harness.add_project_scope(&context).await;
    ProjectService::new(harness.pool.clone())
        .update(UpdateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id,
            name: None,
            description: None,
            clear_description: false,
            status: Some(ProjectStatus::Archived),
        })
        .await
        .unwrap();
    let started = harness
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "archived-bootstrap",
            &EventId::new_v7().to_string(),
            None,
            &context.snapshot,
            SessionBindingMode::LocalUnbound,
        )
        .await
        .unwrap();
    assert_eq!(started.session.binding_mode, "local_unbound");
}
