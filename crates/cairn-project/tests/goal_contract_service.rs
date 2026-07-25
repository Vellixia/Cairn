use cairn_domain::{GoalContractField, GoalContractViolation};
use cairn_project::{parse_goal_contract_json, ProjectTaskError};
use serde_json::json;

fn valid() -> serde_json::Value {
    json!({
        "schema_version": 1,
        "goal": "\u{2003}Goal\r\nline\u{2003}",
        "included_scope": [" first ", "second"],
        "excluded_scope": [],
        "acceptance_criteria": ["accept"],
        "constraints": ["constraint"]
    })
}

fn violations(value: serde_json::Value) -> Vec<GoalContractViolation> {
    match parse_goal_contract_json(&serde_json::to_vec(&value).unwrap()).unwrap_err() {
        ProjectTaskError::InvalidGoalContract { violations } => violations,
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn canonicalization_and_fingerprint_are_deterministic_and_order_sensitive() {
    let first = parse_goal_contract_json(&serde_json::to_vec(&valid()).unwrap()).unwrap();
    let mut equivalent = valid();
    equivalent["goal"] = json!("Goal\nline");
    let second = parse_goal_contract_json(&serde_json::to_vec(&equivalent).unwrap()).unwrap();
    assert_eq!(first.canonical_bytes(), second.canonical_bytes());
    assert_eq!(first.fingerprint(), second.fingerprint());
    assert_eq!(first.goal(), "Goal\nline");

    let mut reordered = equivalent;
    reordered["included_scope"] = json!(["second", "first"]);
    let reordered = parse_goal_contract_json(&serde_json::to_vec(&reordered).unwrap()).unwrap();
    assert_ne!(first.canonical_bytes(), reordered.canonical_bytes());
    assert_ne!(first.fingerprint(), reordered.fingerprint());
}

#[test]
fn every_closed_goal_contract_violation_is_preserved_without_contract_content() {
    let fields = [
        ("schema_version", GoalContractField::SchemaVersion),
        ("goal", GoalContractField::Goal),
        ("included_scope", GoalContractField::IncludedScope),
        ("excluded_scope", GoalContractField::ExcludedScope),
        ("acceptance_criteria", GoalContractField::AcceptanceCriteria),
        ("constraints", GoalContractField::Constraints),
    ];
    for (field, expected) in fields {
        let mut input = valid();
        input.as_object_mut().unwrap().remove(field);
        assert_eq!(
            violations(input),
            vec![GoalContractViolation::MissingRequiredField { field: expected }]
        );
    }

    assert_eq!(
        parse_goal_contract_json(b"not-json").unwrap_err(),
        ProjectTaskError::InvalidGoalContract {
            violations: vec![GoalContractViolation::MalformedStructure {
                field: GoalContractField::GoalContract,
            }],
        }
    );
    assert!(matches!(
        violations(json!([])).as_slice(),
        [GoalContractViolation::MalformedStructure { .. }]
    ));

    let mut empty_goal = valid();
    empty_goal["goal"] = json!(" \r\n ");
    assert!(matches!(
        violations(empty_goal).as_slice(),
        [GoalContractViolation::EmptyGoal { .. }]
    ));

    for field in [
        "included_scope",
        "excluded_scope",
        "acceptance_criteria",
        "constraints",
    ] {
        let mut input = valid();
        input[field] = json!(["sentinel-private-contract", " \r "]);
        let error = violations(input);
        assert!(matches!(
            error.as_slice(),
            [GoalContractViolation::EmptyListEntry { index: 1, .. }]
        ));
        assert!(!format!("{error:?}").contains("sentinel-private-contract"));
    }

    let mut unsupported = valid();
    unsupported["schema_version"] = json!(2);
    assert_eq!(
        violations(unsupported),
        vec![GoalContractViolation::UnsupportedVersion { version: 2 }]
    );

    let mut many = valid();
    many["constraints"] = json!(vec![""; 40]);
    assert_eq!(violations(many).len(), 32);
}
