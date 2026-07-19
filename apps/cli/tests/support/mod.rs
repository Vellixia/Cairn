//! CLI test harness: in-process daemon on an isolated endpoint; the real
//! `cairn` binary driven via std::process with env pointing at it.
//!
//! Included by many test binaries; not every binary uses every helper.
#![allow(dead_code)]

use std::process::Output;

pub struct CliHarness {
    pub dir: tempfile::TempDir,
    pub config: cairn_daemon::DaemonConfig,
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl CliHarness {
    pub async fn start() -> Self {
        Self::start_with_session_config(cairn_session::SessionConfig {
            initial_lease_secs: 900,
            heartbeat_ttl_secs: 90,
            grace_secs: 900,
        })
        .await
    }

    pub async fn start_with_session_config(session: cairn_session::SessionConfig) -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let unique = uuid::Uuid::new_v4().simple().to_string();
        let config = cairn_daemon::DaemonConfig {
            data_dir: dir.path().join("data"),
            socket_path: dir.path().join("daemon.sock"),
            pipe_name: format!("cairn-cli-test-{unique}"),
            sweep_interval_ms: 200,
            debounce_quiescence_ms: 150,
            debounce_deadline_ms: 1000,
            foreground: false,
            session,
            watcher_test_controls: Some(std::sync::Arc::new(
                cairn_daemon::watch::WatcherTestControls::default(),
            )),
        };
        std::fs::create_dir_all(&config.data_dir).expect("data dir");
        let (tx, rx) = tokio::sync::watch::channel(false);
        let cfg = config.clone();
        tokio::spawn(async move {
            let _ = cairn_daemon::run(cfg, rx).await;
        });
        let harness = Self {
            dir,
            config,
            shutdown: tx,
        };
        harness.wait_ready().await;
        harness
    }

    /// Restart the daemon on the same data dir (recovery flows).
    pub async fn restart(&mut self) {
        let _ = self.shutdown.send(true);
        // Nudge + settle.
        let _ = self.cairn(&["daemon", "status", "--json"], None).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let (tx, rx) = tokio::sync::watch::channel(false);
        self.shutdown = tx;
        let cfg = self.config.clone();
        tokio::spawn(async move {
            let _ = cairn_daemon::run(cfg, rx).await;
        });
        self.wait_ready().await;
    }

    /// Run `cairn` with data written to stdin and extra env vars.
    pub async fn cairn_full(
        &self,
        args: &[&str],
        cwd: Option<&std::path::Path>,
        stdin_data: Option<&str>,
        envs: &[(&str, &str)],
    ) -> Output {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_cairn"));
        cmd.args(args)
            .env("CAIRN_SOCKET_PATH", &self.config.socket_path)
            .env("CAIRN_PIPE_NAME", &self.config.pipe_name)
            .env("CAIRN_DATA_DIR", &self.config.data_dir)
            .env("CAIRN_NO_SPAWN", "1") // in-process daemon only — never fork a real one
            .env_remove("CAIRN_AGENT_INSTANCE")
            .env_remove("CAIRN_RESUME_TOKEN");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let stdin_owned = stdin_data.map(str::to_string);
        cmd.stdin(if stdin_owned.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        tokio::task::spawn_blocking(move || {
            let mut c = cmd;
            let mut child = c.spawn().expect("spawn cairn");
            if let Some(data) = stdin_owned {
                use std::io::Write;
                let mut si = child.stdin.take().expect("piped stdin");
                si.write_all(data.as_bytes()).expect("write stdin");
                drop(si);
            }
            child.wait_with_output().expect("wait cairn")
        })
        .await
        .expect("join")
    }

    async fn wait_ready(&self) {
        for _ in 0..100 {
            let out = self.cairn(&["daemon", "status", "--json"], None).await;
            if out.status.code() == Some(0) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        panic!("daemon not ready for CLI tests");
    }

    /// Run the real `cairn` binary with the daemon endpoint injected.
    pub async fn cairn(&self, args: &[&str], cwd: Option<&std::path::Path>) -> Output {
        self.cairn_full(args, cwd, None, &[]).await
    }

    /// Run and parse the stdout envelope, asserting the schema marker.
    pub async fn cairn_json(
        &self,
        args: &[&str],
        cwd: Option<&std::path::Path>,
    ) -> (serde_json::Value, i32) {
        let out = self.cairn(args, cwd).await;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let envelope: serde_json::Value = serde_json::from_str(stdout.trim())
            .unwrap_or_else(|e| panic!("stdout is not one JSON envelope: {e}\n{stdout}"));
        assert_eq!(envelope["schema"], serde_json::json!("cairn.cli.v1"));
        (envelope, out.status.code().unwrap_or(-1))
    }

    pub fn stop(self) {
        let _ = self.shutdown.send(true);
    }
}
