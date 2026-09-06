//! Canonical pattern delivery, and the repair for the defect that broke it
//! (`retrieve.rs::SectionPattern`, `deliver.rs::merge_durable_sections`,
//! `briefing.rs::level1_patterns` gate, `wire.rs::BriefingPattern`).
//!
//! # The defect this file exists to keep fixed
//!
//! The server selected a canonical `shared_patterns` row X, budgeted it and
//! recorded `pattern:X` as the selected trace item. The daemon then dropped
//! the response's `patterns` section entirely and ran its own
//! `level1_patterns()` over `reusable_patterns` — a different local store,
//! keyed by different identities — rendering a local match Y, or nothing.
//! The daemon then reported the transmission successful anyway, and the
//! server copied its selected refs into `delivered_context`. X was recorded
//! delivered without the record behind it ever having been rendered, and any
//! later retrieval withheld X forever on the strength of a delivery that
//! never happened.
//!
//! Closing that gap took four changes, and each has a test here that fails
//! without it: the server's selection carries the canonical fields all the
//! way to the wire (`SectionPattern`); the daemon merges *only* those fields
//! under the id the server traced (`merge_durable_sections`); the daemon's
//! own local matcher is gated off entirely once a project's briefing is
//! server-authoritative (`Durable::FromServer`); and a canonical selection
//! carries no invented signal count, because no signal comparison ran for it
//! (`BriefingPattern::signal_overlap`).
//!
//! # Two fixtures, because the defect spans two stores
//!
//! Trace and delivery evidence (`retrieval_traces`, `retrieval_trace_items`,
//! `delivered_context`) lives on the server and is read directly through
//! `feature005::Pg`, exactly as `feature005_delivery.rs` and
//! `feature005_retrieval_traces.rs` already do — no daemon needed to prove a
//! fact about what the server itself recorded.
//!
//! What the *agent* actually receives can only be seen by driving a real,
//! linked `Sandbox` through its hook path (`hook_as`), the way
//! `feature005_briefing_cache.rs` does: `hookSpecificOutput.additionalContext`
//! is the literal text an agent reads, and nothing short of that path proves
//! the daemon rendered the server's answer rather than something of its own.
//! Tests that need to catch a *local* substitute (a `reusable_patterns` row,
//! seeded with `exec_sql`) necessarily use this fixture, since the substitute
//! exists only in the local store the direct-API fixture never touches.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{attach_server, post_json_status_bearer, Sandbox};
use serde_json::{json, Value};
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
// Direct-API fixture helpers (trace and delivery evidence, on the server)
// ---------------------------------------------------------------------------

fn retrieve(pg: &Pg, who: &Account, session: Uuid) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        "/api/retrieve",
        &json!({ "session_id": session, "trigger": "session_open" }),
        &who.token,
    )
}

fn report(pg: &Pg, who: &Account, trace: &str, body: Value) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        &format!("/api/retrieval-traces/{trace}/transmission"),
        &body,
        &who.token,
    )
}

fn trace_id_of(resp: &Value) -> String {
    resp["trace_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no trace_id in {resp}"))
        .to_string()
}

fn trace_state(pg: &Pg, trace: &str) -> String {
    pg.server.text(&format!(
        "SELECT delivery_state FROM retrieval_traces WHERE trace_id = '{trace}'"
    ))
}

fn delivered_reference_keys(pg: &Pg, session: Uuid) -> Vec<String> {
    pg.server.query_column(&format!(
        "SELECT reference_key FROM delivered_context
          WHERE session_id = '{session}' ORDER BY reference_key"
    ))
}

fn delivered_count_for(pg: &Pg, session: Uuid, key: &str) -> i64 {
    pg.server.count(&format!(
        "SELECT count(*) FROM delivered_context
          WHERE session_id = '{session}' AND reference_key = '{key}'"
    ))
}

/// The `reference_key`s of a retrieval's *selected* `patterns` items.
///
/// Scoped to the `patterns` section deliberately: this file is about pattern
/// delivery specifically, and reading only that section is what keeps a
/// project-memory item that happens to share a rank from ever being mistaken
/// for the pattern this file is asserting about.
fn selected_pattern_keys(resp: &Value) -> std::collections::BTreeSet<String> {
    resp["sections"]["patterns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["reference_key"].as_str())
        .map(str::to_string)
        .collect()
}

/// Seed one `shared_patterns` row for `pg.owner`, with a title and an
/// approach both derived from `marker` — which is what makes a single
/// substring check on the rendered text specific to *this* record and not a
/// coincidence of fixture text.
fn seed_pattern(pg: &Pg, marker: &str) -> Uuid {
    let id = Uuid::now_v7();
    let stored = pg.seed_pattern_with_id(&pg.owner, id, marker);
    assert!(
        stored,
        "shared_patterns does not exist at this schema version; the fixture \
         cannot seed a canonical pattern to test against"
    );
    id
}

// ---------------------------------------------------------------------------
// Sandbox fixture helpers (what the agent actually receives, and the local
// store a linked project's briefing must never read from)
// ---------------------------------------------------------------------------

/// A sandbox linked to the fixture's server, authenticated as `pg.owner`.
fn linked_device(pg: &Pg, label: &str) -> Sandbox {
    let sandbox = Sandbox::new();
    let remote = format!("git@localhost:cairnfixture/{label}.git");
    sandbox.git(&["remote", "add", "origin", &remote]);
    sandbox.must(&["init"]);
    attach_server(&sandbox, &pg.server, &pg.owner.token);
    sandbox.must(&["link", "--project", &pg.project.to_string()]);
    sandbox
}

fn settle(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timed out waiting for: {what}");
}

/// Fire a `SessionStart` hook and return what the agent actually received:
/// `hookSpecificOutput.additionalContext`, read from the hook's own stdout —
/// not a JSON structure this test reaches in behind the daemon's back.
fn open_session(sandbox: &Sandbox, key: &str) -> String {
    let out = sandbox.hook_as(
        "claude-code",
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    );
    assert_eq!(out.code, 0, "a hook always exits 0: {}", out.stderr);
    let emitted: Value = serde_json::from_str(out.stdout.trim())
        .unwrap_or_else(|e| panic!("hook did not emit context JSON ({e}): {:?}", out.stdout));
    emitted["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Open a session on a linked sandbox and wait until the server holds it,
/// then return this machine's own id for it — the same id the server holds,
/// since synchronization preserves the primary key.
///
/// A session that has not yet reached the server cannot be bound by
/// `/api/retrieve` (`auth::bind_session`), so the *first* `SessionStart` on a
/// freshly linked sandbox legitimately answers `fresh_knowledge_unavailable`
/// rather than a real selection — this exists so every test below drives its
/// real assertion against a session the server can actually resolve.
fn established_session(pg: &Pg, sandbox: &Sandbox, key: &str) -> Uuid {
    let _ = open_session(sandbox, key);
    settle("the session reaches the server", || {
        pg.server.count(&format!(
            "SELECT count(*) FROM sessions WHERE project_id = '{}'",
            pg.project
        )) > 0
    });
    sandbox
        .query_column("SELECT id FROM sessions ORDER BY started_at DESC LIMIT 1")
        .first()
        .and_then(|s| s.parse().ok())
        .expect("a synced local session")
}

/// The sandbox's own local project row id — the id `reusable_patterns` and
/// `memories` foreign keys point at, which is never the server's project id.
fn local_project_id(sandbox: &Sandbox) -> String {
    sandbox
        .query_column("SELECT id FROM projects WHERE deleted_at IS NULL ORDER BY created_at")
        .first()
        .cloned()
        .expect("the sandbox's own project row")
}

/// Seed a local, never-promoted `reusable_patterns` row directly — the local
/// matcher's own candidate, and the one a linked project's briefing must
/// never substitute for the server's canonical selection.
fn seed_local_pattern(sandbox: &Sandbox, id: Uuid, title: &str, approach: &str, signals: &[&str]) {
    let signals_json = serde_json::to_string(signals).expect("signals encode");
    sandbox.exec_sql(&format!(
        "INSERT INTO reusable_patterns
            (id, title, problem, signals, signal_digest, applicability, root_cause,
             root_cause_digest, approach, constraints, trust, origin_ref,
             sanitization_report, created_at, updated_at)
         VALUES ('{id}', '{title}', '{title} problem', '{signals_json}', 'digest-{id}',
                 '[]', '{title} root cause', 'rc-{id}', '{approach}', '[]',
                 'sanitized', 'salted-{id}', '{{}}', '2026-09-01T09:00:00Z',
                 '2026-09-01T09:00:00Z')"
    ));
}

/// Seed project-scope `failure` memories carrying the given signal texts, so
/// the local matcher's own `project_signals` has something to match against.
fn seed_local_failure_signals(
    sandbox: &Sandbox,
    project_id: &str,
    session_id: &str,
    texts: &[&str],
) {
    for (i, text) in texts.iter().enumerate() {
        sandbox.exec_sql(&format!(
            "INSERT INTO memories (id, project_id, type, scope, scope_key, content, state,
                                   origin_session_id, created_at, updated_at)
             VALUES ('{}', '{project_id}', 'failure', 'project', '{project_id}', '{text}',
                     'active', '{session_id}', '2026-09-01T09:0{i}:00Z',
                     '2026-09-01T09:0{i}:00Z')",
            Uuid::now_v7()
        ));
    }
}

// ---------------------------------------------------------------------------
// 1 — exact canonical render
// ---------------------------------------------------------------------------

/// The briefing an agent actually receives carries X's canonical title and
/// approach, read from the literal hook output rather than from any
/// structure this test reaches in behind the daemon's back.
///
/// **Falsified by** the daemon rendering anything else — a local
/// substitute, a stale cache entry, or nothing at all — under the id the
/// server selected, budgeted and traced.
#[test]
fn the_agent_receives_xs_exact_canonical_title_and_approach() {
    let pg = pg!();
    let sandbox = linked_device(&pg, "canonical-render");
    let key = format!("canon-{}", Uuid::now_v7());
    established_session(&pg, &sandbox, &key);

    let marker = format!("canonical-x-{}", Uuid::now_v7().simple());
    seed_pattern(&pg, &marker);

    let context = open_session(&sandbox, &key);
    assert!(
        context.contains(&marker),
        "the briefing the agent received did not carry X's canonical title: {context}"
    );
    assert!(
        context.contains(&format!("{marker} approach")),
        "the briefing the agent received did not carry X's canonical approach: {context}"
    );
}

// ---------------------------------------------------------------------------
// 2 — trace correspondence
// ---------------------------------------------------------------------------

/// The trace records exactly `pattern:X` as selected, and a successful
/// transmission report leaves `delivered_context` holding exactly that
/// reference for this session — no more, no fewer, no substitute.
///
/// **Falsified by** a trace whose selected pattern item names something
/// other than X, or a transmission report writing a delivery row for a
/// reference the trace never selected.
#[test]
fn the_selected_trace_item_is_pattern_x_and_delivered_context_matches_it_exactly() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let x = seed_pattern(&pg, &format!("trace-corr-{}", Uuid::now_v7()));

    let (resp, status) = retrieve(&pg, &pg.owner, session);
    assert_eq!(status, 200, "{resp}");
    let trace = trace_id_of(&resp);

    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM retrieval_trace_items
              WHERE trace_id = '{trace}' AND status = 'selected'
                AND ref_kind = 'pattern' AND reference_key = 'pattern:{x}'"
        )),
        1,
        "the trace's own selected pattern item was not exactly pattern:{x}"
    );

    let (report_body, status) = report(&pg, &pg.owner, &trace, json!({ "outcome": "transmitted" }));
    assert_eq!(status, 200, "{report_body}");

    assert_eq!(
        delivered_reference_keys(&pg, session),
        vec![format!("pattern:{x}")],
        "delivered_context did not hold exactly pattern:{x} for this session"
    );
}

// ---------------------------------------------------------------------------
// 3 — no local substitution
// ---------------------------------------------------------------------------

/// A conflicting local `reusable_patterns` row Y, whose signals would match
/// this session's own recorded failures, never substitutes for the server's
/// canonical selection X once a project's briefing is server-authoritative
/// — the exact defect this feature repaired.
///
/// **Falsified by** the rendered briefing carrying Y's title or approach in
/// place of, or alongside, X's.
#[test]
fn a_conflicting_local_pattern_never_substitutes_for_the_servers_canonical_selection() {
    let pg = pg!();
    let sandbox = linked_device(&pg, "no-local-substitution");
    let key = format!("nosub-{}", Uuid::now_v7());
    established_session(&pg, &sandbox, &key);

    let local_project = local_project_id(&sandbox);
    let local_session = sandbox
        .query_column("SELECT id FROM sessions ORDER BY started_at DESC LIMIT 1")
        .first()
        .cloned()
        .expect("a local session");

    // Y: a conflicting local pattern whose signals match this project's own
    // recorded failures — exactly what the old fallback would have offered.
    let y_title = format!("LOCAL-Y-TITLE-{}", Uuid::now_v7().simple());
    let y_approach = format!("LOCAL-Y-APPROACH-{}", Uuid::now_v7().simple());
    seed_local_pattern(
        &sandbox,
        Uuid::now_v7(),
        &y_title,
        &y_approach,
        &["flakywidget9137", "brokenconnector9137"],
    );
    seed_local_failure_signals(
        &sandbox,
        &local_project,
        &local_session,
        &[
            "the flakywidget9137 keeps crashing under load",
            "a brokenconnector9137 followed shortly after",
        ],
    );

    // X: canonical, on the server.
    let marker = format!("canonical-x-{}", Uuid::now_v7().simple());
    seed_pattern(&pg, &marker);

    let context = open_session(&sandbox, &key);
    assert!(
        context.contains(&marker),
        "the linked briefing did not render the server's canonical pattern X: {context}"
    );
    assert!(
        !context.contains(&y_title),
        "a conflicting local pattern's title leaked into a linked, \
         server-authoritative briefing: {context}"
    );
    assert!(
        !context.contains(&y_approach),
        "a conflicting local pattern's approach leaked into a linked, \
         server-authoritative briefing: {context}"
    );
}

// ---------------------------------------------------------------------------
// 4 — no unpromoted local leak
// ---------------------------------------------------------------------------

/// A local `reusable_patterns` row that was never promoted to the server
/// cannot appear in a linked project's automatic briefing at all, even when
/// it is locally relevant — the local matcher must not run once a project's
/// briefing is server-authoritative, not merely lose an arbitration.
///
/// **Falsified by** the rendered text or `briefing.patterns[]` naming a
/// local, never-promoted record.
#[test]
fn an_unpromoted_local_pattern_never_reaches_a_linked_automatic_briefing() {
    let pg = pg!();
    let sandbox = linked_device(&pg, "no-unpromoted-leak");
    let key = format!("noleak-{}", Uuid::now_v7());
    established_session(&pg, &sandbox, &key);

    let local_project = local_project_id(&sandbox);
    let local_session = sandbox
        .query_column("SELECT id FROM sessions ORDER BY started_at DESC LIMIT 1")
        .first()
        .cloned()
        .expect("a local session");

    let y_title = format!("LOCAL-UNPROMOTED-TITLE-{}", Uuid::now_v7().simple());
    let y_approach = format!("LOCAL-UNPROMOTED-APPROACH-{}", Uuid::now_v7().simple());
    seed_local_pattern(
        &sandbox,
        Uuid::now_v7(),
        &y_title,
        &y_approach,
        &["stalecache8214", "orphanedlockfile8214"],
    );
    seed_local_failure_signals(
        &sandbox,
        &local_project,
        &local_session,
        &[
            "a stalecache8214 was never invalidated on deploy",
            "an orphanedlockfile8214 blocked the retry loop",
        ],
    );
    assert_eq!(
        sandbox.query_column(&format!(
            "SELECT CAST(count(*) AS TEXT) FROM reusable_patterns WHERE title = '{y_title}'"
        )),
        vec!["1".to_string()],
        "the local pattern was never written, so its absence below would prove nothing"
    );

    let context = open_session(&sandbox, &key);
    assert!(
        !context.contains(&y_title),
        "an unpromoted local pattern's title reached a linked automatic briefing: {context}"
    );
    assert!(
        !context.contains(&y_approach),
        "an unpromoted local pattern's approach reached a linked automatic briefing: {context}"
    );

    let payload = sandbox.json(&["context", "--session", &local_session]);
    assert!(
        payload["briefing"].get("patterns").is_none(),
        "no canonical pattern exists on the server, so `patterns` must be \
         entirely absent from a linked briefing rather than carry the \
         unpromoted local row: {payload}"
    );
}

// ---------------------------------------------------------------------------
// 8 — transmission failure writes no delivery
// ---------------------------------------------------------------------------

/// A reported transmission failure for X's trace leaves the trace `failed`
/// with its declared reason, and writes no `delivered_context` row for X —
/// dedup must never withhold, for the life of the session, a pattern the
/// agent never actually received.
///
/// **Falsified by** a delivery row appearing for X despite the reported
/// failure, or the trace surviving as anything but `failed`.
#[test]
fn a_reported_transmission_failure_for_pattern_x_writes_no_delivery_row() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let x = seed_pattern(&pg, &format!("failed-transmit-{}", Uuid::now_v7()));

    let (resp, status) = retrieve(&pg, &pg.owner, session);
    assert_eq!(status, 200, "{resp}");
    let trace = trace_id_of(&resp);
    assert!(
        selected_pattern_keys(&resp).contains(&format!("pattern:{x}")),
        "X was never selected, so this test proves nothing: {resp}"
    );

    let (report_body, status) = report(
        &pg,
        &pg.owner,
        &trace,
        json!({ "outcome": "failed", "failure_reason": "hook_transmission_failed" }),
    );
    assert_eq!(status, 200, "{report_body}");
    assert_eq!(trace_state(&pg, &trace), "failed");
    assert_eq!(
        delivered_count_for(&pg, session, &format!("pattern:{x}")),
        0,
        "a failed transmission still wrote a delivery row for X"
    );
}

// ---------------------------------------------------------------------------
// 9 — dedup truth
// ---------------------------------------------------------------------------

/// X is excluded from a later retrieval only once it was actually rendered
/// *and* transmitted — never before, and always after.
///
/// Three retrievals against the same session: the first selects X; a second,
/// run before any transmission is reported, still offers X, because nothing
/// has established that it was ever received; a third, run after a
/// successful report, no longer offers it.
///
/// **Falsified by** X disappearing before any report, or surviving after a
/// successful one.
#[test]
fn x_is_excluded_only_after_it_was_rendered_and_transmitted() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let x = seed_pattern(&pg, &format!("dedup-{}", Uuid::now_v7()));
    let key = format!("pattern:{x}");

    let (first, status) = retrieve(&pg, &pg.owner, session);
    assert_eq!(status, 200, "{first}");
    assert!(
        selected_pattern_keys(&first).contains(&key),
        "the first retrieval never selected X, so this test proves nothing: {first}"
    );

    // Before any transmission report, a second retrieval still offers it.
    let (second, status) = retrieve(&pg, &pg.owner, session);
    assert_eq!(status, 200, "{second}");
    assert!(
        selected_pattern_keys(&second).contains(&key),
        "X was withheld before any transmission was ever reported: {second}"
    );

    let trace = trace_id_of(&second);
    let (report_body, status) = report(&pg, &pg.owner, &trace, json!({ "outcome": "transmitted" }));
    assert_eq!(status, 200, "{report_body}");

    // After a successful report, a third retrieval no longer offers it.
    let (third, status) = retrieve(&pg, &pg.owner, session);
    assert_eq!(status, 200, "{third}");
    assert!(
        !selected_pattern_keys(&third).contains(&key),
        "X was still offered after a successful transmission was reported: {third}"
    );
}

// ---------------------------------------------------------------------------
// 11 — budget correspondence
// ---------------------------------------------------------------------------

/// The selected item's `content` — what the retrieval budget was actually
/// spent on — is exactly `"{title} — Approach: {approach}"` for X, and the
/// briefing an agent actually receives carries that same title and approach.
/// The point is not a token count (an implementation detail this test
/// deliberately never asserts): it is that the content the cost was computed
/// from and the content that was rendered are one record, not two.
///
/// **Falsified by** the budgeted `content` differing from the canonical
/// shape, or the rendered briefing carrying a title or approach other than
/// the one the cost was computed from.
#[test]
fn the_selected_items_cost_is_computed_from_exactly_the_content_rendered() {
    let pg = pg!();
    let sandbox = linked_device(&pg, "budget-correspondence");
    let key = format!("budget-{}", Uuid::now_v7());
    let session = established_session(&pg, &sandbox, &key);

    let marker = format!("budget-x-{}", Uuid::now_v7().simple());
    seed_pattern(&pg, &marker);

    // The server's own selection: `content` is what the budget was spent on.
    let (resp, status) = retrieve(&pg, &pg.owner, session);
    assert_eq!(status, 200, "{resp}");
    let items = resp["sections"]["patterns"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(items.len(), 1, "{resp}");
    let item = &items[0];
    let expected_content = format!("{marker} — Approach: {marker} approach");
    assert_eq!(
        item["content"], expected_content,
        "the budgeted content was not the canonical `{{title}} — Approach: \
         {{approach}}` shape: {item}"
    );
    assert!(
        item["cost"].as_u64().unwrap_or(0) > 0,
        "a selected item's cost was zero, so nothing was actually budgeted: {item}"
    );

    // No transmission was reported for the call above, so X is still fresh:
    // the rendered text an agent actually receives carries the same title
    // and approach the cost above was computed from.
    let context = open_session(&sandbox, &key);
    assert!(
        context.contains(&marker),
        "the rendered briefing did not carry the title the cost was computed from: {context}"
    );
    assert!(
        context.contains(&format!("{marker} approach")),
        "the rendered briefing did not carry the approach the cost was computed from: {context}"
    );
}

// ---------------------------------------------------------------------------
// 13 — response-loss idempotency
// ---------------------------------------------------------------------------

/// Reporting `transmitted` twice for the same trace leaves exactly one
/// `delivered_context` row for X and one `transmitted` trace, and the
/// second call is answered — as a duplicate — rather than refused. A retry
/// after a lost response is the ordinary case this endpoint exists for, and
/// refusing it would force a caller to choose between reporting twice and
/// never confirming delivery at all.
///
/// **Falsified by** a second delivery row appearing for X, or the repeated
/// report coming back as anything but a successful, explicit duplicate.
#[test]
fn reporting_transmitted_twice_for_x_leaves_one_delivery_and_is_answered_not_refused() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let x = seed_pattern(&pg, &format!("idempotent-{}", Uuid::now_v7()));

    let (resp, status) = retrieve(&pg, &pg.owner, session);
    assert_eq!(status, 200, "{resp}");
    let trace = trace_id_of(&resp);

    let (first, status) = report(&pg, &pg.owner, &trace, json!({ "outcome": "transmitted" }));
    assert_eq!(status, 200, "{first}");
    assert_eq!(
        delivered_count_for(&pg, session, &format!("pattern:{x}")),
        1
    );

    let (second, status) = report(&pg, &pg.owner, &trace, json!({ "outcome": "transmitted" }));
    assert_eq!(
        status, 200,
        "a retry after a lost response was refused rather than answered: {second}"
    );
    assert_eq!(second["status"], "duplicate", "{second}");
    assert_eq!(
        delivered_count_for(&pg, session, &format!("pattern:{x}")),
        1,
        "a duplicate report wrote a second delivery row for X"
    );
    assert_eq!(trace_state(&pg, &trace), "transmitted");
}

// ---------------------------------------------------------------------------
// Guard — signal_overlap is present only where a comparison actually ran
// ---------------------------------------------------------------------------

/// `briefing.patterns[]` for a linked project's automatic briefing carries
/// **no** `signal_overlap` field at all — the server selects by budget, and a
/// count would assert a signal comparison that never ran. An **unlinked**
/// project's briefing, built by the local matcher that genuinely does
/// compare, still reports its real overlap count.
///
/// This is the guard on the whole file: every test above proves what a
/// linked briefing contains; this proves the *shape* of what it contains is
/// held to the same honesty the local matcher already was.
///
/// **Falsified by** a canonical pattern inventing a signal count, or a
/// locally matched pattern losing the count it actually computed.
#[test]
fn signal_overlap_is_present_only_where_a_signal_comparison_actually_ran() {
    // Linked half: a server-authoritative briefing carries no signal_overlap.
    let pg = pg!();
    let sandbox = linked_device(&pg, "signal-overlap-linked");
    let key = format!("overlap-linked-{}", Uuid::now_v7());
    let session = established_session(&pg, &sandbox, &key);
    seed_pattern(&pg, &format!("overlap-x-{}", Uuid::now_v7().simple()));

    let linked_payload = sandbox.json(&["context", "--session", &session.to_string()]);
    let linked_patterns = linked_payload["briefing"]["patterns"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !linked_patterns.is_empty(),
        "the canonical pattern was never rendered, so this proves nothing: {linked_payload}"
    );
    for item in &linked_patterns {
        assert!(
            item.get("signal_overlap").is_none(),
            "a server-selected canonical pattern carried `signal_overlap`, \
             asserting a signal comparison the server never ran: {item}"
        );
    }

    // Unlinked half: no server authority, so the local matcher is the only
    // authority there is — and it still reports what it actually compared.
    let unlinked = Sandbox::new();
    let ukey = format!("overlap-unlinked-{}", Uuid::now_v7());
    let out = unlinked.hook_as(
        "claude-code",
        "SessionStart",
        json!({ "session_id": ukey, "source": "startup" }),
    );
    assert_eq!(out.code, 0, "a hook always exits 0: {}", out.stderr);
    let local_project = local_project_id(&unlinked);
    let local_session = unlinked
        .query_column("SELECT id FROM sessions ORDER BY started_at DESC LIMIT 1")
        .first()
        .cloned()
        .expect("a local session");

    seed_local_pattern(
        &unlinked,
        Uuid::now_v7(),
        "LOCAL-MATCHER-TITLE",
        "LOCAL-MATCHER-APPROACH",
        &["timeoutissue4471", "retrystorm4471"],
    );
    seed_local_failure_signals(
        &unlinked,
        &local_project,
        &local_session,
        &[
            "a timeoutissue4471 was recorded again this morning",
            "a retrystorm4471 followed immediately after",
        ],
    );

    let unlinked_payload = unlinked.json(&["context", "--session", &local_session]);
    let unlinked_patterns = unlinked_payload["briefing"]["patterns"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !unlinked_patterns.is_empty(),
        "the local matcher never offered its pattern, so this proves nothing: {unlinked_payload}"
    );
    for item in &unlinked_patterns {
        assert!(
            item.get("signal_overlap").and_then(Value::as_u64).is_some(),
            "the local matcher's own comparison must be reported, not omitted: {item}"
        );
    }
}

// ---------------------------------------------------------------------------
// 14 — the local assembly spends no budget on patterns it will not render
// ---------------------------------------------------------------------------

/// A linked project's *local* assembly holds no patterns at all, so it can
/// never report the `patterns` section omitted for budget — there is nothing
/// there to omit. The canonical section is merged in afterwards, against the
/// budget the server already spent and reported.
///
/// This is the half of budget truthfulness test 7 cannot see. Test 7 proves
/// the server's own selected-item cost was computed from the content that
/// was rendered; this proves the daemon does not *also* charge its local
/// budget for a pattern nobody will ever read. Running `level1_patterns` for
/// a linked project is invisible in the rendered text — the merge overwrites
/// whatever it produced — but it is not free: it spends the local budget on
/// a candidate that is then thrown away, and the briefing then costs more
/// than it says it does (FR-029).
///
/// The bait is deliberately far too large to fit the local budget below, so
/// a local matcher that ran at all must report `patterns` omitted.
///
/// **Falsified by** `omitted_sections` naming `patterns` for a linked
/// briefing, which can only mean the local matcher offered one.
#[test]
fn a_linked_briefings_local_assembly_spends_no_budget_on_local_patterns() {
    let pg = pg!();
    let sandbox = linked_device(&pg, "no-local-pattern-spend");
    let key = format!("localspend-{}", Uuid::now_v7());
    let session = established_session(&pg, &sandbox, &key);

    let marker = format!("spend-x-{}", Uuid::now_v7().simple());
    seed_pattern(&pg, &marker);

    let local_project = local_project_id(&sandbox);
    let local_session = session.to_string();
    let fat = "z".repeat(8000);
    seed_local_pattern(
        &sandbox,
        Uuid::now_v7(),
        "LOCAL-FAT-TITLE",
        &fat,
        &["budgeteater5501", "spendtrap5501"],
    );
    seed_local_failure_signals(
        &sandbox,
        &local_project,
        &local_session,
        &[
            "a budgeteater5501 was recorded again this morning",
            "a spendtrap5501 followed immediately after",
        ],
    );

    let payload = sandbox.json(&["context", "--session", &local_session, "--budget", "150"]);
    let omitted: Vec<String> = payload["omitted_sections"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        !omitted.contains(&"patterns".to_string()),
        "a linked briefing reported the `patterns` section omitted for \
         budget, which it can only do if the local matcher offered one and \
         was charged for it: {payload}"
    );
    assert!(
        !payload.to_string().contains("LOCAL-FAT-TITLE"),
        "the local bait reached a linked briefing: {payload}"
    );
}
