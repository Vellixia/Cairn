//! Retrieval's deadline behaviour, measured rather than assumed (T081,
//! FR-835, FR-836, `contracts/retrieval-delivery.md` §5).
//!
//! # What is actually being pinned
//!
//! Not "retrieval is fast". A latency assertion on shared CI hardware measures
//! the runner, and a test that fails when a machine is busy teaches people to
//! re-run rather than to look. What FR-835 and FR-836 ask for is stronger and
//! cheaper to hold:
//!
//! - the level reached is one of exactly four, and never a fifth;
//! - the level is recorded on the answer **and** in the trace, and the two agree;
//! - **wall-clock latency is never itself an input to briefing content** — two
//!   retrievals over identical inputs at the same declared level return
//!   byte-identical content, however long either took.
//!
//! The last one is the falsifiable form of the claim, and it is the one a
//! caching or timing bug would break. Latency is recorded *about* a retrieval;
//! it must never leak *into* one.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::post_json_status_bearer;
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

/// The four levels §5 declares, and there is no fifth.
const LEVELS: &[&str] = &["full", "reduced", "minimal", "none"];

fn seed_project_memory(pg: &Pg, session: Uuid, how_many: usize) {
    for i in 0..how_many {
        pg.server.execute(&format!(
            "INSERT INTO memories
                (id, project_id, type, scope, scope_key, content, state, origin_session_id,
                 topic_key, value_key, origin_kind)
             VALUES ('{}', '{}', 'fact', 'project', '{}', 'project fact number {i}', 'active',
                     '{session}', 'topic.n{i}', 'v{i}', 'explicit')",
            Uuid::now_v7(),
            pg.project,
            pg.project
        ));
    }
}

fn retrieve(pg: &Pg, who: &Account, session: Uuid, trigger: &str) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        "/api/retrieve",
        &json!({ "session_id": session, "trigger": trigger }),
        &who.token,
    )
}

/// Everything about an answer except the parts that are *about* the retrieval
/// rather than *in* it.
///
/// `trace_id` and the trace's latency identify one attempt; the content is what
/// two attempts must agree on.
fn content_only(answer: &Value) -> Value {
    json!({
        "delivery_point": answer["delivery_point"],
        "degradation_level": answer["degradation_level"],
        "budget": answer["budget"],
        "sections": answer["sections"],
    })
}

#[test]
fn the_level_is_one_of_exactly_four_and_the_trace_agrees_with_the_answer() {
    // FR-836. The level is pre-declared and recorded in both places; a fifth
    // value, or two places disagreeing, would make "how degraded was this"
    // unanswerable from the record.
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    seed_project_memory(&pg, session, 3);

    for trigger in ["session_open", "prompt_submit", "explicit"] {
        let (answer, status) = retrieve(&pg, &pg.owner, session, trigger);
        assert_eq!(status, 200, "{trigger}: {answer}");
        let level = answer["degradation_level"].as_str().expect("a level");
        assert!(
            LEVELS.contains(&level),
            "{trigger} reached {level:?}, which is not one of the four declared levels"
        );

        let trace_id = answer["trace_id"].as_str().expect("a trace");
        let recorded = pg.server.text(&format!(
            "SELECT degradation_level FROM retrieval_traces WHERE trace_id = '{trace_id}'"
        ));
        assert_eq!(
            recorded, level,
            "{trigger}: the trace and the answer disagree about how degraded the briefing was"
        );
    }
}

#[test]
fn latency_is_recorded_about_a_retrieval_and_never_inside_one() {
    // FR-835, and the reason it is written as a comparison rather than a
    // threshold: identical inputs at the same declared level must produce an
    // identical briefing, so the only way latency could be an input is if two
    // runs that took different times differed in content.
    //
    // `explicit` is the trigger to compare on, because it is exempt from dedup
    // — a second `session_open` would legitimately differ once the first was
    // transmitted, and that difference would hide the one being looked for.
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    seed_project_memory(&pg, session, 6);

    let (first, _) = retrieve(&pg, &pg.owner, session, "explicit");
    let (second, _) = retrieve(&pg, &pg.owner, session, "explicit");

    assert_eq!(
        first["degradation_level"], second["degradation_level"],
        "two identical retrievals declared different levels"
    );
    assert_eq!(
        content_only(&first),
        content_only(&second),
        "two retrievals over identical inputs returned different content"
    );
    assert_ne!(
        first["trace_id"], second["trace_id"],
        "two retrievals shared one trace, so one of them is unrecorded"
    );

    // Each attempt recorded its own latency, and the two are free to differ —
    // which is the point. A recorded number that had to match would be a number
    // the content depended on.
    for answer in [&first, &second] {
        let trace_id = answer["trace_id"].as_str().expect("a trace");
        let latency: i64 = pg
            .server
            .text(&format!(
                "SELECT latency_ms::text FROM retrieval_traces WHERE trace_id = '{trace_id}'"
            ))
            .parse()
            .expect("a recorded latency");
        assert!(latency >= 0, "a retrieval reported a negative duration");
    }
}

#[test]
fn a_prompt_time_retrieval_is_measured_against_a_tighter_budget_than_a_session_open_one() {
    // §4: the two points cannot restate each other, so prompt time gets 25% of
    // the briefing budget. This is the budget half of the deadline story — the
    // tighter point is tighter in both dimensions, and the one that is
    // deterministic is the one worth asserting.
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    seed_project_memory(&pg, session, 2);

    let (open, _) = retrieve(&pg, &pg.owner, session, "session_open");
    let (prompt, _) = retrieve(&pg, &pg.owner, session, "prompt_submit");

    let open_budget = open["budget"]["tokens"].as_u64().expect("a budget");
    let prompt_budget = prompt["budget"]["tokens"].as_u64().expect("a budget");
    assert_eq!(
        prompt_budget,
        (open_budget as f64 * 0.25).floor() as u64,
        "prompt time did not get a quarter of the session-open budget"
    );
    assert_eq!(
        open["delivery_point"], "session_open",
        "a session-open retrieval was aimed somewhere else"
    );
    assert_eq!(prompt["delivery_point"], "prompt_time");
}

#[test]
fn every_retrieval_reports_a_spend_within_the_budget_it_declared() {
    // SC-709, asserted at every delivery point rather than only the generous
    // one: a budget that holds at 3000 and not at 750 is not a budget.
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    seed_project_memory(&pg, session, 40);

    for trigger in ["session_open", "prompt_submit", "explicit"] {
        let (answer, status) = retrieve(&pg, &pg.owner, session, trigger);
        assert_eq!(status, 200, "{trigger}: {answer}");
        let tokens = answer["budget"]["tokens"].as_u64().expect("a budget");
        let spent = answer["budget"]["spent"].as_u64().expect("a spend");
        assert!(
            spent <= tokens,
            "{trigger} spent {spent} of a {tokens} budget"
        );

        // And the reported spend is the sum of what was admitted, so the
        // number is an account of the selection rather than a summary beside
        // it (SC-711).
        let mut summed = 0u64;
        if let Some(sections) = answer["sections"].as_object() {
            for items in sections.values() {
                for item in items.as_array().into_iter().flatten() {
                    summed += item["cost"].as_u64().expect("a cost");
                }
            }
        }
        assert_eq!(
            summed, spent,
            "{trigger}: the reported spend is not the cost of what was delivered"
        );
    }
}

#[test]
fn a_degraded_level_never_appears_without_the_trace_saying_so() {
    // FR-836 again, from the other side: the level is recorded *on the
    // briefing and in its trace*. A briefing that degraded silently would leave
    // a reader unable to tell a thin project from a truncated answer.
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    seed_project_memory(&pg, session, 1);

    let (answer, _) = retrieve(&pg, &pg.owner, session, "session_open");
    let trace_id = answer["trace_id"].as_str().expect("a trace");

    let state = pg.server.text(&format!(
        "SELECT delivery_state FROM retrieval_traces WHERE trace_id = '{trace_id}'"
    ));
    assert_eq!(
        state, "generated",
        "a successful retrieval left its trace in {state:?}"
    );
    // A generated trace always carries a level and a latency: the schema's own
    // CHECKs say so, and this asserts the server satisfies them rather than
    // trusting that it must.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM retrieval_traces
              WHERE trace_id = '{trace_id}'
                AND degradation_level IS NOT NULL AND latency_ms IS NOT NULL"
        )),
        1,
        "a generated trace is missing its level or its latency"
    );
}
