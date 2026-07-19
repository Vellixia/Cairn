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
}
