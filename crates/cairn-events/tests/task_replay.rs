use cairn_domain::{
    EventId, GoalContractV1, ProjectId, Task, TaskId, TaskRevision, TaskRevisionId, Timestamp,
};
use cairn_events::aggregate::EventOperationMethod;
use cairn_events::catalog::{EventBuilder, TaskCreatedPayload, TaskRevisionCreatedPayload};
use cairn_events::replay::{live_task_projections, rebuild_task_projections};
use cairn_storage_local::records::{ProjectRow, TaskRevisionRow, TaskRow};
use cairn_storage_local::writer::{begin_immediate, WriterPolicy};
use cairn_storage_local::{events, open_pool_at, projects, tasks};

fn contract(goal: &str) -> GoalContractV1 {
    GoalContractV1::new(
        goal.into(),
        vec!["first".into(), "second".into()],
        vec![],
        vec!["accepted".into()],
        vec!["private-sentinel-is-ledger-only".into()],
    )
    .unwrap()
}

fn task_row(task: &Task) -> TaskRow {
    TaskRow {
        id: task.id.to_string(),
        project_id: task.project_id.to_string(),
        title: task.title.clone(),
        latest_revision_number: task.latest_revision_number as i64,
        created_at: task.created_at.to_rfc3339(),
        updated_at: task.updated_at.to_rfc3339(),
    }
}

fn revision_row(revision: &TaskRevision) -> TaskRevisionRow {
    TaskRevisionRow {
        id: revision.id.to_string(),
        task_id: revision.task_id.to_string(),
        revision_number: revision.revision_number as i64,
        parent_revision_id: revision.parent_revision_id.map(|id| id.to_string()),
        goal_contract_json: String::from_utf8(revision.goal_contract.canonical_bytes()).unwrap(),
        goal_contract_schema_version: i64::from(revision.goal_contract.schema_version()),
        goal_contract_fingerprint: revision.goal_contract_fingerprint.clone(),
        created_at: revision.created_at.to_rfc3339(),
    }
}

#[tokio::test]
async fn task_events_rebuild_complete_tasks_and_immutable_revisions_field_for_field() {
    let temp = tempfile::tempdir().unwrap();
    let pool = open_pool_at(&temp.path().join("task-replay.db"))
        .await
        .unwrap();
    let now = Timestamp::now();
    let project_id = ProjectId::new_v7();
    let task = Task::new(TaskId::new_v7(), project_id, "Immutable", now).unwrap();
    let revision_one = TaskRevision::new(
        TaskRevisionId::new_v7(),
        task.id,
        1,
        None,
        contract("revision 1"),
        now,
    )
    .unwrap();
    let later = Timestamp::now();
    let revision_two = TaskRevision::new(
        TaskRevisionId::new_v7(),
        task.id,
        2,
        Some(revision_one.id),
        contract("revision 2"),
        later,
    )
    .unwrap();
    let mut latest_task = task.clone();
    latest_task.latest_revision_number = 2;
    latest_task.updated_at = later;

    let task_event = EventBuilder::task_created(
        EventId::new_v7(),
        "task-create-replay",
        &TaskCreatedPayload {
            schema_version: 1,
            task: task.clone().into(),
        },
    );
    let revision_one_event = EventBuilder::task_revision_created(
        EventId::new_v7(),
        "task-create-replay",
        EventOperationMethod::TaskCreate,
        1,
        &TaskRevisionCreatedPayload {
            schema_version: 1,
            revision: revision_one.clone().into(),
            task: task.clone().into(),
        },
    );
    let revision_two_event = EventBuilder::task_revision_created(
        EventId::new_v7(),
        "task-revise-replay",
        EventOperationMethod::TaskRevise,
        0,
        &TaskRevisionCreatedPayload {
            schema_version: 1,
            revision: revision_two.clone().into(),
            task: latest_task.clone().into(),
        },
    );
    let project_row = ProjectRow {
        id: project_id.to_string(),
        name: "Replay".into(),
        description: None,
        status: "active".into(),
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };
    let initial_task_row = task_row(&task);
    let revision_one_row = revision_row(&revision_one);
    let revision_two_row = revision_row(&revision_two);

    begin_immediate(
        &pool,
        WriterPolicy::default(),
        None,
        Box::new(move |conn| {
            Box::pin(async move {
                projects::insert(conn, &project_row).await?;
                events::append_event(conn, &task_event).await?;
                events::append_event(conn, &revision_one_event).await?;
                tasks::insert_task(conn, &initial_task_row, &revision_one_row).await?;
                let stored =
                    tasks::insert_next_revision(conn, revision_two_row, &later.to_rfc3339(), None)
                        .await?;
                assert_eq!(stored.revision_number, 2);
                events::append_event(conn, &revision_two_event).await?;
                Ok(())
            })
        }),
    )
    .await
    .unwrap();

    let rebuilt = rebuild_task_projections(&pool).await.unwrap();
    let live = live_task_projections(&pool).await.unwrap();
    assert_eq!(rebuilt, live);
    assert_eq!(rebuilt.tasks[&task.id].latest_revision_number, 2);
    assert_eq!(rebuilt.revisions.len(), 2);

    let rows = events::list_events(&pool, None, None, None, None, 100)
        .await
        .unwrap();
    let task_rows: Vec<_> = rows
        .iter()
        .filter(|event| event.aggregate_id.as_deref() == Some(&task.id.to_string()))
        .collect();
    assert_eq!(task_rows.len(), 3);
    assert_eq!(
        task_rows
            .iter()
            .map(|event| event.aggregate_seq)
            .collect::<Vec<_>>(),
        vec![Some(1), Some(2), Some(3)]
    );
    assert!(task_rows.iter().all(|event| {
        event.aggregate_type.as_deref() == Some("task")
            && event.repository_id.is_none()
            && event.worktree_id.is_none()
            && event.session_id.is_none()
    }));
}
