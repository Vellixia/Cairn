//! T014 (analysis I4): per-worktree single-writer serialization.
//!
//! Every event append + projection change for one worktree flows through the
//! same async mutex, so sequence assignment and projection updates can never
//! interleave out of order. Events without a worktree scope serialize on a
//! reserved global key.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Reserved key for events not scoped to a worktree.
pub const GLOBAL_KEY: &str = "__global__";

#[derive(Default)]
pub struct WorktreeWriters {
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl WorktreeWriters {
    pub fn new() -> Self {
        Self::default()
    }

    /// The serialization lock for one worktree (or the global key).
    pub fn lock_for(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self.locks.lock().expect("writer lock map poisoned");
        map.entry(key.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}
