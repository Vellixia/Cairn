//! The canonical-pattern repair, at the boundaries a single successful
//! delivery cannot see: what a destroyed local store does to it, what an
//! outage cache may and may not reuse, and whose it is — by account and by
//! server instance.
//!
//! `tests/tests/feature005_pattern_delivery.rs` owns the end-to-end honesty
//! of one delivery (the server selects X, the daemon renders X, the trace
//! and `delivered_context` agree). This file owns five properties that
//! survive across a *second* event: the machine is rebuilt, the server goes
//! away, a different account signs in, or a different deployment answers at
//! the address this daemon still believes is the one that authorized it.
//!
//! # House rule
//!
//! A test here that finds a real defect leaves the failing assertion in
//! place and reports it — it does not weaken the assertion to get green, and
//! it does not touch production code.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{attach_server, post_json_status_bearer, Sandbox, Server};
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

fn server() -> Option<Server> {
    match Server::start_own_database() {
        Some(s) => Some(s),
        None => {
            eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
            None
        }
    }
}

const SETTLE: Duration = Duration::from_secs(30);

fn settle(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + SETTLE;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timed out waiting for: {what}");
}

// ---------------------------------------------------------------------------
// A linked device — one account, one project, one sandbox — for the tests
// that drive a real daemon rather than the server's API alone.
// ---------------------------------------------------------------------------

struct Device {
    sandbox: Sandbox,
    project: Uuid,
    token: String,
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
    Device {
        sandbox,
        project,
        token,
    }
}

/// Promote a pattern through the real route, as `device` does — so it is the
/// server's own canonical record, `content_key` and all, rather than a row
/// this file happens to know the shape of.
///
/// Content is chosen free of anything the global content screen matches: the
/// fixture project's remote is `git@localhost:cairnfixture/<label>.git`, and
/// `all_identities_for` screens every promotion against the caller's own
/// projects. A marker word unrelated to any device label in this file (an
/// animal name, never a label substring) keeps that screen out of the way of
/// the property actually under test.
fn promote_pattern(
    server: &Server,
    token: &str,
    title: &str,
    problem: &str,
    approach: &str,
) -> Uuid {
    let (body, status) = post_json_status_bearer(
        &server.base,
        "/api/patterns",
        &json!({
            "title": title,
            "problem": problem,
            "root_cause": format!("{problem}, traced back one step further"),
            "approach": approach,
            "constraints": [],
            "applicability": [],
        }),
        token,
    );
    assert_eq!(status, 200, "promoting the fixture pattern: {body}");
    body["pattern_id"]
        .as_str()
        .expect("pattern_id")
        .parse()
        .expect("a uuid")
}

/// A local `reusable_patterns` row, seeded directly — the bait. It is never
/// promoted and never authorized by anyone, so its title must never appear in
/// any briefing this file inspects. If it does, the daemon's own local
/// matcher (`level1_patterns`) ran somewhere it must not: on an outage cache
/// hit, or on a cache miss for an account nobody has authorized (repair
/// item 3 — `briefing.rs` runs it only when `durable == Durable::Local`,
/// which a linked, server-side project never is).
fn seed_local_pattern(sandbox: &Sandbox, marker: &str) {
    sandbox.exec_sql(&format!(
        "INSERT INTO reusable_patterns (id, title, problem, signals, signal_digest,
                                        applicability, root_cause, root_cause_digest,
                                        approach, constraints, trust, origin_ref,
                                        sanitization_report, created_at, updated_at)
         VALUES ('{}', '{marker}', '{marker}-problem',
                 '[\"{marker}sigone\",\"{marker}sigtwo\"]', 'digest-{marker}', '[]',
                 '{marker}-root-cause', 'rc-{marker}', '{marker}-approach', '[]',
                 'sanitized', 'salted-{marker}', '{{}}', '2026-09-01T09:00:00Z',
                 '2026-09-01T09:00:00Z')",
        Uuid::now_v7()
    ));
}

/// Give the bait above something to be matched *against*.
///
/// `briefing::level1_patterns` returns early when `project_signals` is empty,
/// and `project_signals` is built from this project's recorded errors and
/// `failure` memories. A bait pattern seeded without them can never be
/// selected by the local matcher, which would make every "the bait did not
/// leak" assertion below true for the wrong reason — the local matcher was
/// never in a position to offer it. These memories carry the bait's own
/// signal tokens, so the matcher would offer it the moment it ran.
fn seed_matching_local_signals(sandbox: &Sandbox, marker: &str) {
    let project_id = sandbox
        .query_column("SELECT id FROM projects WHERE deleted_at IS NULL ORDER BY created_at")
        .first()
        .cloned()
        .expect("the sandbox's own project row");
    let session_id = sandbox
        .query_column("SELECT id FROM sessions ORDER BY started_at DESC LIMIT 1")
        .first()
        .cloned()
        .expect("a local session to attribute the failure memories to");
    for (i, text) in [
        format!("a {marker}sigone was recorded again this morning"),
        format!("a {marker}sigtwo followed immediately after"),
    ]
    .iter()
    .enumerate()
    {
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

/// Open (or reopen) a session through the real hook entry point and return
/// what the agent actually received — the rendered `additionalContext`, the
/// same text a vendor session reads.
fn open_session_text(d: &Device, key: &str) -> String {
    let out = d.sandbox.hook_as(
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

// ---------------------------------------------------------------------------
// 5 — SQLite-loss recovery
// ---------------------------------------------------------------------------

/// A canonical pattern is a promise the *server* keeps, not one the local
/// store keeps on its behalf — proved by destroying the store before it has
/// ever held anything, then delivering the pattern to a session opened on
/// the rebuilt one (T083/T091's durability claim, for the `patterns` durable
/// section specifically; SC-738).
///
/// The store is destroyed and rebuilt *before* any hook, sync, or cache entry
/// has ever touched this machine, so nothing below can be riding on a local
/// row this test happened to leave behind — see
/// `tests/tests/feature005_local_loss.rs` for the same destroy-and-rebuild
/// idiom applied to the other durable domains.
///
/// **Falsified by** any repair that reads the canonical pattern from
/// `cached_patterns`, or from anywhere in the SQLite file destroying and
/// recreating the store erases: if delivery depended on a local row
/// surviving, this test destroys it before the row could ever be written.
#[test]
fn a_canonical_pattern_is_delivered_again_after_the_local_store_is_destroyed_and_rebuilt() {
    let Some(server) = server() else { return };
    let d = device(&server, "loss-device");
    let marker = format!("falcon-{}", Uuid::now_v7().simple());
    let title = format!("{marker} pattern title");
    let approach = format!("{marker} approach text");
    let pattern = promote_pattern(
        &server,
        &d.token,
        &title,
        &format!("{marker} problem statement"),
        &approach,
    );

    // Destroy the local store the way losing a laptop destroys it.
    d.sandbox.stop_daemon();
    let db = d.sandbox.db_path();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    assert!(
        !db.exists(),
        "the local store is still there; the rest of this test would prove nothing"
    );
    d.sandbox.restart_daemon();

    // Setup, not repair (FR-704): the same two commands a genuinely new
    // machine needs, and nothing else — no import, no reconciliation.
    d.sandbox.must(&["init"]);
    attach_server(&d.sandbox, &server, &d.token);
    d.sandbox
        .must(&["link", "--project", &d.project.to_string()]);

    let key = format!("loss-{}", Uuid::now_v7());
    let briefing = open_session_text(&d, &key);
    assert!(
        briefing.contains(&title) && briefing.contains(&approach),
        "the canonical pattern was not delivered to a session opened on a \
         freshly rebuilt store, even though the server never lost it: {briefing}"
    );

    settle("the rebuilt store's retrieval reaches the server", || {
        server.count(&format!(
            "SELECT count(*) FROM retrieval_traces WHERE project_id = '{}'",
            d.project
        )) > 0
    });
    let trace: String = server
        .query_column(&format!(
            "SELECT trace_id::text FROM retrieval_traces WHERE project_id = '{}'
              ORDER BY created_at DESC LIMIT 1",
            d.project
        ))
        .first()
        .cloned()
        .expect("a trace for the rebuilt-store retrieval");
    assert_eq!(
        server.text(&format!(
            "SELECT status FROM retrieval_trace_items
              WHERE trace_id = '{trace}' AND reference_key = 'pattern:{pattern}'"
        )),
        "selected",
        "the trace does not record the canonical pattern's own server \
         `PatternRef` as selected, so what was rendered above cannot be shown \
         to be what the server actually chose"
    );
}

// ---------------------------------------------------------------------------
// 6 — the authorized outage cache
// ---------------------------------------------------------------------------

/// An outage cache hit reuses *only* the exact canonical pattern this
/// account's own, previously authorized retrieval selected — never a pattern
/// from anywhere else (FR-789, FR-790a, SC-718).
///
/// The local bait (`seed_local_pattern`) is what makes this test sharp rather
/// than merely hopeful: a cache hit is exactly the moment the old,
/// pre-repair code path ran the daemon's own `level1_patterns` matcher over
/// the local store unconditionally. If that ever runs again during a cache
/// hit, this is what would leak into the answer.
///
/// **Falsified by** a cached briefing that is missing the canonical pattern's
/// content, that fails to label itself as cached, or that carries any
/// pattern reference the authorized response never selected.
#[test]
fn an_outage_cache_hit_reuses_only_the_accounts_own_authorized_pattern() {
    let Some(mut server) = server() else { return };
    let d = device(&server, "cache-device");
    let marker = format!("otter-{}", Uuid::now_v7().simple());
    let title = format!("{marker} pattern title");
    let approach = format!("{marker} approach text");
    let _pattern = promote_pattern(
        &server,
        &d.token,
        &title,
        &format!("{marker} problem statement"),
        &approach,
    );
    let bait = format!("{marker}-bait");
    seed_local_pattern(&d.sandbox, &bait);

    let key = format!("cache-{}", Uuid::now_v7());
    let warm = open_session_text(&d, &key);
    // Only now — the bait's matching signals need a local session to be
    // attributed to, and this is the first one that exists.
    seed_matching_local_signals(&d.sandbox, &bait);
    assert!(
        warm.contains(&title) && warm.contains(&approach),
        "the first, reachable-server answer never carried the canonical \
         pattern, so the outage answer below would prove nothing: {warm}"
    );
    assert!(
        !warm.contains("local cache"),
        "a briefing assembled from a reachable server claimed to be cached: {warm}"
    );

    server.go_offline();
    let cached = open_session_text(&d, &key);
    assert!(
        cached.contains("local cache"),
        "an outage briefing was not labelled as served from a cache, which \
         SC-718 requires of every cached answer: {cached}"
    );
    assert!(
        cached.contains(&title) && cached.contains(&approach),
        "the cached briefing lost the canonical pattern's own content, so an \
         outage degrades durable memory to nothing rather than to what was \
         last authorized: {cached}"
    );
    assert!(
        !cached.contains(&bait),
        "the cached briefing carries a pattern this account's authorized \
         response never selected — the local matcher ran during a cache hit, \
         which is exactly the fallback the repair's item 3 exists to close: \
         {cached}"
    );
}

// ---------------------------------------------------------------------------
// 7 — no outage fallback on a cache miss
// ---------------------------------------------------------------------------

/// A cache miss — a different account signed in on the same machine, with
/// the server unreachable — carries no pattern content at all: not the
/// server's own canonical pattern (never authorized for this account), and
/// not a local `reusable_patterns` row (never authorized for anyone) either
/// (FR-790a).
///
/// **Falsified by** a briefing that carries either pattern's title, or that
/// omits the notice saying fresh knowledge is unavailable this turn.
#[test]
fn an_outage_cache_miss_carries_no_pattern_at_all() {
    let Some(mut server) = server() else { return };
    let d = device(&server, "miss-device");
    let marker = format!("heron-{}", Uuid::now_v7().simple());
    let title = format!("{marker} pattern title");
    let approach = format!("{marker} approach text");
    let _pattern = promote_pattern(
        &server,
        &d.token,
        &title,
        &format!("{marker} problem statement"),
        &approach,
    );
    let bait = format!("{marker}-bait");
    seed_local_pattern(&d.sandbox, &bait);

    let key = format!("miss-{}", Uuid::now_v7());
    let warm = open_session_text(&d, &key);
    // See `seed_matching_local_signals`: without these the local matcher
    // could never have offered the bait, and its absence below would prove
    // nothing about the fallback FR-790a forbids.
    seed_matching_local_signals(&d.sandbox, &bait);
    assert!(
        warm.contains(&title),
        "the authorized answer never carried the canonical pattern, so the \
         miss below would prove nothing: {warm}"
    );
    let session_id = d
        .sandbox
        .query_column("SELECT id FROM sessions ORDER BY started_at DESC LIMIT 1")
        .first()
        .cloned()
        .expect("the session just opened has a local row");

    // A second account signs in while the server is still reachable, so the
    // switch is genuine and not an artifact of the outage below.
    let second = server.new_user_token("miss-second-account");
    let switched = d
        .sandbox
        .cairn(&["auth", "token", "set", &second, "--server", &server.base]);
    assert!(switched.ok(), "auth token set: {}", switched.stderr);
    server.go_offline();

    let after_text = open_session_text(&d, &key);
    assert!(
        !after_text.contains(&title) && !after_text.contains(&approach),
        "a second account's briefing carried the first account's canonical \
         pattern during an outage with nothing cached for it (FR-790a): \
         {after_text}"
    );
    assert!(
        !after_text.contains(&bait),
        "a second account's briefing carried a pattern from this machine's \
         local store on a cache miss — the fallback FR-790a forbids: \
         {after_text}"
    );
    assert!(
        after_text.contains("has no cached briefing"),
        "an outage with no cache entry for this account did not say fresh \
         knowledge is unavailable: {after_text}"
    );

    // The structured answer, not only the rendered text: `briefing.patterns`
    // is absent or empty, and the reply says why.
    let after = d.sandbox.json(&["context", "--session", &session_id]);
    assert_eq!(
        after["fresh_knowledge_unavailable"].as_bool(),
        Some(true),
        "a cache miss with the server unreachable did not report fresh \
         knowledge as unavailable: {after}"
    );
    let patterns_empty = after
        .get("briefing")
        .and_then(|b| b.get("patterns"))
        .and_then(|p| p.as_array())
        .map(|a| a.is_empty())
        .unwrap_or(true);
    assert!(
        patterns_empty,
        "briefing.patterns was neither absent nor empty on a cache miss: {after}"
    );
}

// ---------------------------------------------------------------------------
// 10 — cross-account isolation
// ---------------------------------------------------------------------------

fn retrieve(pg: &Pg, who: &Account, session: Uuid, trigger: &str) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        "/api/retrieve",
        &json!({ "session_id": session, "trigger": trigger }),
        &who.token,
    )
}

/// A canonical pattern owned by one account must not be selected, traced, or
/// delivered for another — not even a co-member of the very project the
/// retrieval is for (SC-761, FR-708d, FR-790a).
///
/// Proved in both directions with two distinct patterns so neither assertion
/// can be passing on the strength of the other account's absence rather than
/// on the boundary actually holding.
///
/// **Falsified by** any trace item, any `delivered_context` row, or any
/// rendered content naming a pattern the retrieving account does not own.
#[test]
fn a_canonical_pattern_owned_by_one_account_is_never_reachable_through_another() {
    let pg = pg!();

    let x = Uuid::now_v7();
    assert!(
        pg.seed_pattern_with_id(&pg.owner, x, "isolation pattern x canonical title"),
        "shared_patterns does not exist at this schema; the fixture is not v4"
    );

    let session_b = pg.session_for(&pg.member);
    let (resp_b, status) = retrieve(&pg, &pg.member, session_b, "session_open");
    assert_eq!(status, 200, "{resp_b}");
    let trace_b = resp_b["trace_id"].as_str().expect("trace_id").to_string();

    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM retrieval_trace_items
              WHERE trace_id = '{trace_b}' AND reference_key = 'pattern:{x}'"
        )),
        0,
        "account B's own retrieval considered or selected account A's \
         pattern — pattern selection is not account-scoped"
    );
    assert!(
        !resp_b
            .to_string()
            .contains("isolation pattern x canonical title"),
        "account B's retrieval response named account A's pattern by title: \
         {resp_b}"
    );

    // Report the transmission as B, exactly as a real daemon would, so
    // `delivered_context` is populated the way it would be in production
    // rather than merely being empty because nobody asked it to be filled.
    let (reported, code) = post_json_status_bearer(
        &pg.server.base,
        &format!("/api/retrieval-traces/{trace_b}/transmission"),
        &json!({ "outcome": "transmitted" }),
        &pg.member.token,
    );
    assert_eq!(code, 200, "{reported}");
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM delivered_context
              WHERE session_id = '{session_b}' AND reference_key = 'pattern:{x}'"
        )),
        0,
        "account B's delivered_context recorded a pattern it was never \
         authorized to see"
    );

    // The reverse direction: a pattern owned by the (former) reader, read by
    // the (former) owner.
    let y = Uuid::now_v7();
    pg.seed_pattern_with_id(&pg.member, y, "isolation pattern y canonical title");
    let session_a = pg.session_for(&pg.owner);
    let (resp_a, status) = retrieve(&pg, &pg.owner, session_a, "session_open");
    assert_eq!(status, 200, "{resp_a}");
    let trace_a = resp_a["trace_id"].as_str().expect("trace_id").to_string();

    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM retrieval_trace_items
              WHERE trace_id = '{trace_a}' AND reference_key = 'pattern:{y}'"
        )),
        0,
        "account A's own retrieval considered or selected account B's pattern"
    );
    assert!(
        !resp_a
            .to_string()
            .contains("isolation pattern y canonical title"),
        "account A's retrieval response named account B's pattern by title: \
         {resp_a}"
    );
}

// ---------------------------------------------------------------------------
// Exact-server-instance authority for the pattern cache
// ---------------------------------------------------------------------------

/// A credential minted by one deployment must not be treated as though a
/// *different* deployment at the same address had authorized anything,
/// merely because the outage cache still holds an answer from before the
/// replacement (FR-791, `contracts/retrieval-delivery.md` §12.3).
///
/// `feature005_spool_instance_binding.rs` and the fourth scenario of
/// `feature005_identity_outage.rs` already prove this for queued events and
/// commands, keyed by `server_instance_id` end to end. This asks the same
/// question of the briefing outage cache, which is a separate mechanism
/// (`crates/cairnd/src/deliver.rs::OutageCache`) with no `server_instance`
/// column at all — see that file's header and `OutageCache::get`, which
/// matches on `(session_id, account_id)` alone, both read from local state a
/// replacement at the same address does not change.
///
/// Deliberately **not** re-authenticated against the replacement: the whole
/// point is what the cache does with a credential the daemon still believes
/// is good, issued by a peer that no longer exists at that address. Every
/// token the predecessor issued dies with it, so any request the daemon
/// makes to the replacement can only ever come back unauthorized — which
/// `retrieve_remote` (deliver.rs) treats identically to "unreachable",
/// exactly the condition that consults the cache.
///
/// **If this is what the outage cache turns out to do, the assertion below
/// is left failing rather than weakened.** It states the property FR-791
/// requires; whether the implementation currently has it is what running
/// this test answers.
#[test]
fn a_replacement_deployment_does_not_inherit_the_predecessors_authorized_pattern_cache() {
    let Some(mut original) = server() else { return };
    let d = device(&original, "instance-device");
    let marker = format!("gazelle-{}", Uuid::now_v7().simple());
    let title = format!("{marker} pattern title");
    let approach = format!("{marker} approach text");
    let _pattern = promote_pattern(
        &original,
        &d.token,
        &title,
        &format!("{marker} problem statement"),
        &approach,
    );

    let key = format!("instance-{}", Uuid::now_v7());
    let warm = open_session_text(&d, &key);
    assert!(
        warm.contains(&title) && warm.contains(&approach),
        "the authorized answer never carried the canonical pattern, so this \
         test proves nothing: {warm}"
    );

    // A different deployment, same address: its own database, its own
    // `server_instance`, and no knowledge whatsoever of this token, this
    // account, this project, or this pattern.
    let replacement = original.replaced_at_same_address();

    let after = open_session_text(&d, &key);
    assert!(
        !after.contains(&title) && !after.contains(&approach),
        "a replacement deployment at the same address was served the \
         predecessor's authorized canonical pattern out of the outage cache. \
         The cache is keyed on (session, account) alone with no \
         server-instance check, so a credential the daemon still holds for a \
         peer that no longer exists is treated as though the *new* \
         deployment had authorized it (FR-791) — see \
         `crates/cairnd/src/deliver.rs`, `OutageCache::get`: {after}"
    );
    assert_eq!(
        replacement.count("SELECT COUNT(*) FROM retrieval_traces"),
        0,
        "the replacement deployment recorded a retrieval it never generated"
    );
}
