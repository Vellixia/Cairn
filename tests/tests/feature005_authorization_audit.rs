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

use cairn_e2e::feature005::Pg;
use cairn_e2e::{get_json_status_bearer, post_status_bearer};
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
