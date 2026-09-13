//! Health and disposition reporting, the shared primitive (T035, FR-851–FR-860).
//!
//! The matrix's value is entirely in its honesty. Its failure mode is not being
//! wrong, it is being *confident*: a cell that says `supported` because a
//! config file was read back, or a cell that is simply absent and renders as
//! neither working nor unknown. So the assertions here are mostly about what
//! the server refuses to be told.

use cairn_e2e::feature005::Pg;
use cairn_e2e::{get_json_status_bearer, post_json_status_bearer, post_status_bearer};
use serde_json::{json, Value};

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

fn cell(capability: &str, status: &str, evidence: Option<&str>, observed: bool) -> Value {
    let mut c = json!({
        "agent": "claude_code",
        "capability": capability,
        "stage": "runtime_hook_fired",
        "status": status,
    });
    if let Some(e) = evidence {
        c["evidence_kind"] = json!(e);
    }
    if observed {
        c["observed_at"] = json!("2026-09-02T10:00:00Z");
    }
    c
}

fn report(pg: &Pg, cells: Vec<Value>) -> u16 {
    post_status_bearer(
        &pg.server.base,
        &format!("/api/projects/{}/health", pg.project),
        &json!({ "writer_id": "laptop-a", "cells": cells }),
        &pg.owner.token,
    )
}

#[test]
fn an_observed_capability_is_accepted_and_read_back() {
    let pg = pg!();
    assert_eq!(
        report(
            &pg,
            vec![cell(
                "event:tool_failed",
                "supported",
                Some("observation"),
                true
            )]
        ),
        200
    );
    let (body, code) = get_json_status_bearer(
        &pg.server.base,
        &format!("/api/projects/{}/health", pg.project),
        &pg.owner.token,
    );
    assert_eq!(code, 200, "{body}");
    let cells = body["cells"].as_array().expect("cells");
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0]["status"], "supported");
    assert_eq!(cells[0]["evidence_kind"], "observation");
    assert_eq!(
        cells[0]["writer_id"], "laptop-a",
        "the machine that observed it was not recorded (FR-857)"
    );
}

#[test]
fn configuration_read_back_cannot_claim_that_something_works() {
    let pg = pg!();
    // FR-852. Reading a hook out of a settings file establishes that Cairn
    // wrote it, not that the agent ever fired it, and a matrix that showed both
    // the same way would report confidence it has not earned.
    assert_eq!(
        report(
            &pg,
            vec![cell(
                "event:tool_failed",
                "supported",
                Some("introspection"),
                true
            )]
        ),
        400
    );
    assert_eq!(
        report(
            &pg,
            vec![cell("event:tool_failed", "supported", None, false)],
        ),
        400,
        "a behavioural claim was accepted with no evidence at all"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM integration_health WHERE project_id = '{}'",
            pg.project
        )),
        0
    );
}

#[test]
fn a_cell_claiming_no_evidence_may_not_carry_any() {
    let pg = pg!();
    assert_eq!(
        report(
            &pg,
            vec![cell("receipt", "no_evidence", Some("observation"), true)]
        ),
        400,
        "a no-evidence cell carried an observation, which reads as confidence"
    );
    // And the honest form is accepted.
    assert_eq!(
        report(&pg, vec![cell("receipt", "no_evidence", None, false)]),
        200
    );
}

#[test]
fn a_cairn_decision_is_not_reported_as_a_vendor_absence() {
    let pg = pg!();
    // OpenCode delivery is the live case: the hooks exist and are beta, so
    // declining to guarantee delivery is Cairn's decision. Reporting it as
    // `unsupported_by_vendor` would be untrue (FR-838b), and both statuses
    // exist so the difference can be said.
    for status in ["declined_by_cairn", "unsupported_by_vendor"] {
        assert_eq!(
            report(&pg, vec![cell("deliver:prompt_time", status, None, false)]),
            200,
            "{status} was refused"
        );
    }
    let stored = pg.server.query_column(&format!(
        "SELECT DISTINCT status FROM integration_health WHERE project_id = '{}'",
        pg.project
    ));
    assert_eq!(
        stored.len(),
        1,
        "two reports of one cell should leave the last one, not merge them"
    );
}

#[test]
fn a_capability_the_matrix_has_no_cell_for_is_refused() {
    let pg = pg!();
    for unknown in [
        "event:prompt_submitted",
        "deliver:telepathy",
        "",
        "receipts",
    ] {
        assert_eq!(
            report(&pg, vec![cell(unknown, "no_evidence", None, false)]),
            400,
            "{unknown:?} was accepted as a capability"
        );
    }
    // An unknown *status* is refused too: a cell rendering as an unrecognized
    // string is a blank cell wearing a value.
    assert_eq!(
        report(&pg, vec![cell("receipt", "probably_fine", None, false)]),
        400
    );
}

#[test]
fn a_report_is_bounded_and_authorized() {
    let pg = pg!();
    let many: Vec<Value> = (0..1001)
        .map(|_| cell("receipt", "no_evidence", None, false))
        .collect();
    assert_eq!(report(&pg, many), 400, "an unbounded report was accepted");

    // A non-member files health for nothing.
    assert_eq!(
        post_status_bearer(
            &pg.server.base,
            &format!("/api/projects/{}/health", pg.project),
            &json!({ "writer_id": "w", "cells": [] }),
            &pg.outsider.token
        ),
        403
    );
    assert_eq!(
        post_status_bearer(
            &pg.server.base,
            &format!("/api/projects/{}/health", pg.project),
            &json!({ "writer_id": "w", "cells": [] }),
            "not-a-real-token"
        ),
        401
    );
}

#[test]
fn a_report_must_name_the_machine_it_observed_on() {
    let pg = pg!();
    // A capability is observed on a machine (FR-857). One account on two
    // laptops can legitimately see two answers, and a report with no machine
    // would let a working one silently overwrite a broken one.
    let code = post_status_bearer(
        &pg.server.base,
        &format!("/api/projects/{}/health", pg.project),
        &json!({ "cells": [cell("receipt", "no_evidence", None, false)] }),
        &pg.owner.token,
    );
    assert_eq!(code, 400);
}

#[test]
fn two_machines_reporting_one_capability_are_two_cells() {
    let pg = pg!();
    for writer in ["laptop-a", "laptop-b"] {
        let code = post_status_bearer(
            &pg.server.base,
            &format!("/api/projects/{}/health", pg.project),
            &json!({
                "writer_id": writer,
                "cells": [cell("event:file_read", "supported", Some("observation"), true)],
            }),
            &pg.owner.token,
        );
        assert_eq!(code, 200);
    }
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM integration_health
              WHERE project_id = '{}' AND capability = 'event:file_read'",
            pg.project
        )),
        2,
        "a second machine's answer replaced the first's"
    );
}

// ---------------------------------------------------------------------------
// Dispositions
// ---------------------------------------------------------------------------

fn dispositions(pg: &Pg, rows: Vec<Value>) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        &format!("/api/projects/{}/dispositions", pg.project),
        &json!({ "counts": rows }),
        &pg.owner.token,
    )
}

#[test]
fn disposition_counts_accumulate_rather_than_replace() {
    let pg = pg!();
    let row = json!({
        "agent": "claude_code",
        "kind": "tool_invoked",
        "disposition": "capture_deadline_exceeded",
        "day": "2026-09-02",
        "n": 3,
    });
    for _ in 0..2 {
        let (body, code) = dispositions(&pg, vec![row.clone()]);
        assert_eq!(code, 200, "{body}");
    }
    // A client reports what it saw since last time; two reports of one day are
    // two batches of the same funnel, not a correction of it.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT n FROM capture_dispositions WHERE project_id = '{}'",
            pg.project
        )),
        6
    );
}

#[test]
fn an_unrecognized_disposition_is_refused_by_name() {
    let pg = pg!();
    let (_, code) = dispositions(
        &pg,
        vec![json!({
            "agent": "claude_code",
            "kind": "tool_invoked",
            "disposition": "went_fine_probably",
            "day": "2026-09-02",
            "n": 1,
        })],
    );
    assert_eq!(code, 400);
    // Refused before the database sees it, so the error names the value rather
    // than being a constraint violation.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM capture_dispositions WHERE project_id = '{}'",
            pg.project
        )),
        0
    );
}

#[test]
fn the_honest_dispositions_are_all_accepted() {
    let pg = pg!();
    // Including the two that record loss. A funnel that could not express
    // `capture_deadline_exceeded` would report the agent's success and never
    // Cairn's drop (FR-749c).
    for disposition in [
        "captured",
        "declined_by_policy",
        "capture_deadline_exceeded",
        "redaction_failed",
        "privacy_refused",
        "no_safe_semantic_mapping",
        "spooled",
        "spool_overflow_dropped",
        "spool_saturated_dropped",
        "transmitted",
        "accepted",
        "rejected_by_server",
        "persisted",
    ] {
        let (body, code) = dispositions(
            &pg,
            vec![json!({
                "agent": "claude_code",
                "kind": "tool_invoked",
                "disposition": disposition,
                "day": "2026-09-02",
                "n": 1,
            })],
        );
        assert_eq!(code, 200, "{disposition} was refused: {body}");
    }
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM capture_dispositions WHERE project_id = '{}'",
            pg.project
        )),
        13
    );
}
