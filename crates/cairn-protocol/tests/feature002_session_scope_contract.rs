use std::path::PathBuf;

use cairn_protocol::*;
use schemars::schema_for;

fn golden(name: &str) -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("goldens/ipc")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn omitted_explicit_unbound_and_bound_start_requests_are_closed_and_compatible() {
    let omitted: Request =
        serde_json::from_value(golden("session-start-unbound-omitted-request.json")).unwrap();
    let omitted_params: SessionStartParams = serde_json::from_value(omitted.params).unwrap();
    assert!(omitted_params.scope.is_none());

    let explicit: Request =
        serde_json::from_value(golden("session-start-unbound-explicit-request.json")).unwrap();
    let explicit_params: SessionStartParams = serde_json::from_value(explicit.params).unwrap();
    assert_eq!(explicit_params.scope, Some(SessionScopeDto::LocalUnbound));

    let bound: Request =
        serde_json::from_value(golden("session-start-bound-request.json")).unwrap();
    let bound_params: SessionStartParams = serde_json::from_value(bound.params).unwrap();
    assert!(matches!(
        bound_params.scope,
        Some(SessionScopeDto::ProjectBound { .. })
    ));

    let legacy: Request = serde_json::from_value(golden("session-start-request.json")).unwrap();
    let legacy_params: SessionStartParams = serde_json::from_value(legacy.params).unwrap();
    assert!(legacy_params.scope.is_none());
    assert!(
        serde_json::from_value::<SessionStartParams>(serde_json::json!({
            "repository_id":"repo-001",
            "agent_type":"contract-agent",
            "agent_instance_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d03",
            "scope": {
                "mode":"project_bound",
                "project_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d04"
            }
        }))
        .is_err()
    );
}

#[test]
fn scoped_start_success_goldens_are_typed_and_event_order_is_normative() {
    let unbound: Response =
        serde_json::from_value(golden("session-start-unbound-response.json")).unwrap();
    let unbound_result: SessionStartResult =
        serde_json::from_value(unbound.result.unwrap()).unwrap();
    assert_eq!(unbound_result.session.scope, SessionScopeDto::LocalUnbound);

    let bound: Response =
        serde_json::from_value(golden("session-start-bound-response.json")).unwrap();
    let bound_result: SessionStartResult = serde_json::from_value(bound.result.unwrap()).unwrap();
    assert!(matches!(
        bound_result.session.scope,
        SessionScopeDto::ProjectBound { .. }
    ));
    assert_eq!(
        golden("session-start-bound-event-order.json"),
        serde_json::json!(["session.started", "session.bound"])
    );
}

#[test]
fn every_scoped_start_error_golden_is_typed_private_and_has_a_stable_exit_code() {
    for (file, code, exit) in [
        (
            "session-start-project-scope-required-error.json",
            ErrorCode::ProjectScopeRequired,
            1,
        ),
        (
            "session-start-scope-conflict-error.json",
            ErrorCode::SessionScopeConflict,
            1,
        ),
        (
            "session-start-project-archived-error.json",
            ErrorCode::ProjectArchived,
            1,
        ),
        (
            "session-start-project-not-found-error.json",
            ErrorCode::ProjectNotFound,
            1,
        ),
        (
            "session-start-repository-mismatch-error.json",
            ErrorCode::RepositoryNotAssociated,
            1,
        ),
        (
            "session-start-task-mismatch-error.json",
            ErrorCode::TaskRevisionProjectMismatch,
            1,
        ),
        (
            "session-start-revision-not-found-error.json",
            ErrorCode::TaskRevisionNotFound,
            1,
        ),
        (
            "session-start-incomplete-bound-error.json",
            ErrorCode::Usage,
            2,
        ),
        (
            "watcher-start-failed-install.json",
            ErrorCode::WatcherStartFailed,
            1,
        ),
        (
            "watcher-start-failed-reconcile.json",
            ErrorCode::WatcherStartFailed,
            1,
        ),
    ] {
        let value = golden(file);
        let response: Response = serde_json::from_value(value.clone()).unwrap();
        let error = response.error.unwrap();
        assert_eq!(error.code, code, "{file}");
        assert_eq!(error.code.exit_code(), exit, "{file}");
        let rendered = serde_json::to_string(&value).unwrap();
        for forbidden in ["resume_token", "/private/", "SELECT ", "goal_contract"] {
            assert!(!rendered.contains(forbidden), "{file} leaked {forbidden}");
        }
    }
}

#[test]
fn session_scope_schema_tripwires_cover_start_get_list_reattach_and_stop() {
    let start = serde_json::to_string(&schema_for!(SessionStartParams)).unwrap();
    let session = serde_json::to_string(&schema_for!(SessionDto)).unwrap();
    let summary = serde_json::to_string(&schema_for!(SessionSummaryDto)).unwrap();
    let list = serde_json::to_string(&schema_for!(SessionListParams)).unwrap();
    for required in [
        "local_unbound",
        "project_bound",
        "project_id",
        "task_revision_id",
    ] {
        assert!(start.contains(required));
        assert!(session.contains(required));
        assert!(summary.contains(required));
    }
    assert!(list.contains("project_id"));
    assert!(list.contains("task_revision_id"));
    assert!(
        serde_json::from_value::<SessionScopeDto>(serde_json::json!({
            "mode":"project_bound",
            "project_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d04",
            "task_revision_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d05",
            "internal_path":"/private/secret"
        }))
        .is_err()
    );
}
