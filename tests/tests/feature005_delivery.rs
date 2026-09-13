//! Delivery outcomes, and what may be claimed from them (T067,
//! `contracts/retrieval-delivery.md` §3, §6.2, §7, invariants 11 and 12).
//!
//! # The distinction the whole file is about
//!
//! Generating a briefing is not evidence that an agent received one. The two
//! are separate states written by separate parties at separate moments, and
//! every test here exists because collapsing them would let the first stand in
//! for the second — which is the overclaim Principle X forbids and the reason
//! `delivered_context` is written by the outcome report and never by selection.
//!
//! Acknowledgement is a third step further still, and nothing in this feature
//! can reach it: no named vendor mechanism establishes receipt for any
//! committed agent, so it stays `unavailable / no evidence` after every outcome
//! (FR-838e, FR-844, SC-712).

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

fn seed(pg: &Pg, session: Uuid, content: &str) {
    pg.server.execute(&format!(
        "INSERT INTO memories
            (id, project_id, type, scope, scope_key, content, state, origin_session_id,
             topic_key, value_key, origin_kind)
         VALUES ('{}', '{}', 'fact', 'project', '{}', '{content}', 'active', '{session}',
                 'topic.{}', 'settled', 'explicit')",
        Uuid::now_v7(),
        pg.project,
        pg.project,
        Uuid::now_v7().simple()
    ));
}

fn retrieve(pg: &Pg, who: &Account, session: Uuid) -> Value {
    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/retrieve",
        &json!({ "session_id": session, "trigger": "session_open" }),
        &who.token,
    );
    assert_eq!(status, 200, "retrieve: {body}");
    body
}

fn report(pg: &Pg, who: &Account, trace: &str, body: Value) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        &format!("/api/retrieval-traces/{trace}/transmission"),
        &body,
        &who.token,
    )
}

fn trace_id(answer: &Value) -> String {
    answer["trace_id"].as_str().expect("a trace").to_string()
}

fn delivered_rows(pg: &Pg, session: Uuid) -> i64 {
    pg.server.count(&format!(
        "SELECT count(*) FROM delivered_context WHERE session_id = '{session}'"
    ))
}

fn state_of(pg: &Pg, trace: &str) -> String {
    pg.server.text(&format!(
        "SELECT delivery_state FROM retrieval_traces WHERE trace_id = '{trace}'"
    ))
}

/// A session with something to deliver, and its generated trace.
fn generated(pg: &Pg) -> (Uuid, String) {
    let session = pg.session_for(&pg.owner);
    seed(pg, session, "a durable project fact");
    let answer = retrieve(pg, &pg.owner, session);
    (session, trace_id(&answer))
}

#[test]
fn a_reported_transmission_failure_leaves_a_failed_trace_and_no_delivery_rows() {
    // Invariant 11. Writing a delivery row for a transmission that failed
    // would make dedup withhold, for the life of the session, an item the
    // agent never received — the dedup enforcing a delivery that did not
    // happen.
    let pg = pg!();
    let (session, trace) = generated(&pg);

    let (body, status) = report(
        &pg,
        &pg.owner,
        &trace,
        json!({ "outcome": "failed", "failure_reason": "hook_transmission_failed" }),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(state_of(&pg, &trace), "failed");
    assert_eq!(
        pg.server.text(&format!(
            "SELECT failure_reason FROM retrieval_traces WHERE trace_id = '{trace}'"
        )),
        "hook_transmission_failed"
    );
    assert_eq!(
        delivered_rows(&pg, session),
        0,
        "a failed transmission wrote delivery rows"
    );
}

#[test]
fn a_reported_success_records_exactly_the_selected_items_as_delivered() {
    // Invariant 11, the other half: the rows come from the trace's own selected
    // items, so the daemon can neither widen what was delivered nor backdate it.
    let pg = pg!();
    let (session, trace) = generated(&pg);

    let selected = pg.server.count(&format!(
        "SELECT count(*) FROM retrieval_trace_items
          WHERE trace_id = '{trace}' AND status = 'selected'"
    ));
    assert!(selected > 0, "nothing was selected, so this proves nothing");

    let (body, status) = report(&pg, &pg.owner, &trace, json!({ "outcome": "transmitted" }));
    assert_eq!(status, 200, "{body}");
    assert_eq!(state_of(&pg, &trace), "transmitted");
    assert_eq!(delivered_rows(&pg, session), selected);
}

#[test]
fn repeating_the_same_outcome_is_a_duplicate_with_no_second_effect() {
    // A retry after a lost response is the ordinary case this endpoint is built
    // for. Answering it with an error would make the daemon choose between
    // reporting twice and not reporting at all.
    let pg = pg!();
    let (session, trace) = generated(&pg);
    report(&pg, &pg.owner, &trace, json!({ "outcome": "transmitted" }));

    let before_rows = delivered_rows(&pg, session);
    let before_at = pg.server.text(&format!(
        "SELECT max(delivered_at)::text FROM delivered_context WHERE session_id = '{session}'"
    ));

    let (body, status) = report(&pg, &pg.owner, &trace, json!({ "outcome": "transmitted" }));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "duplicate");
    assert_eq!(delivered_rows(&pg, session), before_rows);
    assert_eq!(
        pg.server.text(&format!(
            "SELECT max(delivered_at)::text FROM delivered_context WHERE session_id = '{session}'"
        )),
        before_at,
        "a duplicate report moved the delivery timestamp, so it had a second effect"
    );
}

#[test]
fn an_opposite_terminal_outcome_is_refused_and_the_first_one_stands() {
    // Whichever was true, one of the two reports is wrong. Taking the later one
    // would make the record agree with the last caller rather than with what
    // happened.
    let pg = pg!();
    let (_, trace) = generated(&pg);
    report(&pg, &pg.owner, &trace, json!({ "outcome": "transmitted" }));

    let (body, status) = report(
        &pg,
        &pg.owner,
        &trace,
        json!({ "outcome": "failed", "failure_reason": "hook_transmission_failed" }),
    );
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "outcome_conflict");
    assert_eq!(state_of(&pg, &trace), "transmitted");
}

#[test]
fn a_foreign_account_and_an_unknown_trace_are_answered_identically() {
    // A caller that could tell "not yours" from "no such trace" could
    // enumerate which traces exist, one guess at a time.
    let pg = pg!();
    let (_, trace) = generated(&pg);

    let (foreign, foreign_status) =
        report(&pg, &pg.member, &trace, json!({ "outcome": "transmitted" }));
    let (unknown, unknown_status) = report(
        &pg,
        &pg.owner,
        &Uuid::now_v7().to_string(),
        json!({ "outcome": "transmitted" }),
    );

    assert_eq!(foreign_status, 404);
    assert_eq!(unknown_status, 404);
    assert_eq!(
        foreign, unknown,
        "the two refusals differ, so one of them identifies a real trace"
    );
    assert_eq!(
        state_of(&pg, &trace),
        "generated",
        "a foreign report changed the trace it was refused for"
    );
}

#[test]
fn a_trace_that_never_generated_has_no_transmission_to_report() {
    // `requested` means selection never finished. There is nothing whose
    // transmission could have succeeded or failed.
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let trace = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO retrieval_traces
            (trace_id, project_id, session_id, account_id, trigger, delivery_point,
             budget_tokens, delivery_state)
         VALUES ('{trace}', '{}', '{session}', '{}', 'session_open', 'session_open', 3000,
                 'requested')",
        pg.project, pg.owner.id
    ));

    let (body, status) = report(
        &pg,
        &pg.owner,
        &trace.to_string(),
        json!({ "outcome": "transmitted" }),
    );
    assert_eq!(status, 409, "{body}");
    assert_eq!(state_of(&pg, &trace.to_string()), "requested");
}

#[test]
fn the_report_accepts_an_outcome_and_a_declared_reason_and_nothing_else() {
    // Everything else the server already holds, and everything beyond that is
    // authority a caller must not be able to assert. The type is closed, so an
    // extra field is refused rather than ignored — an ignored `account_id`
    // reads to the caller exactly like an accepted one.
    let pg = pg!();
    let (_, trace) = generated(&pg);

    for forged in [
        json!({ "outcome": "transmitted", "acknowledgement_state": "acknowledged" }),
        json!({ "outcome": "transmitted", "account_id": Uuid::now_v7() }),
        json!({ "outcome": "transmitted", "project_id": Uuid::now_v7() }),
        json!({ "outcome": "transmitted", "reference_key": "knowledge:project:x" }),
        json!({ "outcome": "failed", "failure_reason": "something_else_entirely" }),
        json!({ "outcome": "failed" }),
    ] {
        let (body, status) = report(&pg, &pg.owner, &trace, forged.clone());
        assert!(
            status >= 400,
            "the boundary accepted {forged}: {status} {body}"
        );
    }
    assert_eq!(
        state_of(&pg, &trace),
        "generated",
        "a refused report changed the trace anyway"
    );
}

#[test]
fn acknowledgement_stays_unavailable_after_every_outcome() {
    // SC-712. Zero agents report acknowledgement as confirmed, and zero report
    // it as unsupported-by-vendor either — the evidence licenses neither.
    let pg = pg!();
    let (_, transmitted) = generated(&pg);
    report(
        &pg,
        &pg.owner,
        &transmitted,
        json!({ "outcome": "transmitted" }),
    );
    let (_, failed) = generated(&pg);
    report(
        &pg,
        &pg.owner,
        &failed,
        json!({ "outcome": "failed", "failure_reason": "hook_transmission_deadline_exceeded" }),
    );

    assert_eq!(
        pg.server.count(
            "SELECT count(*) FROM retrieval_traces WHERE acknowledgement_state <> 'unavailable'"
        ),
        0,
        "a trace claimed an acknowledgement no vendor mechanism establishes"
    );
}

#[test]
fn a_compact_session_open_is_the_post_compaction_restoration_point() {
    // FR-838d. There is no post-compaction delivery point of its own — at least
    // one committed vendor's post-compaction event cannot carry returned
    // context at all — so restoration is reached through the next session open,
    // distinguished by its trigger.
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    seed(&pg, session, "something worth restoring");

    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/retrieve",
        &json!({ "session_id": session, "trigger": "session_open", "open_trigger": "compact" }),
        &pg.owner.token,
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["restored_after_compaction"], true);
    assert_eq!(body["open_trigger"], "compact");
    assert_eq!(body["delivery_point"], "session_open");
}

#[test]
fn an_open_trigger_is_refused_where_it_does_not_belong_and_when_it_is_not_one() {
    // A misspelled `compact` would silently deliver an ordinary session-open
    // briefing instead of a restoration, and an unvalidated string is a
    // free-text field on an otherwise closed boundary.
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    for body in [
        json!({ "session_id": session, "trigger": "session_open", "open_trigger": "compacted" }),
        json!({ "session_id": session, "trigger": "prompt_submit", "open_trigger": "compact" }),
        json!({ "session_id": session, "trigger": "explicit", "open_trigger": "startup" }),
    ] {
        let (answer, status) =
            post_json_status_bearer(&pg.server.base, "/api/retrieve", &body, &pg.owner.token);
        assert!(status >= 400, "accepted {body}: {status} {answer}");
    }
}

#[test]
fn only_the_two_committed_agents_await_delivery_evidence_and_opencode_is_declined() {
    // SC-708's population, and the distinction FR-838b insists on: OpenCode's
    // absence is Cairn's decision about a beta surface, never a claim that the
    // vendor cannot do it — OpenCode 2 does expose the hooks.
    use cairn_integrate::capability::{declared_matrix, MatrixStatus};

    for agent in ["claude_code", "codex"] {
        for cell in declared_matrix(agent)
            .into_iter()
            .filter(|c| c.capability.starts_with("deliver:"))
        {
            assert_eq!(
                cell.status,
                MatrixStatus::NoEvidence,
                "{agent}:{} is declared rather than awaiting an observation",
                cell.capability
            );
        }
    }
    for cell in declared_matrix("opencode")
        .into_iter()
        .filter(|c| c.capability.starts_with("deliver:"))
    {
        assert_eq!(
            cell.status,
            MatrixStatus::DeclinedByCairn,
            "{}",
            cell.capability
        );
        assert_ne!(cell.status, MatrixStatus::UnsupportedByVendor);
    }
    // And receipt is no-evidence everywhere, for the same reason and not this
    // one: nothing was found, which is not the same as nothing existing.
    for agent in ["claude_code", "codex", "opencode"] {
        let receipt = declared_matrix(agent)
            .into_iter()
            .find(|c| c.capability == "receipt")
            .expect("a receipt cell");
        assert_eq!(receipt.status, MatrixStatus::NoEvidence, "{agent}");
    }
}
