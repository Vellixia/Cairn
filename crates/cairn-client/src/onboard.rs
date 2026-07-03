//! `cairn onboard` - zero-prompt setup for first-run installs.
//!
//! 1. **Resolve credentials** - `--code` (claim a dashboard pairing code),
//!    `--server`/`--token` flags, env vars, `~/.cairn/config.toml`, or a
//!    one-shot `localhost:7777` probe, in that order.
//! 2. **Verify** - `cairn doctor` (connectivity + diagnostics).
//! 3. **Wire agents** - `setup::run` in-process for every detected agent (no
//!    subprocess, no stdout-scraping for a wired-agent count).
//! 4. **Print a summary.**
//!
//! Re-running is safe and shows "Re-onboarding" to signal the incremental
//! update - `is_reonboarding` below reuses the exact same "would `cairn
//! reset` find anything to remove" check `reset.rs` runs, so the two
//! commands can never disagree about whether Cairn was previously set up.

use anyhow::Result;
use std::io::IsTerminal;
use std::path::PathBuf;

use super::doctor;

#[derive(Debug, Default)]
pub struct OnboardOptions {
    /// Skip agent auto-detection and wiring (useful for CI).
    pub skip_agents: bool,
    /// Run `doctor --fix` on failures before reporting green.
    pub fix: bool,
    /// Claim this dashboard-minted pairing code instead of a raw token.
    pub code: Option<String>,
    /// Remote server URL.
    pub server: Option<String>,
    /// Remote server token (ignored if `code` is set).
    pub token: Option<String>,
}

pub fn run(opts: OnboardOptions) -> Result<()> {
    let reonboarding = is_reonboarding();

    if reonboarding {
        eprintln!("[cairn]  Re-onboarding - updating agent wiring\n");
    } else {
        eprintln!("[cairn]  Cairn onboard - zero-prompt setup\n");
    }

    let (server, token) = resolve_credentials(&opts)?;

    // Validate once, up front, and persist immediately - mirrors `setup::run`'s own
    // validate-then-save sequencing so `onboard` can't wire agents against a token
    // it never actually checked.
    if let (Some(srv), Some(tok)) = (&server, &token) {
        crate::http::validate_token(srv, tok)?;
        let is_fresh_config = crate::config::config_path().is_some_and(|p| !p.exists());
        crate::config::save_server(Some(srv), Some(tok))?;
        if is_fresh_config {
            crate::config::save_inject_context_default(true)?;
        }
    }

    let interactive = std::io::stdout().is_terminal();
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
        let wired = crate::setup::run(None, true, server.as_deref(), token.as_deref(), false, false)?;
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
    }
    eprintln!("  - open a session in your AI agent (Claude Code, OpenCode, Codex)");
    eprintln!("  - check status with `cairn status`");

    Ok(())
}

/// Resolve server+token from, in priority order: `--code` (claim a pairing
/// code via `pair::claim`), `--server`/`--token` flags, `CAIRN_SERVER`/
/// `CAIRN_TOKEN` env, `~/.cairn/config.toml` - or, only when NOTHING else
/// supplied credentials, a one-shot probe of `http://localhost:7777` that
/// prints the dashboard pairing URL if a server answers but isn't yet
/// authenticated against.
fn resolve_credentials(opts: &OnboardOptions) -> Result<(Option<String>, Option<String>)> {
    if let Some(code) = &opts.code {
        let server = crate::pair::resolve_server(opts.server.as_deref())?;
        let token = crate::pair::claim(&server, code)?;
        return Ok((Some(server), Some(token)));
    }
    if opts.server.is_some() || opts.token.is_some() {
        return Ok((opts.server.clone(), opts.token.clone()));
    }
    if let Ok(s) = std::env::var("CAIRN_SERVER") {
        if !s.trim().is_empty() {
            let token = std::env::var("CAIRN_TOKEN").ok().filter(|t| !t.is_empty());
            return Ok((Some(s), token));
        }
    }
    let resolved = crate::config::resolve(None);
    if resolved.server.is_some() {
        return Ok((
            resolved.server.map(|(s, _)| s),
            resolved.token.map(|(t, _)| t),
        ));
    }
    // Nothing configured anywhere - try localhost, the docker-compose default port.
    let localhost = "http://localhost:7777";
    let client = crate::http::ApiClient::new(localhost, None);
    if client.server_version().is_some() {
        eprintln!(
            "cairn: found a Cairn server at {localhost} but no token configured.\n\
             Open {localhost}/you/pair in your browser to generate a pairing code, then run:\n\
             \n  cairn onboard --code <CODE>\n\
             \nContinuing onboard without a server for now (local-only checks)."
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
