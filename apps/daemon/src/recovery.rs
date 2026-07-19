//! T047/T048: boot recovery and the grace/staleness sweeper.

use anyhow::Result;
use tokio::sync::watch;

use crate::state::AppState;

/// Boot: every previously active session → recovering, preserving an already
/// set `recovering_since` (analysis A2); rebuild the watcher set. Corrupted
/// state short-circuits — no fabricated recovery (FR-033).
pub async fn on_boot(state: &AppState) -> Result<()> {
    if state.inner.corrupted.is_some() {
        tracing::error!("local state corrupted; serving STATE_CORRUPTED only");
        return Ok(());
    }
    let moved = state.inner.sessions.mark_all_active_recovering().await?;
    if moved > 0 {
        tracing::info!(sessions = moved, "moved active sessions to recovering");
    }

    // Re-watch every worktree that still has a live session.
    let live = [
        cairn_domain::SessionState::Active,
        cairn_domain::SessionState::Recovering,
    ];
    let rows = cairn_storage_local::sessions::list(state.pool(), None, Some(&live)).await?;
    for row in rows {
        if let Some(wt) =
            cairn_storage_local::worktrees::get_by_id(state.pool(), &row.worktree_id).await?
        {
            crate::watch::request_ready(
                state,
                row.repository_id.clone(),
                wt.id.clone(),
                wt.path.clone().into(),
            )
            .await
            .map_err(|failure| {
                anyhow::anyhow!(
                    "watcher recovery failed at {}: {}",
                    failure.stage.as_str(),
                    failure.message
                )
            })?;
        }
    }
    Ok(())
}

/// Periodic sweep: grace-deadline expiry → interrupted; active sessions with
/// expired lease AND verifiably dead process → interrupted (T048).
pub async fn sweeper_loop(state: AppState, mut shutdown: watch::Receiver<bool>) {
    if state.inner.corrupted.is_some() {
        return;
    }
    let interval = std::time::Duration::from_millis(state.inner.config.sweep_interval_ms);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tokio::time::sleep(interval) => {
                match state.inner.sessions.sweep().await {
                    Ok(n) if n > 0 => tracing::info!(interrupted = n, "sweeper pass"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "sweeper pass failed"),
                }
            }
        }
    }
}
