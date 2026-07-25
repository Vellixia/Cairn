mod support;

use cairn_domain::{GoalContractV1, IdempotencyKey, ProjectStatus};
use cairn_project::{
    CreateProject, CreateTask, ProjectService, ProjectTaskError, ReviseTask, TaskService,
    UpdateProject,
};
use support::Harness;

fn contract(goal: &str) -> GoalContractV1 {
    GoalContractV1::new(
        goal.into(),
        vec!["included".into()],
        vec!["excluded".into()],
        vec!["accepted".into()],
        vec!["constraint".into()],
    )
    .unwrap()
}

#[tokio::test]
async fn task_creation_is_atomic_duplicate_titles_are_allowed_and_revisions_are_immutable() {
    let harness = Harness::new().await;
    let projects = ProjectService::new(harness.pool.clone());
    let tasks = TaskService::new(harness.pool.clone());
    let project = projects
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Task project".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;

    let first = tasks
        .create(CreateTask {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            title: "  Duplicate title  ".into(),
            goal_contract: contract("revision one"),
        })
        .await
        .unwrap();
    assert!(first.created);
    assert_eq!(first.task.title, "Duplicate title");
    assert_eq!(first.task.latest_revision_number, 1);
    assert_eq!(first.revision.revision_number, 1);
    assert_eq!(first.revision.parent_revision_id, None);

    let duplicate = tasks
        .create(CreateTask {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            title: "Duplicate title".into(),
            goal_contract: contract("another task"),
        })
        .await
        .unwrap();
    assert_ne!(duplicate.task.id, first.task.id);

    let revision_one_bytes = first.revision.goal_contract.canonical_bytes();
    let revision_one_fingerprint = first.revision.goal_contract_fingerprint.clone();
    let revision_two = tasks
        .revise(ReviseTask {
            idempotency_key: IdempotencyKey::new_v7(),
            task_id: first.task.id,
            parent_revision_id: None,
            goal_contract: contract("revision two"),
        })
        .await
        .unwrap();
    assert_eq!(revision_two.revision.revision_number, 2);
    assert_eq!(
        revision_two.revision.parent_revision_id,
        Some(first.revision.id)
    );
    assert_eq!(revision_two.task.id, first.task.id);
    assert_eq!(revision_two.task.project_id, first.task.project_id);
    assert_eq!(revision_two.task.latest_revision_number, 2);

    let latest = tasks.get(first.task.id, None).await.unwrap();
    let historical = tasks
        .get(first.task.id, Some(first.revision.id))
        .await
        .unwrap();
    assert_eq!(latest.revision.id, revision_two.revision.id);
    assert_eq!(historical.revision.id, first.revision.id);
    assert_eq!(
        historical.revision.goal_contract.canonical_bytes(),
        revision_one_bytes
    );
    assert_eq!(
        historical.revision.goal_contract_fingerprint,
        revision_one_fingerprint
    );

    let listed = tasks.list(project.id, None, 50).await.unwrap();
    assert_eq!(listed.tasks.len(), 2);
    assert!(listed.tasks.windows(2).all(|pair| pair[0].id < pair[1].id));

    let update = sqlx::query("UPDATE task_revisions SET created_at=created_at WHERE id=?")
        .bind(first.revision.id.to_string())
        .execute(&harness.pool)
        .await;
    let delete = sqlx::query("DELETE FROM task_revisions WHERE id=?")
        .bind(first.revision.id.to_string())
        .execute(&harness.pool)
        .await;
    assert!(update.is_err());
    assert!(delete.is_err());
}

#[tokio::test]
async fn explicit_parent_rules_and_archived_project_guards_are_enforced() {
    let harness = Harness::new().await;
    let projects = ProjectService::new(harness.pool.clone());
    let tasks = TaskService::new(harness.pool.clone());
    let project = projects
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Parents".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    let a = tasks
        .create(CreateTask {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            title: "A".into(),
            goal_contract: contract("a1"),
        })
        .await
        .unwrap();
    let b = tasks
        .create(CreateTask {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            title: "B".into(),
            goal_contract: contract("b1"),
        })
        .await
        .unwrap();
    let a2 = tasks
        .revise(ReviseTask {
            idempotency_key: IdempotencyKey::new_v7(),
            task_id: a.task.id,
            parent_revision_id: Some(a.revision.id),
            goal_contract: contract("a2"),
        })
        .await
        .unwrap();
    let a3 = tasks
        .revise(ReviseTask {
            idempotency_key: IdempotencyKey::new_v7(),
            task_id: a.task.id,
            parent_revision_id: Some(a.revision.id),
            goal_contract: contract("a3 branches from earlier revision"),
        })
        .await
        .unwrap();
    assert_eq!(a3.revision.revision_number, 3);
    assert_eq!(a3.revision.parent_revision_id, Some(a.revision.id));

    let wrong_task = tasks
        .revise(ReviseTask {
            idempotency_key: IdempotencyKey::new_v7(),
            task_id: a.task.id,
            parent_revision_id: Some(b.revision.id),
            goal_contract: contract("bad parent"),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        wrong_task,
        ProjectTaskError::TaskRevisionConflict { task_id } if task_id == a.task.id
    ));

    let not_earlier = tasks
        .revise(ReviseTask {
            idempotency_key: IdempotencyKey::new_v7(),
            task_id: a.task.id,
            parent_revision_id: Some(a3.revision.id),
            goal_contract: contract("parent is current latest and therefore earlier than new"),
        })
        .await
        .unwrap();
    assert_eq!(not_earlier.revision.revision_number, 4);
    assert_eq!(
        not_earlier.revision.parent_revision_id,
        Some(a3.revision.id)
    );
    assert_eq!(a2.revision.revision_number, 2);

    projects
        .update(UpdateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            name: None,
            description: None,
            clear_description: false,
            status: Some(ProjectStatus::Archived),
        })
        .await
        .unwrap();
    let create_error = tasks
        .create(CreateTask {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            title: "Rejected".into(),
            goal_contract: contract("rejected"),
        })
        .await
        .unwrap_err();
    let revise_error = tasks
        .revise(ReviseTask {
            idempotency_key: IdempotencyKey::new_v7(),
            task_id: a.task.id,
            parent_revision_id: None,
            goal_contract: contract("rejected"),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        create_error,
        ProjectTaskError::ProjectArchived { .. }
    ));
    assert!(matches!(
        revise_error,
        ProjectTaskError::ProjectArchived { .. }
    ));
    assert_eq!(
        tasks
            .get(a.task.id, None)
            .await
            .unwrap()
            .task
            .latest_revision_number,
        4
    );
}
