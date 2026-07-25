use cairn_domain::GoalContractV1;

use crate::ProjectTaskError;

/// Parses a goal contract at the application boundary while preserving only
/// the bounded typed violations. Submitted contract content is never copied
/// into the returned error.
pub fn parse_goal_contract_json(input: &[u8]) -> Result<GoalContractV1, ProjectTaskError> {
    GoalContractV1::from_json_slice(input).map_err(|error| ProjectTaskError::InvalidGoalContract {
        violations: error.violations().to_vec(),
    })
}
