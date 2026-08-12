//! Turning a planned change into actual bytes (FR-151's apply step).
//!
//! `plan` decides *what* should change; this decides *how*, per agent and
//! resource kind, and hands the result to `apply` to be written atomically and
//! verified. Keeping the two apart is what lets `--dry-run` compute a complete
//! plan without a single write (SC-118): materializing is pure, and only
//! `commit` touches the filesystem.
//!
//! Two bits about Cairn's own edit travel with each materialization, because
//! removal needs them to put a file back exactly as it was and Cairn is
//! forbidden from keeping a copy of the file itself (FR-156, FR-238):
//! whether the container it wrote into was on one line, and whether Cairn
//! created that container at all.

use crate::agents::{claude_code, codex, opencode};
use crate::apply::{self, ApplyError, TreeWrite};
use crate::edit::{json, markdown, toml, Change, EditError};
use crate::markers::CONTRACT_ID;
use crate::model::{canonical_hash, AgentId, ArtifactVersion, InstallationScope, ResourceKind};
use crate::plan::RecordedInstall;
use crate::scope::{self, Env};
use crate::{render, revision};
use std::path::PathBuf;

/// What writing this resource actually means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Replace one file's contents atomically.
    File { contents: String },
    /// Write a directory Cairn generated in full.
    Tree { files: Vec<(String, String)> },
    /// Remove a directory Cairn generated in full.
    RemoveTree,
    /// Already exactly right; writing would change nothing (FR-157).
    Unchanged,
}

/// One resource, resolved to bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Materialized {
    pub agent: AgentId,
    pub kind: ResourceKind,
    pub scope: InstallationScope,
    pub location: PathBuf,
    pub op: Op,
    /// Canonical hash of exactly what Cairn owns here — the entry, the block,
    /// or the generated file. Never a hash of the enclosing file.
    pub content_hash: Option<String>,
    pub artifact: Option<ArtifactVersion>,
    pub container_single_line: bool,
    pub created_container: bool,
}

impl Materialized {
    pub fn writes(&self) -> bool {
        !matches!(self.op, Op::Unchanged)
    }
}

/// The Skill tree as files, from the embedded canonical source.
///
/// Verbatim, not canonicalized: the installed `SKILL.md` carries its real
/// revision so the agents' frontmatter reads correctly and doctor can compare
/// what a Skill claims against what its files hash to.
pub fn skill_files() -> Vec<(String, String)> {
    revision::embedded_files_verbatim()
        .into_iter()
        .map(|f| (f.path, f.content))
        .collect()
}

/// The keys Cairn's MCP entry lives under, for one agent.
fn mcp_keys(agent: AgentId) -> &'static [&'static str] {
    match agent {
        AgentId::Opencode => &["mcp", crate::MCP_SERVER_NAME],
        _ => &["mcpServers", crate::MCP_SERVER_NAME],
    }
}

/// Resolve one resource to the bytes that install it.
///
/// Pure: reads the current file, computes the replacement, writes nothing.
pub fn materialize_install(
    env: &Env,
    agent: AgentId,
    kind: ResourceKind,
    scope: InstallationScope,
) -> Result<Materialized, EditError> {
    let location =
        scope::location(env, agent, kind, scope).ok_or_else(|| EditError::UnexpectedShape {
            path: format!("{agent}/{kind}"),
            detail: "this agent installs no resource of that kind".into(),
        })?;
    let display = location.display().to_string();
    let current = std::fs::read_to_string(&location).unwrap_or_default();

    let mut m = Materialized {
        agent,
        kind,
        scope,
        location: location.clone(),
        op: Op::Unchanged,
        content_hash: None,
        artifact: None,
        container_single_line: false,
        created_container: false,
    };

    match (agent, kind) {
        (_, ResourceKind::Mcp) if agent == AgentId::Codex => {
            let entry = crate::mcp_entry();
            m.content_hash = Some(canonical_hash(&entry.to_string()));
            m.created_container = toml::get(&display, &current, &["mcp_servers"])?.is_none();
            m.op = write_or_unchanged(toml::upsert(
                &display,
                &current,
                &["mcp_servers", crate::MCP_SERVER_NAME],
                &entry,
            )?);
        }
        (_, ResourceKind::Mcp) => {
            let keys = mcp_keys(agent);
            let entry = crate::mcp_entry();
            m.content_hash = Some(canonical_hash(&entry.to_string()));
            m.container_single_line = json::container_is_single_line(&display, &current, keys);
            m.created_container = json::get(&display, &current, &keys[..keys.len() - 1])?.is_none();
            m.op = write_or_unchanged(json::upsert(&display, &current, keys, &entry)?);
        }

        (AgentId::ClaudeCode, ResourceKind::Lifecycle) => {
            let mut text = current.clone();
            let mut changed = false;
            for ev in claude_code::EVENTS {
                m.created_container |= json::get(&display, &current, &["hooks", ev])?.is_none();
                m.container_single_line |=
                    json::value_is_single_line(&display, &current, &["hooks", ev]);
                let entry = claude_code::hook_entry(ev);
                let is_ours = |v: &serde_json::Value| claude_code::is_cairn_hook_entry(v, ev);
                if let Change::Written(s) =
                    json::upsert_array_entry(&display, &text, &["hooks", ev], &is_ours, &entry)?
                {
                    text = s;
                    changed = true;
                }
            }
            m.content_hash = Some(canonical_hash(&registration_digest_source(
                claude_code::EVENTS,
                claude_code::hook_entry,
            )));
            m.op = if changed {
                Op::File { contents: text }
            } else {
                Op::Unchanged
            };
        }

        (AgentId::Codex, ResourceKind::Lifecycle) => {
            let mut text = current.clone();
            let mut changed = false;
            m.created_container = json::get(&display, &current, &["hooks"])?.is_none();
            m.container_single_line = json::value_is_single_line(&display, &current, &["hooks"]);
            for ev in codex::EVENTS {
                let entry = codex::hook_entry(ev);
                let is_ours = |v: &serde_json::Value| codex::is_cairn_hook_entry(v, ev);
                if let Change::Written(s) =
                    json::upsert_array_entry(&display, &text, &["hooks"], &is_ours, &entry)?
                {
                    text = s;
                    changed = true;
                }
            }
            m.content_hash = Some(canonical_hash(&registration_digest_source(
                codex::EVENTS,
                codex::hook_entry,
            )));
            m.op = if changed {
                Op::File { contents: text }
            } else {
                Op::Unchanged
            };
        }

        // A file drop, not a config edit: OpenCode auto-discovers plugins in
        // every config directory, so nothing in `opencode.json` is touched
        // (D32). Cairn generates the whole file and owns every byte.
        (AgentId::Opencode, ResourceKind::Lifecycle) => {
            m.content_hash = Some(canonical_hash(opencode::PLUGIN_SOURCE));
            m.op = if canonical_hash(&current) == canonical_hash(opencode::PLUGIN_SOURCE) {
                Op::Unchanged
            } else {
                Op::File {
                    contents: opencode::PLUGIN_SOURCE.to_string(),
                }
            };
        }

        (_, ResourceKind::Instructions) => {
            let contract = render::Contract::canonical();
            let body = contract.block_body();
            m.artifact = Some(contract.version());
            m.content_hash = Some(canonical_hash(&body));
            m.op = write_or_unchanged(markdown::upsert(
                &display,
                &current,
                CONTRACT_ID,
                contract.schema,
                &body,
            )?);
        }

        (_, ResourceKind::Skill) => {
            let files = skill_files();
            m.artifact = Some(ArtifactVersion::new(
                revision::embedded_schema(),
                revision::embedded_revision(),
            ));
            m.content_hash = Some(revision::embedded_revision());
            let installed_matches = revision::installed_revision(&location)
                .map(|i| i.matches(revision::embedded_schema(), &revision::embedded_revision()))
                .unwrap_or(false);
            m.op = if installed_matches {
                Op::Unchanged
            } else {
                Op::Tree { files }
            };
        }

        (AgentId::GenericMcp, _) => {
            return Err(EditError::UnexpectedShape {
                path: display,
                detail: "Cairn writes nothing for a generic MCP client".into(),
            })
        }
    }
    Ok(m)
}

/// Resolve one resource to the bytes that remove it.
///
/// Uses the two recorded bits about Cairn's own edit so a minified file comes
/// back exactly as it was, and so a container the developer wrote is never
/// pruned (FR-178, FR-180).
pub fn materialize_removal(
    env: &Env,
    agent: AgentId,
    kind: ResourceKind,
    scope: InstallationScope,
    recorded: Option<&RecordedInstall>,
) -> Result<Materialized, EditError> {
    let location = recorded
        .map(|r| r.location.clone())
        .or_else(|| scope::location(env, agent, kind, scope))
        .ok_or_else(|| EditError::UnexpectedShape {
            path: format!("{agent}/{kind}"),
            detail: "no recorded location to remove".into(),
        })?;
    let display = location.display().to_string();
    let current = std::fs::read_to_string(&location).unwrap_or_default();

    let single_line = recorded.map(|r| r.container_single_line).unwrap_or(false);
    let created = recorded.map(|r| r.created_container).unwrap_or(false);

    let mut m = Materialized {
        agent,
        kind,
        scope,
        location: location.clone(),
        op: Op::Unchanged,
        content_hash: None,
        artifact: None,
        container_single_line: single_line,
        created_container: created,
    };

    match (agent, kind) {
        (AgentId::Codex, ResourceKind::Mcp) => {
            m.op = write_or_unchanged(toml::remove(
                &display,
                &current,
                &["mcp_servers", crate::MCP_SERVER_NAME],
            )?);
        }
        (_, ResourceKind::Mcp) => {
            let keys = mcp_keys(agent);
            let removed = if single_line {
                json::remove_collapsing(&display, &current, keys)?
            } else {
                json::remove(&display, &current, keys)?
            };
            m.op = write_or_unchanged(removed);
        }

        (AgentId::ClaudeCode, ResourceKind::Lifecycle) => {
            let mut text = current.clone();
            let mut changed = false;
            for ev in claude_code::EVENTS {
                let is_ours = |v: &serde_json::Value| claude_code::is_cairn_hook_entry(v, ev);
                if let Change::Written(s) =
                    json::remove_array_entries(&display, &text, &["hooks", ev], &is_ours, created)?
                {
                    text = s;
                    changed = true;
                }
            }
            if changed && single_line {
                text = json::collapse_path(&text, &["hooks"]);
            }
            m.op = if changed {
                Op::File { contents: text }
            } else {
                Op::Unchanged
            };
        }

        (AgentId::Codex, ResourceKind::Lifecycle) => {
            let mut text = current.clone();
            let mut changed = false;
            for ev in codex::EVENTS {
                let is_ours = |v: &serde_json::Value| codex::is_cairn_hook_entry(v, ev);
                if let Change::Written(s) =
                    json::remove_array_entries(&display, &text, &["hooks"], &is_ours, created)?
                {
                    text = s;
                    changed = true;
                }
            }
            if changed && single_line {
                text = json::collapse_path(&text, &["hooks"]);
            }
            m.op = if changed {
                Op::File { contents: text }
            } else {
                Op::Unchanged
            };
        }

        // Cairn generated the whole plugin file, so removing it is removing
        // the file — and nothing else in that directory is touched.
        (AgentId::Opencode, ResourceKind::Lifecycle) => {
            m.op = if location.exists() {
                Op::RemoveTree
            } else {
                Op::Unchanged
            };
        }

        (_, ResourceKind::Instructions) => {
            m.op = write_or_unchanged(markdown::remove(&display, &current, CONTRACT_ID)?);
        }

        (_, ResourceKind::Skill) => {
            m.op = if location.join("SKILL.md").exists() {
                Op::RemoveTree
            } else {
                Op::Unchanged
            };
        }

        (AgentId::GenericMcp, _) => m.op = Op::Unchanged,
    }
    Ok(m)
}

/// Write one materialized resource atomically and verify it landed.
pub fn commit(m: &Materialized) -> Result<(), ApplyError> {
    match &m.op {
        Op::Unchanged => Ok(()),
        Op::File { contents } => {
            apply::write_atomic(&m.location, contents)?;
            apply::verify_file(&m.location, contents)
        }
        Op::Tree { files } => {
            // A Skill directory is replaced wholesale rather than merged, so a
            // file removed from the canonical tree does not linger.
            apply::remove_tree(&m.location)?;
            apply::write_tree(&TreeWrite {
                root: m.location.clone(),
                files: files.clone(),
                agent: m.agent,
                kind: m.kind,
            })?;
            for (rel, contents) in files {
                apply::verify_file(&m.location.join(rel), contents)?;
            }
            Ok(())
        }
        Op::RemoveTree => {
            if m.location.is_dir() {
                apply::remove_tree(&m.location)
            } else {
                match std::fs::remove_file(&m.location) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(ApplyError::Io {
                        path: m.location.display().to_string(),
                        detail: e.to_string(),
                    }),
                }
            }
        }
    }
}

/// The Cairn-owned content preserved before a forced repair.
///
/// Only the managed block body, the owned entry, or a wholly Cairn-generated
/// file — never the enclosing configuration file (FR-238). Returns `None`
/// where the owned content cannot be isolated, and the caller then reports the
/// condition and changes nothing.
pub fn owned_content(
    env: &Env,
    agent: AgentId,
    kind: ResourceKind,
    scope: InstallationScope,
) -> Option<String> {
    let location = scope::location(env, agent, kind, scope)?;
    let display = location.display().to_string();
    let current = std::fs::read_to_string(&location).ok()?;
    match kind {
        ResourceKind::Instructions => markdown::find(&display, &current, CONTRACT_ID)
            .ok()
            .flatten()
            .map(|b| b.body),
        ResourceKind::Mcp => {
            let value = if agent == AgentId::Codex {
                toml::get(&display, &current, &["mcp_servers", crate::MCP_SERVER_NAME]).ok()?
            } else {
                json::get(&display, &current, mcp_keys(agent)).ok()?
            };
            value.map(|v| serde_json::to_string_pretty(&v).unwrap_or_default())
        }
        // Cairn generated these in full, so the file *is* the owned content.
        ResourceKind::Lifecycle if agent == AgentId::Opencode => Some(current),
        ResourceKind::Lifecycle => {
            let value = json::read(&display, &current).ok()?;
            value
                .get("hooks")
                .map(|h| serde_json::to_string_pretty(h).unwrap_or_default())
        }
        ResourceKind::Skill => std::fs::read_to_string(location.join("SKILL.md")).ok(),
    }
}

fn write_or_unchanged(change: Change) -> Op {
    match change {
        Change::Unchanged => Op::Unchanged,
        Change::Written(contents) => Op::File { contents },
    }
}

/// A stable digest source for a set of registrations, so the record can tell
/// "behind this build" from "edited by hand".
fn registration_digest_source(events: &[&str], entry: fn(&str) -> serde_json::Value) -> String {
    events
        .iter()
        .map(|ev| entry(ev).to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> (tempfile::TempDir, Env) {
        let dir = tempfile::tempdir().expect("tempdir");
        let env = Env::new(dir.path().join("home"), dir.path().join("repo"));
        std::fs::create_dir_all(&env.worktree).unwrap();
        std::fs::create_dir_all(&env.home).unwrap();
        (dir, env)
    }

    #[test]
    fn materializing_writes_nothing() {
        // SC-118: this is the half of the inertness guarantee that lives here.
        let (dir, env) = env();
        let before: Vec<_> = walkdir(dir.path());
        for agent in [AgentId::ClaudeCode, AgentId::Codex, AgentId::Opencode] {
            for kind in ResourceKind::ALL {
                let scope = scope::resolve_scope(agent, kind, false, None).unwrap();
                let _ = materialize_install(&env, agent, kind, scope);
                let _ = materialize_removal(&env, agent, kind, scope, None);
            }
        }
        assert_eq!(before, walkdir(dir.path()));
    }

    fn walkdir(root: &std::path::Path) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(root) {
            for e in entries.filter_map(|e| e.ok()) {
                let p = e.path();
                out.push(p.display().to_string());
                if p.is_dir() {
                    out.extend(walkdir(&p));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn installing_then_removing_returns_a_file_to_its_original_bytes() {
        // SC-104 through the apply path rather than the editors directly.
        let (_dir, env) = env();
        let original = "{\n  \"mcpServers\": {\n    \"other\": { \"command\": \"o\" }\n  }\n}\n";
        let path = env.home.join(".claude.json");
        std::fs::write(&path, original).unwrap();

        let install = materialize_install(
            &env,
            AgentId::ClaudeCode,
            ResourceKind::Mcp,
            InstallationScope::User,
        )
        .unwrap();
        commit(&install).unwrap();
        assert_ne!(std::fs::read_to_string(&path).unwrap(), original);

        let recorded = RecordedInstall {
            agent: AgentId::ClaudeCode,
            kind: ResourceKind::Mcp,
            owner: crate::model::ResourceOwner::Direct,
            scope: InstallationScope::User,
            location: path.clone(),
            content_hash: install.content_hash.clone(),
            artifact_schema: None,
            artifact_revision: None,
            activation: crate::model::ActivationState::NotApplicable,
            serves: vec![AgentId::ClaudeCode],
            container_single_line: install.container_single_line,
            created_container: install.created_container,
        };
        let removal = materialize_removal(
            &env,
            AgentId::ClaudeCode,
            ResourceKind::Mcp,
            InstallationScope::User,
            Some(&recorded),
        )
        .unwrap();
        commit(&removal).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn a_second_install_is_unchanged() {
        // FR-157, SC-102 through the apply path.
        let (_dir, env) = env();
        for (agent, kind) in [
            (AgentId::ClaudeCode, ResourceKind::Mcp),
            (AgentId::ClaudeCode, ResourceKind::Lifecycle),
            (AgentId::ClaudeCode, ResourceKind::Instructions),
            (AgentId::ClaudeCode, ResourceKind::Skill),
            (AgentId::Codex, ResourceKind::Mcp),
            (AgentId::Codex, ResourceKind::Lifecycle),
            (AgentId::Opencode, ResourceKind::Lifecycle),
            (AgentId::Opencode, ResourceKind::Mcp),
        ] {
            let scope = scope::resolve_scope(agent, kind, false, None).unwrap();
            let first = materialize_install(&env, agent, kind, scope).unwrap();
            assert!(first.writes(), "{agent}/{kind} had nothing to install");
            commit(&first).unwrap();
            let second = materialize_install(&env, agent, kind, scope).unwrap();
            assert!(
                !second.writes(),
                "{agent}/{kind} rewrote itself on a second install"
            );
        }
    }

    #[test]
    fn the_skill_tree_is_written_whole_and_removed_whole() {
        let (_dir, env) = env();
        let m = materialize_install(
            &env,
            AgentId::ClaudeCode,
            ResourceKind::Skill,
            InstallationScope::User,
        )
        .unwrap();
        commit(&m).unwrap();
        let root = env.home.join(".claude/skills/cairn");
        assert!(root.join("SKILL.md").exists());
        assert!(root.join("references/resuming-work.md").exists());

        // Doctor's comparison sees exactly this build.
        let installed = revision::installed_revision(&root).unwrap();
        assert!(installed.matches(revision::embedded_schema(), &revision::embedded_revision()));
        assert!(installed.self_consistent());

        let r = materialize_removal(
            &env,
            AgentId::ClaudeCode,
            ResourceKind::Skill,
            InstallationScope::User,
            None,
        )
        .unwrap();
        commit(&r).unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn the_opencode_plugin_is_a_file_drop() {
        // D32: no mutation of opencode.json is involved.
        let (_dir, env) = env();
        let before = std::fs::read_to_string(env.config_home.join("opencode/opencode.json")).ok();
        let m = materialize_install(
            &env,
            AgentId::Opencode,
            ResourceKind::Lifecycle,
            InstallationScope::User,
        )
        .unwrap();
        commit(&m).unwrap();
        assert!(env.config_home.join("opencode/plugin/cairn.js").exists());
        assert_eq!(
            std::fs::read_to_string(env.config_home.join("opencode/opencode.json")).ok(),
            before
        );
    }

    #[test]
    fn owned_content_is_the_block_not_the_file() {
        // FR-238: never the enclosing configuration file.
        let (_dir, env) = env();
        std::fs::write(
            env.worktree.join("CLAUDE.md"),
            "# Project\n\nA developer secret: sk-not-a-real-secret-000\n",
        )
        .unwrap();
        let m = materialize_install(
            &env,
            AgentId::ClaudeCode,
            ResourceKind::Instructions,
            InstallationScope::ProjectShared,
        )
        .unwrap();
        commit(&m).unwrap();

        let owned = owned_content(
            &env,
            AgentId::ClaudeCode,
            ResourceKind::Instructions,
            InstallationScope::ProjectShared,
        )
        .expect("the block");
        assert!(owned.contains("Cairn"));
        assert!(
            !owned.contains("sk-not-a-real-secret-000"),
            "the recovery content carried the developer's file"
        );
    }

    #[test]
    fn cairn_writes_nothing_for_a_generic_client() {
        let (_dir, env) = env();
        assert!(materialize_install(
            &env,
            AgentId::GenericMcp,
            ResourceKind::Mcp,
            InstallationScope::User
        )
        .is_err());
    }

    #[test]
    fn a_malformed_file_is_refused_rather_than_replaced() {
        // FR-137: report the condition, change nothing.
        let (_dir, env) = env();
        let path = env.home.join(".claude.json");
        std::fs::write(&path, "{ \"mcpServers\": ").unwrap();
        let e = materialize_install(
            &env,
            AgentId::ClaudeCode,
            ResourceKind::Mcp,
            InstallationScope::User,
        )
        .unwrap_err();
        assert_eq!(
            e.condition(),
            crate::model::HealthCondition::MalformedConfig
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ \"mcpServers\": "
        );
    }
}
