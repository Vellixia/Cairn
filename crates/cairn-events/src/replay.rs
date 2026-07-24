//! T017 support: rebuild projections by replaying events in seq order and
//! compare against live projections (constitution: event replay).

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use cairn_domain::{
    Project, ProjectId, ProjectRepositoryAssociation, ProjectRepositoryAssociationId,
    ProjectStatus, SessionBinding, SessionBindingMode, SessionId, Task, TaskId, TaskRevision,
    TaskRevisionId, Timestamp,
};
use cairn_storage_local::{events, projects, session_bindings, tasks, StorageError};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use thiserror::Error;

/// Typed registration point for Feature 002 story handlers. The dispatcher
/// exists before US1; each story registers its own payload only when that
/// payload and projection are implemented.
pub trait ReplayHandler<State>: Send + Sync {
    fn event_type(&self) -> &'static str;
    fn schema_version(&self) -> u16;
    fn apply(
        &self,
        state: &mut State,
        payload: &serde_json::Value,
    ) -> Result<(), ReplayDispatchError>;
}

pub struct ReplayDispatcher<State> {
    handlers: BTreeMap<String, Box<dyn ReplayHandler<State>>>,
}

impl<State> Default for ReplayDispatcher<State> {
    fn default() -> Self {
        Self {
            handlers: BTreeMap::new(),
        }
    }
}

impl<State> ReplayDispatcher<State> {
    pub fn register(
        &mut self,
        handler: impl ReplayHandler<State> + 'static,
    ) -> Result<(), ReplayDispatchError> {
        let event_type = handler.event_type().to_string();
        if self.handlers.contains_key(&event_type) {
            return Err(ReplayDispatchError::DuplicateHandler(event_type));
        }
        self.handlers.insert(event_type, Box::new(handler));
        Ok(())
    }

    pub fn dispatch(
        &self,
        state: &mut State,
        event_type: &str,
        payload: &serde_json::Value,
    ) -> Result<(), ReplayDispatchError> {
        let handler = self
            .handlers
            .get(event_type)
            .ok_or_else(|| ReplayDispatchError::UnsupportedEventType(event_type.to_string()))?;
        let version = payload
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| ReplayDispatchError::MalformedPayload(event_type.to_string()))?;
        if version != handler.schema_version() {
            return Err(ReplayDispatchError::UnsupportedPayloadVersion {
                event_type: event_type.to_string(),
                version,
            });
        }
        handler.apply(state, payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ReplayDispatchError {
    #[error("duplicate replay handler for {0}")]
    DuplicateHandler(String),
    #[error("unsupported event type {0}")]
    UnsupportedEventType(String),
    #[error("malformed payload for {0}")]
    MalformedPayload(String),
    #[error("unsupported payload version {version} for {event_type}")]
    UnsupportedPayloadVersion { event_type: String, version: u16 },
    #[error("invalid payload for {0}")]
    InvalidPayload(String),
}

/// The replay-reconstructable portion of a session projection. Fields that
/// are not event-sourced (resume token hash, lease clock) are excluded by
/// design: they are authentication material, not history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedSession {
    pub session_id: String,
    pub state: String,
    pub current_snapshot_id: Option<String>,
    pub start_snapshot_id: Option<String>,
    pub ended: bool,
}

/// Replay all events (seq order) into projected session states.
pub async fn rebuild_sessions(
    pool: &SqlitePool,
) -> Result<BTreeMap<String, ProjectedSession>, StorageError> {
    let mut out: BTreeMap<String, ProjectedSession> = BTreeMap::new();
    let mut after = None;
    loop {
        let page = events::list_events(pool, None, None, None, after, 1000).await?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|e| e.seq);
        for ev in &page {
            let Some(session_id) = ev.session_id.clone() else {
                continue;
            };
            let payload: serde_json::Value =
                serde_json::from_str(&ev.payload).unwrap_or(serde_json::Value::Null);
            match ev.event_type.as_str() {
                crate::catalog::SESSION_STARTED => {
                    let snap = ev.snapshot_id.clone();
                    out.insert(
                        session_id.clone(),
                        ProjectedSession {
                            session_id,
                            state: "active".into(),
                            current_snapshot_id: snap.clone(),
                            start_snapshot_id: snap,
                            ended: false,
                        },
                    );
                }
                crate::catalog::SESSION_STOPPED => {
                    if let Some(s) = out.get_mut(&session_id) {
                        s.state = "stopped".into();
                        s.ended = true;
                        if let Some(fs) = payload.get("final_snapshot_id").and_then(|v| v.as_str())
                        {
                            s.current_snapshot_id = Some(fs.to_string());
                        }
                    }
                }
                crate::catalog::SESSION_INTERRUPTED => {
                    if let Some(s) = out.get_mut(&session_id) {
                        s.state = "interrupted".into();
                        s.ended = true;
                    }
                }
                crate::catalog::SESSION_RECOVERED => {
                    if let Some(s) = out.get_mut(&session_id) {
                        s.state = "active".into();
                        if let Some(fs) = ev.snapshot_id.clone() {
                            s.current_snapshot_id = Some(fs);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // repository.state_changed updates every live session in the worktree.
    // Second pass keeps ordering semantics simple: interleave via seq again.
    let mut after = None;
    let mut worktree_sessions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    loop {
        let page = events::list_events(pool, None, None, None, after, 1000).await?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|e| e.seq);
        for ev in &page {
            match ev.event_type.as_str() {
                crate::catalog::SESSION_STARTED => {
                    if let (Some(w), Some(s)) = (ev.worktree_id.clone(), ev.session_id.clone()) {
                        worktree_sessions.entry(w).or_default().push(s);
                    }
                }
                crate::catalog::REPOSITORY_STATE_CHANGED => {
                    let Some(w) = ev.worktree_id.clone() else {
                        continue;
                    };
                    let Some(to_snap) = ev.snapshot_id.clone() else {
                        continue;
                    };
                    for sid in worktree_sessions.get(&w).cloned().unwrap_or_default() {
                        if let Some(s) = out.get_mut(&sid) {
                            if !s.ended {
                                s.current_snapshot_id = Some(to_snap.clone());
                            }
                        }
                    }
                }
                crate::catalog::SESSION_STOPPED | crate::catalog::SESSION_INTERRUPTED => {
                    // ended sessions stop following state changes; `ended`
                    // flag already handled in first pass by seq order over
                    // the same total sequence.
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// Read the live sessions table into the same projected shape.
pub async fn live_sessions(
    pool: &SqlitePool,
) -> Result<BTreeMap<String, ProjectedSession>, StorageError> {
    let rows = cairn_storage_local::sessions::list(pool, None, None).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            // `recovering` is a runtime liveness state, not event-sourced:
            // replay reconstructs it as `active` (the last recorded state).
            let state = if r.state == "recovering" {
                "active".to_string()
            } else {
                r.state
            };
            let ended = state == "stopped" || state == "interrupted";
            (
                r.id.clone(),
                ProjectedSession {
                    session_id: r.id,
                    state,
                    current_snapshot_id: Some(r.current_snapshot_id),
                    start_snapshot_id: Some(r.start_snapshot_id),
                    ended,
                },
            )
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectProjectionState {
    pub projects: BTreeMap<ProjectId, Project>,
    pub associations: BTreeMap<String, ProjectRepositoryAssociation>,
    current_event_seq: Option<i64>,
}

struct ProjectCreatedReplay;

impl ReplayHandler<ProjectProjectionState> for ProjectCreatedReplay {
    fn event_type(&self) -> &'static str {
        crate::catalog::PROJECT_CREATED
    }

    fn schema_version(&self) -> u16 {
        1
    }

    fn apply(
        &self,
        state: &mut ProjectProjectionState,
        payload: &serde_json::Value,
    ) -> Result<(), ReplayDispatchError> {
        let payload: crate::catalog::ProjectCreatedPayload =
            serde_json::from_value(payload.clone())
                .map_err(|_| ReplayDispatchError::InvalidPayload(self.event_type().into()))?;
        let project: Project = payload.project.into();
        if state.projects.insert(project.id, project).is_some() {
            return Err(ReplayDispatchError::InvalidPayload(
                self.event_type().into(),
            ));
        }
        Ok(())
    }
}

struct ProjectUpdatedReplay;

impl ReplayHandler<ProjectProjectionState> for ProjectUpdatedReplay {
    fn event_type(&self) -> &'static str {
        crate::catalog::PROJECT_UPDATED
    }

    fn schema_version(&self) -> u16 {
        1
    }

    fn apply(
        &self,
        state: &mut ProjectProjectionState,
        payload: &serde_json::Value,
    ) -> Result<(), ReplayDispatchError> {
        let payload: crate::catalog::ProjectUpdatedPayload =
            serde_json::from_value(payload.clone())
                .map_err(|_| ReplayDispatchError::InvalidPayload(self.event_type().into()))?;
        let project: Project = payload.project.into();
        if !state.projects.contains_key(&project.id) {
            return Err(ReplayDispatchError::InvalidPayload(
                self.event_type().into(),
            ));
        }
        state.projects.insert(project.id, project);
        Ok(())
    }
}

struct ProjectRepositoryAssociatedReplay;

impl ReplayHandler<ProjectProjectionState> for ProjectRepositoryAssociatedReplay {
    fn event_type(&self) -> &'static str {
        crate::catalog::PROJECT_REPOSITORY_ASSOCIATED
    }

    fn schema_version(&self) -> u16 {
        1
    }

    fn apply(
        &self,
        state: &mut ProjectProjectionState,
        payload: &serde_json::Value,
    ) -> Result<(), ReplayDispatchError> {
        let payload: crate::catalog::ProjectRepositoryAssociatedPayload =
            serde_json::from_value(payload.clone())
                .map_err(|_| ReplayDispatchError::InvalidPayload(self.event_type().into()))?;
        if !state.projects.contains_key(&payload.association.project_id) {
            return Err(ReplayDispatchError::InvalidPayload(
                self.event_type().into(),
            ));
        }
        let event_seq = state
            .current_event_seq
            .ok_or_else(|| ReplayDispatchError::InvalidPayload(self.event_type().into()))?;
        let association = ProjectRepositoryAssociation::new(
            payload.association.association_id,
            payload.association.project_id,
            payload.association.repository_id.clone(),
            payload.association.associated_at,
            event_seq,
        )
        .map_err(|_| ReplayDispatchError::InvalidPayload(self.event_type().into()))?;
        if state
            .associations
            .insert(payload.association.repository_id, association)
            .is_some()
        {
            return Err(ReplayDispatchError::InvalidPayload(
                self.event_type().into(),
            ));
        }
        Ok(())
    }
}

pub fn project_replay_dispatcher(
) -> Result<ReplayDispatcher<ProjectProjectionState>, ReplayDispatchError> {
    let mut dispatcher = ReplayDispatcher::default();
    dispatcher.register(ProjectCreatedReplay)?;
    dispatcher.register(ProjectUpdatedReplay)?;
    dispatcher.register(ProjectRepositoryAssociatedReplay)?;
    Ok(dispatcher)
}

pub async fn rebuild_project_projections(
    pool: &SqlitePool,
) -> Result<ProjectProjectionState, StorageError> {
    let dispatcher = project_replay_dispatcher()
        .map_err(|_| StorageError::Corrupted("project replay dispatcher is invalid".into()))?;
    let mut state = ProjectProjectionState::default();
    let mut aggregate_heads: BTreeMap<(String, String), i64> = BTreeMap::new();
    let mut after = None;
    loop {
        let page = events::list_events(pool, None, None, None, after, 1000).await?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|event| event.seq);
        for event in page {
            validate_event_sequence_if_scoped(&event, &mut aggregate_heads)?;
            if !matches!(
                event.event_type.as_str(),
                crate::catalog::PROJECT_CREATED
                    | crate::catalog::PROJECT_UPDATED
                    | crate::catalog::PROJECT_REPOSITORY_ASSOCIATED
            ) {
                continue;
            }
            validate_project_event_scope(&event)?;
            let payload = serde_json::from_str(&event.payload)
                .map_err(|_| StorageError::Corrupted("project event payload is invalid".into()))?;
            state.current_event_seq = Some(event.seq);
            dispatcher
                .dispatch(&mut state, &event.event_type, &payload)
                .map_err(|_| StorageError::Corrupted("project event replay failed".into()))?;
            state.current_event_seq = None;
        }
    }
    Ok(state)
}

pub async fn live_project_projections(
    pool: &SqlitePool,
) -> Result<ProjectProjectionState, StorageError> {
    let project_rows = projects::list(pool, None, u32::MAX).await?;
    let association_rows = projects::list_all_associations(pool).await?;
    let projects = project_rows
        .into_iter()
        .map(|row| project_from_row(row).map(|project| (project.id, project)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let associations = association_rows
        .into_iter()
        .map(|row| {
            association_from_row(row)
                .map(|association| (association.repository_id.clone(), association))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(ProjectProjectionState {
        projects,
        associations,
        current_event_seq: None,
    })
}

fn validate_event_sequence_if_scoped(
    event: &events::EventRow,
    heads: &mut BTreeMap<(String, String), i64>,
) -> Result<(), StorageError> {
    let (aggregate_type, aggregate_id, aggregate_seq) = match (
        &event.aggregate_type,
        &event.aggregate_id,
        event.aggregate_seq,
    ) {
        (None, None, None) => return Ok(()),
        (Some(aggregate_type), Some(aggregate_id), Some(aggregate_seq)) => (
            aggregate_type.as_str(),
            aggregate_id.as_str(),
            aggregate_seq,
        ),
        _ => {
            return Err(StorageError::Corrupted(
                "event aggregate scope is incomplete".into(),
            ))
        }
    };
    if aggregate_id.trim().is_empty() {
        return Err(StorageError::Corrupted(
            "event aggregate scope is invalid".into(),
        ));
    }
    let key = (aggregate_type.to_string(), aggregate_id.to_string());
    let expected_seq = heads.get(&key).copied().unwrap_or(0) + 1;
    if aggregate_seq != expected_seq {
        return Err(StorageError::Corrupted(
            "event aggregate sequence has a gap".into(),
        ));
    }
    heads.insert(key, aggregate_seq);
    Ok(())
}

fn validate_project_event_scope(event: &events::EventRow) -> Result<(), StorageError> {
    let aggregate_type = event
        .aggregate_type
        .as_deref()
        .ok_or_else(|| StorageError::Corrupted("project event aggregate type is missing".into()))?;
    let aggregate_id = event
        .aggregate_id
        .as_deref()
        .ok_or_else(|| StorageError::Corrupted("project event aggregate id is missing".into()))?;
    let expected_type = if event.event_type == crate::catalog::PROJECT_REPOSITORY_ASSOCIATED {
        "repository"
    } else {
        "project"
    };
    if aggregate_type != expected_type || aggregate_id.trim().is_empty() {
        return Err(StorageError::Corrupted(
            "project event aggregate scope is invalid".into(),
        ));
    }
    Ok(())
}

fn project_from_row(row: cairn_storage_local::ProjectRow) -> Result<Project, StorageError> {
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

fn association_from_row(
    row: cairn_storage_local::ProjectRepositoryAssociationRow,
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskProjectionState {
    pub tasks: BTreeMap<TaskId, Task>,
    pub revisions: BTreeMap<TaskRevisionId, TaskRevision>,
}

struct TaskCreatedReplay;

impl ReplayHandler<TaskProjectionState> for TaskCreatedReplay {
    fn event_type(&self) -> &'static str {
        crate::catalog::TASK_CREATED
    }

    fn schema_version(&self) -> u16 {
        1
    }

    fn apply(
        &self,
        state: &mut TaskProjectionState,
        payload: &serde_json::Value,
    ) -> Result<(), ReplayDispatchError> {
        let payload: crate::catalog::TaskCreatedPayload =
            serde_json::from_value(payload.clone())
                .map_err(|_| ReplayDispatchError::InvalidPayload(self.event_type().into()))?;
        let task = Task {
            id: payload.task.task_id,
            project_id: payload.task.project_id,
            title: payload.task.title,
            latest_revision_number: 1,
            created_at: payload.task.created_at,
            updated_at: payload.task.created_at,
        };
        if state.tasks.insert(task.id, task).is_some() {
            return Err(ReplayDispatchError::InvalidPayload(
                self.event_type().into(),
            ));
        }
        Ok(())
    }
}

struct TaskRevisionCreatedReplay;

impl ReplayHandler<TaskProjectionState> for TaskRevisionCreatedReplay {
    fn event_type(&self) -> &'static str {
        crate::catalog::TASK_REVISION_CREATED
    }

    fn schema_version(&self) -> u16 {
        1
    }

    fn apply(
        &self,
        state: &mut TaskProjectionState,
        payload: &serde_json::Value,
    ) -> Result<(), ReplayDispatchError> {
        let payload: crate::catalog::TaskRevisionCreatedPayload =
            serde_json::from_value(payload.clone())
                .map_err(|_| ReplayDispatchError::InvalidPayload(self.event_type().into()))?;
        let revision = TaskRevision::try_from(payload.revision)
            .map_err(|_| ReplayDispatchError::InvalidPayload(self.event_type().into()))?;
        let task: Task = payload.task.into();
        let current = state
            .tasks
            .get(&task.id)
            .ok_or_else(|| ReplayDispatchError::InvalidPayload(self.event_type().into()))?;
        let stable_task_fields_match = current.project_id == task.project_id
            && current.title == task.title
            && current.created_at == task.created_at
            && revision.task_id == task.id
            && revision.revision_number == task.latest_revision_number;
        let number_advances = (revision.revision_number == 1
            && current.latest_revision_number == 1
            && revision.parent_revision_id.is_none())
            || (revision.revision_number == current.latest_revision_number + 1);
        if !stable_task_fields_match || !number_advances {
            return Err(ReplayDispatchError::InvalidPayload(
                self.event_type().into(),
            ));
        }
        let parent = revision
            .parent_revision_id
            .and_then(|parent_id| state.revisions.get(&parent_id));
        revision
            .validate_parent(parent)
            .map_err(|_| ReplayDispatchError::InvalidPayload(self.event_type().into()))?;
        if state.revisions.insert(revision.id, revision).is_some() {
            return Err(ReplayDispatchError::InvalidPayload(
                self.event_type().into(),
            ));
        }
        state.tasks.insert(task.id, task);
        Ok(())
    }
}

pub fn task_replay_dispatcher() -> Result<ReplayDispatcher<TaskProjectionState>, ReplayDispatchError>
{
    let mut dispatcher = ReplayDispatcher::default();
    dispatcher.register(TaskCreatedReplay)?;
    dispatcher.register(TaskRevisionCreatedReplay)?;
    Ok(dispatcher)
}

pub async fn rebuild_task_projections(
    pool: &SqlitePool,
) -> Result<TaskProjectionState, StorageError> {
    let dispatcher = task_replay_dispatcher()
        .map_err(|_| StorageError::Corrupted("task replay dispatcher is invalid".into()))?;
    let mut state = TaskProjectionState::default();
    let mut aggregate_heads: BTreeMap<(String, String), i64> = BTreeMap::new();
    let mut after = None;
    loop {
        let page = events::list_events(pool, None, None, None, after, 1000).await?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|event| event.seq);
        for event in page {
            validate_event_sequence_if_scoped(&event, &mut aggregate_heads)?;
            if !matches!(
                event.event_type.as_str(),
                crate::catalog::TASK_CREATED | crate::catalog::TASK_REVISION_CREATED
            ) {
                continue;
            }
            if event.aggregate_type.as_deref() != Some("task")
                || event.aggregate_id.as_deref().is_none()
                || event.repository_id.is_some()
                || event.worktree_id.is_some()
                || event.session_id.is_some()
            {
                return Err(StorageError::Corrupted(
                    "task event scope is invalid".into(),
                ));
            }
            let payload = serde_json::from_str(&event.payload)
                .map_err(|_| StorageError::Corrupted("task event payload is invalid".into()))?;
            dispatcher
                .dispatch(&mut state, &event.event_type, &payload)
                .map_err(|_| StorageError::Corrupted("task event replay failed".into()))?;
        }
    }
    Ok(state)
}

pub async fn live_task_projections(pool: &SqlitePool) -> Result<TaskProjectionState, StorageError> {
    let tasks = tasks::list_all(pool)
        .await?
        .into_iter()
        .map(|row| task_from_storage_row(row).map(|task| (task.id, task)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let revisions = tasks::list_all_revisions(pool)
        .await?
        .into_iter()
        .map(|row| revision_from_storage_row(row).map(|revision| (revision.id, revision)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(TaskProjectionState { tasks, revisions })
}

fn task_from_storage_row(row: cairn_storage_local::TaskRow) -> Result<Task, StorageError> {
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

fn revision_from_storage_row(
    row: cairn_storage_local::TaskRevisionRow,
) -> Result<TaskRevision, StorageError> {
    let goal_contract =
        cairn_domain::GoalContractV1::from_json_slice(row.goal_contract_json.as_bytes())
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
            .map_err(|_| StorageError::Corrupted("invalid task revision number".into()))?,
        row.parent_revision_id
            .map(|id| TaskRevisionId::from_str(&id))
            .transpose()
            .map_err(|_| StorageError::Corrupted("invalid parent revision id".into()))?,
        goal_contract,
        Timestamp::parse(&row.created_at)
            .map_err(|_| StorageError::Corrupted("invalid task revision timestamp".into()))?,
    )
    .map_err(|_| StorageError::Corrupted("invalid task revision".into()))
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionBindingProjectionState {
    pub scopes: BTreeMap<SessionId, SessionBindingMode>,
    pub bindings: BTreeMap<SessionId, SessionBinding>,
    current_event_seq: Option<i64>,
}

struct SessionBoundReplay;

impl ReplayHandler<SessionBindingProjectionState> for SessionBoundReplay {
    fn event_type(&self) -> &'static str {
        crate::catalog::SESSION_BOUND
    }

    fn schema_version(&self) -> u16 {
        1
    }

    fn apply(
        &self,
        state: &mut SessionBindingProjectionState,
        payload: &serde_json::Value,
    ) -> Result<(), ReplayDispatchError> {
        let payload: crate::catalog::SessionBoundPayload = serde_json::from_value(payload.clone())
            .map_err(|_| ReplayDispatchError::InvalidPayload(self.event_type().into()))?;
        if state.scopes.get(&payload.binding.session_id) != Some(&SessionBindingMode::LocalUnbound)
        {
            return Err(ReplayDispatchError::InvalidPayload(
                self.event_type().into(),
            ));
        }
        let binding = SessionBinding {
            session_id: payload.binding.session_id,
            project_id: payload.binding.project_id,
            task_revision_id: payload.binding.task_revision_id,
            bound_at: payload.binding.bound_at,
            binding_event_seq: state
                .current_event_seq
                .ok_or_else(|| ReplayDispatchError::InvalidPayload(self.event_type().into()))?,
        };
        if state.bindings.insert(binding.session_id, binding).is_some() {
            return Err(ReplayDispatchError::InvalidPayload(
                self.event_type().into(),
            ));
        }
        state.scopes.insert(
            binding.session_id,
            SessionBindingMode::ProjectBound {
                project_id: binding.project_id,
                task_revision_id: binding.task_revision_id,
            },
        );
        Ok(())
    }
}

pub fn session_binding_replay_dispatcher(
) -> Result<ReplayDispatcher<SessionBindingProjectionState>, ReplayDispatchError> {
    let mut dispatcher = ReplayDispatcher::default();
    dispatcher.register(SessionBoundReplay)?;
    Ok(dispatcher)
}

pub async fn rebuild_session_binding_projections(
    pool: &SqlitePool,
) -> Result<SessionBindingProjectionState, StorageError> {
    let projects = rebuild_project_projections(pool).await?;
    let tasks = rebuild_task_projections(pool).await?;
    let dispatcher = session_binding_replay_dispatcher().map_err(|_| {
        StorageError::Corrupted("session binding replay dispatcher is invalid".into())
    })?;
    let mut state = SessionBindingProjectionState::default();
    let mut session_origins: BTreeMap<SessionId, (String, String)> = BTreeMap::new();
    let mut aggregate_heads: BTreeMap<(String, String), i64> = BTreeMap::new();
    let mut after = None;
    loop {
        let page = events::list_events(pool, None, None, None, after, 1000).await?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|event| event.seq);
        for event in page {
            validate_event_sequence_if_scoped(&event, &mut aggregate_heads)?;
            if event.event_type == crate::catalog::SESSION_STARTED {
                let session_id = event
                    .session_id
                    .as_deref()
                    .and_then(|id| SessionId::from_str(id).ok())
                    .ok_or_else(|| {
                        StorageError::Corrupted("session.started session scope is invalid".into())
                    })?;
                if let (Some(aggregate_type), Some(aggregate_id)) = (
                    event.aggregate_type.as_deref(),
                    event.aggregate_id.as_deref(),
                ) {
                    if aggregate_type != "session" || aggregate_id != session_id.to_string() {
                        return Err(StorageError::Corrupted(
                            "session.started aggregate scope is invalid".into(),
                        ));
                    }
                }
                let repository_id = event.repository_id.clone().ok_or_else(|| {
                    StorageError::Corrupted("session.started repository is missing".into())
                })?;
                let worktree_id = event.worktree_id.clone().ok_or_else(|| {
                    StorageError::Corrupted("session.started worktree is missing".into())
                })?;
                if state
                    .scopes
                    .insert(session_id, SessionBindingMode::LocalUnbound)
                    .is_some()
                    || session_origins
                        .insert(session_id, (repository_id, worktree_id))
                        .is_some()
                {
                    return Err(StorageError::Corrupted(
                        "duplicate session.started event".into(),
                    ));
                }
                continue;
            }
            if event.event_type != crate::catalog::SESSION_BOUND {
                continue;
            }
            if event.aggregate_type.as_deref() != Some("session")
                || event.aggregate_id.as_deref() != event.session_id.as_deref()
                || event.session_id.is_none()
                || event.repository_id.is_none()
                || event.worktree_id.is_none()
            {
                return Err(StorageError::Corrupted(
                    "session.bound aggregate scope is invalid".into(),
                ));
            }
            let payload: crate::catalog::SessionBoundPayload = serde_json::from_str(&event.payload)
                .map_err(|_| StorageError::Corrupted("session.bound payload is invalid".into()))?;
            if event.session_id.as_deref() != Some(payload.binding.session_id.to_string().as_str())
                || event.repository_id.as_deref() != Some(payload.binding.repository_id.as_str())
                || event.worktree_id.as_deref() != Some(payload.binding.worktree_id.as_str())
                || session_origins.get(&payload.binding.session_id)
                    != Some(&(
                        payload.binding.repository_id.clone(),
                        payload.binding.worktree_id.clone(),
                    ))
            {
                return Err(StorageError::Corrupted(
                    "session.bound provenance is invalid".into(),
                ));
            }
            let association = projects
                .associations
                .get(&payload.binding.repository_id)
                .filter(|association| association.project_id == payload.binding.project_id)
                .ok_or_else(|| {
                    StorageError::Corrupted(
                        "session.bound repository association is invalid".into(),
                    )
                })?;
            let revision = tasks
                .revisions
                .get(&payload.binding.task_revision_id)
                .filter(|revision| revision.task_id == payload.binding.task_id)
                .ok_or_else(|| {
                    StorageError::Corrupted("session.bound task revision is invalid".into())
                })?;
            let task = tasks
                .tasks
                .get(&revision.task_id)
                .filter(|task| task.project_id == association.project_id)
                .ok_or_else(|| {
                    StorageError::Corrupted("session.bound task project is invalid".into())
                })?;
            if task.project_id != payload.binding.project_id {
                return Err(StorageError::Corrupted(
                    "session.bound project is invalid".into(),
                ));
            }
            state.current_event_seq = Some(event.seq);
            let value = serde_json::to_value(payload)
                .map_err(|_| StorageError::Corrupted("session.bound payload is invalid".into()))?;
            dispatcher
                .dispatch(&mut state, &event.event_type, &value)
                .map_err(|_| StorageError::Corrupted("session binding replay failed".into()))?;
            state.current_event_seq = None;
        }
    }
    Ok(state)
}

pub async fn live_session_binding_projections(
    pool: &SqlitePool,
) -> Result<SessionBindingProjectionState, StorageError> {
    let mut scopes = BTreeMap::new();
    for row in cairn_storage_local::sessions::list(pool, None, None).await? {
        let session_id = SessionId::from_str(&row.id)
            .map_err(|_| StorageError::Corrupted("invalid session id".into()))?;
        let scope = match row.binding_mode.as_str() {
            "local_unbound" => SessionBindingMode::LocalUnbound,
            "project_bound" => {
                let binding = session_bindings::get(pool, &row.id).await?.ok_or_else(|| {
                    StorageError::Corrupted("bound session projection is missing".into())
                })?;
                SessionBindingMode::ProjectBound {
                    project_id: ProjectId::from_str(&binding.project_id).map_err(|_| {
                        StorageError::Corrupted("invalid binding project id".into())
                    })?,
                    task_revision_id: TaskRevisionId::from_str(&binding.task_revision_id).map_err(
                        |_| StorageError::Corrupted("invalid binding revision id".into()),
                    )?,
                }
            }
            _ => {
                return Err(StorageError::Corrupted(
                    "invalid session binding mode".into(),
                ))
            }
        };
        scopes.insert(session_id, scope);
    }
    let mut bindings = BTreeMap::new();
    for row in session_bindings::list_all(pool).await? {
        let binding = SessionBinding {
            session_id: SessionId::from_str(&row.session_id)
                .map_err(|_| StorageError::Corrupted("invalid binding session id".into()))?,
            project_id: ProjectId::from_str(&row.project_id)
                .map_err(|_| StorageError::Corrupted("invalid binding project id".into()))?,
            task_revision_id: TaskRevisionId::from_str(&row.task_revision_id)
                .map_err(|_| StorageError::Corrupted("invalid binding revision id".into()))?,
            bound_at: Timestamp::parse(&row.bound_at)
                .map_err(|_| StorageError::Corrupted("invalid binding timestamp".into()))?,
            binding_event_seq: row.binding_event_seq,
        };
        bindings.insert(binding.session_id, binding);
    }
    Ok(SessionBindingProjectionState {
        scopes,
        bindings,
        current_event_seq: None,
    })
}

/// Stable projections reconstructed from one globally ordered mixed Feature 001/002 ledger.
/// Runtime-only lease and token material deliberately remains outside replay state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MixedProjectionState {
    pub repositories: BTreeSet<String>,
    pub worktrees: BTreeMap<String, String>,
    pub sessions: BTreeMap<String, ProjectedSession>,
    pub projects: ProjectProjectionState,
    pub tasks: TaskProjectionState,
    pub session_bindings: SessionBindingProjectionState,
}

/// Closed, content-free failures returned by strict mixed-ledger replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MixedReplayError {
    #[error("mixed ledger storage failure")]
    StorageFailure,
    #[error("mixed ledger global order is invalid")]
    InvalidGlobalOrder,
    #[error("mixed ledger event type is unsupported")]
    UnsupportedEventType,
    #[error("mixed ledger payload version is unsupported")]
    UnsupportedPayloadVersion,
    #[error("mixed ledger payload is malformed")]
    MalformedPayload,
    #[error("mixed ledger aggregate sequence is invalid")]
    InvalidAggregateSequence,
    #[error("mixed ledger aggregate scope is invalid")]
    InvalidAggregateScope,
    #[error("mixed ledger reference is missing or inconsistent")]
    InvalidReference,
    #[error("mixed ledger contains a conflicting session binding")]
    ConflictingBinding,
    #[error("mixed replay differs from live projections")]
    ProjectionMismatch,
}

/// Strictly replay a complete ordered ledger without mutating either the source ledger or
/// live projection tables. This is the integration point for the story-owned handlers.
pub fn replay_mixed_rows(
    rows: &[events::EventRow],
) -> Result<MixedProjectionState, MixedReplayError> {
    let project_dispatcher = project_replay_dispatcher().map_err(map_dispatch_error)?;
    let task_dispatcher = task_replay_dispatcher().map_err(map_dispatch_error)?;
    let binding_dispatcher = session_binding_replay_dispatcher().map_err(map_dispatch_error)?;
    let mut state = MixedProjectionState::default();
    let mut aggregate_heads = BTreeMap::new();
    let mut origins: BTreeMap<SessionId, (String, String)> = BTreeMap::new();
    let mut worktree_sessions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut previous_global_seq = None;

    for event in rows {
        if previous_global_seq.is_some_and(|previous| event.seq <= previous) {
            return Err(MixedReplayError::InvalidGlobalOrder);
        }
        previous_global_seq = Some(event.seq);
        if !crate::catalog::ALL_EVENT_TYPES.contains(&event.event_type.as_str()) {
            return Err(MixedReplayError::UnsupportedEventType);
        }
        validate_event_sequence_if_scoped(event, &mut aggregate_heads)
            .map_err(|_| MixedReplayError::InvalidAggregateSequence)?;
        validate_mixed_scope(event)?;
        let payload: serde_json::Value =
            serde_json::from_str(&event.payload).map_err(|_| MixedReplayError::MalformedPayload)?;

        match event.event_type.as_str() {
            crate::catalog::REPOSITORY_REGISTERED => {
                let repository_id = event
                    .repository_id
                    .clone()
                    .ok_or(MixedReplayError::InvalidAggregateScope)?;
                if !state.repositories.insert(repository_id) {
                    return Err(MixedReplayError::InvalidReference);
                }
            }
            crate::catalog::WORKTREE_REGISTERED => {
                let repository_id = event
                    .repository_id
                    .clone()
                    .ok_or(MixedReplayError::InvalidAggregateScope)?;
                let worktree_id = event
                    .worktree_id
                    .clone()
                    .ok_or(MixedReplayError::InvalidAggregateScope)?;
                if !state.repositories.contains(&repository_id)
                    || state.worktrees.insert(worktree_id, repository_id).is_some()
                {
                    return Err(MixedReplayError::InvalidReference);
                }
            }
            crate::catalog::PROJECT_CREATED => {
                let decoded: crate::catalog::ProjectCreatedPayload =
                    serde_json::from_value(payload.clone())
                        .map_err(|_| MixedReplayError::MalformedPayload)?;
                if event.aggregate_id.as_deref()
                    != Some(decoded.project.project_id.to_string().as_str())
                {
                    return Err(MixedReplayError::InvalidAggregateScope);
                }
                state.projects.current_event_seq = Some(event.seq);
                project_dispatcher
                    .dispatch(&mut state.projects, &event.event_type, &payload)
                    .map_err(map_dispatch_error)?;
                state.projects.current_event_seq = None;
            }
            crate::catalog::PROJECT_UPDATED => {
                let decoded: crate::catalog::ProjectUpdatedPayload =
                    serde_json::from_value(payload.clone())
                        .map_err(|_| MixedReplayError::MalformedPayload)?;
                if event.aggregate_id.as_deref()
                    != Some(decoded.project.project_id.to_string().as_str())
                {
                    return Err(MixedReplayError::InvalidAggregateScope);
                }
                state.projects.current_event_seq = Some(event.seq);
                project_dispatcher
                    .dispatch(&mut state.projects, &event.event_type, &payload)
                    .map_err(map_dispatch_error)?;
                state.projects.current_event_seq = None;
            }
            crate::catalog::PROJECT_REPOSITORY_ASSOCIATED => {
                let decoded: crate::catalog::ProjectRepositoryAssociatedPayload =
                    serde_json::from_value(payload.clone())
                        .map_err(|_| MixedReplayError::MalformedPayload)?;
                if !state
                    .repositories
                    .contains(&decoded.association.repository_id)
                    || event.aggregate_id.as_deref()
                        != Some(decoded.association.repository_id.as_str())
                {
                    return Err(MixedReplayError::InvalidReference);
                }
                state.projects.current_event_seq = Some(event.seq);
                project_dispatcher
                    .dispatch(&mut state.projects, &event.event_type, &payload)
                    .map_err(map_dispatch_error)?;
                state.projects.current_event_seq = None;
            }
            crate::catalog::TASK_CREATED => {
                let decoded: crate::catalog::TaskCreatedPayload =
                    serde_json::from_value(payload.clone())
                        .map_err(|_| MixedReplayError::MalformedPayload)?;
                if !state
                    .projects
                    .projects
                    .contains_key(&decoded.task.project_id)
                {
                    return Err(MixedReplayError::InvalidReference);
                }
                if event.aggregate_id.as_deref() != Some(decoded.task.task_id.to_string().as_str())
                {
                    return Err(MixedReplayError::InvalidAggregateScope);
                }
                task_dispatcher
                    .dispatch(&mut state.tasks, &event.event_type, &payload)
                    .map_err(map_dispatch_error)?;
            }
            crate::catalog::TASK_REVISION_CREATED => {
                let decoded: crate::catalog::TaskRevisionCreatedPayload =
                    serde_json::from_value(payload.clone())
                        .map_err(|_| MixedReplayError::MalformedPayload)?;
                if event.aggregate_id.as_deref() != Some(decoded.task.task_id.to_string().as_str())
                {
                    return Err(MixedReplayError::InvalidAggregateScope);
                }
                task_dispatcher
                    .dispatch(&mut state.tasks, &event.event_type, &payload)
                    .map_err(map_dispatch_error)?;
            }
            crate::catalog::SESSION_STARTED => {
                let _: crate::catalog::SessionStartedPayload = serde_json::from_value(payload)
                    .map_err(|_| MixedReplayError::MalformedPayload)?;
                let session_id_text = event
                    .session_id
                    .clone()
                    .ok_or(MixedReplayError::InvalidAggregateScope)?;
                let session_id = SessionId::from_str(&session_id_text)
                    .map_err(|_| MixedReplayError::InvalidReference)?;
                let repository_id = event
                    .repository_id
                    .clone()
                    .ok_or(MixedReplayError::InvalidAggregateScope)?;
                let worktree_id = event
                    .worktree_id
                    .clone()
                    .ok_or(MixedReplayError::InvalidAggregateScope)?;
                if !state.repositories.contains(&repository_id)
                    || state.worktrees.get(&worktree_id) != Some(&repository_id)
                    || state
                        .sessions
                        .insert(
                            session_id_text.clone(),
                            ProjectedSession {
                                session_id: session_id_text.clone(),
                                state: "active".into(),
                                current_snapshot_id: event.snapshot_id.clone(),
                                start_snapshot_id: event.snapshot_id.clone(),
                                ended: false,
                            },
                        )
                        .is_some()
                    || state
                        .session_bindings
                        .scopes
                        .insert(session_id, SessionBindingMode::LocalUnbound)
                        .is_some()
                    || origins
                        .insert(session_id, (repository_id, worktree_id.clone()))
                        .is_some()
                {
                    return Err(MixedReplayError::InvalidReference);
                }
                worktree_sessions
                    .entry(worktree_id)
                    .or_default()
                    .push(session_id_text);
            }
            crate::catalog::REPOSITORY_STATE_CHANGED => {
                let worktree_id = event
                    .worktree_id
                    .as_ref()
                    .ok_or(MixedReplayError::InvalidAggregateScope)?;
                let snapshot_id = event
                    .snapshot_id
                    .clone()
                    .or_else(|| {
                        payload
                            .get("to_snapshot_id")
                            .and_then(serde_json::Value::as_str)
                            .map(String::from)
                    })
                    .ok_or(MixedReplayError::MalformedPayload)?;
                for session_id in worktree_sessions.get(worktree_id).into_iter().flatten() {
                    if let Some(session) = state.sessions.get_mut(session_id) {
                        if !session.ended {
                            session.current_snapshot_id = Some(snapshot_id.clone());
                        }
                    }
                }
            }
            crate::catalog::SESSION_STOPPED => {
                let session = session_for_event_mut(&mut state, event)?;
                session.state = "stopped".into();
                session.ended = true;
                session.current_snapshot_id = payload
                    .get("final_snapshot_id")
                    .and_then(serde_json::Value::as_str)
                    .map(String::from)
                    .or_else(|| event.snapshot_id.clone());
            }
            crate::catalog::SESSION_INTERRUPTED => {
                let session = session_for_event_mut(&mut state, event)?;
                session.state = "interrupted".into();
                session.ended = true;
            }
            crate::catalog::SESSION_RECOVERED => {
                let session = session_for_event_mut(&mut state, event)?;
                session.state = "active".into();
                session.ended = false;
                if let Some(snapshot_id) = event.snapshot_id.clone() {
                    session.current_snapshot_id = Some(snapshot_id);
                }
            }
            crate::catalog::SESSION_BOUND => {
                let decoded: crate::catalog::SessionBoundPayload =
                    serde_json::from_value(payload.clone())
                        .map_err(|_| MixedReplayError::MalformedPayload)?;
                if origins.get(&decoded.binding.session_id)
                    != Some(&(
                        decoded.binding.repository_id.clone(),
                        decoded.binding.worktree_id.clone(),
                    ))
                    || state
                        .projects
                        .associations
                        .get(&decoded.binding.repository_id)
                        .is_none_or(|association| {
                            association.project_id != decoded.binding.project_id
                        })
                    || state
                        .tasks
                        .revisions
                        .get(&decoded.binding.task_revision_id)
                        .is_none_or(|revision| revision.task_id != decoded.binding.task_id)
                    || state
                        .tasks
                        .tasks
                        .get(&decoded.binding.task_id)
                        .is_none_or(|task| task.project_id != decoded.binding.project_id)
                {
                    return Err(MixedReplayError::InvalidReference);
                }
                state.session_bindings.current_event_seq = Some(event.seq);
                binding_dispatcher
                    .dispatch(&mut state.session_bindings, &event.event_type, &payload)
                    .map_err(|error| match error {
                        ReplayDispatchError::InvalidPayload(_) => {
                            MixedReplayError::ConflictingBinding
                        }
                        other => map_dispatch_error(other),
                    })?;
                state.session_bindings.current_event_seq = None;
            }
            crate::catalog::SNAPSHOT_CREATED
            | crate::catalog::BRANCH_CHANGED
            | crate::catalog::SESSION_REATTACH_REJECTED
            | crate::catalog::IDENTITY_MARKER_RESTORED => {}
            _ => return Err(MixedReplayError::UnsupportedEventType),
        }
    }
    Ok(state)
}

pub async fn rebuild_mixed_projections(
    pool: &SqlitePool,
) -> Result<MixedProjectionState, MixedReplayError> {
    let mut rows = Vec::new();
    let mut after = None;
    loop {
        let page = events::list_events(pool, None, None, None, after, 1000)
            .await
            .map_err(|_| MixedReplayError::StorageFailure)?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|event| event.seq);
        rows.extend(page);
    }
    replay_mixed_rows(&rows)
}

pub async fn live_mixed_projections(
    pool: &SqlitePool,
) -> Result<MixedProjectionState, MixedReplayError> {
    let repositories = sqlx::query_scalar::<_, String>("SELECT id FROM repositories ORDER BY id")
        .fetch_all(pool)
        .await
        .map_err(|_| MixedReplayError::StorageFailure)?
        .into_iter()
        .collect();
    let worktrees = sqlx::query_as::<_, (String, String)>(
        "SELECT id, repository_id FROM worktrees ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| MixedReplayError::StorageFailure)?
    .into_iter()
    .collect();
    Ok(MixedProjectionState {
        repositories,
        worktrees,
        sessions: live_sessions(pool)
            .await
            .map_err(|_| MixedReplayError::StorageFailure)?,
        projects: live_project_projections(pool)
            .await
            .map_err(|_| MixedReplayError::StorageFailure)?,
        tasks: live_task_projections(pool)
            .await
            .map_err(|_| MixedReplayError::StorageFailure)?,
        session_bindings: live_session_binding_projections(pool)
            .await
            .map_err(|_| MixedReplayError::StorageFailure)?,
    })
}

pub async fn verify_mixed_projections(pool: &SqlitePool) -> Result<(), MixedReplayError> {
    let rebuilt = rebuild_mixed_projections(pool).await?;
    let live = live_mixed_projections(pool).await?;
    if rebuilt == live {
        Ok(())
    } else {
        Err(MixedReplayError::ProjectionMismatch)
    }
}

fn session_for_event_mut<'a>(
    state: &'a mut MixedProjectionState,
    event: &events::EventRow,
) -> Result<&'a mut ProjectedSession, MixedReplayError> {
    let session_id = event
        .session_id
        .as_ref()
        .ok_or(MixedReplayError::InvalidAggregateScope)?;
    state
        .sessions
        .get_mut(session_id)
        .ok_or(MixedReplayError::InvalidReference)
}

fn map_dispatch_error(error: ReplayDispatchError) -> MixedReplayError {
    match error {
        ReplayDispatchError::UnsupportedEventType(_) => MixedReplayError::UnsupportedEventType,
        ReplayDispatchError::UnsupportedPayloadVersion { .. } => {
            MixedReplayError::UnsupportedPayloadVersion
        }
        ReplayDispatchError::MalformedPayload(_) => MixedReplayError::MalformedPayload,
        ReplayDispatchError::DuplicateHandler(_) | ReplayDispatchError::InvalidPayload(_) => {
            MixedReplayError::InvalidReference
        }
    }
}

fn validate_mixed_scope(event: &events::EventRow) -> Result<(), MixedReplayError> {
    let scope = match (
        event.aggregate_type.as_deref(),
        event.aggregate_id.as_deref(),
        event.aggregate_seq,
    ) {
        (None, None, None) => {
            return if crate::catalog::FEATURE002_EVENT_TYPES.contains(&event.event_type.as_str()) {
                Err(MixedReplayError::InvalidAggregateScope)
            } else {
                Ok(())
            }
        }
        (Some(kind), Some(id), Some(_)) if !id.trim().is_empty() => (kind, id),
        _ => return Err(MixedReplayError::InvalidAggregateScope),
    };

    let expected = match event.event_type.as_str() {
        crate::catalog::REPOSITORY_REGISTERED => ("repository", event.repository_id.as_deref()),
        crate::catalog::WORKTREE_REGISTERED
        | crate::catalog::SNAPSHOT_CREATED
        | crate::catalog::REPOSITORY_STATE_CHANGED
        | crate::catalog::BRANCH_CHANGED => ("worktree", event.worktree_id.as_deref()),
        crate::catalog::SESSION_STARTED
        | crate::catalog::SESSION_STOPPED
        | crate::catalog::SESSION_INTERRUPTED
        | crate::catalog::SESSION_RECOVERED
        | crate::catalog::SESSION_REATTACH_REJECTED
        | crate::catalog::SESSION_BOUND => ("session", event.session_id.as_deref()),
        crate::catalog::IDENTITY_MARKER_RESTORED => {
            if event.worktree_id.is_some() {
                ("worktree", event.worktree_id.as_deref())
            } else {
                ("repository", event.repository_id.as_deref())
            }
        }
        crate::catalog::PROJECT_CREATED | crate::catalog::PROJECT_UPDATED => {
            if event.repository_id.is_some()
                || event.worktree_id.is_some()
                || event.session_id.is_some()
            {
                return Err(MixedReplayError::InvalidAggregateScope);
            }
            ("project", Some(scope.1))
        }
        crate::catalog::PROJECT_REPOSITORY_ASSOCIATED => {
            if event.repository_id.is_some()
                || event.worktree_id.is_some()
                || event.session_id.is_some()
            {
                return Err(MixedReplayError::InvalidAggregateScope);
            }
            ("repository", Some(scope.1))
        }
        crate::catalog::TASK_CREATED | crate::catalog::TASK_REVISION_CREATED => {
            if event.repository_id.is_some()
                || event.worktree_id.is_some()
                || event.session_id.is_some()
            {
                return Err(MixedReplayError::InvalidAggregateScope);
            }
            ("task", Some(scope.1))
        }
        _ => return Err(MixedReplayError::UnsupportedEventType),
    };
    if scope.0 != expected.0 || Some(scope.1) != expected.1 {
        return Err(MixedReplayError::InvalidAggregateScope);
    }
    Ok(())
}
