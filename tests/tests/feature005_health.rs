//! Health truthfulness — what the matrix is allowed to say (T127, US6,
//! SC-724–SC-726, FR-851–FR-860).
//!
//! One thesis, restated at every assertion below: **configuration presence and
//! optimistic interpretation must never manufacture runtime health.** Every
//! way of getting health wrong is a way of upgrading something Cairn merely
//! arranged into something Cairn observed — a hook that was written into a
//! settings file reported as a hook that fired, a cell nobody has looked at
//! reported as working, an observation from last January reported as the
//! present tense, one laptop's success reported on another laptop's behalf.
//!
//! # What this file does not restate
//!
//! `feature005_health_reports.rs` covers T035's write boundary: the refusals
//! (`supported` on `introspection`, `no_evidence` carrying an observation, an
//! unknown capability, an unknown status, an unbounded report, a missing
//! `writer_id`, a non-member) and the two-machine row count. The unit tests in
//! `cairn-integrate::capability` cover the vocabulary in isolation, and
//! `feature005_capture_matrix.rs` covers the declared matrix against the
//! adapters.
//!
//! What none of them reach, and what this file is for, is the **round trip**:
//! whether the six statuses, the evidence kind, the pipeline stage, the
//! observation time and the machine survive storage and come back out of the
//! read APIs still saying what the reporter said — and whether the read adds
//! any judgement of its own. A vocabulary that is honest in `capability.rs`
//! and is flattened on the way through PostgreSQL is not an honest matrix.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{get_json_status_bearer, post_status_bearer};
use cairn_integrate::capability::{complete_matrix, declared_matrix, MatrixCapability};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

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

/// Every agent the matrix declares an answer for. `generic_mcp` is in the list
/// on purpose: an agent reachable only through the explicit tool surface still
/// owes twenty-five answers, and "we have no adapter" is one of them
/// (FR-729, FR-838f).
const AGENTS: [&str; 4] = ["claude_code", "codex", "opencode", "generic_mcp"];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A cell in the shape a reporting client actually posts, with nothing implied.
///
/// Deliberately free-form rather than built from `MatrixCell`: several tests
/// below post shapes a correct client would never produce, and a helper that
/// could only express legal cells could not ask whether an illegal one is
/// refused.
fn cell(
    agent: &str,
    capability: &str,
    stage: &str,
    status: &str,
    evidence_kind: Option<&str>,
    observed_at: Option<&str>,
) -> Value {
    let mut c = json!({
        "agent": agent,
        "capability": capability,
        "stage": stage,
        "status": status,
    });
    if let Some(kind) = evidence_kind {
        c["evidence_kind"] = json!(kind);
    }
    if let Some(at) = observed_at {
        c["observed_at"] = json!(at);
    }
    c
}

/// A behavioural claim with the observation that makes it claimable, at the
/// capability's own default stage.
fn observed(agent: &str, capability: &str, status: &str, at: &str) -> Value {
    cell(
        agent,
        capability,
        &default_stage(capability),
        status,
        Some("observation"),
        Some(at),
    )
}

fn default_stage(capability: &str) -> String {
    MatrixCapability::parse(capability)
        .unwrap_or_else(|| panic!("{capability} is not a capability the matrix has a cell for"))
        .default_stage()
        .as_str()
        .to_string()
}

/// The declared matrix for one agent, in wire form.
///
/// Serialized from `declared_matrix` rather than retyped, because the point of
/// several tests below is that *what Cairn declares* survives the round trip.
/// A hand-written copy would be asserting that two literals match.
fn declared_cells(agent: &str) -> Vec<Value> {
    declared_matrix(agent)
        .iter()
        .map(|c| serde_json::to_value(c).expect("a matrix cell serializes"))
        .collect()
}

fn complete_cells(agent: &str) -> Vec<Value> {
    complete_matrix(agent)
        .iter()
        .map(|c| serde_json::to_value(c).expect("a matrix cell serializes"))
        .collect()
}

fn report_as(pg: &Pg, who: &Account, writer: &str, cells: Vec<Value>) -> u16 {
    post_status_bearer(
        &pg.server.base,
        &format!("/api/projects/{}/health", pg.project),
        &json!({ "writer_id": writer, "cells": cells }),
        &who.token,
    )
}

fn report(pg: &Pg, writer: &str, cells: Vec<Value>) -> u16 {
    let owner = &pg.owner;
    report_as(pg, owner, writer, cells)
}

/// `GET /health` — US6's own read.
fn health_cells(pg: &Pg) -> Vec<Value> {
    let (body, code) = get_json_status_bearer(
        &pg.server.base,
        &format!("/api/projects/{}/health", pg.project),
        &pg.owner.token,
    );
    assert_eq!(code, 200, "{body}");
    body["cells"]
        .as_array()
        .unwrap_or_else(|| panic!("the health read has no `cells` array: {body}"))
        .clone()
}

/// `GET /integration-health` — US5's agents screen, over the same query.
///
/// Read here as well as `/health` because §5's rules are about what a *reader*
/// is shown, and two envelopes over one query are still two things a reader can
/// be shown. A rule enforced on one and not the other is not enforced.
fn integration_rows(pg: &Pg) -> Vec<Value> {
    let (body, code) = get_json_status_bearer(
        &pg.server.base,
        &format!("/api/projects/{}/integration-health", pg.project),
        &pg.owner.token,
    );
    assert_eq!(code, 200, "{body}");
    body["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("the integration-health read has no `rows` array: {body}"))
        .clone()
}

fn rows_for<'a>(rows: &'a [Value], agent: &str, capability: &str) -> Vec<&'a Value> {
    rows.iter()
        .filter(|r| r["agent"] == agent && r["capability"] == capability)
        .collect()
}

fn one_row<'a>(rows: &'a [Value], agent: &str, capability: &str) -> &'a Value {
    let found = rows_for(rows, agent, capability);
    assert_eq!(
        found.len(),
        1,
        "{agent}:{capability} came back as {} rows, not one",
        found.len()
    );
    found[0]
}

fn status_of(rows: &[Value], agent: &str, capability: &str) -> String {
    one_row(rows, agent, capability)["status"]
        .as_str()
        .expect("a status is a string")
        .to_string()
}

/// Every status the read surfaced, indexed by the cell it belongs to.
fn status_map(rows: &[Value], agent: &str) -> BTreeMap<String, String> {
    rows.iter()
        .filter(|r| r["agent"] == agent)
        .map(|r| {
            (
                r["capability"].as_str().unwrap_or_default().to_string(),
                r["status"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

/// The one invariant every test in this file may assume about every row it
/// reads back, asserted directly wherever a read happens.
///
/// A `supported` or `runtime_failure` row is a claim about behaviour, and a
/// claim about behaviour that does not carry a runtime observation is the
/// conflation FR-852 forbids. This would be falsified by a write path that
/// dropped `evidence_kind` on the way to the table, or by a read that
/// defaulted it — either would let a configuration read-back come back out
/// wearing a green badge.
fn assert_behavioural_rows_carry_observations(rows: &[Value], label: &str) {
    for row in rows {
        let status = row["status"].as_str().unwrap_or_default();
        if status == "supported" || status == "runtime_failure" {
            assert_eq!(
                row["evidence_kind"], "observation",
                "{label}: {status} came back without a runtime observation behind it: {row}"
            );
            assert!(
                row["observed_at"].is_string(),
                "{label}: {status} came back with no observation time: {row}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Part 1 — the matrix is whole, and the six statuses stay six
// ---------------------------------------------------------------------------

#[test]
fn the_whole_declared_matrix_survives_the_round_trip_for_every_agent() {
    let pg = pg!();
    // SC-726. The matrix is complete before anything runs, and completeness is
    // the reporter's obligation: twenty-five cells per agent, every one of them
    // answered. A matrix that simply omitted the cells nothing is known about
    // would report absence as if it were a decision — the reader would see a
    // short list of green rows and no way to tell what was not on it.
    //
    // Falsified by: a cell dropped between `declared_matrix` and the table, a
    // read that filters, or a write that silently coalesces two cells into one.
    let mut cells = Vec::new();
    for agent in AGENTS {
        let declared = declared_cells(agent);
        assert_eq!(declared.len(), 25, "{agent} declares an incomplete matrix");
        cells.extend(declared);
    }
    assert_eq!(report(&pg, "laptop-a", cells), 200);

    let rows = health_cells(&pg);
    assert_eq!(rows.len(), 100, "a declared cell did not survive storage");
    assert_behavioural_rows_carry_observations(&rows, "declared matrix");

    let expected: BTreeSet<String> = MatrixCapability::all().iter().map(|c| c.key()).collect();
    for agent in AGENTS {
        let stored = status_map(&rows, agent);
        let seen: BTreeSet<String> = stored.keys().cloned().collect();
        assert_eq!(
            seen, expected,
            "{agent}: the stored matrix is not the declared population"
        );
        // And every answer is the one Cairn declared, not one the server
        // improved on. The server is a recorder here; a status it rewrote would
        // be a second opinion with no evidence behind it (FR-859).
        for declared in declared_matrix(agent) {
            assert_eq!(
                stored[&declared.capability],
                declared.status.as_str(),
                "{agent}:{} was stored as something other than what was declared",
                declared.capability
            );
        }
    }

    // Both read surfaces answer the same question, so both owe the same matrix.
    assert_eq!(integration_rows(&pg).len(), 100);
}

#[test]
fn every_status_in_the_vocabulary_comes_back_as_itself() {
    let pg = pg!();
    // SC-726 names five and FR-856 adds `no_evidence`; the six exist because
    // each calls for a different action. Collapsing any two — "not offered by
    // the vendor" into "we haven't built it", "we declined" into "it is broken"
    // — would send a reader to fix the wrong thing.
    //
    // Falsified by: a status column that stores fewer distinct values than it
    // was given, or a read that maps several onto one display value.
    let wanted: BTreeMap<&str, &str> = BTreeMap::from([
        ("event:tool_failed", "supported"),
        ("event:file_changed", "runtime_failure"),
        ("event:subagent_started", "unsupported_by_vendor"),
        ("deliver:prompt_time", "declined_by_cairn"),
        ("event:session_resumed", "adapter_unimplemented"),
        ("receipt", "no_evidence"),
    ]);
    let cells: Vec<Value> = wanted
        .iter()
        .map(|(capability, status)| match *status {
            "supported" | "runtime_failure" => {
                observed("claude_code", capability, status, "2026-09-02T10:00:00Z")
            }
            _ => cell(
                "claude_code",
                capability,
                &default_stage(capability),
                status,
                None,
                None,
            ),
        })
        .collect();
    assert_eq!(report(&pg, "laptop-a", cells), 200);

    let rows = health_cells(&pg);
    assert_eq!(rows.len(), 6);
    assert_behavioural_rows_carry_observations(&rows, "one cell per status");
    for (capability, status) in &wanted {
        assert_eq!(
            &status_of(&rows, "claude_code", capability),
            status,
            "{capability} did not come back as {status}"
        );
    }
    let distinct: BTreeSet<&str> = rows.iter().filter_map(|r| r["status"].as_str()).collect();
    assert_eq!(
        distinct.len(),
        6,
        "six statuses went in and {} came out: {distinct:?}",
        distinct.len()
    );
}

#[test]
fn a_capability_nobody_reported_is_missing_rather_than_invented() {
    let pg = pg!();
    // FR-855. "No report ever arrived for this cell" and "a machine looked and
    // saw nothing" are different facts, and only the second is `no_evidence`.
    // A read that filled the gap in would make a daemon that never ran
    // indistinguishable from one that ran and observed nothing — the silence
    // that most needs to be visible is exactly the one that would be papered
    // over.
    //
    // Falsified by: a read that seeds absent cells from `complete_matrix`.
    let withheld = "receipt";
    let cells: Vec<Value> = declared_cells("codex")
        .into_iter()
        .filter(|c| c["capability"] != withheld)
        .collect();
    assert_eq!(cells.len(), 24);
    assert_eq!(report(&pg, "laptop-a", cells), 200);

    let rows = health_cells(&pg);
    assert_eq!(rows.len(), 24, "the read invented a cell nobody reported");
    assert!(
        rows_for(&rows, "codex", withheld).is_empty(),
        "{withheld} was synthesized as though a machine had answered for it"
    );
    // And the gap is *detectable* rather than merely honest: a reader holding
    // the capability population can name what is missing, which is the whole
    // reason the population is enumerated rather than inferred from reports.
    let seen: BTreeSet<String> = status_map(&rows, "codex").keys().cloned().collect();
    let missing: Vec<String> = MatrixCapability::all()
        .iter()
        .map(|c| c.key())
        .filter(|k| !seen.contains(k))
        .collect();
    assert_eq!(missing, vec![withheld.to_string()]);
}

// ---------------------------------------------------------------------------
// Part 2 — configured is not working (SC-724, SC-725, FR-852, FR-853)
// ---------------------------------------------------------------------------

#[test]
fn a_configured_integration_that_never_fired_is_reported_as_not_capturing() {
    let pg = pg!();
    // SC-724. `laptop-config` has Cairn's hook in its settings file and Cairn
    // has read it back and found it correct — the `configured` and `installed`
    // stages are as far as it got. Nothing has fired. The honest answer at the
    // runtime stage is `no_evidence`, and the reader must not be able to reach
    // "this machine is capturing" from any row here.
    //
    // Falsified by: a status derived from configuration presence, at any stage.
    let capability = "event:tool_failed";
    let configured: Vec<Value> = ["configured", "installed", "runtime_hook_fired"]
        .into_iter()
        .map(|stage| cell("claude_code", capability, stage, "no_evidence", None, None))
        .collect();
    assert_eq!(report(&pg, "laptop-config", configured), 200);

    let rows = health_cells(&pg);
    assert_eq!(rows.len(), 3);
    for row in &rows {
        assert_eq!(
            row["status"], "no_evidence",
            "a configured-only machine reported as something other than silent: {row}"
        );
        assert_eq!(row["evidence_kind"], Value::Null, "{row}");
        assert_eq!(row["observed_at"], Value::Null, "{row}");
    }

    // A second machine where the hook actually fired. SC-725: the two resolve
    // to visibly different rows, and — FR-857 — the working one does not lend
    // its confidence to the configured one.
    assert_eq!(
        report(
            &pg,
            "laptop-run",
            vec![observed(
                "claude_code",
                capability,
                "supported",
                "2026-09-02T10:00:00Z"
            )]
        ),
        200
    );
    let rows = health_cells(&pg);
    assert_behavioural_rows_carry_observations(&rows, "configured versus observed");
    let working: Vec<&Value> = rows.iter().filter(|r| r["status"] == "supported").collect();
    assert_eq!(working.len(), 1, "configuration acquired a second success");
    assert_eq!(
        working[0]["writer_id"], "laptop-run",
        "the success was attributed to the machine that only has a config file"
    );
    for row in rows.iter().filter(|r| r["writer_id"] == "laptop-config") {
        assert_eq!(
            row["status"], "no_evidence",
            "another machine's observation upgraded a configured-only cell: {row}"
        );
    }
}

#[test]
fn configuration_read_back_cannot_claim_support_at_any_stage() {
    let pg = pg!();
    // FR-852/FR-853. `introspection` says Cairn wrote a file and read it back;
    // it establishes authorship, never behaviour. The refusal is already proven
    // at the default stage in `feature005_health_reports.rs`; what is asserted
    // here is that the stage is not a way around it. A validator that checked
    // coherence only for `runtime_hook_fired` would accept the identical
    // overclaim filed as `installed`.
    //
    // Falsified by: any stage at which `supported` is accepted on
    // configuration-derived evidence.
    for stage in [
        "configured",
        "installed",
        "runtime_hook_fired",
        "event_received",
        "context_generated",
    ] {
        assert_eq!(
            report(
                &pg,
                "laptop-a",
                vec![cell(
                    "claude_code",
                    "event:tool_failed",
                    stage,
                    "supported",
                    Some("introspection"),
                    Some("2026-09-02T10:00:00Z"),
                )]
            ),
            400,
            "`supported` on configuration evidence was accepted at stage {stage}"
        );
    }
    assert!(
        health_cells(&pg).is_empty(),
        "a rejected overclaim reached the matrix anyway"
    );
}

// ---------------------------------------------------------------------------
// Part 3 — no observation is not a failure, and not a success (FR-856)
// ---------------------------------------------------------------------------

#[test]
fn silence_is_neither_failure_nor_success_and_stays_silent() {
    let pg = pg!();
    // FR-856, and §5's "**never** rendered as failing or as working". The two
    // wrong readings pull in opposite directions and both are wrong: a matrix
    // that renders silence as red sends someone to debug a working install, and
    // one that renders it as green hides an install that was never exercised.
    //
    // Falsified by: any row of a freshly reported complete matrix coming back
    // as anything but `no_evidence`.
    assert_eq!(report(&pg, "laptop-a", complete_cells("codex")), 200);
    let rows = health_cells(&pg);
    assert_eq!(rows.len(), 25);
    for row in &rows {
        assert_eq!(row["status"], "no_evidence", "{row}");
        assert_eq!(row["evidence_kind"], Value::Null, "{row}");
        assert_eq!(row["observed_at"], Value::Null, "{row}");
    }

    // One cell earns its observation. The other twenty-four are not carried
    // along with it: a working `tool_failed` hook says nothing whatever about
    // whether `test_result` was ever captured, and a matrix that generalized
    // from one green cell to its neighbours would be inventing twenty-four
    // observations from one.
    assert_eq!(
        report(
            &pg,
            "laptop-a",
            vec![observed(
                "codex",
                "event:tool_failed",
                "supported",
                "2026-09-02T10:00:00Z"
            )]
        ),
        200
    );
    let rows = health_cells(&pg);
    assert_eq!(rows.len(), 25, "a report changed the population");
    assert_eq!(status_of(&rows, "codex", "event:tool_failed"), "supported");
    let silent = rows
        .iter()
        .filter(|r| r["capability"] != "event:tool_failed")
        .count();
    assert_eq!(silent, 24);
    for row in rows
        .iter()
        .filter(|r| r["capability"] != "event:tool_failed")
    {
        assert_eq!(
            row["status"], "no_evidence",
            "one observation spread to a cell nothing was observed for: {row}"
        );
    }
}

#[test]
fn a_failure_claim_needs_an_observation_just_as_a_success_does() {
    let pg = pg!();
    // The mirror of FR-852, and the one direction the existing write-boundary
    // tests do not exercise. `runtime_failure` is as much a claim about
    // behaviour as `supported` is: it says the thing fired, or should have, and
    // did not. Accepting it on no evidence would turn every unexercised cell
    // into a red one the moment a client felt pessimistic — which is
    // `no_evidence` being converted into a failure, the exact conversion T135
    // is forbidden to make.
    //
    // Falsified by: a coherence rule that gates `supported` and not
    // `runtime_failure`.
    for (evidence, at) in [
        (None, None),
        (Some("introspection"), Some("2026-09-02T10:00:00Z")),
        (Some("introspection"), None),
        (None, Some("2026-09-02T10:00:00Z")),
    ] {
        assert_eq!(
            report(
                &pg,
                "laptop-a",
                vec![cell(
                    "claude_code",
                    "event:tool_failed",
                    "runtime_hook_fired",
                    "runtime_failure",
                    evidence,
                    at,
                )]
            ),
            400,
            "a failure was recorded with evidence_kind={evidence:?} observed_at={at:?}"
        );
    }
    assert!(
        health_cells(&pg).is_empty(),
        "an unevidenced failure reached the matrix"
    );

    // And the evidenced form is accepted, so the rule above is a rule about
    // evidence and not a refusal to record failure at all.
    assert_eq!(
        report(
            &pg,
            "laptop-a",
            vec![observed(
                "claude_code",
                "event:tool_failed",
                "runtime_failure",
                "2026-09-02T10:00:00Z"
            )]
        ),
        200
    );
}

// ---------------------------------------------------------------------------
// Part 4 — stale evidence is not current success (FR-860, §5)
// ---------------------------------------------------------------------------

#[test]
fn an_old_observation_keeps_its_timestamp_and_the_server_offers_no_verdict_on_it() {
    let pg = pg!();
    // FR-860 plus §5. Two halves, and the second is the easy one to get wrong.
    //
    // The server owes the reader `observed_at` and the machine it came from —
    // without the timestamp, an integration that worked last January reads as
    // working now, which is the whole failure FR-860 names. But §5 puts the
    // staleness *judgement* client-side, against a per-capability freshness
    // window, because capabilities go stale at different rates: a session-open
    // delivery unobserved for a week means something quite different from a
    // subagent hook unobserved for a week. A server that also computed `stale`
    // would be a second opinion baked to one window, and the two would disagree
    // the first time the window moved.
    //
    // Falsified by: `observed_at` dropped from either read, a server that
    // downgrades an old `supported` on its own authority, or any freshness
    // verdict appearing in the row.
    let long_ago = "2025-01-15T09:30:00Z";
    assert_eq!(
        report(
            &pg,
            "laptop-a",
            vec![observed(
                "claude_code",
                "event:file_changed",
                "supported",
                long_ago
            )]
        ),
        200
    );

    let expected = chrono::DateTime::parse_from_rfc3339(long_ago).expect("a fixed timestamp");
    for (label, rows) in [
        ("health", health_cells(&pg)),
        ("integration-health", integration_rows(&pg)),
    ] {
        let row = one_row(&rows, "claude_code", "event:file_changed");
        // Still `supported`. The row says "worked as of `observed_at`", and the
        // view is what turns that into "worked" rather than "working"; a server
        // that rewrote the status would have destroyed the evidence it was
        // reasoning from.
        assert_eq!(row["status"], "supported", "{label}: {row}");
        let carried = row["observed_at"]
            .as_str()
            .unwrap_or_else(|| panic!("{label}: the observation time was dropped: {row}"));
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(carried)
                .unwrap_or_else(|e| panic!("{label}: {carried} is not a timestamp: {e}")),
            expected,
            "{label}: the observation time came back as a different instant"
        );
        assert_eq!(row["writer_id"], "laptop-a", "{label}: {row}");

        let object: &Map<String, Value> = row
            .as_object()
            .unwrap_or_else(|| panic!("{label}: a row is not an object"));
        for verdict in [
            "stale",
            "is_stale",
            "stale_at",
            "fresh",
            "freshness",
            "age_seconds",
            "expired",
        ] {
            assert!(
                !object.contains_key(verdict),
                "{label}: the server shipped its own freshness verdict `{verdict}`, \
                 which §5 makes the client's derivation from observed_at: {row}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Part 5 — the failure stage stays visible (FR-858, FR-851)
// ---------------------------------------------------------------------------

#[test]
fn a_failure_names_the_stage_it_failed_at_and_is_not_flattened_into_failing() {
    let pg = pg!();
    // FR-858. "The hook never fired", "the hook fired and the payload would not
    // parse" and "the server refused what capture produced" are three different
    // bugs owned by three different people, and one red badge saying "failing"
    // sends all three to the same wrong place. The stage is part of the cell's
    // identity precisely so the three can coexist and be told apart.
    //
    // Falsified by: a write that keys on (agent, capability) and lets a later
    // stage overwrite an earlier one, or a read that drops `stage`.
    let capability = "event:tool_failed";
    let at = "2026-09-02T10:00:00Z";
    assert_eq!(
        report(
            &pg,
            "laptop-a",
            vec![
                // The hook fired: capture's own stage is healthy.
                cell(
                    "claude_code",
                    capability,
                    "runtime_hook_fired",
                    "supported",
                    Some("observation"),
                    Some(at)
                ),
                // And the payload did not survive parsing.
                cell(
                    "claude_code",
                    capability,
                    "event_parsed",
                    "runtime_failure",
                    Some("observation"),
                    Some(at)
                ),
                // And the server refused what did survive. A distinct fact
                // again, and the one that would send someone to the server
                // logs rather than the adapter.
                cell(
                    "claude_code",
                    capability,
                    "server_persisted_event",
                    "runtime_failure",
                    Some("observation"),
                    Some(at)
                ),
            ]
        ),
        200
    );

    let rows = health_cells(&pg);
    let cells = rows_for(&rows, "claude_code", capability);
    assert_eq!(
        cells.len(),
        3,
        "three stages of one capability collapsed into {} row(s)",
        cells.len()
    );
    let by_stage: BTreeMap<&str, &str> = cells
        .iter()
        .map(|r| {
            (
                r["stage"].as_str().expect("a stage"),
                r["status"].as_str().expect("a status"),
            )
        })
        .collect();
    assert_eq!(by_stage["runtime_hook_fired"], "supported");
    assert_eq!(by_stage["event_parsed"], "runtime_failure");
    assert_eq!(by_stage["server_persisted_event"], "runtime_failure");
    assert_behavioural_rows_carry_observations(&rows, "staged failures");
}

#[test]
fn a_stage_outside_the_pipeline_vocabulary_is_refused() {
    let pg = pg!();
    // FR-851 enumerates the stages, and the reason it enumerates them is the
    // same reason FR-855 enumerates the statuses: a cell rendering an
    // unrecognized string is a blank cell wearing a value. Worse here than for
    // status, because `stage` is part of the row's identity — an unrecognized
    // stage does not overwrite the real cell, it silently adds a fourth row
    // beside it, and the matrix grows a cell nobody can act on.
    //
    // Falsified by: any stage string outside `PipelineStage` reaching the
    // table.
    for stage in [
        "",
        "definitely_working",
        "runtime_hook_fired ",
        "RuntimeHookFired",
        "Configured",
        "context_receipt_confirmed_probably",
    ] {
        assert_eq!(
            report(
                &pg,
                "laptop-a",
                vec![cell(
                    "claude_code",
                    "event:tool_failed",
                    stage,
                    "no_evidence",
                    None,
                    None
                )]
            ),
            400,
            "{stage:?} was accepted as a pipeline stage"
        );
    }
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM integration_health WHERE project_id = '{}'",
            pg.project
        )),
        0,
        "a cell with an unrecognized stage reached the matrix"
    );
}

// ---------------------------------------------------------------------------
// Part 6 — machine attribution (FR-857, §10)
// ---------------------------------------------------------------------------

#[test]
fn one_machines_observation_is_never_attributed_to_another() {
    let pg = pg!();
    // FR-857, and the sharpest rule in the file. A capability is observed on a
    // *machine*. One account with a laptop and a desktop can legitimately see
    // two different answers for the same cell, and the failure mode is not
    // subtle: if the rows merged, whichever machine reported last would speak
    // for both, and a developer whose hooks are broken would be told they work
    // because a colleague's do.
    //
    // Falsified by: a row identity that omits `writer_id`, or a read that
    // deduplicates on (agent, capability, stage).
    let capability = "event:test_result";
    let at = "2026-09-02T10:00:00Z";
    assert_eq!(
        report(
            &pg,
            "laptop-a",
            vec![observed("claude_code", capability, "supported", at)]
        ),
        200
    );
    assert_eq!(
        report(
            &pg,
            "laptop-b",
            vec![observed("claude_code", capability, "runtime_failure", at)]
        ),
        200
    );

    let attribution = |rows: &[Value]| -> BTreeMap<String, String> {
        rows_for(rows, "claude_code", capability)
            .iter()
            .map(|r| {
                (
                    r["writer_id"].as_str().unwrap_or_default().to_string(),
                    r["status"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    };

    let rows = health_cells(&pg);
    assert_eq!(
        attribution(&rows),
        BTreeMap::from([
            ("laptop-a".to_string(), "supported".to_string()),
            ("laptop-b".to_string(), "runtime_failure".to_string()),
        ]),
        "one machine's answer was reported for the other"
    );

    // The working machine breaks. Its own row moves and the other machine's is
    // untouched — an update is scoped to the machine that filed it.
    assert_eq!(
        report(
            &pg,
            "laptop-a",
            vec![observed("claude_code", capability, "runtime_failure", at)]
        ),
        200
    );
    let rows = health_cells(&pg);
    assert_eq!(rows_for(&rows, "claude_code", capability).len(), 2);
    assert_eq!(
        attribution(&rows)["laptop-b"],
        "runtime_failure",
        "a report from one machine rewrote another machine's row"
    );

    // A third machine that has observed nothing joins. Silence from one machine
    // never erases an observation made on another — the matrix gains a row
    // rather than losing a fact.
    assert_eq!(
        report(
            &pg,
            "laptop-c",
            vec![cell(
                "claude_code",
                capability,
                &default_stage(capability),
                "no_evidence",
                None,
                None
            )]
        ),
        200
    );
    let rows = health_cells(&pg);
    let seen = attribution(&rows);
    assert_eq!(seen.len(), 3);
    assert_eq!(seen["laptop-c"], "no_evidence");
    assert_eq!(seen["laptop-a"], "runtime_failure");
    assert_eq!(seen["laptop-b"], "runtime_failure");
}

#[test]
fn two_accounts_behind_one_machine_label_stay_attributable() {
    let pg = pg!();
    // FR-857 with SC-726's "zero cells are blank **or ambiguous**". The row's
    // stored identity includes the account (it is in the table's primary key),
    // so two accounts reporting the same machine label are two facts and are
    // correctly kept apart in storage. What a reader is handed is another
    // matter: if the account is not in the response, the same machine appears
    // twice with contradictory statuses and nothing to attribute either to.
    // "laptop-a is working" and "laptop-a is failing", side by side, is not a
    // truthful cell — it is a blank one with two values in it.
    //
    // The label collides in practice: a hostname, a container name or a CI
    // runner id is shared far more readily than an account is.
    //
    // Falsified by: two rows that differ only in `status`. Satisfied by any
    // discriminator the reader can attribute with — the account id, the
    // reporting user, a composite writer identity — this asserts that one
    // exists, not which.
    let capability = "event:test_result";
    let at = "2026-09-02T10:00:00Z";
    assert_eq!(
        report_as(
            &pg,
            &pg.owner,
            "shared-ci",
            vec![observed("claude_code", capability, "supported", at)]
        ),
        200
    );
    assert_eq!(
        report_as(
            &pg,
            &pg.member,
            "shared-ci",
            vec![observed("claude_code", capability, "runtime_failure", at)]
        ),
        200
    );

    // Storage kept them apart.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM integration_health
              WHERE project_id = '{}' AND capability = '{capability}'",
            pg.project
        )),
        2,
        "two accounts' observations were merged in the table"
    );

    for (label, rows) in [
        ("health", health_cells(&pg)),
        ("integration-health", integration_rows(&pg)),
    ] {
        let cells = rows_for(&rows, "claude_code", capability);
        assert_eq!(
            cells.len(),
            2,
            "{label}: the two reports did not both surface"
        );
        let identities: BTreeSet<String> = cells
            .iter()
            .map(|r| {
                let mut without_status = r.as_object().expect("a row is an object").clone();
                without_status.remove("status");
                without_status.remove("evidence_kind");
                without_status.remove("observed_at");
                without_status.remove("degraded");
                Value::Object(without_status).to_string()
            })
            .collect();
        assert_eq!(
            identities.len(),
            2,
            "{label}: two contradictory rows for one machine label carry no way to \
             tell whose observation is whose: {cells:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Part 7 — an OpenCode decline is Cairn's decision (FR-838b, §5)
// ---------------------------------------------------------------------------

#[test]
fn opencodes_declines_stay_declines_in_both_directions() {
    let pg = pg!();
    // FR-838b. `declined_by_cairn` and `unsupported_by_vendor` render nearly
    // alike — both grey, both "you will not get this" — and that is exactly why
    // they must not be allowed to drift into each other. Calling Cairn's
    // decision a vendor absence blames OpenCode for surfaces it does expose;
    // calling a real vendor absence a Cairn decision sends a user to open an
    // issue against Cairn for a hook that does not exist. Both directions are
    // asserted because a mapping that collapsed them would satisfy either
    // half alone.
    //
    // Falsified by: any of these statuses being rewritten between
    // `declared_matrix` and the read.
    assert_eq!(report(&pg, "laptop-a", declared_cells("opencode")), 200);
    let rows = health_cells(&pg);
    assert_eq!(rows.len(), 25);

    // Cairn's decision. The v1 surfaces are undocumented and the v2 ones beta,
    // and declining to rest a guarantee on a beta surface is a choice Cairn
    // made and owns.
    for declined in [
        "event:user_instruction_signal",
        "event:decision_signal",
        "deliver:session_open",
        "deliver:prompt_time",
        "deliver:post_compaction",
    ] {
        let status = status_of(&rows, "opencode", declined);
        assert_eq!(status, "declined_by_cairn", "{declined}");
        assert_ne!(
            status, "unsupported_by_vendor",
            "{declined}: Cairn's own decision was reported as the vendor's absence"
        );
        assert_ne!(
            status, "supported",
            "{declined}: a declined capability was reported as working"
        );
    }

    // The vendor's absence, told apart from the decision. OpenCode signals no
    // session end and exposes no subagent hooks at all.
    for absent in [
        "event:session_closed",
        "event:subagent_started",
        "event:subagent_completed",
    ] {
        let status = status_of(&rows, "opencode", absent);
        assert_eq!(status, "unsupported_by_vendor", "{absent}");
        assert_ne!(
            status, "declined_by_cairn",
            "{absent}: a vendor absence was reported as a choice Cairn made"
        );
    }

    // And nothing OpenCode does support was declined on its behalf. Over-
    // declining is the same lie pointed the other way: it would report
    // structural capture as switched off when it is merely unobserved.
    for structural in [
        "event:file_changed",
        "event:command_executed",
        "event:test_result",
    ] {
        assert_eq!(
            status_of(&rows, "opencode", structural),
            "no_evidence",
            "{structural} was declined rather than left honestly unobserved"
        );
    }

    // No OpenCode cell claims support before anything has been observed.
    assert!(
        !rows.iter().any(|r| r["status"] == "supported"),
        "a declared matrix claimed a working capability: {rows:?}"
    );
    assert_behavioural_rows_carry_observations(&rows, "opencode declared matrix");
}

// ---------------------------------------------------------------------------
// Part 8 — receipt, and the absence that stays an absence (FR-838e, FR-854)
// ---------------------------------------------------------------------------

#[test]
fn receipt_is_not_upgraded_by_the_absence_of_a_failure() {
    let pg = pg!();
    // FR-838e with FR-854. No vendor mechanism was established for confirming
    // that delivered context actually reached the agent, so acknowledgement is
    // `no_evidence` — an absence of evidence, never a vendor statement that
    // none exists, and never a success.
    //
    // The tempting inference is the one asserted against here: every delivery
    // cell around it is `supported` and nothing anywhere reported a failure, so
    // surely the context arrived. It does not follow. Writing context to a
    // channel is not the agent having consumed it (FR-854), and "nothing went
    // wrong" is not an observation of anything going right.
    //
    // Falsified by: a receipt cell derived from delivery success, or from the
    // absence of a failure row.
    let at = "2026-09-02T10:00:00Z";
    for agent in ["claude_code", "codex"] {
        let mut cells: Vec<Value> = declared_cells(agent)
            .into_iter()
            .filter(|c| {
                !c["capability"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("deliver:")
            })
            .collect();
        for point in [
            "deliver:session_open",
            "deliver:prompt_time",
            "deliver:post_compaction",
        ] {
            cells.push(observed(agent, point, "supported", at));
        }
        assert_eq!(report(&pg, "laptop-a", cells), 200);
    }

    let rows = health_cells(&pg);
    assert_behavioural_rows_carry_observations(&rows, "delivery observed, receipt not");
    for agent in ["claude_code", "codex"] {
        for point in [
            "deliver:session_open",
            "deliver:prompt_time",
            "deliver:post_compaction",
        ] {
            assert_eq!(
                status_of(&rows, agent, point),
                "supported",
                "{agent}:{point}"
            );
        }
        let receipt = one_row(&rows, agent, "receipt");
        assert_eq!(
            receipt["status"], "no_evidence",
            "{agent}: acknowledgement was upgraded by three successful deliveries"
        );
        assert_eq!(receipt["evidence_kind"], Value::Null, "{receipt}");
        assert_eq!(receipt["observed_at"], Value::Null, "{receipt}");
        assert_eq!(
            receipt["stage"], "context_receipt_confirmed",
            "{agent}: the receipt cell lost the stage that says what it is about"
        );
    }

    // Nor can the absence be converted the other way. "Nothing came back" is
    // not a failure any more than it is a success: reporting `runtime_failure`
    // for acknowledgement would assert that a mechanism exists and broke.
    assert_eq!(
        report(
            &pg,
            "laptop-a",
            vec![cell(
                "claude_code",
                "receipt",
                "context_receipt_confirmed",
                "runtime_failure",
                None,
                None
            )]
        ),
        400
    );
    assert_eq!(
        report(
            &pg,
            "laptop-a",
            vec![cell(
                "claude_code",
                "receipt",
                "context_receipt_confirmed",
                "supported",
                Some("introspection"),
                Some(at)
            )]
        ),
        400
    );
    assert_eq!(
        status_of(&health_cells(&pg), "claude_code", "receipt"),
        "no_evidence",
        "a refused claim still moved the receipt cell"
    );
}
