use std::path::PathBuf;

use cairn_protocol::*;
use schemars::schema_for;

type ResultParser = fn(serde_json::Value);

fn golden(name: &str) -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("goldens")
        .join("ipc")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).expect("read task IPC golden"))
        .expect("valid task IPC golden")
}

#[test]
fn task_requests_and_success_results_match_goldens() {
    for (file, method) in [
        ("task-create-request.json", methods::TASK_CREATE),
        ("task-revise-request.json", methods::TASK_REVISE),
        ("task-list-request.json", methods::TASK_LIST),
        ("task-get-request.json", methods::TASK_GET),
        ("task-get-historical-request.json", methods::TASK_GET),
    ] {
        let value = golden(file);
        let request: Request = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(request.method, method);
        assert_eq!(serde_json::to_value(request).unwrap(), value);
    }

    let _: TaskCreateParams =
        serde_json::from_value(golden("task-create-request.json")["params"].clone()).unwrap();
    let _: TaskReviseParams =
        serde_json::from_value(golden("task-revise-request.json")["params"].clone()).unwrap();
    let _: TaskListParams =
        serde_json::from_value(golden("task-list-request.json")["params"].clone()).unwrap();
    let _: TaskGetParams =
        serde_json::from_value(golden("task-get-request.json")["params"].clone()).unwrap();
    let historical: TaskGetParams =
        serde_json::from_value(golden("task-get-historical-request.json")["params"].clone())
            .unwrap();
    assert!(historical.revision_id.is_some());

    let cases: [(&str, ResultParser); 5] = [
        ("task-create-response.json", |value| {
            let _: TaskCreateResult = serde_json::from_value(value).unwrap();
        }),
        ("task-revise-response.json", |value| {
            let _: TaskReviseResult = serde_json::from_value(value).unwrap();
        }),
        ("task-create-retry-response.json", |value| {
            let result: TaskCreateResult = serde_json::from_value(value).unwrap();
            assert!(result.created);
        }),
        ("task-list-response.json", |value| {
            let _: TaskListResult = serde_json::from_value(value).unwrap();
        }),
        ("task-get-historical-response.json", |value| {
            let result: TaskGetResult = serde_json::from_value(value).unwrap();
            assert_eq!(result.revision.revision_number, 1);
        }),
    ];
    for (file, parse) in cases {
        let value = golden(file);
        let response: Response = serde_json::from_value(value.clone()).unwrap();
        assert!(response.error.is_none());
        parse(response.result.clone().unwrap());
        assert_eq!(serde_json::to_value(response).unwrap(), value);
    }
}

#[test]
fn task_error_goldens_are_closed_typed_private_and_stable() {
    for (file, code) in [
        ("task-archived-error.json", ErrorCode::ProjectArchived),
        (
            "task-parent-conflict-error.json",
            ErrorCode::TaskRevisionConflict,
        ),
        (
            "task-idempotency-conflict-error.json",
            ErrorCode::IdempotencyConflict,
        ),
    ] {
        let value = golden(file);
        let response: Response = serde_json::from_value(value.clone()).unwrap();
        let error = response.error.expect("error response");
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

    let serialized = serde_json::to_string(&golden("task-goal-contract-errors.json")).unwrap();
    for forbidden in [
        "private-goal-sentinel",
        "/private/",
        "SELECT ",
        "resume-token",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    let cases = golden("task-goal-contract-errors.json")
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(cases.len(), 13);
    let mut missing_fields = std::collections::BTreeSet::new();
    let mut empty_lists = std::collections::BTreeSet::new();
    let mut saw_malformed = false;
    let mut saw_empty_goal = false;
    let mut saw_unsupported = false;
    for value in cases {
        let response: Response = serde_json::from_value(value).unwrap();
        let error = response.error.unwrap();
        assert_eq!(error.code, ErrorCode::InvalidGoalContract);
        assert_eq!(error.code.exit_code(), 1);
        let ErrorData::InvalidGoalContract { violations } = error.data.unwrap() else {
            panic!("typed goal-contract error data required");
        };
        for violation in violations.as_slice() {
            match violation {
                GoalContractViolation::MissingRequiredField { field } => {
                    missing_fields.insert(format!("{field:?}"));
                }
                GoalContractViolation::MalformedStructure { .. } => saw_malformed = true,
                GoalContractViolation::EmptyGoal { .. } => saw_empty_goal = true,
                GoalContractViolation::EmptyListEntry { field, .. } => {
                    empty_lists.insert(format!("{field:?}"));
                }
                GoalContractViolation::UnsupportedVersion { .. } => saw_unsupported = true,
            }
        }
    }
    assert_eq!(missing_fields.len(), 6);
    assert_eq!(empty_lists.len(), 4);
    assert!(saw_malformed && saw_empty_goal && saw_unsupported);

    assert!(serde_json::from_value::<ErrorData>(serde_json::json!({
        "kind":"invalid_goal_contract",
        "violations":[{"violation":"empty_goal","field":"goal"}],
        "goal_contract":{"goal":"private-goal-sentinel"}
    }))
    .is_err());
}

#[test]
fn task_request_validation_and_schema_compatibility_tripwires_are_closed() {
    assert!(
        serde_json::from_value::<TaskCreateParams>(serde_json::json!({
            "idempotency_key":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0e",
            "project_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0a",
            "title":"Task",
            "goal_contract":{
                "schema_version":1,"goal":"Goal","included_scope":[],"excluded_scope":[],
                "acceptance_criteria":[],"constraints":[]
            },
            "raw_request_body":"must be rejected"
        }))
        .is_err()
    );

    let create_schema = serde_json::to_string(&schema_for!(TaskCreateParams)).unwrap();
    for required in [
        "schema_version",
        "goal",
        "included_scope",
        "excluded_scope",
        "acceptance_criteria",
        "constraints",
    ] {
        assert!(create_schema.contains(required));
    }
    let error_schema = serde_json::to_string(&schema_for!(ErrorData)).unwrap();
    for violation in [
        "missing_required_field",
        "malformed_structure",
        "empty_goal",
        "empty_list_entry",
        "unsupported_version",
    ] {
        assert!(error_schema.contains(violation));
    }
    for method in [
        methods::TASK_CREATE,
        methods::TASK_REVISE,
        methods::TASK_LIST,
        methods::TASK_GET,
    ] {
        assert!(methods::ALL_METHODS.contains(&method));
    }
}
