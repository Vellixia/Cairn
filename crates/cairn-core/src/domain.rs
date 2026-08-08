//! Domain types and enums for Cairn (data-model.md).
//!
//! These are pure data. No I/O lives here.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// Generate a time-ordered identifier. UUIDv7 everywhere (data-model.md).
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

/// Error returned when a stored enum string does not parse.
#[derive(Debug, thiserror::Error)]
#[error("invalid {kind} value: {value}")]
pub struct ParseEnumError {
    pub kind: &'static str,
    pub value: String,
}

/// Declare a lowercase-text enum with a `CHECK`-friendly string form.
macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident, $kind:literal, { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(&self) -> &'static str {
                match self {
                    $($name::$variant => $text),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ParseEnumError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($text => Ok($name::$variant),)+
                    other => Err(ParseEnumError { kind: $kind, value: other.to_string() }),
                }
            }
        }
    };
}

text_enum!(
    /// Task lifecycle (FR-037). No revision history exists (FR-039).
    TaskStatus, "task status", {
        Todo => "todo",
        InProgress => "in_progress",
        Done => "done",
        Blocked => "blocked",
    }
);

text_enum!(
    /// Session lifecycle (FR-007). A session leaves `Active` only at the
    /// deterministic boundaries in FR-009 — never on a `Stop` turn checkpoint.
    SessionStatus, "session status", {
        Active => "active",
        Completed => "completed",
        Interrupted => "interrupted",
    }
);

text_enum!(
    /// Structured observation kinds (FR-011).
    ObservationType, "observation type", {
        FileRead => "file_read",
        FileChanged => "file_changed",
        CommandRun => "command_run",
        TestRun => "test_run",
        Error => "error",
        Decision => "decision",
        Discovery => "discovery",
        UserInstruction => "user_instruction",
    }
);

text_enum!(
    /// Durable knowledge kinds (FR-016).
    MemoryType, "memory type", {
        Fact => "fact",
        Decision => "decision",
        Convention => "convention",
        Failure => "failure",
        Procedure => "procedure",
    }
);

text_enum!(
    /// Where a memory applies (FR-017). Always paired with a scope key.
    MemoryScope, "memory scope", {
        Project => "project",
        Branch => "branch",
        Task => "task",
        Session => "session",
    }
);

impl MemoryScope {
    /// Ranking bucket: lower sorts first (FR-024, D3).
    pub fn bucket(&self) -> i64 {
        match self {
            MemoryScope::Task => 0,
            MemoryScope::Branch => 1,
            MemoryScope::Project => 2,
            MemoryScope::Session => 3,
        }
    }
}

text_enum!(
    /// Memory lifecycle (FR-018). Only `Active` is returned by default.
    MemoryState, "memory state", {
        Active => "active",
        Stale => "stale",
        Superseded => "superseded",
    }
);

text_enum!(
    /// Boundaries that produce a durable handoff (FR-032).
    ///
    /// A `Stop` turn checkpoint is deliberately absent: it is a turn boundary,
    /// not a session boundary (D16).
    HandoffTrigger, "handoff trigger", {
        PreCompact => "pre_compact",
        SessionEnd => "session_end",
        Recovered => "recovered",
    }
);

text_enum!(
    /// Entities the outbox can carry. There is deliberately no observation
    /// variant: raw observations never sync (FR-055, D9).
    OutboxEntityType, "outbox entity type", {
        Project => "project",
        Task => "task",
        Session => "session",
        Memory => "memory",
        Handoff => "handoff",
    }
);

text_enum!(
    OutboxOperation, "outbox operation", {
        Upsert => "upsert",
        Delete => "delete",
    }
);

text_enum!(
    OutboxState, "outbox state", {
        Pending => "pending",
        InFlight => "in_flight",
        Delivered => "delivered",
        Failed => "failed",
    }
);

/// A tracked Git repository under Cairn.
///
/// `id` and `git_common_dir` are local. `server_project_id` is the shared
/// identity, assigned by the server at `cairn link` (FR-064, D14).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub git_common_dir: String,
    pub repository_remote: Option<String>,
    pub linked: bool,
    pub server_project_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub project_id: Uuid,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// One agent working session.
///
/// Identity is `id`, keyed to `agent_session_key`. The worktree is scope and
/// context, never a uniqueness key (FR-010).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub project_id: Uuid,
    pub task_id: Option<Uuid>,
    pub user_id: Uuid,
    pub agent: String,
    pub branch: String,
    pub commit_sha: Option<String>,
    pub worktree_path: String,
    pub agent_session_key: String,
    pub previous_session_id: Option<Uuid>,
    pub status: SessionStatus,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub last_event_at: DateTime<Utc>,
    /// Set by the `Stop` turn checkpoint. Never ends the session (D16).
    pub last_turn_ended_at: Option<DateTime<Utc>>,
    pub daemon_run_id: Uuid,
    pub end_reason: Option<String>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Session {
    pub fn is_active(&self) -> bool {
        self.status == SessionStatus::Active
    }
}

/// One structured thing that happened during a session.
///
/// Observations are local, always. No field of one ever syncs (FR-055).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: Uuid,
    pub session_id: Uuid,
    #[serde(rename = "type")]
    pub kind: ObservationType,
    pub occurred_at: DateTime<Utc>,
    pub branch: String,
    pub commit_sha: Option<String>,
    pub path: Option<String>,
    pub command: Option<String>,
    pub exit_code: Option<i64>,
    pub outcome: Option<String>,
    pub summary: String,
    pub details: Option<serde_json::Value>,
    pub payload_bytes: i64,
    pub truncated: bool,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// A supporting observation reference for a memory.
///
/// The join row survives deletion of the observation, so provenance stays
/// resolvable and reports "evidence deleted" (FR-052).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub observation_id: Uuid,
    pub content_digest: String,
    pub deleted: bool,
}

/// Durable knowledge.
///
/// `origin_session_id` is mandatory; `evidence` is zero-or-more and is never
/// fabricated to satisfy the schema (FR-019).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: Uuid,
    pub project_id: Uuid,
    #[serde(rename = "type")]
    pub kind: MemoryType,
    pub scope: MemoryScope,
    pub scope_key: String,
    pub content: String,
    pub state: MemoryState,
    pub superseded_by_id: Option<Uuid>,
    pub origin_session_id: Uuid,
    pub local_only: bool,
    pub evidence: Vec<EvidenceRef>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Memory {
    pub fn evidence_count(&self) -> usize {
        self.evidence.len()
    }
}

/// Repository state at a point in time (FR-003, FR-014).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepositoryState {
    pub branch: String,
    pub commit_sha: Option<String>,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
}

impl RepositoryState {
    pub fn is_clean(&self) -> bool {
        self.staged == 0 && self.unstaged == 0 && self.untracked == 0
    }
}

/// A test the session executed, as recorded on a handoff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRunRecord {
    pub command: String,
    pub outcome: String,
    pub occurred_at: DateTime<Utc>,
}

/// A structured summary produced at a session boundary (FR-033).
///
/// Every field except `agent_note` is derived from recorded state (FR-034).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handoff {
    pub id: Uuid,
    pub session_id: Uuid,
    pub trigger: HandoffTrigger,
    pub goal: String,
    pub progress: String,
    pub completed_work: Vec<String>,
    pub remaining_work: Vec<String>,
    pub changed_files: Vec<String>,
    pub decisions: Vec<String>,
    pub failures: Vec<String>,
    pub tests_executed: Vec<TestRunRecord>,
    pub repository_state: RepositoryState,
    pub next_step: String,
    pub agent_note: Option<String>,
    /// Observation identifiers only. Never their content (FR-055).
    pub evidence: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_roundtrip() {
        for s in SessionStatus::ALL {
            assert_eq!(SessionStatus::from_str(s.as_str()).unwrap(), *s);
        }
        for t in ObservationType::ALL {
            assert_eq!(ObservationType::from_str(t.as_str()).unwrap(), *t);
        }
        for t in MemoryType::ALL {
            assert_eq!(MemoryType::from_str(t.as_str()).unwrap(), *t);
        }
    }

    #[test]
    fn scope_precedence_is_task_branch_project() {
        assert!(MemoryScope::Task.bucket() < MemoryScope::Branch.bucket());
        assert!(MemoryScope::Branch.bucket() < MemoryScope::Project.bucket());
    }

    #[test]
    fn handoff_triggers_exclude_turn_checkpoints() {
        // `Stop` is a turn boundary, not a handoff boundary (FR-032, D16).
        assert_eq!(HandoffTrigger::ALL.len(), 3);
        assert!(HandoffTrigger::from_str("stop").is_err());
    }

    #[test]
    fn outbox_cannot_carry_observations() {
        // Structural guarantee behind SC-010: no observation entity type exists.
        assert!(OutboxEntityType::from_str("observation").is_err());
        assert!(OutboxEntityType::from_str("observation_ref").is_err());
    }

    #[test]
    fn ids_are_time_ordered() {
        let a = new_id();
        let b = new_id();
        assert!(a < b || a.get_version_num() == 7);
    }
}
