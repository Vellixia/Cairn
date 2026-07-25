//! Transactional append-only event persistence.

use sqlx::{SqliteConnection, SqlitePool};

use crate::db::StorageError;
use crate::writer::{begin_immediate, ImmediateWriteFn, WorktreeWriters, WriterPolicy};

#[derive(Debug, Clone)]
pub struct NewEvent {
    pub id: String,
    pub idempotency_key: String,
    pub event_type: String,
    pub repository_id: Option<String>,
    pub worktree_id: Option<String>,
    pub session_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub payload: serde_json::Value,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Copy)]
pub struct AppendOutcome {
    pub seq: i64,
    pub deduplicated: bool,
}

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
    pub aggregate_type: Option<String>,
    pub aggregate_id: Option<String>,
    pub aggregate_seq: Option<i64>,
}

pub async fn append_event(
    conn: &mut SqliteConnection,
    event: &NewEvent,
) -> Result<AppendOutcome, StorageError> {
    crate::aggregate_events::append_aggregate_event(conn, event).await
}

pub type TxnFn<T> = ImmediateWriteFn<T>;

/// The process-local lock preserves Feature 001 scheduling behavior; SQLite's
/// BEGIN IMMEDIATE lock is the cross-connection correctness boundary.
pub async fn serialized_txn<T: Send + 'static>(
    pool: &SqlitePool,
    writers: &WorktreeWriters,
    worktree_key: &str,
    f: TxnFn<T>,
) -> Result<T, StorageError> {
    let lock = writers.lock_for(worktree_key);
    let _guard = lock.lock().await;
    begin_immediate(pool, WriterPolicy::default(), None, f).await
}

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

pub async fn list_events(
    pool: &SqlitePool,
    repository_id: Option<&str>,
    worktree_id: Option<&str>,
    session_id: Option<&str>,
    after_seq: Option<i64>,
    limit: u32,
) -> Result<Vec<EventRow>, StorageError> {
    list_events_filtered(
        pool,
        repository_id,
        worktree_id,
        session_id,
        None,
        None,
        after_seq,
        limit,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn list_events_filtered(
    pool: &SqlitePool,
    repository_id: Option<&str>,
    worktree_id: Option<&str>,
    session_id: Option<&str>,
    aggregate_type: Option<&str>,
    aggregate_id: Option<&str>,
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
    if aggregate_type.is_some() {
        sql.push_str(" AND aggregate_type = ?");
    }
    if aggregate_id.is_some() {
        sql.push_str(" AND aggregate_id = ?");
    }
    if after_seq.is_some() {
        sql.push_str(" AND seq > ?");
    }
    sql.push_str(" ORDER BY seq ASC LIMIT ?");

    let mut query = sqlx::query_as::<_, EventRow>(&sql);
    if let Some(value) = repository_id {
        query = query.bind(value.to_string());
    }
    if let Some(value) = worktree_id {
        query = query.bind(value.to_string());
    }
    if let Some(value) = session_id {
        query = query.bind(value.to_string());
    }
    if let Some(value) = aggregate_type {
        query = query.bind(value.to_string());
    }
    if let Some(value) = aggregate_id {
        query = query.bind(value.to_string());
    }
    if let Some(value) = after_seq {
        query = query.bind(value);
    }
    query = query.bind(i64::from(limit));
    Ok(query.fetch_all(pool).await?)
}

pub async fn count_events(pool: &SqlitePool) -> Result<u64, StorageError> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events")
        .fetch_one(pool)
        .await?;
    Ok(count as u64)
}

pub async fn get_by_id_in_tx(
    conn: &mut SqliteConnection,
    event_id: &str,
) -> Result<Option<EventRow>, StorageError> {
    Ok(sqlx::query_as("SELECT * FROM events WHERE id=?")
        .bind(event_id)
        .fetch_optional(&mut *conn)
        .await?)
}
