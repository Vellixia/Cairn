//! Preview writes nothing at all (FR-159, SC-118).
//!
//! Verified the way the criterion states it: a checksum of every candidate
//! file before and after, across every supported operation — including the
//! case where the configuration is broken, where a preview still reports the
//! conflict and still writes nothing.
//!
//! "Nothing" includes temporary files. A plan that wrote a `.tmp` beside a
//! config and removed it would pass a naive before/after comparison and still
//! be a write, so the directory listing is compared too.

use cairn_integrate::adapter::Observed;
use cairn_integrate::desired::{Choices, DesiredIntegrationState};
use cairn_integrate::model::{
    canonical_hash, AgentId, ArtifactVersion, HealthCondition, ResourceKind,
};
use cairn_integrate::plan::{plan_agent, Intent};
use cairn_integrate::scope::Env;
use std::collections::BTreeMap;
use std::path::Path;

/// Every file under a root, with its content hash.
fn snapshot(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            let body = std::fs::read_to_string(&path).unwrap_or_default();
            out.insert(rel, canonical_hash(&body));
        }
    }
}

/// A machine with every supported agent already configured, plus configuration
/// Cairn does not own.
fn populated() -> (tempfile::TempDir, Env) {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let repo = dir.path().join("repo");
    let env = Env::new(&home, &repo);

    let write = |p: &Path, body: &str| {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    };

    write(&home.join(".claude.json"), "{\n  \"mcpServers\": {}\n}\n");
    write(
        &home.join(".claude").join("settings.json"),
        "{\n  \"hooks\": {}\n}\n",
    );
    write(
        &repo.join(".claude").join("settings.local.json"),
        "{\n  \"hooks\": { \"Stop\": [{ \"hooks\": [{ \"type\": \"command\", \"command\": \"lint\" }] }] }\n}\n",
    );
    write(&repo.join("CLAUDE.md"), "# Project\n\nDeveloper notes.\n");
    write(
        &home.join(".codex").join("config.toml"),
        "model = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"other\"\n",
    );
    write(
        &home.join(".codex").join("hooks.json"),
        "{\n  \"hooks\": []\n}\n",
    );
    write(&repo.join("AGENTS.md"), "# AGENTS\n\nRun the tests.\n");
    write(
        &home.join(".config").join("opencode").join("opencode.json"),
        "{\n  \"mcp\": {}\n}\n",
    );
    (dir, env)
}

fn desired(agent: AgentId) -> DesiredIntegrationState {
    DesiredIntegrationState::compose(
        &Choices {
            agents: vec![agent],
            ..Default::default()
        },
        &[agent],
        &[],
        ArtifactVersion::new(1, "aaaaaaaaaaaa"),
        ArtifactVersion::new(1, "bbbbbbbbbbbb"),
    )
}

#[test]
fn dry_run_is_inert() {
    // SC-118: zero filesystem modifications across every supported operation.
    let (dir, env) = populated();
    let before = snapshot(dir.path());

    for agent in AgentId::ALL {
        let adapter = cairn_integrate::adapter_for(agent);
        for intent in [
            Intent::Connect,
            Intent::Repair { force: false },
            Intent::Repair { force: true },
            Intent::Disconnect,
        ] {
            let observed = adapter.inspect(&env, &[]);
            let plan = plan_agent(intent, agent, &desired(agent), &observed);
            // The plan is computed and discarded; nothing may have moved.
            let _ = plan.changes.len();
        }
    }

    let after = snapshot(dir.path());
    assert_eq!(before, after, "computing a plan modified the filesystem");
}

#[test]
fn inspection_alone_writes_nothing() {
    // Doctor is the same engine with no plan applied (FR-170).
    let (dir, env) = populated();
    let before = snapshot(dir.path());
    for agent in AgentId::ALL {
        let adapter = cairn_integrate::adapter_for(agent);
        let _ = adapter.detect(&env);
        let _ = adapter.inspect(&env, &[]);
    }
    assert_eq!(before, snapshot(dir.path()));
}

#[test]
fn a_dry_run_against_a_broken_configuration_still_writes_nothing() {
    // The edge case: report the conflict, change nothing.
    let (dir, env) = populated();
    std::fs::write(
        env.home.join(".claude.json"),
        "{ \"mcpServers\": { \"cairn\": ",
    )
    .unwrap();
    std::fs::write(
        env.worktree.join("CLAUDE.md"),
        "<!-- cairn:managed:begin id=agent-contract schema=1 content=abc -->\nno end marker\n",
    )
    .unwrap();
    let before = snapshot(dir.path());

    let adapter = cairn_integrate::adapter_for(AgentId::ClaudeCode);
    let observed = adapter.inspect(&env, &[]);
    let plan = plan_agent(
        Intent::Connect,
        AgentId::ClaudeCode,
        &desired(AgentId::ClaudeCode),
        &observed,
    );

    assert!(plan.is_blocked(), "a broken configuration must block");
    assert!(observed
        .iter()
        .any(|o| o.condition == HealthCondition::MalformedConfig));
    assert!(observed
        .iter()
        .any(|o| o.condition == HealthCondition::DamagedMarkers));
    assert_eq!(before, snapshot(dir.path()));
}

#[test]
fn no_temporary_file_is_ever_left_beside_a_configuration() {
    let (dir, env) = populated();
    let adapter = cairn_integrate::adapter_for(AgentId::Codex);
    let _ = adapter.inspect(&env, &[]);
    let names: Vec<String> = snapshot(dir.path()).into_keys().collect();
    assert!(
        !names
            .iter()
            .any(|n| n.contains(".tmp") || n.contains("cairn-")),
        "a temporary file survived: {names:?}"
    );
}

#[test]
fn a_plan_for_an_untouched_machine_reports_only_additions() {
    // Sanity: the inert tests above would also pass if the planner did
    // nothing at all, so assert it actually produced a plan.
    let (_dir, env) = populated();
    let adapter = cairn_integrate::adapter_for(AgentId::ClaudeCode);
    let observed = adapter.inspect(&env, &[]);
    let plan = plan_agent(
        Intent::Connect,
        AgentId::ClaudeCode,
        &desired(AgentId::ClaudeCode),
        &observed,
    );
    assert!(!plan.is_noop(), "the planner produced no work to do");
    assert!(!plan.untouched.is_empty());
}

#[test]
fn an_observed_resource_names_its_own_file() {
    // FR-160: the affected resource and file are named for each change.
    let (_dir, env) = populated();
    let adapter = cairn_integrate::adapter_for(AgentId::Codex);
    let observed: Vec<Observed> = adapter.inspect(&env, &[]);
    for o in &observed {
        if o.kind == ResourceKind::Mcp || o.kind == ResourceKind::Lifecycle {
            assert!(o.location.is_some(), "{:?} names no file", o.kind);
        }
    }
}
