//! Deleting the local store loses nothing the server accepted, and says exactly
//! what it did lose (T083, T091; FR-703, FR-704, FR-705, FR-706, FR-710,
//! FR-710a, SC-713, SC-714, SC-738).
//!
//! # Why the records are seeded through the server directly
//!
//! US3's independent test says to, and the reason is the same one US2 gives: if
//! this test needed capture, consolidation or the command drain to work first,
//! a regression in any of them would fail it and nobody would learn anything
//! about durability. What is under test here is narrower and sharper — the
//! server holds four kinds of knowledge, the local store is destroyed, and the
//! knowledge is reachable again without anybody repairing anything.
//!
//! # "Restorable" is not "durable", and the difference is the point
//!
//! Two claims live in this file and they must not be allowed to blur:
//!
//! - Server-accepted knowledge survives, because the rows come back on the next
//!   pull. What survived is the *knowledge*; the local rows did not.
//! - Machine-local records do not survive, at all, ever. Observations, evidence,
//!   verification runs, checkpoints, pattern applications and anything marked
//!   local-only have no server table (FR-707), which is what makes "it stays
//!   local" a fact about the schema rather than a promise — and what makes
//!   losing them permanent.
//!
//! A report that collapsed those into one "safe" would be answering a question
//! nobody asked. So the inventory is asserted category by category, and the test
//! that proves knowledge comes back also proves the local-only categories did
//! not.

use cairn_e2e::feature005::Local;
use cairn_e2e::{attach_server, post_json_status_bearer, Sandbox, Server};
use cairn_store::diag::{self, CacheState, DurabilityClass};
use serde_json::json;
use uuid::Uuid;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
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

// ---------------------------------------------------------------------------
// 1. The inventory — what a store would lose, category by category
// ---------------------------------------------------------------------------

/// The categories a durability report must never stop naming.
///
/// Written out here rather than derived from the inventory itself, because an
/// assertion derived from the thing it checks passes when the thing disappears.
/// A category dropped from `local_inventory` fails this list; a category added
/// to the schema and not to the inventory fails the exhaustiveness check below.
const MUST_BE_NAMED: &[&str] = &[
    // The five FR-705 names outright: spooled-but-unaccepted events,
    // machine-local integration state, cached knowledge, local-only knowledge,
    // and local-only diagnostic records.
    "spooled events",
    "spooled knowledge commands",
    "integration state",
    "observations",
    "evidence facts",
    "verification runs",
    "continuity checkpoints",
    "pattern applications",
    "reusable patterns (local)",
    "task change history",
    "criterion evidence",
    "local-only memory",
    // And the caches, which are the other half of the answer.
    "project memory",
    "personal knowledge",
    "team knowledge",
    "cached patterns",
];

#[test]
fn the_inventory_names_every_category_and_loses_none_silently() {
    rt().block_on(async {
        let db = Local::new().await;
        let inventory = diag::local_inventory(&db.store).await.expect("inventory");
        let named: Vec<&str> = inventory.iter().map(|e| e.category).collect();

        for wanted in MUST_BE_NAMED {
            assert!(
                named.contains(wanted),
                "`{wanted}` is not in the durability inventory. A category the \
                 report does not name is a category lost silently, which is \
                 exactly what SC-714 forbids — the reader takes the omission \
                 for an assurance. Named: {named:?}"
            );
        }
    });
}

/// Every category states a class, and the class decides the promise.
///
/// The falsification this guards: an inventory that reported everything as
/// `Cache` would pass an existence check and be a lie. So the classes are
/// asserted individually, and the local-only ones are asserted to *not* survive.
#[test]
fn the_local_only_categories_are_the_ones_that_do_not_survive() {
    rt().block_on(async {
        let db = Local::new().await;
        let inventory = diag::local_inventory(&db.store).await.expect("inventory");
        let class_of = |name: &str| {
            inventory
                .iter()
                .find(|e| e.category == name)
                .unwrap_or_else(|| panic!("`{name}` is not in the inventory"))
                .class
        };

        for local in [
            "observations",
            "pattern applications",
            "reusable patterns (local)",
            "continuity checkpoints",
            "local-only memory",
            "evidence facts",
            "verification runs",
        ] {
            assert_eq!(
                class_of(local),
                DurabilityClass::LocalOnly,
                "`{local}` has no server table (FR-707), so reporting it as \
                 anything but local-only promises a restoration that cannot happen"
            );
            assert!(
                !class_of(local).survives_local_loss(),
                "`{local}` was reported as surviving a deletion it cannot survive"
            );
        }

        for cache in [
            "project memory",
            "personal knowledge",
            "team knowledge",
            "cached patterns",
        ] {
            assert_eq!(
                class_of(cache),
                DurabilityClass::Cache,
                "`{cache}` holds the server's knowledge and refills from it"
            );
            assert!(class_of(cache).survives_local_loss());
        }

        for queued in ["spooled events", "spooled knowledge commands"] {
            assert_eq!(
                class_of(queued),
                DurabilityClass::QueuedForServer,
                "a queued write is not durable knowledge (FR-709) and not \
                 restorable either — nobody has accepted it yet"
            );
            assert!(
                !class_of(queued).survives_local_loss(),
                "a spool row nobody has accepted is lost with the spool"
            );
        }
    });
}

/// An empty cache and an absence of knowledge look identical. They are not.
///
/// FR-710a exists because of exactly this confusion: a store whose cache has not
/// refilled after local loss must say so rather than present emptiness as an
/// answer. `NeverRefilled` and `Empty` are therefore two states and not one.
#[test]
fn a_cache_that_never_refilled_is_not_the_same_as_one_holding_nothing() {
    rt().block_on(async {
        let db = Local::new().await;
        let instance = Uuid::now_v7();
        let owner = Uuid::now_v7();
        let namespace = cairn_core::domain::SyncNamespace::Personal(instance, owner);

        // No lane at all: nothing has ever been established, so there is nothing
        // to report as fresh or stale.
        let before = diag::cache_status(&db.store).await.expect("status");
        assert!(
            !before.iter().any(|c| c.namespace == namespace.key()),
            "a lane nobody opened was reported on"
        );

        // A lane exists and has never succeeded.
        cairn_store::cursor::establish(&db.store, &namespace)
            .await
            .expect("establish");
        let opened = diag::cache_status(&db.store).await.expect("status");
        let lane = opened
            .iter()
            .find(|c| c.namespace == namespace.key())
            .expect("the established lane is reported");
        assert_eq!(
            lane.state,
            CacheState::NeverRefilled,
            "a lane that has never pulled must say so; reporting it as empty \
             would claim the server holds nothing, which nobody has established"
        );
        assert!(lane.last_refilled_at.is_none());

        // A lane that pulled and found nothing is a different answer.
        cairn_store::cursor::set_pull_cursor(&db.store, &namespace, "cursor-1")
            .await
            .expect("cursor");
        cairn_store::cursor::record_success(&db.store, &namespace)
            .await
            .expect("success");
        let refilled = diag::cache_status(&db.store).await.expect("status");
        let lane = refilled
            .iter()
            .find(|c| c.namespace == namespace.key())
            .expect("lane");
        assert_eq!(
            lane.state,
            CacheState::Empty,
            "a lane that pulled successfully and holds nothing is empty, which \
             is a fact about the server rather than about the cache"
        );
        assert!(lane.last_refilled_at.is_some());
    });
}

// ---------------------------------------------------------------------------
// 2. Restoration — destroy the store, get the knowledge back
// ---------------------------------------------------------------------------

struct Device {
    sandbox: Sandbox,
    project: Uuid,
    account: Uuid,
    token: String,
}

/// A linked, authenticated sandbox with a project on the server.
fn device(server: &Server, label: &str) -> Device {
    let sandbox = Sandbox::new();
    let remote = format!("git@localhost:cairnfixture/{label}.git");
    sandbox.git(&["remote", "add", "origin", &remote]);
    sandbox.must(&["init"]);

    let (account, token) = server.new_user(label);
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
    sandbox.must(&["sync", "now"]);
    Device {
        sandbox,
        project,
        account,
        token,
    }
}

/// One of each of the four kinds of knowledge, written straight into the server.
fn seed_every_domain(server: &Server, d: &Device) {
    let session = Uuid::now_v7();
    server.execute(&format!(
        "INSERT INTO sessions (id, project_id, agent, branch, status, started_at)
         VALUES ('{session}', '{}', 'claude_code', 'main', 'active', now())",
        d.project
    ));
    server.execute(&format!(
        "INSERT INTO memories
            (id, project_id, type, scope, scope_key, content, state, origin_session_id,
             topic_key, value_key, origin_kind)
         VALUES ('{}', '{}', 'decision', 'project', '{}',
                 'the widget pipeline is the settled owner of validation',
                 'active', '{session}', 'decision.widget', 'settled', 'explicit')",
        Uuid::now_v7(),
        d.project,
        d.project
    ));
    server.execute(&format!(
        "INSERT INTO personal_knowledge
            (id, owner_user_id, knowledge_type, content, topic_key, value_key,
             writer_id, writer_seq, created_at)
         VALUES ('{}', '{}', 'convention', 'I keep the lockfile at the workspace root',
                 'convention.lockfile', 'workspace-root', '{}', 1, now())",
        Uuid::now_v7(),
        d.account,
        Uuid::now_v7()
    ));
    server.execute(&format!(
        "INSERT INTO team_knowledge
            (id, knowledge_type, content, topic_key, value_key, state,
             proposed_by_user_id, ratified_by_user_id, ratified_at,
             writer_id, writer_seq, created_at)
         VALUES ('{}', 'convention', 'reviews need one approval before merge',
                 'convention.review', 'one-approval', 'authoritative',
                 '{}', '{}', now(), '{}', 1, now())",
        Uuid::now_v7(),
        d.account,
        d.account,
        Uuid::now_v7()
    ));
}

/// Promote a pattern through the real route, so it is the server's own record.
fn promote_pattern(server: &Server, d: &Device, problem: &str) -> Uuid {
    let (body, status) = post_json_status_bearer(
        &server.base,
        "/api/patterns",
        &json!({
            "title": "a flaky check that is really a clock",
            "problem": problem,
            "root_cause": "the assertion compares wall time across two machines",
            "approach": "compare a monotonic counter and assert an ordering",
            "constraints": ["needs a monotonic source"],
        }),
        &d.token,
    );
    assert_eq!(status, 200, "promote a pattern: {body}");
    body["pattern_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("uuid")
}

/// Local-only records, which exist only here and are the ones that go.
fn seed_local_only(d: &Device) {
    d.sandbox.must(&[
        "memory",
        "add",
        "a note I asked never to leave this machine",
        "--local-only",
    ]);
    // An observation is written by ordinary hook traffic rather than by hand:
    // it is machine-local evidence with no server table, and the point is that
    // real evidence disappears, not that a hand-inserted row does.
    let key = format!("loss-{}", Uuid::now_v7());
    let out = d.sandbox.hook(
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    );
    assert_eq!(out.code, 0, "a hook always exits 0: {}", out.stderr);
    let out = d.sandbox.hook(
        "PostToolUse",
        json!({
            "session_id": key,
            "tool_name": "Edit",
            "tool_input": { "file_path": "src/widget.rs" }
        }),
    );
    assert_eq!(out.code, 0, "a hook always exits 0: {}", out.stderr);
    d.sandbox.settle_observations(1);
}

/// Destroy the local store the way losing a laptop destroys it.
///
/// The daemon is stopped first because it holds the file open, and the
/// configuration is deliberately left alone: this is local *store* loss, not a
/// reinstall. A test that also removed the credentials would be proving that
/// signing in again works, which is a different and much weaker claim.
fn destroy_the_local_store(d: &Device) {
    d.sandbox.stop_daemon();
    let db = d.sandbox.db_path();
    for suffix in ["", "-wal", "-shm"] {
        let path = format!("{}{suffix}", db.display());
        let _ = std::fs::remove_file(&path);
    }
    assert!(
        !db.exists(),
        "the local store is still there; the rest of this test would prove nothing"
    );
}

/// What a new machine has to be told, and what it must not have to repair.
///
/// The project link lives in the store that was just deleted: `projects.server_project_id`
/// is a local row, so a store with no rows is a store that does not know which
/// server project this checkout is. Naming it again is **setup**, the same setup
/// a genuinely new machine needs, and it is deliberately not automatic —
/// `cairn link` with no argument *offers* remote-derived candidates and never
/// applies one silently (FR-064, D14), because linking a checkout to a project
/// nobody chose is worse than asking.
///
/// What FR-704 forbids is a *repair* step: an import, a reconciliation, a merge
/// the user has to resolve. None of that happens here. Everything after this
/// line is synchronization doing what it already does.
///
/// The credential is untouched on purpose: it lives in a token file rather than
/// in the store (`cairn_core::paths::token_path`), so the account-scoped lanes —
/// personal, team and patterns — re-establish with no setup at all.
fn name_the_project_again(sandbox: &cairn_e2e::Sandbox, project: Uuid) {
    sandbox.must(&["init"]);
    sandbox.must(&["link", "--project", &project.to_string()]);
}

const SETTLE: std::time::Duration = std::time::Duration::from_secs(60);

/// Wait for a condition, re-driving synchronization while waiting.
///
/// `Sandbox::settle` allows five seconds and only observes; both are wrong here.
/// A refill after local loss has to establish its lanes first, and the lane a
/// pull needs may not exist on the first attempt — so this drives `sync now`
/// each round rather than watching a background worker's backoff, and gives the
/// whole sequence a minute. A test that waited passively would be measuring the
/// worker's schedule instead of whether the knowledge comes back.
fn settle_syncing(sandbox: &Sandbox, what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + SETTLE;
    while std::time::Instant::now() < deadline {
        if predicate() {
            return;
        }
        let _ = sandbox.cairn(&["sync", "now"]);
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    panic!("timed out waiting for: {what}");
}

fn local_count(d: &Device, sql: &str) -> i64 {
    d.sandbox
        .query_column(sql)
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// The whole story: four domains in, the store deleted, four domains back.
///
/// **Falsified by** removing the pattern lane from `establish_global_lanes`, by
/// reverting `merge_synced_pattern` to insert-once, or by making any of the four
/// restorations depend on a command the user has to run. The last one is what
/// FR-704 is about: the only thing done between destroying the store and
/// counting the rows is a sync, which the daemon does on its own — no repair, no
/// reconciliation, no import.
#[test]
fn server_accepted_knowledge_survives_deleting_the_local_store() {
    let Some(server) = server() else { return };
    let d = device(&server, "loss");

    seed_every_domain(&server, &d);
    let pattern = promote_pattern(&server, &d, "a check that passes alone and fails in CI");
    seed_local_only(&d);

    d.sandbox.must(&["sync", "now"]);
    settle_syncing(&d.sandbox, "the knowledge reaches this store", || {
        local_count(
            &d,
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE local_only = 0",
        ) > 0
            && local_count(&d, "SELECT CAST(COUNT(*) AS TEXT) FROM personal_knowledge") > 0
            && local_count(&d, "SELECT CAST(COUNT(*) AS TEXT) FROM team_knowledge") > 0
            && local_count(&d, "SELECT CAST(COUNT(*) AS TEXT) FROM cached_patterns") > 0
    });

    let observations_before = local_count(&d, "SELECT CAST(COUNT(*) AS TEXT) FROM observations");
    assert!(
        observations_before > 0,
        "no observation was recorded, so the local-only half of this test would \
         pass vacuously"
    );
    assert_eq!(
        local_count(
            &d,
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE local_only = 1"
        ),
        1,
        "the local-only memory was not written"
    );

    destroy_the_local_store(&d);

    // Setup, not repair — see `name_the_project_again`. The store itself is
    // recreated by migration on the daemon's next start, and the lanes refill
    // themselves.
    name_the_project_again(&d.sandbox, d.project);
    d.sandbox.must(&["sync", "now"]);
    settle_syncing(&d.sandbox, "every domain refills from the server", || {
        local_count(
            &d,
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE local_only = 0",
        ) > 0
            && local_count(&d, "SELECT CAST(COUNT(*) AS TEXT) FROM personal_knowledge") > 0
            && local_count(&d, "SELECT CAST(COUNT(*) AS TEXT) FROM team_knowledge") > 0
            && local_count(&d, "SELECT CAST(COUNT(*) AS TEXT) FROM cached_patterns") > 0
    });

    assert_eq!(
        local_count(
            &d,
            &format!(
                "SELECT CAST(COUNT(*) AS TEXT) FROM cached_patterns WHERE pattern_id = '{pattern}'"
            )
        ),
        1,
        "the promoted pattern did not come back, so FR-708's whole reason for \
         existing — closing the gap Feature 004 left — is not closed (SC-738)"
    );

    // And the other half, which matters just as much: what was local-only is
    // gone, and gone for good.
    assert_eq!(
        local_count(&d, "SELECT CAST(COUNT(*) AS TEXT) FROM observations"),
        0,
        "an observation survived a deletion it has no server table to survive"
    );
    assert_eq!(
        local_count(
            &d,
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE local_only = 1"
        ),
        0,
        "a local-only memory came back, which would mean it had left the machine"
    );
}

/// The inventory, read through the command a user would actually run.
///
/// The store-level tests above assert the classes; this asserts that a person
/// can find them out, and that the answer names the lost categories rather than
/// summarising them away (SC-714).
#[test]
fn the_durability_report_names_what_was_lost_and_what_was_not() {
    let Some(server) = server() else { return };
    let d = device(&server, "inventory");
    seed_every_domain(&server, &d);
    seed_local_only(&d);
    d.sandbox.must(&["sync", "now"]);

    let report = d.sandbox.json(&["doctor", "--durability"]);

    let names = |key: &str| -> Vec<String> {
        report[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|e| e["category"].as_str().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    let lost = names("lost_on_deletion");
    let restorable = names("restorable_from_server");

    for category in [
        "observations",
        "local-only memory",
        "reusable patterns (local)",
    ] {
        assert!(
            lost.contains(&category.to_string()),
            "`{category}` is machine-local and must be reported as lost. Got: {lost:?}"
        );
        assert!(
            !restorable.contains(&category.to_string()),
            "`{category}` was reported as restorable from a server that has no \
             table for it"
        );
    }
    for category in ["project memory", "personal knowledge", "team knowledge"] {
        assert!(
            restorable.contains(&category.to_string()),
            "`{category}` refills from the server. Got: {restorable:?}"
        );
    }

    assert!(
        report["caches"].as_array().map(|a| !a.is_empty()) == Some(true),
        "no cache was reported, so a store whose cache has not refilled has \
         nothing to say about it (FR-710a)"
    );
}

/// Choosing local-only says what it costs, where the choice is made.
///
/// FR-706 puts this at the point of choosing, not in a manual. Asserted on the
/// reply to the write itself, because that is what both a human and an agent
/// actually see.
#[test]
fn marking_a_memory_local_only_states_the_consequence_at_the_point_of_choosing() {
    rt().block_on(async {
        let db = Local::new().await;
        // The warning is for the end state. Before cutover every memory is local
        // and the note would be a statement about the installation rather than
        // about this write, which is why the handler withholds it — asserted
        // here so the withholding is deliberate rather than an omission.
        assert_eq!(
            cairn_store::authority::mode(&db.store).await.unwrap(),
            cairn_store::authority::AuthorityMode::Feature004
        );
    });

    let Some(server) = server() else { return };
    let d = device(&server, "localonly");
    // **Set in the store rather than through a command, deliberately.** The
    // cutover that moves a real installation into this mode is US7's (T138
    // onward) and has not shipped; inventing a flag for it here would be
    // inventing US7's interface in a US3 test. What US3 owns is the behaviour
    // once the mode is set, and this is the smallest way to reach it.
    d.sandbox.stop_daemon();
    d.sandbox
        .execute_sql("UPDATE authority_mode SET mode = 'server_authoritative' WHERE id = 1");
    d.sandbox.must(&["daemon", "start"]);

    let out = d
        .sandbox
        .json(&["memory", "add", "a private note", "--local-only"]);
    let durability = &out["durability"];
    assert_eq!(
        durability["survives_local_loss"],
        json!(false),
        "a local-only write must say it does not survive: {out}"
    );
    let note = durability["note"].as_str().unwrap_or_default();
    assert!(
        note.contains("deleting this store deletes it"),
        "the note must state the consequence in the reply, not gesture at it: {note:?}"
    );

    // And the ordinary write says nothing, because a warning on every write is
    // noise, and noise is how a warning stops being read.
    let ordinary = d.sandbox.json(&["memory", "add", "a shareable note"]);
    assert!(
        ordinary.get("durability").is_none(),
        "an ordinary write carried the local-only warning: {ordinary}"
    );
}
