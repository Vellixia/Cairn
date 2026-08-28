//! What a store believes about *who it is*, and what a cursor believes about
//! *whose feed it walked* (FR-591, FR-592, FR-567, FR-568, FR-496).
//!
//! Both are cached facts learned from a server, and both were trusted for longer
//! than they were true.
//!
//! `account_id` is persisted on purpose: a daemon that restarts offline must
//! still know whose personal partition it is holding, because falling back to
//! the machine's local id would silently reassign every existing row. But
//! `learn_account_identity` is best-effort — on an unreachable server it reports
//! success when an id is *already* recorded — so setting a second account's token
//! while offline left the first account's id in place, and namespace
//! establishment then skipped relearning for the same reason: the field was
//! `Some`. Account B read and wrote inside account A's partition with nothing
//! anywhere reporting a failure.
//!
//! The team cursor is the same shape of mistake one level up. A `personal:*` lane
//! key carries its owner, so two accounts get two lanes and two cursors. A
//! `team:*` key carries only the server instance — deliberately, since a store
//! binds to exactly one server's team corpus — while the team *feed* is not the
//! same feed for every caller: a pending proposal reaches its author and any
//! admin and no one else. A member promoted to admin therefore inherited a cursor
//! that had already walked past proposals it could not see then and can see now,
//! and a monotonic cursor never asks for them again.
//!
//! These tests are written against the local store and the server's own tables
//! rather than against log lines, because every one of these failures was silent.

use cairn_e2e::{attach_server, post_json_status_bearer, Mcp, Sandbox, Server};
use serde_json::json;
use uuid::Uuid;

fn server() -> Option<Server> {
    match Server::start() {
        Some(s) => Some(s),
        None => {
            eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the identity suite");
            None
        }
    }
}

/// A linked sandbox authenticated as a fresh account.
struct Device {
    sandbox: Sandbox,
    email: String,
    token: String,
}

fn device(server: &Server, label: &str) -> Device {
    let sandbox = Sandbox::new();
    let remote = format!("git@localhost:cairnfixture/{label}.git");
    sandbox.git(&["remote", "add", "origin", &remote]);
    sandbox.must(&["init"]);

    let email = format!("{label}-{}@example.test", Uuid::now_v7().simple());
    server.create_user(&email, label, "correct-horse-battery");
    let token = server.token_for(&email, "correct-horse-battery");

    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": label, "repository_remote": remote }),
        &token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let project = created["id"].as_str().expect("id").to_string();

    attach_server(&sandbox, server, &token);
    sandbox.must(&["link", "--project", &project]);
    Device {
        sandbox,
        email,
        token,
    }
}

/// Record one personal note through the MCP surface an agent actually uses.
fn remember_personal(s: &Sandbox, content: &str) {
    let mut mcp = Mcp::start(s);
    let created = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "create",
            "domain": "personal",
            "type": "fact",
            "content": content,
        }),
        &s.repo_path().to_string_lossy(),
    );
    assert_eq!(
        created["isError"], false,
        "personal create failed: {created}"
    );
}

fn owners(s: &Sandbox) -> Vec<String> {
    s.query_column("SELECT DISTINCT owner_user_id FROM personal_knowledge ORDER BY owner_user_id")
}

fn account_id_in_config(s: &Sandbox) -> Option<String> {
    let text = std::fs::read_to_string(s.cairn_home().join("config.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("server_account_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// The account id the server itself assigned to `email`.
fn server_account_id(server: &Server, email: &str) -> String {
    server
        .query_column(&format!(
            "SELECT id::text FROM users WHERE email = '{email}'"
        ))
        .first()
        .cloned()
        .expect("the server knows this account")
}

// ---------------------------------------------------------------------------
// 1. A credential change invalidates the identity it no longer proves
// ---------------------------------------------------------------------------

/// Setting a second account's token while the server is unreachable leaves this
/// store with **no** account identity rather than the previous one.
///
/// This is the offline case the persisted id exists for, turned against it. The
/// lookup is allowed to fail; what must not survive the failure is the belief
/// that this process is still account A.
///
/// Falsified by removing the `credential_changed` invalidation from `set_token`:
/// the config keeps A's id, and every later personal write lands in A's
/// partition under a token that belongs to somebody else.
#[test]
fn an_offline_token_switch_forgets_the_previous_account_identity() {
    let Some(server) = server() else { return };
    let a = device(&server, "idswitch-a");
    let b = device(&server, "idswitch-b");

    a.sandbox.must(&["sync", "now"]);
    let a_id = server_account_id(&server, &a.email);
    assert_eq!(
        account_id_in_config(&a.sandbox).as_deref(),
        Some(a_id.as_str()),
        "the first link did not record the authenticated account at all"
    );

    // B's token, and a server that cannot answer `GET /api/auth/me`. Port 1 is
    // reserved and never listening, so the lookup fails for a reason that has
    // nothing to do with the credential being wrong.
    let out = a.sandbox.cairn(&[
        "auth",
        "token",
        "set",
        &b.token,
        "--server",
        "http://127.0.0.1:1",
    ]);
    assert!(out.ok(), "auth token set failed: {}", out.stderr);

    assert_eq!(
        account_id_in_config(&a.sandbox),
        None,
        "a token switch that could not learn the new identity kept the old one"
    );
}

/// After an **offline** token switch, account B neither reads account A's
/// personal knowledge nor writes into A's partition.
///
/// Offline is the whole point. With the server reachable the identity lookup
/// succeeds and overwrites A's id on its own, so an online switch proves nothing
/// about the invalidation — it passes with the bug fully present. The failure
/// needs the lookup to fail: that is when the stale id survives, and when reading
/// and writing both keep pointing at A.
///
/// The two halves are separate failures with one cause. Reading is
/// `recall_personal` filtered on whatever `owner_identity` returns; writing is
/// `create_personal` stamping the same value.
///
/// Falsified by removing the `credential_changed` invalidation from `set_token`:
/// A's marker comes back in B's search, and B's note joins A's rows under A's
/// owner id.
#[test]
fn account_b_cannot_read_or_write_under_account_a_partition_offline() {
    let Some(server) = server() else { return };
    let a = device(&server, "partition-a");
    let b = device(&server, "partition-b");

    let marker = format!("alpha-marker-{}", Uuid::now_v7().simple());
    remember_personal(&a.sandbox, &format!("the estimator {marker} undercounts"));
    a.sandbox.must(&["sync", "now"]);

    let a_id = server_account_id(&server, &a.email);
    assert_eq!(owners(&a.sandbox), vec![a_id.clone()]);

    // The same machine, now holding B's token, with nothing to ask who B is.
    let out = a.sandbox.cairn(&[
        "auth",
        "token",
        "set",
        &b.token,
        "--server",
        "http://127.0.0.1:1",
    ]);
    assert!(out.ok(), "auth token set failed: {}", out.stderr);

    // Reading: A's marker is not B's to see. Through `memory search`, whose
    // personal section is `recall_personal` scoped to the owning account — the
    // read path a stale identity pointed straight at A.
    let searched = a.sandbox.json(&["memory", "search", &marker]);
    let text = searched.to_string();
    assert!(
        !text.contains(&marker),
        "a machine holding B's token recalled A's personal knowledge: {text}"
    );

    // Writing: the new note is not filed under A, and A's row keeps its own
    // owner. Which account the write *does* belong to is undecided until a live
    // `GET /api/auth/me` answers, and this is what failing closed looks like: the
    // machine's own id, which is nobody's account partition.
    let b_marker = format!("beta-marker-{}", Uuid::now_v7().simple());
    remember_personal(&a.sandbox, &format!("the parser {b_marker} is greedy"));
    let after = owners(&a.sandbox);
    assert_eq!(
        after.len(),
        2,
        "the offline write did not open a partition of its own: {after:?}"
    );
    assert!(
        after.contains(&a_id),
        "A's existing row lost its owner: {after:?}"
    );
    let new_owner = after.iter().find(|o| **o != a_id).expect("a second owner");
    assert_ne!(
        *new_owner, a_id,
        "the write landed in A's partition under someone else's token"
    );
}

/// Logging out drops the identity along with the credential.
///
/// The same class as the token switch, one route over: `logout` cleared the token
/// and left the account id, so local personal writes kept aiming at the
/// logged-out account's partition — and the id was still sitting there, looking
/// authoritative, for whoever logged in on this machine next.
///
/// Falsified by removing the `forget_account_identity` call from `logout`.
#[test]
fn logging_out_forgets_the_account_identity() {
    let Some(server) = server() else { return };
    let a = device(&server, "logout-id");
    a.sandbox.must(&["sync", "now"]);
    assert!(account_id_in_config(&a.sandbox).is_some());

    a.sandbox.must(&["auth", "logout"]);
    assert_eq!(
        account_id_in_config(&a.sandbox),
        None,
        "logout left this machine believing it is still the logged-out account"
    );
}

// ---------------------------------------------------------------------------
// 2. A team cursor is a position in one caller's feed
// ---------------------------------------------------------------------------

fn team_lane(s: &Sandbox) -> String {
    s.query_column("SELECT namespace FROM sync_cursor WHERE namespace LIKE 'team:%'")
        .first()
        .cloned()
        .expect("a team lane")
}

fn team_cursor(s: &Sandbox) -> Option<String> {
    s.query_column(&format!(
        "SELECT COALESCE(pull_cursor, '') FROM sync_cursor WHERE namespace = '{}'",
        team_lane(s)
    ))
    .first()
    .filter(|c| !c.is_empty())
    .cloned()
}

fn team_visibility(s: &Sandbox) -> Option<String> {
    s.query_column(&format!(
        "SELECT COALESCE(visibility_context, '') FROM sync_cursor WHERE namespace = '{}'",
        team_lane(s)
    ))
    .first()
    .filter(|c| !c.is_empty())
    .cloned()
}

fn local_team_ids(s: &Sandbox) -> Vec<String> {
    s.query_column("SELECT id FROM team_knowledge ORDER BY id")
}

/// A member promoted to administrator receives the pending proposals that were
/// invisible to it before, even though its cursor has already moved past them.
///
/// The setup is the one that hides the bug: an invisible row first, then a
/// visible one, so the member's cursor legitimately advances *beyond* the
/// proposal it could not see. Without a recorded visibility context the promotion
/// changes what the feed contains and nothing changes what the cursor asks for,
/// so the proposal is unreachable on this device permanently.
///
/// Falsified by dropping the `visibility` field from `sync_team_changes`, or the
/// reset in `pull_global`: the promoted admin's store never receives the pending
/// entry, and `cairn team list` cannot show an admin the thing it exists to
/// ratify.
#[test]
fn a_member_promoted_to_admin_receives_the_proposal_it_could_not_see() {
    // A database of its own, because team knowledge is server-wide and the cursor
    // is the subject here: on the shared test database every previous run's team
    // rows are in this lane's feed, and one of them failing to merge holds the
    // cursor for a reason that has nothing to do with visibility. An isolated
    // corpus makes "the cursor advanced past the hidden row" an actual assertion
    // rather than a race with the rest of the suite.
    let Some(server) = Server::start_own_database() else {
        eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the identity suite");
        return;
    };
    let proposer = device(&server, "vis-proposer");
    let member = device(&server, "vis-member");

    let hidden_topic = format!("hidden-{}", Uuid::now_v7().simple());
    let hidden = proposer.sandbox.json(&[
        "team",
        "propose",
        "prefer the bounded reader for large payloads",
        "--topic-key",
        &hidden_topic,
        "--value-key",
        "bounded",
    ]);
    let hidden_id = hidden["entry"]["id"]
        .as_str()
        .expect("a proposed id")
        .to_string();
    proposer.sandbox.must(&["sync", "now"]);

    // A second entry that *is* visible to a member, ratified so it lands in the
    // member's feed after the hidden one and drags the cursor past it.
    let visible_topic = format!("visible-{}", Uuid::now_v7().simple());
    let visible = proposer.sandbox.json(&[
        "team",
        "propose",
        "retry a blocked lane with its own backoff",
        "--topic-key",
        &visible_topic,
        "--value-key",
        "perlane",
    ]);
    let visible_id = visible["entry"]["id"]
        .as_str()
        .expect("a proposed id")
        .to_string();
    proposer.sandbox.must(&["sync", "now"]);
    server.execute(&format!(
        "UPDATE users SET role = 'admin' WHERE email = '{}'",
        proposer.email
    ));
    let ratify = proposer.sandbox.cairn(&["team", "ratify", &visible_id]);
    assert!(ratify.ok(), "ratify failed: {}", ratify.stderr);

    // The member pulls as a member: it gets the ratified entry and not the
    // pending one, and its cursor is now past both.
    member.sandbox.must(&["sync", "now"]);
    let as_member = local_team_ids(&member.sandbox);
    assert!(
        as_member.contains(&visible_id),
        "a member did not receive ratified team guidance: {as_member:?}"
    );
    assert!(
        !as_member.contains(&hidden_id),
        "a member received someone else's pending proposal: {as_member:?}"
    );
    let cursor_as_member = team_cursor(&member.sandbox);
    assert!(
        cursor_as_member.is_some(),
        "the member's team cursor never advanced, so this test would pass vacuously"
    );

    // Promotion widens the feed without touching a single row.
    server.execute(&format!(
        "UPDATE users SET role = 'admin' WHERE email = '{}'",
        member.email
    ));

    // First pull notices the changed view and re-reads from the beginning; the
    // second walks it. Draining twice is the documented cost of a visibility
    // change, not a flake allowance.
    member.sandbox.must(&["sync", "now"]);
    assert_eq!(
        team_cursor(&member.sandbox),
        None,
        "the cursor was not reset when the caller's view of the lane changed"
    );
    member.sandbox.must(&["sync", "now"]);

    let as_admin = local_team_ids(&member.sandbox);
    assert!(
        as_admin.contains(&hidden_id),
        "a promoted admin never received the pending proposal it can now see: {as_admin:?}"
    );
}

/// The recorded visibility context is the server's statement about the
/// authenticated caller, and it changes when the caller does.
///
/// Asserted separately from the behavioural test above because the value is what
/// makes the reset decidable: a client cannot compute it, since role is the
/// server's to state and role is half of what decides the filter.
///
/// Falsified by fingerprinting anything the caller sends instead of `SettledUser`,
/// or by keying the fingerprint on the account alone — a promotion then looks
/// like no change at all.
#[test]
fn the_team_visibility_context_tracks_the_authenticated_caller() {
    let Some(server) = Server::start_own_database() else {
        eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the identity suite");
        return;
    };
    let d = device(&server, "vis-context");
    d.sandbox.must(&["sync", "now"]);

    let id = server_account_id(&server, &d.email);
    assert_eq!(
        team_visibility(&d.sandbox).as_deref(),
        Some(format!("{id}:member").as_str()),
        "the lane did not record whose view of the team feed it read"
    );

    server.execute(&format!(
        "UPDATE users SET role = 'admin' WHERE email = '{}'",
        d.email
    ));
    d.sandbox.must(&["sync", "now"]);
    assert_eq!(
        team_visibility(&d.sandbox).as_deref(),
        Some(format!("{id}:admin").as_str()),
        "a role change did not register as a change of view"
    );
}
