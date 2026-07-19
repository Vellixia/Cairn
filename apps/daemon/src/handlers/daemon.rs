//! T029: real daemon status.

use cairn_domain::SessionState;
use cairn_protocol::DaemonStatusResult;
use cairn_storage_local::sessions;

use super::HandlerResult;
use crate::state::AppState;

pub async fn status(state: &AppState) -> HandlerResult<DaemonStatusResult> {
    let db_healthy = state.inner.corrupted.is_none()
        && sqlx::query("SELECT 1").execute(state.pool()).await.is_ok();
    let active_sessions = if db_healthy {
        sessions::count_by_state(state.pool(), SessionState::Active)
            .await
            .unwrap_or(0)
    } else {
        0
    };
    Ok(DaemonStatusResult {
        version: env!("CARGO_PKG_VERSION").to_string(),
        pid: std::process::id(),
        uptime_seconds: state.inner.started.elapsed().as_secs(),
        db_path: state.inner.config.db_path().to_string_lossy().to_string(),
        db_healthy,
        watched_repositories: state.watched_count(),
        active_sessions,
    })
}
