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
use crate::http::validate_token;
use crate::paths;
use anyhow::{anyhow, Result};
use std::path::Path;

/// Returns the number of agents actually installed, so callers (`cairn
/// onboard`, `cairn pair`) can report a wired-agent count without spawning a
/// subprocess and scraping its stdout for it.
pub fn run(
    agent: Option<&str>,
    all: bool,
    server: Option<&str>,
    token: Option<&str>,
    project_flag: bool,
    embed_env: bool,
) -> Result<usize> {
    // Fall back to CAIRN_SERVER/CAIRN_TOKEN env vars when the flags are not passed explicitly.
    let env_server = server
        .is_none()
        .then(|| std::env::var("CAIRN_SERVER").ok())
        .flatten();
    let effective_server = server.or(env_server.as_deref());

    let env_token = token
        .is_none()
        .then(|| std::env::var("CAIRN_TOKEN").ok())
        .flatten()
        .filter(|t| !t.is_empty());
    let effective_token = token.or(env_token.as_deref());

    // Validate once, up front, no matter how many agents get wired below. `setup --all` used to
    // make one identical validation round-trip per detected agent; a bad token now fails fast
    // before any config file is touched, instead of failing on whichever agent happened to be
    // installed first.
    if let (Some(srv), Some(tok)) = (effective_server, effective_token) {
        validate_token(srv, tok)?;
    }

    // v0.8.0 client redesign: persist server/token to `~/.cairn/config.toml` so `cairn hook`
    // and `cairn mcp` work without every agent config file embedding them (see `config.rs`,
    // `hook.rs`). This is what makes `embed_env: false` (the default just below) safe - the
    // values aren't lost, they just live in one shared file instead of N agent-specific ones.
    // A brand-new config.toml also means a brand-new user: turn the flagship context-injection
    // feature on for them (existing installs that never touch this again keep the in-binary
    // default of off - see `hook.rs`).
    if effective_server.is_some() || effective_token.is_some() {
        let is_fresh_config = crate::config::config_path().is_some_and(|p| !p.exists());
        crate::config::save_server(effective_server, effective_token)?;
        if is_fresh_config {
            crate::config::save_inject_context_default(true)?;
            println!(
                "cairn: wrote server/token to ~/.cairn/config.toml and enabled context injection \
                 by default (adds ~1k tokens/prompt; disable with CAIRN_INJECT_CONTEXT=false or by \
                 editing that file)."
            );
        }
    }

    let project = std::env::current_dir()?;
    let home = paths::home_dir();

    // `--project` overrides the default global scope for Claude Code so the user can opt into
    // per-project config when they want it. Other agents ignore scope because their config
    // locations are inherently user-level (Codex: ~/.codex; OpenCode: ~/.config/opencode).
    let scope = if project_flag {
        Scope::Project
    } else {
        Scope::Global
    };

    if all {
        let detected = agents::detect_all(&project, home.as_deref());
        for a in &detected {
            install_one(
                *a,
                &project,
                home.as_deref(),
                scope,
                effective_server,
                effective_token,
                embed_env,
            )?;
        }
        if detected.is_empty() {
            println!("cairn: no supported agents detected here or in your home directory.");
            println!("Install one explicitly, e.g. `cairn setup claude-code`.");
            println!("Supported: {}.", agents::ids().join(", "));
        } else if let Some(srv) = effective_server {
            println!("\nCairn server: {srv}. Open a session in your agent.");
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
    install_one(
        a,
        &project,
        home.as_deref(),
        scope,
        effective_server,
        effective_token,
        embed_env,
    )?;
    if let Some(srv) = effective_server {
        println!("\nCairn server: {srv}. Open a session in your agent.");
    } else {
        println!("\nNo server configured. Run with --server <url> or set CAIRN_SERVER.");
    }
    Ok(1)
}

#[allow(clippy::too_many_arguments)]
fn install_one(
    a: &dyn agents::Agent,
    project: &Path,
    home: Option<&Path>,
    scope: Scope,
    server: Option<&str>,
    token: Option<&str>,
    embed_env: bool,
) -> Result<()> {
    let ctx = InstallCtx {
        project,
        home,
        scope,
        server,
        token,
        embed_env,
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
            let err = run(Some("emacs"), false, None, None, false, false).unwrap_err();
            let msg = err.to_string();
            assert!(msg.contains("emacs"));
            assert!(msg.contains("claude-code"));
        });
    }
}
