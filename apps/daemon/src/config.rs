//! Daemon configuration: endpoint naming, data dir, timing knobs.
//! Env overrides keep tests isolated (research R11).

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub data_dir: PathBuf,
    /// Unix socket path (unix) — CAIRN_SOCKET_PATH override.
    pub socket_path: PathBuf,
    /// Named pipe name (windows) — CAIRN_PIPE_NAME override.
    pub pipe_name: String,
    /// Sweeper interval in milliseconds.
    pub sweep_interval_ms: u64,
    /// Watcher quiescence window (ms) and hard deadline (ms).
    pub debounce_quiescence_ms: u64,
    pub debounce_deadline_ms: u64,
    /// Foreground mode also logs human-readable to stderr.
    pub foreground: bool,
    /// Session lifecycle timing (lease/TTL/grace).
    pub session: cairn_session::SessionConfig,
    /// Deterministic watcher coordination used only by integration tests.
    /// Production configuration always leaves this unset.
    pub watcher_test_controls: Option<std::sync::Arc<crate::watch::WatcherTestControls>>,
}

impl DaemonConfig {
    pub fn from_env() -> Self {
        let data_dir = cairn_storage_local::data_dir();
        let socket_path = std::env::var("CAIRN_SOCKET_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_socket_path(&data_dir));
        let pipe_name = std::env::var("CAIRN_PIPE_NAME")
            .unwrap_or_else(|_| format!("cairn-{}-daemon", whoami::username()));
        fn env_u64(key: &str, default: u64) -> u64 {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        Self {
            data_dir,
            socket_path,
            pipe_name,
            sweep_interval_ms: env_u64("CAIRN_SWEEP_INTERVAL_MS", 10_000),
            debounce_quiescence_ms: env_u64("CAIRN_DEBOUNCE_QUIESCENCE_MS", 500),
            debounce_deadline_ms: env_u64("CAIRN_DEBOUNCE_DEADLINE_MS", 3_000),
            foreground: std::env::var("CAIRN_FOREGROUND").is_ok(),
            session: cairn_session::SessionConfig::from_env(),
            watcher_test_controls: None,
        }
    }

    /// The daemon-local database file.
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("cairn.db")
    }
}

fn default_socket_path(data_dir: &std::path::Path) -> PathBuf {
    #[cfg(unix)]
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime).join("cairn").join("daemon.sock");
    }
    data_dir.join("daemon.sock")
}
