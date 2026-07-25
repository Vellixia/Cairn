use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git binary unavailable: {0}")]
    GitUnavailable(String),
    #[error("not a git repository: {0}")]
    NotARepository(String),
    #[error("bare repository (no working tree): {0}")]
    NotAWorktree(String),
    #[error("git command failed: {0}")]
    CommandFailed(String),
    #[error("failed to parse git output: {0}")]
    Parse(String),
    #[error("snapshot contention: repository changed during {0} consistency attempts")]
    SnapshotContention(u32),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
