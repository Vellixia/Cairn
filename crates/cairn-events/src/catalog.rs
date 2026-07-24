//! T016: the 11-type event catalog (data-model.md) with typed payloads and
//! deterministic idempotency-key derivation (research R7).
//!
//! `session.reattach_rejected` payloads MUST never contain token values.

use cairn_domain::{
    EventId, GoalContractV1, Project, ProjectId, ProjectRepositoryAssociationId, SessionId, Task,
    TaskId, TaskRevision, TaskRevisionId, Timestamp, WatcherStartStage,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use cairn_storage_local::NewEvent;

pub const REPOSITORY_REGISTERED: &str = "repository.registered";
pub const WORKTREE_REGISTERED: &str = "worktree.registered";
pub const SNAPSHOT_CREATED: &str = "snapshot.created";
pub const SESSION_STARTED: &str = "session.started";
pub const REPOSITORY_STATE_CHANGED: &str = "repository.state_changed";
pub const BRANCH_CHANGED: &str = "branch.changed";
pub const SESSION_STOPPED: &str = "session.stopped";
pub const SESSION_INTERRUPTED: &str = "session.interrupted";
pub const SESSION_RECOVERED: &str = "session.recovered";
pub const SESSION_REATTACH_REJECTED: &str = "session.reattach_rejected";
pub const IDENTITY_MARKER_RESTORED: &str = "identity.marker_restored";

// Feature 002 type declarations only. Story-owned payload builders and replay
// handlers are intentionally added with US1–US4.
pub const PROJECT_CREATED: &str = "project.created";
pub const PROJECT_UPDATED: &str = "project.updated";
pub const PROJECT_REPOSITORY_ASSOCIATED: &str = "project.repository_associated";
pub const TASK_CREATED: &str = "task.created";
pub const TASK_REVISION_CREATED: &str = "task.revision_created";
pub const SESSION_BOUND: &str = "session.bound";

pub const FEATURE002_EVENT_TYPES: &[&str] = &[
    PROJECT_CREATED,
    PROJECT_UPDATED,
    PROJECT_REPOSITORY_ASSOCIATED,
    TASK_CREATED,
    TASK_REVISION_CREATED,
    SESSION_BOUND,
];

pub const ALL_EVENT_TYPES: &[&str] = &[
    REPOSITORY_REGISTERED,
    WORKTREE_REGISTERED,
    SNAPSHOT_CREATED,
    SESSION_STARTED,
    REPOSITORY_STATE_CHANGED,
    BRANCH_CHANGED,
    SESSION_STOPPED,
    SESSION_INTERRUPTED,
    SESSION_RECOVERED,
    SESSION_REATTACH_REJECTED,
    IDENTITY_MARKER_RESTORED,
    PROJECT_CREATED,
    PROJECT_UPDATED,
    PROJECT_REPOSITORY_ASSOCIATED,
    TASK_CREATED,
    TASK_REVISION_CREATED,
    SESSION_BOUND,
];

/// Typed builder ensuring every event carries a deterministic idempotency key
/// derived from event type + entity id + causal input (arch rule 6).
pub struct EventBuilder;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartedPayload {
    pub agent_type: String,
    pub agent_instance_id: String,
    pub start_snapshot_id: String,
    pub local_user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateChangedPayload {
    pub worktree_id: String,
    pub from_snapshot_id: Option<String>,
    pub to_snapshot_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchChangedPayload {
    pub from_branch: Option<String>,
    pub to_branch: Option<String>,
    pub from_head: Option<String>,
    pub to_head: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectCreatedPayload {
    pub schema_version: u16,
    pub project: ProjectEventState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectEventState {
    pub project_id: ProjectId,
    pub name: String,
    pub description: Option<String>,
    pub status: cairn_domain::ProjectStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl From<Project> for ProjectEventState {
    fn from(project: Project) -> Self {
        Self {
            project_id: project.id,
            name: project.name,
            description: project.description,
            status: project.status,
            created_at: project.created_at,
            updated_at: project.updated_at,
        }
    }
}

impl From<ProjectEventState> for Project {
    fn from(project: ProjectEventState) -> Self {
        Self {
            id: project.project_id,
            name: project.name,
            description: project.description,
            status: project.status,
            created_at: project.created_at,
            updated_at: project.updated_at,
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum ProjectChangedField {
    Description,
    Name,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectUpdatedPayload {
    pub schema_version: u16,
    pub project: ProjectEventState,
    pub changed_fields: Vec<ProjectChangedField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectAssociationEvent {
    pub association_id: ProjectRepositoryAssociationId,
    pub project_id: ProjectId,
    pub repository_id: String,
    pub associated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectRepositoryAssociatedPayload {
    pub schema_version: u16,
    pub association: ProjectAssociationEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskCreatedEvent {
    pub task_id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub created_at: Timestamp,
}

impl From<Task> for TaskCreatedEvent {
    fn from(task: Task) -> Self {
        Self {
            task_id: task.id,
            project_id: task.project_id,
            title: task.title,
            created_at: task.created_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskEventState {
    pub task_id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub latest_revision_number: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl From<Task> for TaskEventState {
    fn from(task: Task) -> Self {
        Self {
            task_id: task.id,
            project_id: task.project_id,
            title: task.title,
            latest_revision_number: task.latest_revision_number,
            created_at: task.created_at,
            updated_at: task.updated_at,
        }
    }
}

impl From<TaskEventState> for Task {
    fn from(task: TaskEventState) -> Self {
        Self {
            id: task.task_id,
            project_id: task.project_id,
            title: task.title,
            latest_revision_number: task.latest_revision_number,
            created_at: task.created_at,
            updated_at: task.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskRevisionEvent {
    pub revision_id: TaskRevisionId,
    pub task_id: TaskId,
    pub revision_number: u64,
    pub parent_revision_id: Option<TaskRevisionId>,
    pub goal_contract_schema_version: u16,
    pub goal_contract: GoalContractV1,
    pub goal_contract_fingerprint: String,
    pub created_at: Timestamp,
}

impl From<TaskRevision> for TaskRevisionEvent {
    fn from(revision: TaskRevision) -> Self {
        Self {
            revision_id: revision.id,
            task_id: revision.task_id,
            revision_number: revision.revision_number,
            parent_revision_id: revision.parent_revision_id,
            goal_contract_schema_version: revision.goal_contract.schema_version(),
            goal_contract: revision.goal_contract,
            goal_contract_fingerprint: revision.goal_contract_fingerprint,
            created_at: revision.created_at,
        }
    }
}

impl TryFrom<TaskRevisionEvent> for TaskRevision {
    type Error = &'static str;

    fn try_from(revision: TaskRevisionEvent) -> Result<Self, Self::Error> {
        if revision.goal_contract_schema_version != revision.goal_contract.schema_version()
            || revision.goal_contract_fingerprint != revision.goal_contract.fingerprint()
        {
            return Err("goal contract metadata mismatch");
        }
        TaskRevision::new(
            revision.revision_id,
            revision.task_id,
            revision.revision_number,
            revision.parent_revision_id,
            revision.goal_contract,
            revision.created_at,
        )
        .map_err(|_| "invalid task revision")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskCreatedPayload {
    pub schema_version: u16,
    pub task: TaskCreatedEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskRevisionCreatedPayload {
    pub schema_version: u16,
    pub revision: TaskRevisionEvent,
    pub task: TaskEventState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionBindingEvent {
    pub session_id: SessionId,
    pub repository_id: String,
    pub worktree_id: String,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub task_revision_id: TaskRevisionId,
    pub bound_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionBoundPayload {
    pub schema_version: u16,
    pub binding: SessionBindingEvent,
}

fn base(
    event_type: &str,
    idempotency_key: String,
    aggregate_type: &str,
    aggregate_id: &str,
    payload: serde_json::Value,
) -> NewEvent {
    NewEvent {
        id: EventId::new_v7().to_string(),
        idempotency_key,
        event_type: event_type.to_string(),
        repository_id: None,
        worktree_id: None,
        session_id: None,
        snapshot_id: None,
        aggregate_type: aggregate_type.to_string(),
        aggregate_id: aggregate_id.to_string(),
        payload,
        recorded_at: Timestamp::now().to_rfc3339(),
    }
}

impl EventBuilder {
    pub fn project_created(
        event_id: EventId,
        operation_identity: &str,
        payload: &ProjectCreatedPayload,
    ) -> NewEvent {
        feature002_event(
            event_id,
            operation_identity,
            crate::aggregate::EventOperationMethod::ProjectCreate,
            0,
            PROJECT_CREATED,
            "project",
            &payload.project.project_id.to_string(),
            serde_json::to_value(payload).expect("serializable project-created payload"),
            payload.project.created_at,
        )
    }

    pub fn project_updated(
        event_id: EventId,
        operation_identity: &str,
        payload: &ProjectUpdatedPayload,
    ) -> NewEvent {
        feature002_event(
            event_id,
            operation_identity,
            crate::aggregate::EventOperationMethod::ProjectUpdate,
            0,
            PROJECT_UPDATED,
            "project",
            &payload.project.project_id.to_string(),
            serde_json::to_value(payload).expect("serializable project-updated payload"),
            payload.project.updated_at,
        )
    }

    pub fn project_repository_associated(
        event_id: EventId,
        operation_identity: &str,
        payload: &ProjectRepositoryAssociatedPayload,
    ) -> NewEvent {
        feature002_event(
            event_id,
            operation_identity,
            crate::aggregate::EventOperationMethod::ProjectRepositoryAssociate,
            0,
            PROJECT_REPOSITORY_ASSOCIATED,
            "repository",
            &payload.association.repository_id,
            serde_json::to_value(payload)
                .expect("serializable project-repository-associated payload"),
            payload.association.associated_at,
        )
    }

    pub fn task_created(
        event_id: EventId,
        operation_identity: &str,
        payload: &TaskCreatedPayload,
    ) -> NewEvent {
        feature002_event(
            event_id,
            operation_identity,
            crate::aggregate::EventOperationMethod::TaskCreate,
            0,
            TASK_CREATED,
            "task",
            &payload.task.task_id.to_string(),
            serde_json::to_value(payload).expect("serializable task-created payload"),
            payload.task.created_at,
        )
    }

    pub fn task_revision_created(
        event_id: EventId,
        operation_identity: &str,
        method: crate::aggregate::EventOperationMethod,
        event_position: u16,
        payload: &TaskRevisionCreatedPayload,
    ) -> NewEvent {
        feature002_event(
            event_id,
            operation_identity,
            method,
            event_position,
            TASK_REVISION_CREATED,
            "task",
            &payload.task.task_id.to_string(),
            serde_json::to_value(payload).expect("serializable task-revision-created payload"),
            payload.revision.created_at,
        )
    }

    pub fn session_bound(
        event_id: EventId,
        operation_identity: &str,
        payload: &SessionBoundPayload,
    ) -> NewEvent {
        let mut event = feature002_event(
            event_id,
            operation_identity,
            crate::aggregate::EventOperationMethod::SessionBind,
            0,
            SESSION_BOUND,
            "session",
            &payload.binding.session_id.to_string(),
            serde_json::to_value(payload).expect("serializable session-bound payload"),
            payload.binding.bound_at,
        );
        event.repository_id = Some(payload.binding.repository_id.clone());
        event.worktree_id = Some(payload.binding.worktree_id.clone());
        event.session_id = Some(payload.binding.session_id.to_string());
        event
    }

    /// Bound-start event derived from the stable Feature 001 start identity.
    /// Position 1 follows `session.started` and keeps legacy clients free of a
    /// new caller-supplied idempotency field (event-catalog R11/R14).
    pub fn session_bound_for_start(
        event_id: EventId,
        session_start_identity: &str,
        payload: &SessionBoundPayload,
    ) -> NewEvent {
        let mut event = feature002_event(
            event_id,
            session_start_identity,
            crate::aggregate::EventOperationMethod::SessionBind,
            1,
            SESSION_BOUND,
            "session",
            &payload.binding.session_id.to_string(),
            serde_json::to_value(payload).expect("serializable session-bound payload"),
            payload.binding.bound_at,
        );
        event.repository_id = Some(payload.binding.repository_id.clone());
        event.worktree_id = Some(payload.binding.worktree_id.clone());
        event.session_id = Some(payload.binding.session_id.to_string());
        event
    }

    pub fn repository_registered(
        repository_id: &str,
        repo_uuid: &str,
        canonical_path: &str,
        remote: Option<(&str, &str)>,
    ) -> NewEvent {
        let mut e = base(
            REPOSITORY_REGISTERED,
            format!("{REPOSITORY_REGISTERED}:{repository_id}"),
            "repository",
            repository_id,
            json!({
                "repo_uuid": repo_uuid,
                "canonical_path": canonical_path,
                "remote": remote.map(|(n, u)| json!({"name": n, "url": u})),
            }),
        );
        e.repository_id = Some(repository_id.to_string());
        e
    }

    pub fn worktree_registered(
        repository_id: &str,
        worktree_id: &str,
        worktree_uuid: &str,
        path: &str,
        is_main: bool,
    ) -> NewEvent {
        let mut e = base(
            WORKTREE_REGISTERED,
            format!("{WORKTREE_REGISTERED}:{worktree_id}"),
            "worktree",
            worktree_id,
            json!({"worktree_uuid": worktree_uuid, "path": path, "is_main": is_main}),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e
    }

    pub fn snapshot_created(
        repository_id: &str,
        worktree_id: &str,
        snapshot_id: &str,
        snapshot_fp: &str,
        branch: Option<&str>,
        head_commit: &str,
    ) -> NewEvent {
        let mut e = base(
            SNAPSHOT_CREATED,
            format!("{SNAPSHOT_CREATED}:{worktree_id}:{snapshot_fp}"),
            "worktree",
            worktree_id,
            json!({
                "snapshot_fp": snapshot_fp,
                "branch": branch,
                "head_commit": head_commit,
            }),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.snapshot_id = Some(snapshot_id.to_string());
        e
    }

    pub fn session_started(
        repository_id: &str,
        worktree_id: &str,
        session_id: &str,
        payload: &SessionStartedPayload,
    ) -> NewEvent {
        let mut e = base(
            SESSION_STARTED,
            format!("{SESSION_STARTED}:{session_id}"),
            "session",
            session_id,
            serde_json::to_value(payload).expect("serializable payload"),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.session_id = Some(session_id.to_string());
        e.snapshot_id = Some(payload.start_snapshot_id.clone());
        e
    }

    pub fn repository_state_changed(
        repository_id: &str,
        worktree_id: &str,
        payload: &StateChangedPayload,
    ) -> NewEvent {
        let mut e = base(
            REPOSITORY_STATE_CHANGED,
            format!(
                "{REPOSITORY_STATE_CHANGED}:{worktree_id}:{}",
                payload.to_snapshot_id
            ),
            "worktree",
            worktree_id,
            serde_json::to_value(payload).expect("serializable payload"),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.snapshot_id = Some(payload.to_snapshot_id.clone());
        e
    }

    pub fn branch_changed(
        repository_id: &str,
        worktree_id: &str,
        payload: &BranchChangedPayload,
    ) -> NewEvent {
        let mut e = base(
            BRANCH_CHANGED,
            format!(
                "{BRANCH_CHANGED}:{worktree_id}:{}:{}",
                payload.to_branch.as_deref().unwrap_or("DETACHED"),
                payload.to_head
            ),
            "worktree",
            worktree_id,
            serde_json::to_value(payload).expect("serializable payload"),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e
    }

    pub fn session_stopped(
        repository_id: &str,
        worktree_id: &str,
        session_id: &str,
        final_snapshot_id: &str,
    ) -> NewEvent {
        let mut e = base(
            SESSION_STOPPED,
            format!("{SESSION_STOPPED}:{session_id}"),
            "session",
            session_id,
            json!({"final_snapshot_id": final_snapshot_id}),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.session_id = Some(session_id.to_string());
        e.snapshot_id = Some(final_snapshot_id.to_string());
        e
    }

    pub fn session_interrupted(
        repository_id: &str,
        worktree_id: &str,
        session_id: &str,
        reason: &str,
        liveness_detail: &str,
    ) -> NewEvent {
        let mut e = base(
            SESSION_INTERRUPTED,
            format!("{SESSION_INTERRUPTED}:{session_id}"),
            "session",
            session_id,
            json!({"reason": reason, "liveness_detail": liveness_detail}),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.session_id = Some(session_id.to_string());
        e
    }

    /// Watcher-readiness failure. The payload is deliberately bounded to the
    /// stable stage code and never includes paths, contents, environment
    /// values, or token material (FR-038).
    pub fn session_watcher_start_failed(
        repository_id: &str,
        worktree_id: &str,
        session_id: &str,
        watcher_stage: WatcherStartStage,
    ) -> NewEvent {
        let mut e = base(
            SESSION_INTERRUPTED,
            format!("{SESSION_INTERRUPTED}:{session_id}"),
            "session",
            session_id,
            json!({
                "reason": "watcher_start_failed",
                "watcher_stage": watcher_stage.as_str(),
            }),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.session_id = Some(session_id.to_string());
        e
    }

    pub fn session_recovered(
        repository_id: &str,
        worktree_id: &str,
        session_id: &str,
        fresh_snapshot_id: &str,
    ) -> NewEvent {
        let mut e = base(
            SESSION_RECOVERED,
            // A session may recover multiple times across restarts: key on
            // the fresh snapshot to keep each recovery distinct.
            format!("{SESSION_RECOVERED}:{session_id}:{fresh_snapshot_id}"),
            "session",
            session_id,
            json!({"fresh_snapshot_id": fresh_snapshot_id}),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.session_id = Some(session_id.to_string());
        e.snapshot_id = Some(fresh_snapshot_id.to_string());
        e
    }

    /// Audit event for rejected reattachment. NEVER include token material.
    pub fn session_reattach_rejected(
        repository_id: &str,
        worktree_id: &str,
        session_id: &str,
        presented_instance_id: &str,
        reason: &str,
        attempt_id: &str,
    ) -> NewEvent {
        let mut e = base(
            SESSION_REATTACH_REJECTED,
            // Every rejected attempt is a distinct audit record.
            format!("{SESSION_REATTACH_REJECTED}:{session_id}:{attempt_id}"),
            "session",
            session_id,
            json!({
                "agent_instance_id_presented": presented_instance_id,
                "reason": reason,
            }),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.session_id = Some(session_id.to_string());
        e
    }

    pub fn identity_marker_restored(
        repository_id: &str,
        worktree_id: Option<&str>,
        restored_from: &str,
    ) -> NewEvent {
        let mut e = base(
            IDENTITY_MARKER_RESTORED,
            format!(
                "{IDENTITY_MARKER_RESTORED}:{repository_id}:{}",
                worktree_id.unwrap_or("-")
            ),
            "repository",
            repository_id,
            json!({"restored_from": restored_from}),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = worktree_id.map(str::to_string);
        e
    }
}

#[allow(clippy::too_many_arguments)]
fn feature002_event(
    event_id: EventId,
    operation_identity: &str,
    method: crate::aggregate::EventOperationMethod,
    event_position: u16,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    payload: serde_json::Value,
    recorded_at: Timestamp,
) -> NewEvent {
    let idempotency_key =
        crate::aggregate::derive_event_key(&crate::aggregate::DerivedEventKeyInput {
            operation_identity,
            method,
            event_position,
            event_type,
        });
    NewEvent {
        id: event_id.to_string(),
        idempotency_key,
        event_type: event_type.to_string(),
        repository_id: None,
        worktree_id: None,
        session_id: None,
        snapshot_id: None,
        aggregate_type: aggregate_type.to_string(),
        aggregate_id: aggregate_id.to_string(),
        payload,
        recorded_at: recorded_at.to_rfc3339(),
    }
}
