//! Every Feature 005 project-scoped route is guarded, enumerated rather than
//! sampled (T038, FR-769, FR-894a).
//!
//! An audit, not a set of examples. A test naming three routes keeps passing
//! the day a fourth is added without a guard, which is exactly the failure this
//! file exists to prevent — so the sweeps below walk the route set as it
//! actually is and fail on anything in it that is not guarded.
//!
//! Two properties, and they are different:
//!
//! - **Membership decides access.** A project-scoped route answers a non-member
//!   with a refusal, never an empty result (FR-057), and never with data.
//! - **Identity is never read from the body** (FR-769). A route that accepted
//!   `owner_user_id` or `origin_session_id` from a request could attribute one
//!   account's work to another, which is the same class of defect as a
//!   falsified session on an event.
//!
//! The static half of the audit reads `api.rs` and `commands.rs`. That is
//! deliberate: a live probe can only test the routes it thinks of, and the
//! point is to catch the one nobody thought of.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{get_json_status_bearer, post_json_status_bearer, post_status_bearer};
use serde_json::{json, Value};
use uuid::Uuid;

macro_rules! pg {
    () => {
        match Pg::start() {
            Some(pg) => pg,
            None => {
                eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

fn source(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// The static inventory
// ---------------------------------------------------------------------------

/// Every Feature 005 handler that reaches project data, and which of the two
/// membership guards it must carry.
///
/// A handler reaches project data one of two ways, and the two need *different*
/// guards because they disclose different things:
///
/// - **The path names the project.** `require_member`, whose refusal is `403`.
///   The caller supplied the project id, so refusing by name discloses nothing
///   they did not already know.
/// - **The path names a record.** `project_of_record`, whose refusal is `404`
///   and is identical to the answer a missing record gets. Here whether the
///   record exists is precisely what must not leak, so a `403` would be an
///   enumeration oracle (FR-894a).
///
/// Getting these the wrong way round is not a style question: this audit caught
/// exactly that, with `reinforce` answering `403` for a memory that existed and
/// `404` for one that did not.
const PROJECT_ADDRESSED: &[&str] = &["create_memory", "record_relation"];
const RECORD_ADDRESSED: &[&str] = &["supersede_memory", "reinforce_memory", "pin_memory"];

/// One handler's body, up to the next top-level item.
fn handler_body<'a>(source: &'a str, handler: &str) -> &'a str {
    let start = source
        .find(&format!("pub async fn {handler}("))
        .unwrap_or_else(|| panic!("{handler} is gone; update this audit deliberately"));
    let body = &source[start..];
    let end = body[1..]
        .find("\npub async fn ")
        .map(|i| i + 1)
        .unwrap_or(body.len());
    &body[..end]
}

#[test]
fn every_handler_reaching_project_data_carries_the_guard_that_fits_it() {
    let commands = source("crates/cairn-server/src/commands.rs");
    for handler in PROJECT_ADDRESSED {
        assert!(
            handler_body(&commands, handler).contains("require_member("),
            "{handler} names its project and does not call require_member"
        );
    }
    for handler in RECORD_ADDRESSED {
        let body = handler_body(&commands, handler);
        assert!(
            body.contains("project_of_record("),
            "{handler} names a record and does not resolve membership through \
             project_of_record"
        );
        assert!(
            !body.contains("require_member("),
            "{handler} names a record but refuses with require_member's 403, \
             which tells a non-member whether the record exists"
        );
    }
}

/// A record-addressed **read** gives one answer too.
///
/// The audit above scans `commands.rs`, because that is where the
/// record-addressed mutations live. `GET /api/memories/{id}` is a read in
/// `api.rs` and sat outside the scan — it used `require_member`, so a memory in
/// somebody else's project answered `403` and a memory that does not exist
/// answered `404`, which sorts ids into real and imaginary for anyone with an
/// account. Asserted here by probing the live route rather than by widening the
/// string scan, because the rule is about the answer and not about which
/// function produced it.
#[test]
fn a_record_addressed_read_does_not_say_whether_the_record_exists() {
    let pg = pg!();
    let memory = seed_memory(&pg);
    let imaginary = Uuid::now_v7();

    let (real, real_status) = get_json_status_bearer(
        &pg.server.base,
        &format!("/api/memories/{memory}"),
        &pg.outsider.token,
    );
    let (absent, absent_status) = get_json_status_bearer(
        &pg.server.base,
        &format!("/api/memories/{imaginary}"),
        &pg.outsider.token,
    );
    assert_eq!(
        real_status, absent_status,
        "a non-member learns whether a memory exists from the status alone: \
         {real_status} for a real one, {absent_status} for an invented one"
    );
    assert_eq!(
        real["error"]["code"], absent["error"]["code"],
        "the refusal codes differ, which is the same oracle in the body: \
         {real} vs {absent}"
    );
    assert_eq!(
        real_status, 404,
        "the shared answer should be the blunt one"
    );
}

#[test]
fn the_record_addressed_guard_gives_one_answer_to_both_questions() {
    // The static half of the oracle check: `project_of_record` must not have
    // two exits a caller could tell apart.
    let commands = source("crates/cairn-server/src/commands.rs");
    let body = {
        let start = commands
            .find("async fn project_of_record(")
            .expect("project_of_record is gone");
        let rest = &commands[start..];
        let end = rest[1..]
            .find("\n// ----")
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        &rest[..end]
    };
    assert!(
        body.contains("let hidden = ||"),
        "project_of_record no longer funnels its refusals through one answer"
    );
    assert!(
        !body.contains("forbidden("),
        "project_of_record has a distinguishable refusal, which is an \
         enumeration oracle"
    );
}

#[test]
fn no_feature_005_handler_reads_identity_out_of_a_request_body() {
    // The guard is `reject_server_owned`, which refuses the credential-bound
    // names outright. This asserts every command handler runs it — a handler
    // that skipped it could accept `owner_user_id` and write on somebody
    // else's behalf.
    let commands = source("crates/cairn-server/src/commands.rs");
    let handlers: Vec<&str> = commands
        .match_indices("pub async fn ")
        .map(|(i, _)| {
            let rest = &commands[i + "pub async fn ".len()..];
            &rest[..rest.find('(').unwrap_or(0)]
        })
        .collect();
    assert!(handlers.len() >= 9, "the audit found no handlers to check");

    let mut screened = 0;
    for handler in handlers {
        let start = commands.find(&format!("pub async fn {handler}(")).unwrap();
        let body = &commands[start..];
        let end = body[1..]
            .find("\npub async fn ")
            .map(|i| i + 1)
            .unwrap_or(body.len());
        let body = &body[..end];
        let signature = &body[..body.find(" -> ApiResult").unwrap_or(body.len())];
        // **A handler with no request body has nothing to screen**, and US3's
        // pattern list is the first of those: a `GET` route whose whole input is
        // the credential cannot be made to name an identity, because there is no
        // field in which to name one. Requiring the call anyway would mean
        // writing a screen over an argument that does not exist, which is how an
        // audit teaches people to add a line to satisfy it.
        //
        // The exemption is decided from the signature rather than from a list of
        // handler names, so a new route is covered or exempt by what it actually
        // accepts — a list would have to be remembered.
        if !signature.contains("Json(") {
            continue;
        }
        screened += 1;
        // Matched on the call, not on a variable name. `command_envelope`
        // screens `&envelope` rather than `&body` — it checks the whole
        // envelope, so a field named outside `payload` is refused too — and an
        // audit keyed to one spelling would have reported that as a missing
        // guard.
        assert!(
            body.contains("reject_server_owned(&"),
            "{handler} does not screen its input for server-owned fields, so a \
             client could name an identity or assert a derived value"
        );
    }
    assert!(
        screened >= 9,
        "the audit exempted almost everything; {screened} handlers were actually \
         checked, which means the signature rule above is matching nothing"
    );
}

/// The source with its Rust comment lines removed.
///
/// The scan below finds the SQL statement around an occurrence by walking out
/// to the surrounding quotes, which works on code and not on prose: a doc
/// comment that *mentions* `shared_patterns` sits between two string literals,
/// so the walk swallows the next constant and the audit reports a statement
/// that never contained the word. Removing comment lines first means every
/// remaining occurrence is one the compiler sees too.
///
/// This narrows what the audit reads, not what it requires. A comment cannot
/// reach a row, and the `found >= 5` floor below is what proves the narrowing
/// did not also hide a real statement.
fn strip_rust_comments(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A pattern is owner-only, and every route that can reach one says so in the
/// query (T090, FR-708d, SC-761).
///
/// The static half. A live probe is in `feature005_patterns.rs`; this exists
/// because a probe can only test the routes it thinks of, and the failure being
/// guarded against is a *new* route that forgets the filter.
#[test]
fn every_pattern_query_is_bound_to_the_owning_account() {
    // **Every server source file, not just `commands.rs`.** The rule is about
    // where a pattern can be read from, and a pattern can be read from anywhere
    // that writes SQL — retrieval builds candidates from it, the reference
    // authorization check resolves one, and the changes feed pages over it. An
    // audit scoped to the module where the routes happen to live today would
    // pass on the day one of them moves.
    let sources: Vec<String> = std::fs::read_dir(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join("crates/cairn-server/src"),
    )
    .expect("the server crate's sources")
    .filter_map(|e| e.ok())
    .map(|e| e.path())
    .filter(|p| p.extension().is_some_and(|x| x == "rs"))
    .map(|p| strip_rust_comments(&std::fs::read_to_string(&p).expect("read")))
    .collect();
    let commands = sources.join("\n");
    let mut found = 0;
    for (i, _) in commands.match_indices("shared_patterns") {
        // The statement this occurrence belongs to: back to the opening quote of
        // the SQL literal, forward to its close.
        let start = commands[..i].rfind('"').unwrap_or(0);
        let end = commands[i..]
            .find("\",")
            .map(|j| i + j)
            .unwrap_or(commands.len());
        let statement = &commands[start..end];
        // A `CREATE`/comment mention is not a query. Only statements that read
        // or write rows have an owner to bind.
        let upper = statement.to_uppercase();
        if !upper.contains("SELECT") && !upper.contains("UPDATE") && !upper.contains("INSERT") {
            continue;
        }
        found += 1;
        assert!(
            statement.contains("owner_user_id"),
            "a `shared_patterns` statement does not name owner_user_id, so it \
             can reach another account's pattern:\n{statement}"
        );
    }
    assert!(
        found >= 5,
        "the audit found {found} pattern statements; promotion, the list, the \
         changes feed, retrieval's candidate query and the reference \
         authorization check are at least five, so the scan is matching nothing"
    );
}

#[test]
fn the_retrieval_routes_bind_every_identity_from_the_session_or_the_trace() {
    // The audit exists so a *new* endpoint cannot slip past the rule, and US2
    // added three. Retrieval is where the rule matters most: the answer spans
    // project, personal and team knowledge, so a caller that could name an
    // account or a project in the body could read across all three.
    let retrieve = source("crates/cairn-server/src/retrieve.rs");

    // Retrieval derives its project from a verified session, exactly as ingest
    // does.
    assert!(
        retrieve.contains("bind_session("),
        "retrieval no longer derives its project from a verified session"
    );

    // The request types are closed. `deny_unknown_fields` is what turns "the
    // server ignores an extra field" into "the server refuses it" — an ignored
    // `account_id` reads to the caller exactly like an accepted one.
    let closed = retrieve.matches("#[serde(deny_unknown_fields)]").count();
    assert!(
        closed >= 2,
        "a retrieval request type is open, so a caller can send fields the          server silently ignores"
    );

    // None of the identity-bearing names may be read from a body at all.
    for forbidden in [
        "body.account_id",
        "body.project_id",
        "body.owner_user_id",
        "body.session_owner",
        "report.account_id",
        "report.project_id",
        "report.session_id",
    ] {
        assert!(
            !retrieve.contains(forbidden),
            "retrieval reads {forbidden} out of a request body"
        );
    }

    // The transmission report's whole surface is an outcome and a bounded
    // reason. Anything else is authority a caller must not assert — including
    // acknowledgement, which no vendor mechanism establishes for any agent.
    let start = retrieve
        .find("pub struct TransmissionReport {")
        .expect("the transmission report type exists");
    let report = &retrieve[start..start + retrieve[start..].find('}').unwrap_or(0)];
    for forbidden in [
        "account",
        "project",
        "session",
        "acknowledg",
        "reference",
        "domain",
    ] {
        assert!(
            !report.contains(forbidden),
            "the transmission report carries a {forbidden} field, which is              authority the caller must not be able to assert"
        );
    }
}

#[test]
fn the_ingest_route_binds_its_project_from_the_session_and_not_the_body() {
    let events = source("crates/cairn-server/src/events.rs");
    // The project is derived from the verified session. A route that took it
    // from the body would let a caller attribute events to a project they have
    // nothing to do with (FR-769).
    assert!(
        events.contains("bind_session("),
        "ingest no longer derives its project from a verified session"
    );
    assert!(
        !events.contains("body.project_id") && !events.contains("\"project_id\""),
        "ingest reads a project id out of the request body"
    );
}

// ---------------------------------------------------------------------------
// The live sweep
// ---------------------------------------------------------------------------

/// Every Feature 005 project-scoped endpoint, with a body that would succeed
/// for a member.
fn project_scoped_requests(pg: &Pg, memory: Uuid) -> Vec<(String, Value)> {
    vec![
        (
            format!("/api/projects/{}/memories", pg.project),
            json!({ "type": "fact", "scope": "project", "content": "x" }),
        ),
        (
            format!("/api/projects/{}/memory-relations", pg.project),
            json!({ "from_memory_id": memory, "to_memory_id": memory, "kind": "reinforces" }),
        ),
        (
            format!("/api/memories/{memory}/supersede"),
            json!({ "type": "fact", "scope": "project", "content": "x" }),
        ),
        (format!("/api/memories/{memory}/reinforce"), json!({})),
        (
            format!("/api/memories/{memory}/pin"),
            json!({ "pinned": true }),
        ),
        // A batch with a real event in it. An *empty* batch names no session,
        // so there is nothing to authorize against and answering it 200 is
        // correct — which makes it useless as a test of the guard.
        (
            "/api/events/batch".to_string(),
            json!({ "contract_version": 1, "events": [session_event(pg)] }),
        ),
    ]
}

/// One legal event naming a session in the fixture project.
fn session_event(pg: &Pg) -> Value {
    let session = pg.session_for(&pg.owner);
    json!({
        "event_id": cairn_core::eventid::event_id(session, 1),
        "contract_version": 1,
        "kind": "file_read",
        "agent": "claude_code",
        "vendor_event": null,
        "session_id": session,
        "session_seq": 1,
        "occurred_at": "2026-09-02T10:00:00Z",
        "content": { "File": {
            "repo_file": "a.rs", "repo_file_from": null,
            "change_kind": null, "file_identity": "present"
        }},
    })
}

fn seed_memory(pg: &Pg) -> Uuid {
    let id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content,
                               origin_session_id)
         VALUES ('{id}', '{}', 'fact', 'project', '{}', 'a claim',
                 '00000000-0000-0000-0000-000000000000')",
        pg.project, pg.project
    ));
    id
}

#[test]
fn a_non_member_is_refused_by_every_project_scoped_endpoint() {
    let pg = pg!();
    let memory = seed_memory(&pg);
    for (path, body) in project_scoped_requests(&pg, memory) {
        let code = post_status_bearer(&pg.server.base, &path, &body, &pg.outsider.token);
        // 403 where the caller named the project — they already knew it, so a
        // refusal discloses nothing. 404 where the caller named a *record* —
        // there, whether it exists is precisely what must not leak, so the
        // answer matches the one a missing record gets (FR-894a).
        assert!(
            code == 403 || code == 404,
            "{path} answered a non-member {code}; a refusal is the only correct \
             answer, and an empty result is not one (FR-057)"
        );
    }
}

#[test]
fn an_unauthenticated_caller_is_refused_by_every_project_scoped_endpoint() {
    let pg = pg!();
    let memory = seed_memory(&pg);
    for (path, body) in project_scoped_requests(&pg, memory) {
        let code = post_status_bearer(&pg.server.base, &path, &body, "not-a-real-token");
        assert_eq!(
            code, 401,
            "{path} answered an unauthenticated caller {code}"
        );
    }
}

#[test]
fn a_non_member_learns_nothing_from_the_shape_of_the_refusal() {
    let pg = pg!();
    let real = seed_memory(&pg);
    let imaginary = Uuid::now_v7();
    // A different answer for a record that exists and one that does not would
    // let anyone with an account enumerate record ids across the whole server,
    // one guess at a time (FR-894a).
    for path in ["reinforce", "pin"] {
        let existing = post_status_bearer(
            &pg.server.base,
            &format!("/api/memories/{real}/{path}"),
            &json!({}),
            &pg.outsider.token,
        );
        let absent = post_status_bearer(
            &pg.server.base,
            &format!("/api/memories/{imaginary}/{path}"),
            &json!({}),
            &pg.outsider.token,
        );
        assert_eq!(
            existing, absent,
            "/{path} tells an outsider whether a memory exists: {existing} vs {absent}"
        );
    }
}

#[test]
fn personal_and_team_routes_are_account_scoped_rather_than_project_scoped() {
    let pg = pg!();
    // These are project-independent, so `require_member` is the wrong guard for
    // them and its absence is correct. What guards them is ownership, which is
    // asserted in `feature005_commands.rs`; what is asserted here is that they
    // are not silently reachable without authentication.
    for (path, body) in [
        (
            "/api/personal/knowledge",
            json!({ "knowledge_type": "fact", "content": "x" }),
        ),
        (
            "/api/team/knowledge",
            json!({ "knowledge_type": "fact", "content": "x" }),
        ),
        (
            "/api/patterns",
            json!({ "title": "t", "problem": "p", "root_cause": "r", "approach": "a" }),
        ),
    ] {
        assert_eq!(
            post_status_bearer(&pg.server.base, path, &body, "not-a-real-token"),
            401,
            "{path} is reachable without authentication"
        );
    }
}

// ---------------------------------------------------------------------------
// The migration and cutover routes (T161, `contracts/migration-cutover.md`)
// ---------------------------------------------------------------------------

// `POST /api/migration/register`, `/drain`, `/possession`, `/complete` and
// `POST /api/admin/cutover` are new surfaces this feature adds after the rest
// of the audit above was written, and every property the sweeps above exist
// to protect applies to them just as much:
//
// - **Migration is per-account, not per-project.** A drain call authenticated
//   as one account must not be able to deliver a record into a project that
//   account does not belong to merely by naming it in the payload, and a
//   migration token minted for one account must not authenticate a drain or
//   completion for another (`migration-cutover.md` §12.1) — the same
//   enumeration-oracle reasoning FR-894a applies to a record id applies here
//   to a token: a distinguishable "wrong account" answer would let a caller
//   learn which tokens exist.
// - **`indeterminate` is not `missing`.** §12.5 is explicit that a caller who
//   cannot see a team proposal must never be told the record does not exist,
//   because a client would act on `missing` by retaining a writable copy of
//   something the server may actually hold. Collapsing the third answer into
//   the second is a privacy leak that also corrupts the client's own state.
// - **Cutover is `AdminUser`-gated**, the same shape as every other
//   administrator action.
//
// None of these five routes is called anywhere in `web/` — there is no
// button that hides them from a member and no page that filters what a
// non-admin sees. The refusals below are produced by the routes themselves,
// probed directly over HTTP with no client in front of them, which is what
// "no API relies on web-side filtering" means in practice: there is no web
// side to rely on, and the guarantee has to come from the route.
//
// What would falsify this section: any one of these five routes answering
// `200` to a caller these tests deny it to, or `/possession` answering
// `missing` for a record `classify_possession` should have called
// `indeterminate`.

/// Promote an already-seeded account to administrator, directly — mirrors
/// `feature005_cutover.rs`'s own `make_admin`. Routing this through an admin
/// route would make the assertion depend on an admin already existing.
fn make_admin(pg: &Pg, who: &Account) {
    pg.server.execute(&format!(
        "UPDATE users SET role = 'admin' WHERE id = '{}'",
        who.id
    ));
}

/// Register a migration for `who`, returning its token.
fn register_migration(pg: &Pg, who: &Account, writer_id: &str) -> String {
    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/migration/register",
        &json!({ "writer_id": writer_id }),
        &who.token,
    );
    assert_eq!(status, 200, "registering a migration: {body}");
    body["migration_token"]
        .as_str()
        .unwrap_or_else(|| panic!("no migration_token in {body}"))
        .to_string()
}

/// A `team_knowledge` row still `proposed`, authored by `proposer` — the shape
/// `classify_possession` answers `indeterminate` for when the caller is
/// neither the author nor an admin (§5, §12.5).
fn seed_proposed_team_knowledge(pg: &Pg, proposer: &Account) -> Uuid {
    let id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO team_knowledge (id, knowledge_type, content, state,
                                      proposed_by_user_id, writer_id, writer_seq)
         VALUES ('{id}', 'convention', 'a proposal only its author can see',
                 'proposed', '{}', 'audit-fixture-{id}', 1)",
        proposer.id
    ));
    id
}

/// Every route unauthenticated traffic can reach with nothing but a fake
/// bearer token, none of them 200 — the same sweep the personal/team/pattern
/// routes above already get, extended to the five routes this section adds.
///
/// A request body shaped to *succeed* for a real account, deliberately: the
/// point is that authentication is checked before anything about the body is,
/// so an invalid token is refused the same way whether the body is perfect or
/// garbage. `SettledUser`/`AdminUser` are `axum` extractors that run ahead of
/// `Json<_>` in the handler's parameter list, so none of these bodies is ever
/// parsed for this call.
#[test]
fn migration_and_cutover_routes_are_never_reachable_without_authentication() {
    let pg = pg!();
    let real_memory = seed_memory(&pg);
    for (path, body) in [
        (
            "/api/migration/register".to_string(),
            json!({ "writer_id": "audit-unauth" }),
        ),
        (
            "/api/migration/drain".to_string(),
            json!({ "migration_token": "whatever", "items": [] }),
        ),
        (
            "/api/migration/possession".to_string(),
            json!({ "records": [{ "ref_kind": "knowledge", "domain": "project", "id": real_memory }] }),
        ),
        (
            "/api/migration/complete".to_string(),
            json!({ "migration_token": "whatever" }),
        ),
        ("/api/admin/cutover".to_string(), json!({})),
    ] {
        assert_eq!(
            post_status_bearer(&pg.server.base, &path, &body, "not-a-real-token"),
            401,
            "{path} is reachable without authentication"
        );
    }
}

/// A registered migration token authenticates the **account** it was
/// registered for, and nobody else's (`migration-cutover.md` §12.1).
///
/// Without this, one account's migration token would double as a bearer of
/// somebody else's authority: any account that could get hold of a token —
/// logged, guessed, or simply reused from a shared fixture — could drain
/// records under a different identity than the one that registered it. The
/// answer is the refusal `require_registered_migration` already gives an
/// unknown token, on purpose (see its own doc comment): distinguishing "your
/// token" from "a valid token, just not yours" would let a caller enumerate
/// which tokens exist for other accounts.
#[test]
fn drain_refuses_a_migration_token_registered_to_another_account() {
    let pg = pg!();
    let token = register_migration(&pg, &pg.owner, "audit-owner-store");

    for impostor in [&pg.member, &pg.outsider] {
        let (body, status) = post_json_status_bearer(
            &pg.server.base,
            "/api/migration/drain",
            &json!({
                "migration_token": token,
                "items": [],
            }),
            &impostor.token,
        );
        assert_eq!(
            status, 403,
            "{} drained using a token registered to a different account: {body}",
            impostor.email
        );
        assert_eq!(
            body["error"]["code"], "migration_not_registered",
            "the refusal for a token belonging to another account must be the \
             same one an unknown token gets, or a caller could tell the two \
             apart and enumerate live tokens: {body}"
        );
    }

    // The same token, presented by the account that actually registered it,
    // still works — the refusal above is about identity, not about the token
    // having gone bad.
    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/migration/drain",
        &json!({ "migration_token": token, "items": [] }),
        &pg.owner.token,
    );
    assert_eq!(status, 200, "the registering account's own drain: {body}");
}

/// The same cross-account refusal, for `/api/migration/complete`
/// (`migration-cutover.md` §12.1: the token "closes when the migration
/// completes", which only means anything if closing it is also bound to the
/// account that opened it).
#[test]
fn complete_refuses_a_migration_token_registered_to_another_account() {
    let pg = pg!();
    let token = register_migration(&pg, &pg.owner, "audit-owner-store-2");

    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/migration/complete",
        &json!({ "migration_token": token }),
        &pg.outsider.token,
    );
    assert_eq!(
        status, 403,
        "an outsider completed a migration token registered to another account: {body}"
    );
    assert_eq!(body["error"]["code"], "migration_not_registered", "{body}");
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM client_migrations
              WHERE migration_token = '{token}' AND completed_at IS NOT NULL"
        )),
        0,
        "a refused completion closed the token anyway"
    );
}

/// A non-member cannot deliver a record into a project it does not belong to
/// through `/api/migration/drain`, merely by naming that project's id in the
/// item payload (`migration-cutover.md` §4.2, FR-769's reasoning applied to
/// the migration-scoped ingest route rather than the ordinary one).
///
/// `drain_one` checks `auth::require_member` before either reused upsert
/// runs, for both `memory` and `memory_relation` — this asserts both, because
/// the two branches call the guard from two different match arms and a defect
/// in one would not show up testing only the other.
#[test]
fn a_non_member_cannot_drain_a_record_into_a_project_it_does_not_belong_to() {
    let pg = pg!();
    let token = register_migration(&pg, &pg.outsider, "audit-outsider-store");
    let memory_id = Uuid::now_v7();
    let relation_from = Uuid::now_v7();
    let relation_to = Uuid::now_v7();

    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/migration/drain",
        &json!({
            "migration_token": token,
            "items": [
                {
                    "entity_type": "memory",
                    "entity_id": memory_id,
                    "operation": "upsert",
                    "payload": { "project_id": pg.project },
                },
                {
                    "entity_type": "memory_relation",
                    "entity_id": Uuid::now_v7(),
                    "operation": "upsert",
                    "payload": {
                        "project_id": pg.project,
                        "from_memory_id": relation_from,
                        "to_memory_id": relation_to,
                        "kind": "reinforces",
                    },
                },
            ],
        }),
        &pg.outsider.token,
    );
    assert_eq!(
        status, 200,
        "a drain call answers per item, not as a batch: {body}"
    );

    let results = body["results"].as_array().cloned().unwrap_or_default();
    assert_eq!(results.len(), 2, "{body}");
    for result in &results {
        assert_eq!(
            result["accepted"],
            json!(false),
            "an outsider drained a record into a project it does not belong \
             to: {result}"
        );
    }

    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memories WHERE id = '{memory_id}'"
        )),
        0,
        "the drain reported the memory refused but wrote it anyway"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memory_relations
              WHERE from_memory_id = '{relation_from}' AND to_memory_id = '{relation_to}'"
        )),
        0,
        "the drain reported the relation refused but wrote it anyway"
    );
}

/// The same non-membership, asked of `/api/migration/possession` instead of
/// `/drain`: a project record the caller is not a member of is answered
/// `missing`, never `held` (`migration-cutover.md` §5).
///
/// This is the possession half of the same non-member boundary the test above
/// checks for drain — the two routes reach project data through different
/// code paths (`classify_possession` vs. `require_member` in `drain_one`), so
/// a defect in either is invisible to a test of only the other.
#[test]
fn possession_answers_missing_not_held_for_a_project_record_the_caller_is_not_a_member_of() {
    let pg = pg!();
    let memory = seed_memory(&pg);

    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/migration/possession",
        &json!({
            "records": [
                { "ref_kind": "knowledge", "domain": "project", "id": memory },
            ],
        }),
        &pg.outsider.token,
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["held"].as_array().map(Vec::len).unwrap_or(0),
        0,
        "a non-member's possession check reported a project record it does \
         not belong to as held: {body}"
    );
    assert_eq!(
        body["missing"].as_array().map(Vec::len).unwrap_or(0),
        1,
        "a project record the caller cannot see must answer `missing`, not \
         `indeterminate` — that answer is reserved for a proposed team entry, \
         where the caller genuinely cannot tell whether the server holds it \
         (§5): {body}"
    );
}

/// A team proposal the caller may not see answers `indeterminate`, never
/// `missing` (`migration-cutover.md` §5, §12.5).
///
/// §12.5 is explicit about why the distinction matters: `missing` is a claim
/// the server does not hold the record, and a client acting on that claim
/// would keep a writable local copy of something the server actually has —
/// two disagreeing "truths" for the same record. Collapsing `indeterminate`
/// into `missing` is not a smaller lie than collapsing it into `held`; it is
/// the specific lie this contract calls out by name.
#[test]
fn possession_answers_indeterminate_not_missing_for_a_team_proposal_the_caller_may_not_see() {
    let pg = pg!();
    let proposal = seed_proposed_team_knowledge(&pg, &pg.owner);
    let record = json!({ "ref_kind": "knowledge", "domain": "team", "id": proposal });

    // Neither the proposal's author nor an administrator: `member` is a
    // project peer of `owner` but has no standing over a team proposal it did
    // not author.
    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/migration/possession",
        &json!({ "records": [record] }),
        &pg.member.token,
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["missing"].as_array().map(Vec::len).unwrap_or(0),
        0,
        "a proposed team entry the caller may not see was reported `missing`, \
         which is the one answer §12.5 forbids for this exact case: {body}"
    );
    assert_eq!(
        body["held"].as_array().map(Vec::len).unwrap_or(0),
        0,
        "a proposal this caller did not author and is not an admin over was \
         reported `held`: {body}"
    );
    assert_eq!(
        body["indeterminate"],
        json!([record]),
        "an unreadable team proposal must answer `indeterminate`, with the \
         same reference object the caller sent: {body}"
    );

    // The control: the proposal's own author sees it as held.
    let (author_body, author_status) = post_json_status_bearer(
        &pg.server.base,
        "/api/migration/possession",
        &json!({ "records": [record] }),
        &pg.owner.token,
    );
    assert_eq!(author_status, 200, "{author_body}");
    assert_eq!(
        author_body["held"],
        json!([record]),
        "the proposal's own author must see it as held: {author_body}"
    );

    // The other control: an administrator sees it as held too, even though
    // they neither authored it nor are a member of any project it might be
    // scoped to — team knowledge has no project at all (§5's own table).
    make_admin(&pg, &pg.outsider);
    let (admin_body, admin_status) = post_json_status_bearer(
        &pg.server.base,
        "/api/migration/possession",
        &json!({ "records": [record] }),
        &pg.outsider.token,
    );
    assert_eq!(admin_status, 200, "{admin_body}");
    assert_eq!(
        admin_body["held"],
        json!([record]),
        "an administrator must be able to confirm possession of a proposal \
         they did not author: {admin_body}"
    );
}

/// `POST /api/admin/cutover` refuses a non-admin, which the rest of this file
/// already establishes as the shape every administrator action takes
/// (`AdminUser`) — restated here so this route appears in the same enumerated
/// audit as the rest of migration and cutover, rather than living only in
/// `feature005_cutover.rs`'s dedicated mode-transition coverage.
#[test]
fn admin_cutover_refuses_every_non_admin_account() {
    let pg = pg!();
    for who in [&pg.owner, &pg.member, &pg.outsider] {
        let (body, status) = post_json_status_bearer(
            &pg.server.base,
            "/api/admin/cutover",
            &json!({}),
            &who.token,
        );
        assert_eq!(
            status, 403,
            "{} was allowed to cut this deployment over without being an \
             administrator: {body}",
            who.email
        );
    }
    assert_eq!(
        pg.server
            .text("SELECT mode FROM server_authority WHERE id = 1"),
        "pre_cutover",
        "a refused cutover attempt changed `server_authority.mode` anyway"
    );
}
