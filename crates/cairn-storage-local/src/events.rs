//! T014: idempotent, transactional, serialized event append (arch rules 3–6).
//!
//! - Append + projection update = ONE SQLite transaction.
//! - Sequence assignment happens inside that serialized transaction.
//! - A duplicate idempotency key returns the previously accepted event's
//!   result and does NOT run the projection function again.
//! - The events table's triggers make UPDATE/DELETE impossible (FR-020).

use std::future::Future;
use std::pin::Pin;

use sqlx::{Sqlite, SqliteConnection, SqlitePool, Transaction};

use crate::db::StorageError;
use crate::writer::WorktreeWriters;

/// A new event to append (data-model.md `events`).
#[derive(Debug, Clone)]
pub struct NewEvent {
    pub id: String,
    pub idempotency_key: String,
    pub event_type: String,
    pub repository_id: Option<String>,
    pub worktree_id: Option<String>,
    pub session_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub payload: serde_json::Value,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Copy)]
pub struct AppendOutcome {
    pub seq: i64,
    pub deduplicated: bool,
}

/// A stored event row (replay/list order = seq).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventRow {
    pub seq: i64,
    pub id: String,
    pub idempotency_key: String,
    pub event_type: String,
    pub repository_id: Option<String>,
    pub worktree_id: Option<String>,
    pub session_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub payload: String,
    pub recorded_at: String,
}

/// Append one event inside an existing transaction. Duplicate idempotency
/// keys dedupe to the original row.
pub async fn append_event(
    conn: &mut SqliteConnection,
    event: &NewEvent,
) -> Result<AppendOutcome, StorageError> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO events (id, idempotency_key, event_type, repository_id, \
         worktree_id, session_id, snapshot_id, payload, recorded_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&event.id)
    .bind(&event.idempotency_key)
    .bind(&event.event_type)
    .bind(&event.repository_id)
    .bind(&event.worktree_id)
    .bind(&event.session_id)
    .bind(&event.snapshot_id)
    .bind(event.payload.to_string())
    .bind(&event.recorded_at)
    .execute(&mut *conn)
    .await?;

    if res.rows_affected() == 0 {
        let (seq,): (i64,) = sqlx::query_as("SELECT seq FROM events WHERE idempotency_key = ?")
            .bind(&event.idempotency_key)
            .fetch_one(&mut *conn)
            .await?;
        return Ok(AppendOutcome {
            seq,
            deduplicated: true,
        });
    }
    let (seq,): (i64,) = sqlx::query_as("SELECT last_insert_rowid()")
        .fetch_one(&mut *conn)
        .await?;
    Ok(AppendOutcome {
        seq,
        deduplicated: false,
    })
}

pub type TxnFn<T> = Box<
    dyn for<'c> FnOnce(
            &'c mut SqliteConnection,
        )
            -> Pin<Box<dyn Future<Output = Result<T, StorageError>> + Send + 'c>>
        + Send
        + 'static,
>;

/// Run `f` inside one transaction serialized on the worktree key. This is the
/// only sanctioned way to mutate projections alongside event appends.
pub async fn serialized_txn<T: Send + 'static>(
    pool: &SqlitePool,
    writers: &WorktreeWriters,
    worktree_key: &str,
    f: TxnFn<T>,
) -> Result<T, StorageError> {
    let lock = writers.lock_for(worktree_key);
    let _guard = lock.lock().await;
    let mut tx: Transaction<'_, Sqlite> = pool.begin().await?;
    let out = f(&mut tx).await;
    match out {
        Ok(v) => {
            tx.commit().await?;
            Ok(v)
        }
        Err(e) => {
            tx.rollback().await?;
            Err(e)
        }
    }
}

/// Convenience used by tests and simple call sites: append one event and run
/// its projection in one serialized transaction. On dedup, the projection is
/// skipped and the original event's outcome returned.
pub async fn append_with_projection(
    pool: &SqlitePool,
    writers: &WorktreeWriters,
    worktree_key: &str,
    event: NewEvent,
    projection: TxnFn<()>,
) -> Result<AppendOutcome, StorageError> {
    serialized_txn(
        pool,
        writers,
        worktree_key,
        Box::new(move |conn| {
            Box::pin(async move {
                let outcome = append_event(conn, &event).await?;
                if !outcome.deduplicated {
                    projection(conn).await?;
                }
                Ok(outcome)
            })
        }),
    )
    .await
}

/// Seq-ordered event listing with optional filters (T024 backing query).
pub async fn list_events(
    pool: &SqlitePool,
    repository_id: Option<&str>,
    worktree_id: Option<&str>,
    session_id: Option<&str>,
    after_seq: Option<i64>,
    limit: u32,
) -> Result<Vec<EventRow>, StorageError> {
    let mut sql = String::from("SELECT * FROM events WHERE 1=1");
    if repository_id.is_some() {
        sql.push_str(" AND repository_id = ?");
    }
    if worktree_id.is_some() {
        sql.push_str(" AND worktree_id = ?");
    }
    if session_id.is_some() {
        sql.push_str(" AND session_id = ?");
    }
    if after_seq.is_some() {
        sql.push_str(" AND seq > ?");
    }
    sql.push_str(" ORDER BY seq ASC LIMIT ?");

    let mut q = sqlx::query_as::<_, EventRow>(&sql);
    if let Some(v) = repository_id {
        q = q.bind(v.to_string());
    }
    if let Some(v) = worktree_id {
        q = q.bind(v.to_string());
    }
    if let Some(v) = session_id {
        q = q.bind(v.to_string());
    }
    if let Some(v) = after_seq {
        q = q.bind(v);
    }
    q = q.bind(i64::from(limit));
    Ok(q.fetch_all(pool).await?)
}

pub async fn count_events(pool: &SqlitePool) -> Result<u64, StorageError> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events")
        .fetch_one(pool)
        .await?;
    Ok(n as u64)
}
