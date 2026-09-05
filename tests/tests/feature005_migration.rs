//! Migrating a populated Feature 004 store (T138; SC-719–SC-723).
//!
//! Every test here starts from a **real** v7 installation — the shipped
//! migrations run to v7, then populated, then reopened by the current build —
//! because `migration-cutover.md` §11 requires the proof to be against the
//! schema users have rather than against a clean database.

use cairn_e2e::feature005::{install_legacy_v7, LegacyIds, LOCAL_SCHEMA_V10};
use cairn_e2e::{attach_server, Sandbox, Server};

/// A populated v7 store, upgraded in place, with a server attached.
struct Migrating {
    s: Sandbox,
    server: Server,
    ids: LegacyIds,
    /// The account the store migrates under. Every server-side count is scoped
    /// to it, because one PostgreSQL database serves the whole suite and a
    /// bare `count(*)` would be measuring the other tests too.
    account: uuid::Uuid,
    token: String,
}

fn start() -> Option<Migrating> {
    let server = Server::start()?;
    let s = Sandbox::new();
    // The store's global rows are owned by the account that will migrate it,
    // which is what a Feature 004 store in use actually looks like.
    let (account, token) = server.new_user("migrating");
    let ids = install_legacy_v7(&s, account);
    attach_server(&s, &server, &token);
    // A project memory drains only from a linked project: the server needs the
    // id it knows the project by, and an unlinked project has none.
    let linked = s.cairn(&["link", "--create"]);
    assert!(
        linked.ok(),
        "linking the fixture project: {}",
        linked.stderr
    );
    Some(Migrating {
        s,
        server,
        ids,
        account,
        token,
    })
}

macro_rules! migrating {
    () => {
        match start() {
            Some(m) => m,
            None => {
                eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

/// The fixture is what it claims to be: a real v7 store, upgraded, with every
/// legacy row still in it.
///
/// Not a migration test in itself — a guard on the thing every other test in
/// this file stands on. If the upgrade silently dropped the legacy corpus, the
/// migration assertions would all pass against an empty store.
#[test]
fn a_populated_feature_004_store_survives_being_reopened_by_this_build() {
    let m = migrating!();
    let ids = &m.ids;

    assert_eq!(
        m.s.query_column("SELECT CAST(MAX(version) AS TEXT) FROM schema_migrations"),
        vec![LOCAL_SCHEMA_V10.to_string()],
        "the daemon did not migrate the swapped-in v7 store to the current schema"
    );
    assert_eq!(
        m.s.query_column("SELECT mode FROM authority_mode WHERE id = 1"),
        vec!["feature_004".to_string()],
        "a store that has not migrated is `feature_004`, whatever schema it is on"
    );

    for (what, sql) in [
        (
            "project memories",
            "SELECT CAST(count(*) AS TEXT) FROM memories",
        ),
        (
            "the relation",
            "SELECT CAST(count(*) AS TEXT) FROM memory_relations",
        ),
        (
            "personal knowledge",
            "SELECT CAST(count(*) AS TEXT) FROM personal_knowledge",
        ),
        (
            "team knowledge",
            "SELECT CAST(count(*) AS TEXT) FROM team_knowledge",
        ),
        (
            "reusable patterns",
            "SELECT CAST(count(*) AS TEXT) FROM reusable_patterns",
        ),
        (
            "the local pattern evidence",
            "SELECT CAST(count(*) AS TEXT) FROM pattern_applications",
        ),
        (
            "queued outbox rows",
            "SELECT CAST(count(*) AS TEXT) FROM outbox",
        ),
    ] {
        let n: i64 = m.s.query_column(sql)[0].parse().expect("a count");
        assert!(n > 0, "the upgrade lost {what}");
    }

    // The named rows specifically, since the tests that follow address them.
    for id in [
        ids.memory_queued,
        ids.memory_local_only,
        ids.memory_collides,
        ids.personal_queued,
        ids.team_authoritative,
        ids.pattern_claimable,
        ids.pattern_unclaimed,
    ] {
        let found = m.s.query_column(&format!(
            "SELECT '{id}' WHERE EXISTS (SELECT 1 FROM memories WHERE id = '{id}')
                OR EXISTS (SELECT 1 FROM personal_knowledge WHERE id = '{id}')
                OR EXISTS (SELECT 1 FROM team_knowledge WHERE id = '{id}')
                OR EXISTS (SELECT 1 FROM reusable_patterns WHERE id = '{id}')"
        ));
        assert_eq!(found.len(), 1, "{id} did not survive the upgrade");
    }

    assert!(!m.token.is_empty());
}

/// `--inspect` counts and reports, and changes nothing else (§4.1).
///
/// The one write it is allowed is its own `migration_state` row and the move to
/// `migrating`. Everything else — content, keys, the queue, the patterns — is
/// exactly as it was, because a user runs this to decide *whether* to migrate
/// and an inspect that quietly started one would answer a question nobody
/// asked.
///
/// **Falsified by** inspecting through a path that normalizes keys, drains, or
/// claims a pattern on the user's behalf.
#[test]
fn inspect_counts_what_is_there_and_changes_nothing_else() {
    let m = migrating!();
    let ids = &m.ids;

    let before = m.s.query_column(
        "SELECT id || '|' || COALESCE(topic_key,'') || '|' || COALESCE(value_key,'')
           FROM memories ORDER BY id",
    );
    let patterns_before =
        m.s.query_column("SELECT id FROM reusable_patterns ORDER BY id");

    let v = m.s.json(&["migrate", "--inspect"]);
    let i = &v["inspect"];

    assert_eq!(
        i["local_only_memories"].as_i64(),
        Some(1),
        "the local-only memory was not counted: {i}"
    );
    assert!(
        i["records"]["memory:active"].as_i64().unwrap_or(0) >= 3,
        "the project memories were not counted: {i}"
    );
    // At least the four the fixture queued. Linking the project queues more of
    // its own, and pinning an exact total here would make this a test of how
    // many rows `cairn link` writes.
    assert!(
        i["outbox"]["pending"].as_i64().unwrap_or(0) >= 4,
        "the queued rows were not counted by state: {i}"
    );
    assert_eq!(
        i["outbox"]["delivered"].as_i64(),
        Some(1),
        "an already-delivered row is still a row, and inspect reports it: {i}"
    );
    assert!(
        i["outbox_without_author"].as_i64().unwrap_or(0) >= 2,
        "a project row carries no author, which is what the v7 CHECK requires \
         of it, and inspect says so rather than hiding it: {i}"
    );

    let eligible: Vec<String> = i["patterns_eligible_for_claim"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(eligible.len(), 2, "both legacy patterns are claimable: {i}");
    assert!(
        eligible.contains(&ids.pattern_claimable.to_string())
            && eligible.contains(&ids.pattern_unclaimed.to_string()),
        "inspect named patterns other than the two in the store: {eligible:?}"
    );

    // Nothing moved.
    assert_eq!(
        m.s.query_column(
            "SELECT id || '|' || COALESCE(topic_key,'') || '|' || COALESCE(value_key,'')
               FROM memories ORDER BY id"
        ),
        before,
        "inspect rewrote a key. It is a read, and re-keying belongs to the drain \
         phase where the record is about to move"
    );
    assert_eq!(
        m.s.query_column("SELECT id FROM reusable_patterns ORDER BY id"),
        patterns_before,
        "inspect changed the pattern table"
    );
    assert_eq!(
        m.s.query_column("SELECT CAST(count(*) AS TEXT) FROM legacy_pattern_claims"),
        vec!["0".to_string()],
        "inspect claimed a pattern. Ownership is never inferred, and running a \
         read is not consent to assign it"
    );
    assert_eq!(
        m.s.query_column("SELECT mode FROM authority_mode WHERE id = 1"),
        vec!["migrating".to_string()],
        "inspect is phase 1 and its postcondition is `migrating`"
    );
}

/// The whole path, on a populated store: what moves, what is confirmed, what is
/// named as an exception, and what is demoted only after all three.
///
/// # What this is really asserting
///
/// The phase order is the safety property, and this reads it off the recorded
/// state rather than from the code: `demote` finishes last, it never finishes
/// before `verify_possession`, and a record the server does not hold is in
/// `retained_local` rather than demoted.
///
/// **Falsified by** demoting anything the possession check did not confirm, by
/// reordering the phases, or by dropping a drained shape.
#[test]
fn a_populated_store_migrates_with_every_shape_accounted_for() {
    let m = migrating!();
    let ids = &m.ids;

    // Claim one pattern; leave the other deliberately unclaimed.
    let claims = m.s.json(&[
        "migrate",
        "--claim-patterns",
        &ids.pattern_claimable.to_string(),
    ]);
    assert_eq!(
        claims["claims"][0]["outcome"], "claimed",
        "the first claim of an owner-less legacy pattern: {claims}"
    );

    let v = m.s.json(&["migrate", "--run"]);
    let run = &v["run"];
    assert_eq!(
        run["mode"], "server_authoritative",
        "the store did not reach server authority: {run}"
    );
    assert!(
        run["drain"]["delivered"].as_i64().unwrap_or(0) >= 5,
        "not every drainable shape was delivered: {run}"
    );
    assert!(
        run["possession"]["held"].as_i64().unwrap_or(0) > 0,
        "possession confirmed nothing, so nothing was safe to demote: {run}"
    );

    // Phase order, read off the store.
    let phases =
        m.s.query_column("SELECT phase || '=' || state FROM migration_state ORDER BY phase");
    for expected in [
        "inspect=done",
        "claim_pattern_ownership=done",
        "verify_possession=done",
        "switch_authority=done",
        "demote=done",
    ] {
        assert!(
            phases.iter().any(|p| p == expected),
            "phase `{expected}` is missing or unfinished: {phases:?}"
        );
    }
    let finished: Vec<String> = m.s.query_column(
        "SELECT phase FROM migration_state WHERE finished_at IS NOT NULL
          ORDER BY finished_at, phase",
    );
    let position = |p: &str| finished.iter().position(|x| x == p);
    assert!(
        position("verify_possession") < position("demote"),
        "demotion finished before possession was established. \"Delivered\" and \
         \"durably held\" are different facts and only the second authorizes \
         demotion: {finished:?}"
    );
    assert!(
        position("drain") < position("verify_possession"),
        "possession was checked before anything was handed over: {finished:?}"
    );

    // Every drained shape reached the server, named by its own reference.
    assert_eq!(
        m.server.count(&format!(
            "SELECT count(*) FROM personal_knowledge WHERE id = '{}'",
            ids.personal_queued
        )),
        1,
        "the personal record did not arrive"
    );
    assert_eq!(
        m.server.count(&format!(
            "SELECT count(*) FROM team_knowledge WHERE id = '{}'",
            ids.team_authoritative
        )),
        1,
        "the team record did not arrive"
    );
    assert_eq!(
        m.server.count(&format!(
            "SELECT count(*) FROM memories WHERE id = '{}'",
            ids.memory_queued
        )),
        1,
        "the project memory did not arrive. Project memory is refused on the \
         write path after cutover, so the drain is the only transfer path it has"
    );
    let (from, to, kind) = ids.relation;
    assert_eq!(
        m.server.count(&format!(
            "SELECT count(*) FROM memory_relations
              WHERE from_memory_id = '{from}' AND to_memory_id = '{to}' AND kind = '{kind}'"
        )),
        1,
        "the relation did not arrive. It has no id of its own and travels as \
         its (from, to, kind) triple"
    );
    assert_eq!(
        m.server.count(&format!(
            "SELECT count(*) FROM shared_patterns WHERE owner_user_id = '{}'",
            m.account
        )),
        1,
        "exactly the claimed pattern should have been promoted, and only it"
    );

    // The unclaimed pattern stayed home, and is named as an exception.
    assert_eq!(
        m.s.query_column(&format!(
            "SELECT CAST(count(*) AS TEXT) FROM reusable_patterns WHERE id = '{}'",
            ids.pattern_unclaimed
        )),
        vec!["1".to_string()],
        "an unclaimed pattern was deleted rather than retained"
    );
    let retained =
        m.s.query_column("SELECT dedupe_key || '=' || reason FROM retained_local");
    assert!(
        retained
            .iter()
            .any(|r| r == &format!("pattern:{}=owner_unclaimed", ids.pattern_unclaimed)),
        "the unclaimed pattern is not reported as retained: {retained:?}"
    );
    assert!(
        retained
            .iter()
            .any(|r| r == &format!("knowledge:project:{}=local_only", ids.memory_local_only)),
        "the local-only memory is not reported as retained: {retained:?}"
    );
    // A team proposal this account did not write is not this account's to hand
    // over. It stays local, and the reason names the truth: the server does not
    // hold it, and nobody refused it either.
    assert!(
        retained
            .iter()
            .any(|r| r == &format!("knowledge:team:{}=local_only", ids.team_proposed)),
        "another account's proposal is not reported as retained: {retained:?}"
    );
    assert!(
        !retained.iter().any(|r| r.ends_with("=server_refused")),
        "something was recorded as refused by the server when the server \
         refused nothing: {retained:?}"
    );

    // Machine-local evidence never travels (FR-707).
    assert_eq!(
        m.s.query_column("SELECT CAST(count(*) AS TEXT) FROM pattern_applications"),
        vec!["1".to_string()],
        "the local pattern evidence was moved or dropped"
    );

    // Nothing local was deleted by any of it.
    for (what, sql) in [
        (
            "project memories",
            "SELECT CAST(count(*) AS TEXT) FROM memories",
        ),
        (
            "personal knowledge",
            "SELECT CAST(count(*) AS TEXT) FROM personal_knowledge",
        ),
        (
            "team knowledge",
            "SELECT CAST(count(*) AS TEXT) FROM team_knowledge",
        ),
        (
            "reusable patterns",
            "SELECT CAST(count(*) AS TEXT) FROM reusable_patterns",
        ),
    ] {
        let n: i64 = m.s.query_column(sql)[0].parse().expect("a count");
        assert!(
            n > 0,
            "migration deleted the local {what}. Demotion makes a replica \
             non-authoritative; it does not remove it"
        );
    }
}
