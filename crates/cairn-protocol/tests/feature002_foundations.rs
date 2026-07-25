use cairn_domain::{GoalContractV1, ProjectId, SessionBindingMode, TaskRevisionId};
use cairn_protocol::{
    CandidateIds, ErrorData, GoalContractViolations, ProjectDto, SessionScopeDto, TaskDto,
};
use schemars::schema_for;
use serde_json::json;

#[test]
fn shared_project_task_and_scope_dtos_are_typed() {
    let project_schema = serde_json::to_value(schema_for!(ProjectDto)).unwrap();
    let task_schema = serde_json::to_value(schema_for!(TaskDto)).unwrap();
    let scope_schema = serde_json::to_value(schema_for!(SessionScopeDto)).unwrap();
    assert!(project_schema.to_string().contains("project_id"));
    assert!(task_schema.to_string().contains("latest_revision_number"));
    assert!(scope_schema.to_string().contains("project_bound"));

    let domain = SessionBindingMode::ProjectBound {
        project_id: ProjectId::new_v7(),
        task_revision_id: TaskRevisionId::new_v7(),
    };
    let dto = SessionScopeDto::from(domain);
    assert_eq!(SessionBindingMode::from(dto), domain);
}

#[test]
fn error_data_rejects_unknown_fields_and_unbounded_arrays() {
    let candidates: Vec<_> = (0..21).map(|index| format!("candidate-{index}")).collect();
    assert!(serde_json::from_value::<ErrorData>(json!({
        "kind": "ambiguous_name",
        "entity": "project",
        "candidate_ids": candidates,
        "truncated": true
    }))
    .is_err());
    assert!(serde_json::from_value::<ErrorData>(json!({
        "kind": "storage_busy",
        "max_elapsed_ms": 5000,
        "raw_sql": "secret"
    }))
    .is_err());

    assert!(CandidateIds::new(vec!["one".into(); 20]).is_ok());
    assert!(CandidateIds::new(vec!["one".into(); 21]).is_err());
    assert!(GoalContractViolations::new(vec![]).is_err());
}

#[test]
fn goal_contract_errors_never_echo_contract_content() {
    let sensitive = "do-not-log-this-goal";
    let error = GoalContractV1::from_value(json!({
        "schema_version": 1,
        "goal": sensitive,
        "included_scope": [""],
        "excluded_scope": [],
        "acceptance_criteria": [],
        "constraints": []
    }))
    .unwrap_err();
    let violations = GoalContractViolations::new(error.violations().to_vec()).unwrap();
    let data = ErrorData::InvalidGoalContract { violations };
    let rendered = serde_json::to_string(&data).unwrap();
    assert!(!rendered.contains(sensitive));
    assert!(rendered.contains("empty_list_entry"));
}
