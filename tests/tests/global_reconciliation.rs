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

use cairn_e2e::{
    attach_server, get_json_status_bearer, post_json_status_bearer, Mcp, Sandbox, Server,
};
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
    email: String,
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
    Admin { sandbox, email }
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

    // A marker unique to this run, in the *content*. The suite shares one server
    // and team knowledge is server-wide, so every previous run's ratified
    // guidance is legitimately in this store — and a query matching all of it
    // would fill the result limit before reaching this run's rows.
    let run = format!("zz{}", Uuid::now_v7().simple());
    let topic = format!("style.commit_message.{run}");
    let old = ratified(
        &a,
        &topic,
        &format!("{run} commit messages are free-form"),
        "free_form",
        None,
    );
    let new = ratified(
        &a,
        &topic,
        &format!("{run} commit messages follow Conventional Commits"),
        "conventional",
        Some(&old),
    );

    // Search: the replacement is there, the replaced entry is not.
    //
    // Given a moment to become true rather than sampled once. `team ratify
    // --supersedes` is a server decision followed by a local write, and the
    // search reads the local copy — so an immediate read can catch the instant
    // between them and see the entry this test just replaced. That is the
    // transition working, observed too early, and asserting on the first sample
    // turned it into a failure about supersession roughly one run in four.
    //
    // The assertion itself is unchanged and still both-sided: waiting for the
    // replacement to appear would prove nothing on its own, so the superseded
    // entry must be gone *at the same observation*.
    let mut mcp = Mcp::start(&a.sandbox);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut searched;
    let mut ids: Vec<String>;
    loop {
        searched = mcp.tool_result("cairn_search", json!({ "query": run.clone() }), &cwd);
        ids = searched["content"][0]["text"]["team"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|t| t["id"].as_str().map(str::to_string))
            .collect();
        if ids.iter().any(|i| i == &new) || std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
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
        !context.contains(&format!("{run} commit messages are free-form")),
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

// ---------------------------------------------------------------------------
// FR-457 — who acted reaches every device, for both transitions
// ---------------------------------------------------------------------------

/// A retirement carries its actor to a second device, exactly as a ratification
/// does.
///
/// FR-457 requires every state transition to be recorded with who acted *and*
/// when, and to remain inspectable afterwards. The two halves of the lifecycle
/// were treated asymmetrically: `ratified_by_user_id` crossed the wire and
/// `retired_by_user_id` did not — it was not on the record type at all, so even
/// the machine that performed the retirement could not report it, and a device
/// that learned of one by pulling saw a timestamp and no actor.
///
/// "Who removed this guidance" is the question an operator asks first, and it is
/// the one the record was least able to answer.
///
/// Falsified by dropping `retired_by_user_id` from the wire row, from
/// `SyncedTeamKnowledge`, or from `team_bare`.
#[test]
fn a_retirement_carries_who_acted_to_a_second_device() {
    let Some(server) = server() else { return };
    let a = admin(&server, "retire-actor");
    let topic = format!("release.tags.{}", Uuid::now_v7().simple());
    let id = ratified(&a, &topic, "release tags are annotated", "annotated", None);

    let retired = a.sandbox.cairn(&["--json", "team", "retire", &id]);
    assert!(
        retired.ok(),
        "retire failed: {}{}",
        retired.stdout,
        retired.stderr
    );

    // The acting device records both halves.
    let local = a.sandbox.query_column(&format!(
        "SELECT COALESCE(retired_by_user_id, '') || '|' || COALESCE(retired_at, '') \
           FROM team_knowledge WHERE id = '{id}'"
    ));
    assert!(
        local.first().is_some_and(|r| {
            let (who, when) = r.split_once('|').unwrap_or(("", ""));
            !who.is_empty() && !when.is_empty()
        }),
        "the acting device recorded a retirement without its actor: {local:?}"
    );

    // And a second device, which learns of it only by pulling, records both too.
    let second = Sandbox::new();
    second.git(&[
        "remote",
        "add",
        "origin",
        "git@localhost:cairnfixture/retire-b.git",
    ]);
    second.must(&["init"]);
    let token = server.token_for(&a.email, "hunter2hunter2");
    attach_server(&second, &server, &token);
    // No `sync now`: that command requires a linked project, and this device has
    // none — team guidance is server-wide and needs no project at all. The
    // background worker establishes the team lane from the credentials alone and
    // pulls on its own schedule, which is exactly the consume-only path this
    // feature exists to support.
    second.settle_within(
        "the second device to receive the retirement",
        std::time::Duration::from_secs(60),
        |s| {
            s.query_column(&format!(
                "SELECT CAST(COUNT(*) AS TEXT) FROM team_knowledge \
                  WHERE id = '{id}' AND retired_at IS NOT NULL"
            )) == vec!["1".to_string()]
        },
    );

    let pulled = second.query_column(&format!(
        "SELECT COALESCE(retired_by_user_id, '') FROM team_knowledge WHERE id = '{id}'"
    ));
    assert!(
        pulled.first().is_some_and(|w| !w.is_empty()),
        "the retirement reached the second device without its actor, so \"who \
         removed this guidance\" is answerable only on the server (FR-457): {pulled:?}"
    );

    // The same is true of ratification, so the two halves stay symmetric.
    let ratifier = second.query_column(&format!(
        "SELECT COALESCE(ratified_by_user_id, '') FROM team_knowledge WHERE id = '{id}'"
    ));
    assert!(
        ratifier.first().is_some_and(|w| !w.is_empty()),
        "the ratification reached the second device without its actor: {ratifier:?}"
    );
}

// ===========================================================================
// PR #52 review: ingest immutability, attribution, and cursor safety
// ===========================================================================

/// Re-pushing an existing team id cannot re-scope authoritative guidance.
///
/// `DO NOTHING` made the row immutable through ingest, and `store_applicability`
/// ran underneath it anyway — deleting and reinserting the facts of a row the
/// statement had just declined to touch. Any member holding an id received from
/// team sync could re-push it with a fresh idempotency key and make ratified
/// guidance universal, or hide it from selected stacks, with no administrator
/// involved. Applicability is part of what an administrator ratified (FR-460), so
/// changing it is a ratification decision.
///
/// Falsified by calling `store_applicability` unconditionally again.
#[test]
fn re_pushing_a_team_id_cannot_change_where_it_applies() {
    let Some(server) = server() else { return };
    let a = admin(&server, "reingest-applic");
    let topic = format!("style.imports.{}", Uuid::now_v7().simple());

    // Ratified guidance restricted to one language.
    let proposed = a.sandbox.json(&[
        "team",
        "propose",
        "imports are grouped std / external / crate",
        "--topic-key",
        &topic,
        "--value-key",
        "grouped",
        "--applies-to",
        "language=rust",
    ]);
    let id = proposed["entry"]["id"].as_str().expect("id").to_string();
    a.sandbox.must(&["sync", "now"]);
    let out = a.sandbox.cairn(&["team", "ratify", &id]);
    assert!(out.ok(), "ratify failed: {}{}", out.stdout, out.stderr);

    let before = server.count(&format!(
        "SELECT COUNT(*) FROM team_knowledge_applicability WHERE team_id = '{id}'"
    ));
    assert_eq!(
        before, 1,
        "the fixture did not store one applicability fact"
    );

    // An ordinary member re-pushes the same id with the facts stripped, which
    // would make the guidance universal.
    let member_token = server.new_user_token("reingest-member");
    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": "reingest-side", "repository_remote": "git@localhost:x/reingest.git" }),
        &member_token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let member_project = created["id"].as_str().expect("id").to_string();

    for attack in [json!([]), json!([{ "kind": "language", "value": "go" }])] {
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
                    "payload": {
                        "id": id,
                        "knowledge_type": "convention",
                        "content": "imports are grouped std / external / crate",
                        "topic_key": topic,
                        "value_key": "grouped",
                        "writer_id": Uuid::now_v7(),
                        "writer_seq": 1,
                        "applicability": attack,
                    },
                }],
            }),
            &member_token,
        );
        assert_eq!(status, 200, "the batch route itself failed: {body}");

        let facts = server.text(&format!(
            "SELECT COALESCE(string_agg(kind || '=' || value, ',' ORDER BY value), '') \
               FROM team_knowledge_applicability WHERE team_id = '{id}'"
        ));
        assert_eq!(
            facts, "language=rust",
            "sync ingest re-scoped ratified team guidance: {attack}"
        );
    }
}

/// A pushed team proposal is attributed to the caller, not to whoever the
/// payload names.
///
/// `proposed_by_user_id` was read straight out of the untrusted item, so a member
/// could name another account — falsifying the attribution FR-459 keeps, and
/// making the role-filtered feed show that account a proposal it never made as
/// one of its own (FR-464 shows a member their *own* pending proposals).
///
/// Falsified by binding the payload field again.
#[test]
fn a_pushed_team_proposal_is_attributed_to_the_caller() {
    let Some(server) = server() else { return };
    let liar_token = server.new_user_token("attrib-liar");
    // The victim exists only to be named; the test never authenticates as them.
    let _victim_token = server.new_user_token("attrib-victim");

    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": "zephyrworks", "repository_remote": "git@localhost:zephyrworks/x.git" }),
        &liar_token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let project = created["id"].as_str().expect("id").to_string();

    let liar = server.text(
        "SELECT id::text FROM users WHERE email LIKE 'attrib-liar%' ORDER BY created_at DESC LIMIT 1",
    );
    let victim_user = server.text(
        "SELECT id::text FROM users WHERE email LIKE 'attrib-victim%' ORDER BY created_at DESC LIMIT 1",
    );
    assert_ne!(liar, victim_user, "the fixture needs two distinct accounts");

    let id = Uuid::now_v7();
    let (body, status) = post_json_status_bearer(
        &server.base,
        "/api/sync/batch",
        &json!({
            "project_id": project,
            "items": [{
                "idempotency_key": Uuid::now_v7().to_string(),
                "entity_type": "team_knowledge",
                "entity_id": id,
                "operation": "upsert",
                "payload": {
                    "id": id,
                    "knowledge_type": "convention",
                    "content": "guidance pushed under someone else's name",
                    "writer_id": Uuid::now_v7(),
                    "writer_seq": 1,
                    // The lie.
                    "proposed_by_user_id": victim_user,
                },
            }],
        }),
        &liar_token,
    );
    assert_eq!(status, 200, "the batch route itself failed: {body}");
    assert_eq!(
        body["results"][0]["status"].as_str(),
        Some("applied"),
        "the proposal was not ingested at all: {body}"
    );

    let stored = server.text(&format!(
        "SELECT proposed_by_user_id::text FROM team_knowledge WHERE id = '{id}'"
    ));
    assert_eq!(
        stored, liar,
        "the proposal was attributed to the account the payload named, not the \
         caller who pushed it"
    );
}

/// Re-pushing an existing personal id cannot change where it applies.
///
/// FR-440 makes a personal entry immutable after creation, the tombstone
/// excepted. The conflict branch of the ingest upsert honours that for content —
/// and `store_applicability` ran underneath it unconditionally, deleting and
/// reinserting the facts. A client could therefore re-push an id it already owned
/// and move an existing record's scope without forgetting and recreating it: an
/// immutable record whose scope was mutable.
///
/// Falsified by calling `store_applicability` unconditionally again.
#[test]
fn re_pushing_a_personal_id_cannot_change_where_it_applies() {
    let Some(server) = server() else { return };
    let token = server.new_user_token("personal-immutable");
    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": "quillstone", "repository_remote": "git@localhost:quillstone/x.git" }),
        &token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let project = created["id"].as_str().expect("id").to_string();

    let id = Uuid::now_v7();
    let push = |applicability: serde_json::Value, seq: i64| {
        post_json_status_bearer(
            &server.base,
            "/api/sync/batch",
            &json!({
                "project_id": project,
                "items": [{
                    "idempotency_key": Uuid::now_v7().to_string(),
                    "entity_type": "personal_knowledge",
                    "entity_id": id,
                    "operation": "upsert",
                    "payload": {
                        "id": id,
                        "knowledge_type": "fact",
                        "content": "clippy runs with warnings denied",
                        "writer_id": Uuid::now_v7(),
                        "writer_seq": seq,
                        "applicability": applicability,
                    },
                }],
            }),
            &token,
        )
    };

    let (body, status) = push(json!([{ "kind": "language", "value": "rust" }]), 1);
    assert_eq!(status, 200, "the batch route itself failed: {body}");
    assert_eq!(body["results"][0]["status"].as_str(), Some("applied"));
    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM personal_knowledge_applicability WHERE personal_id = '{id}'"
        )),
        1
    );

    // Re-push the same id with the facts stripped, and with a different one.
    for attack in [json!([]), json!([{ "kind": "language", "value": "go" }])] {
        let (body, status) = push(attack.clone(), 2);
        assert_eq!(status, 200, "the batch route itself failed: {body}");
        let facts = server.text(&format!(
            "SELECT COALESCE(string_agg(kind || '=' || value, ',' ORDER BY value), '') \
               FROM personal_knowledge_applicability WHERE personal_id = '{id}'"
        ));
        assert_eq!(
            facts, "language=rust",
            "sync ingest re-scoped an immutable personal record: {attack}"
        );
    }
}

/// A page boundary inside a group of rows sharing one `changed_at` loses nothing.
///
/// With a timestamp-only cursor, a tie group larger than the page limit was split
/// arbitrarily: the page returned some rows, the cursor advanced to that
/// timestamp, and the next request's strict `changed_at > since` skipped the rest
/// — permanently, because nothing asks for that instant again. Batched tombstones
/// share a `forgotten_at` and a bulk ratification shares an instant, so this is
/// reachable rather than theoretical.
///
/// Driven at the real route with a page limit of one, which makes every boundary
/// a tie boundary.
///
/// Falsified by dropping the id from either the ordering or the comparison.
#[test]
fn paging_through_rows_that_share_one_timestamp_loses_none_of_them() {
    let Some(server) = server() else { return };
    let token = server.new_user_token("tiepage");
    let email = server.text(
        "SELECT email FROM users WHERE email LIKE 'tiepage%' ORDER BY created_at DESC LIMIT 1",
    );
    let owner = server.text(&format!(
        "SELECT id::text FROM users WHERE email = '{email}'"
    ));

    // Six rows, one instant. Seeded directly: the point is the timestamp tie,
    // and producing one through the API would be timing-dependent.
    let marker = format!("zz{}", Uuid::now_v7().simple());
    for n in 0..6 {
        server.execute(&format!(
            "INSERT INTO personal_knowledge \
               (id, owner_user_id, knowledge_type, content, writer_id, writer_seq, created_at) \
             VALUES ('{}', '{owner}', 'fact', '{marker} row {n}', '{}', {n}, \
                     '2026-08-01T12:00:00Z')",
            Uuid::now_v7(),
            Uuid::now_v7(),
        ));
    }

    // Walk the feed one row at a time, following the cursor exactly as a client
    // does, and collect everything it hands back.
    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
        let path = match &cursor {
            Some(c) => format!(
                "/api/sync/changes/personal?limit=1&since={}",
                c.replace('+', "%2B")
                    .replace(':', "%3A")
                    .replace('|', "%7C")
            ),
            None => "/api/sync/changes/personal?limit=1".to_string(),
        };
        let (body, status) = get_json_status_bearer(&server.base, &path, &token);
        assert_eq!(status, 200, "read-back failed: {body}");
        let page = body["personal"].as_array().cloned().unwrap_or_default();
        if page.is_empty() {
            break;
        }
        for row in &page {
            if let Some(c) = row["content"].as_str() {
                if c.starts_with(&marker) {
                    seen.push(c.to_string());
                }
            }
        }
        cursor = body["cursor"].as_str().map(str::to_string);
    }

    seen.sort();
    seen.dedup();
    assert_eq!(
        seen.len(),
        6,
        "paging one row at a time through six rows sharing one timestamp returned \
         {} of them: {seen:?}",
        seen.len()
    );
}
