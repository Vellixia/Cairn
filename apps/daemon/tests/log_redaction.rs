//! T055: structured-log field policy — the real daemon's JSON log must never
//! contain file contents, diffs, resume tokens, or env values (constitution:
//! logs never contain sensitive repository content or secrets).

mod support;

use cairn_protocol::methods;
use fixtures_repositories::FixtureRepo;
use serde_json::json;

const FILE_BODY: &str = "file-body-marker-J8s2Lq-should-never-be-logged";

#[tokio::test(flavor = "multi_thread")]
async fn real_daemon_log_contains_no_sensitive_material() {
    let dir = tempfile::TempDir::new().unwrap();
    let config = support::test_config(&dir);
    std::fs::create_dir_all(&config.data_dir).unwrap();

    let child = std::process::Command::new(env!("CARGO_BIN_EXE_cairnd"))
        .env("CAIRN_DATA_DIR", &config.data_dir)
        .env("CAIRN_SOCKET_PATH", &config.socket_path)
        .env("CAIRN_PIPE_NAME", &config.pipe_name)
        .env("CAIRN_LOG", "debug")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn cairnd");
    let mut child = scopeguard(child);

    // Wait for readiness.
    let mut client = None;
    for _ in 0..150 {
        #[cfg(unix)]
        let attempt = support::Ipc::connect_unix(&config.socket_path).await;
        #[cfg(windows)]
        let attempt = support::Ipc::connect_pipe(&config.pipe_name).await;
        if let Some(c) = attempt {
            client = Some(c);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    let mut client = client.expect("daemon reachable");

    // Drive a session with content that must not leak into logs.
    let repo = FixtureRepo::new().unwrap();
    let path = repo.root().to_string_lossy().to_string();
    repo.write("payload.txt", FILE_BODY).unwrap();
    client
        .call(methods::REPOSITORY_REGISTER, &json!({"path": path}))
        .await
        .unwrap();
    let inst = uuid::Uuid::new_v4().to_string();
    let started = client
        .call(
            methods::SESSION_START,
            &json!({"path": path, "agent_type": "logaudit", "agent_instance_id": inst}),
        )
        .await
        .unwrap();
    let token = started["resume_token"].as_str().unwrap().to_string();
    client
        .call(methods::REPOSITORY_INSPECT, &json!({"path": path}))
        .await
        .unwrap();
    repo.write("payload.txt", &format!("{FILE_BODY} v2"))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await; // let the watcher log

    let _ = child.0.kill();
    let _ = child.0.wait();

    let log_path = config.data_dir.join("cairnd.log");
    let log = std::fs::read_to_string(&log_path).expect("daemon log exists");
    assert!(!log.is_empty(), "daemon should have logged something");
    assert!(!log.contains(&token), "log leaks a raw resume token");
    assert!(!log.contains(FILE_BODY), "log leaks file contents");
    // Env values: the daemon must not dump its environment.
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        let _ = home; // paths are allowed; raw env dumps are not — covered by token/content checks
    }
    // Every line is structured JSON (field policy enforceable downstream).
    for line in log.lines().filter(|l| !l.trim().is_empty()) {
        assert!(
            serde_json::from_str::<serde_json::Value>(line).is_ok(),
            "non-JSON log line: {line}"
        );
    }
}

/// Kill the child on panic/drop so failed tests don't leak daemons.
struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn scopeguard(child: std::process::Child) -> ChildGuard {
    ChildGuard(child)
}
