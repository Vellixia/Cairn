use sqlx::SqliteConnection;

use crate::db::StorageError;
use crate::events::{AppendOutcome, NewEvent};

const AGGREGATE_TYPES: &[&str] = &["repository", "worktree", "session", "project", "task"];

pub async fn append_aggregate_event(
    conn: &mut SqliteConnection,
    event: &NewEvent,
) -> Result<AppendOutcome, StorageError> {
    validate_scope(&event.aggregate_type, &event.aggregate_id)?;

    if let Some((seq,)) =
        sqlx::query_as::<_, (i64,)>("SELECT seq FROM events WHERE idempotency_key = ?")
            .bind(&event.idempotency_key)
            .fetch_optional(&mut *conn)
            .await?
    {
        return Ok(AppendOutcome {
            seq,
            deduplicated: true,
        });
    }

    let aggregate_seq =
        allocate_aggregate_seq(conn, &event.aggregate_type, &event.aggregate_id).await?;
    sqlx::query(
        "INSERT INTO events (id, idempotency_key, event_type, repository_id, worktree_id, session_id, snapshot_id, payload, recorded_at, aggregate_type, aggregate_id, aggregate_seq) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(&event.aggregate_type)
    .bind(&event.aggregate_id)
    .bind(aggregate_seq)
    .execute(&mut *conn)
    .await?;
    let (seq,): (i64,) = sqlx::query_as("SELECT last_insert_rowid()")
        .fetch_one(&mut *conn)
        .await?;
    Ok(AppendOutcome {
        seq,
        deduplicated: false,
    })
}

pub async fn allocate_aggregate_seq(
    conn: &mut SqliteConnection,
    aggregate_type: &str,
    aggregate_id: &str,
) -> Result<i64, StorageError> {
    validate_scope(aggregate_type, aggregate_id)?;
    let (sequence,): (i64,) = sqlx::query_as(
        "INSERT INTO event_aggregate_heads (aggregate_type, aggregate_id, last_seq) VALUES (?, ?, 1) ON CONFLICT (aggregate_type, aggregate_id) DO UPDATE SET last_seq = event_aggregate_heads.last_seq + 1 RETURNING last_seq",
    )
    .bind(aggregate_type)
    .bind(aggregate_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(sequence)
}

pub fn validate_scope(aggregate_type: &str, aggregate_id: &str) -> Result<(), StorageError> {
    if !AGGREGATE_TYPES.contains(&aggregate_type) || aggregate_id.trim().is_empty() {
        return Err(StorageError::Conflict("invalid aggregate scope".into()));
    }
    Ok(())
}

pub fn derive_event_key(
    operation_identity: &str,
    method: &str,
    event_position: u16,
    event_type: &str,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"cairn.event-idempotency.v1\0");
    update_part(&mut hasher, operation_identity.as_bytes());
    update_part(&mut hasher, method.as_bytes());
    update_part(&mut hasher, &event_position.to_be_bytes());
    update_part(&mut hasher, event_type.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn update_part(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u64).to_be_bytes());
    hasher.update(value);
}
