//! IPC method handlers. Thin: policy lives in crates (module ownership map).

pub mod convert;
pub mod daemon;
pub mod events;
pub mod projects;
pub mod repository;
pub mod session;
pub mod snapshot;
pub mod tasks;

use cairn_git::GitError;
use cairn_project::ProjectTaskError;
use cairn_protocol::{BindingModeName, ErrorCode, ErrorData};
use cairn_session::{SessionBindingError, SessionError};
use cairn_storage_local::StorageError;

/// Handler-level error carrying its wire code.
#[derive(Debug)]
pub struct HandlerError {
    pub code: ErrorCode,
    pub message: String,
    pub data: Option<ErrorData>,
}

impl HandlerError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: ErrorData) -> Self {
        self.data = Some(data);
        self
    }
}

impl From<GitError> for HandlerError {
    fn from(e: GitError) -> Self {
        let code = match &e {
            GitError::GitUnavailable(_) => ErrorCode::GitUnavailable,
            GitError::NotARepository(_) => ErrorCode::NotARepository,
            GitError::NotAWorktree(_) => ErrorCode::NotAWorktree,
            GitError::SnapshotContention(_) => ErrorCode::SnapshotContention,
            _ => ErrorCode::Internal,
        };
        Self::new(code, e.to_string())
    }
}

impl From<StorageError> for HandlerError {
    fn from(e: StorageError) -> Self {
        let code = if e.is_corruption() {
            ErrorCode::StateCorrupted
        } else {
            ErrorCode::Internal
        };
        Self::new(code, e.to_string())
    }
}

impl From<SessionError> for HandlerError {
    fn from(e: SessionError) -> Self {
        match e {
            SessionError::ProjectScopeRequired {
                repository_id,
                project_id,
            } => Self::new(
                ErrorCode::ProjectScopeRequired,
                "project/task scope is required",
            )
            .with_data(ErrorData::ProjectScopeRequired {
                repository_id,
                project_id,
            }),
            SessionError::ProjectNotFound { project_id } => {
                Self::new(ErrorCode::ProjectNotFound, "project not found")
                    .with_data(ErrorData::ProjectNotFound { project_id })
            }
            SessionError::ProjectArchived { project_id } => {
                Self::new(ErrorCode::ProjectArchived, "project is archived")
                    .with_data(ErrorData::ProjectArchived { project_id })
            }
            SessionError::RepositoryNotAssociated {
                repository_id,
                project_id,
            } => Self::new(
                ErrorCode::RepositoryNotAssociated,
                "repository is not associated with project",
            )
            .with_data(ErrorData::RepositoryNotAssociated {
                repository_id,
                project_id,
            }),
            SessionError::TaskRevisionNotFound { revision_id } => {
                Self::new(ErrorCode::TaskRevisionNotFound, "task revision not found")
                    .with_data(ErrorData::TaskRevisionNotFound { revision_id })
            }
            SessionError::TaskRevisionProjectMismatch {
                revision_id,
                expected_project_id,
            } => Self::new(
                ErrorCode::TaskRevisionProjectMismatch,
                "task revision does not belong to project",
            )
            .with_data(ErrorData::TaskRevisionProjectMismatch {
                revision_id,
                expected_project_id,
            }),
            SessionError::ScopeConflict {
                session_id,
                existing_mode,
                requested_mode,
            } => Self::new(
                ErrorCode::SessionScopeConflict,
                "healthy session scope conflicts with requested scope",
            )
            .with_data(ErrorData::SessionScopeConflict {
                session_id,
                existing_mode: match existing_mode {
                    cairn_session::SessionScopeName::LocalUnbound => BindingModeName::LocalUnbound,
                    cairn_session::SessionScopeName::ProjectBound => BindingModeName::ProjectBound,
                },
                requested_mode: match requested_mode {
                    cairn_session::SessionScopeName::LocalUnbound => BindingModeName::LocalUnbound,
                    cairn_session::SessionScopeName::ProjectBound => BindingModeName::ProjectBound,
                },
            }),
            other => {
                let code = match &other {
                    SessionError::NotFound => ErrorCode::SessionNotFound,
                    SessionError::NotLive => ErrorCode::SessionNotLive,
                    SessionError::NotRecovering => ErrorCode::SessionNotRecovering,
                    SessionError::Ambiguous(_) => ErrorCode::SessionAmbiguous,
                    SessionError::LeaseMismatch => ErrorCode::LeaseMismatch,
                    SessionError::LeaseExpired => ErrorCode::LeaseExpired,
                    SessionError::GraceExpired => ErrorCode::GraceExpired,
                    SessionError::Storage(s) if s.is_corruption() => ErrorCode::StateCorrupted,
                    SessionError::Storage(_) => ErrorCode::Internal,
                    _ => ErrorCode::Internal,
                };
                Self::new(code, other.to_string())
            }
        }
    }
}

impl From<SessionBindingError> for HandlerError {
    fn from(error: SessionBindingError) -> Self {
        use cairn_protocol::{IdempotencyConflictReason, OperationMethod};
        use cairn_session::BindingIdempotencyConflictKind;

        match error {
            SessionBindingError::SessionNotFound { .. } => {
                Self::new(ErrorCode::SessionNotFound, "session not found")
            }
            SessionBindingError::ProjectNotFound { project_id } => {
                Self::new(ErrorCode::ProjectNotFound, "project not found")
                    .with_data(ErrorData::ProjectNotFound { project_id })
            }
            SessionBindingError::ProjectArchived { project_id } => {
                Self::new(ErrorCode::ProjectArchived, "project is archived")
                    .with_data(ErrorData::ProjectArchived { project_id })
            }
            SessionBindingError::RepositoryNotAssociated {
                repository_id,
                project_id,
            } => Self::new(
                ErrorCode::RepositoryNotAssociated,
                "repository is not associated with project",
            )
            .with_data(ErrorData::RepositoryNotAssociated {
                repository_id,
                project_id,
            }),
            SessionBindingError::TaskRevisionNotFound { revision_id } => {
                Self::new(ErrorCode::TaskRevisionNotFound, "task revision not found")
                    .with_data(ErrorData::TaskRevisionNotFound { revision_id })
            }
            SessionBindingError::TaskRevisionProjectMismatch {
                revision_id,
                expected_project_id,
            } => Self::new(
                ErrorCode::TaskRevisionProjectMismatch,
                "task revision does not belong to project",
            )
            .with_data(ErrorData::TaskRevisionProjectMismatch {
                revision_id,
                expected_project_id,
            }),
            SessionBindingError::SessionAlreadyBound {
                session_id,
                existing_project_id,
                existing_revision_id,
            } => Self::new(
                ErrorCode::SessionBindingConflict,
                "session is already bound differently",
            )
            .with_data(ErrorData::SessionAlreadyBound {
                session_id: session_id.to_string(),
                existing_project_id,
                existing_revision_id,
            }),
            SessionBindingError::IdempotencyConflict {
                idempotency_key,
                existing_method,
                reason,
                ..
            } => match operation_method(&existing_method) {
                Some(existing_method) => Self::new(
                    ErrorCode::IdempotencyConflict,
                    "idempotency key conflicts with an earlier operation",
                )
                .with_data(ErrorData::IdempotencyConflict {
                    idempotency_key,
                    existing_method,
                    requested_method: OperationMethod::SessionBind,
                    reason: match reason {
                        BindingIdempotencyConflictKind::MethodMismatch => {
                            IdempotencyConflictReason::MethodMismatch
                        }
                        BindingIdempotencyConflictKind::RequestMismatch => {
                            IdempotencyConflictReason::RequestMismatch
                        }
                    },
                }),
                None => Self::new(ErrorCode::Internal, "session binding failed"),
            },
            SessionBindingError::StorageBusy { max_elapsed_ms } => {
                Self::new(ErrorCode::StorageBusy, "local storage remained busy").with_data(
                    ErrorData::StorageBusy {
                        max_elapsed_ms: u32::try_from(max_elapsed_ms).unwrap_or(u32::MAX),
                    },
                )
            }
            SessionBindingError::StorageFailure => {
                Self::new(ErrorCode::Internal, "session binding failed")
            }
        }
    }
}

impl From<serde_json::Error> for HandlerError {
    fn from(e: serde_json::Error) -> Self {
        Self::new(ErrorCode::Usage, format!("invalid params: {e}"))
    }
}

impl From<ProjectTaskError> for HandlerError {
    fn from(error: ProjectTaskError) -> Self {
        use cairn_project::IdempotencyConflictKind;
        use cairn_protocol::{
            GoalContractViolations, IdempotencyConflictReason, InvalidProjectField,
            InvalidTaskField, TaskRevisionConflictReason, ValidationRule,
        };

        match error {
            ProjectTaskError::InvalidProject { field, rule } => {
                let field = match field {
                    "name" => InvalidProjectField::Name,
                    "description" => InvalidProjectField::Description,
                    _ => InvalidProjectField::Status,
                };
                let rule = match rule {
                    "empty" => ValidationRule::Empty,
                    "conflicting_fields" => ValidationRule::ConflictingFields,
                    _ => ValidationRule::Required,
                };
                Self::new(ErrorCode::InvalidProject, "invalid project input")
                    .with_data(ErrorData::InvalidProject { field, rule })
            }
            ProjectTaskError::ProjectNotFound { project_id } => {
                Self::new(ErrorCode::ProjectNotFound, "project not found")
                    .with_data(ErrorData::ProjectNotFound { project_id })
            }
            ProjectTaskError::ProjectArchived { project_id } => {
                Self::new(ErrorCode::ProjectArchived, "project is archived")
                    .with_data(ErrorData::ProjectArchived { project_id })
            }
            ProjectTaskError::InvalidTask { field: _, rule } => {
                let rule = if rule == "empty" {
                    ValidationRule::Empty
                } else {
                    ValidationRule::Required
                };
                Self::new(ErrorCode::InvalidTask, "invalid task input").with_data(
                    ErrorData::InvalidTask {
                        field: InvalidTaskField::Title,
                        rule,
                    },
                )
            }
            ProjectTaskError::TaskNotFound { task_id } => {
                Self::new(ErrorCode::TaskNotFound, "task not found")
                    .with_data(ErrorData::TaskNotFound { task_id })
            }
            ProjectTaskError::TaskRevisionNotFound { revision_id } => {
                Self::new(ErrorCode::TaskRevisionNotFound, "task revision not found")
                    .with_data(ErrorData::TaskRevisionNotFound { revision_id })
            }
            ProjectTaskError::TaskRevisionConflict { task_id } => Self::new(
                ErrorCode::TaskRevisionConflict,
                "task revision parent conflicts with task history",
            )
            .with_data(ErrorData::TaskRevisionConflict {
                task_id,
                reason: TaskRevisionConflictReason::ParentMismatch,
            }),
            ProjectTaskError::InvalidGoalContract { violations } => {
                match GoalContractViolations::new(violations) {
                    Ok(violations) => {
                        Self::new(ErrorCode::InvalidGoalContract, "invalid goal contract")
                            .with_data(ErrorData::InvalidGoalContract { violations })
                    }
                    Err(_) => Self::new(ErrorCode::Internal, "task operation failed"),
                }
            }
            ProjectTaskError::RepositoryAlreadyAssociated {
                repository_id,
                existing_project_id,
                requested_project_id,
            } => Self::new(
                ErrorCode::RepositoryProjectConflict,
                "repository is already associated with another project",
            )
            .with_data(ErrorData::RepositoryAlreadyAssociated {
                repository_id,
                existing_project_id,
                requested_project_id,
            }),
            ProjectTaskError::RepositoryNotFound { .. } => {
                Self::new(ErrorCode::NotRegistered, "repository is not registered")
            }
            ProjectTaskError::IdempotencyConflict {
                idempotency_key,
                existing_method,
                requested_method,
                reason,
            } => {
                let existing_method = operation_method(&existing_method);
                let requested_method = operation_method(requested_method);
                match (existing_method, requested_method) {
                    (Some(existing_method), Some(requested_method)) => Self::new(
                        ErrorCode::IdempotencyConflict,
                        "idempotency key conflicts with an earlier operation",
                    )
                    .with_data(ErrorData::IdempotencyConflict {
                        idempotency_key,
                        existing_method,
                        requested_method,
                        reason: match reason {
                            IdempotencyConflictKind::MethodMismatch => {
                                IdempotencyConflictReason::MethodMismatch
                            }
                            IdempotencyConflictKind::RequestMismatch => {
                                IdempotencyConflictReason::RequestMismatch
                            }
                        },
                    }),
                    _ => Self::new(ErrorCode::Internal, "project operation failed"),
                }
            }
            ProjectTaskError::StorageBusy { max_elapsed_ms } => {
                Self::new(ErrorCode::StorageBusy, "local storage remained busy").with_data(
                    ErrorData::StorageBusy {
                        max_elapsed_ms: u32::try_from(max_elapsed_ms).unwrap_or(u32::MAX),
                    },
                )
            }
            ProjectTaskError::StorageFailure => {
                Self::new(ErrorCode::Internal, "project operation failed")
            }
            _ => Self::new(ErrorCode::Internal, "project operation failed"),
        }
    }
}

fn operation_method(value: &str) -> Option<cairn_protocol::OperationMethod> {
    use cairn_protocol::OperationMethod;
    Some(match value {
        "project.create" => OperationMethod::ProjectCreate,
        "project.update" => OperationMethod::ProjectUpdate,
        "project.repository_associate" => OperationMethod::ProjectRepositoryAssociate,
        "task.create" => OperationMethod::TaskCreate,
        "task.revise" => OperationMethod::TaskRevise,
        "session.bind" => OperationMethod::SessionBind,
        _ => return None,
    })
}

pub type HandlerResult<T> = Result<T, HandlerError>;
