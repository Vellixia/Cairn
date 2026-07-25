use cairn_domain::{
    IdempotencyKey, ProjectId, SessionBindingMode, SessionId, TaskRevisionId, Timestamp,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Transport scope discriminator. It intentionally contains no lifecycle
/// state because binding and liveness are independent dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionScopeDto {
    LocalUnbound,
    ProjectBound {
        project_id: ProjectId,
        task_revision_id: TaskRevisionId,
    },
}

impl From<SessionBindingMode> for SessionScopeDto {
    fn from(mode: SessionBindingMode) -> Self {
        match mode {
            SessionBindingMode::LocalUnbound => Self::LocalUnbound,
            SessionBindingMode::ProjectBound {
                project_id,
                task_revision_id,
            } => Self::ProjectBound {
                project_id,
                task_revision_id,
            },
        }
    }
}

impl From<SessionScopeDto> for SessionBindingMode {
    fn from(scope: SessionScopeDto) -> Self {
        match scope {
            SessionScopeDto::LocalUnbound => Self::LocalUnbound,
            SessionScopeDto::ProjectBound {
                project_id,
                task_revision_id,
            } => Self::ProjectBound {
                project_id,
                task_revision_id,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionBindParams {
    pub idempotency_key: IdempotencyKey,
    pub session_id: SessionId,
    pub project_id: ProjectId,
    pub task_revision_id: TaskRevisionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionBindResult {
    pub session_id: SessionId,
    pub scope: SessionScopeDto,
    pub bound_at: Timestamp,
    pub created: bool,
}
