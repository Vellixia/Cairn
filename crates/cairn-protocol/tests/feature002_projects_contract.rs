use std::path::PathBuf;

use cairn_protocol::*;
use schemars::schema_for;

fn golden(name: &str) -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("goldens")
        .join("ipc")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).expect("read project IPC golden"))
        .expect("valid project IPC golden")
}

#[test]
fn project_requests_and_success_results_match_goldens() {
    for (file, method) in [
        ("project-create-request.json", methods::PROJECT_CREATE),
        ("project-list-request.json", methods::PROJECT_LIST),
        ("project-get-request.json", methods::PROJECT_GET),
        ("project-update-request.json", methods::PROJECT_UPDATE),
        (
            "project-repository-associate-request.json",
            methods::PROJECT_REPOSITORY_ASSOCIATE,
        ),
    ] {
        let value = golden(file);
        let request: Request = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(request.method, method);
        assert_eq!(serde_json::to_value(request).unwrap(), value);
    }

    let create: ProjectCreateParams =
        serde_json::from_value(golden("project-create-request.json")["params"].clone()).unwrap();
    assert_eq!(create.name, "Cairn");
    let _: ProjectListParams =
        serde_json::from_value(golden("project-list-request.json")["params"].clone()).unwrap();
    let _: ProjectGetParams =
        serde_json::from_value(golden("project-get-request.json")["params"].clone()).unwrap();
    let _: ProjectUpdateParams =
        serde_json::from_value(golden("project-update-request.json")["params"].clone()).unwrap();
    let _: ProjectRepositoryAssociateParams = serde_json::from_value(
        golden("project-repository-associate-request.json")["params"].clone(),
    )
    .unwrap();

    for file in [
        "project-create-response.json",
        "project-create-retry-response.json",
        "project-list-response.json",
        "project-get-response.json",
        "project-update-response.json",
        "project-update-retry-response.json",
        "project-restore-response.json",
        "project-repository-associate-response.json",
        "project-repository-associate-retry-response.json",
    ] {
        let value = golden(file);
        let response: Response = serde_json::from_value(value.clone()).unwrap();
        assert!(response.error.is_none());
        assert_eq!(serde_json::to_value(response).unwrap(), value);
    }
}

#[test]
fn project_error_goldens_are_closed_typed_and_stable() {
    for (file, code) in [
        ("project-archived-error.json", ErrorCode::ProjectArchived),
        (
            "project-association-conflict-error.json",
            ErrorCode::RepositoryProjectConflict,
        ),
        (
            "project-idempotency-conflict-error.json",
            ErrorCode::IdempotencyConflict,
        ),
        (
            "project-idempotency-method-conflict-error.json",
            ErrorCode::IdempotencyConflict,
        ),
        ("project-invalid-request-error.json", ErrorCode::Usage),
    ] {
        let value = golden(file);
        let response: Response = serde_json::from_value(value.clone()).unwrap();
        let error = response.error.expect("error response");
        assert_eq!(error.code, code);
        assert_eq!(
            error.code.exit_code(),
            if code == ErrorCode::Usage { 2 } else { 1 }
        );
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

    assert!(serde_json::from_value::<ErrorData>(serde_json::json!({
        "kind":"project_archived",
        "project_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0a",
        "sql":"SELECT secret"
    }))
    .is_err());
    assert!(serde_json::from_value::<ErrorData>(serde_json::json!({
        "kind":"repository_already_associated",
        "repository_id":"repo-1",
        "existing_project_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0a",
        "requested_project_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0b",
        "path":"/private/repository"
    }))
    .is_err());
}

#[test]
fn project_request_validation_is_closed_and_bounded() {
    assert!(
        serde_json::from_value::<ProjectCreateParams>(serde_json::json!({
            "idempotency_key":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0c",
            "description":null
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ProjectRepositoryAssociateParams>(serde_json::json!({
            "idempotency_key":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0c",
            "project_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0a",
            "repository_id":"repo-1",
            "path":"/must-not-be-identity"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ProjectUpdateParams>(serde_json::json!({
            "idempotency_key":"not-a-uuid",
            "project_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0a"
        }))
        .is_err()
    );
}

#[test]
fn project_schema_compatibility_tripwires_are_present() {
    let rendered = serde_json::to_string(&schema_for!(ProjectRepositoryAssociateParams)).unwrap();
    assert!(rendered.contains("additionalProperties"));
    assert!(rendered.contains("repository_id"));
    assert!(!rendered.contains("canonical_path"));
    assert!(!rendered.contains("remote_url"));

    let list = serde_json::to_value(schema_for!(ProjectListParams)).unwrap();
    let status_ref = serde_json::to_string(&list).unwrap();
    assert!(status_ref.contains("active"));
    assert!(status_ref.contains("archived"));

    for method in [
        methods::PROJECT_CREATE,
        methods::PROJECT_LIST,
        methods::PROJECT_GET,
        methods::PROJECT_UPDATE,
        methods::PROJECT_REPOSITORY_ASSOCIATE,
    ] {
        assert!(methods::ALL_METHODS.contains(&method));
    }
}
