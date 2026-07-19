//! Shared integration-test harness: run the daemon in-process on an isolated
//! endpoint + data dir, and a minimal JSON-lines IPC client.
//!
//! Included by many test binaries; not every binary uses every helper.
#![allow(dead_code)]

use std::path::PathBuf;

use cairn_daemon::DaemonConfig;
use cairn_protocol::{ErrorBody, Request, Response};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub struct TestDaemon {
    pub dir: tempfile::TempDir,
    pub config: DaemonConfig,
    shutdown: tokio::sync::watch::Sender<bool>,
    handle: Option<tokio::task::JoinHandle<Result<(), String>>>,
}

pub fn test_config(dir: &tempfile::TempDir) -> DaemonConfig {
    let unique = uuid::Uuid::new_v4().simple().to_string();
    DaemonConfig {
        data_dir: dir.path().join("data"),
        socket_path: dir.path().join("daemon.sock"),
        pipe_name: format!("cairn-test-{unique}"),
        sweep_interval_ms: 100,
        debounce_quiescence_ms: 150,
        debounce_deadline_ms: 1000,
        foreground: false,
        session: cairn_session::SessionConfig {
            initial_lease_secs: 900,
            heartbeat_ttl_secs: 90,
            grace_secs: 900,
        },
        watcher_test_controls: Some(std::sync::Arc::new(
            cairn_daemon::watch::WatcherTestControls::default(),
        )),
    }
}

impl TestDaemon {
    pub async fn start() -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let config = test_config(&dir);
        Self::start_with(dir, config).await
    }

    pub async fn start_with(dir: tempfile::TempDir, config: DaemonConfig) -> Self {
        std::fs::create_dir_all(&config.data_dir).expect("data dir");
        let (tx, rx) = tokio::sync::watch::channel(false);
        let cfg = config.clone();
        let handle = tokio::spawn(async move {
            cairn_daemon::run(cfg, rx)
                .await
                .map_err(|error| format!("{error:#}"))
        });
        let mut daemon = Self {
            dir,
            config,
            shutdown: tx,
            handle: Some(handle),
        };
        daemon.wait_ready().await;
        daemon
    }

    async fn wait_ready(&mut self) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if self
                .handle
                .as_ref()
                .is_some_and(|handle| handle.is_finished())
            {
                let result = self.handle.take().expect("finished daemon handle").await;
                match result {
                    Ok(Ok(())) => panic!("daemon exited before becoming ready"),
                    Ok(Err(error)) => panic!("daemon failed before becoming ready: {error}"),
                    Err(error) => panic!("daemon task failed before becoming ready: {error}"),
                }
            }
            if self.try_client().await.is_some() {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("daemon did not become ready within 15s");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    pub async fn stop(mut self) -> (tempfile::TempDir, DaemonConfig) {
        let _ = self.shutdown.send(true);
        // Nudge the accept loop (it wakes on connect) then wait for exit.
        let _ = self.try_client().await;
        if let Some(handle) = self.handle.take() {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        }
        (self.dir, self.config)
    }

    #[cfg(unix)]
    async fn try_client(&self) -> Option<Ipc> {
        tokio::net::UnixStream::connect(&self.config.socket_path)
            .await
            .ok()
            .map(|s| Ipc { stream: s })
    }

    #[cfg(windows)]
    async fn try_client(&self) -> Option<Ipc> {
        let addr = format!(r"\\.\pipe\{}", self.config.pipe_name);
        tokio::net::windows::named_pipe::ClientOptions::new()
            .open(&addr)
            .ok()
            .map(|s| Ipc { stream: s })
    }

    pub async fn client(&self) -> Ipc {
        // Windows named pipes briefly report busy between a client connect
        // and the server creating the next instance: retry.
        for _ in 0..50 {
            if let Some(c) = self.try_client().await {
                return c;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("daemon not reachable after retries");
    }

    /// Convenience one-shot call.
    pub async fn call<P: Serialize>(
        &self,
        method: &str,
        params: &P,
    ) -> Result<serde_json::Value, ErrorBody> {
        self.client().await.call(method, params).await
    }

    pub fn db_path(&self) -> PathBuf {
        self.config.db_path()
    }

    pub fn watcher_controls(&self) -> std::sync::Arc<cairn_daemon::watch::WatcherTestControls> {
        self.config
            .watcher_test_controls
            .as_ref()
            .expect("test watcher controls")
            .clone()
    }
}

pub struct Ipc {
    #[cfg(unix)]
    stream: tokio::net::UnixStream,
    #[cfg(windows)]
    stream: tokio::net::windows::named_pipe::NamedPipeClient,
}

impl Ipc {
    #[cfg(unix)]
    pub async fn connect_unix(path: &std::path::Path) -> Option<Self> {
        tokio::net::UnixStream::connect(path)
            .await
            .ok()
            .map(|s| Ipc { stream: s })
    }

    #[cfg(windows)]
    pub async fn connect_pipe(name: &str) -> Option<Self> {
        let addr = format!(r"\\.\pipe\{name}");
        tokio::net::windows::named_pipe::ClientOptions::new()
            .open(&addr)
            .ok()
            .map(|s| Ipc { stream: s })
    }

    pub async fn call<P: Serialize>(
        &mut self,
        method: &str,
        params: &P,
    ) -> Result<serde_json::Value, ErrorBody> {
        let req = Request {
            id: uuid::Uuid::new_v4().to_string(),
            method: method.to_string(),
            params: serde_json::to_value(params).expect("serializable"),
        };
        let mut line = serde_json::to_string(&req).expect("serializable");
        line.push('\n');
        self.stream
            .write_all(line.as_bytes())
            .await
            .expect("ipc write");
        let mut reader = BufReader::new(&mut self.stream);
        let mut buf = String::new();
        reader.read_line(&mut buf).await.expect("ipc read");
        let resp: Response = serde_json::from_str(&buf).expect("valid response json");
        match (resp.result, resp.error) {
            (Some(v), None) => Ok(v),
            (_, Some(e)) => Err(e),
            _ => panic!("response had neither result nor error"),
        }
    }
}
