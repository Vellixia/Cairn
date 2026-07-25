//! T056: performance suite — inspect < 2 s and snapshot < 2 s on a
//! 10,000-tracked-file repository (SC-007); quiescence→snapshot ≤ 5 s
//! (SC-003, exercised in us3_tracking with an explicit bound).
//!
//! `#[ignore]` by default; CI nightly runs `cargo test -- --ignored`.

mod support;

use cairn_protocol::methods;
use fixtures_repositories::FixtureRepo;
use serde_json::json;
use support::TestDaemon;

fn build_10k_repo() -> FixtureRepo {
    let repo = FixtureRepo::empty().unwrap();
    for d in 0..100 {
        for f in 0..100 {
            repo.write(&format!("src/mod{d}/file{f}.rs"), &format!("// {d}:{f}\n"))
                .unwrap();
        }
    }
    repo.git(&["add", "."]).unwrap();
    repo.git(&["commit", "-m", "10k files"]).unwrap();
    repo
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "perf suite — run explicitly or in CI nightly"]
async fn inspect_and_snapshot_under_two_seconds_at_10k_files() {
    let daemon = TestDaemon::start().await;
    let repo = build_10k_repo();
    let path = repo.root().to_string_lossy().to_string();
    daemon
        .call(methods::REPOSITORY_REGISTER, &json!({"path": path}))
        .await
        .unwrap();

    // A little dirt so the fingerprint pipeline hashes real content.
    repo.write("src/mod0/file0.rs", "// changed\n").unwrap();
    repo.write("scratch-untracked.txt", "u\n").unwrap();

    let t = std::time::Instant::now();
    daemon
        .call(methods::REPOSITORY_INSPECT, &json!({"path": path}))
        .await
        .unwrap();
    let inspect_elapsed = t.elapsed();

    let t = std::time::Instant::now();
    daemon
        .call(methods::SNAPSHOT_CREATE, &json!({"path": path}))
        .await
        .unwrap();
    let snapshot_elapsed = t.elapsed();

    println!(
        "SC-007 acceptance: tracked_files=10000 inspect_ms={} snapshot_ms={} inspect_limit_ms=2000 snapshot_limit_ms=2000",
        inspect_elapsed.as_millis(),
        snapshot_elapsed.as_millis(),
    );
    assert!(
        inspect_elapsed < std::time::Duration::from_secs(2),
        "SC-007 inspect bound violated: {inspect_elapsed:?}"
    );
    assert!(
        snapshot_elapsed < std::time::Duration::from_secs(2),
        "SC-007 snapshot bound violated: {snapshot_elapsed:?}"
    );
    daemon.stop().await;
}
