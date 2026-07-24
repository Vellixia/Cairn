use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

pub const GOAL_CONTRACT_SCHEMA_VERSION: u16 = 1;
const MAX_VIOLATIONS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GoalContractField {
    SchemaVersion,
    Goal,
    IncludedScope,
    ExcludedScope,
    AcceptanceCriteria,
    Constraints,
    GoalContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "violation", rename_all = "snake_case")]
pub enum GoalContractViolation {
    MissingRequiredField {
        field: GoalContractField,
    },
    MalformedStructure {
        field: GoalContractField,
    },
    EmptyGoal {
        field: GoalContractField,
    },
    EmptyListEntry {
        field: GoalContractField,
        index: u16,
    },
    UnsupportedVersion {
        version: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalContractError {
    violations: Vec<GoalContractViolation>,
}

impl GoalContractError {
    pub fn violations(&self) -> &[GoalContractViolation] {
        &self.violations
    }

    fn new(mut violations: Vec<GoalContractViolation>) -> Self {
        violations.truncate(MAX_VIOLATIONS);
        Self { violations }
    }
}

impl fmt::Display for GoalContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid goal contract")
    }
}

impl std::error::Error for GoalContractError {}

/// Validated, normalized, immutable version-one goal contract. Field order is
/// declaration order and therefore canonical JSON order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GoalContractV1 {
    schema_version: u16,
    goal: String,
    included_scope: Vec<String>,
    excluded_scope: Vec<String>,
    acceptance_criteria: Vec<String>,
    constraints: Vec<String>,
}

impl GoalContractV1 {
    pub fn new(
        goal: String,
        included_scope: Vec<String>,
        excluded_scope: Vec<String>,
        acceptance_criteria: Vec<String>,
        constraints: Vec<String>,
    ) -> Result<Self, GoalContractError> {
        Self::validate_parts(
            GOAL_CONTRACT_SCHEMA_VERSION,
            goal,
            included_scope,
            excluded_scope,
            acceptance_criteria,
            constraints,
        )
    }

    pub fn from_json_slice(input: &[u8]) -> Result<Self, GoalContractError> {
        let value: serde_json::Value = serde_json::from_slice(input).map_err(|_| {
            GoalContractError::new(vec![GoalContractViolation::MalformedStructure {
                field: GoalContractField::GoalContract,
            }])
        })?;
        Self::from_value(value)
    }

    pub fn from_value(value: serde_json::Value) -> Result<Self, GoalContractError> {
        let Some(object) = value.as_object() else {
            return Err(GoalContractError::new(vec![
                GoalContractViolation::MalformedStructure {
                    field: GoalContractField::GoalContract,
                },
            ]));
        };
        const REQUIRED: [(&str, GoalContractField); 6] = [
            ("schema_version", GoalContractField::SchemaVersion),
            ("goal", GoalContractField::Goal),
            ("included_scope", GoalContractField::IncludedScope),
            ("excluded_scope", GoalContractField::ExcludedScope),
            ("acceptance_criteria", GoalContractField::AcceptanceCriteria),
            ("constraints", GoalContractField::Constraints),
        ];
        let missing: Vec<_> = REQUIRED
            .iter()
            .filter(|(name, _)| !object.contains_key(*name))
            .map(|(_, field)| GoalContractViolation::MissingRequiredField { field: *field })
            .collect();
        if !missing.is_empty() {
            return Err(GoalContractError::new(missing));
        }
        if object.len() != REQUIRED.len() {
            return Err(GoalContractError::new(vec![
                GoalContractViolation::MalformedStructure {
                    field: GoalContractField::GoalContract,
                },
            ]));
        }

        let Some(version_u64) = object["schema_version"].as_u64() else {
            return Err(malformed());
        };
        let Ok(version) = u16::try_from(version_u64) else {
            return Err(malformed());
        };
        if version != GOAL_CONTRACT_SCHEMA_VERSION {
            return Err(GoalContractError::new(vec![
                GoalContractViolation::UnsupportedVersion { version },
            ]));
        }
        let Some(goal) = object["goal"].as_str() else {
            return Err(malformed());
        };
        let included_scope = string_list(&object["included_scope"])?;
        let excluded_scope = string_list(&object["excluded_scope"])?;
        let acceptance_criteria = string_list(&object["acceptance_criteria"])?;
        let constraints = string_list(&object["constraints"])?;
        Self::validate_parts(
            version,
            goal.to_string(),
            included_scope,
            excluded_scope,
            acceptance_criteria,
            constraints,
        )
    }

    fn validate_parts(
        schema_version: u16,
        goal: String,
        included_scope: Vec<String>,
        excluded_scope: Vec<String>,
        acceptance_criteria: Vec<String>,
        constraints: Vec<String>,
    ) -> Result<Self, GoalContractError> {
        if schema_version != GOAL_CONTRACT_SCHEMA_VERSION {
            return Err(GoalContractError::new(vec![
                GoalContractViolation::UnsupportedVersion {
                    version: schema_version,
                },
            ]));
        }
        let goal = normalize(goal);
        let included_scope = normalize_list(included_scope);
        let excluded_scope = normalize_list(excluded_scope);
        let acceptance_criteria = normalize_list(acceptance_criteria);
        let constraints = normalize_list(constraints);
        let mut violations = Vec::new();
        if goal.is_empty() {
            violations.push(GoalContractViolation::EmptyGoal {
                field: GoalContractField::Goal,
            });
        }
        collect_empty(
            &mut violations,
            GoalContractField::IncludedScope,
            &included_scope,
        );
        collect_empty(
            &mut violations,
            GoalContractField::ExcludedScope,
            &excluded_scope,
        );
        collect_empty(
            &mut violations,
            GoalContractField::AcceptanceCriteria,
            &acceptance_criteria,
        );
        collect_empty(
            &mut violations,
            GoalContractField::Constraints,
            &constraints,
        );
        if !violations.is_empty() {
            return Err(GoalContractError::new(violations));
        }
        Ok(Self {
            schema_version,
            goal,
            included_scope,
            excluded_scope,
            acceptance_criteria,
            constraints,
        })
    }

    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub fn goal(&self) -> &str {
        &self.goal
    }

    pub fn included_scope(&self) -> &[String] {
        &self.included_scope
    }

    pub fn excluded_scope(&self) -> &[String] {
        &self.excluded_scope
    }

    pub fn acceptance_criteria(&self) -> &[String] {
        &self.acceptance_criteria
    }

    pub fn constraints(&self) -> &[String] {
        &self.constraints
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("validated goal contract serializes")
    }

    pub fn fingerprint(&self) -> String {
        blake3::hash(&self.canonical_bytes()).to_hex().to_string()
    }
}

impl<'de> Deserialize<'de> for GoalContractV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        Self::from_value(value).map_err(serde::de::Error::custom)
    }
}

fn malformed() -> GoalContractError {
    GoalContractError::new(vec![GoalContractViolation::MalformedStructure {
        field: GoalContractField::GoalContract,
    }])
}

fn string_list(value: &serde_json::Value) -> Result<Vec<String>, GoalContractError> {
    let Some(items) = value.as_array() else {
        return Err(malformed());
    };
    items
        .iter()
        .map(|item| item.as_str().map(str::to_string).ok_or_else(malformed))
        .collect()
}

fn normalize(value: String) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_string()
}

fn normalize_list(values: Vec<String>) -> Vec<String> {
    values.into_iter().map(normalize).collect()
}

fn collect_empty(
    violations: &mut Vec<GoalContractViolation>,
    field: GoalContractField,
    values: &[String],
) {
    for (index, value) in values.iter().enumerate() {
        if value.is_empty() && violations.len() < MAX_VIOLATIONS {
            violations.push(GoalContractViolation::EmptyListEntry {
                field,
                index: u16::try_from(index.min(999)).expect("bounded index"),
            });
        }
    }
}
