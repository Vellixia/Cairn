//! The three synchronization lanes, and what each one does independently of the
//! others (T112–T114, T117, T118, T187; FR-486–FR-489, FR-496, FR-502, FR-522,
//! FR-562, FR-567, FR-582, SC-412, SC-425, SC-428, SC-447, SC-450).
//!
//! The lane split exists because personal and team knowledge are the first
//! content a machine can legitimately **only ever consume**. Every earlier
//! entity type could at least in principle be produced locally, so it was safe
//! for the sync loop to reach `pull` only after `drain` — a machine with nothing
//! to push had nothing to pull either. That stopped being true here: a member who
//! only reads team guidance and never proposes anything must still learn that an
//! administrator ratified something, and a second laptop must still receive
//! personal notes the first one wrote.
//!
//! So the tests here are about *independence*: one lane pulling with an empty
//! outbox, one lane blocked while another drains at full speed, one lane's
//! interrupted claim released without waiting on the others.

use cairn_e2e::{attach_server, post_json_status_bearer, Mcp, Sandbox, Server};
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

fn server() -> Option<Server> {
    match Server::start() {
        Some(s) => Some(s),
        None => {
            eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the namespace suite");
            None
        }
    }
}

/// A sandbox authenticated as `email` against `server`, with a linked project.
///
/// Linked, because personal and team knowledge do not need a project but the
/// daemon still needs credentials, and `attach_server` is how a real client gets
/// them.
struct Device {
    sandbox: Sandbox,
    token: String,
    project: Uuid,
}

fn device(server: &Server, label: &str) -> Device {
    let sandbox = Sandbox::new();
    let remote = format!("git@localhost:cairnfixture/{label}.git");
    sandbox.git(&["remote", "add", "origin", &remote]);
    sandbox.must(&["init"]);

    let token = server.new_user_token(label);
    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": label, "repository_remote": remote }),
        &token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let project: Uuid = created["id"].as_str().expect("id").parse().expect("uuid");

    attach_server(&sandbox, server, &token);
    sandbox.must(&["link", "--project", &project.to_string()]);
    Device {
        sandbox,
        token,
        project,
    }
}

/// A second device for the **same** account: a second store, the same token.
///
/// The same token is the whole point. Personal knowledge follows the account,
/// and the account is what the token names — two devices of one human are two
/// stores authenticating as one user, not two users who happen to be related.
fn second_device(server: &Server, token: &str, project: Uuid, label: &str) -> Sandbox {
    let sandbox = Sandbox::new();
    let remote = format!("git@localhost:cairnfixture/{label}.git");
    sandbox.git(&["remote", "add", "origin", &remote]);
    sandbox.must(&["init"]);
    let result = sandbox.cairn(&["auth", "token", "set", token, "--server", &server.base]);
    assert!(result.ok(), "auth token set failed: {}", result.stderr);
    sandbox.must(&["link", "--project", &project.to_string()]);
    sandbox
}

fn personal_count(s: &Sandbox) -> i64 {
    s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM personal_knowledge")
        .first()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn namespaces(s: &Sandbox) -> Vec<String> {
    s.query_column("SELECT namespace FROM sync_cursor ORDER BY namespace")
}

// ---------------------------------------------------------------------------
// T112 / FR-489 / SC-412 — a lane with nothing to push still pulls
// ---------------------------------------------------------------------------

/// A device with an empty outbox receives another device's personal knowledge
/// within 60 seconds, without pushing anything first.
///
/// Sixty is twice the documented `PULL_INTERVAL_SECONDS` (FR-589), so a passing
/// test does not depend on landing inside a single window.
///
/// Falsified by putting the pull back behind the `pending == 0` short-circuit,
/// or by discovering lanes from the outbox alone: the second device has never
/// written personal knowledge, so it has no `personal:*` outbox row and would
/// never form a target.
#[test]
fn a_device_with_nothing_queued_still_receives_the_others_personal_knowledge() {
    let Some(server) = server() else { return };
    let first = device(&server, "pullonly-a");

    // Device one writes; device two never writes anything at all.
    let mut mcp = Mcp::start(&first.sandbox);
    let created = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "create",
            "domain": "personal",
            "type": "fact",
            "content": "the estimator undercounts by four tokens",
        }),
        &first.sandbox.repo_path().to_string_lossy(),
    );
    assert_eq!(
        created["isError"], false,
        "personal create failed: {created}"
    );
    first.sandbox.must(&["sync", "now"]);

    let second = second_device(&server, &first.token, first.project, "pullonly-b");
    assert_eq!(
        personal_count(&second),
        0,
        "the second device started with personal knowledge of its own"
    );
    assert!(
        namespaces(&second)
            .iter()
            .any(|n| n.starts_with("personal:")),
        "the second device never established a personal lane, so it can never \
         pull one: {:?}",
        namespaces(&second)
    );

    second.settle_within(
        "the second device to receive personal knowledge it never wrote",
        Duration::from_secs(60),
        |s| personal_count(s) == 1,
    );

    let content = second.query_column("SELECT content FROM personal_knowledge");
    assert_eq!(
        content,
        vec!["the estimator undercounts by four tokens".to_string()]
    );
}

// ---------------------------------------------------------------------------
// T114 / FR-492 / FR-582 / SC-450 — writer identity survives the round trip,
// and a hole in a writer's stream is reported
// ---------------------------------------------------------------------------

/// A personal record created on one device pulls into a second with its
/// `writer_id` and `writer_seq` intact, and a deliberately withheld middle
/// record is reported as a **detected gap** rather than silently ignored.
///
/// A gap nobody reports is indistinguishable from a stream that had no gap,
/// which is the entire reason `writer_seq` crosses the wire — it is useless to
/// the store that minted it and useful only to whoever receives it.
///
/// Falsified by making either column nullable on the pull path (the reflex
/// repair, and the wrong one: it destroys the detection the fields exist for),
/// or by dropping the gap report.
#[test]
fn a_pulled_record_keeps_its_writer_identity_and_a_withheld_one_is_a_reported_gap() {
    let Some(server) = server() else { return };
    let first = device(&server, "writerseq-a");
    let cwd = first.sandbox.repo_path().to_string_lossy().to_string();
    let mut mcp = Mcp::start(&first.sandbox);

    for content in ["note one", "note two", "note three"] {
        let created = mcp.tool_result(
            "cairn_remember",
            json!({
                "action": "create",
                "domain": "personal",
                "type": "fact",
                "content": content,
            }),
            &cwd,
        );
        assert_eq!(created["isError"], false, "create failed: {created}");
    }
    // Wait for all three to actually reach the server before withholding one:
    // deleting a row that has not arrived yet withholds nothing, and the gap
    // this test is about would never exist.
    first.sandbox.must(&["sync", "now"]);
    first.sandbox.settle_within(
        "all three personal records to be delivered",
        Duration::from_secs(30),
        |s| {
            s.query_column(
                "SELECT CAST(COUNT(*) AS TEXT) FROM outbox \
                  WHERE namespace LIKE 'personal:%' AND state = 'delivered'",
            ) == vec!["3".to_string()]
        },
    );

    let written = first.sandbox.query_column(
        "SELECT writer_id || ':' || writer_seq FROM personal_knowledge ORDER BY writer_seq",
    );
    assert_eq!(written.len(), 3, "three records: {written:?}");

    // Withhold the middle one at the server, which is exactly what a lost
    // delivery looks like from the far side.
    let middle: String = first
        .sandbox
        .query_column("SELECT id FROM personal_knowledge ORDER BY writer_seq")
        .into_iter()
        .nth(1)
        .expect("a middle record");
    server.execute(&format!(
        "DELETE FROM personal_knowledge WHERE id = '{middle}'"
    ));

    let second = second_device(&server, &first.token, first.project, "writerseq-b");
    second.settle_within(
        "the second device to receive the two records that were not withheld",
        Duration::from_secs(60),
        |s| personal_count(s) == 2,
    );

    // Identity and sequence intact, for both that arrived.
    let pulled = second.query_column(
        "SELECT writer_id || ':' || writer_seq FROM personal_knowledge ORDER BY writer_seq",
    );
    for pair in &pulled {
        assert!(
            written.contains(pair),
            "a pulled record's writer identity changed in transit: {pair} not in {written:?}"
        );
    }

    // And the hole is reported by name rather than being invisible.
    let status = second.json(&["sync", "status"]);
    let rendered = status.to_string();
    assert!(
        rendered.contains("gap") || rendered.contains("missing"),
        "a withheld record in a writer's stream is not reported anywhere: {status}"
    );
}

// ---------------------------------------------------------------------------
// T117 / FR-567 / SC-447 — two identities coexist, recall shows one
// ---------------------------------------------------------------------------

/// A store linked in turn to two different server instances retains both
/// identities' personal knowledge, and returns only the currently linked
/// identity's entries from search, context and listing.
///
/// "Personal knowledge follows the user" sounds like one flat pool per human. It
/// is not, because a user account is per-server: the same person is a different
/// account, with a different id, on every server they link to. Partitioning by
/// account is what makes the sentence true without Cairn having to solve the
/// out-of-scope problem of recognising that two accounts belong to one human.
///
/// Falsified by merging the two partitions, or by leaking the unlinked
/// identity's rows into any read path.
#[test]
fn a_store_holding_two_identities_surfaces_only_the_linked_one() {
    let Some(server_a) = server() else { return };
    let Some(server_b) = Server::start_own_database() else {
        eprintln!("SKIPPED: a second server database is unavailable");
        return;
    };

    let one = device(&server_a, "twoident-a");
    let cwd = one.sandbox.repo_path().to_string_lossy().to_string();

    let mut mcp = Mcp::start(&one.sandbox);
    let created = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "create",
            "domain": "personal",
            "type": "fact",
            "content": "an note belonging to the first identity",
        }),
        &cwd,
    );
    assert_eq!(created["isError"], false, "create failed: {created}");
    drop(mcp);

    // Relink the same store to a different server: a different instance, a
    // different account, a different personal lane.
    let other_token = server_b.new_user_token("twoident-b");
    let result = one.sandbox.cairn(&[
        "auth",
        "token",
        "set",
        &other_token,
        "--server",
        &server_b.base,
    ]);
    assert!(result.ok(), "relink failed: {}", result.stderr);

    let mut mcp = Mcp::start(&one.sandbox);
    let created = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "create",
            "domain": "personal",
            "type": "fact",
            "content": "a note belonging to the second identity",
        }),
        &cwd,
    );
    assert_eq!(created["isError"], false, "create failed: {created}");

    // Both are retained.
    assert_eq!(
        personal_count(&one.sandbox),
        2,
        "relinking discarded the first identity's personal knowledge"
    );
    let lanes = namespaces(&one.sandbox);
    assert!(
        lanes.iter().filter(|n| n.starts_with("personal:")).count() >= 2,
        "the two identities share one lane: {lanes:?}"
    );

    // Only the current one is surfaced.
    let listed = one.sandbox.json(&["personal", "list"]);
    let rendered = listed.to_string();
    assert!(
        rendered.contains("second identity"),
        "the current identity's own note is missing: {listed}"
    );
    assert!(
        !rendered.contains("first identity"),
        "the previous identity's note leaked into listing: {listed}"
    );

    let searched = mcp.tool_result("cairn_search", json!({ "query": "identity" }), &cwd);
    let rendered = searched["content"][0]["text"].to_string();
    assert!(
        !rendered.contains("first identity"),
        "the previous identity's note leaked into search: {rendered}"
    );
}

// ---------------------------------------------------------------------------
// T118 / FR-496 / SC-428 — team is server-bound and refused; personal is not
// ---------------------------------------------------------------------------

/// A store bound to one server instance refuses to merge a second instance's
/// **team** knowledge, while a second identity's **personal** knowledge on the
/// same store is retained and partitioned rather than refused.
///
/// The asymmetry is the design (D438). Team knowledge is a claim about one
/// specific server's ratification history, so blending two deployments' guidance
/// would silently mix policy from a staging server, or a restored backup, into
/// production. Personal knowledge has no such claim in it — it is partitioned by
/// the account that owns it, and one store may legitimately hold several.
///
/// Falsified by symmetrising either half: refusing personal, or admitting team.
#[test]
fn team_knowledge_is_refused_across_instances_and_personal_knowledge_is_not() {
    let Some(server_a) = server() else { return };
    let Some(server_b) = Server::start_own_database() else {
        eprintln!("SKIPPED: a second server database is unavailable");
        return;
    };

    let one = device(&server_a, "bound-a");
    let cwd = one.sandbox.repo_path().to_string_lossy().to_string();

    // Bind this store's team corpus to instance A by proposing there.
    let proposed = one
        .sandbox
        .json(&["team", "propose", "release tags are annotated"]);
    assert!(
        proposed["entry"]["id"].is_string(),
        "the proposal did not land: {proposed}"
    );
    one.sandbox.must(&["sync", "now"]);
    let bound = namespaces(&one.sandbox);
    let team_lane = bound
        .iter()
        .find(|n| n.starts_with("team:"))
        .cloned()
        .expect("a team lane after proposing");

    // Now point the same store at instance B, and give B a team entry of its
    // own to offer.
    let other_token = server_b.new_user_token("bound-b");
    let result = one.sandbox.cairn(&[
        "auth",
        "token",
        "set",
        &other_token,
        "--server",
        &server_b.base,
    ]);
    assert!(result.ok(), "relink failed: {}", result.stderr);

    let mut mcp = Mcp::start(&one.sandbox);
    let created = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "create",
            "domain": "personal",
            "type": "fact",
            "content": "a note made while linked to the second server",
        }),
        &cwd,
    );
    assert_eq!(
        created["isError"], false,
        "personal knowledge was refused on server-instance grounds, which is \
         exactly what D438 says it must never be: {created}"
    );

    // Team stays bound to A: the lane B would need is a different key, and the
    // store's team corpus refuses to move.
    let after = namespaces(&one.sandbox);
    assert!(
        after.contains(&team_lane),
        "the store's original team lane disappeared: {after:?}"
    );
    // Instance B is given team guidance of its own, so there is something for the
    // store to have wrongly merged. Then the assertion is about *that* row, not
    // about a total: this store legitimately holds every entry instance A has
    // ratified, because team knowledge is server-wide (FR-463), and counting all
    // of them would fail for the right behaviour.
    let b_admin_email = format!("bound-b-admin-{}@example.test", Uuid::now_v7());
    server_b.create_user(&b_admin_email, "bound-b-admin", "hunter2hunter2");
    server_b.execute(&format!(
        "UPDATE users SET role = 'admin' WHERE email = '{b_admin_email}'"
    ));
    server_b.execute(
        "INSERT INTO team_knowledge \
           (id, knowledge_type, content, state, proposed_by_user_id, \
            ratified_by_user_id, ratified_at, writer_id, writer_seq, created_at) \
         SELECT gen_random_uuid(), 'convention', \
                'guidance that belongs to the second server instance', \
                'authoritative', id, id, now(), gen_random_uuid()::text, 1, now() \
           FROM users LIMIT 1",
    );

    one.sandbox.must(&["sync", "now"]);
    let leaked = one.sandbox.query_column(
        "SELECT CAST(COUNT(*) AS TEXT) FROM team_knowledge \
          WHERE content LIKE '%second server instance%'",
    );
    assert_eq!(
        leaked,
        vec!["0".to_string()],
        "a second instance's team knowledge merged into this store's corpus"
    );
}

// ---------------------------------------------------------------------------
// T113 / FR-502 / FR-562 — an interrupted claim releases per lane
// ---------------------------------------------------------------------------

/// On daemon start, a lane whose claim was interrupted releases and resumes
/// without waiting on any other lane.
///
/// A drainer that dies mid-send leaves rows `in_flight` with nothing left to
/// acknowledge them. Releasing them per lane is what stops one wedged lane from
/// holding the other two: the personal lane recovering must not depend on the
/// project lane being reachable.
///
/// Falsified by releasing claims globally in one statement conditioned on any
/// single lane, or by not releasing them at start at all.
#[test]
fn an_interrupted_lane_releases_its_own_claim_at_start_without_waiting_on_others() {
    let Some(server) = server() else { return };
    let d = device(&server, "claim");
    let cwd = d.sandbox.repo_path().to_string_lossy().to_string();
    let mut mcp = Mcp::start(&d.sandbox);

    let created = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "create",
            "domain": "personal",
            "type": "fact",
            "content": "a note that will be caught mid-flight",
        }),
        &cwd,
    );
    assert_eq!(created["isError"], false, "create failed: {created}");
    drop(mcp);
    d.sandbox.stop_daemon();

    // Simulate the interrupted send on the personal lane only, and wedge the
    // project lane at the same time so a recovery that waited on it would fail.
    d.sandbox.execute_sql(
        "UPDATE outbox SET state = 'in_flight', claimed_at = '1970-01-01T00:00:00Z' \
          WHERE namespace LIKE 'personal:%'",
    );
    // Scoped to the personal lane. Counting every `in_flight` row would race
    // with the project lane's own drain, which is the very independence this
    // test is about — a flake here would be the test contradicting itself.
    let claimed = d.sandbox.query_column(
        "SELECT CAST(COUNT(*) AS TEXT) FROM outbox \
          WHERE state = 'in_flight' AND namespace LIKE 'personal:%'",
    );
    assert_eq!(
        claimed,
        vec!["1".to_string()],
        "the fixture did not create an interrupted claim: {claimed:?}"
    );

    d.sandbox.restart_daemon();
    d.sandbox.settle_within(
        "the interrupted personal claim to be released and delivered",
        Duration::from_secs(60),
        |s| {
            s.query_column(
                "SELECT CAST(COUNT(*) AS TEXT) FROM outbox \
                  WHERE state = 'in_flight' AND namespace LIKE 'personal:%'",
            ) == vec!["0".to_string()]
        },
    );
}

// ---------------------------------------------------------------------------
// T187 / FR-522 / SC-425 — one lane blocked, the others at full speed
// ---------------------------------------------------------------------------

/// Against a real schema-2 server, personal and team entries go `blocked` while
/// project sync keeps draining at full speed, and the degradation is reported by
/// name.
///
/// This is the guarantee that makes a staged rollout safe: an operator who has
/// not run migration 3 yet must not lose project synchronization, which is what
/// every existing user depends on. Held, not failed — a `blocked` row is
/// delivered after the upgrade with its original idempotency key.
///
/// Falsified by letting a schema-3 entity type reach a schema-2 table (which
/// surfaces as an internal error, and an internal error is not a held item), or
/// by failing the whole lane set on one lane's refusal.
#[test]
fn on_a_schema_two_server_the_global_lanes_block_and_project_sync_keeps_draining() {
    let Some(mut old) = Server::start_at_schema(2) else {
        eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the namespace suite");
        return;
    };
    let d = device(&old, "blocked");
    let cwd = d.sandbox.repo_path().to_string_lossy().to_string();
    let mut mcp = Mcp::start(&d.sandbox);

    let created = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "create",
            "domain": "personal",
            "type": "fact",
            "content": "a note this server has nowhere to put",
        }),
        &cwd,
    );
    assert_eq!(
        created["isError"], false,
        "the local write failed: {created}"
    );

    // Project work, which this server can hold perfectly well.
    d.sandbox.must(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "project work keeps flowing",
    ]);
    d.sandbox.must(&["sync", "now"]);

    d.sandbox.settle_within(
        "the personal lane to be held while the project lane drains",
        Duration::from_secs(60),
        |s| {
            let blocked = s.query_column(
                "SELECT CAST(COUNT(*) AS TEXT) FROM outbox WHERE state = 'blocked' AND namespace LIKE 'personal:%'",
            );
            let project_pending = s.query_column(
                "SELECT CAST(COUNT(*) AS TEXT) FROM outbox WHERE state != 'delivered' AND namespace LIKE 'project:%'",
            );
            blocked == vec!["1".to_string()] && project_pending == vec!["0".to_string()]
        },
    );

    // Reported by name, not merely observable in the table.
    let status = d.sandbox.json(&["sync", "status"]);
    let rendered = status.to_string();
    assert!(
        rendered.contains("blocked"),
        "the degradation is not reported: {status}"
    );

    // And the project lane really did land its work on the server.
    assert_eq!(
        old.count("SELECT COUNT(*) FROM memories WHERE content = 'project work keeps flowing'"),
        1,
        "project synchronization did not keep draining"
    );
    let _ = old.upgraded();
}
