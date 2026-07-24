use sqlx::{SqliteConnection, SqlitePool};

use crate::db::StorageError;
use crate::records::{ProjectRepositoryAssociationRow, ProjectRow};

pub async fn get(pool: &SqlitePool, project_id: &str) -> Result<Option<ProjectRow>, StorageError> {
    Ok(sqlx::query_as("SELECT * FROM projects WHERE id = ?")
        .bind(project_id)
        .fetch_optional(pool)
        .await?)
}

pub async fn get_in_tx(
    conn: &mut SqliteConnection,
    project_id: &str,
) -> Result<Option<ProjectRow>, StorageError> {
    Ok(sqlx::query_as("SELECT * FROM projects WHERE id = ?")
        .bind(project_id)
        .fetch_optional(&mut *conn)
        .await?)
}

pub async fn list(
    pool: &SqlitePool,
    after_id: Option<&str>,
    limit: u32,
) -> Result<Vec<ProjectRow>, StorageError> {
    Ok(
        sqlx::query_as("SELECT * FROM projects WHERE (? IS NULL OR id > ?) ORDER BY id LIMIT ?")
            .bind(after_id)
            .bind(after_id)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await?,
    )
}

pub async fn list_filtered(
    pool: &SqlitePool,
    status: Option<&str>,
    after_id: Option<&str>,
    limit: u32,
) -> Result<Vec<ProjectRow>, StorageError> {
    Ok(sqlx::query_as(
        "SELECT * FROM projects WHERE (? IS NULL OR status = ?) AND (? IS NULL OR id > ?) ORDER BY id LIMIT ?",
    )
    .bind(status)
    .bind(status)
    .bind(after_id)
    .bind(after_id)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?)
}

pub async fn insert(conn: &mut SqliteConnection, row: &ProjectRow) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO projects (id, name, description, status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.name)
    .bind(&row.description)
    .bind(&row.status)
    .bind(&row.created_at)
    .bind(&row.updated_at)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn update_metadata(
    conn: &mut SqliteConnection,
    project_id: &str,
    name: &str,
    description: Option<&str>,
    status: &str,
    updated_at: &str,
) -> Result<(), StorageError> {
    let result =
        sqlx::query("UPDATE projects SET name=?, description=?, status=?, updated_at=? WHERE id=?")
            .bind(name)
            .bind(description)
            .bind(status)
            .bind(updated_at)
            .bind(project_id)
            .execute(&mut *conn)
            .await?;
    if result.rows_affected() == 0 {
        return Err(StorageError::NotFound);
    }
    Ok(())
}

pub async fn require_active(
    conn: &mut SqliteConnection,
    project_id: &str,
) -> Result<(), StorageError> {
    let status: Option<(String,)> = sqlx::query_as("SELECT status FROM projects WHERE id=?")
        .bind(project_id)
        .fetch_optional(&mut *conn)
        .await?;
    match status.as_ref().map(|(value,)| value.as_str()) {
        Some("active") => Ok(()),
        Some(_) => Err(StorageError::Conflict("project is archived".into())),
        None => Err(StorageError::NotFound),
    }
}

pub async fn association_by_repository(
    pool: &SqlitePool,
    repository_id: &str,
) -> Result<Option<ProjectRepositoryAssociationRow>, StorageError> {
    Ok(
        sqlx::query_as("SELECT * FROM project_repository_associations WHERE repository_id=?")
            .bind(repository_id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn association_by_repository_in_tx(
    conn: &mut SqliteConnection,
    repository_id: &str,
) -> Result<Option<ProjectRepositoryAssociationRow>, StorageError> {
    Ok(
        sqlx::query_as("SELECT * FROM project_repository_associations WHERE repository_id=?")
            .bind(repository_id)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

pub async fn association_by_id_in_tx(
    conn: &mut SqliteConnection,
    association_id: &str,
) -> Result<Option<ProjectRepositoryAssociationRow>, StorageError> {
    Ok(
        sqlx::query_as("SELECT * FROM project_repository_associations WHERE id=?")
            .bind(association_id)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

pub async fn list_associations(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<Vec<ProjectRepositoryAssociationRow>, StorageError> {
    Ok(sqlx::query_as(
        "SELECT * FROM project_repository_associations WHERE project_id=? ORDER BY repository_id",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?)
}

pub async fn list_all_associations(
    pool: &SqlitePool,
) -> Result<Vec<ProjectRepositoryAssociationRow>, StorageError> {
    Ok(
        sqlx::query_as("SELECT * FROM project_repository_associations ORDER BY repository_id")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn insert_association(
    conn: &mut SqliteConnection,
    row: &ProjectRepositoryAssociationRow,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO project_repository_associations (id, project_id, repository_id, associated_at, event_seq) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.project_id)
    .bind(&row.repository_id)
    .bind(&row.associated_at)
    .bind(row.event_seq)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn repository_exists_in_tx(
    conn: &mut SqliteConnection,
    repository_id: &str,
) -> Result<bool, StorageError> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM repositories WHERE id=?")
        .bind(repository_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(count == 1)
}

pub async fn project_for_worktree(
    pool: &SqlitePool,
    worktree_id: &str,
) -> Result<Option<ProjectRow>, StorageError> {
    Ok(sqlx::query_as(
        "SELECT p.* FROM worktrees w JOIN project_repository_associations a ON a.repository_id=w.repository_id JOIN projects p ON p.id=a.project_id WHERE w.id=?",
    )
    .bind(worktree_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn task_count(pool: &SqlitePool, project_id: &str) -> Result<u64, StorageError> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tasks WHERE project_id=?")
        .bind(project_id)
        .fetch_one(pool)
        .await?;
    Ok(count as u64)
}

pub async fn bound_session_count(pool: &SqlitePool, project_id: &str) -> Result<u64, StorageError> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM session_bindings WHERE project_id=?")
            .bind(project_id)
            .fetch_one(pool)
            .await?;
    Ok(count as u64)
}
