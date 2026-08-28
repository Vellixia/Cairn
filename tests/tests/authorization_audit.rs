//! Membership is the only route to project access (FR-418–FR-427, SC-406,
//! SC-407, SC-408, SC-465).
//!
//! The chain these tests close was live in v0.1.0-alpha.5: register an account,
//! look a project up by its public git remote, join it by naming the UUID, then
//! read and write everything in it. Each link is now closed, and each test here
//! asserts the closure rather than the capability — a suite that proved a member
//! *can* read a project would pass on a server that let everybody read it.
//!
//! The enumerating tests matter more than the individual ones. `SC-465` asks
//! whether *any* route grants self-membership, and a test naming one route
//! passes unchanged the day a different route is added. So both sweeps below
//! walk the live route set.

use cairn_e2e::{
    get_json_status_bearer, patch_json_bearer, post_json_status_bearer, Sandbox, Server,
};
use serde_json::json;
use uuid::Uuid;

fn server() -> Option<Server> {
    match Server::start() {
        Some(s) => Some(s),
        None => {
            eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the server suite");
            None
        }
    }
}

/// A user, their token, and a project only they belong to.
struct Owner {
    token: String,
    project: Uuid,
    remote: String,
}

fn owner(server: &Server, label: &str) -> Owner {
    let token = server.new_user_token(label);
    let remote = format!("git@example.test:{label}/{}.git", Uuid::now_v7());
    let created = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": label, "repository_remote": remote }),
        &token,
    );
    assert_eq!(created.1, 200, "create project: {}", created.0);
    let project = created.0["id"].as_str().expect("id").parse().expect("uuid");
    Owner {
        token,
        project,
        remote,
    }
}

/// Every project-scoped path the server exposes, for a given project id.
///
/// Written as a list so the sweeps below cover all of them at once. A route
/// added to `api::routes` without being added here is a gap, which is why
/// `every_project_scoped_path_is_covered_by_this_list` checks the two against
/// each other.
fn project_scoped_paths(project: Uuid) -> Vec<String> {
    vec![
        format!("/api/projects/{project}"),
        format!("/api/projects/{project}/tasks"),
        format!("/api/projects/{project}/sessions"),
        format!("/api/projects/{project}/memories"),
        format!("/api/projects/{project}/sync-status"),
        format!("/api/projects/{project}/members"),
        format!("/api/sync/changes?project_id={project}"),
    ]
}

/// Project-scoped routes that take the project in the **body** rather than the
/// path, so a GET sweep cannot reach them.
///
/// `POST /api/sync/batch` is the whole list, and it is swept separately below
/// rather than excluded: it is the most consequential project-scoped route on the
/// server, because it is where writes arrive.
fn body_scoped_paths() -> Vec<&'static str> {
    vec!["/api/sync/batch"]
}

/// Routes that are declared but deliberately dead: they answer `410 Gone` and
/// name their replacement, for every caller, member or not (FR-587).
///
/// These are excluded from the membership sweeps because they hold no project
/// data to protect — but the exclusion is not taken on trust. The sweep below
/// asserts each one really does answer `410` to a member *and* a non-member, so
/// a live route quietly added to this list fails rather than escaping the audit.
fn removed_paths(project: Uuid) -> Vec<String> {
    vec![format!("/api/projects/{project}/join")]
}

/// Routes under `/api/sync` that are deliberately **not** project-scoped.
///
/// Both were added by Feature 004 and neither takes a project. `changes/personal`
/// is scoped to the authenticated account — there is no parameter through which
/// one caller could name another's — and `changes/team` is server-wide by design
/// (FR-463): an authoritative team entry reaches every account regardless of
/// membership, which is the one place in this feature where authorization is not
/// mediated by project membership.
///
/// Excluded from the membership sweeps, and the exclusion is asserted rather than
/// assumed: the test below requires each to refuse an **anonymous** caller and to
/// answer an authenticated non-member, which is exactly the pair of properties
/// that makes "not project-scoped" a design rather than a gap.
fn account_scoped_sync_paths() -> Vec<&'static str> {
    vec!["/api/sync/changes/personal", "/api/sync/changes/team"]
}

/// The global read-back routes need authentication and nothing else.
#[test]
fn the_global_read_back_routes_need_a_token_and_no_membership() {
    let Some(server) = server() else { return };
    // Alice exists only so the server holds a project this caller is not in.
    let _alice = owner(&server, "global-read-alice");
    let outsider = server.new_user_token("global-read-outsider");

    for path in account_scoped_sync_paths() {
        // Anonymous is refused. A route that answered without a token would hand
        // personal knowledge to anyone who could reach the port.
        let anonymous = server.get_status(path, "not-a-token");
        assert!(
            anonymous == 401 || anonymous == 403,
            "{path} answered {anonymous} to an unauthenticated caller"
        );

        // An authenticated caller with no membership in anything is answered,
        // because neither route is about a project.
        let (body, status) = get_json_status_bearer(&server.base, path, &outsider);
        assert_eq!(
            status, 200,
            "{path} refused an authenticated caller with no project membership, which \
             would make team guidance membership-scoped (FR-463): {body}"
        );
    }
}

/// A removed route is gone for everyone, and says so.
#[test]
fn a_removed_project_route_is_gone_for_member_and_non_member_alike() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "gone-alice");
    let bob = server.new_user_token("gone-bob");

    for path in removed_paths(alice.project) {
        for (who, token) in [("the owner", &alice.token), ("a non-member", &bob)] {
            let (body, status) = post_json_status_bearer(&server.base, &path, &json!({}), token);
            assert_eq!(status, 410, "{path} answered {status} to {who}: {body}");
            assert_eq!(
                body["error"]["code"].as_str(),
                Some("route_removed"),
                "{path} refused {who} without saying the route was removed: {body}"
            );
            assert!(
                body["error"]["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("/api/projects/{id}/members")),
                "{path} did not name its replacement to {who}: {body}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// T064 / SC-406 — membership is the only route in
// ---------------------------------------------------------------------------

/// A non-member is refused by **every** project-scoped endpoint.
#[test]
fn a_non_member_is_refused_by_every_project_scoped_endpoint() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "authz-alice");
    let bob = server.new_user_token("authz-bob");

    for path in project_scoped_paths(alice.project) {
        let status = server.get_status(&path, &bob);
        assert!(
            status == 403 || status == 404,
            "{path} answered {status} to a non-member; expected a refusal"
        );
    }
    // The body-scoped routes, which a GET sweep cannot reach.
    for path in body_scoped_paths() {
        let (body, status) = post_json_status_bearer(
            &server.base,
            path,
            &json!({ "project_id": alice.project, "items": [] }),
            &bob,
        );
        assert!(
            status == 403 || status == 404,
            "{path} answered {status} to a non-member: {body}"
        );
    }

    // The owner reaches them all, so the refusals above are about membership
    // rather than about the routes being broken.
    for path in project_scoped_paths(alice.project) {
        let status = server.get_status(&path, &alice.token);
        assert_eq!(status, 200, "{path} refused its own member");
    }
    for path in body_scoped_paths() {
        let (body, status) = post_json_status_bearer(
            &server.base,
            path,
            &json!({ "project_id": alice.project, "items": [] }),
            &alice.token,
        );
        assert_eq!(status, 200, "{path} refused its own member: {body}");
    }
}

/// The path list above is the whole project-scoped surface.
///
/// Read from the router source rather than from memory: a route added later
/// fails here, which is what keeps the sweeps honest.
#[test]
fn every_project_scoped_path_is_covered_by_the_sweep_list() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace")
            .join("crates/cairn-server/src/api.rs"),
    )
    .expect("read api.rs");

    let declared: Vec<String> = source
        .lines()
        .filter_map(|l| l.split_once(".route(\"").map(|(_, rest)| rest))
        .filter_map(|rest| rest.split_once('"').map(|(path, _)| path.to_string()))
        .filter(|p| p.contains("/projects/{id}") || p.starts_with("/api/sync"))
        .collect();
    assert!(
        !declared.is_empty(),
        "the router scan found nothing; this test would pass vacuously"
    );

    let covered = project_scoped_paths(Uuid::nil());
    let removed = removed_paths(Uuid::nil());
    for path in &declared {
        if body_scoped_paths().contains(&path.as_str()) {
            continue;
        }
        // A route that answers `410` to everybody guards nothing. It is still
        // enumerated, and still asserted dead, one test above.
        if removed
            .iter()
            .any(|r| *r == path.replace("{id}", &Uuid::nil().to_string()))
        {
            continue;
        }
        // Not project-scoped, and asserted so by its own test above rather than
        // waved past here.
        if account_scoped_sync_paths().contains(&path.as_str()) {
            continue;
        }
        let shape = path.replace("{id}", &Uuid::nil().to_string());
        let matched = covered
            .iter()
            .any(|c| c.starts_with(shape.trim_end_matches('/')));
        assert!(
            matched,
            "{path} is project-scoped and absent from `project_scoped_paths`; \
             the non-member sweep does not cover it"
        );
    }
}

// ---------------------------------------------------------------------------
// T065 / SC-407 — discovery grants nothing
// ---------------------------------------------------------------------------

/// Discovery by a non-member returns an empty result.
///
/// A git remote is not a secret — it is in every clone and often on a public
/// forge — so a discovery route that matched on it alone turned a remote URL
/// into the project UUIDs behind it, which was precisely the input the deleted
/// join route needed.
#[test]
fn discovery_returns_nothing_to_a_non_member_and_grants_nothing_to_anyone() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "disc-alice");
    let bob = server.new_user_token("disc-bob");

    let (mine, status) = get_json_status_bearer(
        &server.base,
        &format!("/api/projects/lookup?remote={}", alice.remote),
        &alice.token,
    );
    assert_eq!(status, 200);
    assert_eq!(
        mine["projects"].as_array().map(|a| a.len()),
        Some(1),
        "a member cannot discover their own project: {mine}"
    );

    let (theirs, status) = get_json_status_bearer(
        &server.base,
        &format!("/api/projects/lookup?remote={}", alice.remote),
        &bob,
    );
    assert_eq!(status, 200);
    assert_eq!(
        theirs["projects"].as_array().map(|a| a.len()),
        Some(0),
        "discovery leaked a project to a non-member: {theirs}"
    );

    // And looking does not join: Bob's membership set is unchanged.
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM project_members WHERE project_id = '{}'",
            alice.project
        )),
        1,
        "discovery granted membership"
    );
}

// ---------------------------------------------------------------------------
// T066 / SC-408, FR-421 — removal takes effect on the next request
// ---------------------------------------------------------------------------

/// A removed member loses read and sync access on the **very next** request.
///
/// No cache, no session, no token carries a stale membership decision, because
/// every project-scoped route re-evaluates membership per request. The assertion
/// is deliberately made on the request immediately following the removal.
#[test]
fn a_removed_member_loses_access_on_the_next_request() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "revoke-alice");
    let (bob_id, bob_token) = server.new_user("revoke-bob");

    let (granted, status) = post_json_status_bearer(
        &server.base,
        &format!("/api/projects/{}/members", alice.project),
        &json!({ "user_id": bob_id }),
        &alice.token,
    );
    assert_eq!(status, 201, "grant failed: {granted}");

    // Bob reads and syncs, so the loss below is a loss of something he had.
    assert_eq!(
        server.get_status(&format!("/api/projects/{}", alice.project), &bob_token),
        200,
        "the granted member could not read the project"
    );
    assert_eq!(
        server.get_status(
            &format!("/api/sync/changes?project_id={}", alice.project),
            &bob_token
        ),
        200,
        "the granted member could not sync"
    );

    let removed = server.delete_json_bearer(
        &format!("/api/projects/{}/members", alice.project),
        &json!({ "user_id": bob_id }),
        &alice.token,
    );
    assert_eq!(removed, 200, "removal failed");

    assert_eq!(
        server.get_status(&format!("/api/projects/{}", alice.project), &bob_token),
        403,
        "a removed member still reads the project"
    );
    assert_eq!(
        server.get_status(
            &format!("/api/sync/changes?project_id={}", alice.project),
            &bob_token
        ),
        403,
        "a removed member still syncs the project"
    );
}

// ---------------------------------------------------------------------------
// T067 / SC-465 / FR-418 — no route grants self-membership
// ---------------------------------------------------------------------------

/// No authenticated route adds the caller to a project's membership.
///
/// Enumerated from the live router rather than asserted against one named route:
/// the deleted `POST /api/projects/{id}/join` is the defect that was *found*, and
/// FR-418 is about the routes that exist — including ones added after this
/// feature ships. A test asserting one route is absent passes unchanged on the
/// day a different route grants self-membership, which is exactly when it needed
/// to fail.
#[test]
fn no_route_adds_the_caller_to_a_projects_membership() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "self-alice");
    let (bob_id, bob) = server.new_user("self-bob");

    let before = server.count(&format!(
        "SELECT count(*) FROM project_members WHERE user_id = '{bob_id}'"
    ));

    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace")
            .join("crates/cairn-server/src/api.rs"),
    )
    .expect("read api.rs");
    let routes: Vec<String> = source
        .lines()
        .filter_map(|l| l.split_once(".route(\"").map(|(_, rest)| rest))
        .filter_map(|rest| rest.split_once('"').map(|(path, _)| path.to_string()))
        .collect();
    assert!(
        routes.len() > 10,
        "the router scan found only {} routes; it is not reading the router",
        routes.len()
    );

    for route in &routes {
        let path = route
            .replace("{id}", &alice.project.to_string())
            .replace("{project_id}", &alice.project.to_string());
        // Every verb, because a grant hiding behind an unexpected method is
        // still a grant.
        server.get_status(&path, &bob);
        post_json_status_bearer(&server.base, &path, &json!({}), &bob);
        post_json_status_bearer(
            &server.base,
            &path,
            &json!({ "user_id": bob_id, "project_id": alice.project }),
            &bob,
        );
        patch_json_bearer(&server.base, &path, &json!({}), &bob);
        server.delete_json_bearer(&path, &json!({}), &bob);
    }

    let after = server.count(&format!(
        "SELECT count(*) FROM project_members WHERE user_id = '{bob_id}'"
    ));
    assert_eq!(
        before, after,
        "calling every route changed the caller's own membership set: {before} -> {after}. \
         Some route grants self-membership (FR-418, SC-465)"
    );
}

/// A grant naming the caller is refused even from an existing member.
///
/// The rule is "a grant names somebody else", separately from "you must already
/// be a member to grant". Only the second closes the hole — an existing member
/// adding itself again is harmless, but a route shaped to allow it is one
/// refactor away from allowing a non-member to.
#[test]
fn a_membership_grant_naming_the_caller_is_refused() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "grant-alice");
    let alice_id = server
        .query_column(&format!(
            "SELECT user_id::text FROM project_members WHERE project_id = '{}'",
            alice.project
        ))
        .first()
        .cloned()
        .expect("alice's own membership");

    let (body, status) = post_json_status_bearer(
        &server.base,
        &format!("/api/projects/{}/members", alice.project),
        &json!({ "user_id": alice_id }),
        &alice.token,
    );
    assert_eq!(status, 403, "a self-naming grant was accepted: {body}");
}

/// A non-member cannot grant membership to anyone, including themselves.
#[test]
fn a_non_member_cannot_grant_membership_at_all() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "outsider-alice");
    let (bob_id, bob) = server.new_user("outsider-bob");
    let (carol_id, carol) = server.new_user("outsider-carol");
    let _ = carol;

    for (actor, target) in [(&bob, &bob_id), (&bob, &carol_id)] {
        let (body, status) = post_json_status_bearer(
            &server.base,
            &format!("/api/projects/{}/members", alice.project),
            &json!({ "user_id": target }),
            actor,
        );
        assert_eq!(status, 403, "a non-member granted membership: {body}");
    }
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM project_members WHERE project_id = '{}'",
            alice.project
        )),
        1,
        "a non-member's grant took effect"
    );
}

/// The grant records who made it (FR-419).
#[test]
fn a_grant_records_who_made_it() {
    let Some(server) = server() else { return };
    let alice = owner(&server, "audit-alice");
    let bob = server.new_user_token("audit-bob");
    let _ = bob;
    let bob_id = server
        .query_column("SELECT id::text FROM users ORDER BY created_at DESC LIMIT 1")
        .first()
        .cloned()
        .expect("bob's id");

    post_json_status_bearer(
        &server.base,
        &format!("/api/projects/{}/members", alice.project),
        &json!({ "user_id": bob_id }),
        &alice.token,
    );
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM project_members
              WHERE project_id = '{}' AND user_id = '{bob_id}' AND added_by_user_id IS NOT NULL",
            alice.project
        )),
        1,
        "the grant did not record who made it"
    );
}

// ---------------------------------------------------------------------------
// T065 / FR-424 / FR-425 / FR-428 / SC-407 — auto-link declines, never joins
// ---------------------------------------------------------------------------

/// `cairn link` with no `--project` declines rather than joining.
///
/// Three cases, and the middle one is the one that matters. Zero candidates is
/// obviously a decline. More than one is a decline because guessing among
/// memberships is a decision only the human should make. **Exactly one project
/// that the caller is not a member of** is the case that used to be a silent
/// join: discovery matched on the git remote, the remote is in every clone, and
/// the join needed nothing more than the UUID discovery handed back.
///
/// Falsified by restoring the join route, by dropping the membership join from
/// `lookup_projects`, or by making auto-link select a project the caller does not
/// already belong to.
#[test]
fn auto_link_declines_rather_than_joining_anything() {
    let Some(server) = server() else { return };

    // Alice's project is created **through the client**, not through the raw
    // API. That is not incidental: the client normalizes a git remote before it
    // stores one (`example.test/org/repo`), and `lookup` matches on the
    // normalized form. A project seeded with a raw `git@host:org/repo.git`
    // would be undiscoverable by any client, so a fixture that seeded one would
    // be testing its own remote handling rather than auto-link.
    let remote = format!("git@example.test:autolink/{}.git", Uuid::now_v7());
    let alice = Sandbox::new();
    alice.git(&["remote", "add", "origin", &remote]);
    alice.must(&["init"]);
    let alice_token = server.new_user_token("autolink-alice");
    let attached = alice.cairn(&[
        "auth",
        "token",
        "set",
        &alice_token,
        "--server",
        &server.base,
    ]);
    assert!(attached.ok(), "auth token set failed: {}", attached.stderr);
    let created = alice.json(&["link", "--create"]);
    let project = created["project"]["server_project_id"]
        .as_str()
        .or_else(|| created["project"]["id"].as_str())
        .expect("a created shared project")
        .to_string();

    // Bob shares Alice's remote but not her membership: a fresh clone of the
    // same repository, which is exactly the shape the deleted join route
    // exploited — discovery matched on the remote, and the remote is in every
    // clone.
    let bob_token = server.new_user_token("autolink-bob");
    let bob = Sandbox::new();
    bob.git(&["remote", "add", "origin", &remote]);
    bob.must(&["init"]);
    let attached = bob.cairn(&["auth", "token", "set", &bob_token, "--server", &server.base]);
    assert!(attached.ok(), "auth token set failed: {}", attached.stderr);

    let declined = bob.cairn(&["--json", "link"]);
    assert!(
        !declined.ok(),
        "auto-link succeeded for a non-member; it must decline: {}",
        declined.stdout
    );

    // And nothing was granted as a side effect of trying.
    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM project_members WHERE project_id = '{project}'"
        )),
        1,
        "attempting to auto-link added the caller to the project's membership"
    );

    // A second clone belonging to Alice — who *is* a member of exactly one
    // matching project — is selected. Asserted so the decline above is about
    // membership rather than about auto-link being broken for everyone, which is
    // the way this test would otherwise pass while protecting nothing.
    let alice_second = Sandbox::new();
    alice_second.git(&["remote", "add", "origin", &remote]);
    alice_second.must(&["init"]);
    let attached = alice_second.cairn(&[
        "auth",
        "token",
        "set",
        &alice_token,
        "--server",
        &server.base,
    ]);
    assert!(attached.ok(), "auth token set failed: {}", attached.stderr);
    let linked = alice_second.cairn(&["--json", "link"]);
    assert!(
        linked.ok(),
        "auto-link declined for a member of the one matching project: {}{}",
        linked.stdout,
        linked.stderr
    );
}
