//! IPC method handlers. Thin: policy lives in crates (module ownership map).

pub mod convert;
pub mod daemon;
pub mod events;
pub mod repository;
pub mod session;
pub mod snapshot;

use cairn_git::GitError;
use cairn_protocol::{ErrorCode, ErrorData};
use cairn_session::SessionError;
use cairn_storage_local::StorageError;

/// Handler-level error carrying its wire code.
#[derive(Debug)]
pub struct HandlerError {
    pub code: ErrorCode,
    pub message: String,
    pub data: Option<ErrorData>,
}

impl HandlerError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: ErrorData) -> Self {
        self.data = Some(data);
        self
    }
}

impl From<GitError> for HandlerError {
    fn from(e: GitError) -> Self {
        let code = match &e {
            GitError::GitUnavailable(_) => ErrorCode::GitUnavailable,
            GitError::NotARepository(_) => ErrorCode::NotARepository,
            GitError::NotAWorktree(_) => ErrorCode::NotAWorktree,
            GitError::SnapshotContention(_) => ErrorCode::SnapshotContention,
            _ => ErrorCode::Internal,
        };
        Self::new(code, e.to_string())
    }
}

impl From<StorageError> for HandlerError {
    fn from(e: StorageError) -> Self {
        let code = if e.is_corruption() {
            ErrorCode::StateCorrupted
        } else {
            ErrorCode::Internal
        };
        Self::new(code, e.to_string())
    }
}

impl From<SessionError> for HandlerError {
    fn from(e: SessionError) -> Self {
        let code = match &e {
            SessionError::NotFound => ErrorCode::SessionNotFound,
            SessionError::NotLive => ErrorCode::SessionNotLive,
            SessionError::NotRecovering => ErrorCode::SessionNotRecovering,
            SessionError::Ambiguous(_) => ErrorCode::SessionAmbiguous,
            SessionError::LeaseMismatch => ErrorCode::LeaseMismatch,
            SessionError::LeaseExpired => ErrorCode::LeaseExpired,
            SessionError::GraceExpired => ErrorCode::GraceExpired,
            SessionError::Storage(s) if s.is_corruption() => ErrorCode::StateCorrupted,
            SessionError::Storage(_) => ErrorCode::Internal,
        };
        Self::new(code, e.to_string())
    }
}

impl From<serde_json::Error> for HandlerError {
    fn from(e: serde_json::Error) -> Self {
        Self::new(ErrorCode::Usage, format!("invalid params: {e}"))
    }
}

pub type HandlerResult<T> = Result<T, HandlerError>;
