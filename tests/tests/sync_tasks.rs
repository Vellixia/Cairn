//! What crosses the wire for tasks, and what a peer must not overwrite.
//!
//! Three defects found by walking `quickstart.md` US7 and US11 on two real
//! machines against a real server. Each is invisible to a single machine, and
//! each was invisible to the suite because nothing drove a task *through* the
//! server and read it back on the other side.

use cairn_e2e::{attach_server, Sandbox, Server};
use serde_json::Value;

fn server() -> Option<Server> {
    match Server::start() {
        Some(s) => Some(s),
        None => {
            eprintln!(
                "SKIPPED: set CAIRN_TEST_DATABASE_URL (e.g. \
                 `docker run -p 5433:5432 postgres:18-alpine`) to run the server suite"
            );
            None
        }
    }
}

struct Pair {
    a: Sandbox,
    b: Sandbox,
}

fn pair(server: &Server, label: &str) -> Pair {
    let token = server.new_user_token(label);
    let a = Sandbox::new();
    attach_server(&a, server, &token);
    a.must(&["init"]);
    let project_id = a.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .expect("a shared project")
        .to_string();

    let b = Sandbox::new();
    attach_server(&b, server, &token);
    b.must(&["init"]);
    b.json(&["link", "--project", &project_id]);
    Pair { a, b }
}

fn task_with_criteria(s: &Sandbox, title: &str, criteria: &[&str]) -> String {
    let mut argv = vec!["task", "new", "--title", title, "--goal", "a goal"];
    for c in criteria {
        argv.push("--criterion");
        argv.push(c);
    }
    argv.push("--json");
    s.json(&argv)["task"]["id"]
        .as_str()
        .expect("task id")
        .to_string()
}

fn shown(s: &Sandbox, task_id: &str) -> Value {
    s.json(&["task", "show", task_id])
}

/// A task created on one machine reaches the other.
///
/// The server accepted tasks and never handed them back: `/api/sync/changes`
/// returned memories, relations, criteria and blockers. So a criterion arrived
/// naming a `task_id` that could never arrive, was correctly held rather than
/// invented — and stayed held forever.
#[test]
fn a_task_created_on_one_machine_arrives_on_the_other() {
    let Some(server) = server() else { return };
    let p = pair(&server, "tasks-cross");

    let task_id = task_with_criteria(&p.a, "Retry backoff", &["backoff is exponential"]);
    p.a.must(&["sync", "now"]);
    p.b.must(&["sync", "now"]);

    let t = shown(&p.b, &task_id);
    assert_eq!(
        t["task"]["title"], "Retry backoff",
        "the task did not arrive: {t}"
    );
    assert_eq!(
        t["criteria"].as_array().map(Vec::len).unwrap_or(0),
        1,
        "the task arrived without its criteria: {t}"
    );
}

/// Criteria given at creation time cross, not only ones added afterwards.
///
/// `create_task` enqueued the task and not the criteria it had just seeded, so
/// a task created with `--criterion` arrived on a peer as a shell: zero
/// criteria, and a completion readiness of `ready`, because nothing was
/// outstanding.
#[test]
fn criteria_given_at_creation_time_reach_a_peer() {
    let Some(server) = server() else { return };
    let p = pair(&server, "tasks-seeded");

    let task_id = task_with_criteria(
        &p.a,
        "Release readiness",
        &["the config port is 9000", "the docker pools are expanded"],
    );
    p.a.must(&["sync", "now"]);
    p.b.must(&["sync", "now"]);

    let t = shown(&p.b, &task_id);
    assert_eq!(
        t["criteria"].as_array().map(Vec::len).unwrap_or(0),
        2,
        "criteria seeded at creation did not cross: {t}"
    );
    assert_eq!(
        t["completion_readiness"], "not_ready",
        "a task whose criteria did not arrive reported itself ready: {t}"
    );
}

/// Two machines, two criteria, offline: both survive and the digests agree.
///
/// The digest is the guarantee, because both sides compute it from the criteria
/// themselves. `local_revision` is a private counter and is never compared.
#[test]
fn disjoint_offline_criterion_edits_converge_on_one_digest() {
    let Some(server) = server() else { return };
    let p = pair(&server, "tasks-converge");

    let task_id = task_with_criteria(&p.a, "Two machines", &["A's criterion", "B's criterion"]);
    p.a.must(&["sync", "now"]);
    p.b.must(&["sync", "now"]);

    let criteria = shown(&p.a, &task_id)["criteria"].clone();
    let first = criteria[0]["id"].as_str().expect("id").to_string();
    let second = criteria[1]["id"].as_str().expect("id").to_string();

    p.a.must(&["task", "criterion", "set", &first, "--state", "satisfied"]);
    p.b.must(&["task", "criterion", "set", &second, "--state", "satisfied"]);

    for _ in 0..2 {
        p.a.must(&["sync", "now"]);
        p.b.must(&["sync", "now"]);
    }

    let a = shown(&p.a, &task_id);
    let b = shown(&p.b, &task_id);
    assert_eq!(
        a["state_digest"], b["state_digest"],
        "two machines disagreed about the task's state:\nA {a}\nB {b}"
    );
    for view in [&a, &b] {
        let states: Vec<&str> = view["criteria"]
            .as_array()
            .expect("criteria")
            .iter()
            .map(|c| c["state"].as_str().unwrap_or("?"))
            .collect();
        assert_eq!(
            states,
            vec!["satisfied", "satisfied"],
            "one machine's change was lost: {view}"
        );
    }
}

/// A memory this machine verified keeps its own authority through a sync.
///
/// It was pushed, and the pull applied the peer's badge back over the local
/// one: `verified` stayed, so nothing looked wrong, but the authority became
/// `remote_cairn`. Authority decides two things and both then refused it — its
/// own project could no longer promote it, and it no longer counted towards
/// local readiness, on the strength of a check this machine had run.
#[test]
fn a_locally_verified_memory_does_not_come_back_as_a_peers() {
    let Some(server) = server() else { return };
    let p = pair(&server, "authority-roundtrip");
    p.a.write_file("config/app.yml", "server:\n  port: 8080\n");

    let id = p.a.json(&[
        "memory",
        "add",
        "The API listens on 8080",
        "--type",
        "fact",
        "--scope",
        "project",
        "--topic-key",
        "service.api_port",
        "--value-key",
        "8080",
    ])["memory"]["id"]
        .as_str()
        .expect("id")
        .to_string();
    p.a.must(&[
        "evidence",
        "add",
        "--memory",
        &id,
        "--type",
        "configuration",
        "--subject",
        "API port",
        "--value",
        "8080",
        "--locator",
        "config/app.yml#server.port",
    ]);
    p.a.must(&["verify", "--memory", &id]);

    let before = p.a.json(&["memory", "show", &id])["memory"]["verification"].clone();
    assert_eq!(before["authority"], "cairn", "{before}");

    // Out to the server and back.
    p.a.must(&["sync", "now"]);
    p.a.must(&["sync", "now"]);

    let after = p.a.json(&["memory", "show", &id])["memory"]["verification"].clone();
    assert_eq!(
        after["authority"], "cairn",
        "a round trip through the server rewrote this machine's own authority: {after}"
    );

    // And the peer, which really did import it, wears the imported badge.
    p.b.must(&["sync", "now"]);
    let peer = p.b.json(&["memory", "show", &id])["memory"]["verification"].clone();
    assert_eq!(
        peer["authority"], "remote_cairn",
        "an imported check did not say it was imported: {peer}"
    );
}
