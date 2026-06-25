//! `cairn onboard` - zero-prompt setup for first-run installs.
//!
//! 1. **Verify the binary** - `cairn doctor` (status + diagnostics).
//! 2. **Detect agents** - `cairn setup --all` for every supported agent.
//! 3. **Print a summary** - what was detected, what was wired, what the next step is.
//!
//! `--server <url>` and `--token <tok>` control remote-proxy mode.

use anyhow::{Context, Result};
use std::io::IsTerminal;
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
    eprintln!("[cairn]  Cairn onboard - zero-prompt setup\n");

    let interactive = atty_stdout();
    let mut diag = doctor::run(doctor::DoctorOptions {
        fix: opts.fix,
        interactive,
    });

    // If --fix is set and we got failures, re-run to confirm.
    if opts.fix && !diag.ok() {
        diag = doctor::run(doctor::DoctorOptions {
            fix: false,
            interactive,
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
        eprintln!("  - server  : {s}");
    }
    eprintln!("  - open the dashboard at http://127.0.0.1:7777 (or your configured host)");
    eprintln!("  - first agent action: cairn remember \"your first memory\"");

    Ok(())
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
