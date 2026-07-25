use cairn_domain::{
    GoalContractField, GoalContractV1, GoalContractViolation, GOAL_CONTRACT_SCHEMA_VERSION,
};
use serde_json::json;

fn valid_value() -> serde_json::Value {
    json!({
        "schema_version": 1,
        "goal": "  Build\r\nthis  ",
        "included_scope": [" first ", "second\rline"],
        "excluded_scope": [],
        "acceptance_criteria": [" passes "],
        "constraints": []
    })
}

#[test]
fn canonicalization_normalizes_only_edges_and_line_endings() {
    let contract = GoalContractV1::from_value(valid_value()).unwrap();
    assert_eq!(contract.schema_version(), GOAL_CONTRACT_SCHEMA_VERSION);
    assert_eq!(contract.goal(), "Build\nthis");
    assert_eq!(contract.included_scope(), ["first", "second\nline"]);
    assert_eq!(contract.acceptance_criteria(), ["passes"]);
    assert!(contract.excluded_scope().is_empty());
    assert!(contract.constraints().is_empty());
    assert_eq!(
        String::from_utf8(contract.canonical_bytes()).unwrap(),
        r#"{"schema_version":1,"goal":"Build\nthis","included_scope":["first","second\nline"],"excluded_scope":[],"acceptance_criteria":["passes"],"constraints":[]}"#
    );
    assert_eq!(contract.fingerprint().len(), 64);
    assert!(contract
        .fingerprint()
        .chars()
        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
}

#[test]
fn list_order_changes_the_fingerprint_and_is_never_sorted() {
    let first = GoalContractV1::new(
        "Goal".into(),
        vec!["a".into(), "b".into()],
        vec![],
        vec![],
        vec![],
    )
    .unwrap();
    let second = GoalContractV1::new(
        "Goal".into(),
        vec!["b".into(), "a".into()],
        vec![],
        vec![],
        vec![],
    )
    .unwrap();
    assert_eq!(first.included_scope(), ["a", "b"]);
    assert_ne!(first.fingerprint(), second.fingerprint());
}

#[test]
fn every_required_field_has_a_bounded_missing_violation() {
    let fields = [
        ("schema_version", GoalContractField::SchemaVersion),
        ("goal", GoalContractField::Goal),
        ("included_scope", GoalContractField::IncludedScope),
        ("excluded_scope", GoalContractField::ExcludedScope),
        ("acceptance_criteria", GoalContractField::AcceptanceCriteria),
        ("constraints", GoalContractField::Constraints),
    ];
    for (name, field) in fields {
        let mut value = valid_value();
        value.as_object_mut().unwrap().remove(name);
        let error = GoalContractV1::from_value(value).unwrap_err();
        assert_eq!(
            error.violations(),
            &[GoalContractViolation::MissingRequiredField { field }]
        );
        assert_eq!(error.to_string(), "invalid goal contract");
    }
}

#[test]
fn malformed_empty_and_unsupported_inputs_use_the_closed_union() {
    let malformed = GoalContractV1::from_json_slice(br#"{"goal":]"#).unwrap_err();
    assert!(matches!(
        malformed.violations(),
        [GoalContractViolation::MalformedStructure { .. }]
    ));

    let mut empty_goal = valid_value();
    empty_goal["goal"] = json!(" \u{2003} ");
    assert!(matches!(
        GoalContractV1::from_value(empty_goal)
            .unwrap_err()
            .violations(),
        [GoalContractViolation::EmptyGoal { .. }]
    ));

    let mut unsupported = valid_value();
    unsupported["schema_version"] = json!(2);
    assert_eq!(
        GoalContractV1::from_value(unsupported)
            .unwrap_err()
            .violations(),
        &[GoalContractViolation::UnsupportedVersion { version: 2 }]
    );
}

#[test]
fn every_list_rejects_empty_entries_with_only_field_and_index() {
    let fields = [
        ("included_scope", GoalContractField::IncludedScope),
        ("excluded_scope", GoalContractField::ExcludedScope),
        ("acceptance_criteria", GoalContractField::AcceptanceCriteria),
        ("constraints", GoalContractField::Constraints),
    ];
    for (name, field) in fields {
        let mut value = valid_value();
        value[name] = json!(["ok", " \r\n "]);
        let error = GoalContractV1::from_value(value).unwrap_err();
        assert_eq!(
            error.violations(),
            &[GoalContractViolation::EmptyListEntry { field, index: 1 }]
        );
        assert!(!format!("{error:?}").contains("ok"));
    }
}

#[test]
fn violation_output_is_capped_and_empty_lists_are_valid() {
    GoalContractV1::new("Goal".into(), vec![], vec![], vec![], vec![]).unwrap();
    let mut value = valid_value();
    value["included_scope"] = json!(vec![""; 40]);
    let error = GoalContractV1::from_value(value).unwrap_err();
    assert_eq!(error.violations().len(), 32);
}
