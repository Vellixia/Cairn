use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cairn_protocol::{ErrorCode, ErrorData, GoalContractViolation, Response};

const GOLDENS: &[(&str, ErrorCode)] = &[
    ("project-not-found.json", ErrorCode::ProjectNotFound),
    ("project-archived.json", ErrorCode::ProjectArchived),
    (
        "project-scope-required.json",
        ErrorCode::ProjectScopeRequired,
    ),
    ("invalid-project.json", ErrorCode::InvalidProject),
    ("task-not-found.json", ErrorCode::TaskNotFound),
    ("invalid-task.json", ErrorCode::InvalidTask),
    (
        "task-revision-not-found.json",
        ErrorCode::TaskRevisionNotFound,
    ),
    (
        "task-revision-conflict.json",
        ErrorCode::TaskRevisionConflict,
    ),
    (
        "repository-not-associated.json",
        ErrorCode::RepositoryNotAssociated,
    ),
    (
        "repository-project-conflict.json",
        ErrorCode::RepositoryProjectConflict,
    ),
    (
        "task-revision-project-mismatch.json",
        ErrorCode::TaskRevisionProjectMismatch,
    ),
    (
        "session-binding-conflict.json",
        ErrorCode::SessionBindingConflict,
    ),
    (
        "session-scope-conflict.json",
        ErrorCode::SessionScopeConflict,
    ),
    ("ambiguous-name.json", ErrorCode::AmbiguousName),
    ("invalid-goal-missing.json", ErrorCode::InvalidGoalContract),
    (
        "invalid-goal-malformed.json",
        ErrorCode::InvalidGoalContract,
    ),
    ("invalid-goal-empty.json", ErrorCode::InvalidGoalContract),
    (
        "invalid-goal-list-entry.json",
        ErrorCode::InvalidGoalContract,
    ),
    ("invalid-goal-version.json", ErrorCode::InvalidGoalContract),
    ("idempotency-method.json", ErrorCode::IdempotencyConflict),
    ("idempotency-request.json", ErrorCode::IdempotencyConflict),
    ("storage-busy.json", ErrorCode::StorageBusy),
    ("migration-failed.json", ErrorCode::MigrationFailed),
    ("invalid-request.json", ErrorCode::Usage),
];

fn errors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("goldens/errors")
}

fn read(path: &Path) -> (serde_json::Value, Response) {
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("read error golden"))
            .expect("valid error golden JSON");
    let response = serde_json::from_value(value.clone()).expect("typed error response");
    (value, response)
}

#[test]
fn every_feature002_code_has_a_canonical_typed_private_golden() {
    let expected: BTreeSet<_> = ErrorCode::FEATURE002_CODES
        .iter()
        .map(|code| serde_json::to_string(code).unwrap())
        .collect();
    let mut covered = BTreeSet::new();

    for (file, expected_code) in GOLDENS {
        let path = errors_dir().join(file);
        let (value, response) = read(&path);
        assert!(response.result.is_none(), "{file} exposed a result");
        let error = response.error.expect("error body");
        assert_eq!(&error.code, expected_code, "{file}");
        if let Some(data) = error.data.as_ref() {
            assert!(
                error.code.accepts_data(data),
                "wrong code/data pairing in {file}"
            );
            covered.insert(serde_json::to_string(&error.code).unwrap());
        } else {
            assert_eq!(error.code, ErrorCode::Usage, "typed data missing in {file}");
        }
        assert_eq!(
            serde_json::to_value(Response {
                id: response.id,
                result: None,
                error: Some(error),
            })
            .unwrap(),
            value
        );

        let serialized = serde_json::to_string(&value).unwrap();
        for forbidden in [
            "SELECT ",
            "BEGIN IMMEDIATE",
            "/private/",
            "/Users/",
            "resume_token",
            "CAIRN_SECRET",
            "private-goal-sentinel",
            "raw_request",
            "backtrace",
            "source_error",
            "token-value",
        ] {
            assert!(!serialized.contains(forbidden), "{file} leaked {forbidden}");
        }
    }
    assert_eq!(covered, expected);

    let actual: BTreeSet<_> = std::fs::read_dir(errors_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let inventoried: BTreeSet<_> = GOLDENS.iter().map(|(file, _)| (*file).to_owned()).collect();
    assert_eq!(
        actual, inventoried,
        "unregistered or missing canonical error golden"
    );
}

#[test]
fn every_goal_violation_and_idempotency_reason_is_covered() {
    let mut violations = BTreeSet::new();
    let mut idempotency_reasons = BTreeSet::new();
    for (file, _) in GOLDENS {
        let (_, response) = read(&errors_dir().join(file));
        match response.error.unwrap().data {
            Some(ErrorData::InvalidGoalContract { violations: values }) => {
                for value in values.as_slice() {
                    violations.insert(match value {
                        GoalContractViolation::MissingRequiredField { .. } => {
                            "missing_required_field"
                        }
                        GoalContractViolation::MalformedStructure { .. } => "malformed_structure",
                        GoalContractViolation::EmptyGoal { .. } => "empty_goal",
                        GoalContractViolation::EmptyListEntry { .. } => "empty_list_entry",
                        GoalContractViolation::UnsupportedVersion { .. } => "unsupported_version",
                    });
                }
            }
            Some(ErrorData::IdempotencyConflict { reason, .. }) => {
                idempotency_reasons.insert(format!("{reason:?}"));
            }
            _ => {}
        }
    }
    assert_eq!(violations.len(), 5);
    assert_eq!(idempotency_reasons.len(), 2);
}
