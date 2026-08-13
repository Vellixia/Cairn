//! The integration command surface, end to end (US1, US2, US7, US9).
//!
//! These drive the real binaries against a real daemon and a real temporary
//! repository, with an isolated `HOME` so nothing touches the developer's own
//! agent configuration. No vendor binary is involved and no network is
//! reachable: the agents are "installed" by creating the directories their
//! detection looks for, which is exactly what makes this hermetic (FR-204,
//! SC-124).

use cairn_e2e::Sandbox;
use serde_json::{json, Value};

/// A sandbox with a fake home holding the agents named.
fn with_agents(agents: &[&str]) -> Sandbox {
    let s = Sandbox::new();
    for a in agents {
        s.install_agent(a);
    }
    s.must(&["init"]);
    s
}

fn doctor(s: &Sandbox, agent: &str) -> Value {
    // Doctor exits 1 when anything is actionable, which is not a failure of
    // the command, so the envelope is read directly.
    let out = s.cairn(&["--json", "doctor", agent]);
    serde_json::from_str::<Value>(&out.stdout)
        .unwrap_or_else(|e| panic!("doctor envelope: {e}\n{}", out.stdout))["data"]
        .clone()
}

fn resource<'a>(health: &'a Value, kind: &str) -> &'a Value {
    health["agents"][0]["resources"]
        .as_array()
        .expect("resources")
        .iter()
        .find(|r| r["kind"] == kind)
        .unwrap_or_else(|| panic!("no {kind} resource in {health}"))
}

#[test]
fn connect_installs_every_resource_and_doctor_reports_them_healthy() {
    // US1: from `cairn init` to a connected agent, with the contract and the
    // Skill installed.
    let s = with_agents(&["claude-code"]);
    s.must(&["connect", "claude-code", "--yes"]);

    let health = doctor(&s, "claude-code");
    for kind in ["mcp", "lifecycle", "instructions", "skill"] {
        assert_eq!(
            resource(&health, kind)["condition"],
            "healthy",
            "{kind} is not healthy: {health}"
        );
    }

    // Exactly one managed block, and the Skill is really on disk.
    let claude_md = std::fs::read_to_string(s.repo_dir().join("CLAUDE.md")).expect("CLAUDE.md");
    assert_eq!(claude_md.matches("cairn:managed:begin").count(), 1);
    let skill = resource(&health, "skill")["location"]
        .as_str()
        .expect("skill location");
    assert!(std::path::Path::new(skill).join("SKILL.md").exists());
}

#[test]
fn a_second_connect_reports_unchanged_and_writes_nothing() {
    // SC-102: zero configuration changes on the second run.
    let s = with_agents(&["claude-code"]);
    s.must(&["connect", "claude-code", "--yes"]);

    let before = s.checksum_tree();
    let out = s.must(&["connect", "claude-code", "--yes"]);
    assert!(
        out.stdout.contains("unchanged"),
        "a second connect did not report unchanged: {}",
        out.stdout
    );
    assert_eq!(
        before,
        s.checksum_tree(),
        "a second connect wrote something"
    );
}

#[test]
fn dry_run_writes_nothing_at_all() {
    // SC-118, through the real command rather than the library.
    let s = with_agents(&["claude-code"]);
    let before = s.checksum_tree();
    let out = s.must(&["connect", "claude-code", "--dry-run"]);
    assert!(out.stdout.contains("dry run"));
    assert!(out.stdout.contains("ADD"));
    assert_eq!(
        before,
        s.checksum_tree(),
        "--dry-run modified the filesystem"
    );
}

#[test]
fn a_non_interactive_run_without_yes_refuses_and_writes_nothing() {
    // FR-164: applying without confirmation requires an explicit opt-in.
    let s = with_agents(&["claude-code"]);
    let before = s.checksum_tree();
    let e = s.json_err(&["connect", "claude-code"]);
    assert_eq!(e["code"], "confirmation_required");
    assert_eq!(before, s.checksum_tree());
}

#[test]
fn a_feature_001_installation_is_adopted_in_place_and_never_duplicated() {
    // SC-103, US2: exactly one Cairn entry per registered event, exactly one
    // Cairn MCP entry, every unrelated entry byte-identical, and the adopted
    // resources stay at the scope they are already at (FR-217).
    let s = with_agents(&["claude-code"]);

    // Reconstruct a Feature 001 installation: six hook entries and the
    // project MCP entry, at committed project scope, alongside the
    // developer's own configuration.
    let legacy_hooks = r#"{
  "model": "opus",
  "hooks": {
    "SessionStart": [{ "hooks": [{ "type": "command", "command": "cairn hook SessionStart" }] }],
    "PostToolUse": [
      { "matcher": "*", "hooks": [{ "type": "command", "command": "cairn hook PostToolUse" }] },
      { "matcher": "Bash", "hooks": [{ "type": "command", "command": "audit.sh" }] }
    ],
    "PostToolUseFailure": [{ "matcher": "*", "hooks": [{ "type": "command", "command": "cairn hook PostToolUseFailure" }] }],
    "PreCompact": [{ "hooks": [{ "type": "command", "command": "cairn hook PreCompact" }] }],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "cairn hook Stop" }] },
      { "hooks": [{ "type": "command", "command": "echo 'run cairn hook first' && make lint" }] }
    ],
    "SessionEnd": [{ "hooks": [{ "type": "command", "command": "cairn hook SessionEnd" }] }]
  }
}
"#;
    let legacy_mcp = r#"{
  "mcpServers": {
    "cairn": { "command": "cairn", "args": ["mcp"] },
    "other": { "command": "other-server", "env": { "TOKEN": "sk-not-a-real-secret-000" } }
  }
}
"#;
    s.write_file(".claude/settings.json", legacy_hooks);
    s.write_file(".mcp.json", legacy_mcp);

    s.must(&["connect", "claude-code", "--yes"]);

    // Exactly one Cairn entry per registered event, and the developer's own
    // entries untouched.
    let after = std::fs::read_to_string(s.repo_dir().join(".claude/settings.json")).unwrap();
    let parsed: Value = serde_json::from_str(&after).expect("settings still parse");
    for event in [
        "SessionStart",
        "PostToolUse",
        "PostToolUseFailure",
        "PreCompact",
        "Stop",
        "SessionEnd",
    ] {
        let entries = parsed["hooks"][event].as_array().unwrap_or_else(|| {
            panic!("{event} lost its registrations: {after}");
        });
        let ours = entries
            .iter()
            .filter(|e| e["hooks"][0]["command"].as_str() == Some(&format!("cairn hook {event}")))
            .count();
        assert_eq!(ours, 1, "{event} has {ours} Cairn entries");
    }
    assert!(after.contains("audit.sh"), "a developer hook was lost");
    assert!(
        after.contains("run cairn hook first"),
        "a developer command mentioning `cairn hook` was claimed as Cairn's"
    );

    // Exactly one Cairn MCP entry, and the unrelated server byte-identical.
    let mcp = std::fs::read_to_string(s.repo_dir().join(".mcp.json")).unwrap();
    let mcp_parsed: Value = serde_json::from_str(&mcp).expect(".mcp.json still parses");
    let servers = mcp_parsed["mcpServers"].as_object().expect("mcpServers");
    assert_eq!(
        servers.keys().filter(|k| *k == "cairn").count(),
        1,
        "more than one Cairn MCP entry: {mcp}"
    );
    assert!(
        mcp.contains("sk-not-a-real-secret-000"),
        "an unrelated server changed"
    );

    // Adopted in place: the recorded scope is where they already were, not
    // Feature 002's defaults (FR-217).
    let health = doctor(&s, "claude-code");
    assert_eq!(resource(&health, "lifecycle")["scope"], "project_shared");
    assert_eq!(resource(&health, "mcp")["scope"], "project_shared");
    // And no second copy at the Feature 002 default location.
    assert!(!s.repo_dir().join(".claude/settings.local.json").exists());
}

#[test]
fn one_ordinary_session_establishes_the_evidence_that_grants_full() {
    // FR-245, SC-138: before a session, the level is below FULL and says what
    // it is waiting for; one ordinary session establishes it.
    let s = with_agents(&["claude-code"]);
    s.must(&["connect", "claude-code", "--yes"]);

    let before = doctor(&s, "claude-code");
    assert_eq!(before["agents"][0]["level"], "mcp_plus");
    assert!(
        !before["agents"][0]["awaited_behaviors"]
            .as_array()
            .unwrap()
            .is_empty(),
        "a level below FULL must name what it awaits"
    );

    s.hook(
        "SessionStart",
        serde_json::json!({ "session_id": "full-1", "source": "startup" }),
    );
    s.hook(
        "PostToolUse",
        serde_json::json!({
            "session_id": "full-1",
            "tool_name": "Edit",
            "tool_input": { "file_path": "a.rs" }
        }),
    );
    s.hook("Stop", serde_json::json!({ "session_id": "full-1" }));
    s.hook(
        "SessionEnd",
        serde_json::json!({ "session_id": "full-1", "reason": "clear" }),
    );
    s.settle("the boundary's handoff", |s| {
        s.cairn(&["--json", "status"])
            .stdout
            .contains("\"sessions_awaiting_handoff\": 0")
    });

    let after = doctor(&s, "claude-code");
    assert_eq!(
        after["agents"][0]["level"], "full",
        "one ordinary session did not establish FULL: {after}"
    );
    assert_eq!(after["agents"][0]["completion_guarantee"], "demonstrated");
    assert!(after["agents"][0]["awaited_behaviors"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn a_shared_resource_survives_the_first_disconnect_and_goes_with_the_last() {
    // SC-137, FR-243: the failure the old `satisfied_by` design would cause.
    let s = with_agents(&["codex", "opencode"]);
    s.must(&["connect", "codex", "--yes"]);
    s.must(&["connect", "opencode", "--yes"]);

    let agents_md = s.repo_dir().join("AGENTS.md");
    assert!(agents_md.exists());

    // Doctor names both consumers before anyone disconnects.
    let health = doctor(&s, "opencode");
    let instructions = resource(&health, "instructions");
    assert_eq!(instructions["condition"], "shared");
    let serves = instructions["serves"].as_array().expect("serves list");
    assert_eq!(serves.len(), 2, "a shared resource must name its consumers");

    let out = s.cairn(&["disconnect", "codex"]);
    assert!(out.ok(), "{}", out.stderr);
    assert!(
        out.stdout.contains("unbound"),
        "the shared block was not reported as unbound: {}",
        out.stdout
    );
    assert!(
        std::fs::read_to_string(&agents_md)
            .unwrap()
            .contains("cairn:managed:begin"),
        "disconnecting Codex removed the block OpenCode still needs"
    );
    assert_eq!(
        resource(&doctor(&s, "opencode"), "instructions")["condition"],
        "healthy",
        "OpenCode was left unhealthy by another agent's disconnect"
    );

    // The last consumer's disconnect is what removes it.
    assert!(s.cairn(&["disconnect", "opencode"]).ok());
    assert!(
        !std::fs::read_to_string(&agents_md)
            .unwrap()
            .contains("cairn:managed:begin"),
        "the block outlived its last binding"
    );
}

#[test]
fn disconnect_leaves_every_unrelated_setting_and_all_memory_intact() {
    // SC-116, FR-179, FR-180.
    let s = with_agents(&["claude-code"]);
    s.write_file(
        ".claude/settings.local.json",
        "{\n  \"hooks\": { \"Stop\": [{ \"hooks\": [{ \"type\": \"command\", \"command\": \"make lint\" }] }] }\n}\n",
    );
    s.write_file(
        "CLAUDE.md",
        "# Project\n\nThe developer's own instructions.\n",
    );
    s.must(&["connect", "claude-code", "--yes"]);

    s.hook(
        "SessionStart",
        serde_json::json!({ "session_id": "keep", "source": "startup" }),
    );
    s.must(&[
        "memory",
        "add",
        "A durable decision that must survive disconnect",
        "--type",
        "decision",
        "--scope",
        "project",
    ]);

    assert!(s.cairn(&["disconnect", "claude-code"]).ok());

    let settings =
        std::fs::read_to_string(s.repo_dir().join(".claude/settings.local.json")).unwrap();
    assert!(
        settings.contains("make lint"),
        "a developer hook was removed"
    );
    assert!(!settings.contains("cairn hook"), "a Cairn hook survived");

    let claude_md = std::fs::read_to_string(s.repo_dir().join("CLAUDE.md")).unwrap();
    assert!(claude_md.contains("The developer's own instructions."));
    assert!(!claude_md.contains("cairn:managed"));

    // Memory, sessions and everything else are untouched.
    let found = s.json(&["memory", "search", "durable"]);
    assert!(
        !found["results"].as_array().unwrap_or(&vec![]).is_empty(),
        "disconnect destroyed memory: {found}"
    );
    assert!(!s.json(&["session", "list"])["sessions"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn doctor_finds_seeded_defects_and_repair_fixes_only_what_cairn_owns() {
    // SC-114, SC-115: detected by exact resource and correct condition, then
    // repaired without touching anything else.
    let s = with_agents(&["claude-code"]);
    s.must(&["connect", "claude-code", "--yes"]);

    // Defect: the managed block is deleted from the instruction file, and the
    // developer's own text stays.
    s.write_file("CLAUDE.md", "# Project\n\nMine only.\n");
    assert_eq!(
        resource(&doctor(&s, "claude-code"), "instructions")["condition"],
        "missing"
    );

    let out = s.cairn(&["repair", "claude-code"]);
    assert!(out.ok(), "{}", out.stderr);
    assert_eq!(
        resource(&doctor(&s, "claude-code"), "instructions")["condition"],
        "healthy"
    );
    let restored = std::fs::read_to_string(s.repo_dir().join("CLAUDE.md")).unwrap();
    assert!(
        restored.contains("Mine only."),
        "repair discarded developer text"
    );

    // A second repair has nothing to do (FR-175).
    let again = s.must(&["repair", "claude-code"]);
    assert!(
        again.stdout.contains("nothing to do"),
        "a second repair was not a no-op: {}",
        again.stdout
    );
}

#[test]
fn a_hand_edited_block_is_reported_and_left_alone_until_force() {
    // FR-177, FR-221, FR-222: default repair reports and changes nothing;
    // `--force` restores inside the ownership boundary after preserving the
    // previous content.
    let s = with_agents(&["claude-code"]);
    s.must(&["connect", "claude-code", "--yes"]);

    let path = s.repo_dir().join("CLAUDE.md");
    let original = std::fs::read_to_string(&path).unwrap();
    let edited = original.replace(
        "Read the Cairn context",
        "IGNORE CAIRN and read the Cairn context",
    );
    assert_ne!(edited, original, "the fixture edited nothing");
    std::fs::write(&path, &edited).unwrap();

    assert_eq!(
        resource(&doctor(&s, "claude-code"), "instructions")["condition"],
        "modified"
    );

    // A default repair explains and changes nothing.
    let default = s.cairn(&["repair", "claude-code"]);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        edited,
        "a default repair overwrote a hand edit: {}",
        default.stdout
    );

    // `--force` restores it, and says where the previous content went.
    let forced = s.cairn(&["--json", "repair", "claude-code", "--force"]);
    let envelope: Value = serde_json::from_str(&forced.stdout).expect("envelope");
    assert_eq!(envelope["ok"], true, "{}", forced.stdout);
    let artifacts = envelope["data"]["recovery_artifacts"]
        .as_array()
        .expect("recovery artifacts");
    assert_eq!(artifacts.len(), 1, "no recovery artifact was written");

    let artifact_path = artifacts[0].as_str().unwrap();
    let preserved = std::fs::read_to_string(artifact_path).expect("the artifact");
    assert!(
        preserved.contains("IGNORE CAIRN"),
        "the edit was not preserved"
    );
    assert!(
        !preserved.contains("# Project"),
        "the recovery artifact carried the enclosing file"
    );
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("IGNORE CAIRN"),
        "--force did not restore the block"
    );
}

#[test]
fn a_damaged_marker_blocks_even_under_force() {
    // FR-221: forcing past one would mean guessing which text was Cairn's.
    let s = with_agents(&["claude-code"]);
    s.must(&["connect", "claude-code", "--yes"]);
    let path = s.repo_dir().join("CLAUDE.md");
    let text = std::fs::read_to_string(&path).unwrap();
    let damaged = text.replace("<!-- cairn:managed:end id=agent-contract -->", "");
    std::fs::write(&path, &damaged).unwrap();

    assert_eq!(
        resource(&doctor(&s, "claude-code"), "instructions")["condition"],
        "damaged_markers"
    );
    let forced = s.cairn(&["repair", "claude-code", "--force"]);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        damaged,
        "--force wrote past a damaged marker: {}",
        forced.stdout
    );
}

#[test]
fn a_malformed_file_is_reported_and_never_rewritten() {
    // FR-137, US7 #3.
    let s = with_agents(&["claude-code"]);
    s.write_file(".claude/settings.local.json", "{ \"hooks\": ");
    let before = std::fs::read_to_string(s.repo_dir().join(".claude/settings.local.json")).unwrap();

    let e = s.json_err(&["connect", "claude-code", "--yes"]);
    assert_eq!(e["code"], "malformed_config", "{e}");
    assert_eq!(
        std::fs::read_to_string(s.repo_dir().join(".claude/settings.local.json")).unwrap(),
        before,
        "a malformed file was rewritten"
    );
}

#[test]
fn no_credential_appears_in_any_diagnostic_output() {
    // SC-119, FR-171: seeded credentials in the files Cairn inspects appear in
    // none of its output.
    let s = with_agents(&["claude-code"]);
    s.write_file(
        ".mcp.json",
        "{\n  \"mcpServers\": {\n    \"other\": { \"command\": \"x\", \"env\": { \"TOKEN\": \"sk-not-a-real-secret-000\" } }\n  }\n}\n",
    );
    s.must(&["connect", "claude-code", "--yes"]);

    for args in [
        vec!["--json", "doctor"],
        vec!["--json", "agents"],
        vec!["--json", "connect", "claude-code", "--dry-run"],
        vec!["--json", "repair", "claude-code", "--dry-run"],
    ] {
        let out = s.cairn(&args);
        assert!(
            !out.stdout.contains("sk-not-a-real-secret-000")
                && !out.stderr.contains("sk-not-a-real-secret-000"),
            "{args:?} leaked a credential"
        );
    }
}

#[test]
fn the_exported_mcp_configuration_is_deterministic_and_writes_nothing() {
    // FR-131, SC-135, US8 #4.
    let s = with_agents(&["claude-code"]);
    let before = s.checksum_tree();
    let a = s.must(&["integration", "export", "mcp"]);
    let b = s.must(&["integration", "export", "mcp"]);
    assert_eq!(a.stdout, b.stdout);
    assert!(a.stdout.contains("mcpServers"));
    assert!(a.stdout.contains("cairn"));
    assert_eq!(before, s.checksum_tree(), "export wrote something");

    let codex = s.must(&["integration", "export", "mcp", "--agent", "codex"]);
    assert!(codex.stdout.contains("[mcp_servers.cairn]"));
}

#[test]
fn agents_reports_detection_without_touching_anything() {
    // FR-104, FR-105.
    let s = with_agents(&["claude-code", "codex"]);
    let before = s.checksum_tree();
    let out = s.must(&["agents"]);
    assert!(out.stdout.contains("claude-code"));
    assert!(out.stdout.contains("codex"));
    assert!(
        out.stdout.contains("not connected"),
        "an unconnected agent must say so rather than claim a level: {}",
        out.stdout
    );
    assert_eq!(before, s.checksum_tree(), "detection modified something");
}

#[test]
fn repair_restores_what_cairn_owns_and_connects_nothing_new() {
    // Found by walking the quickstart: `cairn repair` reached every *detected*
    // agent rather than every *connected* one, so a developer who had
    // connected Claude Code and then ran `cairn repair` silently got Codex and
    // OpenCode installed too — an opt-in FR-164 requires, performed without
    // one. It also never converged: each run discovered more to install.
    let s = Sandbox::new();
    for a in ["claude-code", "codex", "opencode"] {
        s.install_agent(a);
    }
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    // Break the one thing Cairn does own.
    let hooks = s.repo_dir().join(".claude").join("settings.local.json");
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&hooks).unwrap()).unwrap();
    value["hooks"] = json!({});
    std::fs::write(&hooks, value.to_string()).unwrap();

    let before = s.checksum_tree();
    s.must(&["repair"]);

    // The broken resource is back.
    let after = std::fs::read_to_string(&hooks).unwrap();
    assert!(
        after.contains("cairn hook"),
        "repair did not restore: {after}"
    );

    // And nothing was installed for an agent nobody connected.
    for path in [
        s.fake_home().join(".codex").join("config.toml"),
        s.fake_home().join(".codex").join("hooks.json"),
        s.fake_home()
            .join(".config")
            .join("opencode")
            .join("plugin")
            .join("cairn.js"),
        s.repo_dir().join("AGENTS.md"),
    ] {
        assert!(
            !path.exists(),
            "repair connected an agent the developer never asked for: {}",
            path.display()
        );
    }

    // Repair converges: a second run has nothing to do.
    let out = s.must(&["repair"]);
    assert!(
        out.stdout.contains("nothing to do"),
        "a second repair still had work: {}",
        out.stdout
    );

    // Naming an unconnected agent explicitly still repairs it, because that is
    // an explicit request.
    s.must(&["connect", "codex", "--yes"]);
    let _ = before;
    s.must(&["repair", "codex"]);
}
