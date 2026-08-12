//! The installation scope matrix, as data (FR-210–FR-220, D27).
//!
//! The rule behind the defaults: prefer a project location the developer does
//! **not** commit; where the agent provides none, use per-user and say so;
//! never fall back to a committed file silently (FR-215, FR-218).
//!
//! Instructions are the deliberate exception. They describe how *this
//! repository* uses Cairn, so they are project-scoped and commit-safe
//! (FR-211).
//!
//! This matrix is a planning contract *and* a test source: every row in
//! `contracts/scope-matrix.md` has an assertion below.

use crate::model::{AgentId, InstallationScope, ResourceKind};
use std::path::{Path, PathBuf};

/// One agent × resource row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeRow {
    pub agent: AgentId,
    pub kind: ResourceKind,
    /// Cairn's default scope for this resource.
    pub default_scope: InstallationScope,
    /// The scope `--shared` moves it to, where that differs.
    pub shared_scope: InstallationScope,
    /// Whether an integration manager can own this resource for this agent.
    pub manager_ownable: bool,
    /// Whether the agent offers a developer-local project location at all.
    /// Where it does not and the default would be developer-local, Cairn must
    /// state that per-user is the only option rather than committing silently
    /// (FR-218).
    pub has_project_local: bool,
}

/// The full matrix. Order is stable so serialization is deterministic.
pub fn matrix() -> Vec<ScopeRow> {
    use InstallationScope::*;
    use ResourceKind::*;
    let row =
        |agent, kind, default_scope, shared_scope, manager_ownable, has_project_local| ScopeRow {
            agent,
            kind,
            default_scope,
            shared_scope,
            manager_ownable,
            has_project_local,
        };
    vec![
        // Claude Code — the one agent with a documented gitignored project
        // settings file, which is why lifecycle defaults to project_local.
        row(AgentId::ClaudeCode, Mcp, User, ProjectShared, true, true),
        row(
            AgentId::ClaudeCode,
            Lifecycle,
            ProjectLocal,
            ProjectShared,
            false,
            true,
        ),
        row(
            AgentId::ClaudeCode,
            Instructions,
            ProjectShared,
            ProjectShared,
            false,
            true,
        ),
        row(AgentId::ClaudeCode, Skill, User, User, true, true),
        // Codex — no project-local-ignored configuration file exists, so
        // lifecycle is per-user: machine-local activation, stated at connect
        // time rather than committed silently (FR-218).
        row(AgentId::Codex, Mcp, User, ProjectShared, true, false),
        row(AgentId::Codex, Lifecycle, User, ProjectShared, false, false),
        row(
            AgentId::Codex,
            Instructions,
            ProjectShared,
            ProjectShared,
            false,
            false,
        ),
        row(AgentId::Codex, Skill, User, User, true, false),
        // OpenCode — same reasoning as Codex for lifecycle.
        row(AgentId::Opencode, Mcp, User, ProjectShared, true, false),
        row(
            AgentId::Opencode,
            Lifecycle,
            User,
            ProjectShared,
            false,
            false,
        ),
        row(
            AgentId::Opencode,
            Instructions,
            ProjectShared,
            ProjectShared,
            false,
            false,
        ),
        row(AgentId::Opencode, Skill, User, User, true, false),
        // Generic MCP has one resource and Cairn writes none of it: the
        // developer pastes the exported block (FR-131).
        row(AgentId::GenericMcp, Mcp, User, User, true, false),
    ]
}

/// Look up one row.
pub fn row(agent: AgentId, kind: ResourceKind) -> Option<ScopeRow> {
    matrix()
        .into_iter()
        .find(|r| r.agent == agent && r.kind == kind)
}

/// The resource kinds Cairn installs for an agent.
pub fn kinds_for(agent: AgentId) -> Vec<ResourceKind> {
    matrix()
        .into_iter()
        .filter(|r| r.agent == agent)
        .map(|r| r.kind)
        .collect()
}

/// The scope Cairn will use, given the developer's choices.
pub fn resolve_scope(
    agent: AgentId,
    kind: ResourceKind,
    shared: bool,
    override_scope: Option<InstallationScope>,
) -> Option<InstallationScope> {
    let r = row(agent, kind)?;
    Some(match (override_scope, shared) {
        (Some(s), _) => s,
        (None, true) => r.shared_scope,
        (None, false) => r.default_scope,
    })
}

/// Where the environment puts things. Held explicitly rather than read from
/// globals so fixtures can drive the whole matrix without touching a real home
/// directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    pub home: PathBuf,
    pub worktree: PathBuf,
    /// `$XDG_CONFIG_HOME`, where OpenCode looks.
    pub config_home: PathBuf,
}

impl Env {
    pub fn new(home: impl Into<PathBuf>, worktree: impl Into<PathBuf>) -> Env {
        let home: PathBuf = home.into();
        Env {
            config_home: home.join(".config"),
            home,
            worktree: worktree.into(),
        }
    }

    /// The real environment.
    pub fn discover(worktree: impl Into<PathBuf>) -> Env {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        Env {
            home,
            config_home,
            worktree: worktree.into(),
        }
    }
}

/// The concrete file or directory one resource occupies.
///
/// Concrete paths live here and in `InstalledResource.location` only. They are
/// never serialized into desired state, which is path-free by construction
/// (`data-model.md` §DesiredIntegrationState).
pub fn location(
    env: &Env,
    agent: AgentId,
    kind: ResourceKind,
    scope: InstallationScope,
) -> Option<PathBuf> {
    use AgentId::*;
    use InstallationScope::*;
    use ResourceKind::*;
    let p = match (agent, kind, scope) {
        (ClaudeCode, Mcp, User) => env.home.join(".claude.json"),
        (ClaudeCode, Mcp, ProjectShared) => env.worktree.join(".mcp.json"),
        (ClaudeCode, Lifecycle, ProjectLocal) => {
            env.worktree.join(".claude").join("settings.local.json")
        }
        (ClaudeCode, Lifecycle, ProjectShared) => {
            env.worktree.join(".claude").join("settings.json")
        }
        (ClaudeCode, Lifecycle, User) => env.home.join(".claude").join("settings.json"),
        (ClaudeCode, Instructions, _) => claude_instructions(env),
        (ClaudeCode, Skill, User) => env.home.join(".claude").join("skills").join("cairn"),

        (Codex, Mcp, User) => env.home.join(".codex").join("config.toml"),
        (Codex, Mcp, ProjectShared) => env.worktree.join(".codex").join("config.toml"),
        (Codex, Lifecycle, User) => env.home.join(".codex").join("hooks.json"),
        (Codex, Lifecycle, ProjectShared) => env.worktree.join(".codex").join("config.toml"),
        (Codex, Instructions, _) => env.worktree.join("AGENTS.md"),
        (Codex, Skill, User) => env.home.join(".codex").join("skills").join("cairn"),

        (Opencode, Mcp, User) => env.config_home.join("opencode").join("opencode.json"),
        (Opencode, Mcp, ProjectShared) => env.worktree.join("opencode.json"),
        (Opencode, Lifecycle, User) => env
            .config_home
            .join("opencode")
            .join("plugin")
            .join("cairn.js"),
        (Opencode, Lifecycle, ProjectShared) => env
            .worktree
            .join(".opencode")
            .join("plugin")
            .join("cairn.js"),
        (Opencode, Instructions, _) => env.worktree.join("AGENTS.md"),
        // OpenCode scans `~/.claude/skills` as well as its own config dirs,
        // which is why it usually binds to Claude Code's copy rather than
        // writing a second one (D32, D28).
        (Opencode, Skill, User) => env
            .config_home
            .join("opencode")
            .join("skills")
            .join("cairn"),

        // Cairn writes nothing for a generic client; it exports a block the
        // developer pastes (FR-131).
        (GenericMcp, _, _) => return None,
        _ => return None,
    };
    Some(p)
}

/// Where a Claude Code project keeps its instructions.
///
/// `./CLAUDE.md` unless the project already keeps them in `.claude/CLAUDE.md`,
/// in which case that is where the block belongs — relocating a developer's
/// instruction file is not Cairn's call (FR-217).
fn claude_instructions(env: &Env) -> PathBuf {
    let nested = env.worktree.join(".claude").join("CLAUDE.md");
    if nested.exists() {
        nested
    } else {
        env.worktree.join("CLAUDE.md")
    }
}

/// Where CC Switch writes a resource for one of its target applications
/// (D33). Cairn never writes these when the owner is `manager`; it verifies
/// them.
pub fn manager_location(env: &Env, app: &str, kind: ResourceKind) -> Option<PathBuf> {
    let p = match (app, kind) {
        ("claude", ResourceKind::Mcp) => env.home.join(".claude.json"),
        ("claude", ResourceKind::Skill) => env.home.join(".claude").join("skills").join("cairn"),
        ("codex", ResourceKind::Mcp) => env.home.join(".codex").join("config.toml"),
        ("codex", ResourceKind::Skill) => env.home.join(".codex").join("skills").join("cairn"),
        ("opencode", ResourceKind::Mcp) => env.config_home.join("opencode").join("opencode.json"),
        ("opencode", ResourceKind::Skill) => env
            .config_home
            .join("opencode")
            .join("skills")
            .join("cairn"),
        _ => return None,
    };
    Some(p)
}

/// The manager's application identifier for one of Cairn's agents.
pub fn manager_app_for(agent: AgentId) -> Option<&'static str> {
    match agent {
        AgentId::ClaudeCode => Some("claude"),
        AgentId::Codex => Some("codex"),
        AgentId::Opencode => Some("opencode"),
        AgentId::GenericMcp => None,
    }
}

/// True where writing this resource at this scope would add or change a file a
/// collaborator receives.
pub fn writes_committed_file(
    env: &Env,
    agent: AgentId,
    kind: ResourceKind,
    scope: InstallationScope,
) -> bool {
    scope.is_committed() && location(env, agent, kind, scope).is_some()
}

/// Whether choosing this scope requires the developer's explicit agreement
/// because the agent offers no developer-local alternative (FR-218).
pub fn requires_explicit_agreement(
    agent: AgentId,
    kind: ResourceKind,
    scope: InstallationScope,
) -> bool {
    let Some(r) = row(agent, kind) else {
        return false;
    };
    // Instructions are project-scoped by design and by purpose, so committing
    // them is the intent rather than a fallback (FR-211).
    if kind == ResourceKind::Instructions {
        return false;
    }
    scope.is_committed() && !r.has_project_local
}

/// True where two owners would write the same effective slot, making bounded
/// overlap ambiguous and automatic migration unsafe (FR-148, D38).
pub fn shares_effective_slot(
    env: &Env,
    agent: AgentId,
    kind: ResourceKind,
    scope: InstallationScope,
    app: &str,
) -> bool {
    match (
        location(env, agent, kind, scope),
        manager_location(env, app, kind),
    ) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Whether a path is inside the repository (and so potentially committed).
pub fn is_inside(worktree: &Path, path: &Path) -> bool {
    path.starts_with(worktree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use InstallationScope::*;
    use ResourceKind::*;

    fn env() -> Env {
        Env::new("/home/dev", "/repo")
    }

    #[test]
    fn matrix_rows_match_the_contract() {
        // Every row of contracts/scope-matrix.md §Summary of defaults.
        let expect = [
            (AgentId::ClaudeCode, Mcp, User),
            (AgentId::ClaudeCode, Lifecycle, ProjectLocal),
            (AgentId::ClaudeCode, Instructions, ProjectShared),
            (AgentId::ClaudeCode, Skill, User),
            (AgentId::Codex, Mcp, User),
            (AgentId::Codex, Lifecycle, User),
            (AgentId::Codex, Instructions, ProjectShared),
            (AgentId::Codex, Skill, User),
            (AgentId::Opencode, Mcp, User),
            (AgentId::Opencode, Lifecycle, User),
            (AgentId::Opencode, Instructions, ProjectShared),
            (AgentId::Opencode, Skill, User),
        ];
        for (agent, kind, scope) in expect {
            assert_eq!(
                row(agent, kind).unwrap().default_scope,
                scope,
                "{agent}/{kind}"
            );
        }
    }

    #[test]
    fn scope_defaults_write_no_committed_lifecycle_file() {
        // SC-126: with default scopes, connecting produces zero committed-file
        // changes for lifecycle handlers.
        let env = env();
        for agent in [AgentId::ClaudeCode, AgentId::Codex, AgentId::Opencode] {
            let scope = resolve_scope(agent, Lifecycle, false, None).unwrap();
            assert!(
                !writes_committed_file(&env, agent, Lifecycle, scope),
                "{agent} lifecycle default commits a file"
            );
        }
    }

    #[test]
    fn only_instructions_are_committed_by_default() {
        let env = env();
        for r in matrix() {
            if r.agent == AgentId::GenericMcp {
                continue;
            }
            let committed = writes_committed_file(&env, r.agent, r.kind, r.default_scope);
            assert_eq!(
                committed,
                r.kind == Instructions,
                "{}/{} committed-by-default is wrong",
                r.agent,
                r.kind
            );
        }
    }

    #[test]
    fn shared_moves_exactly_what_the_contract_says() {
        // `--shared` installs lifecycle and MCP into committed project scope.
        let env = env();
        for agent in [AgentId::ClaudeCode, AgentId::Codex, AgentId::Opencode] {
            for kind in [Mcp, Lifecycle] {
                let scope = resolve_scope(agent, kind, true, None).unwrap();
                assert!(
                    writes_committed_file(&env, agent, kind, scope),
                    "--shared did not commit {agent}/{kind}"
                );
            }
            // The Skill stays per-user under --shared: it teaches generic
            // Cairn workflows that do not belong in every repository (FR-214).
            assert_eq!(resolve_scope(agent, Skill, true, None), Some(User));
        }
    }

    #[test]
    fn an_agent_with_no_project_local_location_needs_explicit_agreement() {
        // FR-218: never a silent committed fallback.
        assert!(requires_explicit_agreement(
            AgentId::Codex,
            Lifecycle,
            ProjectShared
        ));
        assert!(requires_explicit_agreement(
            AgentId::Opencode,
            Lifecycle,
            ProjectShared
        ));
        // Claude has `.claude/settings.local.json`, so choosing committed
        // scope there is an ordinary choice.
        assert!(!requires_explicit_agreement(
            AgentId::ClaudeCode,
            Lifecycle,
            ProjectShared
        ));
        // Instructions are committed by design, not by fallback.
        assert!(!requires_explicit_agreement(
            AgentId::Codex,
            Instructions,
            ProjectShared
        ));
    }

    #[test]
    fn codex_and_opencode_share_one_agents_md() {
        // The shared resource FR-144 and FR-243 are about.
        let env = env();
        assert_eq!(
            location(&env, AgentId::Codex, Instructions, ProjectShared),
            location(&env, AgentId::Opencode, Instructions, ProjectShared)
        );
    }

    #[test]
    fn scope_overrides_win_over_both_defaults_and_shared() {
        assert_eq!(
            resolve_scope(AgentId::ClaudeCode, Mcp, true, Some(User)),
            Some(User)
        );
    }

    #[test]
    fn claude_mcp_collides_with_the_manager_at_user_scope() {
        // The genuine collision D38 and FR-219 describe: same file, same
        // scope, so overlap cannot be unambiguous.
        let env = env();
        assert!(shares_effective_slot(
            &env,
            AgentId::ClaudeCode,
            Mcp,
            User,
            "claude"
        ));
        // The Skill does not collide: Cairn's and the manager's targets are
        // the same directory, which is why it is manager-ownable at all.
        assert!(shares_effective_slot(
            &env,
            AgentId::ClaudeCode,
            Skill,
            User,
            "claude"
        ));
        // Lifecycle is never manager-ownable, so there is nothing to collide.
        assert!(!row(AgentId::ClaudeCode, Lifecycle).unwrap().manager_ownable);
    }

    #[test]
    fn generic_mcp_has_no_location_because_cairn_writes_nothing_for_it() {
        let env = env();
        for kind in ResourceKind::ALL {
            assert_eq!(location(&env, AgentId::GenericMcp, kind, User), None);
        }
        assert_eq!(kinds_for(AgentId::GenericMcp), vec![Mcp]);
    }

    #[test]
    fn manager_targets_match_the_documented_table() {
        let env = env();
        assert_eq!(
            manager_location(&env, "codex", Mcp),
            Some(PathBuf::from("/home/dev/.codex/config.toml"))
        );
        assert_eq!(
            manager_location(&env, "opencode", Skill),
            Some(PathBuf::from("/home/dev/.config/opencode/skills/cairn"))
        );
        assert_eq!(manager_location(&env, "gemini", Mcp), None);
    }
}
