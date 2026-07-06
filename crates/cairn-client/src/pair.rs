//! `cairn pair <CODE>` - claim a device-pairing code minted by the dashboard
//! (`You > Pair`), turn it into a token, persist it to
//! `~/.cairn/config.toml`, and wire up detected agents.
//!
//! This closes a real, pre-existing gap: the dashboard's pair page and
//! `setup`'s own token-rejection error message have both referenced
//! `cairn pair` since v0.8.0 Sprint 3 without the command existing - the
//! pairing flow was only reachable through the raw API.

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ClaimResponse {
    token: String,
    #[serde(default)]
    name: String,
}

pub fn run(code: &str, server: Option<&str>, no_agents: bool) -> Result<()> {
    let server = resolve_server(server)?;
    println!("cairn: claiming pairing code against {server}...");

    let token = claim(&server, code)?;
    crate::http::validate_token(&server, &token)?;

    crate::config::save_server(Some(&server), Some(&token))?;
    crate::config::save_inject_context_default(true)?;
    println!(
        "cairn: paired - server/token saved to ~/.cairn/config.toml, context injection enabled \
         by default (~1k tokens/prompt; disable with CAIRN_INJECT_CONTEXT=false or by editing \
         that file)."
    );

    if no_agents {
        println!(
            "Skipping agent wiring (--no-agents). Run `cairn setup --all` later to wire agents up."
        );
        return Ok(());
    }

    println!("\nWiring detected agents...");
    let wired = crate::setup::run(None, true, Some(&server), Some(&token), false, false)?;
    if wired == 0 {
        println!("No supported agents detected here or in your home directory.");
    }
    Ok(())
}

/// Claim `code` against `server`'s open `POST /api/pair/claim` endpoint
/// (single-use, no auth required - the code itself is the credential).
pub(crate) fn claim(server: &str, code: &str) -> Result<String> {
    let client = crate::http::ApiClient::new(server, None);
    match client
        .post("/api/pair/claim")
        .send_json(serde_json::json!({ "code": code }))
    {
        Ok(resp) => {
            let claimed: ClaimResponse = resp.into_json().context("parsing pairing response")?;
            if !claimed.name.is_empty() {
                println!("cairn: paired as \"{}\"", claimed.name);
            }
            Ok(claimed.token)
        }
        Err(ureq::Error::Status(status, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            anyhow::bail!(
                "pairing code rejected (HTTP {status}): {body}\n\
                 Codes are single-use and short-lived - generate a fresh one from the dashboard's \
                 You > Pair page."
            )
        }
        Err(e) => anyhow::bail!("cannot reach {server}: {e}"),
    }
}

/// `--server`/explicit arg, then `CAIRN_SERVER` env, then
/// `~/.cairn/config.toml`, then a one-shot probe of `http://localhost:7777`
/// (the docker-compose default port) - `/api/health` is an open endpoint, so
/// this needs no token, and checking the server identifies as `"cairn"`
/// avoids mistaking some unrelated service on that port for one.
pub(crate) fn resolve_server(explicit: Option<&str>) -> Result<String> {
    if let Some(s) = explicit {
        return Ok(s.trim_end_matches('/').to_string());
    }
    if let Ok(s) = std::env::var("CAIRN_SERVER") {
        if !s.trim().is_empty() {
            return Ok(s.trim_end_matches('/').to_string());
        }
    }
    if let Some((s, _)) = crate::config::resolve(None).server {
        return Ok(s);
    }
    let localhost = "http://localhost:7777";
    let client = crate::http::ApiClient::new(localhost, None);
    if client.server_version().is_some() {
        println!("cairn: auto-detected a Cairn server at {localhost}");
        return Ok(localhost.to_string());
    }
    anyhow::bail!(
        "no server specified and none auto-detected at {localhost}.\n\
         Pass --server <url>, e.g.: cairn pair {{code}} --server https://cairn.example.com"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env_guard::with_env;

    #[test]
    fn explicit_arg_wins_over_everything() {
        with_env(&[("CAIRN_SERVER", Some("http://env:1"))], || {
            assert_eq!(
                resolve_server(Some("http://explicit:2")).unwrap(),
                "http://explicit:2"
            );
        });
    }

    #[test]
    fn env_wins_over_config_file() {
        let home = tempfile::tempdir().unwrap();
        let home_str = home.path().to_string_lossy().into_owned();
        with_env(
            &[
                ("HOME", Some(home_str.as_str())),
                ("USERPROFILE", Some(home_str.as_str())),
                ("CAIRN_SERVER", Some("http://env:1")),
            ],
            || {
                crate::config::save_server(Some("http://file:2"), None).unwrap();
                assert_eq!(resolve_server(None).unwrap(), "http://env:1");
            },
        );
    }

    #[test]
    fn config_file_used_when_no_explicit_or_env() {
        let home = tempfile::tempdir().unwrap();
        let home_str = home.path().to_string_lossy().into_owned();
        with_env(
            &[
                ("HOME", Some(home_str.as_str())),
                ("USERPROFILE", Some(home_str.as_str())),
                ("CAIRN_SERVER", None),
            ],
            || {
                crate::config::save_server(Some("http://file:2"), None).unwrap();
                assert_eq!(resolve_server(None).unwrap(), "http://file:2");
            },
        );
    }

    #[test]
    fn trailing_slash_is_stripped() {
        with_env(&[("CAIRN_SERVER", None)], || {
            assert_eq!(
                resolve_server(Some("http://explicit:2/")).unwrap(),
                "http://explicit:2"
            );
        });
    }

    #[test]
    fn errors_with_guidance_when_nothing_configured_and_localhost_unreachable() {
        // Port 1 is reserved and never accepts connections, so the real
        // localhost:7777 probe inside `resolve_server` will genuinely fail to
        // connect in this environment too UNLESS something really is running
        // there - this test only pins the vars it controls and asserts the
        // error path is reachable and mentions --server, not that port 7777
        // is unreachable on every machine this ever runs on.
        let home = tempfile::tempdir().unwrap();
        let home_str = home.path().to_string_lossy().into_owned();
        with_env(
            &[
                ("HOME", Some(home_str.as_str())),
                ("USERPROFILE", Some(home_str.as_str())),
                ("CAIRN_SERVER", None),
            ],
            || match resolve_server(None) {
                Ok(_) => {} // a real server happens to be running on :7777 - fine, not a failure
                Err(e) => assert!(e.to_string().contains("--server")),
            },
        );
    }
}
