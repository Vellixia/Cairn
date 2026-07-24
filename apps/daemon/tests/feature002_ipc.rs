mod support;

use std::collections::BTreeSet;

use cairn_protocol::*;
use support::binding::{contract, BindingFixture};
use support::TestDaemon;

#[test]
fn protocol_constants_routes_schemas_and_cli_mappings_have_one_inventory() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let router = std::fs::read_to_string(root.join("apps/daemon/src/router.rs")).unwrap();
    let cli = [
        std::fs::read_to_string(root.join("apps/cli/src/commands/project.rs")).unwrap(),
        std::fs::read_to_string(root.join("apps/cli/src/commands/task.rs")).unwrap(),
        std::fs::read_to_string(root.join("apps/cli/src/commands/session.rs")).unwrap(),
    ]
    .join("\n");
    let schemas = root.join("crates/cairn-protocol/schemas");

    let mut methods = BTreeSet::new();
    for method in methods::ALL_METHODS {
        assert!(methods.insert(*method), "duplicate method {method}");
        let constant = method
            .strip_prefix("v1.")
            .unwrap()
            .replace('.', "_")
            .to_ascii_uppercase();
        assert!(
            router.contains(&format!("methods::{constant}")),
            "{method} has no daemon route"
        );
    }

    for surface in methods::FEATURE002_METHODS {
        assert!(schemas
            .join(format!("{}.json", surface.params_schema))
            .is_file());
        assert!(schemas
            .join(format!("{}.json", surface.result_schema))
            .is_file());
        if let Some(command) = surface.cli_command {
            let protocol_constant = surface
                .method
                .strip_prefix("v1.")
                .unwrap()
                .replace('.', "_")
                .to_ascii_uppercase();
            assert!(
                cli.contains(&format!("methods::{protocol_constant}")),
                "{command} is not IPC-backed"
            );
            assert!(
                cli.contains(&format!("\"{command}\"")),
                "{command} has no CLI envelope mapping"
            );
        }
    }

    for forbidden in [
        "repository.transfer",
        "repository.remove",
        "task.delete",
        "task.move",
        "session.unbind",
        "session.rebind",
        "session.revision_transition",
        "sync.",
    ] {
        assert!(!methods.iter().any(|method| method.contains(forbidden)));
    }

    let ipc = root.join("crates/cairn-protocol/goldens/ipc");
    for (method, request, response) in [
        (
            methods::PROJECT_CREATE,
            "project-create-request.json",
            "project-create-response.json",
        ),
        (
            methods::PROJECT_LIST,
            "project-list-request.json",
            "project-list-response.json",
        ),
        (
            methods::PROJECT_GET,
            "project-get-request.json",
            "project-get-response.json",
        ),
        (
            methods::PROJECT_UPDATE,
            "project-update-request.json",
            "project-update-response.json",
        ),
        (
            methods::PROJECT_REPOSITORY_ASSOCIATE,
            "project-repository-associate-request.json",
            "project-repository-associate-response.json",
        ),
        (
            methods::TASK_CREATE,
            "task-create-request.json",
            "task-create-response.json",
        ),
        (
            methods::TASK_REVISE,
            "task-revise-request.json",
            "task-revise-response.json",
        ),
        (
            methods::TASK_LIST,
            "task-list-request.json",
            "task-list-response.json",
        ),
        (
            methods::TASK_GET,
            "task-get-historical-request.json",
            "task-get-historical-response.json",
        ),
        (
            methods::SESSION_BIND,
            "session-bind-request.json",
            "session-bind-created-response.json",
        ),
        (
            methods::SESSION_START,
            "session-start-bound-request.json",
            "session-start-bound-response.json",
        ),
        (
            methods::EVENTS_LIST,
            "events-list-aggregate-request.json",
            "events-list-aggregate-response.json",
        ),
    ] {
        let request: Request =
            serde_json::from_str(&std::fs::read_to_string(ipc.join(request)).unwrap()).unwrap();
        assert_eq!(request.method, method);
        let response: Response =
            serde_json::from_str(&std::fs::read_to_string(ipc.join(response)).unwrap()).unwrap();
        assert!(response.result.is_some() && response.error.is_none());
    }

    let errors = root.join("crates/cairn-protocol/goldens/errors");
    let error_goldens: Vec<_> = std::fs::read_dir(errors).unwrap().collect();
    assert_eq!(error_goldens.len(), 24);
    assert!(error_goldens.into_iter().all(|entry| {
        let response: Response =
            serde_json::from_str(&std::fs::read_to_string(entry.unwrap().path()).unwrap()).unwrap();
        response.result.is_none() && response.error.is_some()
    }));

    let cli_goldens = root.join("apps/cli/tests/goldens");
    let mut cli_envelopes = 0;
    for entry in std::fs::read_dir(cli_goldens).unwrap() {
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(entry.unwrap().path()).unwrap()).unwrap();
        if value.is_object() {
            let _: CliEnvelope = serde_json::from_value(value).unwrap();
            cli_envelopes += 1;
        }
    }
    assert!(
        cli_envelopes >= 20,
        "CLI envelope golden inventory unexpectedly shrank"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn every_feature002_method_runs_over_the_real_local_transport() {
    let daemon = TestDaemon::start().await;
    let fixture = BindingFixture::create(&daemon).await;

    let projects: ProjectListResult = decode(
        daemon
            .call(
                methods::PROJECT_LIST,
                &ProjectListParams {
                    status: None,
                    after_project_id: None,
                    limit: Some(20),
                },
            )
            .await
            .unwrap(),
    );
    assert!(projects
        .projects
        .iter()
        .any(|project| project.project_id == fixture.project_id));
    let _: ProjectGetResult = decode(
        daemon
            .call(
                methods::PROJECT_GET,
                &ProjectGetParams {
                    project_id: fixture.project_id,
                },
            )
            .await
            .unwrap(),
    );
    let _: ProjectUpdateResult = decode(
        daemon
            .call(
                methods::PROJECT_UPDATE,
                &ProjectUpdateParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    project_id: fixture.project_id,
                    name: None,
                    description: Some("transport verified".into()),
                    clear_description: false,
                    status: None,
                },
            )
            .await
            .unwrap(),
    );

    let tasks: TaskListResult = decode(
        daemon
            .call(
                methods::TASK_LIST,
                &TaskListParams {
                    project_id: fixture.project_id,
                    after_task_id: None,
                    limit: Some(20),
                },
            )
            .await
            .unwrap(),
    );
    assert!(tasks
        .tasks
        .iter()
        .any(|task| task.task_id == fixture.task_id));
    let _: TaskGetResult = decode(
        daemon
            .call(
                methods::TASK_GET,
                &TaskGetParams {
                    task_id: fixture.task_id,
                    revision_id: Some(fixture.revision_id),
                },
            )
            .await
            .unwrap(),
    );
    let revised: TaskReviseResult = decode(
        daemon
            .call(
                methods::TASK_REVISE,
                &TaskReviseParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    task_id: fixture.task_id,
                    parent_revision_id: Some(fixture.revision_id),
                    goal_contract: contract("transport revision two"),
                },
            )
            .await
            .unwrap(),
    );
    assert_eq!(revised.revision.revision_number, 2);

    let bound: SessionBindResult = decode(
        daemon
            .call(
                methods::SESSION_BIND,
                &fixture.bind_params(IdempotencyKey::new_v7()),
            )
            .await
            .unwrap(),
    );
    assert!(matches!(bound.scope, SessionScopeDto::ProjectBound { .. }));
    let shown: SessionGetResult = decode(
        daemon
            .call(
                methods::SESSION_GET,
                &SessionGetParams {
                    path: None,
                    repository_id: None,
                    session_id: Some(fixture.session_id),
                    agent_instance_id: None,
                    agent_type: None,
                },
            )
            .await
            .unwrap(),
    );
    assert!(matches!(
        shown.session.unwrap().scope,
        SessionScopeDto::ProjectBound { .. }
    ));
    let listed: SessionListResult = decode(
        daemon
            .call(
                methods::SESSION_LIST,
                &SessionListParams {
                    repository_id: None,
                    state: None,
                    project_id: Some(fixture.project_id),
                    task_revision_id: Some(fixture.revision_id),
                },
            )
            .await
            .unwrap(),
    );
    assert_eq!(listed.sessions.len(), 1);

    let project_id = fixture.project_id.to_string();
    let project_events: EventsListResult = decode(
        daemon
            .call(
                methods::EVENTS_LIST,
                &EventsListParams {
                    repository_id: None,
                    worktree_id: None,
                    session_id: None,
                    aggregate_type: Some(EventAggregateType::Project),
                    aggregate_id: Some(project_id.clone()),
                    after_seq: None,
                    limit: Some(100),
                },
            )
            .await
            .unwrap(),
    );
    assert!(!project_events.events.is_empty());
    assert!(project_events.events.iter().all(|event| {
        event.aggregate_type == Some(EventAggregateType::Project)
            && event.aggregate_id.as_deref() == Some(project_id.as_str())
    }));
    assert!(project_events
        .events
        .windows(2)
        .all(|pair| pair[0].seq < pair[1].seq));
    assert!(project_events
        .events
        .iter()
        .all(|event| event.worktree_id.is_none()));

    let session_id = fixture.session_id.to_string();
    let session_events: EventsListResult = decode(
        daemon
            .call(
                methods::EVENTS_LIST,
                &EventsListParams {
                    repository_id: Some(fixture.repository_id.clone()),
                    worktree_id: None,
                    session_id: Some(session_id.clone()),
                    aggregate_type: Some(EventAggregateType::Session),
                    aggregate_id: Some(session_id),
                    after_seq: None,
                    limit: Some(1),
                },
            )
            .await
            .unwrap(),
    );
    assert_eq!(session_events.events.len(), 1);
    assert!(session_events.next_after_seq.is_some());

    let all_events: EventsListResult = decode(
        daemon
            .call(
                methods::EVENTS_LIST,
                &EventsListParams {
                    repository_id: None,
                    worktree_id: None,
                    session_id: None,
                    aggregate_type: None,
                    aggregate_id: None,
                    after_seq: None,
                    limit: Some(100),
                },
            )
            .await
            .unwrap(),
    );
    assert!(
        all_events.events.iter().all(|event| {
            event.aggregate_type.is_some()
                && event.aggregate_id.is_some()
                && event.aggregate_seq.is_some()
        }),
        "fresh post-migration events must never use legacy-null aggregate scope"
    );

    let incomplete = daemon
        .call(
            methods::EVENTS_LIST,
            &serde_json::json!({"aggregate_type":"project"}),
        )
        .await
        .unwrap_err();
    assert_eq!(incomplete.code, ErrorCode::Usage);
    let incomplete = daemon
        .call(
            methods::EVENTS_LIST,
            &serde_json::json!({"aggregate_id":project_id}),
        )
        .await
        .unwrap_err();
    assert_eq!(incomplete.code, ErrorCode::Usage);
    let unknown = daemon
        .call("v1.feature003.forbidden", &serde_json::json!({}))
        .await
        .unwrap_err();
    assert_eq!(unknown.code, ErrorCode::Usage);
    daemon.stop().await;
}

fn decode<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> T {
    serde_json::from_value(value).unwrap()
}
