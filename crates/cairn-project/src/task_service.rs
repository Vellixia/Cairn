use std::ops::Deref;
use std::str::FromStr;

use cairn_domain::{
    EventId, GoalContractV1, IdempotencyKey, ProjectId, ProjectStatus, Task, TaskId, TaskRevision,
    TaskRevisionId, Timestamp,
};
use cairn_events::aggregate::EventOperationMethod;
use cairn_events::{
    EventBuilder, TaskCreatedPayload, TaskRevisionCreatedPayload, TASK_REVISION_CREATED,
};
use cairn_storage_local::operation_idempotency::{self, Reservation};
use cairn_storage_local::records::{OperationIdempotencyRow, TaskRevisionRow, TaskRow};
use cairn_storage_local::writer::{begin_immediate, WriteCheckpoint};
use cairn_storage_local::{events, projects, tasks, StorageError, WriteTestHooks, WriterPolicy};
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};

use crate::{IdempotencyConflictKind, ProjectTaskError};

const METHOD_CREATE: &str = "task.create";
const METHOD_REVISE: &str = "task.revise";
const RESULT_EVENT: &str = "event";

#[derive(Clone)]
pub struct TaskService {
    pool: SqlitePool,
    writer_policy: WriterPolicy,
    hooks: Option<WriteTestHooks>,
}

impl TaskService {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            writer_policy: WriterPolicy::default(),
            hooks: None,
        }
    }

    pub fn with_test_controls(
        pool: SqlitePool,
        writer_policy: WriterPolicy,
        hooks: WriteTestHooks,
    ) -> Self {
        Self {
            pool,
            writer_policy,
            hooks: Some(hooks),
        }
    }

    pub async fn create(&self, request: CreateTask) -> Result<CreateTaskResult, ProjectTaskError> {
        let now = Timestamp::now();
        let task =
            Task::new(TaskId::new_v7(), request.project_id, request.title, now).map_err(|_| {
                ProjectTaskError::InvalidTask {
                    field: "title",
                    rule: "empty",
                }
            })?;
        let revision = TaskRevision::new(
            TaskRevisionId::new_v7(),
            task.id,
            1,
            None,
            request.goal_contract,
            now,
        )
        .map_err(|_| ProjectTaskError::StorageFailure)?;
        let request_fingerprint = fingerprint(&CreateTaskFingerprint {
            project_id: task.project_id.to_string(),
            title: &task.title,
            goal_contract: &revision.goal_contract,
        });
        let task_event_id = EventId::new_v7();
        let revision_event_id = EventId::new_v7();
        let key = request.idempotency_key.to_string();
        let error_key = request.idempotency_key;
        let project_id = task.project_id;
        let registry = registry_row(
            &key,
            METHOD_CREATE,
            request_fingerprint,
            revision_event_id.to_string(),
            now,
        );
        let task_payload = TaskCreatedPayload {
            schema_version: 1,
            task: task.clone().into(),
        };
        let revision_payload = TaskRevisionCreatedPayload {
            schema_version: 1,
            revision: revision.clone().into(),
            task: task.clone().into(),
        };
        let task_event = EventBuilder::task_created(task_event_id, &key, &task_payload);
        let revision_event = EventBuilder::task_revision_created(
            revision_event_id,
            &key,
            EventOperationMethod::TaskCreate,
            1,
            &revision_payload,
        );
        let task_row = task_to_row(&task);
        let revision_row = revision_to_row(&revision);
        let hooks = self.hooks.clone();
        let closure_hooks = hooks.clone();
        let result = begin_immediate(
            &self.pool,
            self.writer_policy,
            hooks,
            Box::new(move |conn| {
                Box::pin(async move {
                    match operation_idempotency::reserve_or_get(
                        conn,
                        &registry,
                        closure_hooks.as_ref(),
                    )
                    .await?
                    {
                        Reservation::Existing(existing) => {
                            result_from_registry(conn, &existing).await
                        }
                        Reservation::Inserted(_) => {
                            let project = projects::get_in_tx(conn, &project_id.to_string())
                                .await?
                                .ok_or_else(|| {
                                    StorageError::Conflict("project_not_found".into())
                                })?;
                            if project.status != ProjectStatus::Active.as_str() {
                                return Err(StorageError::Conflict("project_archived".into()));
                            }
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreEvent).await?;
                            events::append_event(conn, &task_event).await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::BetweenEvents)
                                .await?;
                            events::append_event(conn, &revision_event).await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PostEvent).await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreProjection)
                                .await?;
                            tasks::insert_task(conn, &task_row, &revision_row).await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PostProjection)
                                .await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreResultLocator)
                                .await?;
                            Ok(CreateTaskResult {
                                task,
                                revision,
                                created: true,
                            })
                        }
                    }
                })
            }),
        )
        .await;
        result.map_err(|error| {
            map_operation_storage(
                error,
                error_key,
                METHOD_CREATE,
                Some(project_id),
                None,
                None,
            )
        })
    }

    pub async fn revise(&self, request: ReviseTask) -> Result<CreateTaskResult, ProjectTaskError> {
        let context_task = tasks::get(&self.pool, &request.task_id.to_string())
            .await
            .map_err(|_| ProjectTaskError::StorageFailure)?
            .ok_or(ProjectTaskError::TaskNotFound {
                task_id: request.task_id,
            })?;
        let context_project_id = ProjectId::from_str(&context_task.project_id)
            .map_err(|_| ProjectTaskError::StorageFailure)?;
        let request_fingerprint = fingerprint(&ReviseTaskFingerprint {
            task_id: request.task_id.to_string(),
            parent_revision_id: request.parent_revision_id.map(|id| id.to_string()),
            goal_contract: &request.goal_contract,
        });
        let now = Timestamp::now();
        let event_id = EventId::new_v7();
        let revision_id = TaskRevisionId::new_v7();
        let key = request.idempotency_key.to_string();
        let error_key = request.idempotency_key;
        let task_id = request.task_id;
        let explicit_parent = request.parent_revision_id;
        let requested_goal = request.goal_contract;
        let registry = registry_row(
            &key,
            METHOD_REVISE,
            request_fingerprint,
            event_id.to_string(),
            now,
        );
        let hooks = self.hooks.clone();
        let closure_hooks = hooks.clone();
        let result = begin_immediate(
            &self.pool,
            self.writer_policy,
            hooks,
            Box::new(move |conn| {
                Box::pin(async move {
                    match operation_idempotency::reserve_or_get(
                        conn,
                        &registry,
                        closure_hooks.as_ref(),
                    )
                    .await?
                    {
                        Reservation::Existing(existing) => {
                            result_from_registry(conn, &existing).await
                        }
                        Reservation::Inserted(_) => {
                            let task_row = tasks::get_in_tx(conn, &task_id.to_string())
                                .await?
                                .ok_or_else(|| StorageError::Conflict("task_not_found".into()))?;
                            let project = projects::get_in_tx(conn, &task_row.project_id)
                                .await?
                                .ok_or_else(|| {
                                    StorageError::Corrupted("task project is missing".into())
                                })?;
                            if project.status != ProjectStatus::Active.as_str() {
                                return Err(StorageError::Conflict("project_archived".into()));
                            }
                            let parent = match explicit_parent {
                                Some(parent_id) => {
                                    tasks::revision_in_tx(conn, &parent_id.to_string())
                                        .await?
                                        .ok_or_else(|| {
                                            StorageError::Conflict("revision_not_found".into())
                                        })?
                                }
                                None => {
                                    tasks::latest_revision_in_tx(
                                        conn,
                                        &task_row.id,
                                        task_row.latest_revision_number,
                                    )
                                    .await?
                                }
                            };
                            let next = task_row.latest_revision_number + 1;
                            if parent.task_id != task_row.id || parent.revision_number >= next {
                                return Err(StorageError::Conflict("invalid_parent".into()));
                            }
                            let parent_id = TaskRevisionId::from_str(&parent.id).map_err(|_| {
                                StorageError::Corrupted("invalid parent revision id".into())
                            })?;
                            let mut revision = TaskRevision::new(
                                revision_id,
                                task_id,
                                u64::try_from(next).map_err(|_| {
                                    StorageError::Corrupted("invalid revision number".into())
                                })?,
                                Some(parent_id),
                                requested_goal,
                                now,
                            )
                            .map_err(|_| StorageError::Conflict("invalid_parent".into()))?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreProjection)
                                .await?;
                            let stored = tasks::insert_next_revision(
                                conn,
                                revision_to_row(&revision),
                                &now.to_rfc3339(),
                                closure_hooks.as_ref(),
                            )
                            .await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PostProjection)
                                .await?;
                            revision.revision_number = u64::try_from(stored.revision_number)
                                .map_err(|_| {
                                    StorageError::Corrupted("invalid revision number".into())
                                })?;
                            let mut task = task_from_row(task_row)?;
                            task.latest_revision_number = revision.revision_number;
                            task.updated_at = now;
                            let payload = TaskRevisionCreatedPayload {
                                schema_version: 1,
                                revision: revision.clone().into(),
                                task: task.clone().into(),
                            };
                            let event = EventBuilder::task_revision_created(
                                event_id,
                                &key,
                                EventOperationMethod::TaskRevise,
                                0,
                                &payload,
                            );
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreEvent).await?;
                            events::append_event(conn, &event).await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PostEvent).await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreResultLocator)
                                .await?;
                            Ok(CreateTaskResult {
                                task,
                                revision,
                                created: true,
                            })
                        }
                    }
                })
            }),
        )
        .await;
        result.map_err(|error| {
            map_operation_storage(
                error,
                error_key,
                METHOD_REVISE,
                Some(context_project_id),
                Some(task_id),
                explicit_parent,
            )
        })
    }

    pub async fn list(
        &self,
        project_id: ProjectId,
        after_task_id: Option<TaskId>,
        limit: u32,
    ) -> Result<TaskList, ProjectTaskError> {
        if projects::get(&self.pool, &project_id.to_string())
            .await
            .map_err(|_| ProjectTaskError::StorageFailure)?
            .is_none()
        {
            return Err(ProjectTaskError::ProjectNotFound { project_id });
        }
        let limit = limit.clamp(1, 100);
        let rows = tasks::list_by_project(
            &self.pool,
            &project_id.to_string(),
            after_task_id.as_ref().map(ToString::to_string).as_deref(),
            limit + 1,
        )
        .await
        .map_err(|_| ProjectTaskError::StorageFailure)?;
        let mut values = Vec::with_capacity(rows.len());
        for row in rows {
            let task = task_from_row(row).map_err(|_| ProjectTaskError::StorageFailure)?;
            let latest = tasks::revisions(&self.pool, &task.id.to_string())
                .await
                .map_err(|_| ProjectTaskError::StorageFailure)?
                .into_iter()
                .last()
                .ok_or(ProjectTaskError::StorageFailure)?;
            values.push(TaskListEntry {
                task,
                latest_revision_fingerprint: latest.goal_contract_fingerprint,
            });
        }
        let has_more = values.len() > limit as usize;
        values.truncate(limit as usize);
        let next_after_task_id = has_more.then(|| values.last().expect("nonempty page").task.id);
        Ok(TaskList {
            tasks: values,
            next_after_task_id,
        })
    }

    pub async fn get(
        &self,
        task_id: TaskId,
        revision_id: Option<TaskRevisionId>,
    ) -> Result<TaskDetail, ProjectTaskError> {
        let task_row = tasks::get(&self.pool, &task_id.to_string())
            .await
            .map_err(|_| ProjectTaskError::StorageFailure)?
            .ok_or(ProjectTaskError::TaskNotFound { task_id })?;
        let task = task_from_row(task_row).map_err(|_| ProjectTaskError::StorageFailure)?;
        let revision_row = match revision_id {
            Some(revision_id) => tasks::revision(&self.pool, &revision_id.to_string())
                .await
                .map_err(|_| ProjectTaskError::StorageFailure)?
                .filter(|row| row.task_id == task_id.to_string())
                .ok_or(ProjectTaskError::TaskRevisionNotFound { revision_id })?,
            None => tasks::revisions(&self.pool, &task_id.to_string())
                .await
                .map_err(|_| ProjectTaskError::StorageFailure)?
                .into_iter()
                .last()
                .ok_or(ProjectTaskError::StorageFailure)?,
        };
        let revision =
            revision_from_row(revision_row).map_err(|_| ProjectTaskError::StorageFailure)?;
        Ok(TaskDetail { task, revision })
    }
}

#[derive(Debug, Clone)]
pub struct CreateTask {
    pub idempotency_key: IdempotencyKey,
    pub project_id: ProjectId,
    pub title: String,
    pub goal_contract: GoalContractV1,
}

#[derive(Debug, Clone)]
pub struct ReviseTask {
    pub idempotency_key: IdempotencyKey,
    pub task_id: TaskId,
    pub parent_revision_id: Option<TaskRevisionId>,
    pub goal_contract: GoalContractV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTaskResult {
    pub task: Task,
    pub revision: TaskRevision,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDetail {
    pub task: Task,
    pub revision: TaskRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskListEntry {
    pub task: Task,
    pub latest_revision_fingerprint: String,
}

impl Deref for TaskListEntry {
    type Target = Task;

    fn deref(&self) -> &Self::Target {
        &self.task
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskList {
    pub tasks: Vec<TaskListEntry>,
    pub next_after_task_id: Option<TaskId>,
}

#[derive(Serialize)]
struct CreateTaskFingerprint<'a> {
    project_id: String,
    title: &'a str,
    goal_contract: &'a GoalContractV1,
}

#[derive(Serialize)]
struct ReviseTaskFingerprint<'a> {
    task_id: String,
    parent_revision_id: Option<String>,
    goal_contract: &'a GoalContractV1,
}

fn fingerprint(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("fingerprint input is serializable");
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"cairn.operation-request.v1\0");
    hasher.update(&(bytes.len() as u64).to_be_bytes());
    hasher.update(&bytes);
    hasher.finalize().to_hex().to_string()
}

fn registry_row(
    key: &str,
    method: &str,
    request_fingerprint: String,
    result_locator: String,
    created_at: Timestamp,
) -> OperationIdempotencyRow {
    OperationIdempotencyRow {
        idempotency_key: key.to_string(),
        method: method.to_string(),
        request_fingerprint,
        result_kind: RESULT_EVENT.to_string(),
        result_locator,
        created_at: created_at.to_rfc3339(),
    }
}

async fn result_from_registry(
    conn: &mut SqliteConnection,
    registry: &OperationIdempotencyRow,
) -> Result<CreateTaskResult, StorageError> {
    if registry.result_kind != RESULT_EVENT {
        return Err(StorageError::Corrupted(
            "task result kind is invalid".into(),
        ));
    }
    let event = events::get_by_id_in_tx(conn, &registry.result_locator)
        .await?
        .ok_or_else(|| StorageError::Corrupted("task result event is missing".into()))?;
    if event.event_type != TASK_REVISION_CREATED {
        return Err(StorageError::Corrupted(
            "task result event type is invalid".into(),
        ));
    }
    let payload: TaskRevisionCreatedPayload = serde_json::from_str(&event.payload)
        .map_err(|_| StorageError::Corrupted("task result event is invalid".into()))?;
    let task: Task = payload.task.into();
    let revision = TaskRevision::try_from(payload.revision)
        .map_err(|_| StorageError::Corrupted("task revision event is invalid".into()))?;
    Ok(CreateTaskResult {
        task,
        revision,
        created: true,
    })
}

fn task_to_row(task: &Task) -> TaskRow {
    TaskRow {
        id: task.id.to_string(),
        project_id: task.project_id.to_string(),
        title: task.title.clone(),
        latest_revision_number: i64::try_from(task.latest_revision_number).unwrap_or(i64::MAX),
        created_at: task.created_at.to_rfc3339(),
        updated_at: task.updated_at.to_rfc3339(),
    }
}

pub(crate) fn task_from_row(row: TaskRow) -> Result<Task, StorageError> {
    Ok(Task {
        id: TaskId::from_str(&row.id)
            .map_err(|_| StorageError::Corrupted("invalid task id".into()))?,
        project_id: ProjectId::from_str(&row.project_id)
            .map_err(|_| StorageError::Corrupted("invalid task project id".into()))?,
        title: row.title,
        latest_revision_number: u64::try_from(row.latest_revision_number)
            .map_err(|_| StorageError::Corrupted("invalid task revision number".into()))?,
        created_at: Timestamp::parse(&row.created_at)
            .map_err(|_| StorageError::Corrupted("invalid task timestamp".into()))?,
        updated_at: Timestamp::parse(&row.updated_at)
            .map_err(|_| StorageError::Corrupted("invalid task timestamp".into()))?,
    })
}

fn revision_to_row(revision: &TaskRevision) -> TaskRevisionRow {
    TaskRevisionRow {
        id: revision.id.to_string(),
        task_id: revision.task_id.to_string(),
        revision_number: i64::try_from(revision.revision_number).unwrap_or(i64::MAX),
        parent_revision_id: revision.parent_revision_id.map(|id| id.to_string()),
        goal_contract_json: String::from_utf8(revision.goal_contract.canonical_bytes())
            .expect("canonical goal contract is UTF-8"),
        goal_contract_schema_version: i64::from(revision.goal_contract.schema_version()),
        goal_contract_fingerprint: revision.goal_contract_fingerprint.clone(),
        created_at: revision.created_at.to_rfc3339(),
    }
}

pub(crate) fn revision_from_row(row: TaskRevisionRow) -> Result<TaskRevision, StorageError> {
    let goal_contract = GoalContractV1::from_json_slice(row.goal_contract_json.as_bytes())
        .map_err(|_| StorageError::Corrupted("invalid stored goal contract".into()))?;
    if i64::from(goal_contract.schema_version()) != row.goal_contract_schema_version
        || goal_contract.fingerprint() != row.goal_contract_fingerprint
    {
        return Err(StorageError::Corrupted(
            "stored goal contract fingerprint mismatch".into(),
        ));
    }
    TaskRevision::new(
        TaskRevisionId::from_str(&row.id)
            .map_err(|_| StorageError::Corrupted("invalid task revision id".into()))?,
        TaskId::from_str(&row.task_id)
            .map_err(|_| StorageError::Corrupted("invalid task id".into()))?,
        u64::try_from(row.revision_number)
            .map_err(|_| StorageError::Corrupted("invalid revision number".into()))?,
        row.parent_revision_id
            .map(|id| TaskRevisionId::from_str(&id))
            .transpose()
            .map_err(|_| StorageError::Corrupted("invalid parent revision id".into()))?,
        goal_contract,
        Timestamp::parse(&row.created_at)
            .map_err(|_| StorageError::Corrupted("invalid revision timestamp".into()))?,
    )
    .map_err(|_| StorageError::Corrupted("invalid task revision".into()))
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

fn map_operation_storage(
    error: StorageError,
    idempotency_key: IdempotencyKey,
    requested_method: &'static str,
    project_id: Option<ProjectId>,
    task_id: Option<TaskId>,
    revision_id: Option<TaskRevisionId>,
) -> ProjectTaskError {
    match error {
        StorageError::IdempotencyConflict {
            existing_method,
            reason,
        } => ProjectTaskError::IdempotencyConflict {
            idempotency_key,
            existing_method,
            requested_method,
            reason: match reason {
                cairn_storage_local::IdempotencyConflictReason::MethodMismatch => {
                    IdempotencyConflictKind::MethodMismatch
                }
                cairn_storage_local::IdempotencyConflictReason::RequestMismatch => {
                    IdempotencyConflictKind::RequestMismatch
                }
            },
        },
        StorageError::StorageBusy { max_elapsed_ms } => {
            ProjectTaskError::StorageBusy { max_elapsed_ms }
        }
        StorageError::Conflict(reason) if reason == "project_not_found" => {
            ProjectTaskError::ProjectNotFound {
                project_id: project_id.expect("project context"),
            }
        }
        StorageError::Conflict(reason) if reason == "project_archived" => {
            ProjectTaskError::ProjectArchived {
                project_id: project_id.expect("project context"),
            }
        }
        StorageError::Conflict(reason) if reason == "task_not_found" => {
            ProjectTaskError::TaskNotFound {
                task_id: task_id.expect("task context"),
            }
        }
        StorageError::Conflict(reason) if reason == "revision_not_found" => {
            ProjectTaskError::TaskRevisionNotFound {
                revision_id: revision_id.expect("revision context"),
            }
        }
        StorageError::Conflict(reason) if reason == "invalid_parent" => {
            ProjectTaskError::TaskRevisionConflict {
                task_id: task_id.expect("task context"),
            }
        }
        _ => ProjectTaskError::StorageFailure,
    }
}
