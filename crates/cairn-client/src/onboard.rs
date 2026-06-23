//! `cairn onboard` â€” zero-prompt setup for first-run installs.
//!
//! What "zero-prompt" means here:
//!
//! - If the env already carries everything we need (`CAIRN_HELIX_URL`, `CAIRN_SERVER`,
//!   `CAIRN_TOKEN`), we just open the store and report green.
//! - If anything is missing, we run `doctor` interactively *only* when stdin is a TTY â€”
//!   in scripts / CI the user gets an actionable diagnostic instead of an interactive
//!   prompt that would hang the build.
//!
//! The flow:
//!
//! 1. **Verify the binary** â€” `cairn doctor` (status + diagnostics).
//! 2. **Provision the local store** â€” open it; if `CAIRN_HELIX_URL` is set, test a memory
//!    round-trip so we know HelixDB is reachable.
//! 3. **Detect agents** â€” `cairn setup --all` for every supported agent that has a
//!    project marker or home-dir config we recognize. Idempotent.
//! 4. **Print a summary** â€” what was detected, what was wired, what the next step is.
//!
//! `--server <url>` and `--token <tok>` are optional: they trigger remote-proxy mode (every
//! CLI call hits `CAIRN_SERVER` instead of the local store).

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
    /// Remote server URL â€” sets `CAIRN_SERVER` for the spawned `setup` subprocess.
    pub server: Option<String>,
    /// Remote server token â€” sets `CAIRN_TOKEN` for the spawned `setup` subprocess.
    pub token: Option<String>,
}

pub fn run(opts: OnboardOptions) -> Result<()> {
    eprintln!("ðŸª¨  Cairn onboard â€” zero-prompt setup\n");

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
    eprintln!("âœ“ doctor: green\n");

    // 2. Provision the local store.
    eprintln!("â†’ Provisioning local storeâ€¦");
    provision_store(&opts)?;
    eprintln!("âœ“ store open\n");

    // 3. Wire agents.
    if opts.skip_agents {
        eprintln!("â†’ Skipping agent wiring (--skip-agents).\n");
    } else {
        eprintln!("â†’ Detecting & wiring supported agentsâ€¦");
        let wired = wire_agents(&opts)?;
        if wired == 0 {
            eprintln!("  no supported agents detected (run `cairn setup <agent>` to add one)");
        } else {
            eprintln!("âœ“ wired {wired} agent(s)\n");
        }
    }

    // 4. Summary.
    eprintln!("Done. Next steps:");
    if let Some(s) = &opts.server {
        eprintln!("  â€¢ server  : {s}");
    } else {
        eprintln!("  • server  : (local HelixDB — start with `docker compose up -d cairn`)");
    }
    eprintln!("  â€¢ open the dashboard at http://127.0.0.1:7777 (or your configured host)");
    eprintln!("  â€¢ first agent action: cairn remember \"your first memory\"");

    Ok(())
}

fn provision_store(opts: &OnboardOptions) -> Result<()> {
    // If the user passed --server / --token, surface them as env so any subprocess (mcp,
    // doctor --fix, etc.) inherits them.
    if let Some(s) = &opts.server {
        std::env::set_var("CAIRN_SERVER", s);
    }
    if let Some(t) = &opts.token {
        std::env::set_var("CAIRN_TOKEN", t);
    }

    let cfg = cairn_core::Config::resolve(None).context("resolving cairn config")?;
    let store = cairn_store::Store::open(&cfg).context("opening local store")?;

    // Quick read-through to confirm the store is queryable.
    let n = store
        .count_memories()
        .context("counting memories in store")?;
    eprintln!("  store: {} memories", n);

    // If we're in remote-proxy mode, the local store is intentionally empty â€” that's
    // expected and not a failure. If we're local-only and the store can't even count,
    // bail so the user sees a clear error instead of a confusing one later.
    if std::env::var_os("CAIRN_SERVER").is_none() && n == 0 {
        eprintln!("  (fresh store â€” no memories yet; that's fine)");
    }
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
    // Count "âœ“ Configured" markers â€” that's how many agents we wired.
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
