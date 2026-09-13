//! Retrieval traces, as a contract (T066, `contracts/retrieval-delivery.md`
//! §6, §6.1, §12.0-§12.2).
//!
//! US2's independent test: knowledge is seeded directly with SQL, never
//! through the capture path, so these assertions stand on their own.
//!
//! What matters here is not "does a trace exist" — it does, trivially — but:
//!
//! - **Every retrieval is recorded, including one that fails** (SC-729): a
//!   trace's lifecycle is `requested -> generated -> transmitted | failed`,
//!   written by different parties at different moments, and a failure is
//!   observable rather than absent.
//! - **A trace never carries the rendered briefing** (FR-839): only
//!   identities, budget accounting and a degradation level.
//! - **Ownership withholds a row entirely** (SC-761, §6.1): a colleague's
//!   `personal_notes`/`patterns` item is dropped, never rendered opaque, and
//!   the surviving ranks are re-densified so the gap cannot betray it
//!   (§12.2). Budget figures are scoped to the trace's own account for the
//!   same reason.
//! - **90-day retention actually sweeps** (FR-847), and **a non-member gets a
//!   flat refusal**, never a partial trace.
//!
//! # Why a generation failure is not faked with a raw SQL row
//!
//! The task packet for this file allows asserting the `failed` shape by
//! direct SQL insert if a failure cannot be induced honestly through the API.
//! It can, here, in the form the API actually produces: `POST
//! .../transmission {"outcome":"failed", ...}` moves a real, generated trace
//! to `delivery_state = 'failed'` with a bounded `failure_reason`, in the same
//! transaction shape a generation-stage failure would use (`failed`, a
//! reason, and — because that branch never runs the `delivered_context`
//! upsert — zero delivery rows). That is a genuine execution path, not a
//! fixture standing in for one, so it is what this file uses. A
//! *generation*-stage failure (the store itself unreachable mid-selection)
//! would additionally require making the fixture's own database misbehave for
//! one request without disturbing any other test sharing it, which this
//! harness has no safe way to do — noted here rather than faked.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{binary, get_json_status_bearer, post_json_status_bearer};
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

/// How long a property is given to become true before the test calls it false.
const SETTLE: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Request helpers
// ---------------------------------------------------------------------------

fn retrieve(pg: &Pg, who: &Account, session: Uuid, trigger: &str) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        "/api/retrieve",
        &json!({ "session_id": session, "trigger": trigger }),
        &who.token,
    )
}

fn report_failed(pg: &Pg, who: &Account, trace_id: &str, reason: &str) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        &format!("/api/retrieval-traces/{trace_id}/transmission"),
        &json!({ "outcome": "failed", "failure_reason": reason }),
        &who.token,
    )
}

fn get_trace(pg: &Pg, who: &Account, trace_id: &str) -> (Value, u16) {
    get_json_status_bearer(
        &pg.server.base,
        &format!("/api/retrieval-traces/{trace_id}"),
        &who.token,
    )
}

// ---------------------------------------------------------------------------
// Seeding helpers — knowledge is planted directly with SQL (US2 is
// independent of Story 1's capture path).
// ---------------------------------------------------------------------------

fn escape(s: &str) -> String {
    s.replace('\'', "''")
}

fn seed_project_memory(pg: &Pg, session: Uuid, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    let content = escape(content);
    pg.server.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content, origin_session_id)
         VALUES ('{id}', '{}', 'fact', 'project', '{}', '{content}', '{session}')",
        pg.project, pg.project
    ));
    id
}

fn seed_personal(pg: &Pg, owner: &Account, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    let content = escape(content);
    pg.server.execute(&format!(
        "INSERT INTO personal_knowledge (id, owner_user_id, knowledge_type, content, writer_id, writer_seq)
         VALUES ('{id}', '{}', 'fact', '{content}', 'writer-{id}', 1)",
        owner.id
    ));
    id
}

fn seed_team_authoritative(pg: &Pg, author: &Account, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    let content = escape(content);
    pg.server.execute(&format!(
        "INSERT INTO team_knowledge
            (id, knowledge_type, content, state, proposed_by_user_id,
             ratified_by_user_id, ratified_at, writer_id, writer_seq)
         VALUES ('{id}', 'fact', '{content}', 'authoritative', '{}', '{}', now(),
                 'writer-{id}', 1)",
        author.id, author.id
    ));
    id
}

/// A rich scenario: one project item, one team item, one personal item and
/// one pattern, all belonging to `owner`, delivered in a single retrieval.
/// Returns the trace id.
fn deliver_a_mixed_briefing(pg: &Pg, owner: &Account) -> (String, Uuid, Uuid, Uuid, Uuid) {
    let session = pg.session_for(owner);
    let project_id = seed_project_memory(pg, session, "project truth the whole project may read");
    let pattern_id = Uuid::now_v7();
    pg.seed_pattern_with_id(owner, pattern_id, "a pattern only its owner may read");
    let personal_id = seed_personal(pg, owner, "a personal note only its owner may read");
    let team_id = seed_team_authoritative(pg, owner, "team guidance the whole project may read");

    let (resp, status) = retrieve(pg, owner, session, "session_open");
    assert_eq!(status, 200, "{resp}");
    let trace_id = resp["trace_id"].as_str().expect("trace_id").to_string();
    (trace_id, project_id, pattern_id, personal_id, team_id)
}

// ---------------------------------------------------------------------------
// A second server on the fixture's database, for the retention sweep
// (mirrors feature005_consolidation_claims.rs: the fixture's own server has a
// pool of four, and the sweep only runs from a pool share of at least one,
// which needs at least five connections, FR-793a1).
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// SC-729 — every retrieval, including a failed one, is recorded
// ---------------------------------------------------------------------------

#[test]
fn sc_729_a_normal_retrieval_persists_a_generated_trace() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let (resp, status) = retrieve(&pg, &pg.owner, session, "session_open");
    assert_eq!(status, 200, "{resp}");
    let trace_id = resp["trace_id"].as_str().expect("trace_id");

    assert_eq!(
        pg.server.text(&format!(
            "SELECT delivery_state FROM retrieval_traces WHERE trace_id = '{trace_id}'"
        )),
        "generated"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM retrieval_traces WHERE trace_id = '{trace_id}'"
        )),
        1,
        "the retrieval left no persisted trace"
    );
}

#[test]
fn the_trace_carries_a_created_at_and_a_generated_trace_carries_latency_and_a_degradation_level() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let (resp, status) = retrieve(&pg, &pg.owner, session, "session_open");
    assert_eq!(status, 200, "{resp}");
    let trace_id = resp["trace_id"].as_str().expect("trace_id");

    // The row exists from the moment of `requested`, before selection or
    // rendering — `created_at` is what proves that moment happened at all.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM retrieval_traces
              WHERE trace_id = '{trace_id}' AND created_at IS NOT NULL"
        )),
        1
    );
    // Once generation completes, the two fields a degraded-vs-failed
    // distinction and a latency measurement both depend on are filled in —
    // the DDL's own CHECK enforces this shape; this asserts the end state a
    // real retrieval actually reaches.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM retrieval_traces
              WHERE trace_id = '{trace_id}' AND delivery_state = 'generated'
                AND latency_ms IS NOT NULL AND degradation_level IS NOT NULL"
        )),
        1
    );
}

#[test]
fn a_reported_transmission_failure_becomes_failed_with_a_reason_and_writes_no_delivery_rows() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let item_id = seed_project_memory(&pg, session, "an item whose transmission will fail");
    let (opened, status) = retrieve(&pg, &pg.owner, session, "session_open");
    assert_eq!(status, 200, "{opened}");
    assert!(opened["sections"]["project_memory"].is_array(), "{opened}");
    let trace_id = opened["trace_id"].as_str().expect("trace_id");

    let (report, status) = report_failed(&pg, &pg.owner, trace_id, "hook_transmission_failed");
    assert_eq!(status, 200, "{report}");
    assert_eq!(report["delivery_state"], json!("failed"), "{report}");

    assert_eq!(
        pg.server.text(&format!(
            "SELECT delivery_state FROM retrieval_traces WHERE trace_id = '{trace_id}'"
        )),
        "failed"
    );
    assert_eq!(
        pg.server.text(&format!(
            "SELECT failure_reason FROM retrieval_traces WHERE trace_id = '{trace_id}'"
        )),
        "hook_transmission_failed"
    );
    // `generated -> failed` writes no delivery rows: dedup must never
    // suppress an item the agent did not actually receive (invariant 11).
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM delivered_context
              WHERE session_id = '{session}'
                AND reference_key = 'knowledge:project:{item_id}'"
        )),
        0,
        "a failed transmission still populated delivered_context"
    );
}

#[test]
fn the_failed_shape_the_ddl_enforces_matches_what_a_real_failure_writes() {
    let pg = pg!();
    // The CHECK is symmetric — `failed` requires a reason, and only `failed`
    // may have one. Both directions are refused.
    pg.refuses(
        "a failed trace with no failure_reason",
        &format!(
            "INSERT INTO retrieval_traces
                (trace_id, project_id, session_id, account_id, trigger, delivery_point,
                 delivery_state, latency_ms, failure_reason)
             VALUES ('{}', '{}', '{}', '{}', 'session_open', 'session_open',
                     'failed', 5, NULL)",
            Uuid::now_v7(),
            pg.project,
            pg.session_for(&pg.owner),
            pg.owner.id
        ),
    );
    pg.refuses(
        "a generated trace carrying a failure_reason",
        &format!(
            "INSERT INTO retrieval_traces
                (trace_id, project_id, session_id, account_id, trigger, delivery_point,
                 delivery_state, latency_ms, degradation_level, failure_reason)
             VALUES ('{}', '{}', '{}', '{}', 'session_open', 'session_open',
                     'generated', 5, 'full', 'store_unreachable')",
            Uuid::now_v7(),
            pg.project,
            pg.session_for(&pg.owner),
            pg.owner.id
        ),
    );
}

// ---------------------------------------------------------------------------
// Complete considered/selected refs, canonical reference_key shape
// ---------------------------------------------------------------------------

#[test]
fn a_trace_records_both_considered_and_selected_items_each_with_a_canonical_reference_key() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // Two items compete for the same, small budget slice by being personal
    // notes (subject to the fixed global cap): the first fits, the second
    // does not, so both a `selected` and a `considered` row exist for one
    // retrieval without needing a huge fixture.
    let big = "y".repeat(4000); // comfortably larger than the global cap alone
    seed_personal(
        &pg,
        &pg.owner,
        "a short personal note that will be selected",
    );
    seed_personal(&pg, &pg.owner, &big);

    let (resp, status) = retrieve(&pg, &pg.owner, session, "session_open");
    assert_eq!(status, 200, "{resp}");
    let trace_id = resp["trace_id"].as_str().expect("trace_id");

    let (trace, status) = get_trace(&pg, &pg.owner, trace_id);
    assert_eq!(status, 200, "{trace}");
    let items = trace["items"].as_array().expect("items");

    let statuses: std::collections::BTreeSet<&str> =
        items.iter().filter_map(|i| i["status"].as_str()).collect();
    assert!(statuses.contains("selected"), "{trace}");
    assert!(statuses.contains("considered"), "{trace}");

    for item in items {
        let key = item["reference_key"].as_str().expect("reference_key");
        let ref_kind = item["ref_kind"].as_str().expect("ref_kind");
        match ref_kind {
            "knowledge" => {
                let domain = item["domain"].as_str().expect("domain on a knowledge ref");
                assert_eq!(
                    key,
                    format!(
                        "knowledge:{domain}:{}",
                        item["knowledge_id"].as_str().unwrap()
                    ),
                    "malformed knowledge reference_key: {item}"
                );
            }
            "pattern" => {
                assert!(
                    item["domain"].is_null(),
                    "a pattern ref carried a domain: {item}"
                );
                assert_eq!(
                    key,
                    format!("pattern:{}", item["knowledge_id"].as_str().unwrap()),
                    "malformed pattern reference_key: {item}"
                );
            }
            other => panic!("ref_kind outside the union: {other}"),
        }
    }
}

// ---------------------------------------------------------------------------
// No briefing text in a trace (FR-839)
// ---------------------------------------------------------------------------

#[test]
fn no_column_of_either_trace_table_contains_a_seeded_records_content() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let marker = "XYZZY-CONTENT-THAT-MUST-NEVER-REACH-A-TRACE-COLUMN-93172";
    seed_project_memory(&pg, session, marker);
    seed_personal(&pg, &pg.owner, marker);
    seed_team_authoritative(&pg, &pg.owner, marker);
    pg.seed_pattern_with_id(&pg.owner, Uuid::now_v7(), marker);

    let (resp, status) = retrieve(&pg, &pg.owner, session, "session_open");
    assert_eq!(status, 200, "{resp}");
    // Confirm the marker really was delivered (the response itself carries
    // content, which is what makes the absence below meaningful rather than
    // vacuous).
    assert!(
        resp.to_string().contains(marker),
        "the fixture's content never reached the briefing at all: {resp}"
    );

    for table in ["retrieval_traces", "retrieval_trace_items"] {
        let columns = pg.server.query_column(&format!(
            "SELECT column_name FROM information_schema.columns
              WHERE table_schema = 'public' AND table_name = '{table}'"
        ));
        for column in columns {
            let hits = pg.server.count(&format!(
                "SELECT count(*) FROM {table} WHERE {column}::text LIKE '%{marker}%'"
            ));
            assert_eq!(hits, 0, "{table}.{column} carried the seeded content");
        }
    }
}

// ---------------------------------------------------------------------------
// SC-761 / §6.1 — owner withholding, and §12.2 — dense ranks, scoped budgets
// ---------------------------------------------------------------------------

#[test]
fn sc_761_a_co_member_reading_the_trace_never_sees_the_owners_personal_notes_or_patterns() {
    let pg = pg!();
    let (trace_id, project_id, pattern_id, personal_id, team_id) =
        deliver_a_mixed_briefing(&pg, &pg.owner);

    let (as_member, status) = get_trace(&pg, &pg.member, &trace_id);
    assert_eq!(status, 200, "{as_member}");
    let items = as_member["items"].as_array().expect("items");

    let project_key = format!("knowledge:project:{project_id}");
    let team_key = format!("knowledge:team:{team_id}");
    let personal_key = format!("knowledge:personal:{personal_id}");
    let pattern_key = format!("pattern:{pattern_id}");

    let keys: std::collections::BTreeSet<&str> = items
        .iter()
        .filter_map(|i| i["reference_key"].as_str())
        .collect();
    assert!(keys.contains(project_key.as_str()), "{as_member}");
    assert!(keys.contains(team_key.as_str()), "{as_member}");
    assert!(
        !keys.contains(personal_key.as_str()),
        "a co-member could see the owner's personal note: {as_member}"
    );
    assert!(
        !keys.contains(pattern_key.as_str()),
        "a co-member could see the owner's pattern: {as_member}"
    );

    // Withheld entirely, not rendered as an opaque handle: nothing in the
    // response should even name the withheld ids.
    let rendered = as_member.to_string();
    assert!(!rendered.contains(&personal_id.to_string()), "{as_member}");
    assert!(!rendered.contains(&pattern_id.to_string()), "{as_member}");

    // The owner, reading their own trace, sees all four.
    let (as_owner, status) = get_trace(&pg, &pg.owner, &trace_id);
    assert_eq!(status, 200, "{as_owner}");
    let owner_keys: std::collections::BTreeSet<&str> = as_owner["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|i| i["reference_key"].as_str())
        .collect();
    for key in [&project_key, &team_key, &personal_key, &pattern_key] {
        assert!(owner_keys.contains(key.as_str()), "{as_owner}");
    }
}

#[test]
fn sc_12_2_the_withheld_reader_sees_dense_ranks_one_through_n_with_no_gaps() {
    let pg = pg!();
    let (trace_id, ..) = deliver_a_mixed_briefing(&pg, &pg.owner);

    let (as_member, status) = get_trace(&pg, &pg.member, &trace_id);
    assert_eq!(status, 200, "{as_member}");
    let selected: Vec<i64> = as_member["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|i| i["status"] == json!("selected"))
        .map(|i| i["rank"].as_i64().expect("rank"))
        .collect();
    assert!(!selected.is_empty(), "{as_member}");
    let mut sorted = selected.clone();
    sorted.sort();
    let expected: Vec<i64> = (1..=sorted.len() as i64).collect();
    assert_eq!(
        sorted, expected,
        "ranks were not dense after the authorization filter: {as_member}"
    );
}

#[test]
fn sc_12_2_budget_and_latency_are_scoped_to_the_traces_own_account() {
    let pg = pg!();
    let (trace_id, ..) = deliver_a_mixed_briefing(&pg, &pg.owner);

    let (as_owner, status) = get_trace(&pg, &pg.owner, &trace_id);
    assert_eq!(status, 200, "{as_owner}");
    assert!(as_owner.get("budget").is_some(), "{as_owner}");
    assert!(as_owner.get("latency_ms").is_some(), "{as_owner}");

    let (as_member, status) = get_trace(&pg, &pg.member, &trace_id);
    assert_eq!(status, 200, "{as_member}");
    assert!(
        as_member.get("budget").is_none(),
        "a non-owning reader was given budget figures: {as_member}"
    );
    assert!(
        as_member.get("latency_ms").is_none(),
        "a non-owning reader was given latency: {as_member}"
    );
    // The degradation level is not scoped — every reader gets it.
    assert!(as_member.get("degradation_level").is_some(), "{as_member}");
}

// ---------------------------------------------------------------------------
// 90-day retention (FR-847)
// ---------------------------------------------------------------------------

#[test]
fn ninety_day_old_traces_are_swept_by_the_background_task() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let old_trace = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO retrieval_traces
            (trace_id, project_id, session_id, account_id, trigger, delivery_point,
             delivery_state, degradation_level, budget_tokens, budget_spent, latency_ms,
             created_at, updated_at)
         VALUES ('{old_trace}', '{}', '{session}', '{}', 'session_open', 'session_open',
                 'generated', 'full', 3000, 0, 5,
                 now() - interval '91 days', now() - interval '91 days')",
        pg.project, pg.owner.id
    ));
    // A fresh trace, well inside the window, as the control: retention must
    // not become "delete everything".
    let (fresh, status) = retrieve(&pg, &pg.owner, session, "session_open");
    assert_eq!(status, 200, "{fresh}");
    let fresh_trace = fresh["trace_id"].as_str().expect("trace_id").to_string();

    let _worker = Worker::start(&pg.server.database_url);
    let deadline = Instant::now() + SETTLE;
    loop {
        let gone = pg.server.count(&format!(
            "SELECT count(*) FROM retrieval_traces WHERE trace_id = '{old_trace}'"
        )) == 0;
        if gone {
            break;
        }
        assert!(Instant::now() < deadline, "the retention sweep never ran");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM retrieval_traces WHERE trace_id = '{fresh_trace}'"
        )),
        1,
        "the sweep removed a trace inside its retention window"
    );
}

// ---------------------------------------------------------------------------
// Membership refusal
// ---------------------------------------------------------------------------

#[test]
fn a_non_member_reading_a_trace_gets_404_trace_not_found_never_a_partial_trace() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let (opened, status) = retrieve(&pg, &pg.owner, session, "session_open");
    assert_eq!(status, 200, "{opened}");
    let trace_id = opened["trace_id"].as_str().expect("trace_id");

    let (resp, status) = get_trace(&pg, &pg.outsider, trace_id);
    assert_eq!(status, 404, "{resp}");
    assert_eq!(resp["error"]["code"], json!("trace_not_found"), "{resp}");
    // Never a partial trace: no field a real trace carries should leak
    // through a refusal.
    assert!(resp.get("items").is_none(), "{resp}");
    assert!(resp.get("trigger").is_none(), "{resp}");
    assert!(resp.get("degradation_level").is_none(), "{resp}");
}
