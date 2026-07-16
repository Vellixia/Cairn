//! Shared credential resolution + persistence for `cairn setup`.
//!
//! Resolution order: CLI flags → `CAIRN_SERVER`/`CAIRN_TOKEN` env vars
//! → `~/.cairn/config.toml`. If both server and token are found, validates
//! the token against the live server, persists both to config, and warns
//! once if `CAIRN_TOKEN` env shadows the saved value.

use anyhow::Result;

/// Resolve server+token from CLI flags, env vars, or config file (in that
/// order). When both are found, validates the token against the server and
/// persists both to `~/.cairn/config.toml`. Warns once if `CAIRN_TOKEN` env
/// var shadows the saved value. When only one is found, falls back to config
/// for the missing piece.
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

    // Partial credentials: fill in missing pieces from config.
    let resolved = crate::config::resolve(None);
    let config_server = resolved.server.map(|(s, _)| s);
    let config_token = resolved.token.map(|(t, _)| t);

    let effective_server = server.or(config_server);
    let effective_token = token.or(config_token);

    Ok((effective_server, effective_token))
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
    fn partial_creds_falls_back_to_config() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_string_lossy().into_owned();
        env_guard::with_env(
            &[
                ("HOME", Some(&home)),
                ("USERPROFILE", Some(&home)),
                ("CAIRN_SERVER", Some("http://env.example.com")),
                ("CAIRN_TOKEN", None),
                ("XDG_CONFIG_HOME", None),
            ],
            || {
                let (srv, tok) = resolve_and_persist(None, None).unwrap();
                assert_eq!(srv.unwrap(), "http://env.example.com");
                // No token in env or config → None
                assert!(tok.is_none());
            },
        );
    }

    #[test]
    fn empty_env_server_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_string_lossy().into_owned();
        env_guard::with_env(
            &[
                ("HOME", Some(&home)),
                ("USERPROFILE", Some(&home)),
                ("CAIRN_SERVER", Some("")),
                ("CAIRN_TOKEN", Some("env-token")),
                ("XDG_CONFIG_HOME", None),
            ],
            || {
                let (srv, tok) = resolve_and_persist(None, None).unwrap();
                assert!(srv.is_none());
                assert_eq!(tok.unwrap(), "env-token");
            },
        );
    }
}
