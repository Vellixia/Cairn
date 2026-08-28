//! Reconciliation is consumed by the surfaces users read from (F2; FR-442,
//! FR-462, D431; T078, T127).
//!
//! Cairn records reconciliation decisions on every global write:
//! `classify_proposal` marks content-identical entries as duplicates and
//! disagreeing ones as conflicting, and a ratifying administrator may record an
//! explicit `supersedes`. All of that was true before this suite existed, and none
//! of it reached a reader: the only code that consumed a relation was
//! `personal_subject`/`team_subject`, which nothing called, and every canonical
//! read filtered on `state = 'authoritative'` alone.
//!
//! The visible consequence was that `cairn team ratify <new> --supersedes <old>`
//! succeeded and the replaced guidance kept being served, indefinitely, to
//! everyone.
//!
//! So every test here reads through a surface a user actually reaches — the CLI,
//! `cairn_search`, `cairn_context` — and never calls a subject function directly.

use cairn_e2e::{attach_server, post_json_status_bearer, Mcp, Sandbox, Server};
use serde_json::json;
use uuid::Uuid;

fn server() -> Option<Server> {
    match Server::start() {
        Some(s) => Some(s),
        None => {
            eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the reconciliation suite");
            None
        }
    }
}

/// A sandbox linked to `server` as an administrator, so it can ratify.
struct Admin {
    sandbox: Sandbox,
}

fn admin(server: &Server, label: &str) -> Admin {
    let sandbox = Sandbox::new();
    let remote = format!("git@localhost:cairnfixture/{label}.git");
    sandbox.git(&["remote", "add", "origin", &remote]);
    sandbox.must(&["init"]);

    // An administrator, because ratification is admin-only and the whole point
    // of this suite is the effect of a ratification.
    // Unique per run: the suite shares one database, so a fixed address collides
    // with the previous run's account.
    let email = format!("{label}-{}@example.test", Uuid::now_v7());
    server.create_user(&email, label, "hunter2hunter2");
    server.execute(&format!(
        "UPDATE users SET role = 'admin' WHERE email = '{email}'"
    ));
    let token = server.token_for(&email, "hunter2hunter2");

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
    Admin { sandbox }
}

/// Propose and ratify one team entry, returning its id.
///
/// `topic_key` is a parameter rather than a constant because team knowledge is
/// server-wide: every sandbox linked to the shared test server pulls every other
/// test's ratified guidance, so two tests sharing a subject key would reconcile
/// against each other. That cross-test visibility is the feature working (FR-463)
/// and the reason each test needs a subject of its own.
fn ratified(
    a: &Admin,
    topic_key: &str,
    content: &str,
    value_key: &str,
    supersedes: Option<&str>,
) -> String {
    let proposed = a.sandbox.json(&[
        "team",
        "propose",
        content,
        "--topic-key",
        topic_key,
        "--value-key",
        value_key,
    ]);
    let id = proposed["entry"]["id"]
        .as_str()
        .expect("a proposed id")
        .to_string();
    // The proposal has to reach the server before an administrator can ratify
    // it: ratification is a server route (`global-memory.md` §5b), and the
    // server cannot act on a row it has not received. In practice an admin
    // ratifies someone else's proposal, which arrived by synchronization long
    // before; here the same delivery is explicit.
    a.sandbox.must(&["sync", "now"]);
    let mut args = vec!["team", "ratify", &id];
    if let Some(old) = supersedes {
        args.push("--supersedes");
        args.push(old);
    }
    let out = a.sandbox.cairn(&args);
    assert!(out.ok(), "ratify failed: {}{}", out.stdout, out.stderr);
    id
}

// ---------------------------------------------------------------------------
// 4. Team supersedes stops the old entry competing — in every canonical read
// ---------------------------------------------------------------------------

/// After `ratify --supersedes`, the replaced entry no longer competes as
/// guidance in search, in the briefing, or in the subject read; the replacement
/// does.
///
/// Read through all three because they were three independent spellings of
/// "current" before this repair, and a fix applied to one would have left the
/// others serving retired guidance.
///
/// Falsified by dropping the `superseded_by_id` write from `ratify_team`, or by
/// reverting any canonical read to a bare `state = 'authoritative'`.
#[test]
fn a_superseded_team_entry_stops_competing_in_every_canonical_read() {
    let Some(server) = server() else { return };
    let a = admin(&server, "supersede");
    let cwd = a.sandbox.repo_path().to_string_lossy().to_string();

    let topic = format!("style.commit_message.{}", Uuid::now_v7().simple());
    let old = ratified(
        &a,
        &topic,
        "commit messages are free-form",
        "free_form",
        None,
    );
    let new = ratified(
        &a,
        &topic,
        "commit messages follow Conventional Commits",
        "conventional",
        Some(&old),
    );

    // Search: the replacement is there, the replaced entry is not.
    let mut mcp = Mcp::start(&a.sandbox);
    let searched = mcp.tool_result("cairn_search", json!({ "query": "commit messages" }), &cwd);
    let team = searched["content"][0]["text"]["team"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let ids: Vec<&str> = team.iter().filter_map(|t| t["id"].as_str()).collect();
    assert!(
        ids.contains(&new.as_str()),
        "the replacement guidance is not in search: {searched}"
    );
    assert!(
        !ids.contains(&old.as_str()),
        "the superseded guidance is still competing in search: {searched}"
    );

    // The briefing, which is what an agent actually reads.
    let context = mcp.tool("cairn_context", json!({}), &cwd);
    assert!(
        context.contains("Conventional Commits"),
        "the replacement guidance did not reach the briefing:\n{context}"
    );
    assert!(
        !context.contains("free-form"),
        "the superseded guidance is still in the briefing:\n{context}"
    );

    // The subject read — the surface that answers "what does the team believe
    // about this?" — agrees, and reports one answer rather than a conflict.
    let subject = a
        .sandbox
        .json(&["memory", "subject", &topic, "--domain", "team"]);
    let answers = subject["subject"]["answers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        answers.len(),
        1,
        "the subject read still sees two competing answers: {subject}"
    );
    assert_eq!(
        answers[0].as_str(),
        Some(new.as_str()),
        "the surviving answer is not the replacement: {subject}"
    );

    // `cairn team list` still shows it, with the pointer: the review surface must
    // not hide what an administrator did, or there is no way to see the history
    // of a decision.
    let listed = a.sandbox.json(&["team", "list"]);
    let rendered = listed.to_string();
    assert!(
        rendered.contains(&old),
        "the review listing hid the superseded entry, so an admin cannot see what \
         was replaced: {listed}"
    );
}

/// A member cannot hide team guidance by pushing a supersession pointer.
///
/// Supersession is an administrator act, reachable only through an
/// `AdminUser`-gated route. Sync ingest used to accept `superseded_by_id` on
/// conflict, which — once canonical reads began consulting it — let any
/// authenticated account remove any authoritative guidance from every reader on
/// the server, without needing a project membership.
///
/// Falsified by restoring `superseded_by_id` to `upsert_team`'s `DO UPDATE SET`.
#[test]
fn sync_ingest_cannot_supersede_or_retire_team_guidance() {
    let Some(server) = server() else { return };
    let a = admin(&server, "ingest-escalation");
    let id = ratified(
        &a,
        &format!("release.tags.{}", Uuid::now_v7().simple()),
        "release tags are annotated",
        "annotated",
        None,
    );

    // An ordinary member, with a project of their own.
    let member_token = server.new_user_token("ingest-member");
    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": "member-side", "repository_remote": "git@localhost:x/member.git" }),
        &member_token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let member_project = created["id"].as_str().expect("id").to_string();

    for attack in [
        json!({ "superseded_by_id": Uuid::now_v7() }),
        json!({ "retired_at": "2026-01-01T00:00:00Z" }),
    ] {
        let mut payload = json!({
            "id": id,
            "knowledge_type": "convention",
            "content": "release tags are annotated",
            "writer_id": Uuid::now_v7(),
            "writer_seq": 1,
            "state": "authoritative",
            "proposed_by_user_id": Uuid::now_v7(),
        });
        for (k, v) in attack.as_object().expect("object") {
            payload[k] = v.clone();
        }
        let (body, status) = post_json_status_bearer(
            &server.base,
            "/api/sync/batch",
            &json!({
                "project_id": member_project,
                "items": [{
                    "idempotency_key": Uuid::now_v7().to_string(),
                    "entity_type": "team_knowledge",
                    "entity_id": id,
                    "operation": "upsert",
                    "payload": payload,
                }],
            }),
            &member_token,
        );
        assert_eq!(status, 200, "the batch route itself failed: {body}");

        // Whether the item is reported applied or duplicate does not matter.
        // What matters is that the row did not move.
        assert_eq!(
            server.count(&format!(
                "SELECT COUNT(*) FROM team_knowledge \
                  WHERE id = '{id}' AND state = 'authoritative' \
                    AND superseded_by_id IS NULL AND retired_at IS NULL"
            )),
            1,
            "sync ingest changed a team entry's lifecycle: {attack}"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. Personal dedup, observable through a real read
// ---------------------------------------------------------------------------

/// Two content-identical personal entries reconcile as duplicates, and the
/// subject read says so.
///
/// FR-442 requires "the same deterministic reconciliation already used for
/// project memory", and project memory exposes it through `cairn memory
/// subject`. Parity therefore means the personal domain exposes it the same way
/// — which it did not, because no surface reached the derivation.
///
/// Both rows are retained deliberately: FR-442 says entries deduplicate in the
/// *reconciliation*, and the accounting is what records that two writes said one
/// thing. Deleting one would destroy the evidence.
#[test]
fn content_identical_personal_entries_reconcile_as_duplicates() {
    let s = Sandbox::new();
    let cwd = s.repo_path().to_string_lossy().to_string();
    let mut mcp = Mcp::start(&s);

    for _ in 0..2 {
        let created = mcp.tool_result(
            "cairn_remember",
            json!({
                "action": "create",
                "domain": "personal",
                "type": "fact",
                "content": "the retry budget is four attempts",
                "topic_key": "retry.budget",
                "value_key": "four",
            }),
            &cwd,
        );
        assert_eq!(
            created["isError"], false,
            "personal create failed: {created}"
        );
    }

    let subject = s.json(&["memory", "subject", "retry.budget", "--domain", "personal"]);
    let view = &subject["subject"];
    assert_eq!(
        view["members"].as_array().map(|m| m.len()),
        Some(2),
        "both writes should be retained as evidence: {subject}"
    );
    assert_eq!(
        view["answers"].as_array().map(|a| a.len()),
        Some(1),
        "two identical entries produced more than one answer: {subject}"
    );
    let accounting = view["accounting"].as_array().cloned().unwrap_or_default();
    // `duplicates` is the list of entries folded into this answer, so a reader
    // can see *which* write was the duplicate rather than only that there was
    // one.
    assert!(
        accounting
            .iter()
            .any(|a| a["duplicates"].as_array().is_some_and(|d| !d.is_empty())),
        "the duplicate was not accounted for anywhere a reader can see: {subject}"
    );
    assert_eq!(
        view["reconciliation"].as_str(),
        Some("reinforced"),
        "two identical entries did not reconcile as agreement: {subject}"
    );
    let decisions = view["decisions"].as_array().cloned().unwrap_or_default();
    assert!(
        decisions
            .iter()
            .any(|d| d["kind"].as_str() == Some("duplicates")),
        "the `duplicates` decision is not reported: {subject}"
    );
}

// ---------------------------------------------------------------------------
// 6. Personal conflict, observable through a real read
// ---------------------------------------------------------------------------

/// Two personal entries disagreeing on one subject surface as a standing
/// conflict, with both answers retained.
///
/// "Marked in conflict rather than one silently prevailing" (FR-442) is a claim
/// about what a reader sees. Before this surface existed the mark was written to
/// `personal_knowledge_relations` and read by nothing, so a reader saw two
/// unrelated notes and no indication that Cairn had noticed they disagreed.
#[test]
fn disagreeing_personal_entries_surface_as_a_standing_conflict() {
    let s = Sandbox::new();
    let cwd = s.repo_path().to_string_lossy().to_string();
    let mut mcp = Mcp::start(&s);

    for (content, value) in [
        ("the retry budget is four attempts", "four"),
        ("the retry budget is two attempts", "two"),
    ] {
        let created = mcp.tool_result(
            "cairn_remember",
            json!({
                "action": "create",
                "domain": "personal",
                "type": "fact",
                "content": content,
                "topic_key": "retry.budget",
                "value_key": value,
            }),
            &cwd,
        );
        assert_eq!(
            created["isError"], false,
            "personal create failed: {created}"
        );
    }

    let subject = s.json(&["memory", "subject", "retry.budget", "--domain", "personal"]);
    let view = &subject["subject"];
    assert_eq!(
        view["reconciliation"].as_str(),
        Some("conflicted"),
        "two disagreeing personal entries did not surface as a conflict: {subject}"
    );
    assert_eq!(
        view["answers"].as_array().map(|a| a.len()),
        Some(2),
        "the conflict does not retain both answers: {subject}"
    );
    let decisions = view["decisions"].as_array().cloned().unwrap_or_default();
    assert!(
        decisions
            .iter()
            .any(|d| d["kind"].as_str() == Some("conflicts_with")),
        "the `conflicts_with` decision is not reported: {subject}"
    );

    // The relation is over this domain's own table only: a project memory on the
    // same topic key does not join the personal subject (FR-517).
    s.must(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "--topic-key",
        "retry.budget",
        "--value-key",
        "eight",
        "the retry budget is eight attempts",
    ]);
    let again = s.json(&["memory", "subject", "retry.budget", "--domain", "personal"]);
    assert_eq!(
        again["subject"]["members"].as_array().map(|m| m.len()),
        Some(2),
        "a project memory leaked into the personal subject: {again}"
    );
}

/// A scope argument is refused for a domain that has none.
#[test]
fn a_global_subject_read_refuses_a_scope_it_cannot_honour() {
    let s = Sandbox::new();
    let refused = s.cairn(&[
        "--json",
        "memory",
        "subject",
        "retry.budget",
        "--domain",
        "personal",
        "--scope",
        "branch",
    ]);
    assert!(
        !refused.ok(),
        "a scoped personal subject read was accepted, so the scope was silently \
         ignored: {}",
        refused.stdout
    );
}
