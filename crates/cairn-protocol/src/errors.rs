//! Closed error-code set (contracts/ipc-contract.md) plus CLI-only codes.

use cairn_domain::{
    GoalContractViolation, IdempotencyKey, ProjectId, TaskId, TaskRevisionId, WatcherStartStage,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CandidateIds(Vec<String>);

impl CandidateIds {
    pub const MAX: usize = 20;

    pub fn new(values: Vec<String>) -> Result<Self, &'static str> {
        if values.len() > Self::MAX {
            return Err("candidate id list exceeds bound");
        }
        Ok(Self(values))
    }

    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for CandidateIds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values = Vec::<String>::deserialize(deserializer)?;
        Self::new(values).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for CandidateIds {
    fn schema_name() -> String {
        "CandidateIds".into()
    }

    fn json_schema(generator: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = <Vec<String>>::json_schema(generator).into_object();
        if let Some(array) = schema.array.as_mut() {
            array.max_items = Some(Self::MAX as u32);
        }
        schemars::schema::Schema::Object(schema)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct GoalContractViolations(Vec<GoalContractViolation>);

impl GoalContractViolations {
    pub const MIN: usize = 1;
    pub const MAX: usize = 32;

    pub fn new(values: Vec<GoalContractViolation>) -> Result<Self, &'static str> {
        if !(Self::MIN..=Self::MAX).contains(&values.len()) {
            return Err("goal contract violation list is outside bounds");
        }
        Ok(Self(values))
    }

    pub fn as_slice(&self) -> &[GoalContractViolation] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for GoalContractViolations {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values = Vec::<GoalContractViolation>::deserialize(deserializer)?;
        Self::new(values).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for GoalContractViolations {
    fn schema_name() -> String {
        "GoalContractViolations".into()
    }

    fn json_schema(generator: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = <Vec<GoalContractViolation>>::json_schema(generator).into_object();
        if let Some(array) = schema.array.as_mut() {
            array.min_items = Some(Self::MIN as u32);
            array.max_items = Some(Self::MAX as u32);
        }
        schemars::schema::Schema::Object(schema)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NotARepository,
    NotAWorktree,
    NotRegistered,
    IdentityConflict,
    SnapshotContention,
    SessionNotFound,
    SessionNotLive,
    SessionNotRecovering,
    SessionAmbiguous,
    LeaseMismatch,
    LeaseExpired,
    GraceExpired,
    InvalidAgentInstance,
    WatcherStartFailed,
    ProjectNotFound,
    ProjectArchived,
    ProjectScopeRequired,
    InvalidProject,
    TaskNotFound,
    InvalidTask,
    TaskRevisionNotFound,
    TaskRevisionConflict,
    RepositoryNotAssociated,
    RepositoryProjectConflict,
    TaskRevisionProjectMismatch,
    SessionBindingConflict,
    SessionScopeConflict,
    AmbiguousName,
    InvalidGoalContract,
    IdempotencyConflict,
    StorageBusy,
    MigrationFailed,
    GitUnavailable,
    StateCorrupted,
    Internal,
    // CLI-only codes:
    DaemonUnavailable,
    Usage,
}

impl ErrorCode {
    pub const FEATURE002_CODES: &'static [Self] = &[
        Self::ProjectNotFound,
        Self::ProjectArchived,
        Self::ProjectScopeRequired,
        Self::InvalidProject,
        Self::TaskNotFound,
        Self::InvalidTask,
        Self::TaskRevisionNotFound,
        Self::TaskRevisionConflict,
        Self::RepositoryNotAssociated,
        Self::RepositoryProjectConflict,
        Self::TaskRevisionProjectMismatch,
        Self::SessionBindingConflict,
        Self::SessionScopeConflict,
        Self::AmbiguousName,
        Self::InvalidGoalContract,
        Self::IdempotencyConflict,
        Self::StorageBusy,
        Self::MigrationFailed,
    ];

    /// Stable CLI exit code mapping (contracts/cli-json-contract.md).
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorCode::NotARepository | ErrorCode::NotAWorktree | ErrorCode::NotRegistered => 3,
            ErrorCode::SessionAmbiguous | ErrorCode::AmbiguousName => 4,
            ErrorCode::DaemonUnavailable => 5,
            ErrorCode::StateCorrupted | ErrorCode::MigrationFailed => 6,
            ErrorCode::Usage => 2,
            _ => 1,
        }
    }

    /// Validate the closed code/data pairing used by Feature 002 contracts.
    pub const fn accepts_data(self, data: &ErrorData) -> bool {
        matches!(
            (self, data),
            (
                Self::WatcherStartFailed,
                ErrorData::WatcherStartFailure { .. }
            ) | (Self::ProjectNotFound, ErrorData::ProjectNotFound { .. })
                | (Self::ProjectArchived, ErrorData::ProjectArchived { .. })
                | (
                    Self::ProjectScopeRequired,
                    ErrorData::ProjectScopeRequired { .. }
                )
                | (Self::InvalidProject, ErrorData::InvalidProject { .. })
                | (Self::TaskNotFound, ErrorData::TaskNotFound { .. })
                | (Self::InvalidTask, ErrorData::InvalidTask { .. })
                | (
                    Self::TaskRevisionNotFound,
                    ErrorData::TaskRevisionNotFound { .. }
                )
                | (
                    Self::TaskRevisionConflict,
                    ErrorData::TaskRevisionConflict { .. }
                )
                | (
                    Self::RepositoryNotAssociated,
                    ErrorData::RepositoryNotAssociated { .. }
                )
                | (
                    Self::RepositoryProjectConflict,
                    ErrorData::RepositoryAlreadyAssociated { .. }
                )
                | (
                    Self::TaskRevisionProjectMismatch,
                    ErrorData::TaskRevisionProjectMismatch { .. }
                )
                | (
                    Self::SessionBindingConflict,
                    ErrorData::SessionAlreadyBound { .. }
                )
                | (
                    Self::SessionScopeConflict,
                    ErrorData::SessionScopeConflict { .. }
                )
                | (Self::AmbiguousName, ErrorData::AmbiguousName { .. })
                | (
                    Self::InvalidGoalContract,
                    ErrorData::InvalidGoalContract { .. }
                )
                | (
                    Self::IdempotencyConflict,
                    ErrorData::IdempotencyConflict { .. }
                )
                | (Self::StorageBusy, ErrorData::StorageBusy { .. })
                | (Self::MigrationFailed, ErrorData::MigrationFailure { .. })
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InvalidProjectField {
    Name,
    Description,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InvalidTaskField {
    Title,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValidationRule {
    Required,
    Empty,
    ConflictingFields,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskRevisionConflictReason {
    ParentMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BindingModeName {
    LocalUnbound,
    ProjectBound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AmbiguousEntity {
    Project,
    Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum OperationMethod {
    #[serde(rename = "project.create")]
    ProjectCreate,
    #[serde(rename = "project.update")]
    ProjectUpdate,
    #[serde(rename = "project.repository_associate")]
    ProjectRepositoryAssociate,
    #[serde(rename = "task.create")]
    TaskCreate,
    #[serde(rename = "task.revise")]
    TaskRevise,
    #[serde(rename = "session.bind")]
    SessionBind,
}

impl OperationMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectCreate => "project.create",
            Self::ProjectUpdate => "project.update",
            Self::ProjectRepositoryAssociate => "project.repository_associate",
            Self::TaskCreate => "task.create",
            Self::TaskRevise => "task.revise",
            Self::SessionBind => "session.bind",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IdempotencyConflictReason {
    MethodMismatch,
    RequestMismatch,
}

/// Typed data for the closed Feature 001+002 error-code set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ErrorData {
    WatcherStartFailure {
        stage: WatcherStartStage,
    },
    ProjectNotFound {
        project_id: ProjectId,
    },
    ProjectArchived {
        project_id: ProjectId,
    },
    ProjectScopeRequired {
        repository_id: String,
        project_id: ProjectId,
    },
    InvalidProject {
        field: InvalidProjectField,
        rule: ValidationRule,
    },
    TaskNotFound {
        task_id: TaskId,
    },
    InvalidTask {
        field: InvalidTaskField,
        rule: ValidationRule,
    },
    TaskRevisionNotFound {
        revision_id: TaskRevisionId,
    },
    TaskRevisionConflict {
        task_id: TaskId,
        reason: TaskRevisionConflictReason,
    },
    RepositoryNotAssociated {
        repository_id: String,
        project_id: ProjectId,
    },
    RepositoryAlreadyAssociated {
        repository_id: String,
        existing_project_id: ProjectId,
        requested_project_id: ProjectId,
    },
    TaskRevisionProjectMismatch {
        revision_id: TaskRevisionId,
        expected_project_id: ProjectId,
    },
    SessionAlreadyBound {
        session_id: String,
        existing_project_id: ProjectId,
        existing_revision_id: TaskRevisionId,
    },
    SessionScopeConflict {
        session_id: String,
        existing_mode: BindingModeName,
        requested_mode: BindingModeName,
    },
    AmbiguousName {
        entity: AmbiguousEntity,
        candidate_ids: CandidateIds,
        truncated: bool,
    },
    InvalidGoalContract {
        violations: GoalContractViolations,
    },
    MigrationFailure {
        target_version: u16,
    },
    IdempotencyConflict {
        idempotency_key: IdempotencyKey,
        existing_method: OperationMethod,
        requested_method: OperationMethod,
        reason: IdempotencyConflictReason,
    },
    StorageBusy {
        max_elapsed_ms: u32,
    },
}

impl ErrorData {
    pub const fn watcher_start_failure(stage: WatcherStartStage) -> Self {
        Self::WatcherStartFailure { stage }
    }

    pub fn watcher_stage(self) -> WatcherStartStage {
        match self {
            Self::WatcherStartFailure { stage } => stage,
            _ => panic!("watcher_stage called for non-watcher error data"),
        }
    }

    pub const fn watcher_stage_ref(&self) -> Option<WatcherStartStage> {
        match self {
            Self::WatcherStartFailure { stage } => Some(*stage),
            _ => None,
        }
    }
}
