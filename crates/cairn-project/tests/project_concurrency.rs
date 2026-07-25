mod support;

use cairn_domain::{IdempotencyKey, ProjectStatus};
use cairn_events::replay::{live_project_projections, rebuild_project_projections};
use cairn_project::{CreateProject, ProjectService, ProjectTaskError, UpdateProject};
use support::Harness;

#[tokio::test]
async fn independent_connections_serialize_updates_and_replay_exactly() {
    let harness = Harness::new().await;
    let service = ProjectService::new(harness.pool.clone());
    let project = service
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Concurrent".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    let left = ProjectService::new(harness.independent_pool().await);
    let right = ProjectService::new(harness.independent_pool().await);

    let (a, b) = tokio::join!(
        left.update(UpdateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            name: Some("Left".into()),
            description: None,
            clear_description: false,
            status: None,
        }),
        right.update(UpdateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            name: Some("Right".into()),
            description: None,
            clear_description: false,
            status: Some(ProjectStatus::Archived),
        })
    );
    assert!(a.is_ok());
    assert!(b.is_ok());

    let events =
        cairn_storage_local::events::list_events(&harness.pool, None, None, None, None, 100)
            .await
            .unwrap();
    let project_events: Vec<_> = events
        .iter()
        .filter(|event| event.aggregate_id.as_deref() == Some(&project.id.to_string()))
        .collect();
    assert_eq!(project_events.len(), 3);
    assert_eq!(
        project_events
            .iter()
            .map(|e| e.aggregate_seq)
            .collect::<Vec<_>>(),
        vec![Some(1), Some(2), Some(3)]
    );
    assert!(project_events.iter().all(|event| {
        event.repository_id.is_none()
            && event.worktree_id.is_none()
            && event.session_id.is_none()
            && event.aggregate_type.as_deref() == Some("project")
    }));
    assert_eq!(
        rebuild_project_projections(&harness.pool).await.unwrap(),
        live_project_projections(&harness.pool).await.unwrap()
    );
}

#[tokio::test]
async fn one_raw_key_has_one_winner_across_independent_connections() {
    let harness = Harness::new().await;
    let service = ProjectService::new(harness.pool.clone());
    let project = service
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Initial".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    let key = IdempotencyKey::new_v7();
    let left = ProjectService::new(harness.independent_pool().await);
    let right = ProjectService::new(harness.independent_pool().await);
    let (a, b) = tokio::join!(
        left.update(UpdateProject {
            idempotency_key: key,
            project_id: project.id,
            name: Some("Winner A".into()),
            description: None,
            clear_description: false,
            status: None,
        }),
        right.update(UpdateProject {
            idempotency_key: key,
            project_id: project.id,
            name: Some("Winner B".into()),
            description: None,
            clear_description: false,
            status: None,
        })
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let error = a.err().or_else(|| b.err()).unwrap();
    assert!(matches!(
        error,
        ProjectTaskError::IdempotencyConflict { .. }
    ));

    let registry: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM operation_idempotency WHERE idempotency_key=?")
            .bind(key.to_string())
            .fetch_one(&harness.pool)
            .await
            .unwrap();
    assert_eq!(registry.0, 1);
}
