//! T069: independent Feature 002 US2 acceptance through real daemon IPC.

mod support;

use cairn_events::replay::{live_task_projections, rebuild_task_projections};
use cairn_protocol::{methods, ErrorCode, TaskCreateResult, TaskGetResult, TaskReviseResult};
use serde_json::json;
use support::TestDaemon;
use uuid::Uuid;

fn key() -> String {
    Uuid::now_v7().to_string()
}

fn contract(goal: &str) -> serde_json::Value {
    json!({
        "schema_version":1,
        "goal":goal,
        "included_scope":["tasks"],
        "excluded_scope":["session binding"],
        "acceptance_criteria":["immutable"],
        "constraints":["offline"]
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn immutable_task_revision_acceptance() {
    let daemon = TestDaemon::start().await;
    let project = daemon
        .call(
            methods::PROJECT_CREATE,
            &json!({"idempotency_key":key(),"name":"Tasks","description":null}),
        )
        .await
        .unwrap();
    let project_id = project["project"]["project_id"]
        .as_str()
        .unwrap()
        .to_string();

    let task_a_key = key();
    let task_a_request = json!({
        "idempotency_key":task_a_key,
        "project_id":project_id,
        "title":"Duplicate",
        "goal_contract":contract("revision one")
    });
    let task_a_value = daemon
        .call(methods::TASK_CREATE, &task_a_request)
        .await
        .unwrap();
    let task_a: TaskCreateResult = serde_json::from_value(task_a_value.clone()).unwrap();
    assert_eq!(task_a.revision.revision_number, 1);
    let revision_one_bytes = serde_json::to_vec(&task_a.revision.goal_contract).unwrap();
    let revision_one_fingerprint = task_a.revision.goal_contract_fingerprint.clone();

    let task_b_value = daemon
        .call(
            methods::TASK_CREATE,
            &json!({
                "idempotency_key":key(),
                "project_id":project_id,
                "title":"Duplicate",
                "goal_contract":contract("other task")
            }),
        )
        .await
        .unwrap();
    let task_b: TaskCreateResult = serde_json::from_value(task_b_value).unwrap();
    assert_ne!(task_a.task.task_id, task_b.task.task_id);

    let create_retry = daemon
        .call(methods::TASK_CREATE, &task_a_request)
        .await
        .unwrap();
    assert_eq!(create_retry, task_a_value);
    let create_conflict = daemon
        .call(
            methods::TASK_CREATE,
            &json!({
                "idempotency_key":task_a_key,
                "project_id":project_id,
                "title":"Different request",
                "goal_contract":contract("revision one")
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(create_conflict.code, ErrorCode::IdempotencyConflict);

    let task_a_id = task_a.task.task_id.to_string();
    let revision_two_key = key();
    let revision_two_request = json!({
        "idempotency_key":revision_two_key,
        "task_id":task_a_id,
        "parent_revision_id":null,
        "goal_contract":contract("revision two")
    });
    let revision_two_value = daemon
        .call(methods::TASK_REVISE, &revision_two_request)
        .await
        .unwrap();
    let revision_two: TaskReviseResult =
        serde_json::from_value(revision_two_value.clone()).unwrap();
    assert_eq!(revision_two.task.task_id, task_a.task.task_id);
    assert_eq!(revision_two.task.latest_revision_number, 2);
    assert_eq!(revision_two.revision.revision_number, 2);
    assert_eq!(
        revision_two.revision.parent_revision_id,
        Some(task_a.revision.revision_id)
    );
    assert_eq!(
        daemon
            .call(methods::TASK_REVISE, &revision_two_request)
            .await
            .unwrap(),
        revision_two_value
    );

    let latest: TaskGetResult = serde_json::from_value(
        daemon
            .call(
                methods::TASK_GET,
                &json!({"task_id":task_a_id,"revision_id":null}),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(latest.revision.revision_number, 2);
    let historical: TaskGetResult = serde_json::from_value(
        daemon
            .call(
                methods::TASK_GET,
                &json!({
                    "task_id":task_a_id,
                    "revision_id":task_a.revision.revision_id
                }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(historical.task.latest_revision_number, 2);
    assert_eq!(
        serde_json::to_vec(&historical.revision.goal_contract).unwrap(),
        revision_one_bytes
    );
    assert_eq!(
        historical.revision.goal_contract_fingerprint,
        revision_one_fingerprint
    );

    let revision_three: TaskReviseResult = serde_json::from_value(
        daemon
            .call(
                methods::TASK_REVISE,
                &json!({
                    "idempotency_key":key(),
                    "task_id":task_a_id,
                    "parent_revision_id":task_a.revision.revision_id,
                    "goal_contract":contract("explicit earlier parent")
                }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(revision_three.revision.revision_number, 3);
    assert_eq!(
        revision_three.revision.parent_revision_id,
        Some(task_a.revision.revision_id)
    );

    let cross_parent = daemon
        .call(
            methods::TASK_REVISE,
            &json!({
                "idempotency_key":key(),
                "task_id":task_a_id,
                "parent_revision_id":task_b.revision.revision_id,
                "goal_contract":contract("invalid cross parent")
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(cross_parent.code, ErrorCode::TaskRevisionConflict);

    let method_conflict = daemon
        .call(
            methods::TASK_REVISE,
            &json!({
                "idempotency_key":task_a_key,
                "task_id":task_a_id,
                "parent_revision_id":null,
                "goal_contract":contract("wrong method")
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(method_conflict.code, ErrorCode::IdempotencyConflict);
    assert_eq!(
        serde_json::to_value(method_conflict.data.unwrap()).unwrap()["reason"],
        json!("method_mismatch")
    );

    daemon
        .call(
            methods::PROJECT_UPDATE,
            &json!({"idempotency_key":key(),"project_id":project_id,"name":null,"description":null,"clear_description":false,"status":"archived"}),
        )
        .await
        .unwrap();
    let archived_create_key = key();
    let archived_create_request = json!({
        "idempotency_key":archived_create_key,
        "project_id":project_id,
        "title":"Rejected",
        "goal_contract":contract("rejected")
    });
    let archived_create = daemon
        .call(methods::TASK_CREATE, &archived_create_request)
        .await
        .unwrap_err();
    assert_eq!(archived_create.code, ErrorCode::ProjectArchived);
    let archived_revise = daemon
        .call(
            methods::TASK_REVISE,
            &json!({
                "idempotency_key":key(),
                "task_id":task_a_id,
                "parent_revision_id":null,
                "goal_contract":contract("rejected")
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(archived_revise.code, ErrorCode::ProjectArchived);

    daemon
        .call(
            methods::PROJECT_UPDATE,
            &json!({"idempotency_key":key(),"project_id":project_id,"name":null,"description":null,"clear_description":false,"status":"active"}),
        )
        .await
        .unwrap();
    let accepted_after_restore = daemon
        .call(methods::TASK_CREATE, &archived_create_request)
        .await
        .unwrap();
    assert_eq!(
        accepted_after_restore["revision"]["revision_number"],
        json!(1)
    );
    let revision_four: TaskReviseResult = serde_json::from_value(
        daemon
            .call(
                methods::TASK_REVISE,
                &json!({
                    "idempotency_key":key(),
                    "task_id":task_a_id,
                    "parent_revision_id":null,
                    "goal_contract":contract("revision four")
                }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(revision_four.revision.revision_number, 4);
    assert_eq!(
        revision_four.revision.parent_revision_id,
        Some(revision_three.revision.revision_id)
    );

    let pool = cairn_storage_local::open_pool_at(&daemon.db_path())
        .await
        .unwrap();
    let task_a_events: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events WHERE aggregate_type='task' AND aggregate_id=?",
    )
    .bind(&task_a_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(task_a_events.0, 5);
    let persisted_revision_one: (String, String) = sqlx::query_as(
        "SELECT goal_contract_json, goal_contract_fingerprint FROM task_revisions WHERE id=?",
    )
    .bind(task_a.revision.revision_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(persisted_revision_one.0.as_bytes(), revision_one_bytes);
    assert_eq!(persisted_revision_one.1, revision_one_fingerprint);
    assert_eq!(
        rebuild_task_projections(&pool).await.unwrap(),
        live_task_projections(&pool).await.unwrap()
    );
    daemon.stop().await;
}
