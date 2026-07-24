//! Daemon-local SQLite persistence: pool bootstrap, migrations, DAOs, and the
//! serialized per-worktree transactional event append (arch rules 2–6).

pub mod aggregate_events;
pub mod db;
pub mod events;
pub mod operation_idempotency;
pub mod projects;
pub mod records;
pub mod repos;
pub mod session_bindings;
pub mod sessions;
pub mod snapshots;
pub mod tasks;
pub mod worktrees;
pub mod writer;

pub use db::{data_dir, db_path, open_pool, open_pool_at, IdempotencyConflictReason, StorageError};
pub use events::{AppendOutcome, EventRow, NewEvent};
pub use records::*;
pub use writer::{WorktreeWriters, WriteCheckpoint, WriteTestHooks, WriterPolicy};
