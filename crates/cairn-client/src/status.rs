//! `cairn status` - show server connection, token info, and agent status.

use crate::{agents, paths};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize)]
struct Status {
    version: String,
    server: Option<String>,
    server_source: Option<&'static str>,
    token: Option<TokenInfo>,
    token_source: Option<&'static str>,
    inject_context: bool,
    inject_context_source: &'static str,
    config_path: Option<String>,
    spool_depth: usize,
    agents: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct TokenInfo {
    name: String,
    scope: String,
    valid: bool,
    expires: Option<String>,
    /// Raw `exp` claim (unix seconds), alongside the human-formatted `expires` above - `doctor`
    /// needs the raw value to compute "days until expiry" without re-parsing the formatted string.
    #[serde(skip)]
    pub(crate) exp: Option<i64>,
}

pub fn run(json_output: bool) -> Result<()> {
    let project = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (project_id, _) = crate::project::detect_project();
    let resolved = crate::config::resolve(project_id.as_deref());

    let server = resolved.server.as_ref().map(|(s, _)| s.clone());
    let server_source = resolved.server.as_ref().map(|(_, src)| src.label());
    let token_str = resolved.token.as_ref().map(|(t, _)| t.clone());
    let token_source = resolved.token.as_ref().map(|(_, src)| src.label());

    // Decode JWT to extract claim info (no signature verification needed). A
    // non-JWT opaque token still counts as "configured" for validity checks below.
    let mut token_info = token_str.as_deref().and_then(decode_jwt_info);

    // Verify the token against the server (timeout-bounded - status must never hang).
    if let (Some(srv), Some(tok)) = (&server, &token_str) {
        if let Some(ref mut info) = token_info {
            let client = crate::http::ApiClient::new(srv, Some(tok));
            info.valid = matches!(
                client.get("/api/memory/wakeup").query("limit", "1").call(),
                Ok(resp) if resp.status() == 200
            );
        }
    }

    let agents = detect_agents(&project);

    let status = Status {
        version: env!("CARGO_PKG_VERSION").to_string(),
        server,
        server_source,
        token: token_info,
        token_source,
        inject_context: resolved.inject_context.0,
        inject_context_source: resolved.inject_context.1.label(),
        config_path: crate::config::config_path().map(|p| p.display().to_string()),
        spool_depth: crate::spool::depth(),
        agents,
    };

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&status).unwrap_or_default()
        );
    } else {
        println!("Cairn client v{}", env!("CARGO_PKG_VERSION"));
        match (&status.server, status.server_source) {
            (Some(s), Some(src)) => println!("Server:     {s}  (from {src})"),
            _ => println!("Server:     (not configured)"),
        }
        match (&status.token, status.token_source) {
            (Some(t), Some(src)) => {
                let valid = if t.valid { "valid" } else { "INVALID" };
                println!("Token:      {} ({} scope, {valid}, from {src})", t.name, t.scope);
                if let Some(exp) = &t.expires {
                    println!("Expires:    {exp}");
                }
            }
            (None, Some(src)) => println!("Token:      opaque token - cannot decode claims (from {src})"),
            _ => println!("Token:      (not configured)"),
        }
        println!(
            "Inject:     {} (from {})",
            if status.inject_context { "on" } else { "off" },
            status.inject_context_source
        );
        if let Some(p) = &status.config_path {
            println!("Config:     {p}");
        }
        if status.spool_depth > 0 {
            println!(
                "Spool:      {} entr{} queued (offline hooks -- flushes on next reachable SessionStart)",
                status.spool_depth,
                if status.spool_depth == 1 { "y" } else { "ies" }
            );
        }
        if status.agents.is_empty() {
            println!("Agents:     (none detected)");
        } else {
            println!("Agents:     {}", status.agents.join(", "));
        }
        if status.server.is_none() || status.token.is_none() {
            println!("\nRun `cairn onboard --server <url> --token <jwt>` to configure.");
        }
    }

    Ok(())
}

/// Decode the JWT payload (middle section) without signature verification
/// to extract token name and scope for display. `pub(crate)` so `doctor`'s
/// token-expiry check reuses the same decode instead of a second copy.
pub(crate) fn decode_jwt_info(jwt: &str) -> Option<TokenInfo> {
    let payload_b64 = jwt.split('.').nth(1)?;
    let bytes = base64_decode(payload_b64)?;
    #[derive(Deserialize)]
    struct Claims {
        sub: String,
        scope: String,
        #[serde(default)]
        exp: Option<i64>,
    }
    let claims: Claims = serde_json::from_slice(&bytes).ok()?;
    let expires = claims.exp.map(|e| {
        let dt = chrono::DateTime::from_timestamp(e, 0).unwrap_or_default();
        dt.format("%Y-%m-%d %H:%M UTC").to_string()
    });
    Some(TokenInfo {
        name: claims.sub,
        scope: claims.scope,
        valid: false, // will be set by server check
        expires,
        exp: claims.exp,
    })
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .ok()
}

fn detect_agents(project: &std::path::Path) -> Vec<String> {
    let home = paths::home_dir();
    agents::detect_all(project, home.as_deref())
        .iter()
        .map(|a| a.id().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_jwt_info_reads_sub_and_scope() {
        // A real device token minted by the server during manual E2E
        // verification of `cairn pair` (harmless, already-expired-by-then
        // test credential) - decoding it used to return `None` (displaying
        // as "opaque token" in status output) because the old code manually
        // padded the payload with `=` and then fed that into a
        // `URL_SAFE_NO_PAD` decoder, which rejects padding characters.
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJqdGkiOiI2MGIyYmJlNzBhNWE0MWRhOTE0OTU3MzVkMmQ0NTA5ZCIsInN1YiI6ImUyZS10ZXN0IiwiaWF0IjoxNzgzMDc3MTkwLCJzY29wZSI6IndyaXRlIn0.sig";
        let info = decode_jwt_info(jwt).expect("must decode a well-formed JWT payload");
        assert_eq!(info.name, "e2e-test");
        assert_eq!(info.scope, "write");
    }

    #[test]
    fn decode_jwt_info_handles_every_padding_length_class() {
        // Base64url payload length mod 4 varies with claim content length;
        // exercise all three valid remainders (0, 2, 3 - a remainder of 1 is
        // never valid base64) to make sure none of them regress.
        for claims in [
            r#"{"sub":"a","scope":"read"}"#,       // one length class
            r#"{"sub":"ab","scope":"read"}"#,      // another
            r#"{"sub":"abc","scope":"readwrite"}"#, // another
        ] {
            let payload = base64::Engine::encode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                claims.as_bytes(),
            );
            let jwt = format!("eyJhbGciOiJIUzI1NiJ9.{payload}.sig");
            assert!(
                decode_jwt_info(&jwt).is_some(),
                "failed to decode payload of length {} (claims: {claims})",
                payload.len()
            );
        }
    }

    #[test]
    fn decode_jwt_info_returns_none_for_malformed_token() {
        assert!(decode_jwt_info("only-one-part").is_none(), "no '.' separator at all");
        assert!(
            decode_jwt_info("a.not-valid-base64!!!.c").is_none(),
            "payload segment isn't valid base64url"
        );
        assert!(
            decode_jwt_info("a.bm90IGpzb24.c").is_none(),
            "payload decodes to bytes that aren't the expected JSON shape"
        );
    }
}
