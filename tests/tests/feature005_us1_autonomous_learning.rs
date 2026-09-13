//! User Story 1, end to end: work becomes knowledge without anyone asking
//! (T064, SC-701, SC-702).
//!
//! The story is one sentence and it is the whole feature: *a real coding
//! session that invokes no Cairn tool produces accurate, governed,
//! provenance-bearing knowledge.* Every other test in this feature holds one
//! link of that chain. This one drives the chain.
//!
//! # What "no tool" means here, exactly
//!
//! No `cairn_remember`, no `cairn memory add`, no MCP call of any kind. The
//! only thing that happens is that an agent works and its hooks fire. If a
//! record exists at the end, nothing asked for it — which is the claim.
//!
//! # What this test can and cannot assert
//!
//! SC-701 says accuracy is *a reviewed judgement recorded as such*, and that
//! the automated portion asserts existence, provenance resolution and rubric
//! completion. So this file asserts those three and does not pretend to assert
//! accuracy: a test that scored its own output against a rubric it also wrote
//! would be measuring agreement with itself. The rubric is pre-registered in
//! `tests/feature005/us1-accuracy-rubric.json`, its criteria are checked for
//! completeness here, and every criterion that *can* be evaluated mechanically
//! — the kind is one of the five, the content is non-empty, the keys are
//! normalized, the provenance resolves to real events in the right session — is
//! evaluated.
//!
//! # Why all three agents
//!
//! FR-838a commits to automatic capture for Claude Code, Codex CLI **and**
//! OpenCode, and SC-701 names all three. OpenCode emits no semantic signals
//! (FR-838b), so its record comes from structural evidence alone — which is
//! exactly the point of R1–R6 needing no prompt text, and is worth proving
//! rather than assuming.

use cairn_e2e::{attach_server, binary, post_json_status_bearer, Sandbox, Server};
use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A second `cairn-server` on the same database, started only for its
/// consolidation task.
///
/// The fixture's own server runs a pool too small to earn a share — below five
/// connections consolidation deliberately does not run at all, so that a small
/// deployment never starves request serving. That is correct behaviour and not
/// something to work around by widening the fixture; the way to get a
/// consolidation task is to run a server that qualifies for one.
struct Worker {
    child: Child,
}

impl Worker {
    fn start(database_url: &str) -> Self {
        let child = Command::new(binary("cairn-server"))
            .args([
                "--addr",
                "127.0.0.1:0",
                "--database-url",
                database_url,
                "--max-connections",
                "5",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cairn-server runs");
        Worker { child }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const SETTLE: Duration = Duration::from_secs(45);

/// A server on its own database.
///
/// Not the shared one. Three stories run in parallel here, each starts a
/// consolidation worker, and a single worker elects one session at a time
/// across every project in its database — so on a shared database the three
/// compete, and a story can wait out its deadline behind sessions that are not
/// its own. That is a property of the fixture and not of consolidation, and the
/// honest place to fix it is the fixture.
fn server() -> Option<Server> {
    match Server::start_own_database() {
        Some(s) => Some(s),
        None => {
            eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
            None
        }
    }
}

/// A device with zero memories, linked to `server`.
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

/// Wait until `predicate` holds, or fail naming what never happened.
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

/// The vendor payloads one agent's no-tool session produces.
///
/// A table per agent rather than three near-identical functions, because the
/// interesting difference between the agents is exactly which events they emit
/// and what they call their fields — and a table shows it side by side.
struct Session {
    agent: &'static str,
    events: Vec<(&'static str, Value)>,
}

fn claude_session(key: &str) -> Session {
    Session {
        agent: "claude-code",
        events: vec![
            (
                "SessionStart",
                json!({ "session_id": key, "source": "startup" }),
            ),
            // Red.
            (
                "PostToolUse",
                json!({
                    "session_id": key,
                    "tool_name": "Bash",
                    "tool_input": { "command": "cargo test -p widget" },
                    "tool_response": { "exit_code": 1 },
                }),
            ),
            // The change.
            (
                "PostToolUse",
                json!({
                    "session_id": key,
                    "tool_name": "Edit",
                    "tool_input": { "file_path": "src/widget/parser.rs" },
                    "tool_response": { "exit_code": 0 },
                }),
            ),
            // Green.
            (
                "PostToolUse",
                json!({
                    "session_id": key,
                    "tool_name": "Bash",
                    "tool_input": { "command": "cargo test -p widget" },
                    "tool_response": { "exit_code": 0 },
                }),
            ),
            // A decision expressed in prose. Only its vocabulary-justified
            // tokens may survive, and only because `parser` and `widget` are
            // already in this session's event stream.
            (
                "Stop",
                json!({
                    "session_id": key,
                    "last_assistant_message": "we should use parser for widget from now on",
                }),
            ),
            (
                "SessionEnd",
                json!({ "session_id": key, "reason": "clear" }),
            ),
        ],
    }
}

fn codex_session(key: &str) -> Session {
    Session {
        agent: "codex",
        events: vec![
            (
                "SessionStart",
                json!({ "thread_id": key, "source": "startup" }),
            ),
            (
                "PostToolUse",
                json!({
                    "thread_id": key,
                    "tool_name": "shell",
                    "tool_input": { "command": "cargo test -p widget" },
                    "tool_response": { "exit_code": 1 },
                }),
            ),
            (
                "PostToolUse",
                json!({
                    "thread_id": key,
                    "tool_name": "apply_patch",
                    "tool_input": { "file_path": "src/widget/parser.rs" },
                    "tool_response": { "exit_code": 0 },
                }),
            ),
            (
                "PostToolUse",
                json!({
                    "thread_id": key,
                    "tool_name": "shell",
                    "tool_input": { "command": "cargo test -p widget" },
                    "tool_response": { "exit_code": 0 },
                }),
            ),
            (
                "Stop",
                json!({
                    "thread_id": key,
                    "last_assistant_message": "we should use parser for widget from now on",
                }),
            ),
            ("SessionEnd", json!({ "thread_id": key, "reason": "clear" })),
        ],
    }
}

/// OpenCode's session, deliberately without any prompt or assistant text.
///
/// It emits no semantic signals at all (FR-838b), and it signals no session end
/// (FR-116) — `session.idle` means the agent went quiet. So its record has to
/// come from structural evidence alone, and the session is closed
/// server-side rather than by a vendor event.
fn opencode_session(key: &str) -> Session {
    Session {
        agent: "opencode",
        events: vec![
            (
                "session.created",
                json!({ "sessionID": key, "source": "startup" }),
            ),
            (
                "tool.execute.after",
                json!({
                    "sessionID": key,
                    "tool": "bash",
                    "args": { "command": "cargo test -p widget" },
                    "output": { "exit_code": 1 },
                }),
            ),
            (
                "tool.execute.after",
                json!({
                    "sessionID": key,
                    "tool": "edit",
                    "args": { "filePath": "src/widget/parser.rs" },
                    "output": { "exit_code": 0 },
                }),
            ),
            (
                "tool.execute.after",
                json!({
                    "sessionID": key,
                    "tool": "bash",
                    "args": { "command": "cargo test -p widget" },
                    "output": { "exit_code": 0 },
                }),
            ),
            ("session.idle", json!({ "sessionID": key })),
        ],
    }
}

/// Drive one agent's whole session and return the server-side session id.
fn drive(device: &Device, server: &Server, session: Session) -> Uuid {
    for (event, payload) in session.events {
        let result = device.sandbox.hook_as(session.agent, event, payload);
        // The hook always exits zero. Cairn is never the reason a session
        // breaks (FR-193, FR-194), and a non-zero exit here would be that.
        assert!(
            result.ok(),
            "{} {event} exited non-zero: {}",
            session.agent,
            result.stderr
        );
    }

    // The session reaches the server through ordinary synchronization; the
    // safe-event spool retries under backoff until it is there, because an
    // event whose session the server does not hold is refused.
    let project = device.project;
    settle("the session reaches the server", || {
        server.count(&format!(
            "SELECT COUNT(*) FROM sessions WHERE project_id = '{project}'"
        )) > 0
    });
    let id: String = server
        .query_column(&format!(
            "SELECT id::text FROM sessions WHERE project_id = '{project}' ORDER BY started_at DESC LIMIT 1"
        ))
        .first()
        .cloned()
        .expect("a synced session");
    id.parse().expect("uuid")
}

/// Every criterion the pre-registered rubric names.
fn rubric() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/feature005/us1-accuracy-rubric.json"
    );
    let text = std::fs::read_to_string(path).expect("the rubric is pre-registered");
    serde_json::from_str(&text).expect("the rubric parses")
}

#[test]
fn the_rubric_is_pre_registered_and_names_every_committed_agent() {
    // SC-701's "rubric completion" half. The rubric is fixed before the run,
    // and a criterion added after the fact would be a rubric written to fit its
    // answer.
    let rubric = rubric();
    let criteria = rubric["criteria"].as_array().expect("criteria");
    assert!(
        criteria.len() >= 4,
        "a rubric of {} criteria is not one somebody could disagree with",
        criteria.len()
    );
    for criterion in criteria {
        assert!(criterion["id"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(criterion["statement"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        assert!(
            criterion["evaluated"]
                .as_str()
                .is_some_and(|e| e == "mechanically" || e == "by_review"),
            "a criterion must say how it is evaluated, so nobody has to guess"
        );
    }
    let agents = rubric["agents"].as_array().expect("agents");
    for expected in ["claude_code", "codex", "opencode"] {
        assert!(
            agents.iter().any(|a| a.as_str() == Some(expected)),
            "{expected} is committed to automatic capture (FR-838a) and is not in the rubric"
        );
    }
}

/// One agent's whole story, asserted.
fn story(agent: &'static str, build: fn(&str) -> Session) {
    let Some(server) = server() else { return };
    let device = device(&server, agent);
    let project = device.project;

    // Zero memories, so nothing that appears later was already there.
    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM memories WHERE project_id = '{project}'"
        )),
        0,
        "the project did not start empty"
    );

    let key = format!("{agent}-us1-{}", Uuid::now_v7());
    let session = drive(&device, &server, build(&key));

    // The safe events reach the server. This is capture, and it happened with
    // no tool call of any kind.
    settle("safe events reach the server", || {
        server.count(&format!(
            "SELECT COUNT(*) FROM safe_events WHERE session_id = '{session}'"
        )) >= 4
    });

    // Close the session so consolidation elects it now rather than in ten
    // minutes. "Closed" is read from the server's own row, which is why a
    // client cannot declare it (contracts/consolidation.md §3) — so the test
    // closes it the way the server would see it.
    server.execute(&format!(
        "UPDATE sessions SET ended_at = now(), status = 'completed' WHERE id = '{session}'"
    ));

    // Consolidation is the deployment's task, not the request path's, so it
    // needs a server that qualifies for a pool share.
    let _worker = Worker::start(&server.database_url);

    settle("consolidation runs", || {
        server.count(&format!(
            "SELECT COUNT(*) FROM consolidation_runs
              WHERE session_id = '{session}' AND state = 'finished'"
        )) > 0
    });

    // SC-701: at least one durable record, and nobody asked for it.
    settle("durable knowledge appears", || {
        server.count(&format!(
            "SELECT COUNT(*) FROM memories
              WHERE project_id = '{project}' AND origin_kind = 'consolidated'"
        )) > 0
    });

    let records = server.query_column(&format!(
        "SELECT id::text FROM memories
          WHERE project_id = '{project}' AND origin_kind = 'consolidated'"
    ));
    assert!(!records.is_empty(), "no consolidated record for {agent}");

    // Every criterion the rubric marks as mechanically evaluable.
    for id in &records {
        let kind = server.text(&format!("SELECT type FROM memories WHERE id = '{id}'"));
        assert!(
            ["fact", "decision", "convention", "failure", "procedure"].contains(&kind.as_str()),
            "{agent} produced a record of kind {kind}, which is not one of the five"
        );
        let content = server.text(&format!("SELECT content FROM memories WHERE id = '{id}'"));
        assert!(
            !content.trim().is_empty(),
            "an empty claim is not knowledge"
        );

        let topic = server.text(&format!(
            "SELECT coalesce(topic_key, '') FROM memories WHERE id = '{id}'"
        ));
        let value = server.text(&format!(
            "SELECT coalesce(value_key, '') FROM memories WHERE id = '{id}'"
        ));
        assert_eq!(
            cairn_core::knowledge::normalize_topic_key(&topic).as_deref(),
            Some(topic.as_str()),
            "a stored topic key is not in its own canonical form"
        );
        assert_eq!(
            cairn_core::knowledge::normalize_value_key(&value).as_deref(),
            Some(value.as_str()),
            "a stored value key is not in its own canonical form"
        );

        // SC-702: 100% resolve to the session and the events they came from,
        // and zero have unresolvable provenance.
        //
        // Joined through `result_knowledge_id` rather than by assuming a
        // record's id equals its candidate's. It does for a record this pass
        // created, and it does not for one this pass reinforced — where the
        // record is the older one and the candidate is this pass's. A query
        // that assumed the first would silently pass on the case it was least
        // able to check.
        let sources = server.count(&format!(
            "SELECT COUNT(*) FROM candidate_source_events cse
               JOIN knowledge_candidates kc ON kc.candidate_id = cse.candidate_id
               JOIN safe_events e ON e.event_id = cse.event_id
              WHERE kc.result_knowledge_id = '{id}' AND e.session_id = '{session}'"
        ));
        assert!(
            sources > 0,
            "{agent}'s record {id} cannot be resolved to the events it came from"
        );
        let foreign = server.count(&format!(
            "SELECT COUNT(*) FROM candidate_source_events cse
               JOIN knowledge_candidates kc ON kc.candidate_id = cse.candidate_id
          LEFT JOIN safe_events e ON e.event_id = cse.event_id
              WHERE kc.result_knowledge_id = '{id}'
                AND (e.event_id IS NULL OR e.project_id <> '{project}')"
        ));
        assert_eq!(
            foreign, 0,
            "{agent}'s record {id} cites an event outside its own project"
        );
    }

    // Nothing was asked for: no explicit record, and no supersession.
    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM memories
              WHERE project_id = '{project}' AND origin_kind = 'explicit'"
        )),
        0,
        "an explicit record appeared in a session that invoked no tool"
    );
    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM memories
              WHERE project_id = '{project}' AND superseded_by_id IS NOT NULL"
        )),
        0,
        "consolidation superseded something, which it may never do (FR-800)"
    );
}

#[test]
fn a_claude_code_session_that_invokes_no_tool_produces_durable_knowledge() {
    story("claude_code", claude_session);
}

#[test]
fn a_codex_session_that_invokes_no_tool_produces_durable_knowledge() {
    story("codex", codex_session);
}

#[test]
fn an_opencode_session_produces_durable_knowledge_from_structure_alone() {
    // OpenCode emits no semantic signals, so this record rests entirely on
    // R1–R6 — which is the claim that structural capture is unaffected by the
    // semantic decline, rather than an assumption that it is.
    story("opencode", opencode_session);
}
