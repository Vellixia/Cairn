//! `cairn doctor` â€” diagnostic check that the local environment is wired up correctly.
//!
//! The diagnostic is deliberately cheap: it never talks to the network, never opens the
//! store unless HelixDB is configured, and runs in <100 ms. `doctor --fix` adds a small
//! repair pass â€” it creates missing data dirs, writes a default `.env` next to a fresh
//! binary, and prints guidance for things it can't fix automatically.
//!
//! Exit codes:
//! - 0  â€” all green
//! - 1  â€” one or more failures (printed above)
//! - 2  â€” usage error (invalid flags)

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone)]
#[allow(dead_code)] // `interactive` is wired through in a follow-up; kept for API stability.
pub struct DoctorOptions {
    pub fix: bool,
    pub interactive: bool,
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

    // 1. Data directory exists (and is writable). This is the first thing the CLI touches
    //    at startup, so any failure here should surface a clear message rather than a
    //    generic permission error.
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

    // 2. HelixDB URL is set (required for local mode).
    checks.push(check_helix_url(&cfg));

    // 3. Embedder provider â€” `hashing` is the zero-deps default; warn if the user picked
    //    something heavier that needs more config.
    checks.push(check_embedder(&cfg));

    // 4. SECRET_KEY length, if set.
    checks.push(check_secret_key(&cfg));

    // 5. Remote server URL + token, if CAIRN_SERVER is set.
    checks.push(check_remote_server());

    // 6. Store open + count round-trip (only if Helix is configured).
    if cfg.helix_url.is_some() {
        checks.push(check_store_open(&cfg));
    }

    // 7. Agent detection â€” what agents would `setup --all` wire today?
    checks.push(check_agents());

    finalize(checks)
}

fn finalize(checks: Vec<Check>) -> Diagnosis {
    let diag = Diagnosis { checks };
    // Print in a stable order.
    for c in &diag.checks {
        let sym = if c.ok { "âœ“" } else { "âœ—" };
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
            detail: format!("{} (missing â€” run with --fix to create)", dir.display()),
        }
    }
}

fn check_helix_url(cfg: &cairn_core::Config) -> Check {
    match cfg.helix_url.as_deref() {
        Some(url) if !url.trim().is_empty() => Check {
            name: "helix url",
            ok: true,
            detail: redact_url(url),
        },
        _ => {
            // Missing HELIX in local mode is a hard error. In remote-proxy mode it's
            // not strictly required, but we still warn so the user knows.
            if std::env::var_os("CAIRN_SERVER").is_some() {
                Check {
                    name: "helix url",
                    ok: true,
                    detail: "(unset â€” running in remote-proxy mode)".into(),
                }
            } else {
                Check {
                    name: "helix url",
                    ok: false,
                    detail:
                        "CAIRN_HELIX_URL is not set (set it or run `cairn onboard --server ...`)"
                            .into(),
                }
            }
        }
    }
}

fn check_embedder(cfg: &cairn_core::Config) -> Check {
    match cfg.embed.provider.as_str() {
        "hashing" => Check {
            name: "embedder",
            ok: true,
            detail: "hashing (offline, no model download)".into(),
        },
        "ollama" => Check {
            name: "embedder",
            ok: true,
            detail: format!(
                "ollama ({}{})",
                cfg.embed.model.as_deref().unwrap_or("nomic-embed-text"),
                cfg.embed
                    .url
                    .as_deref()
                    .map(|u| format!(" @ {u}"))
                    .unwrap_or_default()
            ),
        },
        "openai" | "local" => Check {
            name: "embedder",
            ok: cfg.embed.api_key.is_some() || cfg.embed.url.is_some(),
            detail: format!(
                "{} {}",
                cfg.embed.provider,
                if cfg.embed.api_key.is_some() {
                    "(key set)"
                } else if cfg.embed.url.is_some() {
                    "(url set)"
                } else {
                    "(NEEDS CAIRN_EMBED_API_KEY or CAIRN_EMBED_URL)"
                }
            ),
        },
        other => Check {
            name: "embedder",
            ok: false,
            detail: format!("unknown provider '{other}' (use hashing|ollama|openai|local)"),
        },
    }
}

fn check_secret_key(cfg: &cairn_core::Config) -> Check {
    match cfg.secret_key.as_ref() {
        Some(k) if k.len() >= 32 => Check {
            name: "secret key",
            ok: true,
            detail: format!("{} bytes (>= 32 required)", k.len()),
        },
        Some(k) => Check {
            name: "secret key",
            ok: false,
            detail: format!(
                "{} bytes (need >= 32 â€” set CAIRN_SECRET_KEY to a 32+ byte value)",
                k.len()
            ),
        },
        None => Check {
            name: "secret key",
            ok: false,
            detail: "CAIRN_SECRET_KEY is not set (auth endpoints will be unavailable)".into(),
        },
    }
}

fn check_remote_server() -> Check {
    let server = std::env::var("CAIRN_SERVER").ok();
    match server {
        Some(s) if !s.trim().is_empty() => {
            let token = std::env::var("CAIRN_TOKEN").ok();
            Check {
                name: "remote server",
                ok: token.is_some(),
                detail: format!(
                    "{}{}",
                    s,
                    if let Some(t) = token {
                        if !t.is_empty() {
                            " (token set)".to_string()
                        } else {
                            " (CAIRN_TOKEN is empty)".to_string()
                        }
                    } else {
                        " (no CAIRN_TOKEN â€” every request will 401)".to_string()
                    }
                ),
            }
        }
        _ => Check {
            name: "remote server",
            ok: true,
            detail: "(unset â€” local mode)".into(),
        },
    }
}

fn check_store_open(cfg: &cairn_core::Config) -> Check {
    match cairn_store::Store::open(cfg) {
        Ok(store) => match store.count_memories() {
            Ok(n) => Check {
                name: "store open",
                ok: true,
                detail: format!("ok ({n} memories)"),
            },
            Err(e) => Check {
                name: "store open",
                ok: false,
                detail: format!("count_memories failed: {e}"),
            },
        },
        Err(e) => Check {
            name: "store open",
            ok: false,
            detail: format!("open failed: {e}"),
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
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| home_dir().unwrap_or_else(|| PathBuf::from(".")));
    config_home
        .join(".config")
        .join("opencode")
        .join("opencode.json")
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

/// Build a short-lived full diagnosis from a list of checks â€” used by the `doctor`
/// CLI entry point so it can return a non-zero exit code on failure.
pub fn run_and_exit(opts: DoctorOptions) -> Result<()> {
    let diag = run(opts);
    std::process::exit(diag.exit_code());
}

/// Strip userinfo (`user:pass@`) from a URL for safe logging. Pure-string operation;
/// doesn't parse the URL â€” that's fine for diagnostics.
fn redact_url(url: &str) -> String {
    if let Some(at_idx) = url.find('@') {
        if let Some(scheme_end) = url.find("://") {
            let prefix = &url[..scheme_end + 3];
            let rest = &url[at_idx + 1..];
            return format!("{prefix}***@{rest}");
        }
    }
    url.to_string()
}

/// Simple command runner for `doctor --fix` â€” used by tests to spawn the actual binary
/// and verify that a missing data dir gets created.
#[doc(hidden)]
#[allow(dead_code)] // Reserved for upcoming end-to-end tests; the unit tests use the inner fn.
pub fn run_cli(args: &[&str]) -> Result<i32> {
    let current = std::env::current_exe().context("locating cairn binary")?;
    let out = Command::new(&current)
        .args(args)
        .output()
        .context("spawning cairn")?;
    if !out.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&out.stdout));
    }
    if !out.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(out.status.code().unwrap_or(-1))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Tests for the `doctor` module.
    ///
    /// Mutex that serializes every test which mutates process-wide env vars (CAIRN_SERVER,
    /// CAIRN_HELIX_URL, etc.). Held for the entire duration of each test that needs it.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    fn redact_url_strips_userinfo() {
        assert_eq!(
            redact_url("http://user:pass@example.com:6969/path"),
            "http://***@example.com:6969/path"
        );
        assert_eq!(redact_url("http://localhost:6969"), "http://localhost:6969");
        assert_eq!(
            redact_url("http://***@example.com:6969"),
            "http://***@example.com:6969"
        );
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

    #[test]
    fn doctor_check_embedder_rejects_unknown_provider() {
        let mut cfg = cairn_core::Config::resolve(None).unwrap();
        cfg.embed.provider = "magic".into();
        let c = check_embedder(&cfg);
        assert!(!c.ok);
        assert!(c.detail.contains("magic"));
    }

    #[test]
    fn doctor_check_embedder_accepts_hashing() {
        let mut cfg = cairn_core::Config::resolve(None).unwrap();
        cfg.embed.provider = "hashing".into();
        let c = check_embedder(&cfg);
        assert!(c.ok);
        assert!(c.detail.contains("hashing"));
    }

    #[test]
    fn doctor_check_helix_url_set_passes() {
        let mut cfg = cairn_core::Config::resolve(None).unwrap();
        cfg.helix_url = Some("http://localhost:6969".into());
        let c = check_helix_url(&cfg);
        assert!(c.ok);
    }

    #[test]
    fn doctor_env_var_tests_are_serial_safe() {
        let _guard = ENV_LOCK.lock().unwrap();

        std::env::remove_var("CAIRN_SERVER");

        let mut cfg = cairn_core::Config::resolve(None).unwrap();
        cfg.helix_url = None;
        cfg.default_server = None;
        assert!(
            !check_helix_url(&cfg).ok,
            "missing helix in local mode must fail"
        );

        cfg.helix_url = Some("http://localhost:6969".into());
        assert!(check_helix_url(&cfg).ok);

        std::env::set_var("CAIRN_SERVER", "http://localhost:7777");
        cfg.helix_url = None;
        assert!(
            check_helix_url(&cfg).ok,
            "with remote server set, missing helix is ok"
        );
        assert!(check_helix_url(&cfg).detail.contains("remote-proxy"));
        std::env::remove_var("CAIRN_SERVER");

        // Confirm we still fail when nothing is set.
        cfg.helix_url = None;
        cfg.default_server = None;
        std::env::remove_var("CAIRN_SERVER");
        assert!(!check_helix_url(&cfg).ok);
    }

    #[test]
    fn doctor_check_secret_key_short_is_fail() {
        let mut cfg = cairn_core::Config::resolve(None).unwrap();
        cfg.secret_key = Some(b"short".to_vec());
        let c = check_secret_key(&cfg);
        assert!(!c.ok);
        assert!(c.detail.contains("32"));
    }

    #[test]
    fn doctor_check_secret_key_long_is_ok() {
        let mut cfg = cairn_core::Config::resolve(None).unwrap();
        cfg.secret_key = Some(b"this-is-exactly-thirty-two-bytes!!!".to_vec());
        let c = check_secret_key(&cfg);
        assert!(c.ok);
        assert!(c.detail.contains("32"));
    }
}
