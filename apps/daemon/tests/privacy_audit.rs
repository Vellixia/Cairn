//! T054: privacy audit (SC-006, FR-026…028) — after snapshotting a repository
//! containing ignored secrets, the full DB dump, WAL, and log files contain
//! zero secret bytes, zero ignored-file contents, and zero raw resume tokens.

mod support;

use cairn_protocol::methods;
use fixtures_repositories::FixtureRepo;
use serde_json::json;
use support::TestDaemon;

const SECRET: &str = "TOPSECRET-a8f3k29x-DO-NOT-PERSIST";
const IGNORED_BODY: &str = "ignored-file-body-Zq7Yw2-unique-marker";

fn assert_clean(haystack: &[u8], what: &str, token: &str) {
    let find = |needle: &str| {
        haystack
            .windows(needle.len())
            .any(|w| w == needle.as_bytes())
    };
    assert!(!find(SECRET), "{what} contains the secret value");
    assert!(!find(IGNORED_BODY), "{what} contains ignored-file contents");
    assert!(!find(token), "{what} contains a raw resume token");
}

#[tokio::test(flavor = "multi_thread")]
async fn persisted_state_and_logs_contain_no_secrets_or_tokens() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    let path = repo.root().to_string_lossy().to_string();

    // Secrets: an ignored .env plus an ignored file with a known body.
    repo.ignored_secret(SECRET).unwrap();
    repo.write("ignored-dir/data.txt", IGNORED_BODY).unwrap();
    let mut gitignore = std::fs::read_to_string(repo.root().join(".gitignore")).unwrap();
    gitignore.push_str("ignored-dir/\n");
    std::fs::write(repo.root().join(".gitignore"), gitignore).unwrap();

    daemon
        .call(methods::REPOSITORY_REGISTER, &json!({"path": path}))
        .await
        .unwrap();
    let inst = uuid::Uuid::new_v4().to_string();
    let started = daemon
        .call(
            methods::SESSION_START,
            &json!({"path": path, "agent_type": "audit", "agent_instance_id": inst}),
        )
        .await
        .unwrap();
    let token = started["resume_token"].as_str().unwrap().to_string();
    let sid = started["session"]["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Exercise everything that persists: inspection, snapshots, tracking, stop.
    daemon
        .call(methods::REPOSITORY_INSPECT, &json!({"path": path}))
        .await
        .unwrap();
    repo.write("tracked.txt", "ordinary tracked content\n")
        .unwrap();
    daemon
        .call(methods::SNAPSHOT_CREATE, &json!({"path": path}))
        .await
        .unwrap();
    daemon
        .call(
            methods::SESSION_STOP,
            &json!({"session_id": sid, "resume_token": token}),
        )
        .await
        .unwrap();

    let (dir, config) = daemon.stop().await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Audit every persisted artifact byte-for-byte.
    let db = std::fs::read(config.db_path()).unwrap();
    assert_clean(&db, "database file", &token);
    for sidecar in ["db-wal", "db-shm"] {
        let p = config.db_path().with_extension(sidecar);
        if p.exists() {
            assert_clean(&std::fs::read(&p).unwrap(), "database sidecar", &token);
        }
    }
    let log = config.data_dir.join("cairnd.log");
    if log.exists() {
        assert_clean(&std::fs::read(&log).unwrap(), "daemon log", &token);
    }
    drop(dir);
}
