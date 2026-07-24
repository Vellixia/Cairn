mod support;

use cairn_protocol::*;
use support::binding::{contract, BindingFixture};
use support::TestDaemon;

#[tokio::test(flavor = "multi_thread")]
async fn session_bind_handler_maps_every_relationship_and_conflict_error_without_leakage() {
    let daemon = TestDaemon::start().await;
    let fixture = BindingFixture::create(&daemon).await;
    let other_project: ProjectCreateResult = serde_json::from_value(
        daemon
            .call(
                methods::PROJECT_CREATE,
                &ProjectCreateParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    name: "Other".into(),
                    description: None,
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let other_task: TaskCreateResult = serde_json::from_value(
        daemon
            .call(
                methods::TASK_CREATE,
                &TaskCreateParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    project_id: other_project.project.project_id,
                    title: "Other task".into(),
                    goal_contract: contract("private-goal-sentinel"),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();

    let mismatch = daemon
        .call(
            methods::SESSION_BIND,
            &SessionBindParams {
                idempotency_key: IdempotencyKey::new_v7(),
                session_id: fixture.session_id,
                project_id: fixture.project_id,
                task_revision_id: other_task.revision.revision_id,
            },
        )
        .await
        .unwrap_err();
    assert_private(&mismatch, ErrorCode::TaskRevisionProjectMismatch);

    let repository_mismatch = daemon
        .call(
            methods::SESSION_BIND,
            &SessionBindParams {
                idempotency_key: IdempotencyKey::new_v7(),
                session_id: fixture.session_id,
                project_id: other_project.project.project_id,
                task_revision_id: other_task.revision.revision_id,
            },
        )
        .await
        .unwrap_err();
    assert_private(&repository_mismatch, ErrorCode::RepositoryNotAssociated);

    daemon
        .call(
            methods::PROJECT_UPDATE,
            &ProjectUpdateParams {
                idempotency_key: IdempotencyKey::new_v7(),
                project_id: fixture.project_id,
                name: None,
                description: None,
                clear_description: false,
                status: Some(ProjectStatus::Archived),
            },
        )
        .await
        .unwrap();
    let archived = daemon
        .call(
            methods::SESSION_BIND,
            &fixture.bind_params(IdempotencyKey::new_v7()),
        )
        .await
        .unwrap_err();
    assert_private(&archived, ErrorCode::ProjectArchived);
    daemon
        .call(
            methods::PROJECT_UPDATE,
            &ProjectUpdateParams {
                idempotency_key: IdempotencyKey::new_v7(),
                project_id: fixture.project_id,
                name: None,
                description: None,
                clear_description: false,
                status: Some(ProjectStatus::Active),
            },
        )
        .await
        .unwrap();

    let raw_key = IdempotencyKey::new_v7();
    daemon
        .call(
            methods::PROJECT_CREATE,
            &ProjectCreateParams {
                idempotency_key: raw_key,
                name: "Key owner".into(),
                description: None,
            },
        )
        .await
        .unwrap();
    let key_conflict = daemon
        .call(methods::SESSION_BIND, &fixture.bind_params(raw_key))
        .await
        .unwrap_err();
    assert_private(&key_conflict, ErrorCode::IdempotencyConflict);

    daemon
        .call(
            methods::SESSION_BIND,
            &fixture.bind_params(IdempotencyKey::new_v7()),
        )
        .await
        .unwrap();
    let binding_conflict = daemon
        .call(
            methods::SESSION_BIND,
            &SessionBindParams {
                idempotency_key: IdempotencyKey::new_v7(),
                session_id: fixture.session_id,
                project_id: other_project.project.project_id,
                task_revision_id: other_task.revision.revision_id,
            },
        )
        .await
        .unwrap_err();
    assert_private(&binding_conflict, ErrorCode::SessionBindingConflict);

    let malformed = daemon
        .call(
            methods::SESSION_BIND,
            &serde_json::json!({"session_id": fixture.session_id}),
        )
        .await
        .unwrap_err();
    assert_eq!(malformed.code, ErrorCode::Usage);
    drop(fixture);
    daemon.stop().await;
}

fn assert_private(error: &ErrorBody, code: ErrorCode) {
    assert_eq!(error.code, code);
    assert_eq!(error.code.exit_code(), 1);
    let rendered = serde_json::to_string(error).unwrap();
    for forbidden in [
        "private-goal-sentinel",
        "resume_token",
        "/private/",
        "SELECT ",
        "request_body",
    ] {
        assert!(!rendered.contains(forbidden));
    }
}
