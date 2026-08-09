//! IPC and sync wire types (contracts/agent-integration.md, contracts/server-api.md).
//!
//! The CLI, the MCP server and the daemon all speak these types over a local
//! socket as newline-delimited JSON. The same envelope shape is what `--json`
//! prints.

use crate::domain::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable error codes (contracts/mcp-tools.md).
///
/// There is deliberately no `budget_exceeded`: a briefing is truncated to fit,
/// never rejected for size (FR-029).
pub mod codes {
    pub const NOT_A_REPOSITORY: &str = "not_a_repository";
    pub const NO_ACTIVE_SESSION: &str = "no_active_session";
    pub const AMBIGUOUS_SESSION: &str = "ambiguous_session";
    pub const NOT_FOUND: &str = "not_found";
    pub const INVALID_REQUEST: &str = "invalid_request";
    pub const STORAGE_UNAVAILABLE: &str = "storage_unavailable";
    pub const DAEMON_UNAVAILABLE: &str = "daemon_unavailable";
    pub const NOT_LINKED: &str = "not_linked";
    pub const SERVER_UNAVAILABLE: &str = "server_unavailable";
    pub const UNAUTHORIZED: &str = "unauthorized";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WireError {
    pub code: String,
    pub message: String,
}

impl WireError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
    pub fn not_found(what: impl Into<String>) -> Self {
        Self::new(codes::NOT_FOUND, what)
    }
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::new(codes::INVALID_REQUEST, msg)
    }
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for WireError {}

/// The stable `--json` envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

impl Envelope {
    pub fn ok(data: serde_json::Value) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }
    pub fn err(error: WireError) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error),
        }
    }
    pub fn into_result(self) -> Result<serde_json::Value, WireError> {
        if self.ok {
            Ok(self.data.unwrap_or(serde_json::Value::Null))
        } else {
            Err(self
                .error
                .unwrap_or_else(|| WireError::new("internal", "unspecified failure")))
        }
    }
}

/// Why a briefing is being assembled (contracts/mcp-tools.md).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextReason {
    SessionStart,
    Continuation,
    Refresh,
}

/// What a capture hook observed, before the daemon filters and stores it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationInput {
    pub kind: ObservationType,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub outcome: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
}

/// Which entity a delete targets (FR-052).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeleteTarget {
    Observation,
    Memory,
    Session,
    Handoff,
}

/// Search parameters for memory recall (FR-022, FR-023).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryQuery {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub scope: Option<MemoryScope>,
    #[serde(default)]
    pub scope_key: Option<String>,
    #[serde(default)]
    pub kind: Option<MemoryType>,
    #[serde(default)]
    pub state: Option<MemoryState>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Everything the daemon can be asked to do.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    DaemonStatus,
    DaemonShutdown,

    Init {
        cwd: String,
    },
    Status {
        cwd: String,
    },

    SessionStart {
        cwd: String,
        agent: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        task_id: Option<Uuid>,
    },
    SessionList {
        cwd: String,
    },
    SessionShow {
        cwd: String,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        agent_session_key: Option<String>,
    },
    SessionBindTask {
        cwd: String,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        agent_session_key: Option<String>,
        task_id: Uuid,
    },
    SessionEnd {
        cwd: String,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        agent_session_key: Option<String>,
        status: SessionStatus,
        #[serde(default)]
        reason: Option<String>,
    },
    /// `Stop`: a turn boundary. Never ends the session (FR-032, D16).
    TurnCheckpoint {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
    },

    Observe {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        observation: ObservationInput,
    },

    Context {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        reason: Option<ContextReason>,
        #[serde(default)]
        token_budget: Option<usize>,
    },

    HandoffGenerate {
        cwd: String,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        agent_session_key: Option<String>,
        trigger: HandoffTrigger,
    },
    HandoffLatest {
        cwd: String,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        agent_session_key: Option<String>,
    },
    HandoffAnnotate {
        cwd: String,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        agent_session_key: Option<String>,
        note: String,
    },

    TaskList {
        cwd: String,
        #[serde(default)]
        status: Option<TaskStatus>,
    },
    TaskGet {
        cwd: String,
        task_id: Uuid,
    },
    TaskCreate {
        cwd: String,
        title: String,
        goal: String,
        #[serde(default)]
        acceptance_criteria: Vec<String>,
    },
    TaskUpdate {
        cwd: String,
        task_id: Uuid,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        goal: Option<String>,
        #[serde(default)]
        acceptance_criteria: Option<Vec<String>>,
        #[serde(default)]
        status: Option<TaskStatus>,
    },

    MemoryCreate {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        /// The session to attribute this to, when the key is not to hand.
        #[serde(default)]
        session_id: Option<Uuid>,
        kind: MemoryType,
        #[serde(default)]
        scope: Option<MemoryScope>,
        #[serde(default)]
        scope_key: Option<String>,
        content: String,
        /// Zero or more. Never fabricated (FR-019).
        #[serde(default)]
        evidence_observation_ids: Vec<Uuid>,
        #[serde(default)]
        local_only: bool,
    },
    MemorySupersede {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        memory_id: Uuid,
        kind: MemoryType,
        #[serde(default)]
        scope: Option<MemoryScope>,
        #[serde(default)]
        scope_key: Option<String>,
        content: String,
        #[serde(default)]
        evidence_observation_ids: Vec<Uuid>,
        #[serde(default)]
        local_only: bool,
    },
    MemoryForget {
        cwd: String,
        memory_id: Uuid,
    },
    MemoryGet {
        cwd: String,
        memory_id: Uuid,
    },
    MemorySearch {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(flatten)]
        query: MemoryQuery,
    },

    PrivacyExclude {
        cwd: String,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        command: Option<String>,
    },
    PrivacyUnexclude {
        cwd: String,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        command: Option<String>,
    },
    PrivacyList {
        cwd: String,
    },

    Delete {
        cwd: String,
        target: DeleteTarget,
        id: Uuid,
        #[serde(default)]
        with_memories: bool,
    },

    Link {
        cwd: String,
        #[serde(default)]
        server_project_id: Option<Uuid>,
        #[serde(default)]
        create: bool,
    },
    Unlink {
        cwd: String,
    },
    AuthTokenSet {
        token: String,
        #[serde(default)]
        server_url: Option<String>,
    },
    AuthLogout,
    AuthStatus,
    SyncStatus {
        cwd: String,
    },
    SyncNow {
        cwd: String,
    },
}

/// Repository and project state for `cairn status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusPayload {
    pub project: ProjectSummary,
    pub repository: RepositoryState,
    pub worktree_path: String,
    pub sessions: Vec<SessionSummary>,
    pub integration_mode: String,
    pub daemon: String,
    pub observation_count: i64,
    pub memory_count: i64,
    /// The server a linked project syncs to, when one is configured.
    #[serde(default)]
    pub server_url: Option<String>,
    /// Whether a credential is stored for it.
    #[serde(default)]
    pub authenticated: bool,
    /// The build answering this, so a bug report can name it.
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: Uuid,
    pub name: String,
    pub linked: bool,
    pub server_project_id: Option<Uuid>,
}

impl From<&Project> for ProjectSummary {
    fn from(p: &Project) -> Self {
        Self {
            id: p.id,
            name: p.name.clone(),
            linked: p.linked,
            server_project_id: p.server_project_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: Uuid,
    pub status: SessionStatus,
    pub agent: String,
    pub branch: String,
    pub commit: Option<String>,
    pub task_id: Option<Uuid>,
    pub previous_session_id: Option<Uuid>,
    pub worktree_path: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub last_turn_ended_at: Option<DateTime<Utc>>,
    /// Reported only. Idleness never reclassifies a session (FR-009).
    pub idle_seconds: i64,
}

impl SessionSummary {
    pub fn from_session(s: &Session, now: DateTime<Utc>) -> Self {
        Self {
            id: s.id,
            status: s.status,
            agent: s.agent.clone(),
            branch: s.branch.clone(),
            commit: s.commit_sha.clone(),
            task_id: s.task_id,
            previous_session_id: s.previous_session_id,
            worktree_path: s.worktree_path.clone(),
            started_at: s.started_at,
            ended_at: s.ended_at,
            last_turn_ended_at: s.last_turn_ended_at,
            idle_seconds: (now - s.last_event_at).num_seconds().max(0),
        }
    }
}

/// Provenance carried on every search result (FR-026).
///
/// A mandatory session, zero or more observation ids, and a count that may be
/// zero. Observation *content* is resolved locally and never travels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub session_id: Uuid,
    pub observation_ids: Vec<Uuid>,
    pub evidence_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub deleted_observation_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankInfo {
    pub scope_bucket: i64,
    pub relevance: f64,
    pub age_days: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryResult {
    pub id: Uuid,
    #[serde(rename = "type")]
    pub kind: MemoryType,
    pub scope: MemoryScope,
    pub scope_key: String,
    pub content: String,
    pub state: MemoryState,
    pub local_only: bool,
    pub superseded_by_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub provenance: Provenance,
    pub rank: RankInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPayload {
    pub results: Vec<MemoryResult>,
    pub total: usize,
}

/// The bounded briefing (FR-028).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Briefing {
    pub project: ProjectSummary,
    pub repository: RepositoryState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<BriefingTask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_handoff: Option<BriefingHandoff>,
    pub decisions: Vec<String>,
    pub known_failures: Vec<String>,
    pub memory: BriefingMemory,
    pub no_prior_history: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingTask {
    pub id: Uuid,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingHandoff {
    pub session_id: Uuid,
    pub next_step: String,
    pub remaining_work: Vec<String>,
    pub changed_files: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BriefingMemory {
    pub task: Vec<String>,
    pub branch: Vec<String>,
    pub project: Vec<String>,
}

/// A briefing plus its budget accounting.
///
/// `estimated_tokens` and `budget` are both denominated in Cairn-estimated
/// tokens, not any model's tokenizer (FR-029, D8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPayload {
    pub briefing: Briefing,
    pub estimated_tokens: usize,
    pub budget: usize,
    pub truncated: bool,
    pub omitted_sections: Vec<String>,
    /// True when the briefing could not be fully assembled in time (FR-046).
    pub degraded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusPayload {
    pub linked: bool,
    pub server_project_id: Option<Uuid>,
    pub server_url: Option<String>,
    pub pending: i64,
    pub failed: i64,
    pub last_success_at: Option<DateTime<Utc>>,
    pub failures: Vec<SyncFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncFailure {
    pub entity_type: OutboxEntityType,
    pub entity_id: Uuid,
    pub error: String,
}

// ---------------------------------------------------------------------------
// Server sync wire types (contracts/server-api.md)
// ---------------------------------------------------------------------------

/// One item in a sync batch. There is no observation variant, so a payload
/// carrying observation content cannot be constructed (FR-055, D9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncItem {
    pub idempotency_key: String,
    pub entity_type: OutboxEntityType,
    pub entity_id: Uuid,
    pub operation: OutboxOperation,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncBatch {
    pub project_id: Uuid,
    pub items: Vec<SyncItem>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncItemStatus {
    Applied,
    Duplicate,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncItemResult {
    pub idempotency_key: String,
    pub status: SyncItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncBatchResponse {
    pub results: Vec<SyncItemResult>,
}

/// Fields the server rejects on a session payload: local-only, always
/// (contracts/server-api.md).
pub const REJECTED_SESSION_FIELDS: &[&str] = &[
    "worktree_path",
    "agent_session_key",
    "daemon_run_id",
    "last_event_at",
    "last_turn_ended_at",
];

/// Fields that would carry observation content. Rejected everywhere (FR-055).
pub const REJECTED_OBSERVATION_FIELDS: &[&str] = &[
    "summary",
    "path",
    "command",
    "details",
    "observations",
    "outcome",
    "exit_code",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let e = Envelope::ok(serde_json::json!({"a": 1}));
        let s = serde_json::to_string(&e).unwrap();
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert!(back.ok);
        assert!(back.into_result().is_ok());
    }

    #[test]
    fn error_envelope_has_no_data_key() {
        let s = serde_json::to_string(&Envelope::err(WireError::not_found("task"))).unwrap();
        assert!(!s.contains("\"data\""));
        assert!(s.contains("not_found"));
    }

    #[test]
    fn request_is_tagged_by_op() {
        let r = Request::Status { cwd: "/tmp".into() };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"op\":\"status\""));
        let back: Request = serde_json::from_str(&s).unwrap();
        matches!(back, Request::Status { .. });
    }

    #[test]
    fn provenance_allows_zero_evidence() {
        // Manual MCP mode records memory with no observations (FR-019).
        let p = Provenance {
            session_id: crate::domain::new_id(),
            observation_ids: vec![],
            evidence_count: 0,
            deleted_observation_ids: vec![],
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"evidence_count\":0"));
    }
}
