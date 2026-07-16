//! `cairn setup [agent]` - wire AI agents up to a Cairn server.
//!
//! Every merge is **non-destructive**: existing config is preserved and our entries are added
//! idempotently (running twice changes nothing). Per-agent file formats and locations live in
//! `crate::agents` - this module just resolves flags/env, validates the token once, and drives
//! the `agents::AGENTS` registry.
//!
//! When `--server`/`--token` are passed, the MCP server entry includes `CAIRN_SERVER`/
//! `CAIRN_TOKEN` env vars so `cairn mcp` runs in remote-proxy mode; otherwise it runs in local
//! mode.
//!
//! Without an explicit agent name, all detected agents are configured. Naming an agent
//! explicitly configures only that one.

use crate::agents::{self, InstallCtx, Scope};
use crate::paths;
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

/// Resolve server+token from flags/env/config, return the number of agents
/// actually installed.
pub fn run(
    agent: Option<&str>,
    server: Option<&str>,
    token: Option<&str>,
    project_flag: bool,
) -> Result<usize> {
    let (effective_server, effective_token) = resolve_with_fallback(server, token)?;

    let project = std::env::current_dir()?;
    let home = paths::home_dir();

    let scope = if project_flag {
        Scope::Project
    } else {
        Scope::Global
    };

    if let Some(requested) = agent {
        let a = agents::find(requested).ok_or_else(|| {
            anyhow!(
                "unknown agent '{requested}'. Supported: {}.",
                agents::ids().join(", ")
            )
        })?;
        install_one(a, &project, home.as_deref(), scope)?;
        if let Some(srv) = effective_server.as_deref() {
            if effective_token.is_some() {
                println!("\nCairn server: {srv}. Open a session in your agent.");
            } else {
                println!("\nCairn server: {srv} (not persisted; pass --token to save it).");
            }
        } else {
            println!("\nNo server configured. Run with --server <url> or set CAIRN_SERVER.");
        }
        return Ok(1);
    }

    // No agent specified: detect and configure all.
    if is_reonboarding() {
        eprintln!("[cairn]  Re-onboarding - updating agent wiring\n");
    }

    let detected = agents::detect_all(&project, home.as_deref());
    for a in &detected {
        install_one(*a, &project, home.as_deref(), scope)?;
    }
    if detected.is_empty() {
        println!("cairn: no supported agents detected here or in your home directory.");
        println!("Install one explicitly, e.g. `cairn setup claude-code`.");
        println!("Supported: {}.", agents::ids().join(", "));
    } else {
        eprintln!("\u{2713} wired {} agent(s)\n", detected.len());
    }

    eprintln!("Done. Next steps:");
    if let Some(srv) = effective_server.as_deref() {
        if effective_token.is_some() {
            eprintln!("  - server: {srv}");
        } else {
            eprintln!("  - server: {srv} (not persisted; pass --token to save it)");
        }
    } else {
        eprintln!("  - no server configured yet -- Cairn is running in local-only mode");
        eprintln!(
            "  - to connect to a server: mint a token from the dashboard's You > Tokens \
                   page, then run `cairn setup --server <url> --token <jwt>`"
        );
    }
    eprintln!("  - open a session in your AI agent (Claude Code, OpenCode, Codex)");
    eprintln!("  - check status with `cairn status`");

    Ok(detected.len())
}

fn resolve_with_fallback(
    server: Option<&str>,
    token: Option<&str>,
) -> Result<(Option<String>, Option<String>)> {
    let (server, token) = crate::credentials::resolve_and_persist(server, token)?;

    if server.is_some() || token.is_some() {
        return Ok((server, token));
    }

    let localhost = "http://localhost:7777";
    let client = crate::http::ApiClient::new(localhost, None);
    if client.server_version().is_some() {
        eprintln!(
            "cairn: found a Cairn server at {localhost} but no token configured.\n\
             Open {localhost}/you/tokens in your browser to mint a token, then run:\n\
             \n  cairn setup --server {localhost} --token <jwt>\n\
             \nContinuing setup without a server for now (local-only checks)."
        );
    } else {
        eprintln!(
            "cairn: no Cairn server configured (checked --server/--token flags, \
             CAIRN_SERVER/CAIRN_TOKEN env vars, ~/.cairn/config.toml, and {localhost}) \
             -- continuing in local-only mode.\n\
             To connect to a server: mint a token from the dashboard's You > Tokens page, \
             then run `cairn setup --server <url> --token <jwt>`.\n\
             If local-only mode is what you want, no action needed."
        );
    }
    Ok((None, None))
}

fn install_one(
    a: &dyn agents::Agent,
    project: &Path,
    home: Option<&Path>,
    scope: Scope,
) -> Result<()> {
    let ctx = InstallCtx {
        project,
        home,
        scope,
    };
    let report = a.install(&ctx)?;
    report.print(a.label());
    crate::rules::write_for(a.id(), project, home)?;
    Ok(())
}

/// True when any agent already has a cairn-owned entry in its config.
fn is_reonboarding() -> bool {
    let project = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = crate::paths::home_dir();
    crate::agents::AGENTS.iter().any(|a| {
        a.removal_plan(&project, home.as_deref())
            .iter()
            .any(|action| action.would_change().unwrap_or(false))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_agent_name_is_rejected_with_supported_list() {
        crate::env_guard::with_env(&[("CAIRN_SERVER", None), ("CAIRN_TOKEN", None)], || {
            let err = run(Some("emacs"), None, None, false).unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains("emacs"));
            assert!(msg.contains("claude-code"));
        });
    }
}
