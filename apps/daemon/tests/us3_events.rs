//! T046: events.list contract against the T024 handler — seq ordering,
//! pagination, composable filters, state-change payload linkage.

mod support;

use cairn_protocol::methods;
use fixtures_repositories::FixtureRepo;
use serde_json::json;
use support::TestDaemon;

#[tokio::test(flavor = "multi_thread")]
async fn events_paginate_in_seq_order_with_filters() {
    let daemon = TestDaemon::start().await;

    // Two repositories to prove filter isolation.
    let repo_a = FixtureRepo::new().unwrap();
    let repo_b = FixtureRepo::new().unwrap();
    let reg_a = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": repo_a.root().to_string_lossy()}),
        )
        .await
        .unwrap();
    let reg_b = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": repo_b.root().to_string_lossy()}),
        )
        .await
        .unwrap();
    let id_a = reg_a["repository"]["repository_id"]
        .as_str()
        .unwrap()
        .to_string();
    let id_b = reg_b["repository"]["repository_id"]
        .as_str()
        .unwrap()
        .to_string();
    let wt_a = reg_a["worktree"]["worktree_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Generate snapshot events in A.
    for i in 0..3 {
        repo_a
            .write(&format!("f{i}.txt"), &format!("{i}\n"))
            .unwrap();
        daemon
            .call(methods::SNAPSHOT_CREATE, &json!({"repository_id": id_a}))
            .await
            .unwrap();
    }

    // Unfiltered: strictly increasing seq.
    let all = daemon
        .call(methods::EVENTS_LIST, &json!({"limit": 1000}))
        .await
        .unwrap();
    let seqs: Vec<i64> = all["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_i64().unwrap())
        .collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "seq must be strictly increasing"
    );

    // Repository filter excludes the other repo entirely.
    let only_a = daemon
        .call(
            methods::EVENTS_LIST,
            &json!({"repository_id": id_a, "limit": 1000}),
        )
        .await
        .unwrap();
    assert!(only_a["events"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["repository_id"].as_str() == Some(id_a.as_str())));
    let only_b = daemon
        .call(
            methods::EVENTS_LIST,
            &json!({"repository_id": id_b, "limit": 1000}),
        )
        .await
        .unwrap();
    assert!(!only_b["events"].as_array().unwrap().is_empty());

    // Worktree filter composes.
    let wt_filtered = daemon
        .call(
            methods::EVENTS_LIST,
            &json!({"repository_id": id_a, "worktree_id": wt_a, "limit": 1000}),
        )
        .await
        .unwrap();
    assert!(wt_filtered["events"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["worktree_id"].as_str() == Some(wt_a.as_str())));

    // Pagination: page size 2, walk via next_after_seq, no gaps or overlaps.
    let mut collected = Vec::new();
    let mut after: Option<i64> = None;
    loop {
        let mut params = json!({"repository_id": id_a, "limit": 2});
        if let Some(a) = after {
            params["after_seq"] = json!(a);
        }
        let page = daemon.call(methods::EVENTS_LIST, &params).await.unwrap();
        let items = page["events"].as_array().unwrap().clone();
        collected.extend(items.iter().map(|e| e["seq"].as_i64().unwrap()));
        match page["next_after_seq"].as_i64() {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    let full: Vec<i64> = only_a["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(
        collected, full,
        "paged walk must reproduce the full seq-ordered stream"
    );

    // Limit clamp: absurd limit accepted but bounded.
    let clamped = daemon
        .call(methods::EVENTS_LIST, &json!({"limit": 999999}))
        .await
        .unwrap();
    assert!(clamped["events"].as_array().unwrap().len() <= 1000);
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn state_changed_payload_links_from_and_to_snapshots() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    let reg = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": repo.root().to_string_lossy()}),
        )
        .await
        .unwrap();
    let repo_id = reg["repository"]["repository_id"]
        .as_str()
        .unwrap()
        .to_string();
    let inst = uuid::Uuid::new_v4().to_string();
    let started = daemon
        .call(
            methods::SESSION_START,
            &json!({"path": repo.root().to_string_lossy(), "agent_type": "t",
                    "agent_instance_id": inst}),
        )
        .await
        .unwrap();
    let start_snap = started["session"]["start_snapshot"]["snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();

    repo.write("change.txt", "x\n").unwrap();
    // Wait for the tracked change to land.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let events = daemon
            .call(
                methods::EVENTS_LIST,
                &json!({"repository_id": repo_id, "limit": 1000}),
            )
            .await
            .unwrap();
        if let Some(sc) = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["event_type"] == "repository.state_changed")
        {
            assert_eq!(
                sc["payload"]["from_snapshot_id"].as_str().unwrap(),
                start_snap,
                "from must link the prior snapshot"
            );
            let to = sc["payload"]["to_snapshot_id"].as_str().unwrap();
            assert_eq!(sc["snapshot_id"].as_str().unwrap(), to);
            assert_ne!(to, start_snap);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no state_changed within 5s"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    daemon.stop().await;
}
