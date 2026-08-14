//! T085 — what disconnect is allowed to touch (US9 #8, FR-179, FR-180,
//! SC-116).
//!
//! Removal is the operation a developer is most afraid of, and rightly: they
//! are asking a tool to edit files it did not create, in a home directory full
//! of things it knows nothing about. So the assertion is not that disconnect
//! removes Cairn's resources — it is everything disconnect must leave alone.
//!
//! Two blast radii, and both are absolute. Nothing in the *project's memory*
//! is deleted: a developer who disconnects an agent still has every decision,
//! failure and handoff they recorded, because the integration is how knowledge
//! is captured and not where it lives. And nothing in the *configuration* that
//! Cairn did not write changes by a single byte.

use cairn_e2e::Sandbox;
use serde_json::{json, Value};

/// A developer's own settings, in the files Cairn will edit.
fn seed_configuration(s: &Sandbox) {
    let home = s.fake_home();
    std::fs::create_dir_all(s.repo_dir().join(".claude")).unwrap();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(home.join(".config").join("opencode")).unwrap();

    std::fs::write(
        home.join(".claude.json"),
        json!({
            "version": "2.1.220",
            "primaryApiKey": "sk-ant-DEVELOPERS-OWN-KEY",
            "theme": "dark",
            "mcpServers": {
                "internal": { "command": "internal-mcp", "args": ["--stdio"] }
            }
        })
        .to_string(),
    )
    .unwrap();

    std::fs::write(
        home.join(".codex").join("config.toml"),
        "model = \"gpt-5-codex\"\n\
         approval_policy = \"on-request\"\n\
         \n\
         [mcp_servers.internal]\n\
         command = \"internal-mcp\"\n",
    )
    .unwrap();

    // The developer's own hooks, in the file Cairn registers its own into.
    std::fs::write(
        s.repo_dir().join(".claude").join("settings.local.json"),
        json!({
            "hooks": {
                "SessionStart": [
                    { "hooks": [{ "type": "command", "command": "echo mine" }] }
                ],
                "PreToolUse": [
                    { "matcher": "Bash", "hooks": [{ "type": "command", "command": "guard.sh" }] }
                ]
            },
            "permissions": { "allow": ["Bash(cargo test:*)"] }
        })
        .to_string(),
    )
    .unwrap();

    // The developer's own instructions, in the file Cairn adds a block to.
    std::fs::write(
        s.repo_dir().join("CLAUDE.md"),
        "# House rules\n\n\
         Run `cargo test --workspace` before every commit.\n\n\
         ## Conventions\n\n\
         Errors are values, never panics.\n",
    )
    .unwrap();
}

fn snapshot_of(paths: &[std::path::PathBuf]) -> Vec<(String, String)> {
    paths
        .iter()
        .map(|p| {
            (
                p.display().to_string(),
                std::fs::read_to_string(p).unwrap_or_default(),
            )
        })
        .collect()
}

fn memory_count(s: &Sandbox) -> usize {
    s.json(&["memory", "search", "pool"])["results"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0)
}

#[test]
fn disconnect_deletes_nothing_a_developer_recorded() {
    // FR-180: the project's memory is not part of the integration. A developer
    // who stops using an agent still has everything they learned with it.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    seed_configuration(&s);
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    // Real work, of every kind Cairn stores.
    let task = s.json(&[
        "task",
        "new",
        "--title",
        "Fix the pool",
        "--goal",
        "no contention",
    ]);
    let task_id = task["task"]["id"].as_str().unwrap().to_string();
    s.hook(
        "SessionStart",
        json!({ "session_id": "rm-1", "source": "startup" }),
    );
    s.settle_session_count(1);
    s.must(&["session", "start", "--key", "rm-1", "--task", &task_id]);
    s.hook(
        "PostToolUse",
        json!({
            "session_id": "rm-1",
            "tool_name": "Edit",
            "tool_input": { "file_path": "src/pool.rs" }
        }),
    );
    s.must(&[
        "memory",
        "add",
        "The pool is single-writer on purpose",
        "--type",
        "decision",
        "--scope",
        "project",
    ]);
    s.hook(
        "SessionEnd",
        json!({ "session_id": "rm-1", "reason": "clear" }),
    );
    s.settle("the closed session's handoff", |s| {
        s.cairn(&["--json", "status"])
            .stdout
            .contains("\"sessions_awaiting_handoff\": 0")
    });

    // `idle_seconds` is derived from the clock at read time, not stored, so
    // two reads a second apart differ by a second whatever disconnect did.
    // Comparing it would make this a test of how fast the runner is, and it
    // would fail claiming a session had been deleted.
    let stored = |v: &Value| -> Vec<Value> {
        v["sessions"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|mut s| {
                if let Some(o) = s.as_object_mut() {
                    o.remove("idle_seconds");
                }
                s
            })
            .collect()
    };

    let before = s.json(&["status"]);
    let sessions_before = stored(&s.json(&["session", "list"]));
    let tasks_before = s.json(&["task", "list"])["tasks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let observations_before = s.observation_ids();
    let handoff_before: Value = s.json(&["handoff", "show"])["handoff"].clone();
    assert!(!observations_before.is_empty(), "nothing was captured");
    assert!(handoff_before["next_step"].is_string());

    s.must(&["disconnect", "claude-code"]);

    assert_eq!(
        s.json(&["status"])["project"]["id"],
        before["project"]["id"],
        "the project was deleted or replaced"
    );
    assert_eq!(
        stored(&s.json(&["session", "list"])),
        sessions_before,
        "a session was deleted"
    );
    assert_eq!(
        s.json(&["task", "list"])["tasks"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
        tasks_before,
        "a task was deleted"
    );
    assert_eq!(
        s.observation_ids(),
        observations_before,
        "an observation was deleted"
    );
    assert_eq!(memory_count(&s), 1, "a memory was deleted");
    assert_eq!(
        s.json(&["handoff", "show"])["handoff"],
        handoff_before,
        "a handoff was deleted or rewritten"
    );
    assert_eq!(
        s.json(&["status"])["observation_count"],
        before["observation_count"]
    );
}

#[test]
fn disconnect_changes_nothing_in_a_touched_file_that_cairn_did_not_write() {
    // FR-179, SC-116: every unrelated setting in the files disconnect edits is
    // byte-identical afterwards.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    seed_configuration(&s);
    s.must(&["init"]);

    let touched = vec![
        s.fake_home().join(".claude.json"),
        s.repo_dir().join(".claude").join("settings.local.json"),
        s.repo_dir().join("CLAUDE.md"),
    ];
    let before = snapshot_of(&touched);

    s.must(&["connect", "claude-code", "--yes"]);
    // The install really did touch all three.
    for (path, original) in &before {
        let now = std::fs::read_to_string(path).unwrap_or_default();
        assert_ne!(&now, original, "connect did not touch {path}");
    }

    s.must(&["disconnect", "claude-code"]);

    for (path, original) in &before {
        let now = std::fs::read_to_string(path).unwrap_or_default();
        assert_eq!(
            &now, original,
            "disconnect did not restore {path} byte for byte"
        );
    }

    // Named individually, because "byte-identical" is easy to satisfy by
    // accident when the assertion is one big string compare that never ran.
    let claude = std::fs::read_to_string(s.fake_home().join(".claude.json")).unwrap();
    assert!(claude.contains("sk-ant-DEVELOPERS-OWN-KEY"), "a credential");
    assert!(claude.contains("internal-mcp"), "an unrelated MCP server");
    assert!(claude.contains("\"theme\""), "an unrelated setting");

    let settings =
        std::fs::read_to_string(s.repo_dir().join(".claude").join("settings.local.json")).unwrap();
    assert!(settings.contains("echo mine"), "the developer's own hook");
    assert!(settings.contains("guard.sh"), "an unrelated hook");
    assert!(settings.contains("Bash(cargo test:*)"), "a permission");
}

#[test]
fn an_instruction_file_survives_with_the_developers_content() {
    // US9 #8: only Cairn's block is removed, and the file it lived in stays.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    seed_configuration(&s);
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    let path = s.repo_dir().join("CLAUDE.md");
    let installed = std::fs::read_to_string(&path).unwrap();
    assert!(installed.contains("cairn:managed:begin"), "{installed}");

    s.must(&["disconnect", "claude-code"]);

    assert!(
        path.exists(),
        "the developer's instruction file was deleted"
    );
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        !after.contains("cairn:managed"),
        "the block was left behind"
    );
    assert!(after.contains("# House rules"));
    assert!(after.contains("cargo test --workspace"));
    assert!(after.contains("Errors are values, never panics."));
}

#[test]
fn disconnecting_one_agent_changes_no_other_agents_configuration() {
    // FR-179: two agents connected, one disconnected, and the other's own
    // files are untouched — including the ones the two do not share.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.install_agent("codex");
    std::fs::create_dir_all(s.repo_dir().join(".claude")).unwrap();
    seed_configuration(&s);
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);
    s.must(&["connect", "codex", "--yes"]);

    let codex_files = vec![
        s.fake_home().join(".codex").join("config.toml"),
        s.fake_home().join(".codex").join("hooks.json"),
    ];
    let before = snapshot_of(&codex_files);
    assert!(
        before.iter().any(|(_, body)| body.contains("cairn")),
        "Codex was not connected, so this proves nothing: {before:?}"
    );

    s.must(&["disconnect", "claude-code"]);

    for (path, original) in &before {
        assert_eq!(
            &std::fs::read_to_string(path).unwrap_or_default(),
            original,
            "disconnecting Claude Code changed {path}"
        );
    }
    // And Codex is still healthy, from its own point of view.
    let codex = s.json(&["doctor"])["agents"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .find(|a| a["agent"] == "codex")
        .expect("codex is reported");
    for r in codex["resources"].as_array().cloned().unwrap_or_default() {
        assert!(
            ["healthy", "shared", "installed_not_activated"]
                .contains(&r["condition"].as_str().unwrap_or_default()),
            "another agent's disconnect left Codex unhealthy: {r}"
        );
    }
}

#[test]
fn a_dry_run_disconnect_names_its_blast_radius_and_writes_nothing() {
    // FR-161: the developer sees what would go before anything does.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    seed_configuration(&s);
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    let before = s.checksum_tree();
    let out = s.json(&["disconnect", "claude-code", "--dry-run"]);
    assert_eq!(
        s.checksum_tree(),
        before,
        "a dry-run disconnect wrote something"
    );

    let untouched = out["untouched"].to_string();
    assert!(!untouched.is_empty() && untouched != "null", "{out}");
    assert!(
        untouched.contains("outside the cairn:managed markers"),
        "the plan did not name what it leaves alone: {untouched}"
    );
}

#[test]
fn an_instruction_file_cairn_created_goes_with_its_block() {
    // Found by walking the quickstart: disconnecting from a repository that
    // had no CLAUDE.md left a zero-byte CLAUDE.md behind. Cairn created that
    // file and nothing but Cairn's block was ever in it, so leaving it is
    // litter in someone's repository rather than restraint — while a file the
    // developer wrote keeps existing, with their content (FR-179).
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);
    assert!(
        !s.repo_dir().join("CLAUDE.md").exists(),
        "the fixture already has one"
    );

    s.must(&["connect", "claude-code", "--yes"]);
    assert!(
        s.repo_dir().join("CLAUDE.md").exists(),
        "connect wrote none"
    );

    s.must(&["disconnect", "claude-code"]);
    assert!(
        !s.repo_dir().join("CLAUDE.md").exists(),
        "an empty instruction file Cairn created was left behind"
    );
}
