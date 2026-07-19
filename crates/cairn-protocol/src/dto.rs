//! Request/response DTOs for every v1 method (contracts/ipc-contract.md).

use cairn_domain::{
    AgentInstanceId, LivenessReason, RepoUuid, SessionId, SessionState, SnapshotId, Timestamp,
    WorktreeUuid,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---------- shared objects ----------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RepositoryDto {
    pub repository_id: String,
    pub repo_uuid: RepoUuid,
    pub canonical_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_remote: Option<RemoteDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copied_from_repository_id: Option<String>,
    pub registered_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemoteDto {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorktreeDto {
    pub worktree_id: String,
    pub worktree_uuid: WorktreeUuid,
    pub path: String,
    pub is_main: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotDto {
    pub snapshot_id: SnapshotId,
    pub branch: Option<String>,
    pub detached: bool,
    pub head_commit: String,
    pub staged_fp: String,
    pub unstaged_fp: String,
    pub untracked_fp: String,
    pub snapshot_fp: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionDto {
    pub session_id: SessionId,
    pub repository_id: String,
    pub worktree_id: String,
    pub local_user: String,
    pub agent_type: String,
    pub agent_instance_id: AgentInstanceId,
    pub state: SessionState,
    pub start_snapshot: SnapshotDto,
    pub current_snapshot: SnapshotDto,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    pub last_heartbeat_at: Timestamp,
    pub lease_expires_at: Timestamp,
    pub recovering_since: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionSummaryDto {
    pub session_id: SessionId,
    pub agent_type: String,
    pub agent_instance_id: AgentInstanceId,
    pub state: SessionState,
    pub started_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IgnoredSummaryDto {
    pub total_count: u64,
    pub by_source: IgnoredBySourceDto,
    pub collapsed_roots: Vec<IgnoredRootDto>,
    pub samples: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IgnoredBySourceDto {
    pub gitignore: u64,
    pub cairnignore: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IgnoredRootDto {
    pub path: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FileChangeDto {
    pub path: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InspectionDto {
    pub root: String,
    pub branch: Option<String>,
    pub detached: bool,
    pub head_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_remote: Option<RemoteDto>,
    pub staged: Vec<FileChangeDto>,
    pub unstaged: Vec<FileChangeDto>,
    pub untracked: Vec<String>,
    pub ignored_summary: IgnoredSummaryDto,
    pub worktree: WorktreeInfoDto,
    pub in_progress: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorktreeInfoDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    pub path: String,
    pub is_main: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EventDto {
    pub seq: i64,
    pub id: String,
    pub event_type: String,
    pub repository_id: Option<String>,
    pub worktree_id: Option<String>,
    pub session_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub payload: serde_json::Value,
    pub recorded_at: Timestamp,
}

// ---------- daemon.status ----------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DaemonStatusResult {
    pub version: String,
    pub pid: u32,
    pub uptime_seconds: u64,
    pub db_path: String,
    pub db_healthy: bool,
    pub watched_repositories: u64,
    pub active_sessions: u64,
}

// ---------- repository.register ----------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegisterParams {
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IdentityOutcome {
    Created,
    Existing,
    Restored,
    NewAfterMarkerLoss,
    NewAfterCopy,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegisterResult {
    pub repository: RepositoryDto,
    pub worktree: WorktreeDto,
    pub created: bool,
    pub identity_outcome: IdentityOutcome,
}

// ---------- repository.inspect / ignored_files ----------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InspectParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IgnoredFilesParams {
    pub repository_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IgnoredFilesResult {
    pub paths: Vec<String>,
    pub next_cursor: Option<String>,
}

// ---------- snapshot.create ----------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotCreateParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotCreateResult {
    pub snapshot: SnapshotDto,
    pub created: bool,
}

// ---------- session lifecycle ----------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStartParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    pub agent_type: String,
    pub agent_instance_id: AgentInstanceId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_pid: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StartOutcome {
    Created,
    Existing,
    Takeover,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStartResult {
    pub session: SessionDto,
    /// Issued once on created/takeover; null on idempotent `existing`.
    pub resume_token: Option<String>,
    pub outcome: StartOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionGetParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_instance_id: Option<AgentInstanceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GetResolution {
    Single,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionGetResult {
    pub resolution: GetResolution,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<SessionSummaryDto>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Vec<SessionState>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionListResult {
    pub sessions: Vec<SessionSummaryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionHeartbeatParams {
    pub session_id: SessionId,
    pub agent_instance_id: AgentInstanceId,
    pub resume_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionHeartbeatResult {
    pub state: SessionState,
    pub last_heartbeat_at: Timestamp,
    pub lease_expires_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionReattachParams {
    pub session_id: SessionId,
    pub agent_instance_id: AgentInstanceId,
    pub resume_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionReattachResult {
    pub session: SessionDto,
    pub fresh_snapshot: SnapshotDto,
    /// A fresh resume token is issued on successful reattachment.
    pub resume_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStopParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_instance_id: Option<AgentInstanceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStopResult {
    pub session: SessionDto,
}

// ---------- events.list ----------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EventsListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_seq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EventsListResult {
    pub events: Vec<EventDto>,
    pub next_after_seq: Option<i64>,
}

/// Reason detail attached to liveness decisions in interruption events.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LivenessDetail {
    pub reason: LivenessReason,
}
