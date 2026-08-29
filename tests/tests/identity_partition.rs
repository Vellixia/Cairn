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

/// A second machine authenticated as whoever `token` belongs to, linked to a
/// project of its own so the daemon has credentials and a project to sync.
fn second_device_for(server: &Server, token: &str) -> Sandbox {
    let label = format!("second-{}", Uuid::now_v7().simple());
    let sandbox = Sandbox::new();
    let remote = format!("git@localhost:cairnfixture/{label}.git");
    sandbox.git(&["remote", "add", "origin", &remote]);
    sandbox.must(&["init"]);
    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": label, "repository_remote": remote }),
        token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let project = created["id"].as_str().expect("id").to_string();
    attach_server(&sandbox, server, token);
    sandbox.must(&["link", "--project", &project]);
    sandbox
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

// ---------------------------------------------------------------------------
// 3. The routing invariant, at every entry point that pushes or pulls
// ---------------------------------------------------------------------------

fn lanes(s: &Sandbox) -> Vec<String> {
    s.query_column("SELECT namespace FROM sync_cursor ORDER BY namespace")
}

fn personal_lane_of(s: &Sandbox, account: &str) -> Option<String> {
    lanes(s)
        .into_iter()
        .find(|n| n.starts_with("personal:") && n.ends_with(account))
}

fn pending_on(s: &Sandbox, namespace: &str) -> i64 {
    s.query_column(&format!(
        "SELECT CAST(COUNT(*) AS TEXT) FROM outbox \
         WHERE namespace = '{namespace}' AND state IN ('pending', 'in_flight')"
    ))
    .first()
    .and_then(|n| n.parse().ok())
    .unwrap_or(0)
}

/// The background worker will not pull a personal lane belonging to an account
/// this machine is no longer authenticated as.
///
/// The guard existed in `cairn sync now` and nowhere else, so the guarantee held
/// only for as long as a user synchronized by hand. On the worker's next tick —
/// thirty seconds later, unprompted — A's lane was drained and pulled under B's
/// credentials (FR-593).
///
/// **The pull direction is what this asserts**, because it is the one the lane
/// filter alone prevents: pushing is separately closed by the author filter on
/// the claim (FR-594), so a test written against pushing passes even with the
/// worker's filter removed. A `GET /api/sync/changes/personal` sent on A's lane
/// while holding B's token returns *B's* rows — the server filters that feed by
/// the authenticated caller — and `merge_pulled_personal` files them under the
/// lane's owner, which is A.
///
/// Two details of the arrangement are load-bearing, and both were found by
/// watching an earlier version of this test pass against the defect:
///
/// - **A's note is written after the first `sync now`, and never delivered.** A
///   lane with queued work is discovered from the outbox, which the worker reads
///   before `sync_cursor`, so A's lane is pulled before B's within a tick. The
///   other order hides the bug: B's row lands under B first, and the misfile then
///   fails on the primary key instead of happening.
/// - **The assertion is on the owner of the row that arrived**, not on its
///   absence. Waiting for a row under B and then checking A would time out rather
///   than fail, because a misfiled row makes the correct insert conflict.
///
/// Falsified by removing the `may_sync_lane` filter from `run_worker`'s target
/// construction: B's note arrives owned by A.
#[test]
fn the_background_worker_will_not_pull_a_foreign_personal_lane() {
    let Some(server) = server() else { return };
    let a = device(&server, "worker-a");
    let b = device(&server, "worker-b");

    let a_id = server_account_id(&server, &a.email);
    let b_id = server_account_id(&server, &b.email);

    // A's lane, established and pulled once, with its cursor at the beginning.
    a.sandbox.must(&["sync", "now"]);
    let a_lane = personal_lane_of(&a.sandbox, &a_id).expect("A's personal lane");

    // Queued and deliberately not delivered, so A's lane is discovered from the
    // outbox and therefore reached first on a tick.
    remember_personal(&a.sandbox, "a note that stays in the queue");
    assert!(
        pending_on(&a.sandbox, &a_lane) > 0,
        "nothing is queued on A's lane, so the worker would not reach it first"
    );

    // B has personal knowledge of its own on the server, written from B's own
    // device. This is what a wrongly routed pull delivers.
    let marker = format!("worker-marker-{}", Uuid::now_v7().simple());
    remember_personal(&b.sandbox, &format!("the worker {marker} ticks"));
    b.sandbox.must(&["sync", "now"]);
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM personal_knowledge \
             WHERE owner_user_id = '{b_id}' AND content LIKE '%{marker}%'"
        )),
        1,
        "B's note never reached the server, so this test would pass vacuously"
    );

    // B takes over A's machine. A's lane stays on the store — that is the design,
    // since a store legitimately holds several identities' knowledge.
    attach_server(&a.sandbox, &server, &b.token);
    assert!(
        lanes(&a.sandbox).contains(&a_lane),
        "A's lane vanished; this test needs it present to prove it is not pulled"
    );

    // Restarted, so every lane's clock starts due. Without this, A's lane is due
    // one `PULL_INTERVAL_SECONDS` after its last pull while B's brand-new lane is
    // due at once — B's row lands first, and the misfile then fails on the
    // primary key instead of happening. A restart is not a contrivance: it is the
    // ordinary state of a machine that was switched to another account and then
    // used again later.
    a.sandbox.restart_daemon();

    // No `sync now` from here on: the worker is the entry point under test.
    let owner_column =
        format!("SELECT owner_user_id FROM personal_knowledge WHERE content LIKE '%{marker}%'");
    a.sandbox.settle_within(
        "the worker to deliver B's personal knowledge to this machine",
        std::time::Duration::from_secs(90),
        |s| !s.query_column(&owner_column).is_empty(),
    );

    assert_eq!(
        a.sandbox.query_column(&owner_column),
        vec![b_id],
        "the background worker pulled account A's lane while authenticated as \
         account B, filing B's personal knowledge into A's partition"
    );
}

/// A team proposal authored as A is not submitted after B logs in.
///
/// The `team:*` lane is shared by every account on a server, by design, and the
/// server correctly refuses to trust payload identity — so an undelivered
/// proposal pushed under the wrong token is recorded with the wrong proposer. The
/// proposal would change author by being late. It is held instead, and goes out
/// unchanged when A returns (FR-594).
///
/// Falsified by restoring the unfiltered `claim_namespace` in `drain_global`: the
/// server records B as the proposer of A's text.
#[test]
fn a_team_proposal_authored_as_a_is_not_submitted_as_b() {
    let Some(server) = server() else { return };
    let a = device(&server, "attrib-a");
    let b = device(&server, "attrib-b");

    let a_id = server_account_id(&server, &a.email);
    let b_id = server_account_id(&server, &b.email);

    let topic = format!("attrib-{}", Uuid::now_v7().simple());
    let proposed = a.sandbox.json(&[
        "team",
        "propose",
        "hold the cursor when a page does not fully merge",
        "--topic-key",
        &topic,
        "--value-key",
        "holdcursor",
    ]);
    let id = proposed["entry"]["id"].as_str().expect("an id").to_string();

    // B logs in on this machine while A's proposal is still undelivered.
    attach_server(&a.sandbox, &server, &b.token);

    // That precondition is *constructed* rather than raced for. A's own daemon
    // legitimately delivers this proposal to the server within a tick of it being
    // written — correctly, as A — so a test that merely proposed and then switched
    // was measuring which of the two happened first. Undoing the delivery on both
    // sides restores exactly the state the defect needs: a queued proposal
    // authored by A, on a shared lane, with B authenticated.
    server.execute(&format!("DELETE FROM team_knowledge WHERE id = '{id}'"));
    a.sandbox.execute_sql(&format!(
        "UPDATE outbox SET state = 'pending', delivered_at = NULL, claimed_at = NULL \
          WHERE entity_id = '{id}'"
    ));
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM team_knowledge WHERE id = '{id}'"
        )),
        0,
        "the proposal was not returned to an undelivered state"
    );

    a.sandbox.must(&["sync", "now"]);

    let submitted = server.count(&format!(
        "SELECT count(*) FROM team_knowledge WHERE id = '{id}'"
    ));
    assert_eq!(
        submitted, 0,
        "A's undelivered proposal was pushed while authenticated as B"
    );
    let wrong = server.count(&format!(
        "SELECT count(*) FROM team_knowledge WHERE proposed_by_user_id = '{b_id}' \
         AND topic_key = '{topic}'"
    ));
    assert_eq!(wrong, 0, "the proposal was attributed to B");

    // A returns, and the proposal goes out under its own author.
    attach_server(&a.sandbox, &server, &a.token);
    a.sandbox.must(&["sync", "now"]);
    let now_there = server.query_column(&format!(
        "SELECT proposed_by_user_id::text FROM team_knowledge WHERE id = '{id}'"
    ));
    assert_eq!(
        now_there,
        vec![a_id],
        "the held proposal did not go out as A once A was authenticated again"
    );
}

/// Global push uses a project the **authenticated account** belongs to.
///
/// The batch route authorizes by project membership, and the client offered the
/// first *locally linked* project — a fact about this machine's past, not about
/// who holds the token. A store linked as A and authenticated as B named A's
/// project, the route refused a caller who is not a member, and every global push
/// failed silently for as long as B stayed logged in (FR-595).
///
/// B here is a member of no project A linked, which is what makes the old
/// behaviour fail and the new behaviour succeed.
///
/// Falsified by restoring `any_linked_project`: B's own personal knowledge never
/// reaches the server, because the batch is authorized against A's project.
#[test]
fn global_push_authorizes_against_a_project_this_account_belongs_to() {
    let Some(server) = server() else { return };
    let a = device(&server, "authproj-a");

    // A second account with a project of its own, so it is a member of something
    // — and not of A's.
    let b_email = format!("authproj-b-{}@example.test", Uuid::now_v7().simple());
    server.create_user(&b_email, "authproj-b", "correct-horse-battery");
    let b_token = server.token_for(&b_email, "correct-horse-battery");
    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": "authproj-b", "repository_remote": "git@localhost:cairnfixture/authproj-b.git" }),
        &b_token,
    );
    assert_eq!(status, 200, "create B's project: {created}");

    // The store keeps A's link — the stale local fact — while authenticating as B.
    attach_server(&a.sandbox, &server, &b_token);
    let b_id = server_account_id(&server, &b_email);

    let marker = format!("authproj-marker-{}", Uuid::now_v7().simple());
    remember_personal(&a.sandbox, &format!("the batch {marker} authorizes"));
    a.sandbox.must(&["sync", "now"]);

    let landed = server.count(&format!(
        "SELECT count(*) FROM personal_knowledge \
         WHERE owner_user_id = '{b_id}' AND content LIKE '%{marker}%'"
    ));
    assert_eq!(
        landed, 1,
        "B's own personal knowledge never reached the server: the batch was \
         authorized against a project B is not a member of"
    );
}

/// A cleared account identity stays cleared across a daemon restart.
///
/// Clearing only the in-memory copy is fail-closed for the life of one process.
/// The config still named the previous account, so the next start read it back
/// and paired it with the *new* token — FR-591 reconstituted by a restart, which
/// is why the config write is now propagated rather than logged (FR-596).
///
/// Falsified by returning `Ok(())` from `forget_account_identity` regardless of
/// the save: the restarted daemon attributes new personal writes to A.
#[test]
fn a_cleared_account_identity_survives_a_daemon_restart() {
    let Some(server) = server() else { return };
    let a = device(&server, "restart-a");
    let b = device(&server, "restart-b");

    a.sandbox.must(&["sync", "now"]);
    let a_id = server_account_id(&server, &a.email);
    assert_eq!(
        account_id_in_config(&a.sandbox).as_deref(),
        Some(a_id.as_str())
    );

    // Offline switch, so nothing relearns the identity.
    let out = a.sandbox.cairn(&[
        "auth",
        "token",
        "set",
        &b.token,
        "--server",
        "http://127.0.0.1:1",
    ]);
    assert!(out.ok(), "auth token set failed: {}", out.stderr);
    assert_eq!(account_id_in_config(&a.sandbox), None);

    a.sandbox.restart_daemon();
    assert_eq!(
        account_id_in_config(&a.sandbox),
        None,
        "the restarted daemon read the previous account back off disk"
    );

    // And the restarted daemon does not file new knowledge under A.
    let marker = format!("restart-marker-{}", Uuid::now_v7().simple());
    remember_personal(&a.sandbox, &format!("the restart {marker} holds"));
    let owner_of_marker = a.sandbox.query_column(&format!(
        "SELECT owner_user_id FROM personal_knowledge WHERE content LIKE '%{marker}%'"
    ));
    assert_eq!(owner_of_marker.len(), 1, "the note was not recorded");
    assert_ne!(
        owner_of_marker[0], a_id,
        "the restarted daemon attributed a new note to the previous account"
    );
}

/// Set the read-only flag on a path, cross-platform.
fn set_readonly(path: &std::path::Path, readonly: bool) {
    let mut perms = std::fs::metadata(path)
        .expect("the file exists")
        .permissions();
    perms.set_readonly(readonly);
    std::fs::set_permissions(path, perms).expect("permissions are settable");
}

/// A credential change that cannot durably invalidate the stale account identity
/// fails, and changes nothing.
///
/// Clearing only the in-memory copy is fail-closed for the life of one process
/// and no longer: the config still names the previous account, so the next daemon
/// start reads it back and pairs it with the **new** token — FR-591
/// reconstituted by a restart. The save failure used to be logged at debug and
/// the command reported success over exactly that state (FR-596).
///
/// So the invalidation happens before anything is written down. When it fails,
/// the old token is still on disk beside the old account id and the two still
/// agree: there is no state in which a credential and an identity name different
/// accounts. The caller is told, rather than left to discover it at the next
/// restart.
///
/// Falsified by ignoring the config-save error in `forget_account_identity`, or
/// by moving the invalidation back after the token write: the command succeeds
/// and the token on disk becomes B's while the recorded account stays A's.
#[test]
fn a_credential_change_that_cannot_clear_the_stale_identity_fails_and_changes_nothing() {
    let Some(server) = server() else { return };
    let a = device(&server, "durable-a");
    let b = device(&server, "durable-b");

    a.sandbox.must(&["sync", "now"]);
    let a_id = server_account_id(&server, &a.email);
    assert_eq!(
        account_id_in_config(&a.sandbox).as_deref(),
        Some(a_id.as_str())
    );

    let token_file = a.sandbox.cairn_home().join("token");
    let config_file = a.sandbox.cairn_home().join("config.json");
    let token_before = std::fs::read_to_string(&token_file).expect("a stored token");

    set_readonly(&config_file, true);
    let out = a.sandbox.cairn(&["auth", "token", "set", &b.token]);
    set_readonly(&config_file, false);

    assert!(
        !out.ok(),
        "a token change that could not clear the recorded account reported success: {}{}",
        out.stdout,
        out.stderr
    );
    assert_eq!(
        account_id_in_config(&a.sandbox).as_deref(),
        Some(a_id.as_str()),
        "the recorded account changed even though the save failed"
    );
    assert_eq!(
        std::fs::read_to_string(&token_file).expect("a stored token"),
        token_before,
        "the new token was written while the previous account's identity stayed \
         on disk — the exact pairing that fails closed only until a restart"
    );
}

// ---------------------------------------------------------------------------
// 4. One credential snapshot per operation (FR-597, FR-598, FR-599)
// ---------------------------------------------------------------------------

fn delivered_on(s: &Sandbox, namespace: &str) -> i64 {
    s.query_column(&format!(
        "SELECT CAST(COUNT(*) AS TEXT) FROM outbox \
         WHERE namespace = '{namespace}' AND state = 'delivered'"
    ))
    .first()
    .and_then(|n| n.parse().ok())
    .unwrap_or(0)
}

/// Global work is never pushed to a server the lane does not name.
///
/// `pull_global` checked the peer's instance against the lane's; `drain_global`
/// did not, so after a relink it went on posting `team:<A>` rows at server B.
/// Nothing reported it — a push the peer accepts looks like a successful
/// delivery, and the row was marked delivered against a server that was never
/// meant to receive it (FR-598).
///
/// **The row carries no author, which is what makes this reachable.** A
/// `personal:*` lane is already held back by its owner (FR-593) and an authored
/// row by its author (FR-594), so with those two in place the instance check is
/// only reached by a row neither covers: a `team:*` row — the lane has no
/// identity in its key by design — whose `authored_by_user_id` is NULL. That is
/// not a contrived value. Migration 0007 rebuilds the outbox and adds the column,
/// and its `INSERT … SELECT` carries no author across, so **every row queued
/// before this feature has one**, and an upgraded store that relinks is exactly
/// this test.
///
/// Falsified by removing the `admits` check from `drain_global`: the row is
/// delivered, and lands in server B's tables.
#[test]
fn global_work_is_never_pushed_to_a_server_the_lane_does_not_name() {
    let Some(server_a) = Server::start_own_database() else {
        eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the identity suite");
        return;
    };
    let Some(server_b) = Server::start_own_database() else {
        return;
    };

    let a = device(&server_a, "relink-a");
    a.sandbox.must(&["sync", "now"]);

    // A proposal queued against server A's team lane.
    let topic = format!("relink-{}", Uuid::now_v7().simple());
    let proposed = a.sandbox.json(&[
        "team",
        "propose",
        "bind a push to the instance its lane names",
        "--topic-key",
        &topic,
        "--value-key",
        "bindpush",
    ]);
    let id = proposed["entry"]["id"].as_str().expect("an id").to_string();
    let team = team_lane(&a.sandbox);

    // Server A goes away before the row can be delivered to it — otherwise the
    // worker delivers it within a tick, correctly, and this measures a race.
    drop(server_a);

    // Aged into the state an upgraded store's rows are in: queued, and with no
    // recorded author, because the column did not exist when they were written.
    a.sandbox.execute_sql(&format!(
        "UPDATE outbox SET state = 'pending', delivered_at = NULL, claimed_at = NULL, \
                authored_by_user_id = NULL WHERE entity_id = '{id}'"
    ));
    let queued = pending_on(&a.sandbox, &team);
    assert!(queued > 0, "nothing was queued against server A");

    // The machine is relinked to a different deployment. The team lane is still
    // on the store, still naming server A's instance (FR-496 forbids a second).
    let b_token = server_b.new_user_token("relink-b");
    // B needs a project of its own on server B, or the batch is refused for
    // want of an authorization project (FR-595) and this test would pass
    // without ever reaching the instance check.
    let (created, status) = post_json_status_bearer(
        &server_b.base,
        "/api/projects",
        &json!({ "name": "relink-b", "repository_remote": "git@localhost:cairnfixture/relink-b.git" }),
        &b_token,
    );
    assert_eq!(status, 200, "create B's project: {created}");

    let out = a
        .sandbox
        .cairn(&["auth", "token", "set", &b_token, "--server", &server_b.base]);
    assert!(out.ok(), "auth token set failed: {}", out.stderr);
    assert!(
        lanes(&a.sandbox).contains(&team),
        "the team lane vanished; this test needs it present to prove it is not pushed"
    );

    a.sandbox.must(&["sync", "now"]);

    // Asserted as "never delivered", not as "still exactly pending": retries
    // against the now-unreachable server A move the row through `in_flight` and
    // can exhaust its attempts, which is ordinary bookkeeping and not this
    // test's subject. What must never happen is a delivery.
    assert_eq!(
        delivered_on(&a.sandbox, &team),
        0,
        "rows were marked delivered against server B"
    );
    let _ = queued;
    assert_eq!(
        server_b.count(&format!(
            "SELECT count(*) FROM team_knowledge WHERE id = '{id}'"
        )),
        0,
        "server B received knowledge queued for server A"
    );
}

/// A lane holding only another account's work is not drained at all, so it
/// makes no request.
///
/// The claim is author-scoped (FR-594) but the count the worker gated on was
/// not, so a `team:*` lane whose only queued rows were authored by a logged-out
/// account looked busy on every tick: the drain ran, refreshed capabilities over
/// the network, and claimed nothing. At a 500 ms `WORKER_TICK` that is two
/// `GET /api/version` a second against a queue that cannot move until someone
/// logs back in (FR-599).
///
/// Measured by `last_success_at`, which `process_global_namespace` writes after
/// every drain that rejects nothing — and a drain that claims nothing rejects
/// nothing, so the defect stamps it on every tick. A drain that never runs never
/// touches it. That makes "did the worker attempt this lane" a local, exact
/// question, with no request counter on the server to add.
///
/// Falsified by restoring the unscoped `counts_namespace` gate in
/// `process_global_namespace`: the timestamp advances repeatedly while nothing
/// is ever delivered.
#[test]
fn a_queue_of_only_foreign_work_is_never_drained() {
    let Some(server) = server() else { return };
    let a = device(&server, "idlespin-a");
    let b = device(&server, "idlespin-b");

    // A proposal authored as A, queued on the shared team lane, never delivered.
    let topic = format!("idlespin-{}", Uuid::now_v7().simple());
    let proposed = a.sandbox.json(&[
        "team",
        "propose",
        "count drain eligibility by author, not by namespace",
        "--topic-key",
        &topic,
        "--value-key",
        "byauthor",
    ]);
    let id = proposed["entry"]["id"].as_str().expect("an id").to_string();
    let team = team_lane(&a.sandbox);

    // B takes over. A's proposal is now foreign work: held, unclaimable, and —
    // this is the assertion — not worth attempting.
    attach_server(&a.sandbox, &server, &b.token);

    // The undelivered state is constructed, not raced for: A's own daemon
    // delivers this proposal within a tick of it being written, correctly, so a
    // test that proposed and then switched would be measuring which happened
    // first. Undoing the delivery on both sides restores the state the defect
    // needs — a queued proposal authored by A, on a lane B also uses.
    server.execute(&format!("DELETE FROM team_knowledge WHERE id = '{id}'"));
    a.sandbox.execute_sql(&format!(
        "UPDATE outbox SET state = 'pending', delivered_at = NULL, claimed_at = NULL \
          WHERE entity_id = '{id}'"
    ));
    assert!(
        pending_on(&a.sandbox, &team) > 0,
        "nothing is queued on the team lane, so this test would pass vacuously"
    );

    let stamp = |s: &Sandbox| {
        s.query_column(&format!(
            "SELECT COALESCE(last_success_at, '') FROM sync_cursor WHERE namespace = '{team}'"
        ))
        .first()
        .cloned()
        .unwrap_or_default()
    };

    // Let the worker tick many times over: `WORKER_TICK` is 500 ms, so this is
    // roughly twenty ticks, and the defect drained on every one of them.
    // Settle past the switch itself before measuring. A tick landing in the
    // instant `set_token` is updating the identity may attempt this lane once,
    // which is a boundary, not a spin — and a spin is what this asserts.
    std::thread::sleep(std::time::Duration::from_secs(3));

    // Roughly twenty `WORKER_TICK`s. The defect drained on every one of them.
    let before = stamp(&a.sandbox);
    std::thread::sleep(std::time::Duration::from_secs(10));
    let after = stamp(&a.sandbox);

    assert_eq!(
        before, after,
        "the worker drained a lane whose only queued work belongs to another \
         account, once per tick, each time claiming nothing"
    );
    assert!(
        pending_on(&a.sandbox, &team) > 0,
        "A's held proposal left the queue while B was authenticated"
    );
}

/// Switching accounts while global sync is running never produces an operation
/// routed as one account and authenticated as another.
///
/// The two facts a global operation must agree on — whose lane this is, and whom
/// we are authenticating as — came from two separate reads of the same field,
/// with a network round trip between them. `cairn auth token set` writes that
/// field, so a switch landing in the window produced exactly the pairing FR-593
/// and FR-567 forbid, reached through timing rather than through a missing check.
/// [`GlobalGuard`] closes it by snapshotting the credential once per operation
/// (FR-597).
///
/// Hammering the window rather than contriving one instant: switches are
/// interleaved with syncs for many rounds, while both accounts hold personal
/// knowledge with distinguishable markers. The invariant is checked over the
/// whole run, since any single misroute leaves a permanent, observable trace —
/// a row filed under the account that did not write it.
///
/// Falsified by re-reading the credential inside the operation instead of taking
/// it from the guard — with the gap between the two reads widened, because the
/// real one is microseconds wide and a probabilistic test that only sometimes
/// lands is not evidence either way. Widened, the split happens on nearly every
/// run and **both** markers come back filed under the wrong account, which is
/// what this asserts. The guard does not narrow that window; it removes it, so
/// there is nothing left for the width to matter to.
#[test]
fn switching_accounts_during_sync_never_splits_an_operation() {
    let Some(server) = server() else { return };
    let a = device(&server, "toctou-a");
    let b = device(&server, "toctou-b");

    let a_id = server_account_id(&server, &a.email);
    let b_id = server_account_id(&server, &b.email);

    // Both accounts have personal knowledge on the server, written from their own
    // devices, so either lane has something a wrongly routed pull could deliver.
    // Markers deliberately unlike the sandboxes' project names: the content
    // screen refuses project-identifying text, and a marker echoing the project
    // label is exactly that (FR-517).
    let a_marker = format!("zephyr-{}", Uuid::now_v7().simple());
    let b_marker = format!("marlin-{}", Uuid::now_v7().simple());
    remember_personal(&a.sandbox, &format!("the estimator {a_marker} rounds down"));
    a.sandbox.must(&["sync", "now"]);
    remember_personal(&b.sandbox, &format!("the parser {b_marker} is greedy"));
    b.sandbox.must(&["sync", "now"]);

    // A third machine that holds both identities in turn. The switch has to land
    // *inside* a running sync to reach the window at all, so the two run
    // concurrently against one daemon rather than in sequence — sequential calls
    // never overlap, and a test that alternates them proves nothing.
    let machine = second_device_for(&server, &a.token);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while std::time::Instant::now() < deadline {
                // Not `must`: a sync racing a credential change is allowed to
                // fail. What it may never do is succeed incoherently.
                let _ = machine.cairn(&["sync", "now"]);
            }
        });
        scope.spawn(|| {
            let mut round = 0;
            while std::time::Instant::now() < deadline {
                let token = if round % 2 == 0 { &b.token } else { &a.token };
                let _ = machine.cairn(&["auth", "token", "set", token]);
                round += 1;
            }
        });
    });

    // Every row this machine holds is filed under the account that wrote it. A
    // split operation — routed as one, authenticated as the other — files a row
    // under the wrong owner, and nothing later corrects it.
    let misfiled = machine.query_column(&format!(
        "SELECT owner_user_id || ' :: ' || content FROM personal_knowledge \
         WHERE (content LIKE '%{a_marker}%' AND owner_user_id <> '{a_id}') \
            OR (content LIKE '%{b_marker}%' AND owner_user_id <> '{b_id}')"
    ));
    assert!(
        misfiled.is_empty(),
        "an operation routed as one account and authenticated as another: {misfiled:?}"
    );
}
