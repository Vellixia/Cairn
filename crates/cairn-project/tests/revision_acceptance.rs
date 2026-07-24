mod support;

use cairn_domain::{GoalContractV1, IdempotencyKey};
use cairn_project::{
    CreateProject, CreateTask, ProjectService, ProjectTaskError, ReviseTask, TaskService,
};
use support::Harness;

fn contract(goal: &str) -> GoalContractV1 {
    GoalContractV1::new(goal.into(), vec![], vec![], vec!["accepted".into()], vec![]).unwrap()
}

fn immutable_read_iterations() -> usize {
    std::env::var("CAIRN_TASK_READ_ITERS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(100)
}

#[tokio::test]
async fn global_raw_key_retries_return_original_results_and_conflicts_change_nothing() {
    let harness = Harness::new().await;
    let projects = ProjectService::new(harness.pool.clone());
    let tasks = TaskService::new(harness.pool.clone());
    let project = projects
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Idempotency".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;

    let create_key = IdempotencyKey::new_v7();
    let create = CreateTask {
        idempotency_key: create_key,
        project_id: project.id,
        title: "One".into(),
        goal_contract: contract("one"),
    };
    let first = tasks.create(create.clone()).await.unwrap();
    assert_eq!(tasks.create(create).await.unwrap(), first);
    let changed = tasks
        .create(CreateTask {
            idempotency_key: create_key,
            project_id: project.id,
            title: "Changed".into(),
            goal_contract: contract("one"),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        changed,
        ProjectTaskError::IdempotencyConflict { .. }
    ));

    let project_key = IdempotencyKey::new_v7();
    projects
        .create(CreateProject {
            idempotency_key: project_key,
            name: "Cross method".into(),
            description: None,
        })
        .await
        .unwrap();
    let cross_method = tasks
        .create(CreateTask {
            idempotency_key: project_key,
            project_id: project.id,
            title: "Cross".into(),
            goal_contract: contract("cross"),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        cross_method,
        ProjectTaskError::IdempotencyConflict { .. }
    ));

    let revise_key = IdempotencyKey::new_v7();
    let revise = ReviseTask {
        idempotency_key: revise_key,
        task_id: first.task.id,
        parent_revision_id: None,
        goal_contract: contract("two"),
    };
    let revision_two = tasks.revise(revise.clone()).await.unwrap();
    assert_eq!(tasks.revise(revise).await.unwrap(), revision_two);
    let changed_revise = tasks
        .revise(ReviseTask {
            idempotency_key: revise_key,
            task_id: first.task.id,
            parent_revision_id: None,
            goal_contract: contract("different"),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        changed_revise,
        ProjectTaskError::IdempotencyConflict { .. }
    ));

    let events =
        cairn_storage_local::events::list_events(&harness.pool, None, None, None, None, 100)
            .await
            .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.aggregate_id.as_deref() == Some(&first.task.id.to_string()))
            .count(),
        3
    );
}

#[tokio::test]
async fn repeated_reads_before_and_after_later_revisions_preserve_original_bytes() {
    let harness = Harness::new().await;
    let projects = ProjectService::new(harness.pool.clone());
    let tasks = TaskService::new(harness.pool.clone());
    let project = projects
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Read stability".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    let first = tasks
        .create(CreateTask {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            title: "Stable".into(),
            goal_contract: contract("original"),
        })
        .await
        .unwrap();
    let expected_bytes = first.revision.goal_contract.canonical_bytes();
    let expected_fingerprint = first.revision.goal_contract_fingerprint.clone();
    for _ in 0..immutable_read_iterations() {
        let read = tasks
            .get(first.task.id, Some(first.revision.id))
            .await
            .unwrap();
        assert_eq!(
            read.revision.goal_contract.canonical_bytes(),
            expected_bytes
        );
        assert_eq!(
            read.revision.goal_contract_fingerprint,
            expected_fingerprint
        );
    }
    tasks
        .revise(ReviseTask {
            idempotency_key: IdempotencyKey::new_v7(),
            task_id: first.task.id,
            parent_revision_id: None,
            goal_contract: contract("later"),
        })
        .await
        .unwrap();
    for _ in 0..immutable_read_iterations() {
        let read = tasks
            .get(first.task.id, Some(first.revision.id))
            .await
            .unwrap();
        assert_eq!(
            read.revision.goal_contract.canonical_bytes(),
            expected_bytes
        );
        assert_eq!(
            read.revision.goal_contract_fingerprint,
            expected_fingerprint
        );
    }
}
