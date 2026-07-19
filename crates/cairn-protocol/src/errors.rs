//! Closed error-code set (contracts/ipc-contract.md) plus CLI-only codes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NotARepository,
    NotAWorktree,
    NotRegistered,
    IdentityConflict,
    SnapshotContention,
    SessionNotFound,
    SessionNotLive,
    SessionNotRecovering,
    SessionAmbiguous,
    LeaseMismatch,
    LeaseExpired,
    GraceExpired,
    InvalidAgentInstance,
    WatcherStartFailed,
    GitUnavailable,
    StateCorrupted,
    Internal,
    // CLI-only codes:
    DaemonUnavailable,
    Usage,
}

impl ErrorCode {
    /// Stable CLI exit code mapping (contracts/cli-json-contract.md).
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorCode::NotARepository | ErrorCode::NotAWorktree | ErrorCode::NotRegistered => 3,
            ErrorCode::SessionAmbiguous => 4,
            ErrorCode::DaemonUnavailable => 5,
            ErrorCode::StateCorrupted => 6,
            ErrorCode::Usage => 2,
            _ => 1,
        }
    }
}
