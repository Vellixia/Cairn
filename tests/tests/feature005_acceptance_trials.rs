//! T157: at least ten real-repository trials per supported agent, and the
//! material an independent reviewer needs to apply the accuracy
//! rubric (SC-701; SC-708 and SC-715 are covered by their own dedicated
//! tests — `feature005_us2_automatic_recall.rs` and `feature005_delivery.rs`
//! for delivery, `feature005_outage.rs`, `feature005_us4_fail_soft.rs` and
//! `feature005_performance.rs` for offline deadlines — so this file scopes to
//! the trial-and-rubric shape T157 actually spells out).
//!
//! # Why ten, and why this is not `feature005_us1_autonomous_learning.rs` run
//! ten times
//!
//! `feature005_us1_autonomous_learning.rs` proves the claim exists — one
//! trial per agent shows autonomous capture is possible at all. SC-701 makes
//! a stronger claim about a *population*: a reviewer judges the rubric's two
//! `by_review` criteria against real cases, and a sample of one is not a
//! sample a reviewer can generalize from. This file drives ten independent
//! sessions per agent, so the accuracy judgement has ten real records to look
//! at rather than one.
//!
//! # What this test can and cannot assert
//!
//! Exactly as in the US1 story: the five criteria the rubric
//! (`tests/feature005/us1-accuracy-rubric.json`) marks `mechanically` are
//! evaluated and asserted here, per record, across all ten trials. The two
//! it marks `by_review` — whether the claim is actually true of the project,
//! and whether it says no more than the events establish — belong to an
//! **independent reviewer**: one that is not this implementation or this test.
//! Independence, not humanity, is the property the rule rests on (SC-701), and
//! a reviewer may be a person or an external agent so long as it is not the
//! process being graded. This test does not evaluate them; it exports, for every
//! trial that produced a record, the record's kind and claim and its cited
//! safe events, so a reviewer can perform that judgement afterward. Writing a
//! number next to `by_review` here would be exactly the manufactured
//! evidence this feature spent seven stories forbidding — see
//! `tests/feature005/acceptance-results.md`, which is hand-written from this
//! test's real output and says so explicitly.
//!
//! # Why one project per agent, ten sessions, one consolidation wait
//!
//! Three agents times ten trials, each waiting out its own consolidation
//! pass, would mean thirty independent worker-startup-and-poll cycles. That is
//! avoidable: one worker elects one session at a time across every session in
//! its database regardless of which project it belongs to (see
//! `crates/cairn-server/src/consolidate.rs`'s `ELECT` query — it has no
//! `project_id` filter), so driving all ten sessions for an agent first and
//! only then starting a worker turns ten waits into one. The three agents
//! still each get an owned database and worker, run as three separate `#[test]`
//! functions, so `cargo test`'s default parallelism does across agents what
//! batching does within one.
//!
//! # Performance trade-off, recorded rather than hidden
//!
//! The extractor this suite runs against is the deterministic heuristic one
//! (no vendor API calls), so a ten-session batch costs a handful of
//! `BATCH_YIELD` (100 ms) polling cycles, not ten times a real inference
//! latency. Observed total wall time for this file is recorded in
//! `tests/feature005/acceptance-results.md` alongside the run it came from;
//! if a future run finds this file crowds the five-minute budget the fix is
//! to shrink `CONSOLIDATION_SETTLE`, not to weaken an assertion.

use cairn_e2e::{attach_server, binary, post_json_status_bearer, Sandbox, Server};
use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Real-repository trials per agent (T157's floor).
const TRIALS: usize = 10;

/// A second `cairn-server` on the same database, started only for its
/// consolidation task. See the identical fixture in
/// `feature005_us1_autonomous_learning.rs` for why the pool share needs a
/// dedicated qualifying server rather than the fixture's own undersized pool.
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

/// Deadline for one session's safe events to reach the server — a sync wait,
/// not a consolidation wait, so it stays short even though it runs ten times
/// per agent.
const SYNC_SETTLE: Duration = Duration::from_secs(45);

/// Deadline for all ten sessions in one agent's project to finish
/// consolidation, once the worker starts. Ten sessions behind one worker
/// rather than one, hence wider than `SYNC_SETTLE`; still bounded, so a
/// worker that genuinely wedges fails loudly instead of hanging the suite.
const CONSOLIDATION_SETTLE: Duration = Duration::from_secs(180);

/// A server on its own database, exactly as the US1 story uses — never the
/// suite-shared one, so this file's worker never competes with another
/// test's sessions for election.
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
fn settle(what: &str, timeout: Duration, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
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
/// Identical in shape to `feature005_us1_autonomous_learning.rs`'s `Session` —
/// deliberately not imported, because integration tests under `tests/tests/`
/// are separate crates and cannot share private items across files. This is
/// the "reuse the shape" the brief asks for, done the only way Rust allows it
/// here.
struct Session {
    agent: &'static str,
    events: Vec<(&'static str, Value)>,
    /// The canonical event kind that **closes** this fixture's stream — the
    /// last thing the client spools for the session, and therefore the marker
    /// that says the whole stream has been delivered rather than a prefix of
    /// it. See [`drive`] for why waiting on anything less is a synchronization
    /// bug, not a capability one.
    terminal: &'static str,
}

fn claude_session(key: &str) -> Session {
    Session {
        agent: "claude-code",
        events: vec![
            (
                "SessionStart",
                json!({ "session_id": key, "source": "startup" }),
            ),
            (
                "PostToolUse",
                json!({
                    "session_id": key,
                    "tool_name": "Bash",
                    "tool_input": { "command": "cargo test -p widget" },
                    "tool_response": { "exit_code": 1 },
                }),
            ),
            (
                "PostToolUse",
                json!({
                    "session_id": key,
                    "tool_name": "Edit",
                    "tool_input": { "file_path": "src/widget/parser.rs" },
                    "tool_response": { "exit_code": 0 },
                }),
            ),
            (
                "PostToolUse",
                json!({
                    "session_id": key,
                    "tool_name": "Bash",
                    "tool_input": { "command": "cargo test -p widget" },
                    "tool_response": { "exit_code": 0 },
                }),
            ),
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
        // `SessionEnd` is the last vendor event, and it normalizes to
        // `session_closed`.
        terminal: "session_closed",
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
        // As for Claude Code: `SessionEnd` closes the stream, and the codex
        // adapter normalizes it to `session_closed`.
        terminal: "session_closed",
    }
}

/// OpenCode's session, deliberately without any prompt or assistant text —
/// FR-838b, and the reason its record has to rest on structural evidence
/// alone (R1–R6), exactly as in the US1 story.
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
        // OpenCode has no session-end event at all: `session.idle` means the
        // agent went quiet and is **never** `session_closed`
        // (`crates/cairn-integrate/src/agents/opencode.rs`, `Route::Quiesced`).
        // So the kind that closes this fixture's stream is `agent_quiesced`,
        // and demanding `session_closed` here would wait forever.
        terminal: "agent_quiesced",
    }
}

/// Drive one agent's whole session and return the server-side session id.
///
/// Only syncs — no consolidation wait here, unlike the US1 story's `drive`.
/// That wait is batched once per agent, after all ten trials, by the caller.
///
/// # Two waits, because the session row and its events travel separately
///
/// This used to return as soon as the `sessions` row count for the project had
/// gone up, and that predicate guarantees almost nothing about the evidence
/// consolidation will run on. The session row and the safe events reach the
/// server by **two different paths**: the row rides the ordinary outbox, which
/// `cairn hook` drains synchronously at a delivery point
/// (`cairnd::deliver` → `sync::push_pending`), while the events sit in
/// `event_spool` until the background worker's next 500 ms tick calls
/// `sync::drain_event_spool`. The row therefore lands *first, by design* —
/// event ingest refuses an event whose session the server does not hold yet
/// (`cairn_server::events`, step 6, `SessionBindingError::Unresolvable`), so
/// the ordering is required, not incidental.
///
/// Measured, not assumed: instrumenting the old predicate showed the server
/// held **0 of this session's 12 canonical events** at the instant it fired in
/// 6 of 10 trials, with the remainder arriving 49–263 ms later. `run_trials`
/// then force-completes the sessions and starts consolidation. The only reason
/// the test passed was the wall time of the twenty-odd SQL round trips and the
/// worker spawn that happen to sit in between — accidental slack, which is
/// exactly the kind of thing a loaded CI box does not reproduce. Consolidation
/// on a prefix of a session's stream produces fewer records, or none: with the
/// slack removed, a run electing a session at 0/12 and 6/12 events finished
/// having produced **zero** durable records, which is precisely the "only
/// 9/10" SC-701 reported.
///
/// So this now waits on a *semantic* condition instead of a coincidence: the
/// session's terminal canonical event has arrived **and** the stream leading to
/// it has no holes. `session_seq` is a dense, 1-based, per-session counter
/// allocated when the event is spooled
/// (`cairn_store::spool::allocate_session_seq`), so `COUNT(*) = MAX(session_seq)`
/// is exactly "every sequence number up to the highest is present" — and with
/// the terminal event among them, "every sequence number" is the whole stream.
/// One statement, so the count and the maximum are read from one snapshot and
/// cannot be torn. No sleep anywhere in the condition.
fn drive(device: &Device, server: &Server, session: Session) -> Uuid {
    // Snapshotted *before* this trial's events fire, not after: sync can
    // finish while these events are still being posted, and a snapshot taken
    // afterward could already include this trial's own session — making
    // "greater than before" permanently false and every wait time out.
    let project = device.project;
    let before = server.count(&format!(
        "SELECT COUNT(*) FROM sessions WHERE project_id = '{project}'"
    ));
    let agent = session.agent;
    let terminal = session.terminal;

    for (event, payload) in session.events {
        let result = device.sandbox.hook_as(session.agent, event, payload);
        assert!(
            result.ok(),
            "{} {event} exited non-zero: {}",
            session.agent,
            result.stderr
        );
    }

    // First wait: the session *row*. Not a completion condition — only how
    // this test learns the id the second wait needs.
    settle("the session row reaches the server", SYNC_SETTLE, || {
        server.count(&format!(
            "SELECT COUNT(*) FROM sessions WHERE project_id = '{project}'"
        )) > before
    });
    let id: String = server
        .query_column(&format!(
            "SELECT id::text FROM sessions WHERE project_id = '{project}' ORDER BY started_at DESC LIMIT 1"
        ))
        .first()
        .cloned()
        .expect("a synced session");

    // Second wait: the *evidence*. This is the one the caller's forced
    // completion depends on.
    settle(
        &format!(
            "{agent}'s session {id} to deliver its whole canonical event stream, \
                  terminal `{terminal}` included, before it is closed and consolidated"
        ),
        SYNC_SETTLE,
        || {
            server.count(&format!(
                "SELECT CASE WHEN COUNT(*) > 0
                              AND COUNT(*) = MAX(session_seq)
                              AND COUNT(*) FILTER (WHERE kind = '{terminal}') = 1
                             THEN 1::bigint ELSE 0::bigint END
                   FROM safe_events WHERE session_id = '{id}'"
            )) == 1
        },
    );

    id.parse().expect("uuid")
}

/// Every criterion the rubric names, loaded once so this file
/// fails loudly if the rubric it depends on ever drifts out from under it.
fn rubric() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/feature005/us1-accuracy-rubric.json"
    );
    let text = std::fs::read_to_string(path).expect("the rubric was fixed before this run");
    serde_json::from_str(&text).expect("the rubric parses")
}

/// A safe event cited as a record's source, exported verbatim for the
/// reviewer — no filtering here beyond what already went through the
/// server's privacy boundary before this file ever queried it.
struct CitedEvent {
    kind: String,
    content: String,
}

/// One durable record, with what a reviewer needs to judge the two
/// `by_review` rubric criteria against it.
struct ReviewRecord {
    trial: usize,
    record_id: String,
    kind: String,
    claim: String,
    cited_events: Vec<CitedEvent>,
}

/// One agent's whole run: ten trials, one consolidation wait, the five
/// mechanical criteria checked and counted, and review material exported for
/// every record produced.
struct AgentReport {
    agent: &'static str,
    trials_run: usize,
    trials_with_record: usize,
    /// (criterion id, records checked, records that passed) — always equal
    /// when this function returns at all, because every criterion is also
    /// asserted; kept as counts anyway so the report states what was
    /// actually checked rather than only that nothing failed.
    criteria: Vec<(&'static str, usize, usize)>,
    nothing_asked_for_it_held: bool,
    review_records: Vec<ReviewRecord>,
    /// Which trials produced no durable record, and what the server held for
    /// every trial when the sessions were closed. Only read by the SC-701
    /// assertions, and only when one fails.
    trials_without_record: Vec<usize>,
    evidence_at_close: Vec<String>,
    /// Capture-class events this machine dropped during the run (FR-749c).
    ///
    /// Asserted to be zero before the trials are graded, and carried into the
    /// shortfall as well: a reader looking at "only 9/10" should not have to
    /// take on faith that Cairn lost nothing.
    dropped_by_cairn: String,
}

impl AgentReport {
    /// The two facts "only N/10" cannot carry: which trials, and what evidence
    /// consolidation was started on.
    fn shortfall(&self) -> String {
        format!(
            "\n  trials without a record: {:?}\n  capture-class events Cairn itself dropped: {}\n  \
             evidence held when the sessions were closed:\n    {}",
            self.trials_without_record,
            self.dropped_by_cairn,
            self.evidence_at_close.join("\n    ")
        )
    }
}

/// Run `TRIALS` real sessions for one agent and report what happened.
///
/// Returns `None` when there is no database to run against — the fixture's
/// ordinary skip, reported by the caller.
fn run_trials(agent: &'static str, build: fn(&str) -> Session) -> Option<AgentReport> {
    let server = server()?;
    let device = device(&server, agent);
    let project = device.project;
    // The canonical kind that closes this agent's stream, read off the fixture
    // itself rather than restated here — one source of truth for what "the
    // whole session arrived" means, shared with `drive`.
    let terminal = build("kind-probe").terminal;

    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM memories WHERE project_id = '{project}'"
        )),
        0,
        "the project did not start empty"
    );

    // Ten independent trials, each its own session id, none waiting for
    // consolidation yet.
    let mut sessions = Vec::with_capacity(TRIALS);
    for i in 0..TRIALS {
        let key = format!("{agent}-acceptance-{i}-{}", Uuid::now_v7());
        let session = drive(&device, &server, build(&key));
        sessions.push(session);
    }

    let in_list = sessions
        .iter()
        .map(|s| format!("'{s}'"))
        .collect::<Vec<_>>()
        .join(",");

    // **What the server actually held, per trial, at the instant the sessions
    // were closed.**
    //
    // SC-701 has failed on CI as "only 9/10", and the one thing the assertion
    // cannot say is *which* trial produced nothing and what evidence
    // consolidation had for it. That question is now **settled**: `drive` used
    // to wait only for the session *row*, which reaches the server on a
    // different path from the events and therefore always first, and
    // instrumentation showed the server holding 0 of 12 canonical events at
    // the instant that predicate fired. Closing and consolidating a session on
    // a prefix of its stream produces fewer records, or none. `drive` now waits
    // for the terminal canonical event and a gap-free sequence before it
    // returns, so by the time this snapshot is taken every trial's evidence is
    // complete by construction.
    //
    // The snapshot is kept anyway, and it is now a *discriminator* rather than
    // a hope: if SC-701 ever reports "only 9/10" again, these lines say whether
    // the stream was complete. A complete stream next to a missing record means
    // the fault is in consolidation or extraction, not in synchronization, and
    // the next investigation starts there instead of here.
    //
    // Taken before the close, because afterwards it is no longer the state
    // consolidation was started on.
    let mut evidence_at_close: Vec<String> = Vec::with_capacity(TRIALS);
    let mut incomplete_at_close: Vec<usize> = Vec::new();
    for (trial, session) in sessions.iter().enumerate() {
        let kinds = server
            .query_column(&format!(
                "SELECT kind || 'x' || COUNT(*)::text FROM safe_events
                  WHERE session_id = '{session}' GROUP BY kind ORDER BY kind"
            ))
            .join(",");
        let span = server
            .query_column(&format!(
                "SELECT COALESCE(MIN(session_seq), -1)::text || '..'
                        || COALESCE(MAX(session_seq), -1)::text
                        || ' of ' || COUNT(*)::text
                   FROM safe_events WHERE session_id = '{session}'"
            ))
            .join("");
        let complete = server.count(&format!(
            "SELECT CASE WHEN COUNT(*) > 0
                          AND COUNT(*) = MAX(session_seq)
                          AND COUNT(*) FILTER (WHERE kind = '{terminal}') = 1
                         THEN 1::bigint ELSE 0::bigint END
               FROM safe_events WHERE session_id = '{session}'"
        )) == 1;
        if !complete {
            incomplete_at_close.push(trial);
        }
        // **Why a decline declined**, which is the one thing the kind list
        // cannot say. A trial that produced no durable record because its
        // decision signal was never captured looks, in the kinds alone, exactly
        // like a trial whose extractor found nothing — `capture_declinedx1`
        // either way. The disposition separates them, and they are different
        // findings: `capture_deadline_exceeded` is Cairn dropping a
        // capture-class event under load, which FR-749b requires it to be free
        // to do, while `no_safe_semantic_mapping` or `privacy_refused` is a
        // decision about the content itself and would repeat on every run of a
        // frozen corpus.
        let declines = server
            .query_column(&format!(
                "SELECT COALESCE(content->>'disposition', '?')
                        || '/' || COALESCE(content->>'stage', '?')
                   FROM safe_events
                  WHERE session_id = '{session}' AND kind = 'capture_declined'
                  ORDER BY session_seq"
            ))
            .join(",");
        let declines = if declines.is_empty() {
            String::new()
        } else {
            format!(" declines[{declines}]")
        };
        evidence_at_close.push(format!(
            "trial {trial}: seq {span} complete={complete} [{kinds}]{declines}"
        ));
    }

    // **A run in which Cairn lost a capture-class event is not a measurement of
    // SC-701** (FR-749b, FR-749c).
    //
    // FR-749b permits the drop outright: a capture-class event that misses its
    // deadline is dropped, the hook still exits zero, and the agent never
    // learns. SC-701 says a session produces at least one durable record. Both
    // cannot be unconditionally true, and the gap between them is what "only
    // 9/10" was: one dropped `PostToolUse` costs the session its `file_changed`,
    // R1 has no change between the failing and the passing test and emits
    // nothing, the session's vocabulary is thinner so the closing semantic
    // signal declines too — and the stream that arrives is *dense* and
    // *terminated*, so the completeness check below cannot see it. A dropped
    // event never receives a `session_seq`, so the sequence closes over the gap.
    //
    // `spec.md` §SC-701 now states the population: the criterion is measured
    // over sessions in which Cairn dropped nothing of its own, and a run with a
    // Cairn-side loss is **inconclusive** rather than a pass or an accuracy
    // failure. Inconclusive has to fail a gate — a criterion that was not
    // measured cannot be merged on — but it fails *by name*, which is the whole
    // difference from the number this file used to report.
    //
    // It is answerable at all only because FR-749c is implemented now: the hook
    // journals the drop and the daemon counts it as `capture_deadline_exceeded`
    // (`cairnd::capture::collect_capture_drops`). Before that the disposition
    // existed in every vocabulary and was produced by nothing, which is exactly
    // why the historical decline reason was never recoverable.
    let dropped = device
        .sandbox
        .query_column(
            "SELECT CAST(COALESCE(SUM(n), 0) AS TEXT) FROM capture_disposition_counts               WHERE disposition = 'capture_deadline_exceeded'",
        )
        .first()
        .cloned()
        .unwrap_or_else(|| "0".to_string());
    let uncollected = device
        .sandbox
        .cairn_home()
        .join("capture-drops.ndjson")
        .exists();
    assert!(
        dropped == "0" && !uncollected,
        "{agent}: inconclusive — this machine dropped {dropped} capture-class event(s)          before they reached the daemon (uncollected journal: {uncollected}), so the          evidence SC-701 measures was incomplete through no fault of capture or          extraction. FR-749b permits the drop; SC-701 is measured over sessions          without one. Re-run; if it repeats, the capture deadline is too tight for          this machine (FR-749a)."
    );

    // **The invariant the forced completion below depends on, asserted where it
    // matters.**
    //
    // `drive` already refuses to return until each session's stream is whole,
    // so this cannot fail while that predicate is intact — which is exactly why
    // it belongs here. It names the guarantee at the point of use, so a future
    // change that weakens the wait fails as "the evidence was not there",
    // naming the trials, instead of resurfacing as an unexplained "only 9/10"
    // from the accuracy assertion two hundred lines down. Reverting `drive` to
    // the old session-row-count predicate removes the guarantee this rests on,
    // and it was observed failing under it — but only on a box fast enough to
    // reach here before the spool drained, so this is a guard, not a reliable
    // detector of that mutation. What *is* deterministic is the measurement in
    // `drive`'s comment: the old predicate fires with the stream absent.
    assert!(
        incomplete_at_close.is_empty(),
        "{agent}: consolidation would have been started on a partial event stream \
         for trials {incomplete_at_close:?} — SC-701 measures what the extractor \
         can do with a session's evidence, so the evidence has to be there first\n    {}",
        evidence_at_close.join("\n    ")
    );

    // Close every trial's session the way the server would see it closed
    // (contracts/consolidation.md §3) — one statement for all ten, matching
    // the "wait once" shape this file exists to prove out.
    server.execute(&format!(
        "UPDATE sessions SET ended_at = now(), status = 'completed' WHERE id IN ({in_list})"
    ));

    let _worker = Worker::start(&server.database_url);

    settle(
        "consolidation finishes for every trial",
        CONSOLIDATION_SETTLE,
        || {
            server.count(&format!(
                "SELECT COUNT(*) FROM consolidation_runs
                  WHERE session_id IN ({in_list}) AND state = 'finished'"
            )) == TRIALS as i64
        },
    );

    let mut trials_without_record: Vec<usize> = Vec::new();
    let mut trials_with_record = 0usize;
    let mut criterion_counts: [(usize, usize); 4] = [(0, 0); 4];
    // Index: 0 kind_is_one_of_five, 1 claim_is_non_empty, 2 keys_are_canonical,
    // 3 provenance_resolves. `nothing_asked_for_it` is project-wide, checked
    // once below rather than per record.
    let mut review_records = Vec::new();

    for (trial, session) in sessions.iter().enumerate() {
        let records = server.query_column(&format!(
            "SELECT DISTINCT kc.result_knowledge_id::text
               FROM knowledge_candidates kc
               JOIN consolidation_runs cr ON cr.run_id = kc.run_id
              WHERE cr.session_id = '{session}' AND kc.result_knowledge_id IS NOT NULL"
        ));
        if records.is_empty() {
            trials_without_record.push(trial);
        }
        if !records.is_empty() {
            trials_with_record += 1;
        }

        for id in &records {
            // kind_is_one_of_five (FR-795).
            let kind = server.text(&format!("SELECT type FROM memories WHERE id = '{id}'"));
            criterion_counts[0].0 += 1;
            assert!(
                ["fact", "decision", "convention", "failure", "procedure"].contains(&kind.as_str()),
                "{agent} trial {trial} produced a record of kind {kind}, which is not one of the five"
            );
            criterion_counts[0].1 += 1;

            // claim_is_non_empty.
            let content = server.text(&format!("SELECT content FROM memories WHERE id = '{id}'"));
            criterion_counts[1].0 += 1;
            assert!(
                !content.trim().is_empty(),
                "{agent} trial {trial}'s record {id} has an empty claim, which is not knowledge"
            );
            criterion_counts[1].1 += 1;

            // keys_are_canonical (FR-796a).
            let topic = server.text(&format!(
                "SELECT coalesce(topic_key, '') FROM memories WHERE id = '{id}'"
            ));
            let value = server.text(&format!(
                "SELECT coalesce(value_key, '') FROM memories WHERE id = '{id}'"
            ));
            criterion_counts[2].0 += 1;
            assert_eq!(
                cairn_core::knowledge::normalize_topic_key(&topic).as_deref(),
                Some(topic.as_str()),
                "{agent} trial {trial}'s record {id} has a topic key not in canonical form"
            );
            assert_eq!(
                cairn_core::knowledge::normalize_value_key(&value).as_deref(),
                Some(value.as_str()),
                "{agent} trial {trial}'s record {id} has a value key not in canonical form"
            );
            criterion_counts[2].1 += 1;

            // provenance_resolves (SC-702): every citation exists and belongs
            // to this trial's session, and none point outside the project.
            let sources = server.count(&format!(
                "SELECT COUNT(*) FROM candidate_source_events cse
                   JOIN knowledge_candidates kc ON kc.candidate_id = cse.candidate_id
                   JOIN safe_events e ON e.event_id = cse.event_id
                  WHERE kc.result_knowledge_id = '{id}' AND e.session_id = '{session}'"
            ));
            let foreign = server.count(&format!(
                "SELECT COUNT(*) FROM candidate_source_events cse
                   JOIN knowledge_candidates kc ON kc.candidate_id = cse.candidate_id
              LEFT JOIN safe_events e ON e.event_id = cse.event_id
                  WHERE kc.result_knowledge_id = '{id}'
                    AND (e.event_id IS NULL OR e.project_id <> '{project}')"
            ));
            criterion_counts[3].0 += 1;
            assert!(
                sources > 0,
                "{agent} trial {trial}'s record {id} cannot be resolved to the events it came from"
            );
            assert_eq!(
                foreign, 0,
                "{agent} trial {trial}'s record {id} cites an event outside its own project"
            );
            criterion_counts[3].1 += 1;

            // Export the review material the two `by_review` criteria need,
            // without evaluating them: the kind, the claim, and every cited
            // safe event, in session order.
            let cited_events = server
                .query_column(&format!(
                    "SELECT e.kind || '\t' || e.content::text
                       FROM candidate_source_events cse
                       JOIN knowledge_candidates kc ON kc.candidate_id = cse.candidate_id
                       JOIN safe_events e ON e.event_id = cse.event_id
                      WHERE kc.result_knowledge_id = '{id}'
                      ORDER BY e.session_seq"
                ))
                .into_iter()
                .map(|row| {
                    let mut parts = row.splitn(2, '\t');
                    let kind = parts.next().unwrap_or_default().to_string();
                    let content = parts.next().unwrap_or_default().to_string();
                    CitedEvent { kind, content }
                })
                .collect();

            review_records.push(ReviewRecord {
                trial,
                record_id: id.clone(),
                kind,
                claim: content,
                cited_events,
            });
        }
    }

    // nothing_asked_for_it: across all ten trials, no explicit record and no
    // supersession, project-wide — no session here invoked a Cairn tool.
    let explicit = server.count(&format!(
        "SELECT COUNT(*) FROM memories WHERE project_id = '{project}' AND origin_kind = 'explicit'"
    ));
    let superseded = server.count(&format!(
        "SELECT COUNT(*) FROM memories
          WHERE project_id = '{project}' AND superseded_by_id IS NOT NULL"
    ));
    assert_eq!(
        explicit, 0,
        "an explicit record appeared for {agent} in a run that invoked no tool"
    );
    assert_eq!(
        superseded, 0,
        "{agent}'s run superseded something, which consolidation may never do (FR-800)"
    );

    Some(AgentReport {
        agent,
        trials_run: TRIALS,
        trials_with_record,
        criteria: vec![
            (
                "kind_is_one_of_five",
                criterion_counts[0].0,
                criterion_counts[0].1,
            ),
            (
                "claim_is_non_empty",
                criterion_counts[1].0,
                criterion_counts[1].1,
            ),
            (
                "keys_are_canonical",
                criterion_counts[2].0,
                criterion_counts[2].1,
            ),
            (
                "provenance_resolves",
                criterion_counts[3].0,
                criterion_counts[3].1,
            ),
        ],
        nothing_asked_for_it_held: explicit == 0 && superseded == 0,
        review_records,
        trials_without_record,
        evidence_at_close,
        dropped_by_cairn: dropped,
    })
}

/// Print the report this test's own assertions already vouch for.
///
/// Run with `--nocapture` to read it: this is the source `acceptance-
/// results.md` is transcribed from by hand, not a file this test writes
/// itself, because SC-701's rubric completion means an independent reviewer
/// states what was found — a test that wrote its own results file would be
/// reviewing itself exactly as the `by_review` criteria warn against.
fn report(r: &AgentReport) {
    println!("=== T157 acceptance trials: {} ===", r.agent);
    println!(
        "trials_run={} trials_with_durable_record={}",
        r.trials_run, r.trials_with_record
    );
    for (id, checked, passed) in &r.criteria {
        println!("criterion {id}: {passed}/{checked}");
    }
    // What the extractor was actually given, per trial. Printed rather than
    // held for a failure message: a reviewer applying the rubric needs to know
    // how much evidence each record was drawn from, and the stream length is
    // not always the fixture's maximum: shorter streams have been observed on a
    // loaded machine (nine canonical events rather than twelve, terminal event
    // present and no gaps), so the number of events behind a record is worth
    // stating rather than assuming.
    for line in &r.evidence_at_close {
        println!("evidence {line}");
    }
    println!(
        "nothing_asked_for_it: {}",
        if r.nothing_asked_for_it_held {
            "held"
        } else {
            "VIOLATED"
        }
    );
    for rec in &r.review_records {
        println!(
            "-- trial {} record {} kind={} --",
            rec.trial, rec.record_id, rec.kind
        );
        println!("claim: {}", rec.claim);
        for ev in &rec.cited_events {
            println!("  cited [{}]: {}", ev.kind, ev.content);
        }
    }
    println!("=== end {} ===", r.agent);
}

/// This file's mechanical checks are not free to drift from what the
/// rubric actually names — SC-701's rubric-completion half
/// means the set evaluated here must equal the set the rubric marks
/// `mechanically`, not a superset or subset a later edit quietly grew or
/// shrank.
#[test]
fn this_files_mechanical_criteria_match_the_pre_registered_rubric() {
    let rubric = rubric();
    let mut mechanical: Vec<&str> = rubric["criteria"]
        .as_array()
        .expect("criteria")
        .iter()
        .filter(|c| c["evaluated"] == "mechanically")
        .map(|c| c["id"].as_str().expect("id"))
        .collect();
    mechanical.sort_unstable();

    let mut checked = vec![
        "kind_is_one_of_five",
        "claim_is_non_empty",
        "keys_are_canonical",
        "provenance_resolves",
        "nothing_asked_for_it",
    ];
    checked.sort_unstable();

    assert_eq!(
        mechanical, checked,
        "this file's mechanical criteria have drifted from the rubric"
    );

    let by_review: Vec<&str> = rubric["criteria"]
        .as_array()
        .expect("criteria")
        .iter()
        .filter(|c| c["evaluated"] == "by_review")
        .map(|c| c["id"].as_str().expect("id"))
        .collect();
    assert!(
        !by_review.is_empty(),
        "the rubric names no by_review criteria; accuracy would then be asserted \
         mechanically, which is exactly what SC-701 forbids"
    );
}

#[test]
fn ten_claude_code_trials_produce_durable_knowledge_and_rubric_material() {
    let Some(r) = run_trials("claude_code", claude_session) else {
        return;
    };
    assert_eq!(
        r.trials_with_record,
        TRIALS,
        "SC-701 requires every session that invokes no Cairn tool to produce at least \
         one durable record; only {}/{TRIALS} of claude_code's trials did{}",
        r.trials_with_record,
        r.shortfall()
    );
    report(&r);
}

#[test]
fn ten_codex_trials_produce_durable_knowledge_and_rubric_material() {
    let Some(r) = run_trials("codex", codex_session) else {
        return;
    };
    assert_eq!(
        r.trials_with_record,
        TRIALS,
        "SC-701 requires every session that invokes no Cairn tool to produce at least \
         one durable record; only {}/{TRIALS} of codex's trials did{}",
        r.trials_with_record,
        r.shortfall()
    );
    report(&r);
}

#[test]
fn ten_opencode_trials_produce_durable_knowledge_from_structure_alone() {
    let Some(r) = run_trials("opencode", opencode_session) else {
        return;
    };
    assert_eq!(
        r.trials_with_record,
        TRIALS,
        "SC-701 requires every session that invokes no Cairn tool to produce at least \
         one durable record; only {}/{TRIALS} of opencode's trials did{}",
        r.trials_with_record,
        r.shortfall()
    );
    report(&r);
}
