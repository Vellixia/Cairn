//! `cairn onboard` - zero-prompt setup for first-run installs.
//!
//! 1. **Verify the binary** - `cairn doctor` (connectivity + diagnostics).
//! 2. **Detect agents** - `cairn setup --all` for every supported agent.
//! 3. **Print a summary** - what was detected, what was wired, what the next step is.
//!
//! Pass `--server <url>` and `--token <jwt>` to configure remote access.
//! Re-running is safe and shows "Re-onboarding" to signal the incremental update.

use anyhow::{Context, Result};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::doctor;

#[derive(Debug, Default)]
pub struct OnboardOptions {
    /// Skip agent auto-detection and wiring (useful for CI).
    pub skip_agents: bool,
    /// Run `doctor --fix` on failures before reporting green.
    pub fix: bool,
    /// Remote server URL - sets `CAIRN_SERVER` for the spawned `setup` subprocess.
    pub server: Option<String>,
    /// Remote server token - sets `CAIRN_TOKEN` for the spawned `setup` subprocess.
    pub token: Option<String>,
}

pub fn run(opts: OnboardOptions) -> Result<()> {
    let reonboarding = is_reonboarding();

    if reonboarding {
        eprintln!("[cairn]  Re-onboarding - updating agent wiring\n");
    } else {
        eprintln!("[cairn]  Cairn onboard - zero-prompt setup\n");
    }

    let interactive = atty_stdout();
    let mut diag = doctor::run(doctor::DoctorOptions {
        fix: opts.fix,
        interactive,
        json: false,
    });

    // If --fix is set and we got failures, re-run to confirm.
    if opts.fix && !diag.ok() {
        diag = doctor::run(doctor::DoctorOptions {
            fix: false,
            interactive,
            json: false,
        });
    }

    if !diag.ok() {
        eprintln!("\ncairn onboard: doctor reported failures; aborting before wiring agents.");
        eprintln!("Re-run with --fix to attempt auto-repair, or fix the items above manually.");
        std::process::exit(diag.exit_code());
    }
    eprintln!("[x] doctor: green\n");

    // 2. Wire agents.
    if opts.skip_agents {
        eprintln!("-> Skipping agent wiring (--skip-agents).\n");
    } else {
        eprintln!("-> Detecting & wiring supported agents...");
        let wired = wire_agents(&opts)?;
        if wired == 0 {
            eprintln!("  no supported agents detected (run `cairn setup <agent>` to add one)");
        } else {
            eprintln!("[x] wired {wired} agent(s)\n");
        }
    }

    // 3. Summary.
    eprintln!("Done. Next steps:");
    if let Some(s) = &opts.server {
        eprintln!("  - server: {s}");
    }
    eprintln!("  - open a session in your AI agent (Claude Code, OpenCode, Codex)");
    eprintln!("  - check status with `cairn status`");

    Ok(())
}

/// Detect whether any supported agent already has a cairn config.
fn is_reonboarding() -> bool {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let home = match home.as_ref() {
        Some(h) => h.as_path(),
        None => return false,
    };

    // Claude Code project-scoped .mcp.json (commonest location).
    if has_cairn_entry(home.join(".mcp.json")) {
        return true;
    }
    if has_cairn_entry(home.join(".claude").join(".mcp.json")) {
        return true;
    }
    // Codex hooks — written alongside the config.
    if home.join(".codex").join("hooks.json").exists() {
        return true;
    }
    // OpenCode.
    let opencode_cfg = home.join("opencode").join("opencode.json");
    if opencode_cfg.exists() {
        // Could parse and check, but existence under the known path is sufficient.
        return true;
    }
    false
}

fn has_cairn_entry(path: impl AsRef<Path>) -> bool {
    let p = path.as_ref();
    if !p.exists() {
        return false;
    }
    // Quick check: read first few KB looking for "cairn".
    let Ok(content) = std::fs::read_to_string(p) else {
        return false;
    };
    content.contains("\"cairn\"")
}

fn wire_agents(opts: &OnboardOptions) -> Result<usize> {
    // Spawn `cairn setup --all --server <url> --token <tok>` as a subprocess so it picks up
    // the same arg parsing + env that an interactive user would have. We never want to
    // duplicate the wiring logic in two places.
    let current = std::env::current_exe().context("locating current cairn binary")?;
    let mut cmd = Command::new(&current);
    cmd.arg("setup").arg("--all");
    if let Some(s) = &opts.server {
        cmd.arg("--server").arg(s);
    }
    if let Some(t) = &opts.token {
        cmd.arg("--token").arg(t);
    }
    // If the caller did not pass --server/--token but the env vars are set, pass them
    // explicitly to guarantee the subprocess sees them even if the shell strips them.
    if opts.server.is_none() {
        if let Ok(s) = std::env::var("CAIRN_SERVER") {
            cmd.env("CAIRN_SERVER", s);
        }
    }
    if opts.token.is_none() {
        if let Ok(t) = std::env::var("CAIRN_TOKEN") {
            cmd.env("CAIRN_TOKEN", t);
        }
    }
    let out = cmd.output().context("spawning cairn setup --all")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Count "[x] Configured" markers - that's how many agents we wired.
    let wired = stdout.matches("Configured").count();
    if !out.status.success() && wired == 0 {
        anyhow::bail!(
            "setup --all exited with status {}: {}{}",
            out.status,
            stderr,
            if stderr.is_empty() {
                stdout.as_ref()
            } else {
                ""
            }
        );
    }
    print!("{}", stdout);
    if !stderr.is_empty() {
        eprint!("{}", stderr);
    }
    Ok(wired)
}

fn atty_stdout() -> bool {
    // We can't import the `atty` crate without a new dep; std::io::IsTerminal does the same
    // thing on stable Rust 1.70+.
    std::io::stdout().is_terminal()
}
