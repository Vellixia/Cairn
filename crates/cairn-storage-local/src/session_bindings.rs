use sqlx::{SqliteConnection, SqlitePool};

use crate::db::StorageError;
use crate::operation_idempotency::{self, Reservation};
use crate::records::{OperationIdempotencyRow, SessionBindingRow};
use crate::writer::{begin_immediate, WriteCheckpoint, WriteTestHooks, WriterPolicy};
use crate::{events, projects, sessions, tasks, worktrees, NewEvent};

const METHOD_BIND: &str = "session.bind";
const RESULT_EVENT: &str = "event";
const RESULT_BINDING: &str = "session_binding";

#[derive(Debug, Clone)]
pub struct BindMutation {
    pub idempotency_key: String,
    pub request_fingerprint: String,
    pub session_id: String,
    pub project_id: String,
    pub task_id: String,
    pub task_revision_id: String,
    pub bound_at: String,
    pub event: NewEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindMutationResult {
    pub binding: SessionBindingRow,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedSessionScope {
    pub task_id: String,
}

pub async fn get(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<Option<SessionBindingRow>, StorageError> {
    Ok(
        sqlx::query_as("SELECT * FROM session_bindings WHERE session_id=?")
            .bind(session_id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn get_in_tx(
    conn: &mut SqliteConnection,
    session_id: &str,
) -> Result<Option<SessionBindingRow>, StorageError> {
    Ok(
        sqlx::query_as("SELECT * FROM session_bindings WHERE session_id=?")
            .bind(session_id)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

pub async fn list_all(pool: &SqlitePool) -> Result<Vec<SessionBindingRow>, StorageError> {
    Ok(
        sqlx::query_as("SELECT * FROM session_bindings ORDER BY session_id")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn mode(pool: &SqlitePool, session_id: &str) -> Result<Option<String>, StorageError> {
    Ok(
        sqlx::query_as::<_, (String,)>("SELECT binding_mode FROM sessions WHERE id=?")
            .bind(session_id)
            .fetch_optional(pool)
            .await?
            .map(|(mode,)| mode),
    )
}

pub async fn insert(
    conn: &mut SqliteConnection,
    row: &SessionBindingRow,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO session_bindings (session_id, project_id, task_revision_id, bound_at, binding_event_seq) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&row.session_id)
    .bind(&row.project_id)
    .bind(&row.task_revision_id)
    .bind(&row.bound_at)
    .bind(row.binding_event_seq)
    .execute(&mut *conn)
    .await?;
    let updated = sqlx::query(
        "UPDATE sessions SET binding_mode='project_bound' WHERE id=? AND binding_mode='local_unbound'",
    )
    .bind(&row.session_id)
    .execute(&mut *conn)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(StorageError::Conflict("session is already bound".into()));
    }
    Ok(())
}

pub async fn bind_atomic(
    pool: &SqlitePool,
    policy: WriterPolicy,
    hooks: Option<WriteTestHooks>,
    request: BindMutation,
) -> Result<BindMutationResult, StorageError> {
    let closure_hooks = hooks.clone();
    begin_immediate(
        pool,
        policy,
        hooks,
        Box::new(move |conn| {
            Box::pin(async move {
                if operation_idempotency::get_in_tx(conn, &request.idempotency_key)
                    .await?
                    .is_some()
                {
                    let proposed = registry_row(&request, RESULT_EVENT, &request.event.id);
                    return match operation_idempotency::reserve_or_get(
                        conn,
                        &proposed,
                        closure_hooks.as_ref(),
                    )
                    .await?
                    {
                        Reservation::Existing(existing) => {
                            result_from_registry(conn, &existing).await
                        }
                        Reservation::Inserted(_) => Err(StorageError::Corrupted(
                            "existing binding operation was not resolved".into(),
                        )),
                    };
                }

                let existing_binding = get_in_tx(conn, &request.session_id).await?;
                if let Some(existing) = existing_binding.as_ref() {
                    if existing.project_id != request.project_id
                        || existing.task_revision_id != request.task_revision_id
                    {
                        return Err(StorageError::SessionAlreadyBound {
                            existing_project_id: existing.project_id.clone(),
                            existing_revision_id: existing.task_revision_id.clone(),
                        });
                    }
                }
                let (result_kind, result_locator) = if existing_binding.is_some() {
                    (RESULT_BINDING, request.session_id.as_str())
                } else {
                    (RESULT_EVENT, request.event.id.as_str())
                };
                let registry = registry_row(&request, result_kind, result_locator);
                match operation_idempotency::reserve_or_get(conn, &registry, closure_hooks.as_ref())
                    .await?
                {
                    Reservation::Existing(existing) => {
                        return result_from_registry(conn, &existing).await
                    }
                    Reservation::Inserted(_) => {}
                }

                validate_relationships(conn, &request).await?;
                if let Some(binding) = existing_binding {
                    return Ok(BindMutationResult {
                        binding,
                        created: false,
                    });
                }

                checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreEvent).await?;
                let appended = events::append_event(conn, &request.event).await?;
                checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PostEvent).await?;
                if appended.deduplicated {
                    return Err(StorageError::Corrupted(
                        "new binding event was unexpectedly deduplicated".into(),
                    ));
                }
                checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreProjection).await?;
                let binding = SessionBindingRow {
                    session_id: request.session_id,
                    project_id: request.project_id,
                    task_revision_id: request.task_revision_id,
                    bound_at: request.bound_at,
                    binding_event_seq: appended.seq,
                };
                insert(conn, &binding).await?;
                checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PostProjection).await?;
                checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreResultLocator).await?;
                Ok(BindMutationResult {
                    binding,
                    created: true,
                })
            })
        }),
    )
    .await
}

pub async fn resolve_existing_operation(
    pool: &SqlitePool,
    idempotency_key: &str,
    request_fingerprint: &str,
) -> Result<Option<BindMutationResult>, StorageError> {
    let Some(existing) = operation_idempotency::get(pool, idempotency_key).await? else {
        return Ok(None);
    };
    if existing.method != METHOD_BIND || existing.request_fingerprint != request_fingerprint {
        return Err(StorageError::IdempotencyConflict {
            existing_method: existing.method.clone(),
            reason: if existing.method != METHOD_BIND {
                crate::IdempotencyConflictReason::MethodMismatch
            } else {
                crate::IdempotencyConflictReason::RequestMismatch
            },
        });
    }
    let mut conn = pool.acquire().await?;
    result_from_registry(&mut conn, &existing).await.map(Some)
}

async fn validate_relationships(
    conn: &mut SqliteConnection,
    request: &BindMutation,
) -> Result<(), StorageError> {
    let session = sessions::get_by_id(&mut *conn, &request.session_id)
        .await?
        .ok_or_else(|| StorageError::Conflict("session_not_found".into()))?;
    let validated = validate_scope_for_worktree(
        conn,
        &session.repository_id,
        &session.worktree_id,
        &request.project_id,
        &request.task_revision_id,
    )
    .await?;
    if validated.task_id != request.task_id {
        return Err(StorageError::Conflict(
            "task_revision_project_mismatch".into(),
        ));
    }
    if request.event.aggregate_type != "session"
        || request.event.aggregate_id != request.session_id
        || request.event.session_id.as_deref() != Some(request.session_id.as_str())
        || request.event.repository_id.as_deref() != Some(session.repository_id.as_str())
        || request.event.worktree_id.as_deref() != Some(session.worktree_id.as_str())
    {
        return Err(StorageError::Corrupted(
            "session binding event scope is invalid".into(),
        ));
    }
    Ok(())
}

pub async fn validate_scope_for_worktree(
    conn: &mut SqliteConnection,
    repository_id: &str,
    worktree_id: &str,
    project_id: &str,
    task_revision_id: &str,
) -> Result<ValidatedSessionScope, StorageError> {
    let project = projects::get_in_tx(conn, project_id)
        .await?
        .ok_or_else(|| StorageError::Conflict("project_not_found".into()))?;
    if project.status != "active" {
        return Err(StorageError::Conflict("project_archived".into()));
    }
    let association = projects::association_by_repository_in_tx(conn, repository_id)
        .await?
        .filter(|association| association.project_id == project_id)
        .ok_or_else(|| StorageError::Conflict("repository_not_associated".into()))?;
    let worktree = worktrees::get_by_id(&mut *conn, worktree_id)
        .await?
        .ok_or_else(|| StorageError::Corrupted("session worktree is missing".into()))?;
    if worktree.repository_id != repository_id || association.repository_id != repository_id {
        return Err(StorageError::Conflict("repository_not_associated".into()));
    }
    let revision = tasks::revision_in_tx(conn, task_revision_id)
        .await?
        .ok_or_else(|| StorageError::Conflict("revision_not_found".into()))?;
    let task = tasks::get_in_tx(conn, &revision.task_id)
        .await?
        .ok_or_else(|| StorageError::Corrupted("revision task is missing".into()))?;
    if task.project_id != project_id {
        return Err(StorageError::Conflict(
            "task_revision_project_mismatch".into(),
        ));
    }
    Ok(ValidatedSessionScope { task_id: task.id })
}

/// Return the active associated project when at least one immutable task
/// revision is selectable. An archived association is deliberately not valid
/// project scope and therefore does not block a bootstrap session.
pub async fn project_requiring_scope(
    conn: &mut SqliteConnection,
    repository_id: &str,
) -> Result<Option<String>, StorageError> {
    let Some(association) = projects::association_by_repository_in_tx(conn, repository_id).await?
    else {
        return Ok(None);
    };
    let Some(project) = projects::get_in_tx(conn, &association.project_id).await? else {
        return Err(StorageError::Corrupted(
            "repository association project is missing".into(),
        ));
    };
    if project.status != "active" {
        return Ok(None);
    }
    let selectable: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM tasks t JOIN task_revisions r ON r.task_id=t.id \
         WHERE t.project_id=? LIMIT 1",
    )
    .bind(&association.project_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(selectable.map(|_| association.project_id))
}

fn registry_row(
    request: &BindMutation,
    result_kind: &str,
    result_locator: &str,
) -> OperationIdempotencyRow {
    OperationIdempotencyRow {
        idempotency_key: request.idempotency_key.clone(),
        method: METHOD_BIND.into(),
        request_fingerprint: request.request_fingerprint.clone(),
        result_kind: result_kind.into(),
        result_locator: result_locator.into(),
        created_at: request.bound_at.clone(),
    }
}

async fn result_from_registry(
    conn: &mut SqliteConnection,
    registry: &OperationIdempotencyRow,
) -> Result<BindMutationResult, StorageError> {
    let (session_id, created) = match registry.result_kind.as_str() {
        RESULT_EVENT => {
            let event = events::get_by_id_in_tx(conn, &registry.result_locator)
                .await?
                .ok_or_else(|| StorageError::Corrupted("binding result event is missing".into()))?;
            if event.event_type != "session.bound" {
                return Err(StorageError::Corrupted(
                    "binding result event type is invalid".into(),
                ));
            }
            (
                event.session_id.ok_or_else(|| {
                    StorageError::Corrupted("binding event session is missing".into())
                })?,
                true,
            )
        }
        RESULT_BINDING => (registry.result_locator.clone(), false),
        _ => {
            return Err(StorageError::Corrupted(
                "binding result kind is invalid".into(),
            ))
        }
    };
    let binding = get_in_tx(conn, &session_id)
        .await?
        .ok_or_else(|| StorageError::Corrupted("binding result projection is missing".into()))?;
    Ok(BindMutationResult { binding, created })
}

async fn checkpoint(
    hooks: Option<&WriteTestHooks>,
    point: WriteCheckpoint,
) -> Result<(), StorageError> {
    if let Some(hooks) = hooks {
        hooks.checkpoint(point).await?;
    }
    Ok(())
}
