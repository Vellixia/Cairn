//! The capability-upgrade path, end to end (T190; FR-500, FR-561, FR-562,
//! FR-563, SC-426, SC-445).
//!
//! A staged rollout is a supported configuration: the client ships before the
//! server migration runs. Everything about whether that is safe reduces to one
//! scenario, and `sync-namespaces.md` §11a states it as one sentence — personal
//! and team content queued against a server that cannot accept it is **held**
//! while project synchronization continues at full speed; after that peer is
//! replaced by a supporting server **at the same configured endpoint**, with no
//! new local write and no daemon restart, the held content delivers automatically
//! and exactly once.
//!
//! Every clause in that sentence is a separate way to get this wrong, and each
//! one is asserted below:
//!
//! - *held, not failed* — a `failed` row is never retried, so the content would be
//!   lost silently at exactly the moment the upgrade made it deliverable;
//! - *project sync continues* — losing project synchronization is losing what every
//!   existing user depends on, to gain a feature they have not asked for yet;
//! - *the same endpoint* — a test that re-pointed the client would be testing a
//!   re-link, which is a local write this scenario forbids;
//! - *no new local write, no restart* — the return to eligible has to be a
//!   consequence of the next scheduled capability read, not of the user doing
//!   something;
//! - *exactly once* — release preserves the original idempotency key, so an entry
//!   that was in flight when the capability flipped is not applied twice.

use cairn_e2e::{attach_server, post_json_status_bearer, Mcp, Sandbox, Server};
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

fn count(s: &Sandbox, sql: &str) -> String {
    s.query_column(sql).first().cloned().unwrap_or_default()
}

/// Personal and team rows queued but not yet accepted, in whatever state.
fn held(s: &Sandbox) -> String {
    count(
        s,
        "SELECT CAST(COUNT(*) AS TEXT) FROM outbox \
          WHERE state = 'blocked' AND (namespace LIKE 'personal:%' OR namespace LIKE 'team:%')",
    )
}

fn delivered(s: &Sandbox) -> String {
    count(
        s,
        "SELECT CAST(COUNT(*) AS TEXT) FROM outbox \
          WHERE state = 'delivered' AND (namespace LIKE 'personal:%' OR namespace LIKE 'team:%')",
    )
}

/// The whole scenario, in order.
#[test]
fn held_global_content_delivers_itself_after_the_peer_is_upgraded_in_place() {
    let Some(mut old) = Server::start_at_schema(2) else {
        eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the upgrade suite");
        return;
    };

    // A client against the schema-2 peer.
    let sandbox = Sandbox::new();
    let remote = "git@localhost:cairnfixture/upgrade.git";
    sandbox.git(&["remote", "add", "origin", remote]);
    sandbox.must(&["init"]);
    let token = old.new_user_token("upgrade");
    let (created, status) = post_json_status_bearer(
        &old.base,
        "/api/projects",
        &json!({ "name": "upgrade", "repository_remote": remote }),
        &token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let project: Uuid = created["id"].as_str().expect("id").parse().expect("uuid");
    attach_server(&sandbox, &old, &token);
    sandbox.must(&["link", "--project", &project.to_string()]);

    let cwd = sandbox.repo_path().to_string_lossy().to_string();
    let mut mcp = Mcp::start(&sandbox);

    // Queue one of each global domain, and one piece of project work.
    let personal = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "create",
            "domain": "personal",
            "type": "fact",
            "content": "prefer the workspace lockfile over a per-crate one",
        }),
        &cwd,
    );
    assert_eq!(
        personal["isError"], false,
        "personal create failed: {personal}"
    );
    let proposed = sandbox.json(&["team", "propose", "release tags are annotated"]);
    assert!(
        proposed["entry"]["id"].is_string(),
        "the team proposal did not land locally: {proposed}"
    );
    sandbox.must(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "project work keeps flowing throughout",
    ]);
    sandbox.must(&["sync", "now"]);

    // Held, and the project lane drained anyway.
    sandbox.settle_within(
        "both global lanes to be held while the project lane drains",
        Duration::from_secs(60),
        |s| {
            held(s) == "2"
                && count(
                    s,
                    "SELECT CAST(COUNT(*) AS TEXT) FROM outbox \
                      WHERE namespace LIKE 'project:%' AND state != 'delivered'",
                ) == "0"
        },
    );
    assert_eq!(
        count(
            &sandbox,
            "SELECT CAST(COUNT(*) AS TEXT) FROM outbox \
              WHERE state = 'failed' AND (namespace LIKE 'personal:%' OR namespace LIKE 'team:%')"
        ),
        "0",
        "a capability refusal was recorded as a permanent failure, so the upgrade \
         could never deliver it"
    );
    assert_eq!(
        old.count(
            "SELECT COUNT(*) FROM memories WHERE content = 'project work keeps flowing throughout'"
        ),
        1,
        "project synchronization did not keep draining"
    );

    // The idempotency keys, before the upgrade. Release must preserve them
    // (FR-562), and comparing them afterwards is the only way to know it did.
    let keys_before = sandbox.query_column(
        "SELECT idempotency_key FROM outbox \
          WHERE namespace LIKE 'personal:%' OR namespace LIKE 'team:%' \
          ORDER BY idempotency_key",
    );
    assert_eq!(keys_before.len(), 2);

    // Replace the peer in place. Same database, same address, every migration.
    // No `cairn` command is run against the sandbox from here on, and the daemon
    // is not restarted — anything that happens next happens on its own.
    let endpoint = old.base.clone();
    let upgraded = old.upgraded_in_place();
    assert_eq!(
        upgraded.base, endpoint,
        "the replacement moved the endpoint, so what follows would be testing a \
         re-link rather than an in-place upgrade"
    );

    sandbox.settle_within(
        "the held global content to deliver itself after the upgrade",
        Duration::from_secs(90),
        |s| delivered(s) == "2" && held(s) == "0",
    );

    // Exactly once, and with the keys it was created with.
    let keys_after = sandbox.query_column(
        "SELECT idempotency_key FROM outbox \
          WHERE namespace LIKE 'personal:%' OR namespace LIKE 'team:%' \
          ORDER BY idempotency_key",
    );
    assert_eq!(
        keys_before, keys_after,
        "release re-keyed the held entries, so a partially delivered one would \
         apply twice"
    );
    assert_eq!(
        upgraded.count("SELECT COUNT(*) FROM personal_knowledge WHERE content = 'prefer the workspace lockfile over a per-crate one'"),
        1,
        "the released personal entry was applied more or less than once"
    );
    assert_eq!(
        upgraded.count(
            "SELECT COUNT(*) FROM team_knowledge WHERE content = 'release tags are annotated'"
        ),
        1,
        "the released team entry was applied more or less than once"
    );
}
