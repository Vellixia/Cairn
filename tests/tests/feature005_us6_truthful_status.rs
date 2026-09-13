//! User Story 6, end to end: configure, exercise, and read a status that only
//! claims what was established (T137; SC-724, SC-725, SC-726, SC-765).
//!
//! # What makes this the *story* test
//!
//! The other US6 files each hold one mechanism still — authority, summaries,
//! the health matrix. This walks the sequence a person actually performs, in
//! order, and asks the status the question they would ask at each point:
//!
//! 1. Nothing has happened. Status must say *nothing is known*, and must not
//!    round that down to "not offered" or up to "working".
//! 2. Cairn is configured, and its configuration reads back correctly. Status
//!    must still not say the integration works — a file Cairn wrote and read
//!    back says nothing about whether anything ran.
//! 3. Something runs. Only now may a capability be reported as supported, and
//!    only for the capability that actually fired.
//! 4. Something fails. The failure names the stage it failed at, and does not
//!    smear over the capabilities that are fine.
//! 5. A verification is reported. It is `remote_attested`, whichever route
//!    carried it, and no wording in the request can make it more.
//!
//! # The thesis, in one line
//!
//! **Route names, payload fields, configuration presence and optimistic
//! interpretation must never manufacture verification or health.** Each step
//! below is a place where an optimistic implementation would say more than it
//! knows, and the assertions are written to catch exactly that — mostly as
//! statements about what the status must *not* say.

use cairn_e2e::feature005::Pg;
use cairn_e2e::{get_json_status_bearer, post_json_status_bearer};
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

/// The machine this story happens on.
const WRITER: &str = "us6-laptop";
const AGENT: &str = "claude_code";

fn report_health(pg: &Pg, cells: Vec<Value>) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        &format!("/api/projects/{}/health", pg.project),
        &json!({ "writer_id": WRITER, "cells": cells }),
        &pg.owner.token,
    )
}

fn read_health(pg: &Pg) -> Vec<Value> {
    let (body, status) = get_json_status_bearer(
        &pg.server.base,
        &format!("/api/projects/{}/health", pg.project),
        &pg.owner.token,
    );
    assert_eq!(status, 200, "reading health: {body}");
    body["cells"].as_array().cloned().unwrap_or_default()
}

fn cell_for<'a>(cells: &'a [Value], capability: &str) -> Option<&'a Value> {
    cells.iter().find(|c| c["capability"] == capability)
}

// ---------------------------------------------------------------------------
// The story
// ---------------------------------------------------------------------------

/// Configure, exercise, fail, verify — and at every step the status says only
/// what was established.
///
/// **Falsified by** any of: reporting a configured capability as supported;
/// accepting a failure with no observation; turning an absent report into a
/// state; letting either verification route assign anything but
/// `remote_attested`.
#[test]
fn a_status_read_at_every_step_claims_only_what_was_established() {
    let pg = pg!();

    // -----------------------------------------------------------------------
    // 1. Nothing has happened yet.
    // -----------------------------------------------------------------------
    let cells = read_health(&pg);
    assert!(
        cells.is_empty(),
        "a project nobody has reported on has no cells. Synthesizing them here \
         would make 'no report arrived' indistinguishable from 'reported as no \
         evidence', which is the distinction FR-855 exists to draw: {cells:?}"
    );

    // -----------------------------------------------------------------------
    // 2. Cairn is configured, and the configuration reads back. This is
    //    introspection: a fact about a file Cairn wrote, not about anything
    //    running.
    // -----------------------------------------------------------------------
    let (body, status) = report_health(
        &pg,
        vec![json!({
            "agent": AGENT,
            "capability": "event:tool_failed",
            "stage": "configured",
            "status": "no_evidence",
        })],
    );
    assert_eq!(status, 200, "a configuration report is legal: {body}");

    let cells = read_health(&pg);
    let configured = cell_for(&cells, "event:tool_failed").expect("the reported cell");
    assert_eq!(
        configured["status"],
        json!("no_evidence"),
        "**configured is not working.** Cairn wrote a hook and read it back; \
         nothing has fired. Reporting that as `supported` is the collapse FR-852 \
         draws the evidence-kind distinction to prevent: {configured}"
    );

    // And the server refuses to be told otherwise. A client claiming support on
    // configuration evidence has a bug worth knowing about, and storing a
    // weaker status quietly would hide it.
    let (body, status) = report_health(
        &pg,
        vec![json!({
            "agent": AGENT,
            "capability": "event:tool_failed",
            "stage": "configured",
            "status": "supported",
            "evidence_kind": "introspection",
            "observed_at": "2026-09-04T09:00:00Z",
        })],
    );
    assert_eq!(
        status, 400,
        "`supported` on introspection evidence was accepted, so a configuration \
         file can claim a capability works: {body}"
    );

    // -----------------------------------------------------------------------
    // 3. Something actually runs.
    // -----------------------------------------------------------------------
    let (body, status) = report_health(
        &pg,
        vec![json!({
            "agent": AGENT,
            "capability": "event:tool_failed",
            "stage": "runtime_hook_fired",
            "status": "supported",
            "evidence_kind": "observation",
            "observed_at": "2026-09-04T10:00:00Z",
        })],
    );
    assert_eq!(status, 200, "an observed success is legal: {body}");

    let cells = read_health(&pg);
    let fired = cells
        .iter()
        .find(|c| c["capability"] == "event:tool_failed" && c["stage"] == "runtime_hook_fired")
        .expect("the observed cell");
    assert_eq!(fired["status"], json!("supported"));
    assert_eq!(
        fired["evidence_kind"],
        json!("observation"),
        "a success has to carry the kind of evidence behind it, or a reader \
         cannot tell it from a configuration read-back: {fired}"
    );
    assert!(
        fired["observed_at"].is_string(),
        "the timestamp is what lets a reader decide the observation is still \
         current; the server deliberately offers no verdict of its own on that"
    );
    // The configured cell is untouched. A capability firing at one stage says
    // nothing about another, and the two rows are separate for that reason.
    assert!(
        cells.len() >= 2,
        "the observation replaced the configuration cell instead of joining it: \
         {cells:?}"
    );

    // -----------------------------------------------------------------------
    // 4. Something fails, at a named stage.
    // -----------------------------------------------------------------------
    let (body, status) = report_health(
        &pg,
        vec![json!({
            "agent": AGENT,
            "capability": "event:file_changed",
            "stage": "safe_event_accepted",
            "status": "runtime_failure",
            "evidence_kind": "observation",
            "observed_at": "2026-09-04T10:05:00Z",
        })],
    );
    assert_eq!(status, 200, "an observed failure is legal: {body}");

    let cells = read_health(&pg);
    let failed = cell_for(&cells, "event:file_changed").expect("the failing cell");
    assert_eq!(
        failed["stage"],
        json!("safe_event_accepted"),
        "**the stage survives.** 'capture is broken' and 'the server refused \
         what capture produced' call for different actions, and flattening both \
         into 'failing' loses the only thing that tells them apart: {failed}"
    );
    // A failure somewhere is not a failure everywhere.
    let still_fine = cells
        .iter()
        .find(|c| c["capability"] == "event:tool_failed" && c["stage"] == "runtime_hook_fired")
        .expect("the earlier success");
    assert_eq!(
        still_fine["status"],
        json!("supported"),
        "one capability failing changed another's status: {still_fine}"
    );

    // A failure with nothing behind it is refused, exactly as a success is.
    // Silence is not a fault, and turning it into one sends somebody to debug an
    // integration nobody has exercised.
    let (body, status) = report_health(
        &pg,
        vec![json!({
            "agent": AGENT,
            "capability": "event:session_resumed",
            "stage": "runtime_hook_fired",
            "status": "runtime_failure",
        })],
    );
    assert_eq!(
        status, 400,
        "a failure was accepted with no observation behind it: {body}"
    );

    // -----------------------------------------------------------------------
    // 5. A verification is reported, on the stronger-sounding route.
    // -----------------------------------------------------------------------
    let session = pg.session_for(&pg.owner);
    let memory = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content,
                               origin_session_id)
         VALUES ('{memory}', '{}', 'fact', 'project', '{}',
                 'the release job signs images', '{session}')",
        pg.project, pg.project
    ));

    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/verification/runs",
        &json!({
            "memory_ref": { "domain": "project", "knowledge_id": memory },
            "verdict": "passed",
            "verifier_kind": "test_outcome",
            "run_at": "2026-09-04T10:10:00Z",
        }),
        &pg.owner.token,
    );
    assert_eq!(status, 200, "a legal run report: {body}");
    assert_eq!(
        body["authority"],
        json!("remote_attested"),
        "`/runs` is the stronger-*sounding* route and is not a stronger trust \
         boundary. An authenticated client said a check ran; the server did not \
         watch it run (§4, SC-765): {body}"
    );

    // The record it produced says the same thing, and says it as derived state
    // rather than as anything the caller sent.
    let verified: Vec<String> = pg.server.query_column(&format!(
        "SELECT verification || '/' || COALESCE(verification_authority, '<none>')
           FROM memories WHERE id = '{memory}'"
    ));
    assert_eq!(
        verified,
        vec!["verified/remote_attested".to_string()],
        "the summary derived from one passing report should be `verified` with \
         the only authority an HTTP report can carry"
    );

    // -----------------------------------------------------------------------
    // 6. Nothing anywhere carries an authority no route may produce.
    // -----------------------------------------------------------------------
    assert_eq!(
        pg.server.count(
            "SELECT count(*) FROM verification_reports
              WHERE authority <> 'remote_attested'"
        ),
        0,
        "a report carries an authority no HTTP route may assign"
    );
    assert_eq!(
        pg.server.count(
            "SELECT count(*) FROM memories
              WHERE verification_authority IS NOT NULL
                AND verification_authority <> 'remote_attested'"
        ),
        0,
        "a summary carries an authority no report established"
    );
}

/// OpenCode's declines are Cairn's decisions, and they survive the round trip
/// as such.
///
/// The one status pair most likely to be confused, and the confusion is not
/// cosmetic: `unsupported_by_vendor` blames OpenCode for something Cairn chose.
/// The hooks exist — they are beta, and Cairn declines to rest an automatic
/// guarantee on them (FR-838b).
#[test]
fn what_cairn_declined_is_never_reported_as_something_the_vendor_lacks() {
    let pg = pg!();

    let declared = cairn_integrate::capability::declared_matrix("opencode");
    let declines: Vec<&cairn_integrate::capability::MatrixCell> = declared
        .iter()
        .filter(|c| c.status == cairn_integrate::capability::MatrixStatus::DeclinedByCairn)
        .collect();
    assert!(
        !declines.is_empty(),
        "OpenCode's baseline declines are the subject of this test; if there \
         are none, it proves nothing"
    );

    let cells: Vec<Value> = declines
        .iter()
        .map(|c| serde_json::to_value(c).expect("a cell serializes"))
        .collect();
    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        &format!("/api/projects/{}/health", pg.project),
        &json!({ "writer_id": WRITER, "cells": cells }),
        &pg.owner.token,
    );
    assert_eq!(status, 200, "declines are legal to report: {body}");

    let stored = read_health(&pg);
    for decline in &declines {
        let row = stored
            .iter()
            .find(|r| r["agent"] == "opencode" && r["capability"] == json!(decline.capability))
            .unwrap_or_else(|| panic!("{} is missing from the read", decline.capability));
        assert_eq!(
            row["status"],
            json!("declined_by_cairn"),
            "{} came back as {} — a Cairn decision reported as a vendor absence \
             blames OpenCode for a choice Cairn made, and a vendor absence \
             reported as a decision does the reverse",
            decline.capability,
            row["status"]
        );
    }
}
