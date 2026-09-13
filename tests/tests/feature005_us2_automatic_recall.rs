//! User Story 2, end to end: a second session starts already knowing (T082,
//! SC-708, SC-709, SC-712, SC-729).
//!
//! # What makes this the *story* test
//!
//! Its knowledge is seeded through the server directly, not produced by Story
//! 1's capture path. US2's independent test says so explicitly, and the reason
//! is worth stating: if this test needed capture to work first, a capture
//! regression would fail it and nobody would learn anything about delivery.
//! Here the only thing under test is that knowledge the server already holds
//! reaches a session that asked for nothing.
//!
//! # What "no tool call" means
//!
//! No `cairn_context`, no `cairn memory`, no MCP call. A hook fires because the
//! agent started, and context comes back on the vendor's own return channel. If
//! the second session knows something the first one recorded, nothing asked for
//! it — which is the claim.
//!
//! # Both committed agents, and only those
//!
//! FR-838a commits automatic delivery to Claude Code and Codex CLI. OpenCode is
//! excluded because Cairn **declines** to depend on its beta delivery surface,
//! which is a Cairn decision and not a vendor limitation — asserted here so the
//! exclusion is recorded rather than implied by absence.

use cairn_e2e::{attach_server, post_json_status_bearer, Sandbox, Server};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use uuid::Uuid;

const SETTLE: Duration = Duration::from_secs(30);

fn server() -> Option<Server> {
    match Server::start_own_database() {
        Some(s) => Some(s),
        None => {
            eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
            None
        }
    }
}

struct Device {
    sandbox: Sandbox,
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
    Device { sandbox, project }
}

fn settle(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + SETTLE;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timed out waiting for: {what}");
}

/// Knowledge the server already holds, put there without any capture.
fn seed(server: &Server, project: Uuid, session: Uuid, content: &str, topic: &str) {
    server.execute(&format!(
        "INSERT INTO memories
            (id, project_id, type, scope, scope_key, content, state, origin_session_id,
             topic_key, value_key, origin_kind)
         VALUES ('{}', '{project}', 'decision', 'project', '{project}', '{content}', 'active',
                 '{session}', '{topic}', 'settled', 'explicit')",
        Uuid::now_v7()
    ));
}

/// The context an agent actually received on its own return channel.
///
/// Read from the hook's stdout rather than from the daemon, because what the
/// daemon assembled and what the agent received are two different facts and
/// this test is about the second one.
fn delivered_context(out: &cairn_e2e::CliResult) -> String {
    assert_eq!(out.code, 0, "a hook always exits 0: {}", out.stderr);
    let emitted: Value = serde_json::from_str(out.stdout.trim())
        .unwrap_or_else(|e| panic!("hook did not emit context JSON ({e}): {:?}", out.stdout));
    emitted["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The first session, whose only job is to exist so the second one is second.
fn open_first_session(device: &Device, agent: &str, key: &str) {
    let payload = match agent {
        "codex" => json!({ "thread_id": key, "source": "startup" }),
        _ => json!({ "session_id": key, "source": "startup" }),
    };
    let out = device.sandbox.hook_as(agent, "SessionStart", payload);
    assert_eq!(out.code, 0, "{agent} session start: {}", out.stderr);
}

fn session_ids(server: &Server, project: Uuid) -> Vec<Uuid> {
    server
        .query_column(&format!(
            "SELECT id::text FROM sessions WHERE project_id = '{project}' ORDER BY started_at"
        ))
        .into_iter()
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// One agent's whole second-session story.
fn story(agent: &'static str) {
    let Some(server) = server() else { return };
    let device = device(&server, agent);
    let project = device.project;

    // A first session, so the second is genuinely a second.
    let first_key = format!("{agent}-first-{}", Uuid::now_v7());
    open_first_session(&device, agent, &first_key);
    settle("the first session reaches the server", || {
        !session_ids(&server, project).is_empty()
    });
    let first = session_ids(&server, project)[0];

    // Knowledge the server holds, seeded directly. Nothing captured it.
    seed(
        &server,
        project,
        first,
        "the parser is the settled owner of widget validation",
        "decision.parser",
    );
    seed(
        &server,
        project,
        first,
        "retries are capped at three for the widget client",
        "decision.retries",
    );

    // The second session. One hook, no tool call.
    let second_key = format!("{agent}-second-{}", Uuid::now_v7());
    let payload = match agent {
        "codex" => json!({ "thread_id": second_key, "source": "startup" }),
        _ => json!({ "session_id": second_key, "source": "startup" }),
    };
    let out = device.sandbox.hook_as(agent, "SessionStart", payload);
    let context = delivered_context(&out);

    // SC-708: the second session received relevant prior knowledge, and nobody
    // asked for it.
    assert!(
        context.contains("settled owner of widget validation"),
        "{agent}'s second session did not receive what the first one recorded: {context}"
    );

    // SC-729: the retrieval is traced, and the trace says what happened rather
    // than what was hoped for.
    settle("a trace exists for the second session", || {
        server.count(&format!(
            "SELECT count(*) FROM retrieval_traces WHERE project_id = '{project}'"
        )) > 0
    });
    let states = server.query_column(&format!(
        "SELECT delivery_state FROM retrieval_traces WHERE project_id = '{project}'"
    ));
    assert!(
        !states.is_empty(),
        "{agent}: a briefing was delivered and nothing recorded it"
    );
    assert!(
        states.iter().all(|s| s != "requested"),
        "{agent}: a trace was left mid-flight at `requested`: {states:?}"
    );

    // SC-712: acknowledgement is never claimed, for any agent, after any
    // outcome. Returning context establishes transmission and nothing more.
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM retrieval_traces
              WHERE project_id = '{project}' AND acknowledgement_state <> 'unavailable'"
        )),
        0,
        "{agent}: a trace claimed an acknowledgement no vendor mechanism establishes"
    );

    // Delivery is recorded only where transmission was, and only then.
    let transmitted = server.count(&format!(
        "SELECT count(*) FROM retrieval_traces
          WHERE project_id = '{project}' AND delivery_state = 'transmitted'"
    ));
    let delivered = server.count(&format!(
        "SELECT count(*) FROM delivered_context d
           JOIN sessions s ON s.id = d.session_id
          WHERE s.project_id = '{project}'"
    ));
    if transmitted == 0 {
        assert_eq!(
            delivered, 0,
            "{agent}: delivery rows exist although no transmission was ever reported"
        );
    }
}

#[test]
fn a_second_claude_code_session_starts_already_knowing() {
    story("claude-code");
}

#[test]
fn a_second_codex_session_starts_already_knowing() {
    story("codex");
}

#[test]
fn opencodes_automatic_delivery_stays_declined_by_cairn() {
    // SC-708's exclusion, recorded rather than implied by absence. OpenCode 2
    // does expose prompt and context hooks — they are beta, and declining to
    // rest a guarantee on a beta surface is Cairn's decision. Reporting it as a
    // vendor limitation would be untrue (FR-838b).
    use cairn_integrate::capability::{declared_matrix, MatrixStatus};
    let matrix = declared_matrix("opencode");
    for point in [
        "deliver:session_open",
        "deliver:prompt_time",
        "deliver:post_compaction",
    ] {
        let cell = matrix
            .iter()
            .find(|c| c.capability == point)
            .unwrap_or_else(|| panic!("{point} is missing from the matrix"));
        assert_eq!(
            cell.status,
            MatrixStatus::DeclinedByCairn,
            "{point} is not reported as Cairn's own decision"
        );
        assert_ne!(cell.status, MatrixStatus::UnsupportedByVendor);
    }

    // And the two committed agents are not declined — their delivery awaits an
    // observation, which is a different statement from a refusal.
    for agent in ["claude_code", "codex"] {
        for cell in declared_matrix(agent)
            .into_iter()
            .filter(|c| c.capability.starts_with("deliver:"))
        {
            assert_eq!(
                cell.status,
                MatrixStatus::NoEvidence,
                "{agent}:{} should await evidence, not be declared",
                cell.capability
            );
        }
    }
}
