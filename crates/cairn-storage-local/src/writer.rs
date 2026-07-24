//! T014 (analysis I4): per-worktree single-writer serialization.
//!
//! Every event append + projection change for one worktree flows through the
//! same async mutex, so sequence assignment and projection updates can never
//! interleave out of order. Events without a worktree scope serialize on a
//! reserved global key.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sqlx::{Connection, SqliteConnection, SqlitePool};

use crate::db::StorageError;

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

#[derive(Debug, Clone, Copy)]
pub struct WriterPolicy {
    busy_timeout: Duration,
}

impl Default for WriterPolicy {
    fn default() -> Self {
        Self {
            busy_timeout: Duration::from_millis(5_000),
        }
    }
}

impl WriterPolicy {
    pub fn test_with_busy_timeout(busy_timeout: Duration) -> Self {
        Self { busy_timeout }
    }

    pub fn max_elapsed_ms(self) -> u64 {
        self.busy_timeout.as_millis().try_into().unwrap_or(u64::MAX)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WriteCheckpoint {
    PreRegistryReservation,
    PostRegistryReservation,
    PostCounterAllocation,
    PreEvent,
    PostEvent,
    BetweenEvents,
    PreProjection,
    PostProjection,
    PreResultLocator,
    PreCommit,
}

#[derive(Default)]
struct HookState {
    fail: HashSet<WriteCheckpoint>,
    pauses: HashMap<WriteCheckpoint, Arc<CheckpointPause>>,
}

#[derive(Default)]
struct CheckpointPause {
    reached: tokio::sync::Notify,
    resume: tokio::sync::Notify,
    was_reached: AtomicBool,
    was_resumed: AtomicBool,
}

/// Injectable deterministic controls. Production callers pass `None`.
#[derive(Clone, Default)]
pub struct WriteTestHooks {
    state: Arc<Mutex<HookState>>,
}

impl std::fmt::Debug for WriteTestHooks {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WriteTestHooks(..)")
    }
}

impl WriteTestHooks {
    pub fn fail_at(&self, point: WriteCheckpoint) {
        self.state
            .lock()
            .expect("write hook poisoned")
            .fail
            .insert(point);
    }

    pub fn pause_at(&self, point: WriteCheckpoint) {
        self.state
            .lock()
            .expect("write hook poisoned")
            .pauses
            .insert(point, Arc::new(CheckpointPause::default()));
    }

    pub async fn wait_until_reached(&self, point: WriteCheckpoint) {
        let pause = self
            .state
            .lock()
            .expect("write hook poisoned")
            .pauses
            .get(&point)
            .cloned()
            .expect("checkpoint was not configured");
        while !pause.was_reached.load(Ordering::Acquire) {
            pause.reached.notified().await;
        }
    }

    pub fn resume(&self, point: WriteCheckpoint) {
        if let Some(pause) = self
            .state
            .lock()
            .expect("write hook poisoned")
            .pauses
            .get(&point)
        {
            pause.was_resumed.store(true, Ordering::Release);
            pause.resume.notify_waiters();
        }
    }

    pub async fn checkpoint(&self, point: WriteCheckpoint) -> Result<(), StorageError> {
        let (fail, pause) = {
            let state = self.state.lock().expect("write hook poisoned");
            (
                state.fail.contains(&point),
                state.pauses.get(&point).cloned(),
            )
        };
        if fail {
            return Err(StorageError::Conflict(format!(
                "injected rollback at {point:?}"
            )));
        }
        if let Some(pause) = pause {
            pause.was_reached.store(true, Ordering::Release);
            pause.reached.notify_waiters();
            while !pause.was_resumed.load(Ordering::Acquire) {
                pause.resume.notified().await;
            }
        }
        Ok(())
    }
}

pub type ImmediateWriteFn<T> = Box<
    dyn for<'c> FnOnce(
            &'c mut SqliteConnection,
        )
            -> Pin<Box<dyn Future<Output = Result<T, StorageError>> + Send + 'c>>
        + Send
        + 'static,
>;

/// Runs one mutation under SQLite's write lock. There are no application
/// retries: after the bounded connection timeout, callers receive one stable
/// busy result.
pub async fn begin_immediate<T: Send + 'static>(
    pool: &SqlitePool,
    policy: WriterPolicy,
    hooks: Option<WriteTestHooks>,
    f: ImmediateWriteFn<T>,
) -> Result<T, StorageError> {
    let mut conn = pool.acquire().await?;
    let timeout_ms = policy.max_elapsed_ms();
    sqlx::query(&format!("PRAGMA busy_timeout = {timeout_ms}"))
        .execute(&mut *conn)
        .await?;

    let begin = conn.begin_with("BEGIN IMMEDIATE");
    let mut tx = match tokio::time::timeout(policy.busy_timeout, begin).await {
        Ok(Ok(tx)) => tx,
        Ok(Err(error)) if is_busy(&error) => {
            return Err(StorageError::StorageBusy {
                max_elapsed_ms: timeout_ms,
            });
        }
        Ok(Err(error)) => return Err(StorageError::Sqlx(error)),
        Err(_) => {
            return Err(StorageError::StorageBusy {
                max_elapsed_ms: timeout_ms,
            });
        }
    };

    match f(&mut tx).await {
        Ok(value) => {
            if let Some(hooks) = hooks {
                if let Err(error) = hooks.checkpoint(WriteCheckpoint::PreCommit).await {
                    tx.rollback().await?;
                    return Err(error);
                }
            }
            tx.commit().await?;
            Ok(value)
        }
        Err(error) => {
            tx.rollback().await?;
            Err(error)
        }
    }
}

fn is_busy(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(database) => {
            matches!(database.code().as_deref(), Some("5" | "6"))
                || database.message().contains("locked")
                || database.message().contains("busy")
        }
        _ => false,
    }
}
