//! Session state machine (data-model.md).
//!
//! Legal transitions only:
//!   active→stopped (stop), active→recovering (daemon restart),
//!   active→interrupted (stale takeover or watcher-start failure), recovering→active (authenticated
//!   reattach), recovering→stopped (authenticated stop),
//!   recovering→interrupted (grace-deadline expiry only).
//! A failed reattachment causes NO transition (analysis I3). Terminal states
//! never transition.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Active,
    Recovering,
    Stopped,
    Interrupted,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Active => "active",
            SessionState::Recovering => "recovering",
            SessionState::Stopped => "stopped",
            SessionState::Interrupted => "interrupted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "active" => SessionState::Active,
            "recovering" => SessionState::Recovering,
            "stopped" => SessionState::Stopped,
            "interrupted" => SessionState::Interrupted,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, SessionState::Stopped | SessionState::Interrupted)
    }

    pub fn is_live(self) -> bool {
        matches!(self, SessionState::Active | SessionState::Recovering)
    }
}

/// Why a transition is being requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransitionReason {
    /// Explicit stop of an active session.
    Stop,
    /// Daemon restart moves active sessions to recovering.
    DaemonRestart,
    /// Colliding start found the session stale (FR-034).
    StaleTakeover,
    /// Session start could not establish authoritative live tracking.
    WatcherStartFailed,
    /// Grace deadline (`recovering_since + grace`) expired.
    GraceExpired,
    /// Authenticated reattachment (valid instance id + resume token).
    Reattach,
    /// Authenticated stop of a recovering session.
    AuthenticatedStop,
}

/// Liveness reason codes recorded with staleness determinations (analysis A1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LivenessReason {
    HeartbeatExpired,
    ProcessDead,
    ReattachTimeout,
    ProcessUnknown,
}

impl LivenessReason {
    pub fn as_str(self) -> &'static str {
        match self {
            LivenessReason::HeartbeatExpired => "heartbeat_expired",
            LivenessReason::ProcessDead => "process_dead",
            LivenessReason::ReattachTimeout => "reattach_timeout",
            LivenessReason::ProcessUnknown => "process_unknown",
        }
    }
}

/// Why a session was interrupted (event payload field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InterruptReason {
    StaleTakeover,
    GraceExpired,
    WatcherStartFailed,
}

impl InterruptReason {
    pub fn as_str(self) -> &'static str {
        match self {
            InterruptReason::StaleTakeover => "stale_takeover",
            InterruptReason::GraceExpired => "grace_expired",
            InterruptReason::WatcherStartFailed => "watcher_start_failed",
        }
    }
}

/// Stage of the watcher-readiness protocol that failed (FR-038).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WatcherStartStage {
    Install,
    Reconcile,
}

impl WatcherStartStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Reconcile => "reconcile",
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("illegal session transition {from:?} -> {to:?} (reason {reason:?})")]
pub struct IllegalTransition {
    pub from: SessionState,
    pub to: SessionState,
    pub reason: TransitionReason,
}

/// Typed transition function: the single authority on session-state legality.
pub fn transition(
    from: SessionState,
    to: SessionState,
    reason: TransitionReason,
) -> Result<SessionState, IllegalTransition> {
    use SessionState::*;
    use TransitionReason::*;
    let legal = matches!(
        (from, to, reason),
        (Active, Stopped, Stop)
            | (Active, Recovering, DaemonRestart)
            | (Active, Interrupted, StaleTakeover)
            | (Active, Interrupted, WatcherStartFailed)
            | (Recovering, Active, Reattach)
            | (Recovering, Stopped, AuthenticatedStop)
            | (Recovering, Interrupted, GraceExpired)
    );
    if legal {
        Ok(to)
    } else {
        Err(IllegalTransition { from, to, reason })
    }
}
