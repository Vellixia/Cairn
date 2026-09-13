//! The web control plane's read API, as a contract (T106,
//! `contracts/web-control-plane.md`, FR-879–FR-895, SC-727, SC-728).
//!
//! US5's independent test. Every row these assertions read is seeded with SQL
//! or through an existing write route, never produced by capture or
//! consolidation, so the control plane is measured on its own rather than on
//! whether some other story's pipeline ran.
//!
//! Six properties, and they are what this file exists for. Each one is a way
//! the API could look finished and still be wrong:
//!
//! - **Zero and unavailable are different answers** (FR-880). A stage that
//!   counted nothing reports `0`; a stage whose mechanism this deployment does
//!   not have reports `null`. Collapsing them tells an operator "nothing
//!   happened" when the truth is "nobody looked", so both halves are asserted
//!   against a real deployment of each shape rather than against a mock.
//! - **The activity default is declared, not inferred** (FR-882). The seven
//!   event kinds and two candidate decisions in `web-control-plane.md` §4 are
//!   what arrives with no `kinds` parameter, and the full stream arrives only
//!   when a caller asks for it by name.
//! - **A reference is complete or it is not a reference.** Every reference this
//!   API emits carries its domain — `knowledge:<domain>:<uuid>` or
//!   `pattern:<uuid>` — because two domains can hold the same UUID and a bare
//!   id cannot say which was meant (SC-766).
//! - **Evidence is summarised, never carried** (FR-893). Memory detail says how
//!   much evidence exists and where it is held; it never carries content, a
//!   path or command output, because none of that is on the server to carry.
//! - **A briefing is never rendered** (FR-839). Retrieval detail carries what
//!   was selected, what it cost and how degraded it was, and the response shape
//!   has no field a briefing could live in. Asserted as an absence over the
//!   whole payload, not over a field list somebody has to keep current.
//! - **Refusal is the API's job** (FR-894a, FR-892). Every project-scoped read
//!   answers a non-member exactly as it answers a project that does not exist,
//!   in both status and body, so the API is not an enumeration oracle; every
//!   admin read refuses a member; and no owner-scoped read has a parameter
//!   capable of naming another owner.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{get_json_status_bearer, post_json_status_bearer};
use serde_json::{json, Value};
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
// Request helpers
// ---------------------------------------------------------------------------

fn get(pg: &Pg, who: &Account, path: &str) -> (Value, u16) {
    get_json_status_bearer(&pg.server.base, path, &who.token)
}

/// The value a stage reports, distinguishing the two answers this feature
/// exists to keep apart.
///
/// `Some(n)` is a count that was established; `None` is JSON `null`. A stage
/// that is missing from the response entirely panics rather than reading as
/// `None`, because "the API forgot this stage" and "the API says it cannot
/// establish this stage" are exactly the confusion SC-728 forbids.
fn stage(body: &Value, name: &str) -> Option<i64> {
    let stages = body["stages"]
        .as_array()
        .unwrap_or_else(|| panic!("no `stages` array in {body}"));
    let row = stages
        .iter()
        .find(|s| s["stage"] == name)
        .unwrap_or_else(|| panic!("the funnel has no `{name}` stage: {body}"));
    assert!(
        row.get("count").is_some(),
        "`{name}` carries no `count` key at all: {row}"
    );
    row["count"].as_i64()
}

fn escape(s: &str) -> String {
    s.replace('\'', "''")
}

// ---------------------------------------------------------------------------
// Seeding
// ---------------------------------------------------------------------------

fn seed_event(pg: &Pg, session: Uuid, who: &Account, kind: &str, seq: i64, agent: &str) -> Uuid {
    let id = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO safe_events
             (event_id, project_id, session_id, account_id, agent, kind,
              session_seq, contract_version, content, occurred_at, received_at)
         VALUES ('{id}', '{}', '{session}', '{}', '{agent}', '{kind}', {seq}, 1,
                 '{{}}'::jsonb, now(), now() + make_interval(secs => {seq}))",
        pg.project, who.id
    ));
    id
}

fn seed_run(pg: &Pg, session: Uuid) -> Uuid {
    let run = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO consolidation_runs
             (run_id, project_id, session_id, started_at, finished_at,
              events_claimed, candidates_proposed, candidates_accepted,
              candidates_refused, extractor_kind, state)
         VALUES ('{run}', '{}', '{session}', now(), now(), 4, 3, 1, 1,
                 'deterministic', 'finished')",
        pg.project
    ));
    run
}

/// One candidate with a decision and, when the decision produced something, the
/// complete reference to what it produced.
fn seed_candidate(pg: &Pg, run: Uuid, decision: &str, key: &str, result: Option<Uuid>) -> Uuid {
    let id = Uuid::now_v7();
    let refusal = if decision == "refused" {
        "'bound_exceeded'".to_string()
    } else {
        "NULL".to_string()
    };
    let (ref_kind, domain, knowledge) = match result {
        Some(m) => ("'knowledge'".into(), "'project'".into(), format!("'{m}'")),
        None => ("NULL".to_string(), "NULL".to_string(), "NULL".to_string()),
    };
    pg.server.execute(&format!(
        "INSERT INTO knowledge_candidates
             (candidate_id, run_id, project_id, proposed_kind, proposed_domain,
              topic_key, value_key, content, decision, refusal_reason,
              result_ref_kind, result_domain, result_knowledge_id)
         VALUES ('{id}', '{run}', '{}', 'fact', 'project', '{key}', '{key}-v',
                 'a candidate claim', '{decision}', {refusal},
                 {ref_kind}, {domain}, {knowledge})",
        pg.project
    ));
    id
}

fn seed_memory(pg: &Pg, session: Uuid, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    let content = escape(content);
    pg.server.execute(&format!(
        "INSERT INTO memories
             (id, project_id, type, scope, scope_key, content, origin_session_id,
              origin_kind, observation_ids, evidence_count, importance,
              verification, verification_authority, last_verified_at,
              verification_basis, evidence_fact_count, reinforcement_count)
         VALUES ('{id}', '{}', 'fact', 'project', '{}', '{content}', '{session}',
                 'consolidated', '[\"obs-1\",\"obs-2\"]'::jsonb, 2, 'high',
                 'verified', 'remote_attested', now(),
                 '[\"command_exit\"]'::jsonb, 2, 3)",
        pg.project, pg.project
    ));
    id
}

/// A trace in whatever delivery state the caller needs, with its items.
fn seed_trace(pg: &Pg, session: Uuid, who: &Account, state: &str) -> Uuid {
    let trace = Uuid::now_v7();
    // The table's own CHECKs decide which columns a state may carry: no latency
    // before generation, no degradation level for a briefing never built, no
    // failure without a reason. Seeding a shape the CHECKs refuse would test
    // nothing, so each state is seeded in the shape the server itself writes.
    let (degradation, latency, failure) = match state {
        "requested" => ("NULL".to_string(), "NULL".to_string(), "NULL".to_string()),
        "failed" => (
            "NULL".to_string(),
            "12".to_string(),
            "'transmission_failed'".to_string(),
        ),
        _ => ("'full'".to_string(), "12".to_string(), "NULL".to_string()),
    };
    pg.server.execute(&format!(
        "INSERT INTO retrieval_traces
             (trace_id, project_id, session_id, account_id, trigger, delivery_point,
              degradation_level, budget_tokens, budget_spent, latency_ms,
              delivery_state, failure_reason)
         VALUES ('{trace}', '{}', '{session}', '{}', 'session_open', 'session_start',
                 {degradation}, 4000, 250, {latency}, '{state}', {failure})",
        pg.project, who.id
    ));
    trace
}

fn seed_trace_item(
    pg: &Pg,
    trace: Uuid,
    ref_kind: &str,
    domain: Option<&str>,
    id: Uuid,
    rank: i32,
) {
    let domain = match domain {
        Some(d) => format!("'{d}'"),
        None => "NULL".to_string(),
    };
    pg.server.execute(&format!(
        "INSERT INTO retrieval_trace_items
             (trace_id, ref_kind, domain, knowledge_id, status, selection_rule,
              rank, source_updated_at)
         VALUES ('{trace}', '{ref_kind}', {domain}, '{id}', 'selected',
                 'scope_first', {rank}, now())"
    ));
}

fn seed_personal(pg: &Pg, owner: &Account, content: &str) -> Uuid {
    let id = Uuid::now_v7();
    let content = escape(content);
    pg.server.execute(&format!(
        "INSERT INTO personal_knowledge
             (id, owner_user_id, knowledge_type, content, writer_id, writer_seq)
         VALUES ('{id}', '{}', 'fact', '{content}', 'writer-{id}', 1)",
        owner.id
    ));
    id
}

/// One team entry in the state the caller asks for, with the lifecycle columns
/// that state requires.
///
/// The table CHECKs that anything past `proposed` names who ratified it and
/// when, so the state and its evidence go in together. A two-step seed that set
/// the state first would be refused by the schema — correctly: a ratified entry
/// with no ratifier is exactly the half-record FR-457 exists to prevent.
fn seed_team(pg: &Pg, author: &Account, content: &str, state: &str) -> Uuid {
    let id = Uuid::now_v7();
    let content = escape(content);
    let (ratifier, ratified_at) = match state {
        "proposed" => ("NULL".to_string(), "NULL".to_string()),
        _ => (format!("'{}'", author.id), "now()".to_string()),
    };
    let (retirer, retired_at) = match state {
        "retired" => (format!("'{}'", author.id), "now()".to_string()),
        _ => ("NULL".to_string(), "NULL".to_string()),
    };
    pg.server.execute(&format!(
        "INSERT INTO team_knowledge
             (id, knowledge_type, content, proposed_by_user_id, writer_id, writer_seq,
              state, ratified_by_user_id, ratified_at, retired_by_user_id, retired_at)
         VALUES ('{id}', 'fact', '{content}', '{}', 'writer-{id}', 1,
                 '{state}', {ratifier}, {ratified_at}, {retirer}, {retired_at})",
        author.id
    ));
    id
}

/// Promote one account to administrator.
///
/// Directly, rather than through the admin routes: making an administrator is
/// this fixture's setup, and routing it through the API under test would make
/// every admin assertion depend on another admin already existing.
fn make_admin(pg: &Pg, who: &Account) {
    pg.server.execute(&format!(
        "UPDATE users SET role = 'admin' WHERE id = '{}'",
        who.id
    ));
}

// ---------------------------------------------------------------------------
// §3 — the twelve-stage funnel (FR-879, FR-880, SC-728)
// ---------------------------------------------------------------------------

/// The twelve stages FR-879 enumerates, in the order it enumerates them.
const FUNNEL_STAGES: &[&str] = &[
    "active_agents",
    "sessions",
    "safe_events_received",
    "capture_failures",
    "consolidation_runs",
    "candidates_produced",
    "knowledge_accepted",
    "candidates_rejected_or_duplicate",
    "reinforcements",
    "conflicts",
    "retrievals",
    "delivery_failures",
];

#[test]
fn the_funnel_reports_every_stage_from_events_received_to_knowledge_accepted() {
    let pg = pg!();
    let (body, code) = get(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/funnel", pg.project),
    );
    assert_eq!(code, 200, "{body}");

    let reported: Vec<String> = body["stages"]
        .as_array()
        .unwrap_or_else(|| panic!("no `stages` array: {body}"))
        .iter()
        .map(|s| s["stage"].as_str().unwrap_or_default().to_string())
        .collect();
    // Order as well as membership. A dashboard renders these in the order the
    // API hands them over, and FR-879 states the order the funnel runs in —
    // events arrive before candidates exist, candidates before knowledge. A set
    // comparison would pass on a funnel that reported delivery failures before
    // the events that caused them (SC-728).
    assert_eq!(
        reported,
        FUNNEL_STAGES
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
        "the funnel's stages are not FR-879's twelve in FR-879's order: {body}"
    );
}

#[test]
fn a_stage_that_counted_nothing_reports_zero_and_not_unavailable() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    for (i, kind) in ["session_opened", "file_changed", "test_result"]
        .iter()
        .enumerate()
    {
        seed_event(&pg, session, &pg.owner, kind, i as i64 + 1, "claude_code");
    }

    let (body, code) = get(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/funnel", pg.project),
    );
    assert_eq!(code, 200, "{body}");
    assert_eq!(stage(&body, "safe_events_received"), Some(3), "{body}");
    assert_eq!(stage(&body, "active_agents"), Some(1), "{body}");
    // Nothing ever reported a capture failure or a delivery failure for this
    // project, and the queries that would have found one ran and found none.
    // That is a zero, and an operator reading `—` here would conclude the
    // funnel could not see the stage at all (FR-880).
    assert_eq!(stage(&body, "capture_failures"), Some(0), "{body}");
    assert_eq!(stage(&body, "delivery_failures"), Some(0), "{body}");
    assert_eq!(stage(&body, "consolidation_runs"), Some(0), "{body}");
}

#[test]
fn a_stage_this_deployment_has_no_mechanism_for_reports_unavailable_and_not_zero() {
    // A server pinned below the migration that created the funnel's tables is
    // the honest form of "the mechanism does not exist here": the same code
    // runs, and it has nowhere to count from. Synthesizing the state would test
    // the synthesis (FR-880).
    let Some(server) = Pg::start_at_v3() else {
        eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
        return;
    };
    let (user, token) = server.new_user("v3-member");
    let project = Uuid::now_v7();
    server.execute(&format!(
        "INSERT INTO projects (id, name) VALUES ('{project}', 'v3-project')"
    ));
    server.execute(&format!(
        "INSERT INTO project_members (project_id, user_id) VALUES ('{project}', '{user}')"
    ));

    let (body, code) = get_json_status_bearer(
        &server.base,
        &format!("/api/projects/{project}/funnel"),
        &token,
    );
    assert_eq!(code, 200, "{body}");
    // `sessions` predates this feature, so it is a real count of zero on this
    // deployment. Everything the autonomous-memory migration introduced has no
    // table to read, so it is unavailable. The two answers appear in the same
    // response, which is the only place the distinction can be seen to hold.
    assert_eq!(stage(&body, "sessions"), Some(0), "{body}");
    for unavailable in [
        "safe_events_received",
        "capture_failures",
        "consolidation_runs",
        "candidates_produced",
        "knowledge_accepted",
        "candidates_rejected_or_duplicate",
        "reinforcements",
        "conflicts",
        "retrievals",
        "delivery_failures",
        "active_agents",
    ] {
        assert_eq!(
            stage(&body, unavailable),
            None,
            "`{unavailable}` reported a number on a deployment that has no table \
             to count it from: {body}"
        );
    }
}

#[test]
fn the_funnel_counts_accepted_claims_and_never_counts_a_corroboration_as_one() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let run = seed_run(&pg, session);
    let memory = seed_memory(&pg, session, "an accepted claim");
    seed_candidate(&pg, run, "accepted", "a", Some(memory));
    seed_candidate(&pg, run, "reinforced", "b", Some(memory));
    seed_candidate(&pg, run, "duplicate", "c", None);
    seed_candidate(&pg, run, "refused", "d", None);
    seed_candidate(&pg, run, "conflicted", "e", None);

    let (body, code) = get(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/funnel", pg.project),
    );
    assert_eq!(code, 200, "{body}");
    assert_eq!(stage(&body, "candidates_produced"), Some(5), "{body}");
    // FR-798a: a corroboration is not a distinct claim, so `reinforced` is
    // counted in its own stage and never added to what Cairn knows.
    assert_eq!(stage(&body, "knowledge_accepted"), Some(1), "{body}");
    assert_eq!(stage(&body, "reinforcements"), Some(1), "{body}");
    assert_eq!(
        stage(&body, "candidates_rejected_or_duplicate"),
        Some(2),
        "{body}"
    );
    assert_eq!(stage(&body, "conflicts"), Some(1), "{body}");
    assert_eq!(stage(&body, "consolidation_runs"), Some(1), "{body}");
}

#[test]
fn the_funnel_counts_only_the_project_it_was_asked_about() {
    let pg = pg!();
    let other = pg.extra_project("second", &[&pg.owner]);
    let mine = pg.session_for(&pg.owner);
    let theirs = pg.session_in(other, &pg.owner);
    seed_event(&pg, mine, &pg.owner, "session_opened", 1, "claude_code");
    pg.server.execute(&format!(
        "INSERT INTO safe_events
             (event_id, project_id, session_id, account_id, agent, kind,
              session_seq, contract_version, content, occurred_at)
         VALUES ('{}', '{other}', '{theirs}', '{}', 'codex', 'session_opened', 1, 1,
                 '{{}}'::jsonb, now())",
        Uuid::now_v7(),
        pg.owner.id
    ));

    let (body, code) = get(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/funnel", pg.project),
    );
    assert_eq!(code, 200, "{body}");
    assert_eq!(stage(&body, "safe_events_received"), Some(1), "{body}");
    assert_eq!(stage(&body, "active_agents"), Some(1), "{body}");
}

// ---------------------------------------------------------------------------
// §4 — the activity feed (FR-881, FR-882)
// ---------------------------------------------------------------------------

/// The seven event kinds `web-control-plane.md` §4 declares as the default.
const DEFAULT_EVENT_KINDS: &[&str] = &[
    "session_opened",
    "session_resumed",
    "session_closed",
    "file_changed",
    "test_result",
    "decision_signal",
    "capture_failed",
];

/// Kinds §4 excludes by name, each of which fires once per tool call or
/// internal transition.
const EXCLUDED_EVENT_KINDS: &[&str] = &[
    "tool_started",
    "tool_succeeded",
    "tool_failed",
    "file_read",
    "context_compacting",
    "context_compacted",
    "subagent_started",
    "subagent_completed",
    "command_executed",
    "test_executed",
    "research_activity",
    "user_instruction_signal",
    "capture_declined",
    "agent_quiesced",
];

fn feed_kinds(body: &Value) -> Vec<String> {
    body["items"]
        .as_array()
        .unwrap_or_else(|| panic!("no `items` array: {body}"))
        .iter()
        .map(|i| i["kind"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn the_activity_feed_defaults_to_the_kinds_the_contract_declares() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let mut seq = 0i64;
    for kind in DEFAULT_EVENT_KINDS.iter().chain(EXCLUDED_EVENT_KINDS) {
        seq += 1;
        seed_event(&pg, session, &pg.owner, kind, seq, "claude_code");
    }

    let (body, code) = get(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/activity", pg.project),
    );
    assert_eq!(code, 200, "{body}");
    let seen = feed_kinds(&body);
    for kind in DEFAULT_EVENT_KINDS {
        assert!(
            seen.iter().any(|k| k == kind),
            "the declared default omits `{kind}`: {body}"
        );
    }
    for kind in EXCLUDED_EVENT_KINDS {
        assert!(
            !seen.iter().any(|k| k == kind),
            "`{kind}` is a firehose kind §4 excludes by name and it arrived \
             without being asked for: {body}"
        );
    }
    // The subset is *declared*, which means the response says what it applied.
    // A client that had to infer the default from what happened to arrive could
    // not tell an excluded kind from a kind nothing produced (FR-882).
    let declared: Vec<String> = body["kinds"]
        .as_array()
        .unwrap_or_else(|| panic!("the feed does not state which kinds it applied: {body}"))
        .iter()
        .map(|k| k.as_str().unwrap_or_default().to_string())
        .collect();
    for kind in DEFAULT_EVENT_KINDS {
        assert!(declared.iter().any(|k| k == kind), "{body}");
    }
    for kind in ["accepted", "conflicted"] {
        assert!(
            declared.iter().any(|k| k == kind),
            "the declared default omits the `{kind}` candidate decision: {body}"
        );
    }
}

#[test]
fn the_full_stream_arrives_only_when_a_caller_asks_for_it_by_name() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let mut seq = 0i64;
    for kind in DEFAULT_EVENT_KINDS.iter().chain(EXCLUDED_EVENT_KINDS) {
        seq += 1;
        seed_event(&pg, session, &pg.owner, kind, seq, "claude_code");
    }

    let everything: Vec<&str> = DEFAULT_EVENT_KINDS
        .iter()
        .chain(EXCLUDED_EVENT_KINDS)
        .copied()
        .collect();
    let (body, code) = get(
        &pg,
        &pg.owner,
        &format!(
            "/api/projects/{}/activity?limit=100&kinds={}",
            pg.project,
            everything.join(",")
        ),
    );
    assert_eq!(code, 200, "{body}");
    let seen = feed_kinds(&body);
    for kind in everything {
        assert!(
            seen.iter().any(|k| k == kind),
            "widening to the full set did not produce `{kind}`: {body}"
        );
    }
}

#[test]
fn a_candidate_decision_is_activity_and_the_quiet_decisions_are_not_default() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let run = seed_run(&pg, session);
    let memory = seed_memory(&pg, session, "an accepted claim");
    seed_candidate(&pg, run, "accepted", "a", Some(memory));
    seed_candidate(&pg, run, "conflicted", "b", None);
    seed_candidate(&pg, run, "reinforced", "c", Some(memory));
    seed_candidate(&pg, run, "duplicate", "d", None);
    seed_candidate(&pg, run, "refused", "e", None);

    let (body, code) = get(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/activity", pg.project),
    );
    assert_eq!(code, 200, "{body}");
    let seen = feed_kinds(&body);
    assert!(seen.iter().any(|k| k == "accepted"), "{body}");
    assert!(seen.iter().any(|k| k == "conflicted"), "{body}");
    for quiet in ["reinforced", "duplicate", "refused"] {
        assert!(
            !seen.iter().any(|k| k == quiet),
            "`{quiet}` arrived in the default feed: {body}"
        );
    }

    // An accepted candidate names what it produced, and it names it completely:
    // `knowledge:project:<uuid>` and not a bare id. Two domains can hold the
    // same UUID, so a reference that cannot say which domain is not a reference
    // (SC-766).
    let accepted = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|i| i["kind"] == "accepted")
        .expect("the accepted decision");
    assert_eq!(
        accepted["reference"]["reference_key"],
        json!(format!("knowledge:project:{memory}")),
        "{accepted}"
    );
    assert_eq!(accepted["reference"]["domain"], "project", "{accepted}");
    assert_eq!(accepted["reference"]["ref_kind"], "knowledge", "{accepted}");

    // The widened feed reaches the quiet decisions, and the refusal carries its
    // reason — the fact an operator actually needs from a refusal.
    let (wide, code) = get(
        &pg,
        &pg.owner,
        &format!(
            "/api/projects/{}/activity?kinds=accepted,conflicted,reinforced,duplicate,refused",
            pg.project
        ),
    );
    assert_eq!(code, 200, "{wide}");
    let refused = wide["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|i| i["kind"] == "refused")
        .unwrap_or_else(|| panic!("the widened feed has no refusal: {wide}"));
    assert_eq!(refused["refusal_reason"], "bound_exceeded", "{refused}");
}

#[test]
fn the_activity_feed_pages_without_repeating_or_skipping_a_row() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    for seq in 1..=5 {
        seed_event(&pg, session, &pg.owner, "file_changed", seq, "claude_code");
    }

    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..6 {
        let path = match &cursor {
            Some(c) => format!(
                "/api/projects/{}/activity?limit=2&cursor={}",
                pg.project,
                urlencode(c)
            ),
            None => format!("/api/projects/{}/activity?limit=2", pg.project),
        };
        let (body, code) = get(&pg, &pg.owner, &path);
        assert_eq!(code, 200, "{body}");
        let items = body["items"].as_array().expect("items").clone();
        assert!(items.len() <= 2, "a page of two returned {}", items.len());
        if items.is_empty() {
            break;
        }
        for item in &items {
            seen.push(item["id"].as_str().expect("id").to_string());
        }
        cursor = body["cursor"].as_str().map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }

    let unique: std::collections::BTreeSet<&String> = seen.iter().collect();
    assert_eq!(
        unique.len(),
        seen.len(),
        "the keyset repeated a row across pages: {seen:?}"
    );
    assert_eq!(
        seen.len(),
        5,
        "walking the cursor produced {} of 5 rows: {seen:?}",
        seen.len()
    );
}

#[test]
fn no_list_honours_a_limit_above_its_stated_ceiling() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // One row is enough: the assertion is on the bound the server applied, and
    // a bound is observable from the request being answered at all rather than
    // from the row count. Seeding 101 rows for each of five lists would make
    // this test the slowest in the file and prove the same thing.
    seed_event(&pg, session, &pg.owner, "file_changed", 1, "claude_code");
    seed_run(&pg, session);
    seed_trace(&pg, session, &pg.owner, "generated");
    seed_personal(&pg, &pg.owner, "a personal note");
    seed_team(&pg, &pg.owner, "a team claim", "authoritative");

    for path in [
        format!("/api/projects/{}/activity?limit=5000", pg.project),
        format!("/api/projects/{}/retrieval-traces?limit=5000", pg.project),
        format!("/api/projects/{}/consolidation-runs?limit=5000", pg.project),
        format!("/api/projects/{}/memories?limit=5000", pg.project),
        "/api/personal/knowledge?limit=5000".to_string(),
        "/api/team/knowledge?limit=5000".to_string(),
    ] {
        let (body, code) = get(&pg, &pg.owner, &path);
        // Clamped, not refused: `web-control-plane.md` §7 states the ceiling is
        // applied rather than enforced by rejection, matching the existing
        // `project_memories` behaviour a client already relies on.
        assert_eq!(code, 200, "{path} answered {code}: {body}");
        assert!(
            body["limit"].as_i64().unwrap_or(0) <= 100,
            "{path} reported a limit above the ceiling: {body}"
        );
    }
}

fn urlencode(raw: &str) -> String {
    raw.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            other => format!("%{:02X}", other as u32),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// §2 — the memory explorer and memory detail (FR-883, FR-884, FR-885, FR-893)
// ---------------------------------------------------------------------------

#[test]
fn the_memory_explorer_carries_every_field_the_explorer_filters_on() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    seed_memory(&pg, session, "a consolidated claim");

    let (body, code) = get(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/memories", pg.project),
    );
    assert_eq!(code, 200, "{body}");
    let row = &body["memories"][0];
    for field in [
        "importance",
        "verification",
        "verification_authority",
        "origin_kind",
        "reinforcement_count",
        "relation_count",
    ] {
        assert!(
            row.get(field).is_some(),
            "the explorer's row has no `{field}`, so FR-883's list cannot be \
             rendered from it: {row}"
        );
    }
    assert_eq!(row["origin_kind"], "consolidated", "{row}");
    assert_eq!(row["importance"], "high", "{row}");
    assert_eq!(row["reinforcement_count"], 3, "{row}");
}

#[test]
fn memory_detail_says_where_a_record_came_from_and_whether_anyone_asked_for_it() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = seed_memory(&pg, session, "a consolidated claim");

    let (body, code) = get(&pg, &pg.owner, &format!("/api/memories/{memory}"));
    assert_eq!(code, 200, "{body}");
    let m = &body["memory"];
    // FR-885: explicit or consolidated, stated rather than inferred from
    // whether a session happens to be named.
    assert_eq!(m["origin_kind"], "consolidated", "{m}");
    assert_eq!(m["provenance"]["session_id"], json!(session), "{m}");
    assert_eq!(m["verification"]["state"], "verified", "{m}");
    assert_eq!(m["verification"]["authority"], "remote_attested", "{m}");
    assert!(
        m["verification"].get("last_verified_at").is_some(),
        "verification has no last_verified_at: {m}"
    );
    assert_eq!(m["reinforcement_count"], 3, "{m}");
}

#[test]
fn memory_detail_summarises_its_evidence_and_carries_none_of_it() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = seed_memory(&pg, session, "a consolidated claim");

    let (body, code) = get(&pg, &pg.owner, &format!("/api/memories/{memory}"));
    assert_eq!(code, 200, "{body}");
    let summary = &body["memory"]["evidence_summary"];
    assert!(
        summary.is_object(),
        "memory detail has no evidence summary: {body}"
    );
    assert_eq!(summary["observation_count"], 2, "{summary}");
    assert_eq!(summary["evidence_count"], 2, "{summary}");
    // FR-893: the section states that the material is local rather than
    // rendering an empty box, and it names the session that holds it so a
    // person can go and look.
    assert_eq!(summary["content_available"], json!(false), "{summary}");
    assert_eq!(summary["local_to_session"], json!(session), "{summary}");

    // The absence, asserted positively over the whole payload. A field list
    // would go stale the first time a column is added; this fails on any key
    // whose name is one evidence content could hide behind.
    let raw = body.to_string();
    for forbidden in [
        "\"evidence_content\"",
        "\"file_path\"",
        "\"absolute_path\"",
        "\"command_output\"",
        "\"stdout\"",
        "\"stderr\"",
        "\"transcript\"",
        "\"raw\"",
    ] {
        assert!(
            !raw.contains(forbidden),
            "memory detail carries {forbidden}, which is evidence and is local \
             to the machine that captured it: {body}"
        );
    }
}

#[test]
fn a_relation_on_memory_detail_is_a_complete_reference_in_both_directions() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let subject = seed_memory(&pg, session, "the claim under inspection");
    let successor = seed_memory(&pg, session, "the claim that replaced it");
    let corroborator = seed_memory(&pg, session, "a claim that reinforces it");
    for (from, to, kind) in [
        (subject, successor, "supersedes"),
        (corroborator, subject, "reinforces"),
    ] {
        pg.server.execute(&format!(
            "INSERT INTO memory_relations
                 (from_memory_id, to_memory_id, kind, project_id, decided_by_session, basis)
             VALUES ('{from}', '{to}', '{kind}', '{}', '{session}', 'test')",
            pg.project
        ));
    }

    let (body, code) = get(&pg, &pg.owner, &format!("/api/memories/{subject}"));
    assert_eq!(code, 200, "{body}");
    let relations = body["memory"]["relations"]
        .as_array()
        .unwrap_or_else(|| panic!("memory detail carries no relations: {body}"));
    assert_eq!(relations.len(), 2, "{body}");

    // FR-884 asks what this record supersedes and what reinforces it — two
    // different questions, and a relation list that reported only the outgoing
    // half could answer one of them. The direction is stated rather than
    // implied by which id happens to appear first.
    let outgoing = relations
        .iter()
        .find(|r| r["kind"] == "supersedes")
        .expect("the outgoing relation");
    assert_eq!(outgoing["direction"], "outgoing", "{outgoing}");
    assert_eq!(
        outgoing["other"]["reference_key"],
        json!(format!("knowledge:project:{successor}")),
        "{outgoing}"
    );
    let incoming = relations
        .iter()
        .find(|r| r["kind"] == "reinforces")
        .expect("the incoming relation");
    assert_eq!(incoming["direction"], "incoming", "{incoming}");
    assert_eq!(
        incoming["other"]["reference_key"],
        json!(format!("knowledge:project:{corroborator}")),
        "{incoming}"
    );
    // No relation is ever a bare id: a reader has to be able to resolve it, and
    // `<uuid>` alone does not say which of four domains to look in.
    for relation in relations {
        assert_eq!(relation["other"]["ref_kind"], "knowledge", "{relation}");
        assert_eq!(relation["other"]["domain"], "project", "{relation}");
    }
}

#[test]
fn memory_detail_reports_where_the_record_has_actually_been_retrieved() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = seed_memory(&pg, session, "a claim that gets retrieved");
    let trace = seed_trace(&pg, session, &pg.owner, "transmitted");
    seed_trace_item(&pg, trace, "knowledge", Some("project"), memory, 1);

    let (body, code) = get(&pg, &pg.owner, &format!("/api/memories/{memory}"));
    assert_eq!(code, 200, "{body}");
    let usage = body["memory"]["retrieval_usage"]
        .as_array()
        .unwrap_or_else(|| panic!("memory detail has no retrieval usage: {body}"));
    assert_eq!(usage.len(), 1, "{body}");
    assert_eq!(usage[0]["trace_id"], json!(trace), "{body}");
    assert_eq!(usage[0]["status"], "selected", "{body}");
    assert_eq!(usage[0]["delivery_state"], "transmitted", "{body}");
    assert_eq!(usage[0]["trigger"], "session_open", "{body}");
}

#[test]
fn retrieval_usage_on_memory_detail_stops_at_the_twenty_most_recent() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = seed_memory(&pg, session, "a very popular claim");
    for _ in 0..25 {
        let trace = seed_trace(&pg, session, &pg.owner, "transmitted");
        seed_trace_item(&pg, trace, "knowledge", Some("project"), memory, 1);
    }

    let (body, code) = get(&pg, &pg.owner, &format!("/api/memories/{memory}"));
    assert_eq!(code, 200, "{body}");
    let usage = body["memory"]["retrieval_usage"].as_array().expect("usage");
    // §7 bounds this at twenty with no further pagination, because the full
    // history is reachable through the traces list filtered by this reference.
    // An unbounded embed would make one memory's detail page grow without limit
    // on a project that retrieves it often (FR-895).
    assert_eq!(usage.len(), 20, "{}", usage.len());

    // And the overflow is genuinely reachable: the traces list takes the same
    // complete reference as a filter, so "view all" is not a dead link.
    let (all, code) = get(
        &pg,
        &pg.owner,
        &format!(
            "/api/projects/{}/retrieval-traces?limit=100&reference_key={}",
            pg.project,
            urlencode(&format!("knowledge:project:{memory}"))
        ),
    );
    assert_eq!(code, 200, "{all}");
    assert_eq!(all["traces"].as_array().expect("traces").len(), 25, "{all}");
}

// ---------------------------------------------------------------------------
// §10 — consolidation runs
// ---------------------------------------------------------------------------

#[test]
fn the_consolidation_run_list_reports_what_each_pass_actually_did() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let run = seed_run(&pg, session);
    seed_candidate(&pg, run, "refused", "a", None);
    seed_candidate(&pg, run, "refused", "b", None);
    seed_candidate(&pg, run, "accepted", "c", None);

    let (body, code) = get(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/consolidation-runs", pg.project),
    );
    assert_eq!(code, 200, "{body}");
    let runs = body["runs"].as_array().expect("runs");
    assert_eq!(runs.len(), 1, "{body}");
    let r = &runs[0];
    assert_eq!(r["run_id"], json!(run), "{r}");
    assert_eq!(r["events_claimed"], 4, "{r}");
    assert_eq!(r["candidates_proposed"], 3, "{r}");
    assert_eq!(r["extractor_kind"], "deterministic", "{r}");
    assert!(r.get("started_at").is_some(), "{r}");
    assert!(r.get("finished_at").is_some(), "{r}");
    // Refusal reasons counted rather than listed one per candidate: the
    // question a run answers is "what did this pass turn away and why", and a
    // reason repeated forty times is one fact with a count (FR-804a).
    let reasons = r["refusal_reasons"].as_array().expect("refusal_reasons");
    assert_eq!(reasons.len(), 1, "{r}");
    assert_eq!(reasons[0]["reason"], "bound_exceeded", "{r}");
    assert_eq!(reasons[0]["n"], 2, "{r}");
}

// ---------------------------------------------------------------------------
// §2, §6 — retrieval traces
// ---------------------------------------------------------------------------

#[test]
fn the_trace_list_reports_the_delivery_lifecycle_of_every_retrieval() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let generated = seed_trace(&pg, session, &pg.owner, "generated");
    let failed = seed_trace(&pg, session, &pg.owner, "failed");

    let (body, code) = get(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/retrieval-traces", pg.project),
    );
    assert_eq!(code, 200, "{body}");
    let traces = body["traces"].as_array().expect("traces");
    assert_eq!(traces.len(), 2, "{body}");
    for trace in traces {
        for field in [
            "trace_id",
            "trigger",
            "delivery_point",
            "degradation_level",
            "delivery_state",
            "acknowledgement_state",
            "created_at",
            "session_id",
        ] {
            assert!(
                trace.get(field).is_some(),
                "the trace row has no `{field}`: {trace}"
            );
        }
    }
    let by_id = |id: Uuid| {
        traces
            .iter()
            .find(|t| t["trace_id"] == json!(id))
            .unwrap_or_else(|| panic!("no row for {id}: {body}"))
            .clone()
    };
    assert_eq!(by_id(generated)["delivery_state"], "generated");
    assert_eq!(by_id(failed)["delivery_state"], "failed");
    // A failure is a first-class row rather than an absence: SC-729 requires
    // every retrieval to be traced, including one that never reached an agent.
    assert_eq!(by_id(failed)["failure_reason"], "transmission_failed");
    // Never asserted as receipt. No vendor mechanism establishes it, so the
    // list reports what there is, which is no evidence (FR-838e).
    assert_eq!(by_id(generated)["acknowledgement_state"], "unavailable");
}

#[test]
fn a_retrieval_trace_names_what_it_selected_and_never_the_briefing_it_built() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let memory = seed_memory(&pg, session, "the claim that was selected");
    let trace = seed_trace(&pg, session, &pg.owner, "generated");
    seed_trace_item(&pg, trace, "knowledge", Some("project"), memory, 1);

    let (body, code) = get(&pg, &pg.owner, &format!("/api/retrieval-traces/{trace}"));
    assert_eq!(code, 200, "{body}");
    let item = &body["items"][0];
    assert_eq!(item["ref_kind"], "knowledge", "{body}");
    assert_eq!(item["domain"], "project", "{body}");
    assert_eq!(
        item["reference_key"],
        json!(format!("knowledge:project:{memory}")),
        "{body}"
    );
    assert_eq!(body["degradation_level"], "full", "{body}");
    assert_eq!(body["budget"]["tokens"], 4000, "{body}");
    assert_eq!(body["budget"]["spent"], 250, "{body}");

    // FR-839: there is no field a briefing could live in, asserted over the
    // whole payload rather than over a list of field names somebody has to keep
    // current. The selected memory's own content is the sharpest case — it is
    // material the reader may legitimately see elsewhere, which is exactly why
    // its presence here would look harmless.
    let raw = body.to_string();
    for forbidden in [
        "briefing",
        "the claim that was selected",
        "rendered",
        "prompt",
    ] {
        assert!(
            !raw.contains(forbidden),
            "the trace carries `{forbidden}`; a trace names what was selected, \
             never what was assembled from it: {body}"
        );
    }
}

#[test]
fn filtering_traces_by_a_reference_the_reader_may_not_see_discloses_nothing() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let pattern = Uuid::now_v7();
    assert!(
        pg.seed_pattern_with_id(&pg.owner, pattern, "the owner's own pattern"),
        "shared_patterns does not exist; this fixture cannot test pattern privacy"
    );
    let trace = seed_trace(&pg, session, &pg.owner, "generated");
    seed_trace_item(&pg, trace, "pattern", None, pattern, 1);

    // The owner sees their own pattern's traces.
    let (mine, code) = get(
        &pg,
        &pg.owner,
        &format!(
            "/api/projects/{}/retrieval-traces?reference_key={}",
            pg.project,
            urlencode(&format!("pattern:{pattern}"))
        ),
    );
    assert_eq!(code, 200, "{mine}");
    assert_eq!(
        mine["traces"].as_array().expect("traces").len(),
        1,
        "{mine}"
    );

    // A co-member of the same project does not. The answer is the same one a
    // reference nobody ever retrieved gets — an empty page — so the filter
    // cannot be used to ask whether a colleague's pattern exists (FR-846a).
    let (theirs, code) = get(
        &pg,
        &pg.member,
        &format!(
            "/api/projects/{}/retrieval-traces?reference_key={}",
            pg.project,
            urlencode(&format!("pattern:{pattern}"))
        ),
    );
    assert_eq!(code, 200, "{theirs}");
    let (unused, code) = get(
        &pg,
        &pg.member,
        &format!(
            "/api/projects/{}/retrieval-traces?reference_key={}",
            pg.project,
            urlencode(&format!("pattern:{}", Uuid::now_v7()))
        ),
    );
    assert_eq!(code, 200, "{unused}");
    assert_eq!(theirs["traces"], unused["traces"], "{theirs}");
    assert_eq!(
        theirs["traces"].as_array().expect("traces").len(),
        0,
        "a co-member read a trace of the owner's pattern: {theirs}"
    );
}

// ---------------------------------------------------------------------------
// §5 — integration health, and §2's system health
// ---------------------------------------------------------------------------

#[test]
fn integration_health_reads_back_exactly_what_a_machine_reported() {
    let pg = pg!();
    let (posted, code) = post_json_status_bearer(
        &pg.server.base,
        &format!("/api/projects/{}/health", pg.project),
        &json!({
            "writer_id": "laptop-a",
            "cells": [{
                "agent": "claude_code",
                "capability": "event:file_changed",
                "stage": "runtime_hook_fired",
                "status": "supported",
                "evidence_kind": "observation",
                "observed_at": "2026-09-02T10:00:00Z",
                "degraded": false
            }, {
                "agent": "claude_code",
                "capability": "event:decision_signal",
                "stage": "runtime_hook_fired",
                "status": "no_evidence",
                "evidence_kind": null,
                "observed_at": null,
                "degraded": false
            }]
        }),
        &pg.owner.token,
    );
    assert_eq!(code, 200, "{posted}");

    let (body, code) = get(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/integration-health", pg.project),
    );
    assert_eq!(code, 200, "{body}");
    let rows = body["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 2, "{body}");
    let observed = rows
        .iter()
        .find(|r| r["capability"] == "event:file_changed")
        .expect("the observed capability");
    // FR-852/FR-853: how a capability was established is a separate axis from
    // whether it works, so `evidence_kind` travels beside `status` rather than
    // folded into it.
    assert_eq!(observed["status"], "supported", "{observed}");
    assert_eq!(observed["evidence_kind"], "observation", "{observed}");
    // FR-857: a capability is observed on a machine, and the row says which.
    assert_eq!(observed["writer_id"], "laptop-a", "{observed}");

    let silent = rows
        .iter()
        .find(|r| r["capability"] == "event:decision_signal")
        .expect("the unobserved capability");
    // FR-856: no observation either way is its own answer, never rendered as
    // working and never as failing.
    assert_eq!(silent["status"], "no_evidence", "{silent}");
    assert_eq!(silent["evidence_kind"], Value::Null, "{silent}");
    assert_eq!(silent["observed_at"], Value::Null, "{silent}");
}

#[test]
fn system_health_reports_ingest_consolidation_and_retrieval_to_an_administrator() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    seed_event(&pg, session, &pg.owner, "file_changed", 1, "claude_code");
    seed_trace(&pg, session, &pg.owner, "failed");
    make_admin(&pg, &pg.owner);

    let (body, code) = get(&pg, &pg.owner, "/api/system/health");
    assert_eq!(code, 200, "{body}");
    for section in ["ingest", "consolidation", "retrieval"] {
        assert!(
            body[section].is_object(),
            "system health has no `{section}` section: {body}"
        );
    }
    assert_eq!(body["ingest"]["events_received"], 1, "{body}");
    assert_eq!(body["retrieval"]["traces"], 1, "{body}");
    assert_eq!(body["retrieval"]["failed"], 1, "{body}");
    // Consolidation's backlog comes from the one read that already answers it,
    // so a second answer cannot drift from the first.
    assert!(
        body["consolidation"].get("backlog_depth").is_some(),
        "{body}"
    );
    assert!(
        body["consolidation"].get("oldest_enqueued_at").is_some(),
        "an absent backlog age is a different answer from zero and must be \
         reported as one: {body}"
    );
}

// ---------------------------------------------------------------------------
// §6 — the domains: owner-only means owner-only (FR-888, T110)
// ---------------------------------------------------------------------------

#[test]
fn the_personal_feed_is_the_callers_own_and_has_no_parameter_naming_anyone_else() {
    let pg = pg!();
    let mine = seed_personal(&pg, &pg.owner, "the owner's private note");
    let theirs = seed_personal(&pg, &pg.member, "the member's private note");

    let (body, code) = get(&pg, &pg.owner, "/api/personal/knowledge");
    assert_eq!(code, 200, "{body}");
    let ids: Vec<Value> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["id"].clone())
        .collect();
    assert!(ids.contains(&json!(mine)), "{body}");
    assert!(
        !ids.contains(&json!(theirs)),
        "the owner read a co-member's personal knowledge: {body}"
    );
    assert!(
        !body.to_string().contains("the member's private note"),
        "{body}"
    );

    // The guarantee is the absence of a parameter, not a check on one. Every
    // spelling an owner selector could take is tried, and none of them may
    // change the answer — a route whose response moved when one of these
    // appeared would have the field a later edit could turn into a selector.
    for attempt in [
        format!("?owner_user_id={}", pg.member.id),
        format!("?owner={}", pg.member.id),
        format!("?user_id={}", pg.member.id),
        format!("?account_id={}", pg.member.id),
    ] {
        let (probed, code) = get(&pg, &pg.owner, &format!("/api/personal/knowledge{attempt}"));
        assert!(
            code == 200 || code == 400,
            "/api/personal/knowledge{attempt} answered {code}: {probed}"
        );
        if code == 200 {
            assert_eq!(
                probed["items"], body["items"],
                "/api/personal/knowledge{attempt} changed the answer, so the \
                 route has a field that names an owner: {probed}"
            );
        }
    }
}

#[test]
fn a_pattern_is_readable_by_its_owner_and_by_nobody_else() {
    let pg = pg!();
    let pattern = Uuid::now_v7();
    assert!(
        pg.seed_pattern_with_id(&pg.owner, pattern, "the owner's own pattern"),
        "shared_patterns does not exist; this fixture cannot test pattern privacy"
    );
    make_admin(&pg, &pg.member);

    let (mine, code) = get(&pg, &pg.owner, "/api/patterns");
    assert_eq!(code, 200, "{mine}");
    assert!(mine.to_string().contains(&pattern.to_string()), "{mine}");

    // A co-member who is also an administrator. Administration is authority
    // over the server's shared corpus and over accounts; it is not a key to
    // another account's private notes, and there is no route that makes it one
    // (FR-708d, SC-761).
    let (theirs, code) = get(&pg, &pg.member, "/api/patterns");
    assert_eq!(code, 200, "{theirs}");
    assert!(
        !theirs.to_string().contains(&pattern.to_string()),
        "an administrator read another account's personal pattern: {theirs}"
    );
    assert!(
        !theirs.to_string().contains("the owner's own pattern"),
        "{theirs}"
    );
}

#[test]
fn a_proposed_team_entry_reaches_its_author_and_an_administrator_and_no_one_else() {
    let pg = pg!();
    let proposal = seed_team(&pg, &pg.owner, "a claim awaiting ratification", "proposed");
    let ratified = seed_team(
        &pg,
        &pg.owner,
        "a claim the team stands behind",
        "authoritative",
    );
    let admin = pg.extra_account("team-admin", false);
    make_admin(&pg, &admin);

    let has = |who: &Account, id: Uuid| -> bool {
        let (body, code) = get(&pg, who, "/api/team/knowledge?limit=100");
        assert_eq!(code, 200, "{body}");
        body["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|i| i["id"] == json!(id))
    };

    // The author sees their own proposal; a co-member does not; an
    // administrator does, because deciding what to ratify requires seeing what
    // is waiting (`sync-namespaces.md` §1a, FR-464).
    assert!(
        has(&pg.owner, proposal),
        "the author cannot see their own proposal"
    );
    assert!(!has(&pg.member, proposal), "a co-member read a proposal");
    assert!(!has(&pg.outsider, proposal), "an outsider read a proposal");
    assert!(
        has(&admin, proposal),
        "an administrator cannot see a proposal"
    );

    // Ratified guidance is a server-wide default and reaches every
    // authenticated account, including one that is a member of no project at
    // all — team knowledge is not membership-scoped (FR-463).
    for who in [&pg.owner, &pg.member, &pg.outsider, &admin] {
        assert!(
            has(who, ratified),
            "an authenticated account cannot read authoritative team knowledge"
        );
    }
}

// ---------------------------------------------------------------------------
// §2.1 — refusal, never an empty list (FR-894a, FR-892)
// ---------------------------------------------------------------------------

/// Every project-scoped read this contract adds or extends, addressed by its
/// project.
fn project_scoped_reads(project: Uuid) -> Vec<String> {
    vec![
        format!("/api/projects/{project}/funnel"),
        format!("/api/projects/{project}/activity"),
        format!("/api/projects/{project}/memories"),
        format!("/api/projects/{project}/retrieval-traces"),
        format!("/api/projects/{project}/consolidation-runs"),
        format!("/api/projects/{project}/integration-health"),
    ]
}

#[test]
fn a_non_member_and_a_project_that_does_not_exist_get_the_same_refusal() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    seed_event(&pg, session, &pg.owner, "file_changed", 1, "claude_code");
    seed_run(&pg, session);
    seed_trace(&pg, session, &pg.owner, "generated");
    seed_memory(&pg, session, "a claim an outsider must not read");
    let imaginary = Uuid::now_v7();

    for (real, absent) in project_scoped_reads(pg.project)
        .into_iter()
        .zip(project_scoped_reads(imaginary))
    {
        let (existing_body, existing) = get(&pg, &pg.outsider, &real);
        let (missing_body, missing) = get(&pg, &pg.outsider, &absent);
        // Not "a refusal" but "the same refusal". A `403` for a project that
        // exists and a `404` for one that does not is an enumeration oracle:
        // anyone with an account could walk project ids and learn which are
        // real (FR-894a).
        assert_eq!(
            existing, missing,
            "{real} answered {existing} and {absent} answered {missing}, which \
             tells an outsider that the first project exists"
        );
        assert_eq!(existing, 403, "{real} answered {existing}: {existing_body}");
        assert_eq!(
            existing_body, missing_body,
            "{real} and {absent} refuse with different bodies, which is the \
             same oracle wearing a different status code"
        );
        // And it is a refusal rather than an empty page. An empty list is
        // indistinguishable from a missing guard.
        assert!(
            existing_body["error"]["code"] == "forbidden",
            "{real} did not refuse: {existing_body}"
        );
        assert!(
            existing_body.get("items").is_none()
                && existing_body.get("stages").is_none()
                && existing_body.get("memories").is_none()
                && existing_body.get("rows").is_none()
                && existing_body.get("traces").is_none()
                && existing_body.get("runs").is_none(),
            "{real} answered a non-member with data: {existing_body}"
        );
    }
}

#[test]
fn every_control_plane_read_refuses_an_unauthenticated_caller() {
    let pg = pg!();
    let mut paths = project_scoped_reads(pg.project);
    paths.push("/api/personal/knowledge".to_string());
    paths.push("/api/team/knowledge".to_string());
    paths.push("/api/system/health".to_string());
    for path in paths {
        let (body, code) = get_json_status_bearer(&pg.server.base, &path, "not-a-real-token");
        assert_eq!(code, 401, "{path} answered {code}: {body}");
    }
}

#[test]
fn an_administration_only_read_refuses_a_member_server_side() {
    let pg = pg!();
    // The UI hides the nav entry, and that is not a control. The assertion is
    // made against the route, because the threat model is a member with a token
    // and a shell, not a member with a browser (FR-892).
    let (body, code) = get(&pg, &pg.member, "/api/system/health");
    assert_eq!(code, 403, "{body}");
    assert_eq!(body["error"]["code"], "forbidden", "{body}");

    make_admin(&pg, &pg.member);
    let (body, code) = get(&pg, &pg.member, "/api/system/health");
    assert_eq!(code, 200, "{body}");
}

// ---------------------------------------------------------------------------
// SC-727 — the whole path, walked through the API and nothing else
// ---------------------------------------------------------------------------

#[test]
fn the_path_from_a_session_to_a_delivered_briefing_is_walkable_through_the_api_alone() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let event = seed_event(&pg, session, &pg.owner, "decision_signal", 1, "claude_code");
    let run = seed_run(&pg, session);
    let memory = seed_memory(&pg, session, "the claim the path produces");
    let candidate = seed_candidate(&pg, run, "accepted", "path", Some(memory));
    pg.server.execute(&format!(
        "INSERT INTO candidate_source_events (candidate_id, event_id)
         VALUES ('{candidate}', '{event}')"
    ));
    let trace = seed_trace(&pg, session, &pg.owner, "transmitted");
    seed_trace_item(&pg, trace, "knowledge", Some("project"), memory, 1);

    // 1. The session's activity names the event that arrived.
    let (activity, code) = get(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/activity?limit=100", pg.project),
    );
    assert_eq!(code, 200, "{activity}");
    let arrival = activity["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|i| i["id"] == json!(event))
        .unwrap_or_else(|| panic!("the event is not in the feed: {activity}"));
    assert_eq!(arrival["session_id"], json!(session), "{arrival}");

    // 2. The run that consolidated it, and 3. the decision it reached.
    let (runs, code) = get(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/consolidation-runs", pg.project),
    );
    assert_eq!(code, 200, "{runs}");
    assert!(
        runs["runs"]
            .as_array()
            .expect("runs")
            .iter()
            .any(|r| r["run_id"] == json!(run)),
        "{runs}"
    );
    let decision = activity["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|i| i["reference"]["reference_key"] == json!(format!("knowledge:project:{memory}")))
        .unwrap_or_else(|| panic!("the acceptance is not in the feed: {activity}"));
    assert_eq!(decision["kind"], "accepted", "{decision}");

    // 4. The knowledge itself, with its origin and provenance.
    let (detail, code) = get(&pg, &pg.owner, &format!("/api/memories/{memory}"));
    assert_eq!(code, 200, "{detail}");
    assert_eq!(detail["memory"]["origin_kind"], "consolidated", "{detail}");
    assert_eq!(
        detail["memory"]["provenance"]["session_id"],
        json!(session),
        "{detail}"
    );

    // 5. The retrieval that delivered it, reached from the knowledge and not
    // from a database the web interface cannot see (SC-727).
    let usage = detail["memory"]["retrieval_usage"]
        .as_array()
        .expect("retrieval_usage");
    assert_eq!(usage[0]["trace_id"], json!(trace), "{detail}");
    let (traced, code) = get(&pg, &pg.owner, &format!("/api/retrieval-traces/{trace}"));
    assert_eq!(code, 200, "{traced}");
    assert_eq!(traced["session_id"], json!(session), "{traced}");
    assert_eq!(traced["delivery_state"], "transmitted", "{traced}");
    assert_eq!(
        traced["items"][0]["reference_key"],
        json!(format!("knowledge:project:{memory}")),
        "{traced}"
    );
}
