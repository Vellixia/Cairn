//! T013: repositories DAO.

use sqlx::{Executor, Sqlite, SqlitePool};

use crate::db::StorageError;
use crate::records::RepositoryRow;

pub async fn get_by_uuid(
    pool: &SqlitePool,
    repo_uuid: &str,
) -> Result<Option<RepositoryRow>, StorageError> {
    Ok(
        sqlx::query_as::<_, RepositoryRow>("SELECT * FROM repositories WHERE repo_uuid = ?")
            .bind(repo_uuid)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<Option<RepositoryRow>, StorageError> {
    Ok(
        sqlx::query_as::<_, RepositoryRow>("SELECT * FROM repositories WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Marker-loss restoration lookup (analysis U1): match by canonical path.
pub async fn get_by_canonical_path(
    pool: &SqlitePool,
    canonical_path: &str,
) -> Result<Vec<RepositoryRow>, StorageError> {
    Ok(
        sqlx::query_as::<_, RepositoryRow>("SELECT * FROM repositories WHERE canonical_path = ?")
            .bind(canonical_path)
            .fetch_all(pool)
            .await?,
    )
}

/// Insert within a caller-owned transaction (event append pairs with it).
pub async fn insert<'e, E>(exec: E, row: &RepositoryRow) -> Result<(), StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO repositories (id, repo_uuid, canonical_path, default_remote_name, \
         default_remote_url, copied_from_repository_id, registered_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.repo_uuid)
    .bind(&row.canonical_path)
    .bind(&row.default_remote_name)
    .bind(&row.default_remote_url)
    .bind(&row.copied_from_repository_id)
    .bind(&row.registered_at)
    .execute(exec)
    .await?;
    Ok(())
}

/// Canonical path is mutable metadata (moves/renames preserve identity).
pub async fn update_canonical_path(
    pool: &SqlitePool,
    id: &str,
    canonical_path: &str,
) -> Result<(), StorageError> {
    sqlx::query("UPDATE repositories SET canonical_path = ? WHERE id = ?")
        .bind(canonical_path)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn count(pool: &SqlitePool) -> Result<u64, StorageError> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM repositories")
        .fetch_one(pool)
        .await?;
    Ok(n as u64)
}
