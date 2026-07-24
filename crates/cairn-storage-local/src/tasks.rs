use sqlx::{SqliteConnection, SqlitePool};

use crate::db::StorageError;
use crate::records::{TaskRevisionRow, TaskRow};
use crate::writer::{WriteCheckpoint, WriteTestHooks};

pub async fn get(pool: &SqlitePool, task_id: &str) -> Result<Option<TaskRow>, StorageError> {
    Ok(sqlx::query_as("SELECT * FROM tasks WHERE id=?")
        .bind(task_id)
        .fetch_optional(pool)
        .await?)
}

pub async fn get_in_tx(
    conn: &mut SqliteConnection,
    task_id: &str,
) -> Result<Option<TaskRow>, StorageError> {
    Ok(sqlx::query_as("SELECT * FROM tasks WHERE id=?")
        .bind(task_id)
        .fetch_optional(&mut *conn)
        .await?)
}

pub async fn list_by_project(
    pool: &SqlitePool,
    project_id: &str,
    after_id: Option<&str>,
    limit: u32,
) -> Result<Vec<TaskRow>, StorageError> {
    Ok(sqlx::query_as(
        "SELECT * FROM tasks WHERE project_id=? AND (? IS NULL OR id>?) ORDER BY id LIMIT ?",
    )
    .bind(project_id)
    .bind(after_id)
    .bind(after_id)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?)
}

pub async fn revision(
    pool: &SqlitePool,
    revision_id: &str,
) -> Result<Option<TaskRevisionRow>, StorageError> {
    Ok(sqlx::query_as("SELECT * FROM task_revisions WHERE id=?")
        .bind(revision_id)
        .fetch_optional(pool)
        .await?)
}

pub async fn revision_in_tx(
    conn: &mut SqliteConnection,
    revision_id: &str,
) -> Result<Option<TaskRevisionRow>, StorageError> {
    Ok(sqlx::query_as("SELECT * FROM task_revisions WHERE id=?")
        .bind(revision_id)
        .fetch_optional(&mut *conn)
        .await?)
}

pub async fn latest_revision_in_tx(
    conn: &mut SqliteConnection,
    task_id: &str,
    revision_number: i64,
) -> Result<TaskRevisionRow, StorageError> {
    sqlx::query_as("SELECT * FROM task_revisions WHERE task_id=? AND revision_number=?")
        .bind(task_id)
        .bind(revision_number)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(StorageError::NotFound)
}

pub async fn revisions(
    pool: &SqlitePool,
    task_id: &str,
) -> Result<Vec<TaskRevisionRow>, StorageError> {
    Ok(
        sqlx::query_as("SELECT * FROM task_revisions WHERE task_id=? ORDER BY revision_number")
            .bind(task_id)
            .fetch_all(pool)
            .await?,
    )
}

pub async fn list_all(pool: &SqlitePool) -> Result<Vec<TaskRow>, StorageError> {
    Ok(sqlx::query_as("SELECT * FROM tasks ORDER BY id")
        .fetch_all(pool)
        .await?)
}

pub async fn list_all_revisions(pool: &SqlitePool) -> Result<Vec<TaskRevisionRow>, StorageError> {
    Ok(
        sqlx::query_as("SELECT * FROM task_revisions ORDER BY task_id, revision_number")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn insert_task(
    conn: &mut SqliteConnection,
    task: &TaskRow,
    revision: &TaskRevisionRow,
) -> Result<(), StorageError> {
    if task.latest_revision_number != 1
        || revision.task_id != task.id
        || revision.revision_number != 1
        || revision.parent_revision_id.is_some()
    {
        return Err(StorageError::Conflict("invalid first task revision".into()));
    }
    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, latest_revision_number, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&task.id)
    .bind(&task.project_id)
    .bind(&task.title)
    .bind(task.latest_revision_number)
    .bind(&task.created_at)
    .bind(&task.updated_at)
    .execute(&mut *conn)
    .await?;
    insert_revision(conn, revision).await
}

pub async fn insert_next_revision(
    conn: &mut SqliteConnection,
    mut revision: TaskRevisionRow,
    updated_at: &str,
    hooks: Option<&WriteTestHooks>,
) -> Result<TaskRevisionRow, StorageError> {
    let current: TaskRow = sqlx::query_as("SELECT * FROM tasks WHERE id=?")
        .bind(&revision.task_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(StorageError::NotFound)?;
    let next = current.latest_revision_number + 1;
    let parent_id = revision
        .parent_revision_id
        .as_deref()
        .ok_or_else(|| StorageError::Conflict("later revision requires a parent".into()))?;
    let parent: Option<(i64,)> =
        sqlx::query_as("SELECT revision_number FROM task_revisions WHERE id=? AND task_id=?")
            .bind(parent_id)
            .bind(&revision.task_id)
            .fetch_optional(&mut *conn)
            .await?;
    if parent.is_none_or(|(number,)| number >= next) {
        return Err(StorageError::Conflict(
            "invalid task revision parent".into(),
        ));
    }
    let advanced: Option<(i64,)> = sqlx::query_as(
        "UPDATE tasks SET latest_revision_number=latest_revision_number+1, updated_at=? WHERE id=? AND latest_revision_number=? RETURNING latest_revision_number",
    )
    .bind(updated_at)
    .bind(&revision.task_id)
    .bind(current.latest_revision_number)
    .fetch_optional(&mut *conn)
    .await?;
    if advanced != Some((next,)) {
        return Err(StorageError::Conflict(
            "task revision allocation conflicted".into(),
        ));
    }
    if let Some(hooks) = hooks {
        hooks
            .checkpoint(WriteCheckpoint::PostCounterAllocation)
            .await?;
    }
    revision.revision_number = next;
    insert_revision(conn, &revision).await?;
    Ok(revision)
}

async fn insert_revision(
    conn: &mut SqliteConnection,
    row: &TaskRevisionRow,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO task_revisions (id, task_id, revision_number, parent_revision_id, goal_contract_json, goal_contract_schema_version, goal_contract_fingerprint, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.task_id)
    .bind(row.revision_number)
    .bind(&row.parent_revision_id)
    .bind(&row.goal_contract_json)
    .bind(row.goal_contract_schema_version)
    .bind(&row.goal_contract_fingerprint)
    .bind(&row.created_at)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
