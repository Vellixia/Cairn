//! `cairn onboard` - zero-prompt setup for first-run installs.
//!
//! 1. **Resolve credentials** - `--server`/`--token` flags, env vars,
//!    `~/.cairn/config.toml`, or a one-shot `localhost:7777` probe, in that order.
//! 2. **Verify** - `cairn doctor` (connectivity + diagnostics).
//! 3. **Wire agents** - `setup::run` in-process for every detected agent (no
//!    subprocess, no stdout-scraping for a wired-agent count).
//! 4. **Print a summary.**
//!
//! Deliberately zero-prompt end to end, INCLUDING when no credentials are found
//! anywhere (see `resolve_credentials`'s final branch): this command must be safe
//! to run unattended in CI, so a missing server/token is reported via stderr
//! guidance and the run proceeds in local-only mode rather than blocking on
//! interactive input. If a user finds the guidance insufficient, improve the
//! printed message - do not add a prompt here.
//!
//! Re-running is safe and shows "Re-onboarding" to signal the incremental
//! update - `is_reonboarding` below reuses the exact same "would `cairn
//! reset` find anything to remove" check `reset.rs` runs, so the two
//! commands can never disagree about whether Cairn was previously set up.

use anyhow::Result;
use std::path::PathBuf;

use super::doctor;

#[derive(Debug, Default)]
pub struct OnboardOptions {
    /// Skip agent auto-detection and wiring (useful for CI).
    pub skip_agents: bool,
    /// Run `doctor --fix` on failures before reporting green.
    pub fix: bool,
    /// Remote server URL.
    pub server: Option<String>,
    /// Remote server token.
    pub token: Option<String>,
}

pub fn run(opts: OnboardOptions) -> Result<()> {
    let reonboarding = is_reonboarding();

    if reonboarding {
        eprintln!("[cairn]  Re-onboarding - updating agent wiring\n");
    } else {
        eprintln!("[cairn]  Cairn onboard - zero-prompt setup\n");
    }

    let (server, token) = resolve_and_persist_with_fallback(&opts)?;

    let mut diag = doctor::run(doctor::DoctorOptions {
        fix: opts.fix,
        json: false,
        server_override: server.clone(),
        token_override: token.clone(),
    });

    // If --fix is set and we got failures, re-run to confirm.
    if opts.fix && !diag.ok() {
        diag = doctor::run(doctor::DoctorOptions {
            fix: false,
            json: false,
            server_override: server.clone(),
            token_override: token.clone(),
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
        let wired = crate::setup::run(None, true, server.as_deref(), token.as_deref(), false)?;
        if wired == 0 {
            eprintln!("  no supported agents detected (run `cairn setup <agent>` to add one)");
        } else {
            eprintln!("[x] wired {wired} agent(s)\n");
        }
    }

    // 3. Summary.
    eprintln!("Done. Next steps:");
    if let Some(s) = &server {
        eprintln!("  - server: {s}");
    } else {
        eprintln!("  - no server configured yet -- Cairn is running in local-only mode");
        eprintln!(
            "  - to connect to a server: mint a token from the dashboard's You > Tokens page, \
             then run `cairn onboard --server <url> --token <jwt>`"
        );
    }
    eprintln!("  - open a session in your AI agent (Claude Code, OpenCode, Codex)");
    eprintln!("  - check status with `cairn status`");

    Ok(())
}

/// Like `crate::credentials::resolve_and_persist`, but when nothing is found
/// via flags/env/config, probes `http://localhost:7777` (the docker-compose
/// default) and prints guidance before returning `(None, None)`.
fn resolve_and_persist_with_fallback(
    opts: &OnboardOptions,
) -> Result<(Option<String>, Option<String>)> {
    let (server, token) =
        crate::credentials::resolve_and_persist(opts.server.as_deref(), opts.token.as_deref())?;

    if server.is_some() || token.is_some() {
        return Ok((server, token));
    }

    // Nothing configured anywhere — try localhost, the docker-compose default port.
    let localhost = "http://localhost:7777";
    let client = crate::http::ApiClient::new(localhost, None);
    if client.server_version().is_some() {
        eprintln!(
            "cairn: found a Cairn server at {localhost} but no token configured.\n\
             Open {localhost}/you/tokens in your browser to mint a token, then run:\n\
             \n  cairn onboard --server {localhost} --token <jwt>\n\
             \nContinuing onboard without a server for now (local-only checks)."
        );
    } else {
        eprintln!(
            "cairn: no Cairn server configured (checked --server/--token flags, \
             CAIRN_SERVER/CAIRN_TOKEN env vars, ~/.cairn/config.toml, and {localhost}) \
             -- continuing in local-only mode.\n\
             To connect to a server: mint a token from the dashboard's You > Tokens page, \
             then run `cairn onboard --server <url> --token <jwt>`.\n\
             If local-only mode is what you want, no action needed."
        );
    }
    Ok((None, None))
}

/// True when any agent already has a cairn-owned entry in its config - the
/// exact same "is there something here to remove" check `cairn reset` uses
/// (`Agent::removal_plan` + `RemovalAction::would_change`), rather than
/// hand-checking a fixed list of paths that can drift from what `setup`
/// actually writes.
fn is_reonboarding() -> bool {
    let project = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = crate::paths::home_dir();
    crate::agents::AGENTS.iter().any(|a| {
        a.removal_plan(&project, home.as_deref())
            .iter()
            .any(|action| action.would_change().unwrap_or(false))
    })
}
