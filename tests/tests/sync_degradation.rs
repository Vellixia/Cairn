//! T107, T113, T137 — work an older server cannot hold is retained, not lost
//! (FR-415, FR-418, SC-326, SC-331).
//!
//! The two negatives come first and on their own, because they are what makes
//! `blocked` a recoverable state rather than a decorative one. A test that only
//! asserted "it eventually delivers" would pass against an implementation that
//! retried the refused row on every tick forever, and against one that marked
//! it `failed` and then re-queued a *new* row on upgrade — which is a second
//! delivery, not the retained one.

use cairn_core::domain::{OutboxEntityType, OutboxOperation};
use cairn_core::wire::codes;
use cairn_e2e::{attach_server, Sandbox, Server};
use cairn_store::outbox::{self, SyncPolicy};
use cairn_store::Store;
use serde_json::json;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// T107 — the two negatives, against the state machine itself
// ---------------------------------------------------------------------------

struct Fixture {
    store: Store,
    project: Uuid,
    _dir: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(&dir.path().join("cairn.sqlite3"))
        .await
        .expect("store");
    let project = Uuid::now_v7();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO projects (id, name, git_common_dir, repository_remote, linked,
                               server_project_id, created_at, updated_at, deleted_at)
         VALUES (?1, 'degradation', ?2, NULL, 1, ?1, ?3, ?3, NULL)",
    )
    .bind(project.to_string())
    .bind(format!("/fixture/{project}/.git"))
    .bind(&now)
    .execute(store.pool())
    .await
    .expect("project");
    Fixture {
        store,
        project,
        _dir: dir,
    }
}

impl Fixture {
    fn policy(&self) -> SyncPolicy {
        SyncPolicy {
            linked: true,
            server_project_id: Some(self.project),
        }
    }

    /// Queue one item and hand back its outbox row id.
    async fn queue(&self, entity_type: OutboxEntityType, marker: &str) -> Uuid {
        let mut tx = cairn_store::tx::begin(&self.store, "queue")
            .await
            .expect("tx");
        outbox::enqueue(
            &mut *tx,
            self.policy(),
            self.project,
            entity_type,
            Uuid::now_v7(),
            OutboxOperation::Upsert,
            &json!({ "marker": marker }),
        )
        .await
        .expect("enqueue");
        cairn_store::tx::commit(tx, "queue").await.expect("commit");

        let id: String = sqlx::query_scalar(
            "SELECT id FROM outbox WHERE project_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(self.project.to_string())
        .fetch_one(self.store.pool())
        .await
        .expect("the row just queued");
        Uuid::parse_str(&id).expect("id")
    }

    async fn column(&self, id: Uuid, column: &str) -> String {
        let sql = format!("SELECT COALESCE(CAST({column} AS TEXT), '') FROM outbox WHERE id = ?1");
        sqlx::query_scalar(&sql)
            .bind(id.to_string())
            .fetch_one(self.store.pool())
            .await
            .expect("column")
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// A capability-refused row is retried **zero** times while the server is known
/// to lack the capability (FR-418).
///
/// Not "retried less often" and not "retried with backoff": a server that has
/// no table for this work will refuse it identically every time, so every
/// attempt is a request that cannot succeed and a drain slot that could have
/// carried something that can.
#[test]
fn no_futile_retry() {
    runtime().block_on(async {
        let f = fixture().await;
        let row = f.queue(OutboxEntityType::MemoryRelation, "relation").await;

        // One claim, one refusal — the sequence a real drain produces.
        let claimed = outbox::claim(&f.store, f.project, 100)
            .await
            .expect("claim");
        assert_eq!(
            claimed.len(),
            1,
            "the row should be claimable before it is refused"
        );
        outbox::mark_blocked(
            &f.store,
            row,
            codes::UNKNOWN_ENTITY_TYPE,
            "schema=1;capabilities=",
            "no memory_relations table at schema 1",
        )
        .await
        .expect("block");

        // Every subsequent drain must pass it over.
        for round in 1..=5 {
            let claimed = outbox::claim(&f.store, f.project, 100)
                .await
                .expect("claim");
            assert!(
                claimed.is_empty(),
                "drain {round} claimed a row the server cannot hold: {claimed:?}"
            );
        }
        assert_eq!(
            f.column(row, "attempts").await,
            "0",
            "a row that was never deliverable must not carry delivery attempts"
        );
        assert_eq!(f.column(row, "state").await, "blocked");
        assert_eq!(
            f.column(row, "blocked_reason").await,
            codes::UNKNOWN_ENTITY_TYPE
        );
        assert_eq!(
            f.column(row, "blocked_at_capability").await,
            "schema=1;capabilities=",
            "a person must be able to see what the row is waiting for"
        );
    });
}

/// A capability-refused row is never `failed` and never reported `delivered`
/// (FR-418, SC-326).
///
/// The first would tell the user work was lost that is in fact waiting; the
/// second would tell them it arrived. Both are worse than saying nothing.
#[test]
fn never_permanently_failed() {
    runtime().block_on(async {
        let f = fixture().await;
        let row = f.queue(OutboxEntityType::TaskCriterion, "criterion").await;
        outbox::mark_blocked(
            &f.store,
            row,
            codes::SCHEMA_OLDER,
            "schema=1;capabilities=",
            "this deployment is at schema 1",
        )
        .await
        .expect("block");

        let (pending, failed) = outbox::counts(&f.store, f.project).await.expect("counts");
        assert_eq!(failed, 0, "retained work must not be reported as failed");
        assert_eq!(
            pending, 0,
            "retained work must not be reported as pending either — the queue is \
             not stuck, and saying it is would send someone looking for a fault"
        );
        assert_eq!(
            outbox::blocked_count(&f.store, f.project)
                .await
                .expect("blocked"),
            1
        );
        assert!(
            outbox::failures(&f.store, f.project)
                .await
                .expect("failures")
                .is_empty(),
            "a blocked row must not appear among permanent failures"
        );

        // And it is not delivered. `delivered_at` is what a delivery stamps.
        assert_eq!(f.column(row, "delivered_at").await, "");
    });
}

/// The upgrade returns the retained work with its **original** identity.
///
/// A new idempotency key would make the delivery a second one rather than the
/// one that was waiting, and the server's exactly-once guarantee rests on that
/// key being the same.
#[test]
fn release_preserves_identity() {
    runtime().block_on(async {
        let f = fixture().await;
        let row = f.queue(OutboxEntityType::MemoryRelation, "relation").await;
        let key_before = f.column(row, "idempotency_key").await;
        let payload_before = f.column(row, "payload").await;
        outbox::mark_blocked(
            &f.store,
            row,
            codes::UNKNOWN_ENTITY_TYPE,
            "schema=1;capabilities=",
            "no memory_relations table at schema 1",
        )
        .await
        .expect("block");

        let released =
            // By namespace, which is what production does since Feature 004 —
            // a project's namespace is `project:<id>`, so this is the same set
            // of rows the daemon's own release reaches.
            outbox::release_blocked_namespace(
                &f.store,
                &cairn_core::domain::SyncNamespace::Project(f.project).key(),
                &[OutboxEntityType::MemoryRelation],
            )
                .await
                .expect("release");
        assert_eq!(released, 1);

        assert_eq!(f.column(row, "state").await, "pending");
        assert_eq!(f.column(row, "idempotency_key").await, key_before);
        assert_eq!(f.column(row, "payload").await, payload_before);
        assert_eq!(
            f.column(row, "blocked_reason").await,
            "",
            "a released row carries no stale refusal"
        );
        assert_eq!(
            outbox::claim(&f.store, f.project, 100)
                .await
                .expect("claim")
                .len(),
            1,
            "the released row must be delivered by the ordinary drain"
        );
    });
}

/// A capability the upgrade did **not** bring leaves its rows blocked.
///
/// Releasing everything on any capability change would put work back in front
/// of a server that still cannot hold it, and the futile retry this state
/// exists to prevent would happen anyway.
#[test]
fn a_partial_upgrade_releases_only_what_it_covers() {
    runtime().block_on(async {
        let f = fixture().await;
        let relation = f.queue(OutboxEntityType::MemoryRelation, "relation").await;
        let blocker = f.queue(OutboxEntityType::TaskBlocker, "blocker").await;
        for row in [relation, blocker] {
            outbox::mark_blocked(
                &f.store,
                row,
                codes::UNKNOWN_ENTITY_TYPE,
                "schema=1;capabilities=",
                "no table at schema 1",
            )
            .await
            .expect("block");
        }

        let released =
            // By namespace, which is what production does since Feature 004 —
            // a project's namespace is `project:<id>`, so this is the same set
            // of rows the daemon's own release reaches.
            outbox::release_blocked_namespace(
                &f.store,
                &cairn_core::domain::SyncNamespace::Project(f.project).key(),
                &[OutboxEntityType::MemoryRelation],
            )
                .await
                .expect("release");
        assert_eq!(released, 1);
        assert_eq!(f.column(relation, "state").await, "pending");
        assert_eq!(f.column(blocker, "state").await, "blocked");
        assert_eq!(
            f.column(blocker, "attempts").await,
            "0",
            "the still-unsupported row must not have been retried by the release"
        );
    });
}

// ---------------------------------------------------------------------------
// T113 — end to end, against a real schema-1 server and then a schema-2 one
// ---------------------------------------------------------------------------

/// The heavyweight tests run one at a time.
///
/// Each spins **two** servers against a database of its own — a schema-1 one
/// and the upgraded one that replaces it — on top of every other server the
/// suite is already running. PostgreSQL's connection limit is a fixed resource
/// shared by the whole run, and exhausting it makes a server exit at startup:
/// the failure lands in whichever test happened to ask next, and says nothing
/// about the code it was testing.
///
/// The lock is taken rather than the pool made smaller because these tests are
/// about a server's behaviour, and a server starved of connections is not the
/// server under test.
fn heavy() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // A test that panicked while holding it poisoned nothing that matters here:
    // the guard protects a connection budget, not shared state.
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn schema_1_server() -> Option<Server> {
    match Server::start_at_schema(1) {
        Some(s) => Some(s),
        None => {
            eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the server suite");
            None
        }
    }
}

/// The whole story: refuse, retain, upgrade, deliver exactly once (SC-326,
/// SC-331).
#[test]
fn recovers_after_upgrade() {
    let _serialized = heavy();
    let Some(mut old) = schema_1_server() else {
        return;
    };
    let token = old.new_user_token("degradation");

    let s = Sandbox::new();
    attach_server(&s, &old, &token);
    s.must(&["init"]);
    let project_id = s.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .expect("a shared project")
        .to_string();

    // Work a schema-1 server can hold: a Feature 001 memory, which carries
    // every Feature 003 field at its default and therefore says nothing this
    // server cannot store. SC-326 is about **this** continuing to arrive.
    s.must(&[
        "memory",
        "add",
        "The pool is single-writer on purpose",
        "--type",
        "decision",
        "--scope",
        "project",
    ]);

    // And work it cannot: two subject-carrying proposals and the decision
    // between them.
    let old_id = memory(&s, "api.port", "8080", "The API listens on port 8080");
    let new_id = memory(&s, "api.port", "9000", "The API listens on port 9000");
    let decided = s.json(&[
        "memory",
        "reconcile",
        "--from",
        &new_id,
        "--to",
        &old_id,
        "--relation",
        "supersedes",
        "--basis",
        "explicit_user",
    ]);
    assert!(decided.get("error").is_none(), "{decided}");

    s.json(&["sync", "now"]);

    // The refused work is retained and named, and nothing is failed.
    let status = s.json(&["sync", "status"]);
    let degradation = &status["degradation"];
    assert!(
        degradation
            .get("blocked")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            > 0,
        "a schema-1 server must retain the work it cannot hold: {status}"
    );
    assert_eq!(
        status["failed"], 0,
        "retained work must not be failed: {status}"
    );
    let note = degradation["note"].as_str().unwrap_or_default();
    assert!(
        note.contains("automatically"),
        "the report must say the work will be delivered without intervention: {note}"
    );

    // And doctor says so too, without the developer having to ask sync.
    let doctor = s.cairn(&["--json", "doctor"]);
    assert!(
        doctor.stdout.contains("sync_degradation"),
        "doctor must report the degradation: {}",
        doctor.stdout
    );

    // 100% of the Feature 001 payload arrived throughout (SC-326). Degradation
    // that stopped ordinary sync would be an outage dressed as a compatibility
    // feature.
    let dump = old.dump();
    assert!(
        dump.contains("single-writer on purpose"),
        "ordinary work must keep syncing while some work is retained.\n\
         status: {status}\n\
         blocked rows: {:?}\n\
         dump: {dump}",
        s.query_column(
            "SELECT entity_type || ' ' || state || ' ' || COALESCE(blocked_reason, '-') \
             || ' ' || COALESCE(last_error, '-') FROM outbox"
        )
    );
    assert!(
        !dump.contains("port 9000"),
        "a subject-carrying memory must be retained, not stored with its \
         identity silently dropped"
    );
    assert_eq!(
        old.count(
            "SELECT COUNT(*) FROM information_schema.tables
              WHERE table_schema = 'public' AND table_name = 'memory_relations'"
        ),
        0,
        "a schema-1 server has no relations table at all"
    );
    // --- The upgrade. The same database, a server that applies migration 2.
    let new = old.upgraded();
    drop(old);
    // The sandbox points at the old address; re-point it at the new one.
    attach_server(&s, &new, &token);
    assert_eq!(
        s.json(&["link"])["server_project_id"].as_str(),
        Some(project_id.as_str()),
        "the upgrade must not change which shared project this is"
    );

    s.json(&["sync", "now"]);
    s.json(&["sync", "now"]);

    let status = s.json(&["sync", "status"]);
    assert_eq!(
        status["degradation"],
        serde_json::Value::Null,
        "after the upgrade nothing should still be retained: {status}"
    );
    assert_eq!(status["failed"], 0, "nothing was lost: {status}");
    assert_eq!(status["pending"], 0, "everything drained: {status}");

    // Delivered **exactly once**. The relation's primary key is
    // `(from, to, kind)`, so a second delivery would not add a row — which is
    // why this also asserts the local and remote sets are the same size, where
    // a duplicate would have shown up as a `sync_state` key applied twice.
    assert_eq!(
        new.count("SELECT COUNT(*) FROM memory_relations WHERE kind = 'supersedes'"),
        1,
        "the retained decision must be delivered exactly once"
    );
    let local_relations = s.query_column(
        "SELECT CAST(COUNT(*) AS TEXT) FROM memory_relations WHERE deleted_at IS NULL",
    );
    assert_eq!(
        new.count("SELECT COUNT(*) FROM memory_relations")
            .to_string(),
        local_relations[0],
        "every decision this machine holds, and no more, reached the server"
    );
    assert_eq!(
        new.count(
            "SELECT COUNT(*) FROM (
                 SELECT entity_id FROM sync_state
                  WHERE entity_type = 'memory_relation'
                  GROUP BY entity_id HAVING COUNT(*) > 1
             ) duplicated"
        ),
        0,
        "a released decision arrived under a second idempotency key, which \
         makes it a new delivery rather than the retained one"
    );

    // The subject-carrying memories landed too, with their identity intact.
    assert_eq!(
        new.count("SELECT COUNT(*) FROM memories WHERE topic_key = 'api.port'"),
        2,
        "the retained proposals arrived with their subject identity"
    );

    // No manual repair happened: the only commands run were `sync now`.
    let delivered =
        s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM outbox WHERE state = 'blocked'");
    assert_eq!(delivered, vec!["0".to_string()], "nothing is still blocked");
}

/// The reverse direction needs nothing (T137, FR-415).
///
/// A daemon that sends no Feature 003 field works against a schema-2 server,
/// and arrays it does not understand in the read-back are ignored rather than
/// treated as an error.
#[test]
fn older_daemon_newer_server() {
    let _serialized = heavy();
    let Some(server) = Server::start() else {
        eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the server suite");
        return;
    };
    let token = server.new_user_token("reverse");
    let s = Sandbox::new();
    attach_server(&s, &server, &token);
    s.must(&["init"]);
    s.json(&["link", "--create"]);

    // A Feature 001 memory: no topic key, no value key, no verification.
    s.must(&[
        "memory",
        "add",
        "The pool is single-writer on purpose",
        "--type",
        "decision",
        "--scope",
        "project",
    ]);
    s.json(&["sync", "now"]);

    let status = s.json(&["sync", "status"]);
    assert_eq!(
        status["pending"], 0,
        "an older payload must deliver: {status}"
    );
    assert_eq!(status["failed"], 0, "{status}");
    assert_eq!(
        status["degradation"],
        serde_json::Value::Null,
        "nothing is degraded when the server is the newer side: {status}"
    );

    // The read-back carries arrays this payload never produced. Pulling them is
    // a no-op rather than a failure.
    s.json(&["sync", "now"]);
    assert_eq!(s.json(&["sync", "status"])["failed"], 0);
}

fn memory(s: &Sandbox, topic: &str, value: &str, content: &str) -> String {
    let v = s.json(&[
        "memory",
        "add",
        content,
        "--type",
        "decision",
        "--scope",
        "project",
        "--topic-key",
        topic,
        "--value-key",
        value,
    ]);
    v["memory"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a memory id: {v}"))
        .to_string()
}

/// The upgrade is noticed **without anyone running a command** (FR-418).
///
/// `recovers_after_upgrade` calls `cairn sync now` after the
/// upgrade, which proves the explicit path and routes around the promise the
/// product actually makes: `sync status` tells the user the retained work "is
/// delivered automatically once the server is upgraded".
///
/// It very nearly was not. The background worker skips a project whose
/// `pending` count is zero, and retained work is deliberately not counted as
/// pending — so a project holding only retained work never entered a drain
/// again, never probed, and would have waited forever while telling the user it
/// was waiting for nothing.
#[test]
fn the_background_worker_delivers_retained_work_after_an_upgrade() {
    let _serialized = heavy();
    let Some(mut old) = schema_1_server() else {
        return;
    };
    let token = old.new_user_token("background-upgrade");

    let s = Sandbox::new();
    attach_server(&s, &old, &token);
    s.must(&["init"]);
    s.json(&["link", "--create"]);

    let a = memory(&s, "cache.backend", "redis", "The cache runs on Redis");
    let b = memory(
        &s,
        "cache.backend",
        "memcached",
        "The cache runs on memcached",
    );
    s.json(&[
        "memory",
        "reconcile",
        "--from",
        &b,
        "--to",
        &a,
        "--relation",
        "supersedes",
        "--basis",
        "explicit_user",
    ]);
    s.json(&["sync", "now"]);

    let blocked = |s: &Sandbox| {
        s.json(&["sync", "status"])["degradation"]["blocked"]
            .as_i64()
            .unwrap_or(0)
    };
    assert!(
        blocked(&s) > 0,
        "the schema-1 server should have retained something to deliver later"
    );

    // The upgrade. Nothing else happens here: no command is run against Cairn
    // from this point on except the read the assertion needs.
    let new = old.upgraded();
    drop(old);
    attach_server(&s, &new, &token);

    // Released is not delivered, and the difference is a whole claim timeout.
    //
    // `drop(old)` kills the server the worker is still pointed at, so a drain
    // that has already claimed its rows fails with them `in_flight`. Those rows
    // are neither `blocked` nor `failed`; `outbox::counts` correctly reports
    // them as pending, and they become claimable again only after
    // `CLAIM_TIMEOUT_SECONDS`. Waiting on `blocked == 0` alone therefore
    // returns while the work is still in the queue, and the assertion below
    // reads a server that has not received it yet — which is what failed on CI
    // under load while passing everywhere quieter.
    //
    // So wait for the queue itself to empty, and give it longer than the claim
    // timeout it may have to sit out.
    let queued = |s: &Sandbox| s.json(&["sync", "status"])["pending"].as_i64().unwrap_or(0);
    s.settle_within(
        "the background worker to notice the upgrade and deliver the retained work",
        std::time::Duration::from_secs(120),
        |s| blocked(s) == 0 && queued(s) == 0,
    );

    let status = s.json(&["sync", "status"]);
    assert_eq!(
        status["degradation"],
        serde_json::Value::Null,
        "nothing should still be retained: {status}"
    );
    assert_eq!(status["failed"], 0, "nothing was lost: {status}");
    assert_eq!(
        new.count("SELECT COUNT(*) FROM memory_relations WHERE kind = 'supersedes'"),
        1,
        "the retained decision must have been delivered exactly once"
    );
}
