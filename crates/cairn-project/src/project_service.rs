use std::str::FromStr;

use cairn_domain::{
    EventId, IdempotencyKey, Project, ProjectId, ProjectRepositoryAssociation,
    ProjectRepositoryAssociationId, ProjectStatus, Timestamp,
};
use cairn_events::{
    EventBuilder, ProjectAssociationEvent, ProjectChangedField, ProjectCreatedPayload,
    ProjectRepositoryAssociatedPayload, ProjectUpdatedPayload, PROJECT_CREATED,
    PROJECT_REPOSITORY_ASSOCIATED, PROJECT_UPDATED,
};
use cairn_storage_local::operation_idempotency::{self, Reservation};
use cairn_storage_local::records::{
    OperationIdempotencyRow, ProjectRepositoryAssociationRow, ProjectRow,
};
use cairn_storage_local::writer::{begin_immediate, WriteCheckpoint};
use cairn_storage_local::{events, projects, StorageError, WriteTestHooks, WriterPolicy};
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};

use crate::{IdempotencyConflictKind, ProjectTaskError};

const METHOD_CREATE: &str = "project.create";
const METHOD_UPDATE: &str = "project.update";
const METHOD_ASSOCIATE: &str = "project.repository_associate";
const RESULT_EVENT: &str = "event";
const RESULT_ASSOCIATION: &str = "project_repository_association";

#[derive(Clone)]
pub struct ProjectService {
    pool: SqlitePool,
    writer_policy: WriterPolicy,
    hooks: Option<WriteTestHooks>,
}

impl ProjectService {
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

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn create(
        &self,
        request: CreateProject,
    ) -> Result<CreateProjectResult, ProjectTaskError> {
        let now = Timestamp::now();
        let project = Project::new(ProjectId::new_v7(), request.name, request.description, now)
            .map_err(|_| ProjectTaskError::InvalidProject {
                field: "name",
                rule: "empty",
            })?;
        let fingerprint = fingerprint(&CreateFingerprint {
            name: &project.name,
            description: project.description.as_deref(),
        });
        let event_id = EventId::new_v7();
        let key = request.idempotency_key.to_string();
        let error_key = request.idempotency_key;
        let registry = registry_row(
            &key,
            METHOD_CREATE,
            fingerprint,
            RESULT_EVENT,
            event_id.to_string(),
            now,
        );
        let payload = ProjectCreatedPayload {
            schema_version: 1,
            project: project.clone().into(),
        };
        let event = EventBuilder::project_created(event_id, &key, &payload);
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
                            let project =
                                project_from_event(conn, &existing, PROJECT_CREATED, |value| {
                                    serde_json::from_value::<ProjectCreatedPayload>(value)
                                        .map(|payload| payload.project.into())
                                })
                                .await?;
                            Ok(CreateProjectResult {
                                project,
                                created: true,
                            })
                        }
                        Reservation::Inserted(_) => {
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreEvent).await?;
                            events::append_event(conn, &event).await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PostEvent).await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreProjection)
                                .await?;
                            projects::insert(conn, &project_to_row(&project)).await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PostProjection)
                                .await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreResultLocator)
                                .await?;
                            Ok(CreateProjectResult {
                                project,
                                created: true,
                            })
                        }
                    }
                })
            }),
        )
        .await;
        result.map_err(|error| map_operation_storage(error, error_key, METHOD_CREATE, None, None))
    }

    pub async fn list(
        &self,
        status: Option<ProjectStatus>,
        after_project_id: Option<ProjectId>,
        limit: u32,
    ) -> Result<ProjectList, ProjectTaskError> {
        let limit = limit.clamp(1, 100);
        let rows = projects::list_filtered(
            &self.pool,
            status.map(ProjectStatus::as_str),
            after_project_id
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            limit + 1,
        )
        .await
        .map_err(|error| map_storage(error, None, None))?;
        let mut values = rows
            .into_iter()
            .map(project_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| map_storage(error, None, None))?;
        let has_more = values.len() > limit as usize;
        values.truncate(limit as usize);
        let next_after_project_id = has_more.then(|| values.last().expect("nonempty page").id);
        Ok(ProjectList {
            projects: values,
            next_after_project_id,
        })
    }

    pub async fn get(&self, project_id: ProjectId) -> Result<ProjectDetail, ProjectTaskError> {
        let id = project_id.to_string();
        let row = projects::get(&self.pool, &id)
            .await
            .map_err(|error| map_storage(error, Some(project_id), None))?
            .ok_or(ProjectTaskError::ProjectNotFound { project_id })?;
        let project =
            project_from_row(row).map_err(|error| map_storage(error, Some(project_id), None))?;
        let associations = projects::list_associations(&self.pool, &id)
            .await
            .map_err(|error| map_storage(error, Some(project_id), None))?
            .into_iter()
            .map(association_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| map_storage(error, Some(project_id), None))?;
        let task_count = projects::task_count(&self.pool, &id)
            .await
            .map_err(|error| map_storage(error, Some(project_id), None))?;
        let bound_session_count = projects::bound_session_count(&self.pool, &id)
            .await
            .map_err(|error| map_storage(error, Some(project_id), None))?;
        Ok(ProjectDetail {
            project,
            associations,
            task_count,
            bound_session_count,
        })
    }

    pub async fn update(
        &self,
        request: UpdateProject,
    ) -> Result<UpdateProjectResult, ProjectTaskError> {
        validate_update(&request)?;
        let fingerprint = fingerprint(&UpdateFingerprint::from(&request));
        let now = Timestamp::now();
        let event_id = EventId::new_v7();
        let key = request.idempotency_key.to_string();
        let error_key = request.idempotency_key;
        let project_id = request.project_id;
        let registry = registry_row(
            &key,
            METHOD_UPDATE,
            fingerprint,
            RESULT_EVENT,
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
                            let project =
                                project_from_event(conn, &existing, PROJECT_UPDATED, |value| {
                                    serde_json::from_value::<ProjectUpdatedPayload>(value)
                                        .map(|payload| payload.project.into())
                                })
                                .await?;
                            Ok(UpdateProjectResult {
                                project,
                                updated: true,
                            })
                        }
                        Reservation::Inserted(_) => {
                            let row = projects::get_in_tx(conn, &project_id.to_string())
                                .await?
                                .ok_or(StorageError::NotFound)?;
                            let mut project = project_from_row(row)?;
                            let changed_fields = apply_update(&mut project, &request, now);
                            let payload = ProjectUpdatedPayload {
                                schema_version: 1,
                                project: project.clone().into(),
                                changed_fields,
                            };
                            let event = EventBuilder::project_updated(event_id, &key, &payload);
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreEvent).await?;
                            events::append_event(conn, &event).await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PostEvent).await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreProjection)
                                .await?;
                            projects::update_metadata(
                                conn,
                                &project.id.to_string(),
                                &project.name,
                                project.description.as_deref(),
                                project.status.as_str(),
                                &project.updated_at.to_rfc3339(),
                            )
                            .await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PostProjection)
                                .await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreResultLocator)
                                .await?;
                            Ok(UpdateProjectResult {
                                project,
                                updated: true,
                            })
                        }
                    }
                })
            }),
        )
        .await;
        result.map_err(|error| {
            map_operation_storage(error, error_key, METHOD_UPDATE, Some(project_id), None)
        })
    }

    pub async fn associate_repository(
        &self,
        request: AssociateRepository,
    ) -> Result<AssociateRepositoryResult, ProjectTaskError> {
        if request.repository_id.trim().is_empty() {
            return Err(ProjectTaskError::RepositoryNotFound {
                repository_id: request.repository_id,
            });
        }
        let fingerprint = fingerprint(&AssociateFingerprint::from(&request));
        let now = Timestamp::now();
        let event_id = EventId::new_v7();
        let association_id = ProjectRepositoryAssociationId::new_v7();
        let key = request.idempotency_key.to_string();
        let error_key = request.idempotency_key;
        let project_id = request.project_id;
        let repository_id = request.repository_id.clone();
        let hooks = self.hooks.clone();
        let closure_hooks = hooks.clone();
        let result = begin_immediate(
            &self.pool,
            self.writer_policy,
            hooks,
            Box::new(move |conn| {
                Box::pin(async move {
                    let existing_association =
                        projects::association_by_repository_in_tx(conn, &repository_id).await?;
                    let (result_kind, result_locator) = existing_association
                        .as_ref()
                        .filter(|association| association.project_id == project_id.to_string())
                        .map(|association| (RESULT_ASSOCIATION, association.id.clone()))
                        .unwrap_or((RESULT_EVENT, event_id.to_string()));
                    let registry = registry_row(
                        &key,
                        METHOD_ASSOCIATE,
                        fingerprint,
                        result_kind,
                        result_locator,
                        now,
                    );
                    match operation_idempotency::reserve_or_get(
                        conn,
                        &registry,
                        closure_hooks.as_ref(),
                    )
                    .await?
                    {
                        Reservation::Existing(existing) => {
                            association_result_from_registry(conn, &existing).await
                        }
                        Reservation::Inserted(_) => {
                            if let Some(existing) = existing_association {
                                if existing.project_id == project_id.to_string() {
                                    return Ok(AssociateRepositoryResult {
                                        association: association_from_row(existing)?,
                                        created: false,
                                    });
                                }
                                return Err(StorageError::Conflict(
                                    "repository_project_conflict".into(),
                                ));
                            }
                            let project = projects::get_in_tx(conn, &project_id.to_string())
                                .await?
                                .ok_or(StorageError::NotFound)?;
                            if project.status != ProjectStatus::Active.as_str() {
                                return Err(StorageError::Conflict("project_archived".into()));
                            }
                            if !projects::repository_exists_in_tx(conn, &repository_id).await? {
                                return Err(StorageError::Conflict("repository_not_found".into()));
                            }
                            let event_payload = ProjectRepositoryAssociatedPayload {
                                schema_version: 1,
                                association: ProjectAssociationEvent {
                                    association_id,
                                    project_id,
                                    repository_id: repository_id.clone(),
                                    associated_at: now,
                                },
                            };
                            let event = EventBuilder::project_repository_associated(
                                event_id,
                                &key,
                                &event_payload,
                            );
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreEvent).await?;
                            let outcome = events::append_event(conn, &event).await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PostEvent).await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreProjection)
                                .await?;
                            let association = ProjectRepositoryAssociation::new(
                                association_id,
                                project_id,
                                repository_id,
                                now,
                                outcome.seq,
                            )
                            .map_err(|_| {
                                StorageError::Corrupted("invalid association projection".into())
                            })?;
                            projects::insert_association(conn, &association_to_row(&association))
                                .await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PostProjection)
                                .await?;
                            checkpoint(closure_hooks.as_ref(), WriteCheckpoint::PreResultLocator)
                                .await?;
                            Ok(AssociateRepositoryResult {
                                association,
                                created: true,
                            })
                        }
                    }
                })
            }),
        )
        .await;
        match result {
            Ok(value) => Ok(value),
            Err(StorageError::Conflict(reason)) if reason == "project_archived" => {
                Err(ProjectTaskError::ProjectArchived { project_id })
            }
            Err(StorageError::Conflict(reason)) if reason == "repository_not_found" => {
                Err(ProjectTaskError::RepositoryNotFound {
                    repository_id: request.repository_id,
                })
            }
            Err(StorageError::Conflict(reason)) if reason == "repository_project_conflict" => {
                let existing =
                    projects::association_by_repository(&self.pool, &request.repository_id)
                        .await
                        .map_err(|error| map_storage(error, Some(project_id), None))?
                        .ok_or(ProjectTaskError::StorageFailure)?;
                let existing_project_id = ProjectId::from_str(&existing.project_id)
                    .map_err(|_| ProjectTaskError::StorageFailure)?;
                Err(ProjectTaskError::RepositoryAlreadyAssociated {
                    repository_id: request.repository_id,
                    existing_project_id,
                    requested_project_id: project_id,
                })
            }
            Err(error) => Err(map_operation_storage(
                error,
                error_key,
                METHOD_ASSOCIATE,
                Some(project_id),
                Some(request.repository_id),
            )),
        }
    }

    pub async fn association_for_repository(
        &self,
        repository_id: &str,
    ) -> Result<Option<ProjectRepositoryAssociation>, ProjectTaskError> {
        projects::association_by_repository(&self.pool, repository_id)
            .await
            .map_err(|error| map_storage(error, None, Some(repository_id.to_string())))?
            .map(association_from_row)
            .transpose()
            .map_err(|error| map_storage(error, None, Some(repository_id.to_string())))
    }

    pub async fn project_for_worktree(
        &self,
        worktree_id: &str,
    ) -> Result<Option<Project>, ProjectTaskError> {
        projects::project_for_worktree(&self.pool, worktree_id)
            .await
            .map_err(|error| map_storage(error, None, None))?
            .map(project_from_row)
            .transpose()
            .map_err(|error| map_storage(error, None, None))
    }
}

#[derive(Debug, Clone)]
pub struct CreateProject {
    pub idempotency_key: IdempotencyKey,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProjectResult {
    pub project: Project,
    pub created: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateProject {
    pub idempotency_key: IdempotencyKey,
    pub project_id: ProjectId,
    pub name: Option<String>,
    pub description: Option<String>,
    pub clear_description: bool,
    pub status: Option<ProjectStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateProjectResult {
    pub project: Project,
    pub updated: bool,
}

#[derive(Debug, Clone)]
pub struct AssociateRepository {
    pub idempotency_key: IdempotencyKey,
    pub project_id: ProjectId,
    pub repository_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssociateRepositoryResult {
    pub association: ProjectRepositoryAssociation,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectList {
    pub projects: Vec<Project>,
    pub next_after_project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDetail {
    pub project: Project,
    pub associations: Vec<ProjectRepositoryAssociation>,
    pub task_count: u64,
    pub bound_session_count: u64,
}

#[derive(Serialize)]
struct CreateFingerprint<'a> {
    name: &'a str,
    description: Option<&'a str>,
}

#[derive(Serialize)]
struct UpdateFingerprint<'a> {
    project_id: String,
    name: Option<&'a str>,
    description: Option<&'a str>,
    clear_description: bool,
    status: Option<&'static str>,
}

impl<'a> From<&'a UpdateProject> for UpdateFingerprint<'a> {
    fn from(value: &'a UpdateProject) -> Self {
        Self {
            project_id: value.project_id.to_string(),
            name: value.name.as_deref(),
            description: value.description.as_deref(),
            clear_description: value.clear_description,
            status: value.status.map(ProjectStatus::as_str),
        }
    }
}

#[derive(Serialize)]
struct AssociateFingerprint<'a> {
    project_id: String,
    repository_id: &'a str,
}

impl<'a> From<&'a AssociateRepository> for AssociateFingerprint<'a> {
    fn from(value: &'a AssociateRepository) -> Self {
        Self {
            project_id: value.project_id.to_string(),
            repository_id: &value.repository_id,
        }
    }
}

fn validate_update(request: &UpdateProject) -> Result<(), ProjectTaskError> {
    if request.name.is_none()
        && request.description.is_none()
        && !request.clear_description
        && request.status.is_none()
    {
        return Err(ProjectTaskError::InvalidProject {
            field: "status",
            rule: "required",
        });
    }
    if request.description.is_some() && request.clear_description {
        return Err(ProjectTaskError::InvalidProject {
            field: "description",
            rule: "conflicting_fields",
        });
    }
    if request
        .name
        .as_ref()
        .is_some_and(|name| normalize_display(name).is_empty())
    {
        return Err(ProjectTaskError::InvalidProject {
            field: "name",
            rule: "empty",
        });
    }
    Ok(())
}

fn apply_update(
    project: &mut Project,
    request: &UpdateProject,
    now: Timestamp,
) -> Vec<ProjectChangedField> {
    let mut changed = Vec::new();
    if let Some(name) = &request.name {
        let name = normalize_display(name);
        if project.name != name {
            project.name = name;
            changed.push(ProjectChangedField::Name);
        }
    }
    if request.clear_description {
        if project.description.take().is_some() {
            changed.push(ProjectChangedField::Description);
        }
    } else if let Some(description) = &request.description {
        let description = Some(normalize_display(description));
        if project.description != description {
            project.description = description;
            changed.push(ProjectChangedField::Description);
        }
    }
    if let Some(status) = request.status {
        if project.status != status {
            project.status = status;
            changed.push(ProjectChangedField::Status);
        }
    }
    changed.sort_unstable();
    project.updated_at = now;
    changed
}

fn normalize_display(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_string()
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
    result_kind: &str,
    result_locator: String,
    created_at: Timestamp,
) -> OperationIdempotencyRow {
    OperationIdempotencyRow {
        idempotency_key: key.to_string(),
        method: method.to_string(),
        request_fingerprint,
        result_kind: result_kind.to_string(),
        result_locator,
        created_at: created_at.to_rfc3339(),
    }
}

async fn project_from_event<F>(
    conn: &mut SqliteConnection,
    registry: &OperationIdempotencyRow,
    expected_type: &str,
    decode: F,
) -> Result<Project, StorageError>
where
    F: FnOnce(serde_json::Value) -> Result<Project, serde_json::Error>,
{
    if registry.result_kind != RESULT_EVENT {
        return Err(StorageError::Corrupted(
            "project operation result kind is invalid".into(),
        ));
    }
    let event = events::get_by_id_in_tx(conn, &registry.result_locator)
        .await?
        .ok_or_else(|| StorageError::Corrupted("project operation result is missing".into()))?;
    if event.event_type != expected_type {
        return Err(StorageError::Corrupted(
            "project operation event type is invalid".into(),
        ));
    }
    let value = serde_json::from_str(&event.payload)
        .map_err(|_| StorageError::Corrupted("project event payload is invalid".into()))?;
    decode(value).map_err(|_| StorageError::Corrupted("project event payload is invalid".into()))
}

async fn association_result_from_registry(
    conn: &mut SqliteConnection,
    registry: &OperationIdempotencyRow,
) -> Result<AssociateRepositoryResult, StorageError> {
    match registry.result_kind.as_str() {
        RESULT_EVENT => {
            let event = events::get_by_id_in_tx(conn, &registry.result_locator)
                .await?
                .ok_or_else(|| StorageError::Corrupted("association event is missing".into()))?;
            if event.event_type != PROJECT_REPOSITORY_ASSOCIATED {
                return Err(StorageError::Corrupted(
                    "association event type is invalid".into(),
                ));
            }
            let payload: ProjectRepositoryAssociatedPayload = serde_json::from_str(&event.payload)
                .map_err(|_| {
                    StorageError::Corrupted("association event payload is invalid".into())
                })?;
            let association = ProjectRepositoryAssociation::new(
                payload.association.association_id,
                payload.association.project_id,
                payload.association.repository_id,
                payload.association.associated_at,
                event.seq,
            )
            .map_err(|_| StorageError::Corrupted("association event is invalid".into()))?;
            Ok(AssociateRepositoryResult {
                association,
                created: true,
            })
        }
        RESULT_ASSOCIATION => {
            let association = projects::association_by_id_in_tx(conn, &registry.result_locator)
                .await?
                .ok_or_else(|| StorageError::Corrupted("association result is missing".into()))?;
            Ok(AssociateRepositoryResult {
                association: association_from_row(association)?,
                created: false,
            })
        }
        _ => Err(StorageError::Corrupted(
            "association result kind is invalid".into(),
        )),
    }
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

fn project_to_row(project: &Project) -> ProjectRow {
    ProjectRow {
        id: project.id.to_string(),
        name: project.name.clone(),
        description: project.description.clone(),
        status: project.status.as_str().to_string(),
        created_at: project.created_at.to_rfc3339(),
        updated_at: project.updated_at.to_rfc3339(),
    }
}

pub(crate) fn project_from_row(row: ProjectRow) -> Result<Project, StorageError> {
    let status = match row.status.as_str() {
        "active" => ProjectStatus::Active,
        "archived" => ProjectStatus::Archived,
        _ => return Err(StorageError::Corrupted("invalid project status".into())),
    };
    Ok(Project {
        id: ProjectId::from_str(&row.id)
            .map_err(|_| StorageError::Corrupted("invalid project id".into()))?,
        name: row.name,
        description: row.description,
        status,
        created_at: Timestamp::parse(&row.created_at)
            .map_err(|_| StorageError::Corrupted("invalid project timestamp".into()))?,
        updated_at: Timestamp::parse(&row.updated_at)
            .map_err(|_| StorageError::Corrupted("invalid project timestamp".into()))?,
    })
}

fn association_to_row(
    association: &ProjectRepositoryAssociation,
) -> ProjectRepositoryAssociationRow {
    ProjectRepositoryAssociationRow {
        id: association.id.to_string(),
        project_id: association.project_id.to_string(),
        repository_id: association.repository_id.clone(),
        associated_at: association.associated_at.to_rfc3339(),
        event_seq: association.event_seq,
    }
}

pub(crate) fn association_from_row(
    row: ProjectRepositoryAssociationRow,
) -> Result<ProjectRepositoryAssociation, StorageError> {
    ProjectRepositoryAssociation::new(
        ProjectRepositoryAssociationId::from_str(&row.id)
            .map_err(|_| StorageError::Corrupted("invalid association id".into()))?,
        ProjectId::from_str(&row.project_id)
            .map_err(|_| StorageError::Corrupted("invalid project id".into()))?,
        row.repository_id,
        Timestamp::parse(&row.associated_at)
            .map_err(|_| StorageError::Corrupted("invalid association timestamp".into()))?,
        row.event_seq,
    )
    .map_err(|_| StorageError::Corrupted("invalid association projection".into()))
}

fn map_storage(
    error: StorageError,
    project_id: Option<ProjectId>,
    repository_id: Option<String>,
) -> ProjectTaskError {
    match error {
        StorageError::NotFound => ProjectTaskError::ProjectNotFound {
            project_id: project_id.expect("project context for not-found"),
        },
        StorageError::IdempotencyConflict { .. } => ProjectTaskError::StorageFailure,
        StorageError::StorageBusy { max_elapsed_ms } => {
            ProjectTaskError::StorageBusy { max_elapsed_ms }
        }
        StorageError::Conflict(reason) if reason == "repository_not_found" => {
            ProjectTaskError::RepositoryNotFound {
                repository_id: repository_id.unwrap_or_default(),
            }
        }
        _ => ProjectTaskError::StorageFailure,
    }
}

fn map_operation_storage(
    error: StorageError,
    idempotency_key: IdempotencyKey,
    requested_method: &'static str,
    project_id: Option<ProjectId>,
    repository_id: Option<String>,
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
        other => map_storage(other, project_id, repository_id),
    }
}
