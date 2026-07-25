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
fn session_bind_request_and_both_idempotent_results_are_typed_and_stable() {
    let request_value = golden("session-bind-request.json");
    let request: Request = serde_json::from_value(request_value.clone()).unwrap();
    assert_eq!(request.method, methods::SESSION_BIND);
    let _: SessionBindParams = serde_json::from_value(request.params.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), request_value);

    for (file, created) in [
        ("session-bind-created-response.json", true),
        ("session-bind-existing-response.json", false),
    ] {
        let value = golden(file);
        let response: Response = serde_json::from_value(value.clone()).unwrap();
        let result: SessionBindResult =
            serde_json::from_value(response.result.clone().unwrap()).unwrap();
        assert_eq!(result.created, created);
        assert!(matches!(result.scope, SessionScopeDto::ProjectBound { .. }));
        assert_eq!(serde_json::to_value(response).unwrap(), value);
    }
}

#[test]
fn session_bind_errors_are_closed_private_and_exit_one() {
    for (file, code) in [
        (
            "session-bind-project-archived-error.json",
            ErrorCode::ProjectArchived,
        ),
        (
            "session-bind-repository-mismatch-error.json",
            ErrorCode::RepositoryNotAssociated,
        ),
        (
            "session-bind-task-mismatch-error.json",
            ErrorCode::TaskRevisionProjectMismatch,
        ),
        (
            "session-bind-conflict-error.json",
            ErrorCode::SessionBindingConflict,
        ),
        (
            "session-bind-idempotency-conflict-error.json",
            ErrorCode::IdempotencyConflict,
        ),
    ] {
        let value = golden(file);
        let response: Response = serde_json::from_value(value.clone()).unwrap();
        let error = response.error.unwrap();
        assert_eq!(error.code, code);
        assert_eq!(error.code.exit_code(), 1);
        assert_eq!(
            serde_json::to_value(Response {
                id: value["id"].as_str().unwrap().into(),
                result: None,
                error: Some(error),
            })
            .unwrap(),
            value
        );
    }
    let malformed: Response =
        serde_json::from_value(golden("session-bind-malformed-error.json")).unwrap();
    assert_eq!(malformed.error.unwrap().code, ErrorCode::Usage);
    let rendered = (0..9)
        .map(|index| {
            serde_json::to_string(&golden(match index {
                0 => "session-bind-request.json",
                1 => "session-bind-created-response.json",
                2 => "session-bind-existing-response.json",
                3 => "session-bind-project-archived-error.json",
                4 => "session-bind-repository-mismatch-error.json",
                5 => "session-bind-task-mismatch-error.json",
                6 => "session-bind-conflict-error.json",
                7 => "session-bind-idempotency-conflict-error.json",
                _ => "session-bind-malformed-error.json",
            }))
            .unwrap()
        })
        .collect::<String>();
    for forbidden in ["resume_token", "/private/", "SELECT ", "goal_contract"] {
        assert!(!rendered.contains(forbidden));
    }
}

#[test]
fn session_bind_schema_is_discriminated_and_breaking_changes_trip() {
    assert!(methods::ALL_METHODS.contains(&methods::SESSION_BIND));
    let params = serde_json::to_string(&schema_for!(SessionBindParams)).unwrap();
    let result = serde_json::to_string(&schema_for!(SessionBindResult)).unwrap();
    for field in [
        "idempotency_key",
        "session_id",
        "project_id",
        "task_revision_id",
    ] {
        assert!(params.contains(field));
    }
    for field in [
        "session_id",
        "scope",
        "bound_at",
        "created",
        "project_bound",
    ] {
        assert!(result.contains(field));
    }
    assert!(
        serde_json::from_value::<SessionBindParams>(serde_json::json!({
            "idempotency_key":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0d",
            "session_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0a",
            "project_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0b",
            "task_revision_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0c",
            "internal_path":"/private/secret"
        }))
        .is_err()
    );
}
