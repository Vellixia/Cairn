//! Row → domain mapping.
//!
//! Identifiers and timestamps are stored as text so the database stays
//! readable in any client; these helpers are the single place that knows it.

use crate::{Result, StoreError};
use cairn_core::domain::*;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;

pub fn uuid(row: &SqliteRow, col: &str) -> Result<Uuid> {
    let raw: String = row.try_get(col)?;
    Uuid::parse_str(&raw).map_err(|e| StoreError::Corrupt(format!("{col}: {e}")))
}

pub fn opt_uuid(row: &SqliteRow, col: &str) -> Result<Option<Uuid>> {
    let raw: Option<String> = row.try_get(col)?;
    match raw {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => Uuid::parse_str(&s)
            .map(Some)
            .map_err(|e| StoreError::Corrupt(format!("{col}: {e}"))),
    }
}

pub fn ts(row: &SqliteRow, col: &str) -> Result<DateTime<Utc>> {
    let raw: String = row.try_get(col)?;
    parse_ts(&raw).ok_or_else(|| StoreError::Corrupt(format!("{col}: {raw}")))
}

pub fn opt_ts(row: &SqliteRow, col: &str) -> Result<Option<DateTime<Utc>>> {
    let raw: Option<String> = row.try_get(col)?;
    match raw {
        None => Ok(None),
        Some(s) => parse_ts(&s)
            .map(Some)
            .ok_or_else(|| StoreError::Corrupt(format!("{col}: {s}"))),
    }
}

fn parse_ts(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

pub fn enum_val<T: FromStr>(row: &SqliteRow, col: &str) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    let raw: String = row.try_get(col)?;
    T::from_str(&raw).map_err(|e| StoreError::Corrupt(format!("{col}: {e}")))
}

pub fn json_field<T: DeserializeOwned>(row: &SqliteRow, col: &str) -> Result<T> {
    let raw: String = row.try_get(col)?;
    serde_json::from_str(&raw).map_err(|e| StoreError::Corrupt(format!("{col}: {e}")))
}

pub fn opt_json(row: &SqliteRow, col: &str) -> Result<Option<serde_json::Value>> {
    let raw: Option<String> = row.try_get(col)?;
    match raw {
        None => Ok(None),
        Some(s) => serde_json::from_str(&s)
            .map(Some)
            .map_err(|e| StoreError::Corrupt(format!("{col}: {e}"))),
    }
}

pub fn boolean(row: &SqliteRow, col: &str) -> Result<bool> {
    let v: i64 = row.try_get(col)?;
    Ok(v != 0)
}

pub fn now_text() -> String {
    Utc::now().to_rfc3339()
}

pub fn ts_text(t: DateTime<Utc>) -> String {
    t.to_rfc3339()
}

pub fn project(row: &SqliteRow) -> Result<Project> {
    Ok(Project {
        id: uuid(row, "id")?,
        name: row.try_get("name")?,
        git_common_dir: row.try_get("git_common_dir")?,
        repository_remote: row.try_get("repository_remote")?,
        linked: boolean(row, "linked")?,
        server_project_id: opt_uuid(row, "server_project_id")?,
        created_at: ts(row, "created_at")?,
        updated_at: ts(row, "updated_at")?,
        deleted_at: opt_ts(row, "deleted_at")?,
    })
}

pub fn task(row: &SqliteRow) -> Result<Task> {
    Ok(Task {
        id: uuid(row, "id")?,
        project_id: uuid(row, "project_id")?,
        title: row.try_get("title")?,
        goal: row.try_get("goal")?,
        acceptance_criteria: json_field(row, "acceptance_criteria")?,
        status: enum_val(row, "status")?,
        created_at: ts(row, "created_at")?,
        updated_at: ts(row, "updated_at")?,
        deleted_at: opt_ts(row, "deleted_at")?,
    })
}

pub fn session(row: &SqliteRow) -> Result<Session> {
    Ok(Session {
        id: uuid(row, "id")?,
        project_id: uuid(row, "project_id")?,
        task_id: opt_uuid(row, "task_id")?,
        user_id: uuid(row, "user_id")?,
        agent: row.try_get("agent")?,
        branch: row.try_get("branch")?,
        commit_sha: row.try_get("commit_sha")?,
        worktree_path: row.try_get("worktree_path")?,
        agent_session_key: row.try_get("agent_session_key")?,
        previous_session_id: opt_uuid(row, "previous_session_id")?,
        status: enum_val(row, "status")?,
        started_at: ts(row, "started_at")?,
        ended_at: opt_ts(row, "ended_at")?,
        last_event_at: ts(row, "last_event_at")?,
        last_turn_ended_at: opt_ts(row, "last_turn_ended_at")?,
        daemon_run_id: uuid(row, "daemon_run_id")?,
        end_reason: row.try_get("end_reason")?,
        handoff_pending: row.try_get::<i64, _>("handoff_pending").unwrap_or(0) != 0,
        handoff_attempts: row.try_get("handoff_attempts").unwrap_or(0),
        handoff_error: row.try_get("handoff_error").unwrap_or(None),
        deleted_at: opt_ts(row, "deleted_at")?,
    })
}

pub fn observation(row: &SqliteRow) -> Result<Observation> {
    Ok(Observation {
        id: uuid(row, "id")?,
        session_id: uuid(row, "session_id")?,
        kind: enum_val(row, "type")?,
        occurred_at: ts(row, "occurred_at")?,
        branch: row.try_get("branch")?,
        commit_sha: row.try_get("commit_sha")?,
        path: row.try_get("path")?,
        command: row.try_get("command")?,
        exit_code: row.try_get("exit_code")?,
        outcome: row.try_get("outcome")?,
        summary: row.try_get("summary")?,
        details: opt_json(row, "details")?,
        payload_bytes: row.try_get("payload_bytes")?,
        truncated: boolean(row, "truncated")?,
        deleted_at: opt_ts(row, "deleted_at")?,
    })
}

/// Memory without its evidence; the caller attaches evidence separately so a
/// list query does not fan out one round-trip per row.
pub fn memory_bare(row: &SqliteRow) -> Result<Memory> {
    Ok(Memory {
        id: uuid(row, "id")?,
        project_id: uuid(row, "project_id")?,
        kind: enum_val(row, "type")?,
        scope: enum_val(row, "scope")?,
        scope_key: row.try_get("scope_key")?,
        content: row.try_get("content")?,
        state: enum_val(row, "state")?,
        superseded_by_id: opt_uuid(row, "superseded_by_id")?,
        origin_session_id: uuid(row, "origin_session_id")?,
        local_only: boolean(row, "local_only")?,
        evidence: Vec::new(),
        created_at: ts(row, "created_at")?,
        updated_at: ts(row, "updated_at")?,
        deleted_at: opt_ts(row, "deleted_at")?,
    })
}

pub fn handoff(row: &SqliteRow) -> Result<Handoff> {
    Ok(Handoff {
        id: uuid(row, "id")?,
        session_id: uuid(row, "session_id")?,
        trigger: enum_val(row, "trigger")?,
        goal: row.try_get("goal")?,
        progress: row.try_get("progress")?,
        completed_work: json_field(row, "completed_work")?,
        remaining_work: json_field(row, "remaining_work")?,
        changed_files: json_field(row, "changed_files")?,
        decisions: json_field(row, "decisions")?,
        failures: json_field(row, "failures")?,
        tests_executed: json_field(row, "tests_executed")?,
        repository_state: json_field(row, "repository_state")?,
        next_step: row.try_get("next_step")?,
        agent_note: row.try_get("agent_note")?,
        evidence: json_field(row, "evidence")?,
        created_at: ts(row, "created_at")?,
        deleted_at: opt_ts(row, "deleted_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn ts_text_roundtrips_through_parse_ts() {
        let t = Utc.with_ymd_and_hms(2024, 6, 1, 12, 30, 45).unwrap();
        let text = ts_text(t);
        let back = parse_ts(&text).expect("parse");
        assert_eq!(back, t);
    }

    #[test]
    fn parse_ts_rejects_garbage() {
        assert!(parse_ts("not-a-timestamp").is_none());
        assert!(parse_ts("").is_none());
    }

    #[test]
    fn now_text_is_rfc3339_utc() {
        let text = now_text();
        assert!(parse_ts(&text).is_some(), "now_text must round-trip");
        assert!(text.ends_with("+00:00"), "expected UTC offset, got {text}");
    }

    #[tokio::test]
    async fn opt_uuid_column_treats_empty_string_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::Store::open(&dir.path().join("r.sqlite3"))
            .await
            .unwrap();
        sqlx::query("CREATE TABLE probe (v TEXT)")
            .execute(store.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO probe VALUES ('')")
            .execute(store.pool())
            .await
            .unwrap();
        let row: SqliteRow = sqlx::query("SELECT v AS v FROM probe")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert!(
            opt_uuid(&row, "v").unwrap().is_none(),
            "empty string is not a uuid"
        );
        store.close().await;
    }
}
