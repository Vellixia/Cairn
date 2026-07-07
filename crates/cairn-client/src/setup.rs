//! `cairn setup [agent|--all]` - wire AI agents up to a Cairn server.
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
//! `--all` configures only the agents it actually detects (project markers or home-dir install);
//! naming an agent explicitly configures it regardless.

use crate::agents::{self, InstallCtx, Scope};
use crate::paths;
use anyhow::{anyhow, Result};
use std::path::Path;

/// Resolve server+token from flags/env/config, return the number of agents
/// actually installed so `cairn onboard` can report a count.
pub fn run(
    agent: Option<&str>,
    all: bool,
    server: Option<&str>,
    token: Option<&str>,
    project_flag: bool,
) -> Result<usize> {
    let (effective_server, effective_token) =
        crate::credentials::resolve_and_persist(server, token)?;

    let project = std::env::current_dir()?;
    let home = paths::home_dir();

    let scope = if project_flag {
        Scope::Project
    } else {
        Scope::Global
    };

    if all {
        let detected = agents::detect_all(&project, home.as_deref());
        for a in &detected {
            install_one(*a, &project, home.as_deref(), scope)?;
        }
        if detected.is_empty() {
            println!("cairn: no supported agents detected here or in your home directory.");
            println!("Install one explicitly, e.g. `cairn setup claude-code`.");
            println!("Supported: {}.", agents::ids().join(", "));
        } else if let Some(srv) = effective_server.as_deref() {
            if effective_token.is_some() {
                println!("\nCairn server: {srv}. Open a session in your agent.");
            } else {
                println!("\nCairn server: {srv} (not persisted; pass --token to save it).");
            }
        } else {
            println!("\nNo server configured. Run with --server <url> or set CAIRN_SERVER.");
        }
        return Ok(detected.len());
    }

    let requested = agent.unwrap_or("claude-code");
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
    Ok(1)
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
    crate::rules::write_for(a.id(), project)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_agent_name_is_rejected_with_supported_list() {
        // Pin CAIRN_SERVER/CAIRN_TOKEN unset so this can never attempt a real
        // network validation call if the ambient shell happens to export them.
        crate::env_guard::with_env(&[("CAIRN_SERVER", None), ("CAIRN_TOKEN", None)], || {
            let err = run(Some("emacs"), false, None, None, false).unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains("emacs"));
            assert!(msg.contains("claude-code"));
        });
    }
}
