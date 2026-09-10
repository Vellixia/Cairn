//! SC-701a and SC-701b, proved on **durable** knowledge (T041, T061).
//!
//! # What this file exists to fix
//!
//! SC-701a says at least fourteen of twenty sessions "produce a durable
//! `decision` or `convention` record". The test that previously claimed it
//! counted `EventContent::Decision` values coming out of `capture()` — a
//! signal, not a record — against a table that held only sixteen positive
//! scenarios and four deliberate declines, on a path its own header described
//! as "no PostgreSQL, no daemon". Every one of those is defensible as a
//! capture-layer contract and none of them is the criterion.
//!
//! Worse, the population could not have satisfied the criterion even if the
//! counting had been right. R7 turns each decision signal into one durable
//! decision, and there were eight. R8 converts only `require|forbid`, and only
//! when the same `(kind, subject, object)` appears in two sessions — and all
//! eight instruction rows were unique, so none of them reached it. The
//! reachable maximum was **8 of 20** against a required 14.
//!
//! # What this file does instead
//!
//! It reads the frozen holdout corpus
//! (`tests/feature005/corpora/sc701a-holdout.json`) and drives every scenario
//! down the real chain:
//!
//! ```text
//! vendor payload → cairn hook → capture → daemon spool
//!   → authenticated ingest → safe_events
//!   → consolidation worker → governance → PostgreSQL memories
//! ```
//!
//! A scenario counts as a hit only if a row exists in `memories` with
//! `origin_kind = 'consolidated'`, the expected durable type, the expected
//! normalized `topic_key`/`value_key`, provenance resolving through
//! `knowledge_candidates` and `candidate_source_events` to the session's own
//! `safe_events`, and — separately — a post-signal action recorded after the
//! semantic event in that session.
//!
//! # The two constants are fixed, not derived
//!
//! `QUALIFYING_SESSIONS` and `MINIMUM_DURABLE_MATCHES` are written here as
//! literals and cross-checked against the corpus file. Deriving either from
//! the corpus length or from what the implementation produced is how a
//! criterion quietly becomes "whatever happens".

use cairn_e2e::{attach_server, binary, post_json_status_bearer, Sandbox, Server};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// SC-701a's population size. A literal, so narrowing the corpus fails here.
const QUALIFYING_SESSIONS: usize = 20;
/// SC-701a's threshold. A literal, for the same reason.
const MINIMUM_DURABLE_MATCHES: usize = 14;

const SETTLE: Duration = Duration::from_secs(90);

// ---------------------------------------------------------------------------
// The frozen corpus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Scenario {
    id: String,
    vendor: String,
    semantic_role: String,
    vocabulary_path: String,
    transient_text: String,
    subject: String,
    object: String,
    verb: String,
    durable_type: String,
    topic_key: String,
    value_key: String,
    action_command: String,
}

fn corpus() -> Vec<Scenario> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/feature005/corpora/sc701a-holdout.json"
    );
    let text = std::fs::read_to_string(path).expect("the frozen holdout corpus");
    let doc: Value = serde_json::from_str(&text).expect("the corpus is valid JSON");

    // The corpus states its own population and threshold, and they must agree
    // with the literals above. Two independent statements of one number is how
    // a silent edit to either becomes a failure rather than a new criterion.
    assert_eq!(
        doc["qualifying_sessions"].as_u64(),
        Some(QUALIFYING_SESSIONS as u64),
        "the corpus and this test disagree about the population size"
    );
    assert_eq!(
        doc["minimum_durable_matches"].as_u64(),
        Some(MINIMUM_DURABLE_MATCHES as u64),
        "the corpus and this test disagree about the threshold"
    );

    let s = |v: &Value, k: &str| -> String {
        v[k].as_str()
            .unwrap_or_else(|| panic!("corpus entry {v} has no string field {k:?}"))
            .to_string()
    };
    doc["scenarios"]
        .as_array()
        .expect("scenarios is an array")
        .iter()
        .map(|v| {
            let e = &v["expected"];
            Scenario {
                id: s(v, "id"),
                vendor: s(v, "vendor"),
                semantic_role: s(v, "semantic_role"),
                vocabulary_path: s(v, "vocabulary_path"),
                transient_text: s(v, "transient_text"),
                subject: s(e, "subject"),
                object: s(e, "object"),
                verb: s(e, "verb"),
                durable_type: s(e, "durable_type"),
                topic_key: s(e, "topic_key"),
                value_key: s(e, "value_key"),
                action_command: s(&v["post_signal_action"], "command"),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The pipeline
// ---------------------------------------------------------------------------

/// A second `cairn-server` on the same database, started only for its
/// consolidation task: below five connections consolidation deliberately does
/// not run at all, so the way to get one is to run a server that qualifies.
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

fn settle(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + SETTLE;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    panic!("timed out waiting for: {what}");
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

/// The session-key field, and the edit and shell tool names each vendor uses.
fn vendor_shape(vendor: &str) -> (&'static str, &'static str, &'static str, &'static str) {
    match vendor {
        "claude_code" => ("claude-code", "session_id", "Edit", "Bash"),
        "codex" => ("codex", "thread_id", "apply_patch", "shell"),
        other => panic!("{other} is not in SC-701a's population"),
    }
}

/// Drive one scenario's whole session through the hook, in order.
///
/// The order is the scenario: vocabulary first, then the semantic turn, then
/// the action that proves it was acted on. The action is deliberately a
/// separate event from the vocabulary seed — the same file touched twice, at
/// two different points — because "expressed and then acted on" is a claim
/// about sequence and a single event cannot make it.
fn drive(device: &Device, s: &Scenario) -> String {
    let (agent, key_field, edit_tool, shell_tool) = vendor_shape(&s.vendor);
    let key = format!("{}-{}", s.id, Uuid::now_v7());

    let fire = |event: &str, payload: Value| {
        let out = device.sandbox.hook_as(agent, event, payload);
        assert!(
            out.ok(),
            "{}: {event} exited non-zero — a hook is never the reason a session \
             breaks: {}",
            s.id,
            out.stderr
        );
    };

    fire(
        "SessionStart",
        json!({ key_field: key, "source": "startup" }),
    );
    // 1. The vocabulary seed. Without it the signal's tokens are unjustified
    //    and capture declines them with `insufficient_vocabulary`, which is the
    //    behaviour working, not a bug.
    fire(
        "PostToolUse",
        json!({ key_field: key, "tool_name": edit_tool,
                "tool_input": { "file_path": s.vocabulary_path },
                "tool_response": { "exit_code": 0 } }),
    );
    // **Wait for the seed to be spooled before the semantic turn.**
    //
    // A hook returns as soon as the daemon has taken the payload, and the
    // *next* hook asks the daemon for the session's vocabulary. Fired
    // back-to-back with no wait, the signal's hook can win that race and be
    // handed an empty vocabulary — and then the token it names is genuinely
    // unjustified, so capture declines it. That is Cairn behaving correctly on
    // a sequence no human produces: a person types the next turn seconds later.
    // Waiting on the spool rather than sleeping keeps the fixture honest and
    // fast.
    let spooled = |n: i64| -> bool {
        device
            .sandbox
            .query_column(&format!(
                "SELECT CAST(count(*) AS TEXT) FROM event_spool
                  WHERE session_id IN (SELECT id FROM sessions WHERE agent_session_key = '{key}')"
            ))
            .first()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
            >= n
    };
    settle(&format!("{}: the vocabulary seed is spooled", s.id), || {
        spooled(2)
    });

    // 2. The semantic turn, in the vendor's own field.
    match s.semantic_role.as_str() {
        "assistant_message" => fire(
            "Stop",
            json!({ key_field: key, "last_assistant_message": s.transient_text }),
        ),
        "user_prompt" => fire(
            "UserPromptSubmit",
            json!({ key_field: key, "prompt": s.transient_text }),
        ),
        other => panic!("{other} is not a semantic role this corpus uses"),
    }
    settle(&format!("{}: the semantic turn is spooled", s.id), || {
        spooled(3)
    });

    // 3. Acted on, and only now — a different event, of a different kind, after
    //    the signal. The seed cannot stand in for this: "expressed and then
    //    acted on" is a claim about sequence.
    fire(
        "PostToolUse",
        json!({ key_field: key, "tool_name": shell_tool,
                "tool_input": { "command": s.action_command },
                "tool_response": { "exit_code": 0 } }),
    );
    fire("SessionEnd", json!({ key_field: key, "reason": "clear" }));

    // The session's own id, read from the local store. The server's `sessions`
    // has no `agent_session_key` — the key is a vendor-local handle — and the
    // id is the same on both sides because the client assigns it.
    device
        .sandbox
        .query_column(&format!(
            "SELECT id FROM sessions WHERE agent_session_key = '{key}'"
        ))
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("{}: the session was never created locally", s.id))
}

// ---------------------------------------------------------------------------
// SC-701a
// ---------------------------------------------------------------------------

/// Twenty qualifying sessions; at least fourteen produce a durable record with
/// the identity the corpus declared for them.
///
/// **Falsified by** counting signals instead of `memories`, by an action that
/// precedes its signal, by a population under twenty, by R7 or R8 failing to
/// persist, or by a subject or object that does not match.
#[test]
fn at_least_fourteen_of_twenty_sessions_produce_the_durable_record_they_declared() {
    let Some(server) = Server::start_own_database() else {
        eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
        return;
    };
    let _worker = Worker::start(&server.database_url);
    let scenarios = corpus();
    assert_eq!(
        scenarios.len(),
        QUALIFYING_SESSIONS,
        "the frozen corpus no longer holds exactly {QUALIFYING_SESSIONS} qualifying sessions"
    );

    // One project, so R8's aggregator can see the repeated instructions as
    // repetitions of one standing rule rather than as strangers.
    let device = device(&server, "sc701a-holdout");
    let project = device.project;

    let mut keys: BTreeMap<String, String> = BTreeMap::new();
    for s in &scenarios {
        keys.insert(s.id.clone(), drive(&device, s));
    }

    settle("every session reaches the server", || {
        server.count(&format!(
            "SELECT count(*) FROM sessions WHERE project_id = '{project}'"
        )) >= QUALIFYING_SESSIONS as i64
    });
    settle("the semantic events are accepted", || {
        server.count(&format!(
            "SELECT count(*) FROM safe_events
              WHERE project_id = '{project}'
                AND kind IN ('decision_signal','user_instruction_signal')"
        )) > 0
    });
    settle("consolidation produces durable knowledge", || {
        server.count(&format!(
            "SELECT count(*) FROM memories
              WHERE project_id = '{project}' AND origin_kind = 'consolidated'
                AND (topic_key LIKE 'decision.%' OR topic_key LIKE 'instruction.%')"
        )) > 0
    });
    // Give the aggregator rules a chance to see the whole corpus rather than
    // whichever half had arrived when the first run fired.
    std::thread::sleep(Duration::from_secs(5));

    let mut hits: Vec<&Scenario> = Vec::new();
    let mut misses: Vec<String> = Vec::new();
    let mut decisions = 0usize;
    let mut conventions = 0usize;

    for s in &scenarios {
        let session_id = &keys[&s.id];
        // The durable record, named by the identity the corpus declared, whose
        // provenance reaches this session's own semantic event.
        let durable = server.count(&format!(
            "SELECT count(*) FROM memories m
              WHERE m.project_id = '{project}'
                AND m.origin_kind = 'consolidated'
                AND m.type = '{}'
                AND m.topic_key = '{}'
                AND m.value_key = '{}'
                AND EXISTS (
                      SELECT 1 FROM knowledge_candidates kc
                        JOIN candidate_source_events cse ON cse.candidate_id = kc.candidate_id
                        JOIN safe_events e ON e.event_id = cse.event_id
                        JOIN sessions sess ON sess.id = e.session_id
                       WHERE kc.result_knowledge_id = m.id
                         AND e.session_id = '{session_id}'
                         AND e.kind IN ('decision_signal','user_instruction_signal'))",
            s.durable_type, s.topic_key, s.value_key
        ));
        // Acted on, and *after* the signal. Sequence, not mere presence — and
        // an **action**, not merely a later event.
        //
        // "any event after the signal" is the weaker check and it passes when
        // the action is moved *before* the signal, because a session close is
        // also an event after the signal. The corpus's action is a command, so
        // the kinds a command produces are what this counts.
        let acted_after = server.count(&format!(
            "SELECT count(*) FROM safe_events act
              WHERE act.session_id = '{session_id}'
                AND act.kind IN ('command_executed','test_executed','test_result')
                AND act.session_seq > (
                      SELECT MIN(sig.session_seq) FROM safe_events sig
                       WHERE sig.session_id = act.session_id
                         AND sig.kind IN ('decision_signal','user_instruction_signal'))"
        ));

        if durable > 0 && acted_after > 0 {
            hits.push(s);
            match s.durable_type.as_str() {
                "decision" => decisions += 1,
                "convention" => conventions += 1,
                other => panic!("{other} is not a durable type this corpus declares"),
            }
        } else {
            misses.push(format!(
                "{} (durable={durable}, acted_after={acted_after})",
                s.id
            ));
        }
    }

    eprintln!(
        "SC-701a: {}/{} durable matches — {decisions} decisions, {conventions} conventions",
        hits.len(),
        QUALIFYING_SESSIONS
    );
    // **Both mechanisms have to have fired.** SC-701a already says a run whose
    // records are all structural fails it even if SC-701 passes; the same
    // reasoning applies one level down. The corpus is fourteen decisions and
    // six standing instructions precisely so that R7 and R8 are both exercised,
    // and fourteen decisions alone land exactly on the floor — so breaking R8
    // entirely would otherwise pass this test at 14/20 while contributing
    // nothing. A population built to test two rules has not tested them if one
    // of them produced nothing.
    assert!(
        decisions > 0 && conventions > 0,
        "SC-701a: {decisions} decisions and {conventions} conventions. Both R7 \
         and R8 have to have produced something, or half the corpus is proving \
         nothing"
    );
    assert!(
        hits.len() >= MINIMUM_DURABLE_MATCHES,
        "SC-701a: only {} of {QUALIFYING_SESSIONS} sessions produced the durable \
         record they declared, and the criterion requires at least \
         {MINIMUM_DURABLE_MATCHES}. A run whose records are all structural leaves \
         the feature's actual purpose untested.\nmissed: {misses:#?}",
        hits.len()
    );
}

// ---------------------------------------------------------------------------
// SC-701b, on the same corpus and at the durable boundary
// ---------------------------------------------------------------------------

/// No word of the originating prompt or assistant turn reaches a durable record
/// unless that session's own vocabulary independently established it.
///
/// # How this is checked, and why not by scanning for words
///
/// The obvious check — take every word of the transient text, see which
/// survived into the record, require each to be in the vocabulary — reports
/// `for` on every scenario, because R7 writes "…`<object>` **for**
/// `<subject>`." and the prompt happened to use the same preposition. That is
/// the extractor's own connective prose, not material that crossed the
/// boundary, and a test that cannot tell them apart is a test that will be
/// silenced with a stopword list.
///
/// So the check is exact instead. R7 and R8 publish their sentences
/// (`contracts/extraction.md` §13), and every one is a fixed template with two
/// slots. This renders the template from the corpus's *declared* identity and
/// requires the durable content to equal it — so anything the record contains
/// beyond the template and its two tokens is a leak by construction, with no
/// judgement call about which words count. Then the two slots themselves are
/// required to be vocabulary-established, which is the actual claim: the only
/// thing that crossed is a token the session's own accepted events justified.
///
/// **Falsified by** letting an ungrounded transient word into the record, and
/// by any extractor prose that varies with the input.
#[test]
fn no_ungrounded_word_of_the_transient_turn_reaches_a_durable_record() {
    let Some(server) = Server::start_own_database() else {
        eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
        return;
    };
    let _worker = Worker::start(&server.database_url);
    let scenarios = corpus();
    let device = device(&server, "sc701b-holdout");
    let project = device.project;

    let mut ids: BTreeMap<String, String> = BTreeMap::new();
    for s in &scenarios {
        ids.insert(s.id.clone(), drive(&device, s));
    }
    // **Waited for at the threshold this test then asserts, not at one record.**
    //
    // Waiting for the first durable record and sleeping five seconds makes the
    // sleep load-bearing: on a busy runner the remaining nineteen are still
    // being consolidated when it expires, and the vacuity guard below then
    // fails for a reason that has nothing to do with SC-701b. Observed on CI
    // as "only 19 durable records were inspected". Settling on the population
    // the guard requires removes the guess; the deadline is `SETTLE`, so a
    // consolidation that genuinely stopped still fails here rather than
    // silently grading three records.
    settle(
        "durable knowledge appears for the graded population",
        || {
            server.count(&format!(
                "SELECT count(*) FROM memories
              WHERE project_id = '{project}' AND origin_kind = 'consolidated'
                AND (topic_key LIKE 'decision.%' OR topic_key LIKE 'instruction.%')"
            )) >= MINIMUM_DURABLE_MATCHES as i64
        },
    );
    // Then a moment more, so records arriving after the threshold are graded
    // too. This is opportunistic — the floor above is what the assertions rest
    // on — and it is why the guard reports how many were actually inspected.
    std::thread::sleep(Duration::from_secs(5));

    let mut leaks: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for s in &scenarios {
        let session_id = &ids[&s.id];
        // The sentence the rule writes, rendered from what the corpus declared
        // rather than from what the record says.
        let expected = match s.durable_type.as_str() {
            "decision" => format!(
                "This project decided to {} {} for {}.",
                s.verb, s.object, s.subject
            ),
            "convention" => format!("Here, {} is {} for {}.", s.object, s.verb, s.subject),
            other => panic!("{other} is not a durable type this corpus declares"),
        };
        let contents: Vec<String> = server.query_column(&format!(
            "SELECT m.content FROM memories m
              WHERE m.project_id = '{project}' AND m.origin_kind = 'consolidated'
                AND m.topic_key = '{}' AND m.value_key = '{}'",
            s.topic_key, s.value_key
        ));
        for content in &contents {
            checked += 1;
            if content != &expected {
                leaks.push(format!(
                    "{}: the durable content is `{content}`, and the rule's own \
                     sentence for this identity is `{expected}`. Anything beyond \
                     the template and its two tokens came from somewhere, and \
                     the only somewhere available is the transient turn",
                    s.id
                ));
            }
        }

        // The two slots are the only thing that crossed, and each has to have
        // been established by this session's own accepted events — derived
        // from `safe_events`, never from the corpus, which would be grading the
        // transient text against itself.
        let vocabulary: Vec<String> = server.query_column(&format!(
            "SELECT DISTINCT lower(regexp_split_to_table(e.content::text, '[^a-zA-Z0-9]+'))
               FROM safe_events e
              WHERE e.session_id = '{session_id}'
                AND e.kind NOT IN ('decision_signal','user_instruction_signal')"
        ));
        for token in [&s.subject, &s.object] {
            if !vocabulary.iter().any(|v| v == token) {
                leaks.push(format!(
                    "{}: `{token}` is in the durable record but no accepted event \
                     of that session establishes it",
                    s.id
                ));
            }
        }
    }

    // **The floor is SC-701a's threshold, not the corpus size**, and the
    // difference is the whole of this repair.
    //
    // SC-701b is a claim about the records that exist — "zero durable records
    // contain any word from the originating prompt or assistant turn that is
    // not independently present in that session's derived vocabulary". How many
    // records *must* exist is SC-701a's question, and its answer is fourteen of
    // twenty, stated as a threshold precisely because consolidation over a
    // holdout corpus is not required to be exhaustive.
    //
    // Demanding twenty here therefore asserted more than either criterion
    // grants: a nineteen-record run satisfies SC-701a, is fully measurable for
    // SC-701b, and failed anyway. A shortfall below fourteen is a real result,
    // and it belongs to `at_least_fourteen_of_twenty_sessions_produce_the_durable_record_they_declared`,
    // which reports it as the SC-701a failure it is. This guard exists only so
    // an empty or near-empty sample cannot pass for a proof.
    assert!(
        checked >= MINIMUM_DURABLE_MATCHES,
        "only {checked} durable records were inspected, fewer than the \
         {MINIMUM_DURABLE_MATCHES} SC-701a guarantees over {QUALIFYING_SESSIONS} \
         qualifying sessions, so this proved less than it claims. A shortfall \
         this large is an SC-701a failure, and the sibling test that names \
         fourteen is where it is graded"
    );
    assert!(
        leaks.is_empty(),
        "SC-701b: transient prompt or assistant prose crossed into durable \
         knowledge:\n{leaks:#?}"
    );
}
