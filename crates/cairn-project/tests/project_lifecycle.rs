mod support;

use cairn_domain::{IdempotencyKey, ProjectStatus};
use cairn_project::{CreateProject, ProjectService, ProjectTaskError, UpdateProject};
use support::Harness;

#[tokio::test]
async fn create_list_show_update_archive_restore_and_duplicate_names() {
    let harness = Harness::new().await;
    let service = ProjectService::new(harness.pool.clone());

    let first_key = IdempotencyKey::new_v7();
    let first = service
        .create(CreateProject {
            idempotency_key: first_key,
            name: "  Cairn  ".into(),
            description: Some(" Local foundation ".into()),
        })
        .await
        .expect("create first project");
    let retry = service
        .create(CreateProject {
            idempotency_key: first_key,
            name: "  Cairn  ".into(),
            description: Some(" Local foundation ".into()),
        })
        .await
        .expect("retry create");
    assert!(first.created);
    assert_eq!(retry, first);
    assert_eq!(first.project.name, "Cairn");

    let second = service
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Cairn".into(),
            description: None,
        })
        .await
        .expect("duplicate name is allowed");
    assert_ne!(first.project.id, second.project.id);

    let page = service.list(None, None, 50).await.expect("list projects");
    assert_eq!(page.projects.len(), 2);
    assert!(page.projects.windows(2).all(|p| p[0].id < p[1].id));

    let shown = service.get(first.project.id).await.expect("show project");
    assert_eq!(shown.project, first.project);
    assert!(shown.associations.is_empty());
    assert_eq!(shown.task_count, 0);
    assert_eq!(shown.bound_session_count, 0);

    let update_key = IdempotencyKey::new_v7();
    let archived = service
        .update(UpdateProject {
            idempotency_key: update_key,
            project_id: first.project.id,
            name: Some("Cairn Local".into()),
            description: None,
            clear_description: true,
            status: Some(ProjectStatus::Archived),
        })
        .await
        .expect("archive project");
    assert!(archived.updated);
    assert_eq!(archived.project.status, ProjectStatus::Archived);
    assert_eq!(archived.project.description, None);

    let archived_retry = service
        .update(UpdateProject {
            idempotency_key: update_key,
            project_id: first.project.id,
            name: Some("Cairn Local".into()),
            description: None,
            clear_description: true,
            status: Some(ProjectStatus::Archived),
        })
        .await
        .expect("retry update");
    assert_eq!(archived_retry, archived);

    let restored = service
        .update(UpdateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: first.project.id,
            name: None,
            description: None,
            clear_description: false,
            status: Some(ProjectStatus::Active),
        })
        .await
        .expect("restore project");
    assert_eq!(restored.project.status, ProjectStatus::Active);

    let original_create_retry = service
        .create(CreateProject {
            idempotency_key: first_key,
            name: "  Cairn  ".into(),
            description: Some(" Local foundation ".into()),
        })
        .await
        .expect("create retry after later updates");
    assert_eq!(original_create_retry, first);

    let active = service
        .list(Some(ProjectStatus::Active), None, 1)
        .await
        .expect("filtered page");
    assert_eq!(active.projects.len(), 1);
    assert!(active.next_after_project_id.is_some());
}

#[tokio::test]
async fn invalid_updates_and_raw_key_reuse_fail_without_partial_state() {
    let harness = Harness::new().await;
    let service = ProjectService::new(harness.pool.clone());
    let key = IdempotencyKey::new_v7();
    let created = service
        .create(CreateProject {
            idempotency_key: key,
            name: "One".into(),
            description: None,
        })
        .await
        .unwrap();

    let conflict = service
        .create(CreateProject {
            idempotency_key: key,
            name: "Two".into(),
            description: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        conflict,
        ProjectTaskError::IdempotencyConflict { .. }
    ));

    let invalid = service
        .update(UpdateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: created.project.id,
            name: None,
            description: Some("new".into()),
            clear_description: true,
            status: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(invalid, ProjectTaskError::InvalidProject { .. }));
    assert_eq!(
        service.get(created.project.id).await.unwrap().project,
        created.project
    );

    let cross_method = service
        .update(UpdateProject {
            idempotency_key: key,
            project_id: created.project.id,
            name: Some("Different method".into()),
            description: None,
            clear_description: false,
            status: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        cross_method,
        ProjectTaskError::IdempotencyConflict {
            existing_method,
            requested_method: "project.update",
            reason: cairn_project::IdempotencyConflictKind::MethodMismatch,
            ..
        } if existing_method == "project.create"
    ));
}
