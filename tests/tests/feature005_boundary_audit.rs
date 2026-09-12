//! The refused-name and raw-material boundaries, audited rather than sampled
//! (T037, SC-730, SC-731, SC-751).
//!
//! Two standing refusals, and both are the kind that erode quietly:
//!
//! - **No safe-event field may reuse a name the synchronization boundary
//!   refuses** (FR-777a). Two boundaries on one server disagreeing about the
//!   same name is the drift FR-760 forbids for rejection classes, and the
//!   refused set is full of names that are the *natural* choice for something
//!   a command, test or tool-failure event has to carry — `command`,
//!   `exit_code`, `details`, `summary`. The pressure to reuse one is
//!   continuous.
//! - **Nothing durable holds raw material.** No column, on either side, for a
//!   transcript, a prompt, raw tool output or a vendor's original JSON. Raw
//!   material lives in memory for the duration of parsing and redaction and is
//!   never written (FR-730, FR-763).
//!
//! These are asserted against the **source and the schema**, not against a
//! running server, because a live probe can only test the field it thinks of
//! and the point is to catch the one nobody thought of.

use cairn_e2e::feature005::Pg;

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

fn source(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The names the synchronization boundary refuses, recursively at any depth
/// (`data-model.md` preamble, `crates/cairn-server/src/sync.rs`).
const REFUSED: &[&str] = &[
    "summary",
    "path",
    "command",
    "details",
    "exit_code",
    "observations",
    "observed_value",
    "source_locator",
    "value_digest",
    "fingerprint",
    "relevant_paths",
    "criteria_snapshot",
    "sanitization_report",
    "origin_ref",
    "alternative_cause",
    "signal_digest",
    "pin_reason",
    "rationale",
    "basis_evidence_id",
    "path_fingerprints",
    "task_snapshot_at_bind",
    "detail",
    "prior_value",
    "new_value",
    "content_norm_digest",
    "local_revision",
    // The session refusals apply equally.
    "worktree_path",
    "agent_session_key",
    "daemon_run_id",
    "last_event_at",
    "last_turn_ended_at",
];

/// Field names declared by `event.rs`, read from its serde-facing structs.
///
/// Parsed from the source rather than from a serialized sample, because a
/// sample only contains the fields the sample's variant happens to carry, and
/// the audit has to see every variant including the ones no test builds.
fn declared_event_fields() -> Vec<String> {
    let src = source("crates/cairn-core/src/event.rs");
    let mut fields = Vec::new();
    let mut in_type = false;
    for line in src.lines() {
        let t = line.trim();
        if t.starts_with("pub struct ") || t.starts_with("pub enum ") {
            in_type = true;
            continue;
        }
        if in_type && t == "}" {
            in_type = false;
            continue;
        }
        if !in_type || t.starts_with("//") || t.starts_with('#') {
            continue;
        }
        // Two shapes: a field on its own line, and a single-line enum variant
        // such as `TestInvocation { test_command: String },`. The second is the
        // one an earlier version of this scan missed — and it missed it
        // silently, reporting a clean audit of a model it had not fully read.
        for candidate in t.split(['{', ',']) {
            let Some((name, rest)) = candidate.split_once(':') else {
                continue;
            };
            let name = name.trim().trim_start_matches("pub ").trim();
            if name.is_empty()
                || rest.trim().is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                continue;
            }
            fields.push(name.to_string());
        }
    }
    assert!(
        fields.len() > 25,
        "the field scan found only {} fields, so it is not reading the model",
        fields.len()
    );
    fields
}

/// SQL with its comments removed.
///
/// The schema documents *why* a column for raw material does not exist, in
/// prose that necessarily contains the words a naive scan is looking for. An
/// earlier version of this audit failed on its own explanation.
fn sql_without_comments(relative: &str) -> String {
    source(relative)
        .lines()
        .map(|line| match line.find("--") {
            Some(i) => &line[..i],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase()
}

#[test]
fn no_safe_event_field_reuses_a_refused_name() {
    let declared = declared_event_fields();
    for field in &declared {
        assert!(
            !REFUSED.contains(&field.as_str()),
            "the safe-event model declares `{field}`, which the synchronization \
             boundary refuses; two boundaries on one server must not disagree \
             about a name (FR-777a, SC-751)"
        );
    }
    // And the substitutes the contract chose really are present, so this test
    // cannot pass by the model having lost the fields instead of renaming them.
    for substitute in [
        "repo_file",
        "repo_file_from",
        "command_line",
        "test_command",
        "exit_status",
        "test_outcome",
        "failure_note",
        "failure_kind",
    ] {
        assert!(
            declared.iter().any(|f| f == substitute),
            "`{substitute}` is missing from the event model, so the name it \
             replaces has nowhere to go"
        );
    }
}

#[test]
fn there_is_no_substitute_for_a_human_readable_gist() {
    // `summary` has no replacement and must not acquire one. A per-event
    // human-readable gist is the field transcript content leaks through, and
    // kind plus typed content already carries the meaning
    // (`contracts/safe-events.md` §2).
    let declared = declared_event_fields();
    for banned in [
        "summary",
        "gist",
        "description",
        "message",
        "text",
        "prompt",
        "transcript",
        "note",
        "raw",
        "body",
    ] {
        assert!(
            !declared.iter().any(|f| f == banned),
            "the event model declares `{banned}`, which is a gist field by \
             another name"
        );
    }
}

#[test]
fn no_local_or_server_column_can_hold_raw_material() {
    let pg = pg!();
    // The server side, read from the live schema rather than the migration
    // text, so a column added by a later migration is seen too.
    let columns = pg.server.query_column(
        "SELECT table_name || '.' || column_name
           FROM information_schema.columns
          WHERE table_schema = 'public'",
    );
    assert!(columns.len() > 100, "the schema scan found nothing");

    for column in &columns {
        let name = column.split('.').next_back().unwrap_or("");
        for banned in [
            "transcript",
            "raw_payload",
            "raw_json",
            "vendor_json",
            "vendor_payload",
            "prompt",
            "tool_output",
            "stdout",
            "stderr",
            "message_text",
        ] {
            assert_ne!(
                name, banned,
                "`{column}` is a column raw material could be written into \
                 (FR-730, FR-763, SC-730)"
            );
        }
        // A refused name must not appear as a column on any Feature 005 table
        // either.
        if column.starts_with("safe_events.")
            || column.starts_with("knowledge_candidates.")
            || column.starts_with("shared_patterns.")
            || column.starts_with("retrieval_trace")
        {
            assert!(
                !REFUSED.contains(&name),
                "`{column}` reuses a refused name on a Feature 005 table"
            );
        }
    }
}

#[test]
fn the_local_schema_gained_no_raw_material_column_either() {
    // The local side is where raw material actually passes through, so its
    // schema is the one where a "just cache it for a moment" column would
    // appear first. Comments are stripped: the file explains at length why such
    // a column does not exist, and the explanation contains the words.
    let sql = sql_without_comments("crates/cairn-store/migrations/0008_safe_events.sql");
    for banned in [
        "transcript",
        "raw_payload",
        "raw_json",
        "vendor_json",
        "vendor_payload",
        "prompt",
        "tool_output",
        "stdout",
        "stderr",
    ] {
        assert!(
            !sql.contains(banned),
            "local schema v8 declares something matching `{banned}`; raw \
             material has nowhere durable to land, and that is what makes the \
             guarantee structural rather than a promise"
        );
    }
    // `payload` on the spool is the approved `SafeCanonicalEvent` and nothing
    // else, and the schema says so in as many words. The audit checks the
    // statement survives, because a column called `payload` is exactly the one
    // that would quietly widen.
    let with_comments = source("crates/cairn-store/migrations/0008_safe_events.sql");
    assert!(
        with_comments.contains("the approved `SafeCanonicalEvent`"),
        "event_spool.payload no longer states what it may hold"
    );
}

#[test]
fn the_ingest_boundary_refuses_every_name_the_sync_boundary_does() {
    // The two lists have to be the same list. A name refused by one boundary
    // and accepted by the other is the drift FR-777a1 exists to prevent, and it
    // would be invisible until something used it.
    let events = source("crates/cairn-server/src/events.rs");
    let start = events
        .find("const REFUSED_FIELD_NAMES")
        .expect("the ingest refusal list is gone");
    let list = &events[start..start + events[start..].find("];").expect("list end")];
    for name in REFUSED {
        assert!(
            list.contains(&format!("\"{name}\"")),
            "the ingest boundary does not refuse `{name}`, which the sync \
             boundary does"
        );
    }
}

#[test]
fn the_event_model_carries_no_project_or_account_field() {
    // Bound server-side from the credential and the verified session (FR-769,
    // FR-769a). A client that could name either could attribute another
    // account's work, so the fields do not exist rather than being validated.
    let declared = declared_event_fields();
    for absent in ["project_id", "account_id", "owner_user_id"] {
        assert!(
            !declared.iter().any(|f| f == absent),
            "the event envelope declares `{absent}`, which a client must not \
             be able to assert"
        );
    }
}
