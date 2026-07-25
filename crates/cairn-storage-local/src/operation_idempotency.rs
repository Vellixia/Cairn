use sqlx::{SqliteConnection, SqlitePool};

use crate::db::{IdempotencyConflictReason, StorageError};
use crate::records::OperationIdempotencyRow;
use crate::writer::{WriteCheckpoint, WriteTestHooks};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reservation {
    Inserted(OperationIdempotencyRow),
    Existing(OperationIdempotencyRow),
}

pub async fn get(
    pool: &SqlitePool,
    idempotency_key: &str,
) -> Result<Option<OperationIdempotencyRow>, StorageError> {
    Ok(
        sqlx::query_as("SELECT * FROM operation_idempotency WHERE idempotency_key=?")
            .bind(idempotency_key)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn get_in_tx(
    conn: &mut SqliteConnection,
    idempotency_key: &str,
) -> Result<Option<OperationIdempotencyRow>, StorageError> {
    Ok(
        sqlx::query_as("SELECT * FROM operation_idempotency WHERE idempotency_key=?")
            .bind(idempotency_key)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

pub async fn reserve_or_get(
    conn: &mut SqliteConnection,
    proposed: &OperationIdempotencyRow,
    hooks: Option<&WriteTestHooks>,
) -> Result<Reservation, StorageError> {
    if let Some(hooks) = hooks {
        hooks
            .checkpoint(WriteCheckpoint::PreRegistryReservation)
            .await?;
    }
    if let Some(existing) = sqlx::query_as::<_, OperationIdempotencyRow>(
        "SELECT * FROM operation_idempotency WHERE idempotency_key=?",
    )
    .bind(&proposed.idempotency_key)
    .fetch_optional(&mut *conn)
    .await?
    {
        if existing.method == proposed.method
            && existing.request_fingerprint == proposed.request_fingerprint
        {
            return Ok(Reservation::Existing(existing));
        }
        let reason = if existing.method != proposed.method {
            IdempotencyConflictReason::MethodMismatch
        } else {
            IdempotencyConflictReason::RequestMismatch
        };
        return Err(StorageError::IdempotencyConflict {
            existing_method: existing.method,
            reason,
        });
    }
    sqlx::query(
        "INSERT INTO operation_idempotency (idempotency_key, method, request_fingerprint, result_kind, result_locator, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&proposed.idempotency_key)
    .bind(&proposed.method)
    .bind(&proposed.request_fingerprint)
    .bind(&proposed.result_kind)
    .bind(&proposed.result_locator)
    .bind(&proposed.created_at)
    .execute(&mut *conn)
    .await?;
    if let Some(hooks) = hooks {
        hooks
            .checkpoint(WriteCheckpoint::PostRegistryReservation)
            .await?;
    }
    Ok(Reservation::Inserted(proposed.clone()))
}
