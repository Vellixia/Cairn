//! T013: snapshots DAO — insert-or-get keyed by (worktree_id, snapshot_fp).
//! Snapshots are immutable; the schema's triggers reject UPDATE/DELETE.

use sqlx::{Executor, Sqlite, SqlitePool};

use crate::db::StorageError;
use crate::records::SnapshotRow;

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<Option<SnapshotRow>, StorageError> {
    Ok(
        sqlx::query_as::<_, SnapshotRow>("SELECT * FROM snapshots WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn get_by_fingerprint<'e, E>(
    exec: E,
    worktree_id: &str,
    snapshot_fp: &str,
) -> Result<Option<SnapshotRow>, StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    Ok(sqlx::query_as::<_, SnapshotRow>(
        "SELECT * FROM snapshots WHERE worktree_id = ? AND snapshot_fp = ?",
    )
    .bind(worktree_id)
    .bind(snapshot_fp)
    .fetch_optional(exec)
    .await?)
}

/// Insert a new snapshot row (caller must have checked for an existing
/// fingerprint inside the same serialized transaction).
pub async fn insert<'e, E>(exec: E, row: &SnapshotRow) -> Result<(), StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO snapshots (id, worktree_id, branch, head_commit, staged_fp, unstaged_fp, \
         untracked_fp, snapshot_fp, fp_schema_version, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.worktree_id)
    .bind(&row.branch)
    .bind(&row.head_commit)
    .bind(&row.staged_fp)
    .bind(&row.unstaged_fp)
    .bind(&row.untracked_fp)
    .bind(&row.snapshot_fp)
    .bind(row.fp_schema_version)
    .bind(&row.created_at)
    .execute(exec)
    .await?;
    Ok(())
}
