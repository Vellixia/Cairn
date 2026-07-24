use std::str::FromStr;

use cairn_domain::{
    EventId, IdempotencyKey, ProjectId, SessionBinding, SessionId, TaskId, TaskRevisionId,
    Timestamp,
};
use cairn_events::{EventBuilder, SessionBindingEvent, SessionBoundPayload};
use cairn_storage_local::db::IdempotencyConflictReason as StorageConflictReason;
use cairn_storage_local::{session_bindings, sessions, tasks, StorageError};
use serde::Serialize;
use thiserror::Error;

use crate::SessionService;

const METHOD_BIND: &str = "session.bind";

#[derive(Debug, Clone)]
pub struct BindSession {
    pub idempotency_key: IdempotencyKey,
    pub session_id: SessionId,
    pub project_id: ProjectId,
    pub task_revision_id: TaskRevisionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindSessionResult {
    pub binding: SessionBinding,
    pub created: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingIdempotencyConflictKind {
    MethodMismatch,
    RequestMismatch,
}

#[derive(Debug, Error)]
pub enum SessionBindingError {
    #[error("session not found")]
    SessionNotFound { session_id: SessionId },
    #[error("project not found")]
    ProjectNotFound { project_id: ProjectId },
    #[error("project is archived")]
    ProjectArchived { project_id: ProjectId },
    #[error("repository is not associated with project")]
    RepositoryNotAssociated {
        repository_id: String,
        project_id: ProjectId,
    },
    #[error("task revision not found")]
    TaskRevisionNotFound { revision_id: TaskRevisionId },
    #[error("task revision does not belong to project")]
    TaskRevisionProjectMismatch {
        revision_id: TaskRevisionId,
        expected_project_id: ProjectId,
    },
    #[error("session is already bound differently")]
    SessionAlreadyBound {
        session_id: SessionId,
        existing_project_id: ProjectId,
        existing_revision_id: TaskRevisionId,
    },
    #[error("idempotency key conflicts with an earlier operation")]
    IdempotencyConflict {
        idempotency_key: IdempotencyKey,
        existing_method: String,
        requested_method: &'static str,
        reason: BindingIdempotencyConflictKind,
    },
    #[error("local storage remained busy")]
    StorageBusy { max_elapsed_ms: u64 },
    #[error("session binding failed")]
    StorageFailure,
}

impl SessionService {
    pub async fn bind(
        &self,
        request: BindSession,
    ) -> Result<BindSessionResult, SessionBindingError> {
        let request_fingerprint = fingerprint(&BindingFingerprint {
            session_id: request.session_id.to_string(),
            project_id: request.project_id.to_string(),
            task_revision_id: request.task_revision_id.to_string(),
        });
        if let Some(result) = session_bindings::resolve_existing_operation(
            &self.pool,
            &request.idempotency_key.to_string(),
            &request_fingerprint,
        )
        .await
        .map_err(|error| map_storage(error, &request, ""))?
        {
            return Ok(BindSessionResult {
                binding: binding_from_row(result.binding)?,
                created: result.created,
            });
        }
        let session = sessions::get_by_id(&self.pool, &request.session_id.to_string())
            .await
            .map_err(|_| SessionBindingError::StorageFailure)?
            .ok_or(SessionBindingError::SessionNotFound {
                session_id: request.session_id,
            })?;
        let revision = tasks::revision(&self.pool, &request.task_revision_id.to_string())
            .await
            .map_err(|_| SessionBindingError::StorageFailure)?
            .ok_or(SessionBindingError::TaskRevisionNotFound {
                revision_id: request.task_revision_id,
            })?;
        let task = tasks::get(&self.pool, &revision.task_id)
            .await
            .map_err(|_| SessionBindingError::StorageFailure)?
            .ok_or(SessionBindingError::StorageFailure)?;
        let task_id =
            TaskId::from_str(&task.id).map_err(|_| SessionBindingError::StorageFailure)?;
        let now = Timestamp::now();
        let payload = SessionBoundPayload {
            schema_version: 1,
            binding: SessionBindingEvent {
                session_id: request.session_id,
                repository_id: session.repository_id.clone(),
                worktree_id: session.worktree_id.clone(),
                project_id: request.project_id,
                task_id,
                task_revision_id: request.task_revision_id,
                bound_at: now,
            },
        };
        let event = EventBuilder::session_bound(
            EventId::new_v7(),
            &request.idempotency_key.to_string(),
            &payload,
        );
        let mutation = session_bindings::BindMutation {
            idempotency_key: request.idempotency_key.to_string(),
            request_fingerprint,
            session_id: request.session_id.to_string(),
            project_id: request.project_id.to_string(),
            task_id: task_id.to_string(),
            task_revision_id: request.task_revision_id.to_string(),
            bound_at: now.to_rfc3339(),
            event,
        };
        let result = session_bindings::bind_atomic(
            &self.pool,
            self.binding_writer_policy,
            self.test_hooks.clone(),
            mutation,
        )
        .await
        .map_err(|error| map_storage(error, &request, &session.repository_id))?;
        Ok(BindSessionResult {
            binding: binding_from_row(result.binding)?,
            created: result.created,
        })
    }
}

#[derive(Serialize)]
struct BindingFingerprint {
    session_id: String,
    project_id: String,
    task_revision_id: String,
}

fn fingerprint(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("binding fingerprint is serializable");
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"cairn.operation-request.v1\0");
    hasher.update(&(bytes.len() as u64).to_be_bytes());
    hasher.update(&bytes);
    hasher.finalize().to_hex().to_string()
}

fn binding_from_row(
    row: cairn_storage_local::SessionBindingRow,
) -> Result<SessionBinding, SessionBindingError> {
    Ok(SessionBinding {
        session_id: SessionId::from_str(&row.session_id)
            .map_err(|_| SessionBindingError::StorageFailure)?,
        project_id: ProjectId::from_str(&row.project_id)
            .map_err(|_| SessionBindingError::StorageFailure)?,
        task_revision_id: TaskRevisionId::from_str(&row.task_revision_id)
            .map_err(|_| SessionBindingError::StorageFailure)?,
        bound_at: Timestamp::parse(&row.bound_at)
            .map_err(|_| SessionBindingError::StorageFailure)?,
        binding_event_seq: row.binding_event_seq,
    })
}

fn map_storage(
    error: StorageError,
    request: &BindSession,
    repository_id: &str,
) -> SessionBindingError {
    match error {
        StorageError::Conflict(reason) if reason == "session_not_found" => {
            SessionBindingError::SessionNotFound {
                session_id: request.session_id,
            }
        }
        StorageError::Conflict(reason) if reason == "project_not_found" => {
            SessionBindingError::ProjectNotFound {
                project_id: request.project_id,
            }
        }
        StorageError::Conflict(reason) if reason == "project_archived" => {
            SessionBindingError::ProjectArchived {
                project_id: request.project_id,
            }
        }
        StorageError::Conflict(reason) if reason == "repository_not_associated" => {
            SessionBindingError::RepositoryNotAssociated {
                repository_id: repository_id.to_string(),
                project_id: request.project_id,
            }
        }
        StorageError::Conflict(reason) if reason == "revision_not_found" => {
            SessionBindingError::TaskRevisionNotFound {
                revision_id: request.task_revision_id,
            }
        }
        StorageError::Conflict(reason) if reason == "task_revision_project_mismatch" => {
            SessionBindingError::TaskRevisionProjectMismatch {
                revision_id: request.task_revision_id,
                expected_project_id: request.project_id,
            }
        }
        StorageError::SessionAlreadyBound {
            existing_project_id,
            existing_revision_id,
        } => match (
            ProjectId::from_str(&existing_project_id),
            TaskRevisionId::from_str(&existing_revision_id),
        ) {
            (Ok(existing_project_id), Ok(existing_revision_id)) => {
                SessionBindingError::SessionAlreadyBound {
                    session_id: request.session_id,
                    existing_project_id,
                    existing_revision_id,
                }
            }
            _ => SessionBindingError::StorageFailure,
        },
        StorageError::IdempotencyConflict {
            existing_method,
            reason,
        } => SessionBindingError::IdempotencyConflict {
            idempotency_key: request.idempotency_key,
            existing_method,
            requested_method: METHOD_BIND,
            reason: match reason {
                StorageConflictReason::MethodMismatch => {
                    BindingIdempotencyConflictKind::MethodMismatch
                }
                StorageConflictReason::RequestMismatch => {
                    BindingIdempotencyConflictKind::RequestMismatch
                }
            },
        },
        StorageError::StorageBusy { max_elapsed_ms } => {
            SessionBindingError::StorageBusy { max_elapsed_ms }
        }
        _ => SessionBindingError::StorageFailure,
    }
}
