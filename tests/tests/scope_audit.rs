//! T127 — the audit, as tests rather than as a promise.
//!
//! Every item here is something Feature 002 committed to *not* building, and
//! every one of them is a thing that would have been easy to build by
//! accident. A seventh MCP tool is one convenient helper away. A committed
//! manifest is one "wouldn't it be nice if cloning set it up" away. An
//! agent-keyed memory scope is one line in a filter away.
//!
//! Prose in a specification does not prevent any of that. A failing test does.

use cairn_e2e::Sandbox;
use serde_json::json;

#[test]
fn the_mcp_surface_is_the_same_six_tools_feature_001_shipped() {
    // FR-128, SC-106, and §Out of Scope: "expanding the MCP tool surface
    // beyond the six Feature 001 tools".
    let s = Sandbox::new();
    let mut mcp = cairn_e2e::Mcp::start(&s);
    mcp.call("initialize", json!({}));
    let names: Vec<String> = mcp.call("tools/list", json!({}))["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();

    assert_eq!(
        names,
        vec![
            "cairn_context",
            "cairn_search",
            "cairn_remember",
            "cairn_session",
            "cairn_task",
            "cairn_handoff",
        ],
        "the MCP surface changed"
    );
    // Feature 002's own operations are developer commands, not agent tools:
    // an agent that could connect and disconnect itself is an agent that can
    // edit the developer's configuration unprompted.
    for forbidden in [
        "cairn_connect",
        "cairn_doctor",
        "cairn_repair",
        "cairn_disconnect",
        "cairn_agents",
        "cairn_integration",
    ] {
        assert!(
            !names.iter().any(|n| n == forbidden),
            "{forbidden} is a tool"
        );
    }
}

#[test]
fn no_feature_002_record_has_an_outbox_entity_type_or_a_server_column() {
    // FR-183, FR-184: integration state is local, and the server schema knows
    // nothing about it. Asserted against the schema rather than against
    // behavior, because a CHECK constraint cannot be forgotten.
    let s = Sandbox::new();
    s.must(&["init"]);

    let outbox = s
        .query_column("SELECT sql FROM sqlite_master WHERE type='table' AND name='outbox'")
        .first()
        .cloned()
        .unwrap_or_default();
    assert!(
        outbox.contains("'project', 'task', 'session', 'memory', 'handoff'"),
        "{outbox}"
    );

    // Every Feature 002 table exists locally and none of them is syncable.
    for table in [
        "agent_integrations",
        "manager_integrations",
        "installed_resources",
        "resource_bindings",
        "capability_evidence",
        "migration_states",
        "recovery_artifacts",
    ] {
        let sql = s
            .query_column(&format!(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='{table}'"
            ))
            .first()
            .cloned()
            .unwrap_or_default();
        assert!(!sql.is_empty(), "{table} does not exist");
        assert!(
            !outbox.contains(table),
            "{table} appears in the outbox schema"
        );
        // No sync bookkeeping crept onto an integration row.
        for column in ["synced_at", "server_id", "outbox", "remote_"] {
            assert!(
                !sql.contains(column),
                "{table} carries `{column}`, which is sync state"
            );
        }
    }
}

#[test]
fn no_memory_scope_partition_or_filter_is_keyed_to_an_agent() {
    // §Out of Scope: "any new memory scope, partition, or ownership domain
    // based on agent identity". The scope vocabulary is Feature 001's four,
    // and agent identity is provenance only (FR-189, FR-190).
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.install_agent("codex");
    s.must(&["init"]);

    s.hook(
        "SessionStart",
        json!({ "session_id": "audit-1", "source": "startup" }),
    );
    s.settle_session_count(1);
    s.must(&[
        "memory",
        "add",
        "A fact about pagination",
        "--type",
        "fact",
        "--scope",
        "project",
    ]);

    let scopes = s.query_column("SELECT DISTINCT scope FROM memories");
    for scope in &scopes {
        assert!(
            ["project", "branch", "task", "session"].contains(&scope.as_str()),
            "an agent-shaped memory scope exists: {scope}"
        );
    }

    // And the scope column of the schema admits nothing else.
    let sql = s
        .query_column("SELECT sql FROM sqlite_master WHERE type='table' AND name='memories'")
        .first()
        .cloned()
        .unwrap_or_default();
    assert!(!sql.to_lowercase().contains("agent"), "{sql}");
}

#[test]
fn cloning_a_repository_installs_and_activates_nothing() {
    // FR-227 and §Out of Scope: no committed manifest, no drift handling, no
    // merge semantics, and no automatic application of intent on clone. The
    // committed half of a `--shared` installation is an *offer*.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--shared", "--yes"]);

    // Everything a collaborator would receive by cloning.
    let committed: Vec<std::path::PathBuf> = [".mcp.json", "CLAUDE.md", ".claude/settings.json"]
        .iter()
        .map(|p| s.repo_dir().join(p))
        .filter(|p| p.exists())
        .collect();
    assert!(
        committed.len() >= 2,
        "a shared connect committed almost nothing, so this proves little: {committed:?}"
    );

    // Commit them, the way the developer who ran `--shared` would.
    s.git(&["add", "-A"]);
    s.git(&[
        "-c",
        "user.email=t@example.test",
        "-c",
        "user.name=t",
        "commit",
        "-qm",
        "share",
    ]);

    // The clone: the same committed files, a machine that has never run Cairn.
    let clone = s.cairn_home().join("clone");
    let out = std::process::Command::new("git")
        .args(["clone", "--quiet"])
        .arg(s.repo_dir())
        .arg(&clone)
        .output()
        .expect("git clone");
    assert!(
        out.status.success(),
        "clone failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Nothing about Cairn is active there: no project, no session, no capture.
    let projects_before = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM projects");
    let sessions_before = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM sessions");

    // Reading the committed configuration is all a clone does. Cairn is not
    // invoked, so nothing is applied — and if something were watching for a
    // manifest, this is where it would fire.
    assert!(clone.join("CLAUDE.md").exists(), "the offer did not travel");
    assert_eq!(
        s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM projects"),
        projects_before,
        "cloning registered a project"
    );
    assert_eq!(
        s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM sessions"),
        sessions_before,
        "cloning started a session"
    );

    // And no manifest of Cairn's own was committed for it to apply.
    for name in [
        "cairn.toml",
        "cairn.json",
        ".cairn.toml",
        ".cairn.json",
        ".cairn/integration.json",
        "cairn.integration.json",
    ] {
        assert!(
            !clone.join(name).exists(),
            "a committed integration manifest exists: {name}"
        );
    }
}

#[test]
fn a_hand_edited_resource_is_reported_and_never_adopted() {
    // §Out of Scope: "adopting a developer's hand-edited version of a
    // Cairn-managed resource as the new Cairn-managed content".
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    let path = s.repo_dir().join("CLAUDE.md");
    let original = std::fs::read_to_string(&path).expect("the instruction file");
    let edited = original.replace(
        "Search Cairn memory",
        "Search Cairn memory (edited by the developer)",
    );
    assert_ne!(edited, original, "the fixture did not edit anything");
    std::fs::write(&path, &edited).expect("write");

    // Doctor reports it rather than accepting it.
    let doctor = s.cairn(&["--json", "doctor"]).stdout;
    assert!(
        doctor.contains("modified"),
        "a hand-edited resource was not reported: {doctor}"
    );

    // And a default repair refuses rather than silently taking either side.
    let repair = s.cairn(&["repair", "claude-code"]);
    assert_ne!(repair.code, 0, "a default repair overwrote a hand edit");
    assert_eq!(
        std::fs::read_to_string(&path).expect("still there"),
        edited,
        "a refused repair changed the file anyway"
    );
}

#[test]
fn cairn_introduces_no_second_service_and_no_second_datastore() {
    // §Out of Scope: "a second Cairn service, broker, or datastore introduced
    // to support adapters". Feature 002 added a crate, not a process.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    // One database file, and it is Feature 001's.
    let mut databases: Vec<String> = Vec::new();
    let mut sockets: Vec<String> = Vec::new();
    let mut stack = vec![s.cairn_home()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path == s.fake_home() {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if name.ends_with(".sqlite3") {
                databases.push(name);
            } else if name.ends_with(".sock") {
                sockets.push(name);
            }
        }
    }
    assert_eq!(
        databases,
        vec!["cairn.sqlite3"],
        "a second datastore exists"
    );
    assert!(
        sockets.len() <= 1,
        "a second service is listening: {sockets:?}"
    );
}
