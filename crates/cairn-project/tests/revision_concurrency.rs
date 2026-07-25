mod support;

use std::sync::Arc;
use std::time::Duration;

use cairn_domain::{GoalContractV1, IdempotencyKey};
use cairn_project::{
    CreateProject, CreateTask, ProjectService, ProjectTaskError, ReviseTask, TaskService,
};
use cairn_storage_local::{WriteCheckpoint, WriteTestHooks, WriterPolicy};
use support::Harness;
use tokio::sync::Barrier;

fn contract(goal: impl Into<String>) -> GoalContractV1 {
    GoalContractV1::new(goal.into(), vec![], vec![], vec![], vec![]).unwrap()
}

async fn seeded(harness: &Harness) -> (TaskService, cairn_project::CreateTaskResult) {
    let projects = ProjectService::new(harness.pool.clone());
    let tasks = TaskService::new(harness.pool.clone());
    let project = projects
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Concurrency".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    let task = tasks
        .create(CreateTask {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            title: "Serialized revisions".into(),
            goal_contract: contract("revision 1"),
        })
        .await
        .unwrap();
    (tasks, task)
}

#[tokio::test]
async fn independent_pools_allocate_unique_positive_sequential_gap_free_revisions() {
    let harness = Harness::new().await;
    let (_, task) = seeded(&harness).await;
    let participants = 8;
    let barrier = Arc::new(Barrier::new(participants));
    let mut joins = Vec::new();
    for index in 0..participants {
        let service = TaskService::new(harness.independent_pool().await);
        let barrier = barrier.clone();
        let task_id = task.task.id;
        joins.push(tokio::spawn(async move {
            barrier.wait().await;
            service
                .revise(ReviseTask {
                    idempotency_key: IdempotencyKey::new_v7(),
                    task_id,
                    parent_revision_id: None,
                    goal_contract: contract(format!("revision {index}")),
                })
                .await
                .unwrap()
                .revision
        }));
    }
    let mut numbers = Vec::new();
    for join in joins {
        numbers.push(join.await.unwrap().revision_number);
    }
    numbers.sort_unstable();
    assert_eq!(numbers, (2..=participants as u64 + 1).collect::<Vec<_>>());

    let rows = cairn_storage_local::tasks::revisions(&harness.pool, &task.task.id.to_string())
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row.revision_number)
            .collect::<Vec<_>>(),
        (1..=participants as i64 + 1).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn rollback_and_cancellation_after_counter_allocation_leave_the_next_number_gap_free() {
    let harness = Harness::new().await;
    let (_, task) = seeded(&harness).await;

    let failing_hooks = WriteTestHooks::default();
    failing_hooks.fail_at(WriteCheckpoint::PostCounterAllocation);
    let failing = TaskService::with_test_controls(
        harness.independent_pool().await,
        WriterPolicy::default(),
        failing_hooks,
    );
    assert!(failing
        .revise(ReviseTask {
            idempotency_key: IdempotencyKey::new_v7(),
            task_id: task.task.id,
            parent_revision_id: None,
            goal_contract: contract("rolled back"),
        })
        .await
        .is_err());

    let pause = WriteTestHooks::default();
    pause.pause_at(WriteCheckpoint::PostCounterAllocation);
    let paused = TaskService::with_test_controls(
        harness.independent_pool().await,
        WriterPolicy::default(),
        pause.clone(),
    );
    let task_id = task.task.id;
    let cancelled = tokio::spawn(async move {
        paused
            .revise(ReviseTask {
                idempotency_key: IdempotencyKey::new_v7(),
                task_id,
                parent_revision_id: None,
                goal_contract: contract("cancelled"),
            })
            .await
    });
    pause
        .wait_until_reached(WriteCheckpoint::PostCounterAllocation)
        .await;
    cancelled.abort();
    assert!(cancelled.await.is_err());

    let success = TaskService::new(harness.independent_pool().await)
        .revise(ReviseTask {
            idempotency_key: IdempotencyKey::new_v7(),
            task_id: task.task.id,
            parent_revision_id: None,
            goal_contract: contract("revision 2"),
        })
        .await
        .unwrap();
    assert_eq!(success.revision.revision_number, 2);
}

#[tokio::test]
async fn deterministic_lock_timeout_returns_stable_storage_busy() {
    let harness = Harness::new().await;
    let (_, task) = seeded(&harness).await;
    let mut lock = harness.pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let service = TaskService::with_test_controls(
        harness.independent_pool().await,
        WriterPolicy::test_with_busy_timeout(Duration::from_millis(25)),
        WriteTestHooks::default(),
    );
    let error = service
        .revise(ReviseTask {
            idempotency_key: IdempotencyKey::new_v7(),
            task_id: task.task.id,
            parent_revision_id: None,
            goal_contract: contract("busy"),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ProjectTaskError::StorageBusy { max_elapsed_ms: 25 }
    ));
    sqlx::query("ROLLBACK").execute(&mut *lock).await.unwrap();
}
