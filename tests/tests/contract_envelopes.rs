//! The shape of what a caller actually receives (D1–D4).
//!
//! Eleven defects were found in this branch by reading code and driving a live
//! agent; four more were found by walking `quickstart.md` on a real repository,
//! and all four are the same kind of miss. `ProposalOutcome` and `MemoryResult`
//! are constructed under test hundreds of times, and *serialized* under test
//! nowhere — so a wire object could omit five contract fields, and a documented
//! CLI verb could not exist at all, while 1,043 tests stayed green.
//!
//! These assert the envelope. Every field named here is named by
//! `contracts/mcp-tools.md`; a test that reached into the Rust struct instead
//! would reproduce exactly the blind spot that let these through.

use cairn_e2e::Sandbox;
use serde_json::Value;

fn body(out: &str) -> Value {
    let v: Value = serde_json::from_str(out).expect("json envelope");
    v["data"].clone()
}

fn add(s: &Sandbox, topic: &str, value: &str, content: &str) -> Value {
    let r = s.cairn(&[
        "memory",
        "add",
        content,
        "--type",
        "fact",
        "--scope",
        "project",
        "--topic-key",
        topic,
        "--value-key",
        value,
        "--json",
    ]);
    assert!(r.ok(), "{}", r.stderr);
    body(&r.stdout)
}

/// Every key the contract fixes for `reconciliation`, present and correct on a
/// corroborating write — the case FR-327 exists for.
#[test]
fn a_corroborating_write_reports_every_contract_field() {
    let s = Sandbox::new();
    let first = add(
        &s,
        "auth.strategy",
        "jwt",
        "JWT uses HS256 with a shared secret",
    );
    let second = add(
        &s,
        "auth.strategy",
        "jwt",
        "JWT uses RS256 with rotating public keys",
    );

    let r = &second["reconciliation"];
    assert_eq!(r["outcome"], "corroborating");
    assert_eq!(r["matched_memory_id"], first["memory"]["id"]);
    assert_eq!(r["matched_value_key"], "jwt");
    assert_eq!(r["subject"], "auth.strategy");
    // Nothing was merged and nothing was written. That is the whole point:
    // agreement about a value is not agreement about a claim (D46).
    assert!(
        r["relation_recorded"].is_null(),
        "a corroborating write recorded a relation: {r}"
    );
    assert_eq!(r["conflict_detected"], false);
    assert_eq!(
        r["next_step"], "if this is the same claim, call action=reinforce with memory_id",
        "the prompt FR-327 depends on is missing or reworded"
    );
    assert_eq!(second["notes"][0], "corroborating_member");
}

/// A conflict names every competing member and picks none of them.
///
/// `matched_memory_id` is null on purpose. A conflict is intrinsically several,
/// and naming one of them as *the* match would be arbitration by identifier —
/// which is the silent winner this feature exists to prevent (FR-334).
#[test]
fn a_conflict_names_every_competing_answer_and_arbitrates_none() {
    let s = Sandbox::new();
    let sqs = add(
        &s,
        "deploy.queue_backend",
        "sqs",
        "Deploys queue through SQS",
    );
    let rabbit = add(
        &s,
        "deploy.queue_backend",
        "rabbitmq",
        "Deploys queue through RabbitMQ",
    );

    let r = &rabbit["reconciliation"];
    assert_eq!(r["outcome"], "conflict_detected");
    assert_eq!(r["conflict_detected"], true);
    assert!(
        r["matched_memory_id"].is_null(),
        "a conflict named one member as the match: {r}"
    );
    assert_eq!(r["subject"], "deploy.queue_backend");
    assert_eq!(r["relation_recorded"], "conflicts_with");
    assert_eq!(
        r["competing_memory_ids"],
        serde_json::json!([sqs["memory"]["id"]]),
        "the competing member is missing from the report"
    );
    assert!(
        r["next_step"].as_str().unwrap_or("").contains("reconcile"),
        "a conflict did not point at the explicit decision: {r}"
    );
}

/// A duplicate reports the relation the write actually recorded.
#[test]
fn a_duplicate_reports_the_relation_it_recorded() {
    let s = Sandbox::new();
    let first = add(
        &s,
        "infrastructure.production_database",
        "postgresql",
        "Production runs PostgreSQL 16",
    );
    let again = add(
        &s,
        "infrastructure.production_database",
        "postgresql",
        "Production runs PostgreSQL 16",
    );

    let r = &again["reconciliation"];
    assert_eq!(r["outcome"], "duplicate");
    assert_eq!(r["matched_memory_id"], first["memory"]["id"]);
    assert_eq!(r["relation_recorded"], "duplicates");
    assert_eq!(r["conflict_detected"], false);
    assert!(r["next_step"].is_null(), "a duplicate needs no call: {r}");
}

/// A free-form memory belongs to no subject, and says so by omission.
#[test]
fn a_free_form_write_claims_no_subject() {
    let s = Sandbox::new();
    let r = s.cairn(&[
        "memory",
        "add",
        "Errors are returned, never logged and swallowed",
        "--type",
        "convention",
        "--scope",
        "project",
        "--json",
    ]);
    assert!(r.ok(), "{}", r.stderr);
    let v = body(&r.stdout)["reconciliation"].clone();
    assert_eq!(v["outcome"], "created");
    assert!(v["subject"].is_null());
    assert!(v["matched_memory_id"].is_null());
    assert_eq!(v["conflict_detected"], false);
}

/// Every field `contracts/mcp-tools.md` §`cairn_search` fixes per result.
///
/// Without them a caller cannot tell a verified result from a drifted one, nor
/// a canonical answer from one of several competing ones — which is the entire
/// value the search is supposed to add.
#[test]
fn a_search_result_carries_the_contract_fields() {
    let s = Sandbox::new();
    add(
        &s,
        "auth.strategy",
        "jwt",
        "JWT uses HS256 with a shared secret",
    );
    let second = add(
        &s,
        "auth.strategy",
        "jwt",
        "JWT uses RS256 with rotating public keys",
    );

    let r = s.cairn(&["memory", "search", "--topic-key", "auth.strategy", "--json"]);
    assert!(r.ok(), "{}", r.stderr);
    let results = body(&r.stdout)["results"].clone();
    let hit = results
        .as_array()
        .expect("results")
        .iter()
        .find(|h| h["id"] == second["memory"]["id"])
        .cloned()
        .expect("the memory just written is in its own subject's results");

    assert_eq!(hit["importance"], "normal");
    assert_eq!(hit["pinned"], false);

    let verification = &hit["verification"];
    assert_eq!(verification["state"], "unverified");
    assert!(
        verification["authority"].is_null(),
        "an unverified result claimed an authority"
    );
    assert_eq!(verification["fact_count"], 0);
    assert_eq!(verification["basis"], serde_json::json!([]));
    assert!(verification.get("last_verified_at").is_some());

    // Never presented as verifications (FR-406) — a separate object, always.
    assert_eq!(hit["reinforcement"]["count"], 0);
    // One origin: the session that wrote it. Never zero — a memory always came
    // from somewhere — and never presented as a verification (FR-406).
    assert_eq!(hit["reinforcement"]["distinct_origins"], 1);

    let subject = &hit["subject"];
    assert_eq!(subject["reconciliation"], "corroborated");
    assert_eq!(subject["is_canonical_answer"], true);
    assert_eq!(
        subject["competing_answers"],
        serde_json::json!([]),
        "agreement about a value was reported as competition"
    );
    assert_eq!(
        subject["corroborating_answers"]
            .as_array()
            .expect("corroborating")
            .len(),
        1,
        "the other statement about the same value is missing: {subject}"
    );
}

/// A conflicted subject reports the other answer as competing, not corroborating.
#[test]
fn a_conflicted_result_names_the_answer_it_competes_with() {
    let s = Sandbox::new();
    let sqs = add(
        &s,
        "deploy.queue_backend",
        "sqs",
        "Deploys queue through SQS",
    );
    add(
        &s,
        "deploy.queue_backend",
        "rabbitmq",
        "Deploys queue through RabbitMQ",
    );

    let r = s.cairn(&[
        "memory",
        "search",
        "--topic-key",
        "deploy.queue_backend",
        "--json",
    ]);
    assert!(r.ok(), "{}", r.stderr);
    let results = body(&r.stdout)["results"].clone();
    let hit = results
        .as_array()
        .expect("results")
        .iter()
        .find(|h| h["id"] == sqs["memory"]["id"])
        .cloned()
        .expect("the sqs memory is in the results");

    let subject = &hit["subject"];
    assert_eq!(subject["reconciliation"], "conflicted");
    assert_eq!(subject["is_canonical_answer"], true);
    assert_eq!(
        subject["competing_answers"]
            .as_array()
            .expect("competing")
            .len(),
        1,
        "the competing answer is missing: {subject}"
    );
    assert_eq!(subject["corroborating_answers"], serde_json::json!([]));
}

/// A verified result names what established it.
///
/// The authority is what separates a check Cairn ran from an agent's assertion
/// (FR-370), and a result that omitted it would let the second pass for the
/// first.
#[test]
fn a_verified_result_names_its_authority() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    let m = add(&s, "service.api_port", "8080", "The API listens on 8080");
    let id = m["memory"]["id"].as_str().expect("id").to_string();

    let e = s.cairn(&[
        "evidence",
        "add",
        "--type",
        "configuration",
        "--subject",
        "API port",
        "--value",
        "8080",
        "--locator",
        "config/app.yml#server.port",
        "--memory",
        &id,
        "--json",
    ]);
    assert!(e.ok(), "{}", e.stderr);
    let v = s.cairn(&["verify", "--memory", &id, "--json"]);
    assert!(v.ok(), "{}", v.stderr);

    let r = s.cairn(&[
        "memory",
        "search",
        "--topic-key",
        "service.api_port",
        "--json",
    ]);
    assert!(r.ok(), "{}", r.stderr);
    let hit = body(&r.stdout)["results"][0].clone();
    let verification = &hit["verification"];
    assert_eq!(verification["state"], "verified");
    assert_eq!(
        verification["authority"], "cairn",
        "a verified result did not say what established it: {verification}"
    );
    assert_eq!(verification["fact_count"], 1);
    assert_eq!(
        verification["basis"],
        serde_json::json!(["configuration"]),
        "the basis names verifier kinds only"
    );
    assert!(!verification["last_verified_at"].is_null());
}

/// `cairn memory supersede` exists, and history survives it (D1).
///
/// The capability was reachable over MCP and from `repo::supersede_memory` and
/// from nowhere on the command line, while `quickstart.md` documented the verb
/// in two separate sections.
#[test]
fn the_command_line_can_supersede_a_memory() {
    let s = Sandbox::new();
    let original = add(
        &s,
        "infrastructure.production_database",
        "postgresql",
        "Production runs PostgreSQL 16",
    );
    let id = original["memory"]["id"].as_str().expect("id").to_string();

    let r = s.cairn(&[
        "memory",
        "supersede",
        "Migrated production to CockroachDB in the 2026-08 migration",
        "--memory-id",
        &id,
        "--type",
        "decision",
        "--scope",
        "project",
        "--topic-key",
        "infrastructure.production_database",
        "--value-key",
        "cockroachdb",
        "--json",
    ]);
    assert!(r.ok(), "cairn memory supersede failed: {}", r.stderr);
    let out = body(&r.stdout);
    assert_eq!(out["superseded"], serde_json::json!(id));

    // Today's answer.
    let now = s.cairn(&[
        "memory",
        "search",
        "--topic-key",
        "infrastructure.production_database",
        "--json",
    ]);
    assert!(now.ok(), "{}", now.stderr);
    let current = body(&now.stdout);
    assert_eq!(current["results"][0]["value_key"], "cockroachdb");
    assert_eq!(
        current["total"], 1,
        "the superseded memory is still current: {current}"
    );

    // The original is retained, not deleted — that is what makes a July
    // handoff still make sense (FR-020, FR-342).
    let show = s.cairn(&["memory", "show", &id, "--json"]);
    assert!(show.ok(), "{}", show.stderr);
    assert_eq!(body(&show.stdout)["memory"]["state"], "superseded");
}

/// What reconciliation decided reaches a human, not just the JSON (D2).
#[test]
fn memory_add_renders_what_reconciliation_decided() {
    let s = Sandbox::new();
    let first = add(
        &s,
        "auth.strategy",
        "jwt",
        "JWT uses HS256 with a shared secret",
    );
    let matched = first["memory"]["id"].as_str().expect("id").to_string();

    let r = s.cairn(&[
        "memory",
        "add",
        "JWT uses RS256 with rotating public keys",
        "--type",
        "fact",
        "--scope",
        "project",
        "--topic-key",
        "auth.strategy",
        "--value-key",
        "jwt",
    ]);
    assert!(r.ok(), "{}", r.stderr);
    assert!(
        r.stdout.contains("reconciliation: corroborating"),
        "the outcome was not rendered: {}",
        r.stdout
    );
    assert!(
        r.stdout.contains(&matched),
        "the member it agrees with was not named: {}",
        r.stdout
    );
    assert!(
        r.stdout.contains("cairn memory reinforce"),
        "the one call that would settle it was not offered: {}",
        r.stdout
    );
}

/// A free-form write renders nothing extra: it took part in no reconciliation,
/// and saying `created` would imply it did.
#[test]
fn a_free_form_write_renders_no_reconciliation_line() {
    let s = Sandbox::new();
    let r = s.cairn(&[
        "memory",
        "add",
        "Errors are returned, never logged and swallowed",
        "--type",
        "convention",
        "--scope",
        "project",
    ]);
    assert!(r.ok(), "{}", r.stderr);
    assert!(
        !r.stdout.contains("reconciliation:"),
        "a free-form memory reported a reconciliation: {}",
        r.stdout
    );
}

/// The same envelope over MCP, because that is the surface an agent reads.
#[test]
fn the_mcp_create_response_carries_the_same_reconciliation() {
    let s = Sandbox::new();
    let mut mcp = cairn_e2e::Mcp::start(&s);
    let cwd = s.repo_path().to_string_lossy().to_string();

    let create = |mcp: &mut cairn_e2e::Mcp, content: &str| -> Value {
        mcp.tool_result(
            "cairn_remember",
            serde_json::json!({
                "action": "create", "type": "fact", "scope": "project",
                "content": content,
                "topic_key": "auth.strategy", "value_key": "jwt"
            }),
            &cwd,
        )["content"][0]["text"]
            .clone()
    };
    let first = create(&mut mcp, "JWT uses HS256 with a shared secret");
    let second = create(&mut mcp, "JWT uses RS256 with rotating public keys");

    let r = &second["reconciliation"];
    assert_eq!(r["outcome"], "corroborating", "{second}");
    assert_eq!(r["matched_memory_id"], first["memory"]["id"]);
    assert_eq!(r["subject"], "auth.strategy");
    assert_eq!(
        r["next_step"],
        "if this is the same claim, call action=reinforce with memory_id"
    );
}

/// The Feature 003 write surface works over MCP, not just past the dispatcher.
///
/// `every_advertised_action_is_dispatched` proves no advertised action is
/// refused as unknown. This proves the wiring behind one of them is real: an
/// agent reinforcing a memory over MCP records the relation and the accounting
/// moves.
#[test]
fn an_agent_can_reinforce_over_mcp() {
    let s = Sandbox::new();
    let mut mcp = cairn_e2e::Mcp::start(&s);
    let cwd = s.repo_path().to_string_lossy().to_string();

    let create = |mcp: &mut cairn_e2e::Mcp, content: &str| -> String {
        mcp.tool_result(
            "cairn_remember",
            serde_json::json!({
                "action": "create", "type": "fact", "scope": "project",
                "content": content,
                "topic_key": "infrastructure.production_database",
                "value_key": "postgresql"
            }),
            &cwd,
        )["content"][0]["text"]["memory"]["id"]
            .as_str()
            .expect("id")
            .to_string()
    };
    let first = create(&mut mcp, "Production runs PostgreSQL 16");
    let second = create(&mut mcp, "The production database is Postgres");

    let reinforced = mcp.tool_result(
        "cairn_remember",
        serde_json::json!({
            "action": "reinforce", "memory_id": first, "from_memory_id": second
        }),
        &cwd,
    );
    assert_eq!(
        reinforced["isError"], false,
        "reinforce over MCP failed: {reinforced}"
    );

    // The accounting moved, which is the whole point of an explicit
    // reinforcement (FR-322): reinforcements are counted apart from distinct
    // origins, and neither is a verification.
    let body = &reinforced["content"][0]["text"];
    assert_eq!(
        body["reinforcements"], 1,
        "the reinforcement was not recorded: {body}"
    );
}
