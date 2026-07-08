//! Shared credential resolution + persistence for `cairn setup`.
//!
//! Resolution order: CLI flags → `CAIRN_SERVER`/`CAIRN_TOKEN` env vars
//! → `~/.cairn/config.toml`. If both server and token are found, validates
//! the token against the live server, persists both to config, and warns
//! once if `CAIRN_TOKEN` env shadows the saved value.

use anyhow::Result;

/// Resolve server+token from CLI flags, env vars, or config file (in that
/// order). If both are found, validates the token against the server and
/// persists both to `~/.cairn/config.toml`. Warns once if `CAIRN_TOKEN` env
/// var shadows the saved value.
///
/// Returns the effective `(server, token)` — either may be `None`.
pub fn resolve_and_persist(
    cli_server: Option<&str>,
    cli_token: Option<&str>,
) -> Result<(Option<String>, Option<String>)> {
    if let (Some(s), Some(t)) = (cli_server, cli_token) {
        persist(s, t)?;
        return Ok((Some(s.to_owned()), Some(t.to_owned())));
    }

    let cli_server = cli_server.map(|s| s.to_owned());
    let cli_token = cli_token.map(|t| t.to_owned());

    let env_server = std::env::var("CAIRN_SERVER")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let env_token = std::env::var("CAIRN_TOKEN").ok().filter(|t| !t.is_empty());

    let server = cli_server.clone().or(env_server);
    let token = cli_token.clone().or(env_token);

    if let (Some(s), Some(t)) = (&server, &token) {
        persist(s, t)?;
        return Ok((server, token));
    }

    if cli_server.is_none() && cli_token.is_none() && server.is_none() && token.is_none() {
        let resolved = crate::config::resolve(None);
        return Ok((
            resolved.server.map(|(s, _)| s),
            resolved.token.map(|(t, _)| t),
        ));
    }

    Ok((server, token))
}

fn persist(server: &str, token: &str) -> Result<()> {
    crate::http::validate_token(server, token)?;
    let is_fresh = crate::config::config_path().is_some_and(|p| !p.exists());
    crate::config::save_server(Some(server), Some(token))?;
    crate::config::warn_if_env_token_shadows(token);
    if is_fresh {
        crate::config::save_inject_context_default(true)?;
        eprintln!(
            "cairn: wrote server/token to ~/.cairn/config.toml and enabled context injection \
             by default (adds ~1k tokens/prompt; disable with CAIRN_INJECT_CONTEXT=false or \
             by editing that file)."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env_guard;

    #[test]
    fn partial_creds_returns_partial_no_validate() {
        env_guard::with_env(
            &[
                ("CAIRN_SERVER", Some("http://env.example.com")),
                ("CAIRN_TOKEN", None),
            ],
            || {
                let (srv, tok) = resolve_and_persist(None, None).unwrap();
                assert_eq!(srv.unwrap(), "http://env.example.com");
                assert!(tok.is_none());
            },
        );
    }

    #[test]
    fn empty_env_server_is_ignored() {
        env_guard::with_env(
            &[
                ("CAIRN_SERVER", Some("")),
                ("CAIRN_TOKEN", Some("env-token")),
            ],
            || {
                let (srv, tok) = resolve_and_persist(None, None).unwrap();
                assert!(srv.is_none());
                assert_eq!(tok.unwrap(), "env-token");
            },
        );
    }
}
