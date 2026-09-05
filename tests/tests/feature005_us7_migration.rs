//! User Story 7, end to end: a populated Feature 004 store is migrated by a
//! person running five commands, and the server closes the old write path
//! behind them (T155; FR-878, SC-719–SC-723).
//!
//! # What makes this the *story* test
//!
//! The other US7 files each hold one mechanism still — ownership, restart,
//! cutover, possession. This walks the sequence a developer actually performs,
//! in order, and asks at each step the question they would ask:
//!
//! 1. *What is in here?* — `--inspect`, which must change nothing.
//! 2. *Whose are these patterns?* — `--claim-patterns`, because nobody can
//!    answer that from the data.
//! 3. *Move it.* — `--run`, which hands over what it may and names what it
//!    could not.
//! 4. *What did not move, and why?* — `--status`, record by record.
//! 5. *Try the leftovers again.* — `--retry-retained`.
//!
//! Then the operator cuts the fleet over, and the store that has migrated keeps
//! working while the protocol it used to speak is closed.
//!
//! # The thesis, in one line
//!
//! **No record is lost, duplicated or reassigned, and nothing is demoted that
//! the server was not confirmed to hold.** Every assertion below is a
//! statement about one of those three.

use cairn_e2e::feature005::{install_legacy_v7, LegacyIds};
use cairn_e2e::{attach_server, post_json_status_bearer, Sandbox, Server};
use serde_json::json;
use uuid::Uuid;

/// The whole story, on a store that was in use.
#[test]
fn a_feature_004_store_is_migrated_and_then_the_fleet_cuts_over() {
    let Some(server) = Server::start_with_admin("us7-admin@example.test", "hunter2hunter2") else {
        eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
        return;
    };
    let s = Sandbox::new();
    let (account, token) = server.new_user("us7-migrating");
    let ids: LegacyIds = install_legacy_v7(&s, account);
    attach_server(&s, &server, &token);
    assert!(s.cairn(&["link", "--create"]).ok(), "linking the project");

    // -----------------------------------------------------------------------
    // 1. What is in here?
    // -----------------------------------------------------------------------
    let before = local_fingerprint(&s);
    let inspect = s.json(&["migrate", "--inspect"])["inspect"].clone();
    assert!(
        inspect["records"]["memory:active"].as_i64().unwrap_or(0) >= 3,
        "inspect found no project knowledge in a populated store: {inspect}"
    );
    assert_eq!(
        inspect["patterns_eligible_for_claim"]
            .as_array()
            .map(Vec::len),
        Some(2),
        "both legacy patterns are owner-less and therefore claimable: {inspect}"
    );
    assert_eq!(
        local_fingerprint(&s),
        before,
        "inspect changed the store. A user runs it to decide *whether* to \
         migrate, and one that quietly started a migration answers a question \
         nobody asked"
    );

    // -----------------------------------------------------------------------
    // 2. Whose are these patterns?
    //
    // Nobody can answer that from the data — `reusable_patterns` has no owner
    // column and this store may have been used with several accounts — so the
    // answer comes from the person, once, and is persisted before anything is
    // deliverable.
    // -----------------------------------------------------------------------
    let claims = s.json(&[
        "migrate",
        "--claim-patterns",
        &ids.pattern_claimable.to_string(),
    ]);
    assert_eq!(
        claims["claims"][0]["outcome"], "claimed",
        "the first claim of an owner-less legacy pattern: {claims}"
    );
    let claimed_pattern_id = claims["claims"][0]["pattern_id"]
        .as_str()
        .expect("a claim carries the identity it persisted")
        .to_string();
    assert_eq!(
        s.query_column(&format!(
            "SELECT pattern_id FROM legacy_pattern_claims WHERE local_pattern_id = '{}'",
            ids.pattern_claimable
        )),
        vec![claimed_pattern_id.clone()],
        "the identity was reported but not persisted, so a retry under another \
         credential could mint a different one"
    );

    // -----------------------------------------------------------------------
    // 3. Move it.
    // -----------------------------------------------------------------------
    let run = s.json(&["migrate", "--run"])["run"].clone();
    assert_eq!(
        run["mode"], "server_authoritative",
        "the store did not reach server authority: {run}"
    );
    assert!(
        run["drain"]["delivered"].as_i64().unwrap_or(0) >= 5,
        "not every drainable shape moved: {run}"
    );
    assert!(
        run["demoted"].as_i64().unwrap_or(0) > 0,
        "nothing was demoted, so the migration handed nothing over: {run}"
    );

    // Nothing is lost: every local row is still there. Demotion makes a replica
    // non-authoritative; it is not a deletion (FR-866, SC-747).
    for (what, table) in [
        ("project memories", "memories"),
        ("personal knowledge", "personal_knowledge"),
        ("team knowledge", "team_knowledge"),
        ("reusable patterns", "reusable_patterns"),
        ("local pattern evidence", "pattern_applications"),
    ] {
        let n: i64 = s.query_column(&format!("SELECT CAST(count(*) AS TEXT) FROM {table}"))[0]
            .parse()
            .expect("a count");
        assert!(n > 0, "the migration deleted the local {what}");
    }

    // Nothing is duplicated: the claimed pattern is one row on the server, and
    // running again leaves it one row.
    let patterns_for_account =
        format!("SELECT count(*) FROM shared_patterns WHERE owner_user_id = '{account}'");
    assert_eq!(
        server.count(&patterns_for_account),
        1,
        "exactly the claimed pattern should have been promoted"
    );
    s.json(&["migrate", "--run"]);
    assert_eq!(
        server.count(&patterns_for_account),
        1,
        "a second run produced a second canonical pattern"
    );

    // Nothing is reassigned: the pattern the server holds is keyed by the
    // identity the *claim* persisted, not by anything recomputed at delivery.
    assert_eq!(
        server.query_column(&format!(
            "SELECT pattern_id::text FROM shared_patterns WHERE owner_user_id = '{account}'"
        )),
        vec![claimed_pattern_id],
        "the promoted pattern does not carry the identity its claim recorded"
    );

    // -----------------------------------------------------------------------
    // 4. What did not move, and why?
    // -----------------------------------------------------------------------
    let status = s.json(&["migrate", "--status"])["status"].clone();
    let retained = status["retained"].as_array().cloned().unwrap_or_default();
    assert!(
        !retained.is_empty(),
        "a store with a local-only memory and an unclaimed pattern has \
         exceptions, and they are reported individually rather than summarized \
         away: {status}"
    );
    let reason_for = |reference: String| -> String {
        retained
            .iter()
            .find(|r| r["reference"] == reference)
            .map(|r| r["reason"].as_str().unwrap_or("").to_string())
            .unwrap_or_else(|| format!("(not reported: {retained:?})"))
    };
    assert_eq!(
        reason_for(format!("knowledge:project:{}", ids.memory_local_only)),
        "local_only",
        "a memory the user kept local has nothing canonical to defer to, and \
         that is what the reason has to say"
    );
    assert_eq!(
        reason_for(format!("pattern:{}", ids.pattern_unclaimed)),
        "owner_unclaimed",
        "an unclaimed pattern is named as unclaimed, not attributed to whoever \
         happened to run the migration"
    );
    assert!(
        !status["complete"].as_bool().unwrap_or(true),
        "a store with named exceptions is not 'fully server-authoritative', and \
         saying so would hide them: {status}"
    );

    // -----------------------------------------------------------------------
    // 5. Try the leftovers again.
    //
    // On demand, never on a timer: a retained record is an exception somebody
    // should look at, and retrying it quietly forever turns a reported problem
    // into a background one.
    // -----------------------------------------------------------------------
    let retry = s.json(&["migrate", "--retry-retained"]);
    assert!(
        retry["still_retained"].as_i64().unwrap_or(0) > 0,
        "the local-only memory and the unclaimed pattern cannot be transferred \
         by retrying, and the retry says so: {retry}"
    );
    assert_eq!(
        s.query_column(&format!(
            "SELECT CAST(count(*) AS TEXT) FROM reusable_patterns WHERE id = '{}'",
            ids.pattern_unclaimed
        )),
        vec!["1".to_string()],
        "a retry deleted the record it could not transfer"
    );

    // -----------------------------------------------------------------------
    // 6. The operator closes the old write path.
    // -----------------------------------------------------------------------
    let admin = server.token_for("us7-admin@example.test", "hunter2hunter2");
    let (cut, status_code) =
        post_json_status_bearer(&server.base, "/api/admin/cutover", &json!({}), &admin);
    assert_eq!(status_code, 200, "an admin cuts the fleet over: {cut}");
    assert_eq!(cut["mode"], "server_authoritative");

    // The migrated store keeps working. Its reads are untouched — a demoted
    // replica is a cache, and a cache with no read path can never refill
    // (§11.9) — and it no longer needs the write path that just closed.
    for feed in ["/api/sync/changes/personal", "/api/sync/changes/team"] {
        let code = server.get_status(feed, &token);
        assert_eq!(
            code, 200,
            "{feed} was refused after cutover. Refusing the read would make a \
             migrated store lose personal and team knowledge on local-store \
             loss, which is the opposite of what the authority change promises"
        );
    }

    // And the old dual-authority write path is closed, by shape.
    let (refused, code) = post_json_status_bearer(
        &server.base,
        "/api/sync/batch",
        &json!({ "project_id": server_project(&s), "items": [ {
            "idempotency_key": Uuid::now_v7().to_string(),
            "entity_type": "personal_knowledge",
            "entity_id": Uuid::now_v7(),
            "operation": "upsert",
            "payload": { "id": Uuid::now_v7(), "knowledge_type": "fact",
                         "content": "written the old way", "writer_id": Uuid::now_v7(),
                         "writer_seq": 99, "applicability": [] },
        } ] }),
        &token,
    );
    assert!(
        code == 409 || refused["results"][0]["status"] == "rejected",
        "a pre-005 knowledge write was accepted after cutover: {refused}"
    );

    // Still nothing lost, after all of it.
    let n: i64 = s.query_column("SELECT CAST(count(*) AS TEXT) FROM reusable_patterns")[0]
        .parse()
        .expect("a count");
    assert_eq!(n, 2, "the store lost a pattern somewhere in the story");
}

/// Every local knowledge row, as one comparable string.
///
/// Content and both keys, because the two things an inspect must not do are
/// rewrite content and re-key records — and re-keying is the one that would
/// otherwise look like nothing happened.
fn local_fingerprint(s: &Sandbox) -> Vec<String> {
    let mut out = s.query_column(
        "SELECT 'm:' || id || '|' || content || '|' || COALESCE(topic_key,'') || '|'
                || COALESCE(value_key,'') FROM memories ORDER BY id",
    );
    out.extend(s.query_column(
        "SELECT 'p:' || id || '|' || content || '|' || COALESCE(topic_key,'') || '|'
                || COALESCE(value_key,'') FROM personal_knowledge ORDER BY id",
    ));
    out.extend(s.query_column(
        "SELECT 't:' || id || '|' || content || '|' || COALESCE(topic_key,'') || '|'
                || COALESCE(value_key,'') FROM team_knowledge ORDER BY id",
    ));
    out.extend(
        s.query_column("SELECT 'r:' || id || '|' || title FROM reusable_patterns ORDER BY id"),
    );
    out
}

/// The server's id for the sandbox's linked project.
///
/// A batch names the project it is for, and the server knows it by its own id
/// rather than by the local one.
fn server_project(s: &Sandbox) -> String {
    s.query_column(
        "SELECT server_project_id FROM projects WHERE server_project_id IS NOT NULL LIMIT 1",
    )
    .first()
    .cloned()
    .expect("the project is linked")
}
