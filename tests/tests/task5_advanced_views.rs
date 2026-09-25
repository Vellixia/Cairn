//! Real PostgreSQL checks for bounded advanced project reads.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::get_json_status_bearer;
use serde_json::Value;
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

fn get(pg: &Pg, who: &Account, path: &str) -> (Value, u16) {
    get_json_status_bearer(&pg.server.base, path, &who.token)
}

fn seed_memory(pg: &Pg, session: Uuid, id: Uuid) {
    pg.server.execute(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content,
           state, origin_session_id, origin_kind)
         VALUES ('{id}', '{}', 'fact', 'project', '{}', 'advanced view seed',
                 'active', '{session}', 'explicit')",
        pg.project, pg.project
    ));
}

#[test]
fn graph_and_replay_are_authenticated_isolated_capped_and_safe() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let root = Uuid::now_v7();
    seed_memory(&pg, session, root);
    for _ in 0..51 {
        let leaf = Uuid::now_v7();
        seed_memory(&pg, session, leaf);
        pg.server.execute(&format!(
            "INSERT INTO memory_relations
               (from_memory_id, to_memory_id, kind, project_id, decided_by_session, basis)
             VALUES ('{root}', '{leaf}', 'supersedes', '{}', '{session}', 'deterministic_rule')",
            pg.project
        ));
    }
    for seq in 1..=101 {
        let event = Uuid::now_v7();
        pg.server.execute(&format!(
            "INSERT INTO safe_events
               (event_id, project_id, session_id, account_id, agent, kind, session_seq,
                contract_version, content, occurred_at, received_at)
             VALUES ('{event}', '{}', '{session}', '{}', 'test', 'tool_call', {seq}, 1,
                     '{{\"private_marker\":\"never replay this\"}}'::jsonb, now(),
                     now() + make_interval(secs => {seq}))",
            pg.project, pg.owner.id
        ));
    }

    let graph_path = format!(
        "/api/projects/{}/graph?memory_id={root}&hops=99",
        pg.project
    );
    let replay_path = format!("/api/projects/{}/replay", pg.project);
    let (_, unauth_graph) = get_json_status_bearer(&pg.server.base, &graph_path, "bad-token");
    let (_, unauth_replay) = get_json_status_bearer(&pg.server.base, &replay_path, "bad-token");
    assert_eq!(unauth_graph, 401);
    assert_eq!(unauth_replay, 401);
    let (_, foreign_graph) = get(&pg, &pg.outsider, &graph_path);
    let (_, foreign_replay) = get(&pg, &pg.outsider, &replay_path);
    assert_eq!(foreign_graph, 403);
    assert_eq!(foreign_replay, 403);

    let (graph, status) = get(&pg, &pg.owner, &graph_path);
    assert_eq!(status, 200, "{graph}");
    assert_eq!(graph["hops"], 2);
    assert_eq!(graph["max_hops"], 2);
    assert_eq!(graph["max_edges"], 50);
    assert_eq!(graph["edges"].as_array().unwrap().len(), 50);
    assert_eq!(graph["truncated"], true);

    let (replay, status) = get(&pg, &pg.owner, &replay_path);
    assert_eq!(status, 200, "{replay}");
    assert_eq!(replay["limit"], 100);
    assert_eq!(replay["events"].as_array().unwrap().len(), 100);
    assert_eq!(replay["content_available"], false);
    assert!(!replay.to_string().contains("private_marker"));
}

#[test]
fn graph_traverses_inbound_relations_at_every_hop() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    let c = Uuid::now_v7();
    for id in [a, b, c] {
        seed_memory(&pg, session, id);
    }
    for (from, to) in [(b, a), (c, b)] {
        pg.server.execute(&format!(
            "INSERT INTO memory_relations (from_memory_id, to_memory_id, kind, project_id, decided_by_session, basis) VALUES ('{from}', '{to}', 'reinforces', '{}', '{session}', 'deterministic_rule')",
            pg.project
        ));
    }
    let (graph, status) = get(
        &pg,
        &pg.owner,
        &format!("/api/projects/{}/graph?memory_id={a}&hops=2", pg.project),
    );
    assert_eq!(status, 200, "{graph}");
    let edges = graph["edges"].as_array().unwrap();
    assert!(edges.iter().any(|edge| edge["from"] == b.to_string()
        && edge["to"] == a.to_string()
        && edge["depth"] == 1));
    assert!(edges.iter().any(|edge| edge["from"] == c.to_string()
        && edge["to"] == b.to_string()
        && edge["depth"] == 2));
}

#[test]
fn analytics_counts_only_canonical_project_records() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let foreign_project = pg.extra_project("advanced-foreign", &[&pg.outsider]);
    let foreign_session = pg.session_in(foreign_project, &pg.outsider);

    for (project, session, who, count) in [
        (pg.project, session, &pg.owner, 2_i64),
        (foreign_project, foreign_session, &pg.outsider, 1_i64),
    ] {
        for seq in 1..=count {
            let event = Uuid::now_v7();
            pg.server.execute(&format!(
                "INSERT INTO safe_events (event_id, project_id, session_id, account_id, agent, kind,
                   session_seq, contract_version, content, occurred_at)
                 VALUES ('{event}', '{project}', '{session}', '{}', 'test', 'tool_call', {seq}, 1,
                         '{{}}'::jsonb, now())",
                who.id
            ));
        }
    }
    let run = Uuid::now_v7();
    pg.server.execute(&format!(
        "INSERT INTO consolidation_runs (run_id, project_id, session_id, started_at, extractor_kind, state)
         VALUES ('{run}', '{}', '{session}', now(), 'deterministic', 'finished')",
        pg.project
    ));
    for (state, latency, degradation, failure) in [
        ("transmitted", "10", "'full'", "NULL"),
        ("failed", "30", "NULL", "'hook_transmission_failed'"),
    ] {
        let trace = Uuid::now_v7();
        pg.server.execute(&format!(
            "INSERT INTO retrieval_traces (trace_id, project_id, session_id, account_id, trigger,
               delivery_point, degradation_level, latency_ms, delivery_state, failure_reason)
             VALUES ('{trace}', '{}', '{session}', '{}', 'session_open', 'session_open',
                     {degradation}, {latency}, '{state}', {failure})",
            pg.project, pg.owner.id
        ));
    }
    let path = format!("/api/projects/{}/analytics", pg.project);
    let (_, unauth) = get_json_status_bearer(&pg.server.base, &path, "bad-token");
    assert_eq!(unauth, 401);
    let (_, nonmember) = get(&pg, &pg.outsider, &path);
    assert_eq!(nonmember, 403);
    let (body, status) = get(&pg, &pg.owner, &path);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["capture"], 2);
    assert_eq!(body["consolidation"], 1);
    assert_eq!(body["retrieval"], 2);
    assert_eq!(body["delivery"], 1);
    assert_eq!(body["latency"]["count"], 2);
    assert_eq!(body["latency"]["average_ms"], 20.0);
    assert_eq!(body["failures"], 1);
}
