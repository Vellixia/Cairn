//! Cairn daemon library: IPC server, handlers, watcher, recovery.
//! The binary (`cairnd`) is a thin wrapper; tests drive this library
//! in-process and via the real binary.

pub mod config;
pub mod handlers;
pub mod ipc;
pub mod logging;
pub mod recovery;
pub mod router;
pub mod state;
pub mod watch;

pub use config::DaemonConfig;
pub use state::AppState;

use anyhow::Result;

/// Run the daemon until shutdown is signalled. Returns after cleanup.
pub async fn run(config: DaemonConfig, shutdown: tokio::sync::watch::Receiver<bool>) -> Result<()> {
    let state = AppState::init(config).await?;
    let watcher = tokio::spawn(watch::manager_loop(state.clone(), shutdown.clone()));
    if let Err(error) = recovery::on_boot(&state).await {
        watcher.abort();
        return Err(error);
    }
    let sweeper = tokio::spawn(recovery::sweeper_loop(state.clone(), shutdown.clone()));
    ipc::serve(state, shutdown).await?;
    sweeper.abort();
    watcher.abort();
    Ok(())
}
