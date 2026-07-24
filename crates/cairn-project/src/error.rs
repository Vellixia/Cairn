use cairn_domain::{GoalContractViolation, ProjectId, TaskId, TaskRevisionId};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdempotencyConflictKind {
    MethodMismatch,
    RequestMismatch,
}

/// Focused policy failures. Wire adapters map these variants to the single
/// canonical protocol code declared for each invariant.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectTaskError {
    #[error("invalid project input")]
    InvalidProject {
        field: &'static str,
        rule: &'static str,
    },
    #[error("project not found")]
    ProjectNotFound { project_id: ProjectId },
    #[error("project is archived")]
    ProjectArchived { project_id: ProjectId },
    #[error("invalid task input")]
    InvalidTask {
        field: &'static str,
        rule: &'static str,
    },
    #[error("task not found")]
    TaskNotFound { task_id: TaskId },
    #[error("task revision not found")]
    TaskRevisionNotFound { revision_id: TaskRevisionId },
    #[error("task revision parent conflicts with the task history")]
    TaskRevisionConflict { task_id: TaskId },
    #[error("invalid goal contract")]
    InvalidGoalContract {
        violations: Vec<GoalContractViolation>,
    },
    #[error("repository is already associated with another project")]
    RepositoryAlreadyAssociated {
        repository_id: String,
        existing_project_id: ProjectId,
        requested_project_id: ProjectId,
    },
    #[error("task revision does not belong to the requested project")]
    TaskRevisionProjectMismatch {
        revision_id: TaskRevisionId,
        expected_project_id: ProjectId,
    },
    #[error("session is already bound differently")]
    SessionAlreadyBound {
        session_id: String,
        existing_project_id: ProjectId,
        existing_revision_id: TaskRevisionId,
    },
    #[error("idempotency key was reused for another operation")]
    IdempotencyConflict {
        idempotency_key: cairn_domain::IdempotencyKey,
        existing_method: String,
        requested_method: &'static str,
        reason: IdempotencyConflictKind,
    },
    #[error("repository is not registered")]
    RepositoryNotFound { repository_id: String },
    #[error("local storage remained busy")]
    StorageBusy { max_elapsed_ms: u64 },
    #[error("local project state is unavailable")]
    StorageFailure,
}
