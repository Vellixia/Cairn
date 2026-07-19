//! T013: worktrees DAO.

use sqlx::{Executor, Sqlite, SqlitePool};

use crate::db::StorageError;
use crate::records::WorktreeRow;

pub async fn get_by_uuid(
    pool: &SqlitePool,
    worktree_uuid: &str,
) -> Result<Option<WorktreeRow>, StorageError> {
    Ok(
        sqlx::query_as::<_, WorktreeRow>("SELECT * FROM worktrees WHERE worktree_uuid = ?")
            .bind(worktree_uuid)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<Option<WorktreeRow>, StorageError> {
    Ok(
        sqlx::query_as::<_, WorktreeRow>("SELECT * FROM worktrees WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn list_by_repository(
    pool: &SqlitePool,
    repository_id: &str,
) -> Result<Vec<WorktreeRow>, StorageError> {
    Ok(
        sqlx::query_as::<_, WorktreeRow>("SELECT * FROM worktrees WHERE repository_id = ?")
            .bind(repository_id)
            .fetch_all(pool)
            .await?,
    )
}

pub async fn insert<'e, E>(exec: E, row: &WorktreeRow) -> Result<(), StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO worktrees (id, repository_id, worktree_uuid, path, is_main, registered_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.repository_id)
    .bind(&row.worktree_uuid)
    .bind(&row.path)
    .bind(row.is_main)
    .bind(&row.registered_at)
    .execute(exec)
    .await?;
    Ok(())
}

pub async fn update_path(pool: &SqlitePool, id: &str, path: &str) -> Result<(), StorageError> {
    sqlx::query("UPDATE worktrees SET path = ? WHERE id = ?")
        .bind(path)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
