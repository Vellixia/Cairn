//! `cairn doctor` - diagnostic check for server connectivity and agent config.
//!
//! Checks:
//! - Data directory exists and is writable
//! - Remote server is reachable with a valid token (calls /api/memory/wakeup)
//! - Supported AI agents are detected
//!
//! Exit codes:
//! - 0  - all green
//! - 1  - one or more failures (printed above)
//! - 2  - usage error (invalid flags)

use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DoctorOptions {
    pub fix: bool,
    /// Reserved for the future `--interactive` flag; currently always read
    /// from `std::io::stdout().is_terminal()` at the call site, so the field
    /// here is dead. Kept for API stability with future toggles.
    #[allow(dead_code)]
    pub interactive: bool,
    /// Output machine-readable JSON instead of human-readable text.
    #[allow(dead_code)]
    pub json: bool,
}

/// Outcome of `doctor run`. Used by `onboard` to decide whether to proceed.
#[derive(Debug)]
pub struct Diagnosis {
    pub checks: Vec<Check>,
}

impl Diagnosis {
    pub fn ok(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }

    pub fn exit_code(&self) -> i32 {
        if self.ok() {
            0
        } else {
            1
        }
    }
}

#[derive(Debug, Clone)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

pub fn run(opts: DoctorOptions) -> Diagnosis {
    let mut checks = Vec::new();

    let cfg = match cairn_core::Config::resolve(None) {
        Ok(c) => c,
        Err(e) => {
            checks.push(Check {
                name: "data dir",
                ok: false,
                detail: format!("failed to resolve: {e}"),
            });
            return finalize(checks);
        }
    };
    checks.push(check_data_dir(&cfg, opts.fix));
    checks.push(check_remote_server());
    checks.push(check_agents());
    checks.push(check_project());
    checks.push(check_config_health());

    finalize(checks)
}

fn finalize(checks: Vec<Check>) -> Diagnosis {
    let diag = Diagnosis { checks };
    // Print in a stable order.
    for c in &diag.checks {
        let sym = if c.ok { "OK" } else { "FAIL" };
        eprintln!("  {sym} {:<14} {}", c.name, c.detail);
    }
    if diag.ok() {
        eprintln!("\ncairn doctor: ok");
    } else {
        eprintln!("\ncairn doctor: FAIL");
    }
    diag
}

fn check_data_dir(cfg: &cairn_core::Config, fix: bool) -> Check {
    let dir = cfg.data_dir();
    if dir.exists() {
        // Probe writability with a tiny test file (don't actually persist it).
        let probe = dir.join(".cairn-doctor-probe");
        match std::fs::write(&probe, b"ok") {
            Ok(()) => {
                let _ = std::fs::remove_file(&probe);
                Check {
                    name: "data dir",
                    ok: true,
                    detail: format!("{} (writable)", dir.display()),
                }
            }
            Err(e) => Check {
                name: "data dir",
                ok: false,
                detail: format!("{} (not writable: {e})", dir.display()),
            },
        }
    } else if fix {
        match std::fs::create_dir_all(dir) {
            Ok(()) => Check {
                name: "data dir",
                ok: true,
                detail: format!("{} (created by --fix)", dir.display()),
            },
            Err(e) => Check {
                name: "data dir",
                ok: false,
                detail: format!(
                    "{} (missing and --fix could not create: {e})",
                    dir.display()
                ),
            },
        }
    } else {
        Check {
            name: "data dir",
            ok: false,
            detail: format!("{} (missing - run with --fix to create)", dir.display()),
        }
    }
}

fn check_remote_server() -> Check {
    let server = std::env::var("CAIRN_SERVER").ok();
    match server {
        Some(s) if !s.trim().is_empty() => {
            let token = std::env::var("CAIRN_TOKEN").ok();
            let (ok, detail) = match token {
                Some(t) if !t.is_empty() => {
                    // Validate the token with a real request.
                    let url = format!("{}/api/memory/wakeup?limit=1", s.trim_end_matches('/'));
                    match ureq::get(&url)
                        .set("Authorization", &format!("Bearer {t}"))
                        .call()
                    {
                        Ok(resp) if resp.status() == 200 => (true, format!("{s} (token valid)")),
                        Ok(resp) => {
                            let status = resp.status();
                            let body = resp.into_string().unwrap_or_default();
                            (
                                false,
                                format!("{s} (token rejected: HTTP {status} -- {body})"),
                            )
                        }
                        Err(e) => (false, format!("{s} (token check failed: {e})")),
                    }
                }
                Some(_) => (false, format!("{s} (CAIRN_TOKEN is empty)")),
                None => (
                    false,
                    format!("{s} (no CAIRN_TOKEN -- every request will 401)"),
                ),
            };
            Check {
                name: "remote server",
                ok,
                detail,
            }
        }
        _ => Check {
            name: "remote server",
            ok: true,
            detail: "(unset -- local mode)".into(),
        },
    }
}

fn check_agents() -> Check {
    let project = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = home_dir();
    let mut found = Vec::new();
    for id in ["claude-code", "codex", "opencode"] {
        if detect_agent(id, &project, home.as_deref()) {
            found.push(id);
        }
    }
    if found.is_empty() {
        Check {
            name: "agents",
            ok: true,
            detail: "no supported agents detected (run `cairn setup <agent>`)".into(),
        }
    } else {
        Check {
            name: "agents",
            ok: true,
            detail: format!("detected: {}", found.join(", ")),
        }
    }
}

/// v0.8.0 Sprint 3: show what the `SessionStart` hook would auto-detect as the current
/// project, using the same `detect_project()` the hook itself calls.
fn check_project() -> Check {
    let (id, name) = crate::project::detect_project();
    match id {
        Some(id) => Check {
            name: "project",
            ok: true,
            detail: format!("{name} (hash: {id})"),
        },
        None => Check {
            name: "project",
            ok: true,
            detail: "(no project detected - using global scope)".into(),
        },
    }
}

/// Detect duplicate cairn hook entries across all agent config files and warn
/// when the OpenCode plugin is on disk but not registered in `opencode.json`.
/// These are symptoms of stale state that `cairn setup` can repair on re-run.
fn check_config_health() -> Check {
    let home = home_dir();
    let mut issues: Vec<String> = Vec::new();

    // Codex hooks.json duplicate count
    if let Some(h) = home.as_deref() {
        let hooks_path = h.join(".codex").join("hooks.json");
        if hooks_path.exists() {
            if let Ok(text) = std::fs::read_to_string(&hooks_path) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(obj) = v.get("hooks").and_then(|o| o.as_object()) {
                        for (event, arr) in obj {
                            if let Some(arr) = arr.as_array() {
                                let cairn = arr
                                    .iter()
                                    .filter(|g| {
                                        g.get("hooks")
                                            .and_then(|hs| hs.as_array())
                                            .map(|hs| {
                                                hs.iter().any(|h| {
                                                    h.get("command")
                                                        .and_then(|c| c.as_str())
                                                        .map(|c| {
                                                            let lower = c.to_ascii_lowercase();
                                                            lower.contains("cairn")
                                                                && lower.contains("hook")
                                                        })
                                                        .unwrap_or(false)
                                                })
                                            })
                                            .unwrap_or(false)
                                    })
                                    .count();
                                if cairn > 1 {
                                    issues.push(format!(
                                        "{event}: {cairn} cairn hooks (dedup with `cairn setup codex`)"
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // OpenCode auto-loads local plugin files from its `plugins/` directory. The cairn
    // plugin must therefore NOT appear in opencode.json's `plugin` array (that array is
    // for npm packages) — a local path there makes OpenCode load the plugin twice, so
    // every lifecycle hook fires twice. Older cairn versions wrote such an entry; flag
    // it so `cairn setup opencode` can strip it.
    {
        let cfg = opencode_config_path();
        let double_registered = cfg
            .as_path()
            .exists()
            .then(|| std::fs::read_to_string(&cfg).ok())
            .flatten()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| {
                v.get("plugin").and_then(|p| p.as_array()).map(|arr| {
                    arr.iter().any(|p| {
                        p.as_str().is_some_and(|s| {
                            let n = s.replace('\\', "/").to_ascii_lowercase();
                            n.ends_with("/plugins/cairn.js") || n == "plugins/cairn.js"
                        })
                    })
                })
            })
            .unwrap_or(false);
        if double_registered {
            issues.push(
                "opencode.json lists the cairn plugin in its `plugin` array; it already \
                 auto-loads from plugins/ and will double-fire (`cairn setup opencode` to fix)"
                    .to_string(),
            );
        }
    }

    if issues.is_empty() {
        Check {
            name: "config health",
            ok: true,
            detail: "ok".into(),
        }
    } else {
        Check {
            name: "config health",
            ok: false,
            detail: issues.join("; "),
        }
    }
}

fn detect_agent(id: &str, project: &std::path::Path, home: Option<&std::path::Path>) -> bool {
    let home_has = |rel: &str| home.is_some_and(|h| h.join(rel).exists());
    match id {
        "claude-code" => {
            project.join(".claude").exists()
                || project.join(".mcp.json").exists()
                || home_has(".claude")
                || home_has(".claude.json")
        }
        "codex" => {
            codex_config_path(home).exists()
                || project.join(".codex").join("config.toml").exists()
                || home_has(".codex/config.toml")
        }
        "opencode" => opencode_config_path().exists() || project.join(".opencode").exists(),
        _ => false,
    }
}

fn opencode_config_path() -> PathBuf {
    // XDG_CONFIG_HOME already IS the config root (e.g. ~/.config); don't add .config again.
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("opencode").join("opencode.json");
    }
    let base = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(".config").join("opencode").join("opencode.json")
}

fn codex_config_path(home: Option<&std::path::Path>) -> PathBuf {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(PathBuf::from))
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    config_home.join(".codex").join("config.toml")
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Build a short-lived full diagnosis from a list of checks - used by the `doctor`
/// CLI entry point so it can return a non-zero exit code on failure.
pub fn run_and_exit(opts: DoctorOptions) -> Result<()> {
    let diag = run(opts);
    std::process::exit(diag.exit_code());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnosis_exit_code_reflects_ok_or_fail() {
        let ok = Diagnosis {
            checks: vec![Check {
                name: "x",
                ok: true,
                detail: "ok".into(),
            }],
        };
        assert_eq!(ok.exit_code(), 0);
        assert!(ok.ok());

        let bad = Diagnosis {
            checks: vec![Check {
                name: "x",
                ok: false,
                detail: "fail".into(),
            }],
        };
        assert_eq!(bad.exit_code(), 1);
        assert!(!bad.ok());
    }

    #[test]
    fn doctor_check_data_dir_creates_when_fix_set() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("cairn-data");
        assert!(!target.exists());

        let mut cfg = cairn_core::Config::resolve(None).unwrap();
        cfg.data_dir = target.clone();

        let c = check_data_dir(&cfg, true);
        assert!(
            c.ok,
            "fix=true should create the missing dir; got: {}",
            c.detail
        );
        assert!(target.exists(), "the data dir should have been created");

        let c = check_data_dir(&cfg, false);
        assert!(c.ok);
    }

    #[test]
    fn doctor_check_data_dir_reports_missing_without_fix() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("cairn-data-missing");
        assert!(!target.exists());

        let mut cfg = cairn_core::Config::resolve(None).unwrap();
        cfg.data_dir = target;

        let c = check_data_dir(&cfg, false);
        assert!(!c.ok);
        assert!(c.detail.contains("--fix"));
    }
}
