//! Shared HTTP client for talking to a Cairn server: one `ureq::Agent` with
//! sane timeouts, Bearer auth, and the handful of calls every subcommand needs
//! (`validate_token`, `server_version`).
//!
//! `hook.rs`'s `RemoteClient` (scope-header-aware, spool-integrated) is a
//! separate, richer client layered on top of this in a later iteration — this
//! module exists first so `setup`/`doctor`/`status` stop making bare, unbounded
//! `ureq::get` calls that can hang a CLI invocation forever if the server never
//! responds.

use anyhow::Result;
use std::time::Duration;

/// Hard ceiling on any single request. Ureq 2.x has no default timeout, so
/// without this a hook or doctor check can hang indefinitely against a server
/// that accepted the TCP connection but never answers.
pub const DEFAULT_TIMEOUT_MS: u64 = 4000;

/// `CAIRN_TIMEOUT_MS` env override, falling back to `DEFAULT_TIMEOUT_MS`. A
/// future `~/.cairn/config.toml` `[hooks] timeout_ms` will feed the same value
/// in through this same function once the config layer lands.
pub fn resolve_timeout() -> Duration {
    let ms = std::env::var("CAIRN_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TIMEOUT_MS);
    Duration::from_millis(ms)
}

pub struct ApiClient {
    agent: ureq::Agent,
    server: String,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(server: &str, token: Option<&str>) -> Self {
        Self::with_timeout(server, token, resolve_timeout())
    }

    pub fn with_timeout(server: &str, token: Option<&str>, timeout: Duration) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(timeout)
            .timeout(timeout)
            .build();
        Self {
            agent,
            server: server.trim_end_matches('/').to_string(),
            token: token.map(str::to_string),
        }
    }

    pub fn get(&self, path: &str) -> ureq::Request {
        self.authed(self.agent.get(&format!("{}{path}", self.server)))
    }

    fn authed(&self, req: ureq::Request) -> ureq::Request {
        match &self.token {
            Some(t) => req.set("Authorization", &format!("Bearer {t}")),
            None => req,
        }
    }

    /// `GET /api/health` — open endpoint, no auth required. Returns the
    /// server's reported version when it answers and identifies as `"cairn"`.
    pub fn server_version(&self) -> Option<String> {
        let resp = self
            .agent
            .get(&format!("{}/api/health", self.server))
            .call()
            .ok()?;
        let v: serde_json::Value = resp.into_json().ok()?;
        if v.get("name").and_then(|n| n.as_str()) != Some("cairn") {
            return None;
        }
        v.get("version")
            .and_then(|s| s.as_str())
            .map(str::to_string)
    }
}

/// The plausible causes of a token rejection, shared between `validate_token`'s standalone
/// error and `doctor`'s terser inline report so the two never describe the same HTTP-level
/// failure in contradictory ways.
pub fn token_rejection_causes() -> &'static str {
    "the token may be expired, revoked, or belong to a server with a different secret key"
}

/// Verify that a device token is valid before writing it to agent config files
/// (or reporting it as healthy). Makes an authenticated
/// `GET /api/memory/wakeup?limit=1` request and returns `Ok(())` when the
/// server answers 200.
pub fn validate_token(server: &str, token: &str) -> Result<()> {
    let client = ApiClient::new(server, Some(token));
    match client.get("/api/memory/wakeup").query("limit", "1").call() {
        Ok(resp) if resp.status() == 200 => Ok(()),
        Ok(resp) => {
            let status = resp.status();
            let body = resp.into_string().unwrap_or_default();
            anyhow::bail!(
                "token rejected by server (HTTP {status}) -- {}.\n\
                 Server response: {body}\n\
                 Obtain a fresh token from the dashboard's You > Tokens page.",
                token_rejection_causes()
            )
        }
        Err(e) => {
            anyhow::bail!(
                "cannot reach server at {server} to validate the token: {e}\n\
                 Is the server running and reachable?"
            )
        }
    }
}
