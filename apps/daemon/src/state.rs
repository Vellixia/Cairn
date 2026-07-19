//! Shared daemon state wired at startup.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use cairn_session::{SessionConfig, SessionService};
use cairn_storage_local::WorktreeWriters;
use sqlx::SqlitePool;
use tokio::sync::mpsc;

use crate::config::DaemonConfig;
use crate::watch::WatchCommand;

#[derive(Clone)]
pub struct AppState {
    pub inner: Arc<Inner>,
}

pub struct Inner {
    pub config: DaemonConfig,
    pub pool: SqlitePool,
    pub writers: Arc<WorktreeWriters>,
    pub sessions: SessionService,
    pub started: Instant,
    pub watched: AtomicU64,
    pub watch_tx: mpsc::UnboundedSender<WatchCommand>,
    pub watch_rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<WatchCommand>>>,
    /// Set when the DB failed to open: every request answers STATE_CORRUPTED
    /// instead of fabricating state (FR-033).
    pub corrupted: Option<String>,
}

impl AppState {
    pub async fn init(config: DaemonConfig) -> Result<Self> {
        let (watch_tx, watch_rx) = mpsc::unbounded_channel();
        let (pool, corrupted) = match cairn_storage_local::open_pool_at(&config.db_path()).await {
            Ok(pool) => (Some(pool), None),
            Err(e) if e.is_corruption() => (None, Some(e.to_string())),
            Err(e) => return Err(e.into()),
        };
        // Even when corrupted we still serve IPC so clients get an honest
        // STATE_CORRUPTED answer; a dummy in-memory pool backs that mode.
        let pool = match pool {
            Some(p) => p,
            None => {
                sqlx::sqlite::SqlitePoolOptions::new()
                    .connect("sqlite::memory:")
                    .await?
            }
        };
        let writers = Arc::new(WorktreeWriters::new());
        let session_config: SessionConfig = config.session;
        let sessions = SessionService::new(pool.clone(), writers.clone(), session_config);
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                pool,
                writers,
                sessions,
                started: Instant::now(),
                watched: AtomicU64::new(0),
                watch_tx,
                watch_rx: std::sync::Mutex::new(Some(watch_rx)),
                corrupted,
            }),
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.inner.pool
    }

    pub fn watched_count(&self) -> u64 {
        self.inner.watched.load(Ordering::Relaxed)
    }
}
