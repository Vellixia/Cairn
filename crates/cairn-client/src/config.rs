//! `~/.cairn/config.toml` - persists server URL, token, and hook tuning so
//! agent configs can reference `cairn mcp`/`cairn hook` WITHOUT embedding a
//! secret in every agent's own config file, and so hooks work without the
//! user having to export `CAIRN_SERVER`/`CAIRN_TOKEN` globally (see
//! `hook.rs`'s doc comment: before this file existed, a hook subprocess had no
//! way to see the env vars `cairn setup` wrote into an *agent's* MCP entry -
//! those are only visible to the spawned `cairn mcp` process - so hooks were
//! broken by default for anyone who didn't also export the vars in their
//! shell profile).
//!
//! Env vars always win over this file, and this file always wins over the
//! binary's built-in defaults: `env > file > default`.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub fn config_path() -> Option<PathBuf> {
    crate::paths::cairn_home().map(|h| h.join("config.toml"))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectOverride {
    pub inject_context: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct FileConfig {
    pub server_url: Option<String>,
    pub token: Option<String>,
    pub inject_context: Option<bool>,
    pub timeout_ms: Option<u64>,
    pub debug: Option<bool>,
    /// v0.8.0 Sprint 10 (C-2): opt-in real-time guard on `PreToolUse` - `None`/absent means
    /// off, matching every existing install's behavior unchanged until a user explicitly
    /// turns it on.
    pub guard: Option<bool>,
    pub projects: BTreeMap<String, ProjectOverride>,
}

/// Load `~/.cairn/config.toml`. A missing file is just an empty config (this
/// is the common case pre-migration and for anyone who only uses env vars); a
/// present-but-corrupt file falls back to defaults with a stderr note rather
/// than failing whatever command asked for it.
pub fn load() -> FileConfig {
    let Some(path) = config_path() else {
        return FileConfig::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return FileConfig::default();
    };
    match parse(&text) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("cairn: ~/.cairn/config.toml is invalid, ignoring it ({e})");
            FileConfig::default()
        }
    }
}

fn parse(text: &str) -> Result<FileConfig> {
    let doc: toml_edit::DocumentMut = text.parse().context("parsing config.toml")?;
    let mut cfg = FileConfig::default();
    if let Some(server) = doc.get("server") {
        cfg.server_url = server.get("url").and_then(|v| v.as_str()).map(String::from);
        cfg.token = server
            .get("token")
            .and_then(|v| v.as_str())
            .map(String::from);
    }
    if let Some(hooks) = doc.get("hooks") {
        cfg.inject_context = hooks.get("inject_context").and_then(|v| v.as_bool());
        cfg.timeout_ms = hooks
            .get("timeout_ms")
            .and_then(|v| v.as_integer())
            .and_then(|n| u64::try_from(n).ok());
        cfg.debug = hooks.get("debug").and_then(|v| v.as_bool());
        cfg.guard = hooks.get("guard").and_then(|v| v.as_bool());
    }
    // `as_table_like` (not `as_table`) so this also recognizes a hand-edited
    // `projects = { "id" = { inject_context = false } }` inline form, not
    // just the generated `[projects.id]` table-header form - `toml_edit`
    // represents those as different `Item` variants (`Table` vs
    // `Value(InlineTable)`), and only `as_table_like` unifies both.
    if let Some(projects) = doc.get("projects").and_then(toml_edit::Item::as_table_like) {
        for (id, tbl) in projects.iter() {
            let inject_context = tbl.get("inject_context").and_then(|v| v.as_bool());
            cfg.projects
                .insert(id.to_string(), ProjectOverride { inject_context });
        }
    }
    Ok(cfg)
}

/// Where a resolved value came from - shown by `status`/`doctor` so a user
/// debugging "why is X set to Y" doesn't have to guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Env,
    File,
    Default,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Env => "env",
            Source::File => "config",
            Source::Default => "default",
        }
    }
}

pub struct Resolved {
    pub server: Option<(String, Source)>,
    pub token: Option<(String, Source)>,
    pub inject_context: (bool, Source),
    pub timeout_ms: u64,
    pub debug: bool,
    /// v0.8.0 Sprint 10 (C-2): whether `PreToolUse` should verify/sanitize proposed edits and
    /// commands before they run. Defaults to `false` - opt-in only.
    pub guard: bool,
}

/// Resolve effective settings for the current process: env vars first, then
/// `~/.cairn/config.toml` (a per-project override inside it beats the file's
/// global setting), then the binary's built-in default. `project_id` is the
/// 16-hex-char hash from `project::detect_project()`, used to look up
/// `[projects."<id>"]` overrides; pass `None` when there's no project context
/// (e.g. `cairn status` run outside a project).
pub fn resolve(project_id: Option<&str>) -> Resolved {
    let file = load();

    let server = match std::env::var("CAIRN_SERVER")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        Some(s) => Some((s, Source::Env)),
        None => file.server_url.clone().map(|s| (s, Source::File)),
    };
    let token = match std::env::var("CAIRN_TOKEN").ok().filter(|t| !t.is_empty()) {
        Some(t) => Some((t, Source::Env)),
        None => file.token.clone().map(|t| (t, Source::File)),
    };

    let inject_context = if let Ok(v) = std::env::var("CAIRN_INJECT_CONTEXT") {
        (
            matches!(v.as_str(), "1" | "true" | "yes" | "on"),
            Source::Env,
        )
    } else {
        let project_override = project_id
            .and_then(|pid| file.projects.get(pid))
            .and_then(|p| p.inject_context);
        match project_override.or(file.inject_context) {
            Some(v) => (v, Source::File),
            None => (false, Source::Default),
        }
    };

    let timeout_ms = std::env::var("CAIRN_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .or(file.timeout_ms)
        .unwrap_or(crate::http::DEFAULT_TIMEOUT_MS);

    let debug = std::env::var("CAIRN_DEBUG")
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or_else(|| file.debug.unwrap_or(false));

    let guard = std::env::var("CAIRN_GUARD")
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or_else(|| file.guard.unwrap_or(false));

    Resolved {
        server,
        token,
        inject_context,
        timeout_ms,
        debug,
        guard,
    }
}

/// Merge `server_url`/`token` into `~/.cairn/config.toml`, creating it if
/// absent and preserving every other key (hooks tuning, project overrides,
/// any hand-edited comment) via `toml_edit`. `None` values leave the
/// corresponding existing key untouched - this is a merge, not a replace.
pub fn save_server(server_url: Option<&str>, token: Option<&str>) -> Result<()> {
    let Some(path) = config_path() else {
        anyhow::bail!("cannot determine home directory (set $HOME or $USERPROFILE)");
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let original = if path.exists() {
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?
    } else {
        String::new()
    };
    let mut doc: toml_edit::DocumentMut = if original.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        original
            .parse()
            .context("~/.cairn/config.toml is not valid TOML; refusing to overwrite it")?
    };

    let server_table = doc
        .entry("server")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("config.toml: `server` is not a table")?;
    if let Some(s) = server_url {
        server_table["url"] = toml_edit::value(s);
    }
    if let Some(t) = token {
        server_table["token"] = toml_edit::value(t);
    }

    write_config_file(&path, &doc.to_string())
}

/// Set `[hooks] inject_context = <value>` - called on new `pair`/`onboard`/
/// `setup` runs so freshly-onboarded users get the flagship feature on by
/// default (existing installs, which never touch this function unless they
/// re-onboard, keep the in-binary default of off - see `hook.rs`).
pub fn save_inject_context_default(enabled: bool) -> Result<()> {
    let Some(path) = config_path() else {
        anyhow::bail!("cannot determine home directory (set $HOME or $USERPROFILE)");
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let original = if path.exists() {
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?
    } else {
        String::new()
    };
    let mut doc: toml_edit::DocumentMut = if original.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        original
            .parse()
            .context("~/.cairn/config.toml is not valid TOML; refusing to overwrite it")?
    };
    let hooks_table = doc
        .entry("hooks")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("config.toml: `hooks` is not a table")?;
    hooks_table["inject_context"] = toml_edit::value(enabled);
    write_config_file(&path, &doc.to_string())
}

fn write_config_file(path: &std::path::Path, text: &str) -> Result<()> {
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("setting permissions on {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env_guard::with_env;

    /// `std::sync::Mutex` (what `env_guard::ENV_LOCK` is) is NOT reentrant: a
    /// thread that calls `with_env` again while already inside a `with_env`
    /// closure deadlocks itself forever (confirmed the hard way - see git
    /// history for this comment). Every test below therefore makes exactly
    /// ONE `with_env` call, listing every var it needs - HOME/USERPROFILE
    /// (so `config_path()` resolves under a throwaway temp dir) alongside
    /// whatever `CAIRN_*` vars that specific test cares about - rather than
    /// nesting a "set up the temp home" wrapper around a second call.
    /// Sequential (non-nested) `with_env` calls on the same thread are fine;
    /// only nesting deadlocks.
    struct TempHome {
        dir: tempfile::TempDir,
        home_str: String,
    }

    fn temp_home() -> TempHome {
        let dir = tempfile::tempdir().unwrap();
        let home_str = dir.path().to_string_lossy().into_owned();
        TempHome { dir, home_str }
    }

    impl TempHome {
        fn path(&self) -> &std::path::Path {
            self.dir.path()
        }

        /// `(key, value)` pairs pinning HOME/USERPROFILE to this temp dir -
        /// splice into every `with_env` call alongside test-specific vars.
        fn env_pins(&self) -> [(&str, Option<&str>); 2] {
            [
                ("HOME", Some(self.home_str.as_str())),
                ("USERPROFILE", Some(self.home_str.as_str())),
            ]
        }
    }

    #[test]
    fn env_wins_over_file_wins_over_default() {
        let home = temp_home();

        // File only: write the file under a plain HOME pin (its own single
        // with_env call), then resolve in a SEPARATE with_env call (sequential,
        // not nested) with CAIRN_SERVER/CAIRN_TOKEN explicitly unset.
        with_env(&home.env_pins(), || {
            save_server(Some("http://file-server:7777"), Some("file-token")).unwrap();
        });
        with_env(
            &[
                home.env_pins()[0],
                home.env_pins()[1],
                ("CAIRN_SERVER", None),
                ("CAIRN_TOKEN", None),
            ],
            || {
                let r = resolve(None);
                assert_eq!(
                    r.server.unwrap(),
                    ("http://file-server:7777".to_string(), Source::File)
                );
                assert_eq!(r.token.unwrap(), ("file-token".to_string(), Source::File));
            },
        );

        // Env overrides file.
        with_env(
            &[
                home.env_pins()[0],
                home.env_pins()[1],
                ("CAIRN_SERVER", Some("http://env-server:7777")),
                ("CAIRN_TOKEN", Some("env-token")),
            ],
            || {
                let r = resolve(None);
                assert_eq!(
                    r.server.unwrap(),
                    ("http://env-server:7777".to_string(), Source::Env)
                );
                assert_eq!(r.token.unwrap(), ("env-token".to_string(), Source::Env));
            },
        );
    }

    #[test]
    fn inject_context_env_values_are_case_sensitive() {
        let home = temp_home();
        for v in ["true", "1", "yes", "on"] {
            with_env(
                &[
                    home.env_pins()[0],
                    home.env_pins()[1],
                    ("CAIRN_INJECT_CONTEXT", Some(v)),
                ],
                || {
                    assert_eq!(
                        resolve(None).inject_context,
                        (true, Source::Env),
                        "{v} should enable injection"
                    );
                },
            );
        }
        for v in ["", "false", "0", "no", "off", "TRUE"] {
            with_env(
                &[
                    home.env_pins()[0],
                    home.env_pins()[1],
                    ("CAIRN_INJECT_CONTEXT", Some(v)),
                ],
                || {
                    assert!(
                        !resolve(None).inject_context.0,
                        "{v:?} should NOT enable injection (case-sensitive; only true/1/yes/on)"
                    );
                },
            );
        }
    }

    #[test]
    fn guard_defaults_off_and_is_settable_via_env_or_file() {
        let home = temp_home();
        with_env(
            &[
                home.env_pins()[0],
                home.env_pins()[1],
                ("CAIRN_GUARD", None),
            ],
            || assert!(!resolve(None).guard, "off by default - opt-in only"),
        );
        with_env(
            &[
                home.env_pins()[0],
                home.env_pins()[1],
                ("CAIRN_GUARD", Some("true")),
            ],
            || assert!(resolve(None).guard),
        );
        with_env(&home.env_pins(), || {
            let path = home.path().join(".cairn/config.toml");
            std::fs::create_dir_all(home.path().join(".cairn")).unwrap();
            std::fs::write(&path, "[hooks]\nguard = true\n").unwrap();
        });
        with_env(
            &[
                home.env_pins()[0],
                home.env_pins()[1],
                ("CAIRN_GUARD", None),
            ],
            || assert!(resolve(None).guard, "file setting should take effect"),
        );
    }

    #[test]
    fn missing_file_resolves_to_defaults() {
        let home = temp_home();
        with_env(
            &[
                home.env_pins()[0],
                home.env_pins()[1],
                ("CAIRN_SERVER", None),
                ("CAIRN_TOKEN", None),
                ("CAIRN_INJECT_CONTEXT", None),
                ("CAIRN_TIMEOUT_MS", None),
                ("CAIRN_DEBUG", None),
                ("CAIRN_GUARD", None),
            ],
            || {
                let r = resolve(None);
                assert!(r.server.is_none());
                assert!(r.token.is_none());
                assert_eq!(r.inject_context, (false, Source::Default));
                assert_eq!(r.timeout_ms, crate::http::DEFAULT_TIMEOUT_MS);
                assert!(!r.debug);
                assert!(!r.guard);
            },
        );
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults_without_erroring() {
        let home = temp_home();
        with_env(&home.env_pins(), || {
            std::fs::create_dir_all(home.path().join(".cairn")).unwrap();
            std::fs::write(home.path().join(".cairn/config.toml"), "not [ valid toml").unwrap();
        });
        with_env(
            &[
                home.env_pins()[0],
                home.env_pins()[1],
                ("CAIRN_SERVER", None),
                ("CAIRN_TOKEN", None),
            ],
            || {
                let r = resolve(None);
                assert!(r.server.is_none());
                assert!(r.token.is_none());
            },
        );
    }

    #[test]
    fn project_override_beats_global_inject_context() {
        let home = temp_home();
        with_env(&home.env_pins(), || {
            save_server(None, None).unwrap();
            // Write global true + a project override of false directly (save_server
            // only touches [server]; build the rest by hand for this test).
            let path = home.path().join(".cairn/config.toml");
            let mut doc: toml_edit::DocumentMut =
                std::fs::read_to_string(&path).unwrap().parse().unwrap();
            doc["hooks"]["inject_context"] = toml_edit::value(true);
            doc["projects"]["abc123"]["inject_context"] = toml_edit::value(false);
            std::fs::write(&path, doc.to_string()).unwrap();
        });

        with_env(
            &[
                home.env_pins()[0],
                home.env_pins()[1],
                ("CAIRN_INJECT_CONTEXT", None),
            ],
            || {
                let global = resolve(None);
                assert_eq!(global.inject_context, (true, Source::File));
                let project = resolve(Some("abc123"));
                assert_eq!(project.inject_context, (false, Source::File));
            },
        );
    }

    #[test]
    fn save_server_round_trips_and_preserves_other_keys() {
        let home = temp_home();
        with_env(&home.env_pins(), || {
            let path = home.path().join(".cairn/config.toml");
            std::fs::create_dir_all(home.path().join(".cairn")).unwrap();
            std::fs::write(&path, "[hooks]\ndebug = true\n").unwrap();

            save_server(Some("http://h:7777"), Some("tok")).unwrap();

            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains("debug = true"), "unrelated keys must survive");
            let doc: toml_edit::DocumentMut = text.parse().unwrap();
            assert_eq!(doc["server"]["url"].as_str(), Some("http://h:7777"));
            assert_eq!(doc["server"]["token"].as_str(), Some("tok"));
        });
    }

    #[test]
    fn save_server_is_idempotent() {
        let home = temp_home();
        with_env(&home.env_pins(), || {
            save_server(Some("http://h:7777"), Some("tok")).unwrap();
            let path = config_path().unwrap();
            let first = std::fs::read_to_string(&path).unwrap();
            save_server(Some("http://h:7777"), Some("tok")).unwrap();
            let second = std::fs::read_to_string(&path).unwrap();
            assert_eq!(first, second);
        });
    }

    #[cfg(unix)]
    #[test]
    fn save_server_sets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let home = temp_home();
        with_env(&home.env_pins(), || {
            save_server(Some("http://h:7777"), Some("tok")).unwrap();
            let path = config_path().unwrap();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        });
    }
}
