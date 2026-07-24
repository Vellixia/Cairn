//! Row types shared by DAOs (mirror data-model.md).

use sqlx::FromRow;

#[derive(Debug, Clone, FromRow)]
pub struct RepositoryRow {
    pub id: String,
    pub repo_uuid: String,
    pub canonical_path: String,
    pub default_remote_name: Option<String>,
    pub default_remote_url: Option<String>,
    pub copied_from_repository_id: Option<String>,
    pub registered_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct WorktreeRow {
    pub id: String,
    pub repository_id: String,
    pub worktree_uuid: String,
    pub path: String,
    pub is_main: i64,
    pub registered_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct SnapshotRow {
    pub id: String,
    pub worktree_id: String,
    pub branch: Option<String>,
    pub head_commit: String,
    pub staged_fp: String,
    pub unstaged_fp: String,
    pub untracked_fp: String,
    pub snapshot_fp: String,
    pub fp_schema_version: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct SessionRow {
    pub id: String,
    pub repository_id: String,
    pub worktree_id: String,
    pub local_user: String,
    pub agent_type: String,
    pub agent_instance_id: String,
    pub agent_pid: Option<i64>,
    pub resume_token_hash: String,
    pub lease_expires_at: String,
    pub state: String,
    pub start_snapshot_id: String,
    pub current_snapshot_id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub last_heartbeat_at: String,
    pub recovering_since: Option<String>,
    pub binding_mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ProjectRepositoryAssociationRow {
    pub id: String,
    pub project_id: String,
    pub repository_id: String,
    pub associated_at: String,
    pub event_seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct TaskRow {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub latest_revision_number: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct TaskRevisionRow {
    pub id: String,
    pub task_id: String,
    pub revision_number: i64,
    pub parent_revision_id: Option<String>,
    pub goal_contract_json: String,
    pub goal_contract_schema_version: i64,
    pub goal_contract_fingerprint: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct SessionBindingRow {
    pub session_id: String,
    pub project_id: String,
    pub task_revision_id: String,
    pub bound_at: String,
    pub binding_event_seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AggregateHeadRow {
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub last_seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AggregateEventRow {
    pub seq: i64,
    pub id: String,
    pub idempotency_key: String,
    pub event_type: String,
    pub repository_id: Option<String>,
    pub worktree_id: Option<String>,
    pub session_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub payload: String,
    pub recorded_at: String,
    pub aggregate_type: Option<String>,
    pub aggregate_id: Option<String>,
    pub aggregate_seq: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct OperationIdempotencyRow {
    pub idempotency_key: String,
    pub method: String,
    pub request_fingerprint: String,
    pub result_kind: String,
    pub result_locator: String,
    pub created_at: String,
}
