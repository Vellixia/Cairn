//! Administered accounts: creation, standing, credentials and the last-admin
//! guarantee (FR-401–FR-417, FR-539–FR-543, FR-553–FR-560, FR-585, FR-590).
//!
//! Every test here asserts a **refusal** or an **absence** as well as the happy
//! path, because every requirement in US1 is about something that must not
//! happen. A suite that only proved the successes would pass on a server that
//! refused nothing.
//!
//! Several assertions are deliberately split in two where a single combined
//! assertion could hide a regression behind its other half — SC-436 names this
//! explicitly for disable-versus-token-revocation, and the same reasoning
//! applies to expiry-versus-revocation and reset-versus-re-enable.

use cairn_e2e::{get_json_status_bearer, patch_json_bearer, post_json_status_bearer, Server};
use serde_json::json;
use uuid::Uuid;

const ADMIN_EMAIL: &str = "op@example.test";
const ADMIN_PASSWORD: &str = "hunter2hunter2";

fn server() -> Option<Server> {
    match Server::start_with_admin(ADMIN_EMAIL, ADMIN_PASSWORD) {
        Some(s) => Some(s),
        None => {
            eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the server suite");
            None
        }
    }
}

/// A token for the environment-seeded administrator.
fn admin_token(server: &Server) -> String {
    server.token_for(ADMIN_EMAIL, ADMIN_PASSWORD)
}

/// Create an account through the admin route, returning `(id, temporary_password)`.
fn create_account(server: &Server, admin: &str, label: &str) -> (Uuid, String) {
    let email = format!("{label}-{}@example.test", Uuid::now_v7());
    let (body, status) = post_json_status_bearer(
        &server.base,
        "/api/admin/users",
        &json!({ "email": email, "display_name": label }),
        admin,
    );
    assert_eq!(status, 201, "create {label}: {body}");
    let id = body["id"].as_str().expect("id").parse().expect("uuid");
    let temporary = body["temporary_password"]
        .as_str()
        .expect("temporary_password")
        .to_string();
    (id, temporary)
}

// ---------------------------------------------------------------------------
// T049 / SC-401, SC-402, SC-403 — the temporary credential's whole lifecycle
// ---------------------------------------------------------------------------

/// A temporary password is revealed once, authenticates to the change route and
/// nothing else, and is invalidated by the change it exists to permit.
#[test]
fn a_temporary_password_is_issued_once_and_buys_nothing_but_its_own_replacement() {
    let Some(server) = server() else { return };
    let admin = admin_token(&server);
    let (id, temporary) = create_account(&server, &admin, "member");

    // Revealed exactly once: no route reads it back, including for the admin who
    // created the account (FR-403, SC-402).
    let (listed, status) = get_json_status_bearer(&server.base, "/api/admin/users", &admin);
    assert_eq!(status, 200);
    let listing = listed.to_string();
    assert!(
        !listing.contains(&temporary),
        "the account listing echoed the temporary password"
    );
    assert!(
        listing.contains(&id.to_string()),
        "the account is missing from the listing (FR-411)"
    );

    // The temporary credential authenticates to the change route and nothing
    // else — including token minting, which would otherwise buy its way out of
    // the restriction entirely (FR-407, SC-403).
    let session = server.cookie_for_password(&listed_email(&listed, id), &temporary);
    let (refused, status) =
        server.post_with_cookie("/api/tokens", &json!({ "name": "x" }), &session);
    assert_eq!(
        status, 403,
        "a must-change account minted a token: {refused}"
    );
    assert_eq!(
        refused["error"]["code"], "password_change_required",
        "{refused}"
    );

    // The change succeeds, and afterwards the same account can mint a token.
    let (changed, status) = server.post_with_cookie(
        "/api/auth/password",
        &json!({ "new_password": "a-much-better-password" }),
        &session,
    );
    assert_eq!(status, 200, "password change refused: {changed}");

    let after = server.cookie_for_password(&listed_email(&listed, id), "a-much-better-password");
    let (minted, status) = server.post_with_cookie("/api/tokens", &json!({ "name": "x" }), &after);
    assert_eq!(
        status, 200,
        "token minting still refused after the change: {minted}"
    );

    // And the temporary credential is dead the instant the change lands
    // (FR-572).
    assert!(
        server
            .try_cookie_for_password(&listed_email(&listed, id), &temporary)
            .is_none(),
        "the temporary password still authenticates after the change"
    );
}

fn listed_email(listing: &serde_json::Value, id: Uuid) -> String {
    listing["users"]
        .as_array()
        .expect("users")
        .iter()
        .find(|u| u["id"].as_str() == Some(&id.to_string()))
        .and_then(|u| u["email"].as_str())
        .expect("the created account is in the listing")
        .to_string()
}

// ---------------------------------------------------------------------------
// T050 / SC-404, SC-436 — disabling, as two separate guarantees
// ---------------------------------------------------------------------------

/// A disabled account cannot authenticate with its still-valid password, **and**
/// its previously issued tokens are refused.
///
/// Two assertions, because a regression in either must not hide behind the
/// other: revoking tokens without refusing the password leaves a disabled
/// account able to sign in and mint fresh ones, and refusing the password
/// without revoking tokens leaves every cached token alive.
#[test]
fn disabling_refuses_both_the_password_and_every_token_it_had() {
    let Some(server) = server() else { return };
    let admin = admin_token(&server);
    let (id, temporary) = create_account(&server, &admin, "doomed");
    let (listing, _) = get_json_status_bearer(&server.base, "/api/admin/users", &admin);
    let email = listed_email(&listing, id);

    // Settle the account so it can hold a real token.
    let session = server.cookie_for_password(&email, &temporary);
    server.post_with_cookie(
        "/api/auth/password",
        &json!({ "new_password": "settled-password-1" }),
        &session,
    );
    let after = server.cookie_for_password(&email, "settled-password-1");
    let (minted, _) = server.post_with_cookie("/api/tokens", &json!({ "name": "cached" }), &after);
    let cached = minted["token"].as_str().expect("token").to_string();
    // The token works before the disable, so its later refusal means something.
    assert_eq!(
        server.get_status("/api/projects", &cached),
        200,
        "the token did not work before the disable, so this test proves nothing"
    );

    let (patched, status) = patch_json_bearer(
        &server.base,
        &format!("/api/admin/users/{id}"),
        &json!({ "status": "disabled" }),
        &admin,
    );
    assert_eq!(status, 200, "disable refused: {patched}");

    // Assertion one: the password no longer authenticates, though it is
    // otherwise correct (FR-410, SC-436).
    assert!(
        server
            .try_cookie_for_password(&email, "settled-password-1")
            .is_none(),
        "a disabled account signed in with an otherwise-valid password"
    );

    // Assertion two: the token it already held is refused (FR-409, SC-404).
    assert_eq!(
        server.get_status("/api/projects", &cached),
        401,
        "a token cached before the disable still works"
    );
}

/// Re-enabling does **not** resurrect the tokens revoked while the account was
/// disabled (FR-590, SC-470).
///
/// Asserted separately from the disable case above for the same reason: an
/// implementation that cleared `revoked_at` alongside `status` would pass every
/// disable-side assertion and hand back every token the account ever held.
#[test]
fn re_enabling_does_not_resurrect_a_revoked_token() {
    let Some(server) = server() else { return };
    let admin = admin_token(&server);
    let (id, temporary) = create_account(&server, &admin, "returner");
    let (listing, _) = get_json_status_bearer(&server.base, "/api/admin/users", &admin);
    let email = listed_email(&listing, id);

    let session = server.cookie_for_password(&email, &temporary);
    server.post_with_cookie(
        "/api/auth/password",
        &json!({ "new_password": "settled-password-2" }),
        &session,
    );
    let after = server.cookie_for_password(&email, "settled-password-2");
    let (minted, _) = server.post_with_cookie("/api/tokens", &json!({ "name": "old" }), &after);
    let old = minted["token"].as_str().expect("token").to_string();

    patch_json_bearer(
        &server.base,
        &format!("/api/admin/users/{id}"),
        &json!({ "status": "disabled" }),
        &admin,
    );
    patch_json_bearer(
        &server.base,
        &format!("/api/admin/users/{id}"),
        &json!({ "status": "active" }),
        &admin,
    );

    assert_eq!(
        server.get_status("/api/projects", &old),
        401,
        "re-enabling resurrected a token revoked while the account was disabled"
    );
    // The account itself works again — otherwise this test would pass on a
    // server that simply left it disabled.
    assert!(
        server
            .try_cookie_for_password(&email, "settled-password-2")
            .is_some(),
        "the account was not actually re-enabled"
    );
}

// ---------------------------------------------------------------------------
// T054 / SC-452 — expiry is indistinguishable from revocation
// ---------------------------------------------------------------------------

/// An expired token and a revoked token are refused **identically**, in status
/// and in body.
///
/// A distinguishable refusal is an oracle: it tells whoever holds a stale token
/// that it was once valid for this server, which is information about both the
/// server's history and the account. Nothing legitimate needs the distinction —
/// the remedy for either is a new token.
#[test]
fn an_expired_token_is_refused_exactly_as_a_revoked_one_is() {
    let Some(server) = server() else { return };
    let admin = admin_token(&server);

    // One token given an expiry in the past, one revoked outright.
    let (expiring, _) = post_json_status_bearer(
        &server.base,
        "/api/tokens",
        &json!({ "name": "expired", "expires_at": "2020-01-01T00:00:00Z" }),
        &admin,
    );
    let expired = expiring["token"].as_str().expect("token").to_string();

    let (revocable, _) = post_json_status_bearer(
        &server.base,
        "/api/tokens",
        &json!({ "name": "revoked" }),
        &admin,
    );
    let revoked = revocable["token"].as_str().expect("token").to_string();
    let revoked_id = revocable["id"].as_str().expect("id").to_string();
    server.delete_with_bearer(&format!("/api/tokens/{revoked_id}"), &admin);

    let (expired_body, expired_status) =
        get_json_status_bearer(&server.base, "/api/projects", &expired);
    let (revoked_body, revoked_status) =
        get_json_status_bearer(&server.base, "/api/projects", &revoked);

    assert_eq!(expired_status, 401, "an expired token was accepted");
    assert_eq!(revoked_status, 401, "a revoked token was accepted");
    assert_eq!(
        expired_status, revoked_status,
        "expiry and revocation differ in status, which is a probe"
    );
    assert_eq!(
        expired_body, revoked_body,
        "expiry and revocation differ in body, which is a probe"
    );
}

// ---------------------------------------------------------------------------
// T045 / SC-444, FR-560, FR-574 — the last-admin guarantee under real concurrency
// ---------------------------------------------------------------------------

/// Two concurrent demotions of the two remaining administrators: exactly one
/// succeeds, exactly one is refused, and the server keeps an administrator.
///
/// Issued from two threads against one live database, not reasoned about from an
/// isolation level. The anomaly this guards is **write skew**: the two
/// transactions touch different rows, so no row lock makes either wait, and both
/// can observe the other admin as still active. A `SELECT count(*)` followed by
/// an `UPDATE` — even inside one transaction — passes the sequential version of
/// this test and fails this one.
///
/// Run against a server whose environment names **no** account, so the two
/// racing administrators are the only two the guard can see. On a server with an
/// environment admin there would always be a third, and both demotions would be
/// legal — the race would prove nothing.
#[test]
fn two_concurrent_demotions_of_the_last_two_admins_yield_one_winner() {
    // Its own database: this test rewrites every account's role, which would
    // corrupt any test sharing the database — and would let their accounts
    // corrupt this fixture, so that the race demoted rows it had never seen.
    let Some(server) = Server::start_own_database() else {
        eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the server suite");
        return;
    };
    let actor = server.new_user_token("racer");
    server.new_user_token("bystander");

    // Exactly two active administrators, and the caller is one of them — an
    // admin demoting itself is legal when it is not the last one, which is the
    // point of the guard's `id <> target` predicate.
    server.execute("UPDATE users SET role = 'member'");
    let ids = server.query_column("SELECT id::text FROM users ORDER BY created_at");
    assert_eq!(
        ids.len(),
        2,
        "this race needs exactly two accounts on a database of its own; found {}",
        ids.len()
    );
    server.execute("UPDATE users SET role = 'admin'");
    assert_eq!(
        server.count("SELECT count(*) FROM users WHERE role = 'admin' AND status = 'active'"),
        2,
        "the fixture must start with exactly two administrators"
    );

    // Both requests are held at a barrier so they reach the server together.
    // Sending them sequentially would test the ordinary refusal path, which is a
    // different and much weaker claim.
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [ids[0].clone(), ids[1].clone()]
        .into_iter()
        .map(|target| {
            let base = server.base.clone();
            let token = actor.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                patch_json_bearer(
                    &base,
                    &format!("/api/admin/users/{target}"),
                    &json!({ "role": "member" }),
                    &token,
                )
                .1
            })
        })
        .collect();
    let statuses: Vec<u16> = handles
        .into_iter()
        .map(|h| h.join().expect("thread"))
        .collect();

    let successes = statuses.iter().filter(|s| **s == 200).count();
    let refusals = statuses.iter().filter(|s| **s != 200).count();
    assert_eq!(
        (successes, refusals),
        (1, 1),
        "expected exactly one success and one refusal; statuses were {statuses:?}. \
         Two successes means the guard is a read followed by a write (write skew); \
         two refusals means a legal demotion was blocked by an operation that itself \
         failed (FR-560)"
    );

    // The loser's *code* is deliberately not asserted here, and the reason is
    // worth stating rather than leaving as a loose assertion.
    //
    // The caller has to be one of the two administrators — `AdminUser` admits
    // nobody else, and with a third admin present both demotions would be legal
    // and the guard would never fire. So the two orderings produce two different
    // refusals, both correct: if the caller's own demotion commits first it is a
    // member by the time its second request is authorized and gets `403`; if the
    // other demotion commits first the guard finds no remaining admin and returns
    // `409 last_admin`. Pinning one code here would make the test fail on a
    // legitimate interleaving. What FR-560 actually promises is that exactly one
    // takes effect — which is asserted above — and that an administrator
    // survives, which is asserted below. The guard's own `409` is covered by
    // name in the sequential test that follows.
    assert_eq!(
        server.count("SELECT count(*) FROM users WHERE role = 'admin' AND status = 'active'"),
        1,
        "the server lost its last administrator"
    );
}

/// The guard's own refusal, by name, without a race.
///
/// Companion to the test above: that one proves exactly one of two concurrent
/// demotions takes effect, and deliberately does not pin the loser's status code
/// because two legitimate interleavings produce two different ones. This one
/// pins the code, because with no concurrency there is only one path — the
/// `EXISTS` subquery finds no other active administrator and the statement
/// matches zero rows (FR-413, SC-437).
#[test]
fn demoting_the_last_administrator_is_refused_by_name() {
    let Some(server) = Server::start_own_database() else {
        eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the server suite");
        return;
    };
    let actor = server.new_user_token("sole-admin");
    server.execute("UPDATE users SET role = 'member'");
    let ids = server.query_column("SELECT id::text FROM users ORDER BY created_at");
    assert_eq!(ids.len(), 1, "this test needs exactly one account");
    server.execute("UPDATE users SET role = 'admin'");

    let (body, status) = patch_json_bearer(
        &server.base,
        &format!("/api/admin/users/{}", ids[0]),
        &json!({ "role": "member" }),
        &actor,
    );
    assert_eq!(status, 409, "the last administrator was demoted: {body}");
    assert_eq!(body["error"]["code"], "last_admin", "{body}");
    assert_eq!(
        server.count("SELECT count(*) FROM users WHERE role = 'admin' AND status = 'active'"),
        1,
        "the demotion took effect despite being refused"
    );

    // Disabling is the same guard, same shape — asserted because it is a
    // separate `SET` clause and a regression could reach one and not the other.
    let (body, status) = patch_json_bearer(
        &server.base,
        &format!("/api/admin/users/{}", ids[0]),
        &json!({ "status": "disabled" }),
        &actor,
    );
    assert_eq!(status, 409, "the last administrator was disabled: {body}");
    assert_eq!(body["error"]["code"], "last_admin", "{body}");
}

// ---------------------------------------------------------------------------
// T051 / SC-433, SC-434, SC-437 — the environment account is the break-glass path
// ---------------------------------------------------------------------------

/// Demoting the last non-environment admin is refused at runtime, and the
/// environment account cannot be demoted or disabled at all.
#[test]
fn the_environment_account_refuses_demotion_and_names_the_setting() {
    let Some(server) = server() else { return };
    let admin = admin_token(&server);
    let (listing, _) = get_json_status_bearer(&server.base, "/api/admin/users", &admin);
    let env_id = listing["users"]
        .as_array()
        .expect("users")
        .iter()
        .find(|u| u["email"].as_str() == Some(ADMIN_EMAIL))
        .and_then(|u| u["id"].as_str())
        .expect("the environment account exists")
        .to_string();

    for change in [json!({ "role": "member" }), json!({ "status": "disabled" })] {
        let (body, status) = patch_json_bearer(
            &server.base,
            &format!("/api/admin/users/{env_id}"),
            &change,
            &admin,
        );
        assert_eq!(status, 409, "{change} was accepted: {body}");
        assert_eq!(body["error"]["code"], "environment_account", "{body}");
        // The refusal names the setting, because the operator's next move is to
        // edit it — and a refusal that does not say which variable leaves them
        // guessing at the one account whose configuration is not in the database.
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("CAIRN_ADMIN_EMAIL")),
            "the refusal does not name the environment setting: {body}"
        );
    }

    // And it is observably unchanged afterwards (SC-434).
    let (after, _) = get_json_status_bearer(&server.base, "/api/admin/users", &admin);
    let env = after["users"]
        .as_array()
        .expect("users")
        .iter()
        .find(|u| u["email"].as_str() == Some(ADMIN_EMAIL))
        .expect("still there");
    assert_eq!(env["role"], "admin");
    assert_eq!(env["status"], "active");
}

/// A restart restores administration to the environment-named account even after
/// its role and status were corrupted outside the supported API (SC-433).
///
/// The break-glass path: an operator who has lost administration some other way
/// recovers by restarting, with no database surgery and no reinstallation.
#[test]
fn a_restart_restores_the_environment_account_from_corrupted_state() {
    let Some(mut server) = server() else { return };
    // Corrupt it the way only direct database access can — no API permits this,
    // which is exactly why the recovery path exists.
    server.execute(&format!(
        "UPDATE users SET role = 'member', status = 'disabled' WHERE email = '{ADMIN_EMAIL}'"
    ));
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM users WHERE email = '{ADMIN_EMAIL}' AND role = 'admin'"
        )),
        0,
        "the corruption did not take, so the recovery below proves nothing"
    );

    let restarted = server.restarted_with_admin(ADMIN_EMAIL, ADMIN_PASSWORD);
    assert_eq!(
        restarted.count(&format!(
            "SELECT count(*) FROM users
              WHERE email = '{ADMIN_EMAIL}' AND role = 'admin' AND status = 'active'"
        )),
        1,
        "a restart did not restore the environment account's authority (FR-539)"
    );
    // And it does not owe a password change, which would be an unbreakable loop:
    // the environment re-applies the password on every start, so a forced change
    // would be reverted by the next restart (FR-540).
    assert_eq!(
        restarted.count(&format!(
            "SELECT count(*) FROM users WHERE email = '{ADMIN_EMAIL}' AND must_change_password"
        )),
        0,
        "the environment account owes a password change it can never satisfy"
    );
}

// ---------------------------------------------------------------------------
// T052, T053 / SC-442, SC-443 — administrator password reset
// ---------------------------------------------------------------------------

/// A reset issues a new one-time password, kills the old one, and refuses every
/// token the account held (SC-442).
#[test]
fn a_reset_replaces_the_password_and_refuses_every_token_the_account_held() {
    let Some(server) = server() else { return };
    let admin = admin_token(&server);
    let (id, temporary) = create_account(&server, &admin, "forgetful");
    let (listing, _) = get_json_status_bearer(&server.base, "/api/admin/users", &admin);
    let email = listed_email(&listing, id);

    let session = server.cookie_for_password(&email, &temporary);
    server.post_with_cookie(
        "/api/auth/password",
        &json!({ "new_password": "chosen-password-1" }),
        &session,
    );
    let after = server.cookie_for_password(&email, "chosen-password-1");
    let (minted, _) = server.post_with_cookie("/api/tokens", &json!({ "name": "held" }), &after);
    let held = minted["token"].as_str().expect("token").to_string();

    let (reset, status) = post_json_status_bearer(
        &server.base,
        &format!("/api/admin/users/{id}/reset-password"),
        &json!({}),
        &admin,
    );
    assert_eq!(status, 200, "reset refused: {reset}");
    let fresh = reset["temporary_password"]
        .as_str()
        .expect("password")
        .to_string();
    assert_ne!(fresh, temporary, "the reset reissued the original password");

    // The old password fails immediately (FR-555).
    assert!(
        server
            .try_cookie_for_password(&email, "chosen-password-1")
            .is_none(),
        "the previous password still authenticates after a reset"
    );
    // Every token it held is refused (FR-556).
    assert_eq!(
        server.get_status("/api/projects", &held),
        401,
        "a token the account held survived the reset"
    );
    // The new one authenticates only to the change route (FR-557).
    let reset_session = server.cookie_for_password(&email, &fresh);
    let (refused, status) =
        server.post_with_cookie("/api/tokens", &json!({ "name": "x" }), &reset_session);
    assert_eq!(
        status, 403,
        "the reset credential minted a token: {refused}"
    );
    assert_eq!(refused["error"]["code"], "password_change_required");
}

/// Resetting a disabled account's password leaves it disabled (FR-558, SC-443).
///
/// A reset is a credential operation and re-enabling is an authority operation.
/// Conflating them means an administrator clearing a forgotten password silently
/// readmits an account they disabled on purpose.
#[test]
fn resetting_a_disabled_accounts_password_leaves_it_disabled() {
    let Some(server) = server() else { return };
    let admin = admin_token(&server);
    let (id, _) = create_account(&server, &admin, "suspended");
    let (listing, _) = get_json_status_bearer(&server.base, "/api/admin/users", &admin);
    let email = listed_email(&listing, id);

    patch_json_bearer(
        &server.base,
        &format!("/api/admin/users/{id}"),
        &json!({ "status": "disabled" }),
        &admin,
    );
    let (reset, status) = post_json_status_bearer(
        &server.base,
        &format!("/api/admin/users/{id}/reset-password"),
        &json!({}),
        &admin,
    );
    assert_eq!(status, 200, "the reset itself should succeed: {reset}");
    let fresh = reset["temporary_password"]
        .as_str()
        .expect("password")
        .to_string();

    assert!(
        server.try_cookie_for_password(&email, &fresh).is_none(),
        "the reset re-admitted a disabled account"
    );
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM users WHERE id = '{id}' AND status = 'disabled'"
        )),
        1,
        "the account is no longer disabled"
    );
}

// ---------------------------------------------------------------------------
// Authorization boundaries
// ---------------------------------------------------------------------------

/// Every admin route refuses a member, and refuses an unauthenticated caller.
///
/// Exercised as a set rather than one route, so a route added later that forgot
/// its `AdminUser` extractor is caught by the same test.
#[test]
fn every_admin_route_refuses_a_member_and_an_anonymous_caller() {
    let Some(server) = server() else { return };
    let admin = admin_token(&server);
    let (target, temporary) = create_account(&server, &admin, "ordinary");
    let (listing, _) = get_json_status_bearer(&server.base, "/api/admin/users", &admin);
    let email = listed_email(&listing, target);

    // A settled, ordinary member.
    let session = server.cookie_for_password(&email, &temporary);
    server.post_with_cookie(
        "/api/auth/password",
        &json!({ "new_password": "ordinary-password-1" }),
        &session,
    );
    let settled = server.cookie_for_password(&email, "ordinary-password-1");
    let (minted, _) = server.post_with_cookie("/api/tokens", &json!({ "name": "m" }), &settled);
    let member = minted["token"].as_str().expect("token").to_string();

    assert_eq!(
        server.get_status("/api/admin/users", &member),
        403,
        "a member listed every account"
    );
    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/admin/users",
        &json!({ "email": "sneak@example.test", "display_name": "Sneak" }),
        &member,
    );
    assert_eq!(status, 403, "a member created an account: {created}");
    assert_eq!(
        server.count("SELECT count(*) FROM users WHERE email = 'sneak@example.test'"),
        0,
        "an account was created by a non-administrator"
    );
    let (patched, status) = patch_json_bearer(
        &server.base,
        &format!("/api/admin/users/{target}"),
        &json!({ "role": "admin" }),
        &member,
    );
    assert_eq!(status, 403, "a member promoted itself: {patched}");
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM users WHERE id = '{target}' AND role = 'admin'"
        )),
        0,
        "a member became an administrator"
    );

    // Unauthenticated.
    assert_eq!(server.get_status("/api/admin/users", "not-a-token"), 401);
}

// ---------------------------------------------------------------------------
// T055 / SC-405 — see `migration_alpha5.rs`
// ---------------------------------------------------------------------------

/// The role backfill's four seeded configurations are covered in
/// `tests/tests/migration_alpha5.rs`, next to the migration they exercise:
/// `the_role_backfill_always_leaves_exactly_one_admin`,
/// `a_single_legacy_account_becomes_the_administrator`,
/// `an_empty_server_migrates_without_inventing_an_account`,
/// `the_environment_named_account_wins_over_the_oldest`, and
/// `an_environment_email_matching_nothing_falls_back_to_the_oldest`.
///
/// This marker exists so a reader of T055 finds them rather than concluding they
/// are missing.
#[test]
fn t055_role_backfill_lives_with_the_migration_it_tests() {
    // Intentionally trivial: the assertion is the file it points at.
}

// ---------------------------------------------------------------------------
// T199 / FR-543 / SC-435 — the trust statement is in the shipped documentation
// ---------------------------------------------------------------------------

/// `README.md` and `SECURITY.md` both state who can ultimately obtain
/// administrator access, and why.
///
/// SC-435 says a reader can answer that question "without reading the source".
/// A documentation task that writes the sentence cannot verify that — it is
/// satisfied the moment the sentence exists, whether or not the sentence says
/// anything. So this asserts both halves separately: the **mechanism** (the
/// environment-named account, restored on every start) and the **reason** (whoever
/// controls the host controls the environment). Either alone leaves the reader
/// with half an answer, and the half they are missing is the one that tells them
/// whether it matters for their deployment.
///
/// Falsified by softening either document to a statement of policy without the
/// mechanism, which is the natural way this regresses: "administrators are managed
/// by the operator" is true, reassuring, and useless.
#[test]
fn the_shipped_documentation_states_who_can_obtain_administrator_access_and_why() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");

    for name in ["README.md", "SECURITY.md"] {
        let text = std::fs::read_to_string(root.join(name)).unwrap_or_else(|e| {
            panic!("{name} is not readable, so the shipped documentation cannot be checked: {e}")
        });
        let lowered = text.to_ascii_lowercase();

        // The mechanism.
        assert!(
            lowered.contains("cairn_admin_email"),
            "{name} does not name the environment setting that decides who is an \
             administrator (FR-543)"
        );
        assert!(
            lowered.contains("every start")
                || lowered.contains("on start")
                || lowered.contains("restart"),
            "{name} does not say that the environment-named account is restored when \
             the server starts, which is the whole mechanism"
        );
        assert!(
            lowered.contains("admin") && lowered.contains("active"),
            "{name} does not say what the account is restored *to* — both its role \
             and its status matter, since a restored admin that stayed disabled \
             would not be a break-glass path at all"
        );

        // The reason.
        assert!(
            lowered.contains("environment")
                && (lowered.contains("whoever") || lowered.contains("anyone who")),
            "{name} does not state the consequence — that whoever can set the \
             environment and restart the process can always obtain administrator \
             access (SC-435)"
        );
    }
}
