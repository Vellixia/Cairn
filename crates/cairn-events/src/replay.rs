//! T017 support: rebuild projections by replaying events in seq order and
//! compare against live projections (constitution: event replay).

use std::collections::BTreeMap;

use cairn_storage_local::{events, StorageError};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// The replay-reconstructable portion of a session projection. Fields that
/// are not event-sourced (resume token hash, lease clock) are excluded by
/// design: they are authentication material, not history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedSession {
    pub session_id: String,
    pub state: String,
    pub current_snapshot_id: Option<String>,
    pub start_snapshot_id: Option<String>,
    pub ended: bool,
}

/// Replay all events (seq order) into projected session states.
pub async fn rebuild_sessions(
    pool: &SqlitePool,
) -> Result<BTreeMap<String, ProjectedSession>, StorageError> {
    let mut out: BTreeMap<String, ProjectedSession> = BTreeMap::new();
    let mut after = None;
    loop {
        let page = events::list_events(pool, None, None, None, after, 1000).await?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|e| e.seq);
        for ev in &page {
            let Some(session_id) = ev.session_id.clone() else {
                continue;
            };
            let payload: serde_json::Value =
                serde_json::from_str(&ev.payload).unwrap_or(serde_json::Value::Null);
            match ev.event_type.as_str() {
                crate::catalog::SESSION_STARTED => {
                    let snap = ev.snapshot_id.clone();
                    out.insert(
                        session_id.clone(),
                        ProjectedSession {
                            session_id,
                            state: "active".into(),
                            current_snapshot_id: snap.clone(),
                            start_snapshot_id: snap,
                            ended: false,
                        },
                    );
                }
                crate::catalog::SESSION_STOPPED => {
                    if let Some(s) = out.get_mut(&session_id) {
                        s.state = "stopped".into();
                        s.ended = true;
                        if let Some(fs) = payload.get("final_snapshot_id").and_then(|v| v.as_str())
                        {
                            s.current_snapshot_id = Some(fs.to_string());
                        }
                    }
                }
                crate::catalog::SESSION_INTERRUPTED => {
                    if let Some(s) = out.get_mut(&session_id) {
                        s.state = "interrupted".into();
                        s.ended = true;
                    }
                }
                crate::catalog::SESSION_RECOVERED => {
                    if let Some(s) = out.get_mut(&session_id) {
                        s.state = "active".into();
                        if let Some(fs) = ev.snapshot_id.clone() {
                            s.current_snapshot_id = Some(fs);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // repository.state_changed updates every live session in the worktree.
    // Second pass keeps ordering semantics simple: interleave via seq again.
    let mut after = None;
    let mut worktree_sessions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    loop {
        let page = events::list_events(pool, None, None, None, after, 1000).await?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|e| e.seq);
        for ev in &page {
            match ev.event_type.as_str() {
                crate::catalog::SESSION_STARTED => {
                    if let (Some(w), Some(s)) = (ev.worktree_id.clone(), ev.session_id.clone()) {
                        worktree_sessions.entry(w).or_default().push(s);
                    }
                }
                crate::catalog::REPOSITORY_STATE_CHANGED => {
                    let Some(w) = ev.worktree_id.clone() else {
                        continue;
                    };
                    let Some(to_snap) = ev.snapshot_id.clone() else {
                        continue;
                    };
                    for sid in worktree_sessions.get(&w).cloned().unwrap_or_default() {
                        if let Some(s) = out.get_mut(&sid) {
                            if !s.ended {
                                s.current_snapshot_id = Some(to_snap.clone());
                            }
                        }
                    }
                }
                crate::catalog::SESSION_STOPPED | crate::catalog::SESSION_INTERRUPTED => {
                    // ended sessions stop following state changes; `ended`
                    // flag already handled in first pass by seq order over
                    // the same total sequence.
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// Read the live sessions table into the same projected shape.
pub async fn live_sessions(
    pool: &SqlitePool,
) -> Result<BTreeMap<String, ProjectedSession>, StorageError> {
    let rows = cairn_storage_local::sessions::list(pool, None, None).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            // `recovering` is a runtime liveness state, not event-sourced:
            // replay reconstructs it as `active` (the last recorded state).
            let state = if r.state == "recovering" {
                "active".to_string()
            } else {
                r.state
            };
            let ended = state == "stopped" || state == "interrupted";
            (
                r.id.clone(),
                ProjectedSession {
                    session_id: r.id,
                    state,
                    current_snapshot_id: Some(r.current_snapshot_id),
                    start_snapshot_id: Some(r.start_snapshot_id),
                    ended,
                },
            )
        })
        .collect())
}
