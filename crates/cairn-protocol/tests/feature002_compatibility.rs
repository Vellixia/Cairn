use std::collections::BTreeSet;

use cairn_protocol::*;
use schemars::schema_for;

#[test]
fn feature002_method_surface_is_closed_unique_and_schema_backed() {
    let mut all = BTreeSet::new();
    for method in methods::ALL_METHODS {
        assert!(all.insert(*method), "duplicate method {method}");
    }
    let expected: BTreeSet<_> = [
        methods::PROJECT_CREATE,
        methods::PROJECT_LIST,
        methods::PROJECT_GET,
        methods::PROJECT_UPDATE,
        methods::PROJECT_REPOSITORY_ASSOCIATE,
        methods::TASK_CREATE,
        methods::TASK_REVISE,
        methods::TASK_LIST,
        methods::TASK_GET,
        methods::SESSION_BIND,
        methods::SESSION_START,
        methods::SESSION_GET,
        methods::SESSION_LIST,
        methods::EVENTS_LIST,
    ]
    .into_iter()
    .collect();
    let inventoried: BTreeSet<_> = methods::FEATURE002_METHODS
        .iter()
        .map(|surface| surface.method)
        .collect();
    assert_eq!(inventoried, expected);
}

#[test]
fn ids_status_scope_lifecycle_and_revision_shapes_cannot_widen_silently() {
    assert_eq!(ProjectId::new_v7().0.get_version_num(), 7);
    assert_eq!(TaskId::new_v7().0.get_version_num(), 7);
    assert_eq!(TaskRevisionId::new_v7().0.get_version_num(), 7);

    let status = serde_json::to_string(&schema_for!(ProjectStatus)).unwrap();
    assert!(status.contains("active") && status.contains("archived"));
    assert!(!status.contains("deleted"));

    let scope = serde_json::to_string(&schema_for!(SessionScopeDto)).unwrap();
    assert!(scope.contains("local_unbound") && scope.contains("project_bound"));
    for field in ["project_id", "task_revision_id"] {
        assert!(scope.contains(field));
    }
    let lifecycle = serde_json::to_string(&schema_for!(SessionState)).unwrap();
    for state in ["active", "recovering", "stopped", "interrupted"] {
        assert!(lifecycle.contains(state));
    }
    assert!(!lifecycle.contains("project_bound"));

    let revision = serde_json::to_string(&schema_for!(TaskRevisionDto)).unwrap();
    for required in [
        "revision_id",
        "task_id",
        "revision_number",
        "parent_revision_id",
        "goal_contract",
        "goal_contract_fingerprint",
        "created_at",
    ] {
        assert!(revision.contains(required));
    }
    let task = serde_json::to_string(&schema_for!(TaskDto)).unwrap();
    assert!(task.contains("latest_revision_number"));
    assert!(task.contains("updated_at"));
}

#[test]
fn goal_errors_ambiguity_and_typed_error_data_are_closed_and_bounded() {
    let error = serde_json::to_string(&schema_for!(ErrorData)).unwrap();
    for variant in [
        "missing_required_field",
        "malformed_structure",
        "empty_goal",
        "empty_list_entry",
        "unsupported_version",
    ] {
        assert!(error.contains(variant));
    }
    let candidates = CandidateIds::new(
        (0..CandidateIds::MAX)
            .map(|index| format!("{index:032}"))
            .collect(),
    )
    .unwrap();
    assert_eq!(candidates.as_slice().len(), 20);
    assert!(CandidateIds::new(
        (0..=CandidateIds::MAX)
            .map(|index| index.to_string())
            .collect()
    )
    .is_err());

    assert!(serde_json::from_value::<ErrorData>(serde_json::json!({
        "kind":"project_scope_required",
        "repository_id":"repository-1",
        "project_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0a",
        "internal_path":"/private/secret"
    }))
    .is_err());
    assert!(serde_json::from_value::<ErrorData>(serde_json::json!({
        "kind":"storage_busy",
        "max_elapsed_ms":5000,
        "sql":"BEGIN IMMEDIATE"
    }))
    .is_err());
}

#[test]
fn legacy_scope_omission_and_explicit_scopes_remain_compatible() {
    let base = serde_json::json!({
        "path":"/repo",
        "agent_type":"codex",
        "agent_instance_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0a"
    });
    let omitted: SessionStartParams = serde_json::from_value(base.clone()).unwrap();
    assert!(omitted.scope.is_none());

    let mut explicit = base.clone();
    explicit["scope"] = serde_json::json!({"mode":"local_unbound"});
    let explicit: SessionStartParams = serde_json::from_value(explicit).unwrap();
    assert!(matches!(
        explicit.scope,
        Some(SessionScopeDto::LocalUnbound)
    ));

    let mut bound = base;
    bound["scope"] = serde_json::json!({
        "mode":"project_bound",
        "project_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0b",
        "task_revision_id":"018f4e6e-5f2b-7c3e-9a4d-2f6e8b1c9d0c"
    });
    let bound: SessionStartParams = serde_json::from_value(bound).unwrap();
    assert!(matches!(
        bound.scope,
        Some(SessionScopeDto::ProjectBound { .. })
    ));
}

#[test]
fn arbitrary_json_is_confined_to_legacy_envelopes_and_event_payloads() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut occurrences = Vec::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            for (line, value) in text.lines().enumerate() {
                if value.contains("serde_json::Value") {
                    occurrences.push((path.file_name().unwrap().to_owned(), line + 1));
                }
            }
        }
    }
    assert_eq!(
        occurrences.len(),
        4,
        "new arbitrary JSON escape hatch: {occurrences:?}"
    );
}
