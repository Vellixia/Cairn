use std::path::PathBuf;

use cairn_protocol::{
    methods, EventAggregateType, EventsListParams, EventsListResult, Request, Response,
};
use schemars::schema_for;

fn golden(name: &str) -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("goldens/ipc")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).expect("read events golden"))
        .expect("valid events golden")
}

#[test]
fn aggregate_filter_request_and_results_are_typed_and_stable() {
    let request_value = golden("events-list-aggregate-request.json");
    let request: Request = serde_json::from_value(request_value.clone()).unwrap();
    assert_eq!(request.method, methods::EVENTS_LIST);
    let params: EventsListParams = serde_json::from_value(request.params).unwrap();
    assert_eq!(params.aggregate_type, Some(EventAggregateType::Project));
    assert_eq!(
        params.aggregate_id.as_deref(),
        Some("018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0a")
    );

    let response: Response =
        serde_json::from_value(golden("events-list-aggregate-response.json")).unwrap();
    let result: EventsListResult = serde_json::from_value(response.result.unwrap()).unwrap();
    assert_eq!(result.events.len(), 1);
    assert_eq!(
        result.events[0].aggregate_type,
        Some(EventAggregateType::Project)
    );
    assert_eq!(result.events[0].aggregate_seq, Some(1));

    let response: Response =
        serde_json::from_value(golden("events-list-legacy-null-response.json")).unwrap();
    let result: EventsListResult = serde_json::from_value(response.result.unwrap()).unwrap();
    let event = &result.events[0];
    assert!(event.aggregate_type.is_none());
    assert!(event.aggregate_id.is_none());
    assert!(event.aggregate_seq.is_none());
}

#[test]
fn incomplete_aggregate_pair_has_one_typed_usage_error() {
    let response: Response =
        serde_json::from_value(golden("events-list-incomplete-aggregate-error.json")).unwrap();
    let error = response.error.expect("error response");
    assert_eq!(error.code, cairn_protocol::ErrorCode::Usage);
    assert_eq!(error.code.exit_code(), 2);
    assert!(error.data.is_none());

    for value in [
        serde_json::json!({"aggregate_type":"project"}),
        serde_json::json!({"aggregate_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0a"}),
    ] {
        let params: EventsListParams = serde_json::from_value(value).unwrap();
        assert_ne!(
            params.aggregate_type.is_some(),
            params.aggregate_id.is_some()
        );
    }
}

#[test]
fn aggregate_contract_is_closed_and_legacy_fields_remain_nullable() {
    assert!(
        serde_json::from_value::<EventsListParams>(serde_json::json!({
            "aggregate_type":"organization",
            "aggregate_id":"id"
        }))
        .is_err()
    );

    let params_schema = serde_json::to_string(&schema_for!(EventsListParams)).unwrap();
    let result_schema = serde_json::to_string(&schema_for!(EventsListResult)).unwrap();
    for token in ["repository", "worktree", "session", "project", "task"] {
        assert!(params_schema.contains(token));
    }
    for field in ["aggregate_type", "aggregate_id", "aggregate_seq"] {
        assert!(result_schema.contains(field));
    }
}
