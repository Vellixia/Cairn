//! T016: the 11-type event catalog (data-model.md) with typed payloads and
//! deterministic idempotency-key derivation (research R7).
//!
//! `session.reattach_rejected` payloads MUST never contain token values.

use cairn_domain::{EventId, Timestamp, WatcherStartStage};
use serde::{Deserialize, Serialize};
use serde_json::json;

use cairn_storage_local::NewEvent;

pub const REPOSITORY_REGISTERED: &str = "repository.registered";
pub const WORKTREE_REGISTERED: &str = "worktree.registered";
pub const SNAPSHOT_CREATED: &str = "snapshot.created";
pub const SESSION_STARTED: &str = "session.started";
pub const REPOSITORY_STATE_CHANGED: &str = "repository.state_changed";
pub const BRANCH_CHANGED: &str = "branch.changed";
pub const SESSION_STOPPED: &str = "session.stopped";
pub const SESSION_INTERRUPTED: &str = "session.interrupted";
pub const SESSION_RECOVERED: &str = "session.recovered";
pub const SESSION_REATTACH_REJECTED: &str = "session.reattach_rejected";
pub const IDENTITY_MARKER_RESTORED: &str = "identity.marker_restored";

pub const ALL_EVENT_TYPES: &[&str] = &[
    REPOSITORY_REGISTERED,
    WORKTREE_REGISTERED,
    SNAPSHOT_CREATED,
    SESSION_STARTED,
    REPOSITORY_STATE_CHANGED,
    BRANCH_CHANGED,
    SESSION_STOPPED,
    SESSION_INTERRUPTED,
    SESSION_RECOVERED,
    SESSION_REATTACH_REJECTED,
    IDENTITY_MARKER_RESTORED,
];

/// Typed builder ensuring every event carries a deterministic idempotency key
/// derived from event type + entity id + causal input (arch rule 6).
pub struct EventBuilder;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartedPayload {
    pub agent_type: String,
    pub agent_instance_id: String,
    pub start_snapshot_id: String,
    pub local_user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateChangedPayload {
    pub worktree_id: String,
    pub from_snapshot_id: Option<String>,
    pub to_snapshot_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchChangedPayload {
    pub from_branch: Option<String>,
    pub to_branch: Option<String>,
    pub from_head: Option<String>,
    pub to_head: String,
}

fn base(event_type: &str, idempotency_key: String, payload: serde_json::Value) -> NewEvent {
    NewEvent {
        id: EventId::new_v7().to_string(),
        idempotency_key,
        event_type: event_type.to_string(),
        repository_id: None,
        worktree_id: None,
        session_id: None,
        snapshot_id: None,
        payload,
        recorded_at: Timestamp::now().to_rfc3339(),
    }
}

impl EventBuilder {
    pub fn repository_registered(
        repository_id: &str,
        repo_uuid: &str,
        canonical_path: &str,
        remote: Option<(&str, &str)>,
    ) -> NewEvent {
        let mut e = base(
            REPOSITORY_REGISTERED,
            format!("{REPOSITORY_REGISTERED}:{repository_id}"),
            json!({
                "repo_uuid": repo_uuid,
                "canonical_path": canonical_path,
                "remote": remote.map(|(n, u)| json!({"name": n, "url": u})),
            }),
        );
        e.repository_id = Some(repository_id.to_string());
        e
    }

    pub fn worktree_registered(
        repository_id: &str,
        worktree_id: &str,
        worktree_uuid: &str,
        path: &str,
        is_main: bool,
    ) -> NewEvent {
        let mut e = base(
            WORKTREE_REGISTERED,
            format!("{WORKTREE_REGISTERED}:{worktree_id}"),
            json!({"worktree_uuid": worktree_uuid, "path": path, "is_main": is_main}),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e
    }

    pub fn snapshot_created(
        repository_id: &str,
        worktree_id: &str,
        snapshot_id: &str,
        snapshot_fp: &str,
        branch: Option<&str>,
        head_commit: &str,
    ) -> NewEvent {
        let mut e = base(
            SNAPSHOT_CREATED,
            format!("{SNAPSHOT_CREATED}:{worktree_id}:{snapshot_fp}"),
            json!({
                "snapshot_fp": snapshot_fp,
                "branch": branch,
                "head_commit": head_commit,
            }),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.snapshot_id = Some(snapshot_id.to_string());
        e
    }

    pub fn session_started(
        repository_id: &str,
        worktree_id: &str,
        session_id: &str,
        payload: &SessionStartedPayload,
    ) -> NewEvent {
        let mut e = base(
            SESSION_STARTED,
            format!("{SESSION_STARTED}:{session_id}"),
            serde_json::to_value(payload).expect("serializable payload"),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.session_id = Some(session_id.to_string());
        e.snapshot_id = Some(payload.start_snapshot_id.clone());
        e
    }

    pub fn repository_state_changed(
        repository_id: &str,
        worktree_id: &str,
        payload: &StateChangedPayload,
    ) -> NewEvent {
        let mut e = base(
            REPOSITORY_STATE_CHANGED,
            format!(
                "{REPOSITORY_STATE_CHANGED}:{worktree_id}:{}",
                payload.to_snapshot_id
            ),
            serde_json::to_value(payload).expect("serializable payload"),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.snapshot_id = Some(payload.to_snapshot_id.clone());
        e
    }

    pub fn branch_changed(
        repository_id: &str,
        worktree_id: &str,
        payload: &BranchChangedPayload,
    ) -> NewEvent {
        let mut e = base(
            BRANCH_CHANGED,
            format!(
                "{BRANCH_CHANGED}:{worktree_id}:{}:{}",
                payload.to_branch.as_deref().unwrap_or("DETACHED"),
                payload.to_head
            ),
            serde_json::to_value(payload).expect("serializable payload"),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e
    }

    pub fn session_stopped(
        repository_id: &str,
        worktree_id: &str,
        session_id: &str,
        final_snapshot_id: &str,
    ) -> NewEvent {
        let mut e = base(
            SESSION_STOPPED,
            format!("{SESSION_STOPPED}:{session_id}"),
            json!({"final_snapshot_id": final_snapshot_id}),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.session_id = Some(session_id.to_string());
        e.snapshot_id = Some(final_snapshot_id.to_string());
        e
    }

    pub fn session_interrupted(
        repository_id: &str,
        worktree_id: &str,
        session_id: &str,
        reason: &str,
        liveness_detail: &str,
    ) -> NewEvent {
        let mut e = base(
            SESSION_INTERRUPTED,
            format!("{SESSION_INTERRUPTED}:{session_id}"),
            json!({"reason": reason, "liveness_detail": liveness_detail}),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.session_id = Some(session_id.to_string());
        e
    }

    /// Watcher-readiness failure. The payload is deliberately bounded to the
    /// stable stage code and never includes paths, contents, environment
    /// values, or token material (FR-038).
    pub fn session_watcher_start_failed(
        repository_id: &str,
        worktree_id: &str,
        session_id: &str,
        watcher_stage: WatcherStartStage,
    ) -> NewEvent {
        let mut e = base(
            SESSION_INTERRUPTED,
            format!("{SESSION_INTERRUPTED}:{session_id}"),
            json!({
                "reason": "watcher_start_failed",
                "watcher_stage": watcher_stage.as_str(),
            }),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.session_id = Some(session_id.to_string());
        e
    }

    pub fn session_recovered(
        repository_id: &str,
        worktree_id: &str,
        session_id: &str,
        fresh_snapshot_id: &str,
    ) -> NewEvent {
        let mut e = base(
            SESSION_RECOVERED,
            // A session may recover multiple times across restarts: key on
            // the fresh snapshot to keep each recovery distinct.
            format!("{SESSION_RECOVERED}:{session_id}:{fresh_snapshot_id}"),
            json!({"fresh_snapshot_id": fresh_snapshot_id}),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.session_id = Some(session_id.to_string());
        e.snapshot_id = Some(fresh_snapshot_id.to_string());
        e
    }

    /// Audit event for rejected reattachment. NEVER include token material.
    pub fn session_reattach_rejected(
        repository_id: &str,
        worktree_id: &str,
        session_id: &str,
        presented_instance_id: &str,
        reason: &str,
        attempt_id: &str,
    ) -> NewEvent {
        let mut e = base(
            SESSION_REATTACH_REJECTED,
            // Every rejected attempt is a distinct audit record.
            format!("{SESSION_REATTACH_REJECTED}:{session_id}:{attempt_id}"),
            json!({
                "agent_instance_id_presented": presented_instance_id,
                "reason": reason,
            }),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = Some(worktree_id.to_string());
        e.session_id = Some(session_id.to_string());
        e
    }

    pub fn identity_marker_restored(
        repository_id: &str,
        worktree_id: Option<&str>,
        restored_from: &str,
    ) -> NewEvent {
        let mut e = base(
            IDENTITY_MARKER_RESTORED,
            format!(
                "{IDENTITY_MARKER_RESTORED}:{repository_id}:{}",
                worktree_id.unwrap_or("-")
            ),
            json!({"restored_from": restored_from}),
        );
        e.repository_id = Some(repository_id.to_string());
        e.worktree_id = worktree_id.map(str::to_string);
        e
    }
}
