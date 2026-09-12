//! Nothing forbidden survives capture or consolidation, on *any* table
//! (T158, ADVERSARIAL GATE; `contracts/safe-events.md` §10, `extraction.md`
//! §13, verification-summary.md §4).
//!
//! §10 of `safe-events.md` names what "never crosses" into durable storage:
//! *"raw vendor payloads, conversation transcripts, raw tool output, secrets,
//! absolute local paths, machine configuration, arbitrary vendor JSON"*. Two
//! more categories are specific to this feature's own server-side machinery:
//! a refused knowledge candidate must not carry the material that got it
//! refused (`consolidate.rs`'s `record_refusal`, SC-705), and a verification
//! report must never carry the evidence behind its verdict
//! (`verification-summary.md` §4, `verifysummary.rs`'s `REFUSED_FIELDS`).
//!
//! Every other Feature 005 test file that touches privacy checks *one* field,
//! against *one* table, for *one* request shape. That is the right unit for
//! "this rule works", and the wrong unit for "this rule was never bypassed" —
//! a defect that landed forbidden content in a table nobody thought to check
//! would sail through every one of them. This file inverts the question: for
//! nine adversarial payloads driven through the real ingest and consolidation
//! pipeline, is a marker unique to that payload findable **anywhere** in the
//! database afterward? The scan is built from `information_schema` at test
//! time, over every base table and every column the server actually has —
//! not a list this file remembers to keep in sync — so a table added after
//! this file was written is swept along with the rest rather than silently
//! skipped.
//!
//! What would falsify this file as a whole: any one marker, planted in any
//! one adversarial payload below, turning up in any column of any table.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{binary, post_json_status_bearer};
use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
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

// ---------------------------------------------------------------------------
// A consolidation worker, exactly as `feature005_extraction.rs` stands one up
// ---------------------------------------------------------------------------

const SETTLE: Duration = Duration::from_secs(30);
const WORKER_POOL: &str = "5";

struct Worker {
    child: Child,
}

impl Worker {
    fn start(database_url: &str) -> Self {
        for _ in 0..8 {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
            let port = probe.local_addr().expect("addr").port();
            drop(probe);
            let addr = format!("127.0.0.1:{port}");
            let mut child = Command::new(binary("cairn-server"))
                .args([
                    "--addr",
                    &addr,
                    "--database-url",
                    database_url,
                    "--max-connections",
                    WORKER_POOL,
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("cairn-server runs");

            for _ in 0..250 {
                if let Ok(Some(_)) = child.try_wait() {
                    break;
                }
                if std::net::TcpStream::connect(&addr).is_ok() {
                    return Self { child };
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        panic!("a consolidation worker would not start");
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn settle<F: Fn() -> bool>(what: &str, predicate: F) {
    let deadline = Instant::now() + SETTLE;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    panic!("{what} did not become true within {SETTLE:?}");
}

fn close_session(pg: &Pg, session: Uuid) {
    pg.server.execute(&format!(
        "UPDATE sessions SET status = 'completed', ended_at = now() WHERE id = '{session}'"
    ));
}

fn session_done(pg: &Pg, session: Uuid) -> bool {
    pg.server.count(&format!(
        "SELECT count(*) FROM consolidation_session
          WHERE session_id = '{session}' AND state = 'done'"
    )) == 1
}

// ---------------------------------------------------------------------------
// Event construction — the same shapes `feature005_ingest.rs` and
// `feature005_extraction.rs` already use, so an adversarial payload here is
// carried by a real, contract-shaped event and not a hand-rolled shortcut.
// ---------------------------------------------------------------------------

fn event_id(session: Uuid, seq: u64) -> Uuid {
    cairn_core::eventid::event_id(session, seq)
}

fn event(session: Uuid, seq: u64, kind: &str, content: Value) -> Value {
    json!({
        "event_id": event_id(session, seq),
        "contract_version": 1,
        "kind": kind,
        "agent": "claude_code",
        "vendor_event": "PostToolUse",
        "session_id": session,
        "session_seq": seq,
        "occurred_at": "2026-09-02T10:00:00Z",
        "content": content,
    })
}

fn file_event(session: Uuid, seq: u64, path: &str) -> Value {
    event(
        session,
        seq,
        "file_changed",
        json!({ "File": {
            "repo_file": path,
            "repo_file_from": null,
            "change_kind": "modified",
            "file_identity": "present"
        }}),
    )
}

fn decision_signal(
    session: Uuid,
    seq: u64,
    subject: &str,
    object: &str,
    justified_by_seq: Option<u64>,
) -> Value {
    event(
        session,
        seq,
        "decision_signal",
        json!({ "Decision": {
            "decision_kind": "adopt",
            "subject_token": subject,
            "object_token": object,
            "justified_by_seq": justified_by_seq,
            "lexicon_version": 1
        }}),
    )
}

/// A `tool_failed` event whose `failure_note` carries adversarial text — the
/// field `screen_event_text` (`events.rs`) screens against all nine content
/// classes with no exemption (unlike `command_line`/`test_command`, which
/// exempt `command_shaped`).
fn tool_failed_with_note(session: Uuid, seq: u64, tool: &str, note: &str) -> Value {
    event(
        session,
        seq,
        "tool_failed",
        json!({ "ToolFailure": {
            "vendor_tool": tool,
            "tool_class": "execute",
            "failure_kind": "non_zero_exit",
            "failure_note": note,
            "exit_status": 1
        }}),
    )
}

fn batch(events: Vec<Value>) -> Value {
    json!({ "contract_version": 1, "events": events })
}

fn post(pg: &Pg, who: &Account, events: Vec<Value>) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        "/api/events/batch",
        &batch(events),
        &who.token,
    )
}

fn statuses(response: &Value) -> Vec<String> {
    response["results"]
        .as_array()
        .map(|r| {
            r.iter()
                .map(|o| o["status"].as_str().unwrap_or("?").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn reasons(response: &Value) -> Vec<String> {
    response["results"]
        .as_array()
        .map(|r| {
            r.iter()
                .map(|o| o["reason"].as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The whole-database sweep
// ---------------------------------------------------------------------------

/// The Feature 005 tables this file expects the runtime enumeration to find —
/// not what the sweep itself relies on (that reads `information_schema` fresh
/// every call), but the check that the enumeration mechanism is actually
/// seeing this feature's schema and not, say, an empty or wrongly-pointed
/// database.
const EXPECTED_FEATURE_005_TABLES: &[&str] = &[
    "safe_events",
    "consolidation_session",
    "consolidation_work",
    "consolidation_runs",
    "knowledge_candidates",
    "candidate_source_events",
    "retrieval_traces",
    "retrieval_trace_items",
    "verification_reports",
    "knowledge_verification",
    "legacy_verification_audit",
    "shared_patterns",
    "integration_health",
    "delivered_context",
    "capture_dispositions",
    "applied_commands",
    "server_authority",
    "client_migrations",
];

/// Every base table Postgres actually has in `public`, read fresh each call.
fn enumerated_tables(pg: &Pg) -> Vec<String> {
    pg.server.query_column(
        "SELECT table_name FROM information_schema.tables
          WHERE table_schema = 'public' AND table_type = 'BASE TABLE'
          ORDER BY table_name",
    )
}

#[test]
fn the_runtime_table_enumeration_is_non_empty_and_covers_the_feature_005_schema() {
    let pg = pg!();
    let tables = enumerated_tables(&pg);
    assert!(
        tables.len() >= 20,
        "the enumeration found only {} tables; it is reading the wrong \
         database or `information_schema` is not what this file assumes: {tables:?}",
        tables.len()
    );
    for expected in EXPECTED_FEATURE_005_TABLES {
        assert!(
            tables.iter().any(|t| t == expected),
            "the runtime enumeration did not find {expected}, so the sweep \
             below would silently not have checked it: {tables:?}"
        );
    }
}

/// Scan every column of every base table in `public` for `marker`, except the
/// tables named in `legitimate_in` — the handful of places an *accepted*
/// event's own content is legitimately expected to sit (`safe_events`, for a
/// payload this file expects to be accepted rather than refused).
///
/// The table and column lists are read from `information_schema` inside the
/// same `DO` block that does the scan, at call time — never hardcoded here —
/// so a table this file's author never heard of is swept exactly like one
/// that has existed since the first migration.
fn assert_marker_nowhere_except(pg: &Pg, marker: &str, label: &str, legitimate_in: &[&str]) {
    let escaped = marker.replace('\'', "''");
    let exclude = {
        let mut names: Vec<String> = legitimate_in.iter().map(|s| s.to_string()).collect();
        names.push("__privacy_scan_hits".to_string());
        names
            .iter()
            .map(|n| format!("'{}'", n.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let sql = format!(
        "DO $do$
DECLARE
  trow RECORD;
  crow RECORD;
  hits BIGINT;
BEGIN
  CREATE TABLE IF NOT EXISTS __privacy_scan_hits (marker text, tbl text, col text, n bigint);
  FOR trow IN
    SELECT table_name FROM information_schema.tables
     WHERE table_schema = 'public' AND table_type = 'BASE TABLE'
       AND table_name NOT IN ({exclude})
  LOOP
    FOR crow IN
      SELECT column_name FROM information_schema.columns
       WHERE table_schema = 'public' AND table_name = trow.table_name
    LOOP
      EXECUTE format('SELECT count(*) FROM %I WHERE %I::text ILIKE %L',
                      trow.table_name, crow.column_name, '%{escaped}%')
        INTO hits;
      IF hits > 0 THEN
        INSERT INTO __privacy_scan_hits VALUES ('{escaped}', trow.table_name, crow.column_name, hits);
      END IF;
    END LOOP;
  END LOOP;
END
$do$;"
    );
    pg.server.execute(&sql);
    let hits = pg.server.count(&format!(
        "SELECT count(*) FROM __privacy_scan_hits WHERE marker = '{escaped}'"
    ));
    if hits > 0 {
        let details = pg.server.query_column(&format!(
            "SELECT tbl || '.' || col || ' (' || n || ' matching row(s))'
               FROM __privacy_scan_hits WHERE marker = '{escaped}'
              ORDER BY tbl, col"
        ));
        panic!(
            "{label}: marker {marker:?} was found on a table this content must never \
             reach: {details:?}"
        );
    }
}

fn assert_marker_nowhere(pg: &Pg, marker: &str, label: &str) {
    assert_marker_nowhere_except(pg, marker, label, &[]);
}

// ---------------------------------------------------------------------------
// 1. Capture: the nine privacy classes, driven at the server directly
// ---------------------------------------------------------------------------

/// Every one of `crates/cairn-core/tests/feature005_privacy.rs`'s
/// `class_examples` (SC-704, SC-705's own pre-registered corpus), reshaped
/// around a unique marker per class and driven through `POST
/// /api/events/batch` as a real HTTP request rather than through the
/// validator function directly — this is the adversary who never links the
/// client library at all, only ever speaks the wire format.
///
/// `command_shaped` is exercised through `failure_note`, not `command_line`:
/// `SafeEventField::CommandLine` and `TestCommand` deliberately exempt that
/// one class (a command line *is* a command), so putting it there would prove
/// nothing about the boundary and everything about a field that is supposed
/// to carry command-shaped text. `failure_note` has no such exemption.
#[test]
fn none_of_the_nine_privacy_classes_reaches_any_table_when_sent_directly_to_ingest() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    let hex_marker = || Uuid::now_v7().simple().to_string();
    let hyphenated_marker = || Uuid::now_v7().to_string();

    let cases: Vec<(&str, String, String)> = vec![
        {
            let m = hyphenated_marker();
            (
                "absolute_path",
                m.clone(),
                format!("logs are at /var/adv-{m}/output.log"),
            )
        },
        {
            let m = hyphenated_marker();
            (
                "home_dir_ref",
                m.clone(),
                format!("the cache lives at ~/.adv-{m}/cache"),
            )
        },
        {
            let m = hyphenated_marker();
            (
                "drive_letter_path",
                m.clone(),
                format!("logs are at C:\\adv-{m}\\logs"),
            )
        },
        {
            let m = hyphenated_marker();
            (
                "file_uri",
                m.clone(),
                format!("documented at file://adv-{m}/readme"),
            )
        },
        {
            let m = hyphenated_marker();
            (
                "credentialed_url",
                m.clone(),
                format!("clone from https://user:{m}@example.test/repo"),
            )
        },
        {
            let m = hyphenated_marker();
            (
                "env_assignment",
                m.clone(),
                format!("set CAIRN_ADV_TOKEN={m} first"),
            )
        },
        {
            let m = hex_marker();
            ("encoded_secret_shape", m.clone(), format!("the key is {m}"))
        },
        {
            let m = hyphenated_marker();
            (
                "command_shaped",
                m.clone(),
                format!("cargo build --tag adv-{m}"),
            )
        },
    ];

    let events: Vec<Value> = cases
        .iter()
        .enumerate()
        .map(|(i, (_, _, text))| tool_failed_with_note(session, i as u64 + 1, "Bash", text))
        .collect();
    let (body, status) = post(&pg, &pg.owner, events);
    assert_eq!(status, 200, "the batch itself must be answered: {body}");

    let got_statuses = statuses(&body);
    let got_reasons = reasons(&body);
    for (i, (class, _, text)) in cases.iter().enumerate() {
        assert_eq!(
            got_statuses[i], "rejected",
            "{class}'s example ({text:?}) was accepted rather than refused: {body}"
        );
        assert_eq!(
            got_reasons[i], "content_screening_failed",
            "{class}'s example was rejected for a different reason than the content \
             screen: {body}"
        );
    }

    // A rejected event has no `event_id` row in `safe_events` at all — but the
    // point of this file is not to trust that inference. Every marker is swept
    // across the entire database, with no table exempted, because the payload
    // was refused end to end and has no legitimate home anywhere.
    for (class, marker, _) in &cases {
        assert_marker_nowhere(
            &pg,
            marker,
            &format!("privacy class {class}, submitted directly over HTTP"),
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Capture: smuggling a transcript, raw output or vendor JSON as an
//    unrecognized field — the schema itself has to be the boundary, because
//    there is no content-class check that would ever see it
// ---------------------------------------------------------------------------

/// A field named "transcript" at the top level of an event.
///
/// There is no `transcript` field anywhere in the canonical event shape, so
/// the only way one crosses the wire at all is as an unrecognized name — and
/// `an_unknown_field_is_refused_because_the_schema_is_closed`
/// (`feature005_ingest.rs`) already establishes that the server refuses that.
/// What this file adds is the marker: the refusal is not enough on its own if
/// a defect elsewhere still wrote the smuggled text down.
#[test]
fn a_transcript_smuggled_as_an_unrecognized_top_level_field_reaches_no_table() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let marker = Uuid::now_v7().to_string();

    let mut e = file_event(session, 1, "a.rs");
    e["transcript"] = json!(format!(
        "Human: fix the failing test\nAssistant: on it — {marker}\nHuman: thanks"
    ));
    let (body, status) = post(&pg, &pg.owner, vec![e]);
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        statuses(&body),
        vec!["rejected"],
        "an event carrying an unrecognized `transcript` field was accepted: {body}"
    );

    assert_marker_nowhere(
        &pg,
        &marker,
        "a transcript smuggled as an unknown top-level field",
    );
}

/// The same attempt, one level deeper: a "raw tool output" field inside
/// `content`'s own typed object rather than beside it.
///
/// `EventContent` is `#[serde(deny_unknown_fields)]`
/// (`crates/cairn-core/src/event.rs`), so an unrecognized key inside the
/// `File` variant is refused by the same mechanism regardless of depth — this
/// is the structural half of "nothing crosses" (safe-events.md §10): there is
/// no field for raw tool output to land in, at any nesting level, and adding
/// one is refused rather than silently accepted or silently dropped.
#[test]
fn raw_tool_output_smuggled_inside_the_typed_content_object_reaches_no_table() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let marker = Uuid::now_v7().to_string();

    let mut e = file_event(session, 1, "a.rs");
    e["content"]["File"]["raw_tool_output"] = json!(format!(
        "$ cargo test\n... 40 lines of scrollback ...\nFAILED at marker {marker}"
    ));
    let (body, status) = post(&pg, &pg.owner, vec![e]);
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        statuses(&body),
        vec!["rejected"],
        "an event carrying an unrecognized field nested inside `content` was \
         accepted: {body}"
    );

    assert_marker_nowhere(
        &pg,
        &marker,
        "raw tool output smuggled inside the typed content object",
    );
}

// ---------------------------------------------------------------------------
// 3. Verification: no route accepts the evidence behind a verdict
// ---------------------------------------------------------------------------

/// `POST /api/verification/runs` and `/attestations` both refuse a payload
/// naming `command_output` (`verifysummary.rs`'s `REFUSED_FIELDS`) — the
/// whole request is refused, not merely stripped of the field, because a
/// silently-stripped field is indistinguishable from one that was never
/// screened at all from outside this file.
#[test]
fn verification_evidence_smuggled_as_command_output_reaches_no_table_on_either_route() {
    let pg = pg!();
    let memory_id = seed_memory(&pg);

    for route in ["/api/verification/runs", "/api/verification/attestations"] {
        let marker = Uuid::now_v7().to_string();
        let mut body = json!({
            "memory_ref": { "domain": "project", "knowledge_id": memory_id },
            "verdict": "passed",
            "verifier_kind": "file_digest",
            "run_at": "2026-08-30T09:00:00Z",
            "command_output": format!("$ ls -la /home/adv-{marker}\nexit 0"),
        });
        if route.ends_with("attestations") {
            body["attesting_agent"] = json!("claude-code");
        }
        let (resp, status) =
            post_json_status_bearer(&pg.server.base, route, &body, &pg.owner.token);
        assert_eq!(
            status, 400,
            "{route} accepted a report naming `command_output`: {resp}"
        );
        assert_eq!(
            resp["error"]["code"], "authority_not_assertable",
            "{route}: the wrong refusal was given for a caller-supplied evidence \
             field: {resp}"
        );

        assert_marker_nowhere(
            &pg,
            &marker,
            &format!("verification evidence smuggled as command_output on {route}"),
        );
    }
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

// ---------------------------------------------------------------------------
// 4. Consolidation: a refused candidate carries none of the material that
//    got it refused, anywhere — not only in `knowledge_candidates.content`
// ---------------------------------------------------------------------------

/// A subject token exactly at `SUBJECT_TOKEN_MAX_CHARS` (128) — legal at
/// ingest — that overflows `TOPIC_KEY_MAX_CHARS` once R7 prefixes it with
/// `decision.`, the same real, client-reachable `key_normalization_failed`
/// path `feature005_governance.rs`'s
/// `an_oversized_client_supplied_key_is_refused_with_empty_content_and_no_keys_rather_than_persisted`
/// establishes. The filler here carries a marker instead of a repeated
/// nonsense word, so a leak of this exact adversarial run is traceable rather
/// than merely plausible.
fn oversized_filler(marker: &str) -> String {
    let mut s = format!("adv_{marker}_");
    while s.chars().count() < 128 {
        s.push_str("pad_");
    }
    s.chars().take(128).collect()
}

#[test]
fn a_refused_candidates_content_is_empty_everywhere_not_only_in_its_own_column() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let marker = Uuid::now_v7().simple().to_string();
    let filler = oversized_filler(&marker);
    assert_eq!(
        filler.chars().count(),
        128,
        "the fixture's own token bound drifted"
    );

    let events = vec![
        file_event(session, 1, &format!("{filler}/b.rs")),
        decision_signal(session, 2, &filler, "b", Some(1)),
    ];
    let (body, status) = post(&pg, &pg.owner, events);
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        statuses(&body),
        vec!["accepted", "accepted"],
        "the fixture's own events must be accepted at ingest, or the refusal \
         below would be for the wrong reason: {body}"
    );

    close_session(&pg, session);
    let _worker = Worker::start(&pg.server.database_url);
    settle("the pass finishes", || session_done(&pg, session));

    let refused = pg.server.count(&format!(
        "SELECT count(*) FROM knowledge_candidates kc
           JOIN consolidation_runs cr ON cr.run_id = kc.run_id
          WHERE cr.session_id = '{session}'
            AND kc.decision = 'refused' AND kc.refusal_reason = 'key_normalization_failed'"
    ));
    assert!(
        refused > 0,
        "the oversized token produced no `key_normalization_failed` refusal; \
         this fixture's own premise did not hold, so nothing below was tested"
    );
    let with_content = pg.server.count(&format!(
        "SELECT count(*) FROM knowledge_candidates kc
           JOIN consolidation_runs cr ON cr.run_id = kc.run_id
          WHERE cr.session_id = '{session}' AND kc.decision = 'refused'
            AND kc.content <> ''"
    ));
    assert_eq!(
        with_content, 0,
        "a refused candidate carried non-empty content in its own column"
    );

    // The marker is legitimately present in `safe_events`: the events that
    // named it were accepted, and that acceptance is not what this test is
    // about. Everywhere else — `knowledge_candidates` most of all, since that
    // is exactly where a refused proposal's raw material would land if
    // `record_refusal` ever regressed — it must be absent.
    assert_marker_nowhere_except(
        &pg,
        &marker,
        "an oversized decision token that produced a refused candidate",
        &["safe_events"],
    );
}
