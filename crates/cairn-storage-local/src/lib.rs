//! Daemon-local SQLite persistence: pool bootstrap, migrations, DAOs, and the
//! serialized per-worktree transactional event append (arch rules 2–6).

pub mod db;
pub mod events;
pub mod records;
pub mod repos;
pub mod sessions;
pub mod snapshots;
pub mod worktrees;
pub mod writer;

pub use db::{data_dir, db_path, open_pool, open_pool_at, StorageError};
pub use events::{AppendOutcome, EventRow, NewEvent};
pub use records::*;
pub use writer::WorktreeWriters;
