//! `cairn doctor` - diagnostic check for server connectivity and agent config.
//!
//! Checks:
//! - Data directory exists and is writable
//! - Remote server is reachable with a valid token (calls /api/memory/wakeup)
//! - Supported AI agents are detected
//! - Cairn-owned config files aren't in a stale/duplicated state
//!
//! Exit codes:
//! - 0  - all green
//! - 1  - one or more failures (printed above)
//! - 2  - usage error (invalid flags)

use crate::{agents, paths};
use anyhow::Result;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DoctorOptions {
    pub fix: bool,
    /// Output machine-readable JSON instead of human-readable text.
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
            return finalize(checks, opts.json);
        }
    };
    checks.push(check_data_dir(&cfg, opts.fix));
    checks.push(check_remote_server());
    checks.push(check_env_shadows_file());
    checks.push(check_agents());
    checks.push(check_project());
    checks.push(check_config_health(opts.fix));
    checks.push(check_token_expiry());
    checks.push(check_version_skew());
    checks.push(check_spool_backlog());

    finalize(checks, opts.json)
}

#[derive(Serialize)]
struct CheckJson<'a> {
    name: &'a str,
    ok: bool,
    detail: &'a str,
}

#[derive(Serialize)]
struct DiagnosisJson<'a> {
    ok: bool,
    checks: Vec<CheckJson<'a>>,
}

fn finalize(checks: Vec<Check>, json: bool) -> Diagnosis {
    let diag = Diagnosis { checks };
    if json {
        let out = DiagnosisJson {
            ok: diag.ok(),
            checks: diag
                .checks
                .iter()
                .map(|c| CheckJson {
                    name: c.name,
                    ok: c.ok,
                    detail: &c.detail,
                })
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    } else {
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
    // Env and `~/.cairn/config.toml` both count - a server configured only via
    // the config file (the common case post-v0.8.0-redesign) must show as
    // configured here, not as "unset", or doctor would contradict what
    // `cairn hook`/`cairn mcp` actually do.
    let (project_id, _) = crate::project::detect_project();
    let resolved = crate::config::resolve(project_id.as_deref());
    match resolved.server {
        Some((s, src)) => {
            let src = src.label();
            let (ok, detail) = match resolved.token {
                Some((t, _)) => {
                    // Validate the token with a real, timeout-bounded request.
                    let client = crate::http::ApiClient::new(&s, Some(&t));
                    match client.get("/api/memory/wakeup").query("limit", "1").call() {
                        Ok(resp) if resp.status() == 200 => {
                            (true, format!("{s} (from {src}, token valid)"))
                        }
                        Ok(resp) => {
                            let status = resp.status();
                            let body = resp.into_string().unwrap_or_default();
                            (
                                false,
                                format!(
                                    "{s} (from {src}, token rejected: HTTP {status} -- {} -- \
                                     mint a fresh one from the dashboard's You > Tokens page \
                                     -- {body})",
                                    crate::http::token_rejection_causes()
                                ),
                            )
                        }
                        Err(e) => (
                            false,
                            format!(
                                "{s} (from {src}, token check failed: {e} -- is the server reachable?)"
                            ),
                        ),
                    }
                }
                None => (
                    false,
                    format!(
                        "{s} (from {src}, no token configured -- every request will 401 -- \
                         mint one from the dashboard's You > Tokens page)"
                    ),
                ),
            };
            Check {
                name: "remote server",
                ok,
                detail,
            }
        }
        None => Check {
            name: "remote server",
            ok: true,
            detail: "(unset -- local mode)".into(),
        },
    }
}

/// Surface `CAIRN_SERVER`/`CAIRN_TOKEN` env values that are silently shadowing a *different*
/// value saved in `~/.cairn/config.toml` - the direct cause of "I edited the file/ran `cairn
/// setup` again but nothing changed" confusion, since the active (env) value keeps winning per
/// `env > file > default` precedence and nothing before this check said so explicitly. Always
/// `ok` (informational): the active value may be perfectly correct, this just flags that the
/// file has a different one sitting unused - not a functional break, so it must not block
/// `onboard`'s doctor-gate.
fn check_env_shadows_file() -> Check {
    let (project_id, _) = crate::project::detect_project();
    let resolved = crate::config::resolve(project_id.as_deref());
    let mut notes = Vec::new();
    if resolved.server_shadowed_file_value.is_some() {
        notes.push(crate::config::shadow_note("server", "CAIRN_SERVER"));
    }
    if resolved.token_shadowed_file_value.is_some() {
        notes.push(crate::config::shadow_note("token", "CAIRN_TOKEN"));
    }
    if notes.is_empty() {
        Check {
            name: "shadowing",
            ok: true,
            detail: "no shadowed values".into(),
        }
    } else {
        Check {
            name: "shadowing",
            ok: true,
            detail: notes.join("; "),
        }
    }
}

fn check_agents() -> Check {
    let project = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = paths::home_dir();
    let found: Vec<&'static str> = agents::detect_all(&project, home.as_deref())
        .iter()
        .map(|a| a.id())
        .collect();
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

/// Surface non-fatal config-health issues from every agent (duplicate hook
/// entries, double-registered plugins, ...) - symptoms of stale state that
/// `cairn setup` can repair on re-run. With `fix=true`, self-heals by calling
/// the SAME `agent.install()` a manual `cairn setup <agent>` re-run would use
/// (idempotent - it normalizes/de-dupes existing entries rather than treating
/// this as a fresh install), then re-checks health so the report reflects
/// what's actually still broken afterward, not what the fix merely attempted.
fn check_config_health(fix: bool) -> Check {
    let project = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = paths::home_dir();

    let mut remaining: Vec<String> = Vec::new();
    let mut fixed: Vec<&'static str> = Vec::new();

    for agent in agents::AGENTS.iter() {
        let before = agent.health(&project, home.as_deref());
        if before.is_empty() {
            continue;
        }
        if !fix {
            remaining.extend(before);
            continue;
        }
        let ctx = agents::InstallCtx {
            project: &project,
            home: home.as_deref(),
            scope: agents::Scope::Global,
        };
        if agent.install(&ctx).is_ok() {
            let after = agent.health(&project, home.as_deref());
            if after.is_empty() {
                fixed.push(agent.label());
            } else {
                remaining.extend(after);
            }
        } else {
            remaining.extend(before);
        }
    }

    if remaining.is_empty() && fixed.is_empty() {
        Check {
            name: "config health",
            ok: true,
            detail: "ok".into(),
        }
    } else if remaining.is_empty() {
        Check {
            name: "config health",
            ok: true,
            detail: format!("fixed: {}", fixed.join(", ")),
        }
    } else {
        let fixed_note = if fixed.is_empty() {
            String::new()
        } else {
            format!("fixed: {}; ", fixed.join(", "))
        };
        let hint = if fix {
            ""
        } else {
            " (run with --fix to repair)"
        };
        Check {
            name: "config health",
            ok: false,
            detail: format!("{fixed_note}{}{hint}", remaining.join("; ")),
        }
    }
}

/// Warn when the configured token expires within 7 days (or has already
/// expired) so a user notices before every request starts 401ing.
fn check_token_expiry() -> Check {
    let (project_id, _) = crate::project::detect_project();
    let resolved = crate::config::resolve(project_id.as_deref());
    let Some((token, _)) = resolved.token else {
        return Check {
            name: "token expiry",
            ok: true,
            detail: "(no token configured)".into(),
        };
    };
    let Some(info) = crate::status::decode_jwt_info(&token) else {
        return Check {
            name: "token expiry",
            ok: true,
            detail: "opaque token (cannot check expiry)".into(),
        };
    };
    match info.exp {
        None => Check {
            name: "token expiry",
            ok: true,
            detail: "no expiry claim (long-lived token)".into(),
        },
        Some(exp) => {
            let days_left = (exp - chrono::Utc::now().timestamp()) as f64 / 86400.0;
            if days_left < 0.0 {
                Check {
                    name: "token expiry",
                    ok: false,
                    detail: format!(
                        "EXPIRED {:.1} day(s) ago -- mint a fresh token from the dashboard's \
                         You > Tokens page and run `cairn onboard --token <jwt>`",
                        -days_left
                    ),
                }
            } else if days_left < 7.0 {
                Check {
                    name: "token expiry",
                    ok: false,
                    detail: format!(
                        "expires in {days_left:.1} day(s) -- mint a fresh token from the \
                         dashboard's You > Tokens page and run `cairn onboard --token <jwt>` soon"
                    ),
                }
            } else {
                Check {
                    name: "token expiry",
                    ok: true,
                    detail: format!("expires in {days_left:.0} day(s)"),
                }
            }
        }
    }
}

/// Informational: how far client and server versions have drifted. Skew alone isn't a
/// failure (rolling upgrades are normal) - this exists so a confusing bug report can be
/// ruled in/out by version mismatch at a glance.
fn check_version_skew() -> Check {
    let (project_id, _) = crate::project::detect_project();
    let resolved = crate::config::resolve(project_id.as_deref());
    let Some((server, _)) = resolved.server else {
        return Check {
            name: "version",
            ok: true,
            detail: "(no server configured)".into(),
        };
    };
    let client_version = env!("CARGO_PKG_VERSION");
    let client = crate::http::ApiClient::new(&server, None);
    match client.server_version() {
        Some(server_version) if server_version == client_version => Check {
            name: "version",
            ok: true,
            detail: format!("client and server both v{client_version}"),
        },
        Some(server_version) => Check {
            name: "version",
            ok: true,
            detail: format!(
                "client v{client_version}, server v{server_version} (re-run `cairn setup` after a major upgrade if things look off)"
            ),
        },
        None => Check {
            name: "version",
            ok: true,
            detail: "server did not respond to /api/health".into(),
        },
    }
}

/// Informational: entries queued because a hook couldn't reach the server. A genuinely
/// unreachable server is already caught by the "remote server" check above; this just makes
/// the backlog itself visible.
fn check_spool_backlog() -> Check {
    let depth = crate::spool::depth();
    if depth == 0 {
        Check {
            name: "spool",
            ok: true,
            detail: "empty".into(),
        }
    } else {
        Check {
            name: "spool",
            ok: true,
            detail: format!(
                "{depth} entr{} queued for replay (flushes on next reachable SessionStart)",
                if depth == 1 { "y" } else { "ies" }
            ),
        }
    }
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
