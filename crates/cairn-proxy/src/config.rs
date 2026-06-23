//! Proxy configuration — a TOML-shaped list of upstream peers + bind options.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerEntry {
    /// Display name for the peer — used in logs and the dashboard.
    pub name: String,
    /// Base URL of the upstream registry. Must be reachable over the network
    /// (TLS in production).
    pub base_url: String,
    /// Optional bearer token sent as `Authorization: Bearer <token>`.
    pub token: Option<String>,
    /// Skip this peer on errors instead of failing the whole request. Default: true.
    #[serde(default = "default_true")]
    pub best_effort: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// Host to bind (default `127.0.0.1`).
    #[serde(default = "default_host")]
    pub host: String,
    /// Port to bind (default `7778`).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Upstream peers.
    pub peers: Vec<PeerEntry>,
}

fn default_host() -> String {
    "127.0.0.1".into()
}
fn default_port() -> u16 {
    7778
}

impl ProxyConfig {
    /// Load from a TOML file. Returns an error if the file is missing or malformed.
    pub fn from_toml_file(path: &Path) -> Result<Self, crate::ProxyError> {
        let s =
            std::fs::read_to_string(path).map_err(|e| crate::ProxyError::Config(e.to_string()))?;
        let cfg: ProxyConfig =
            toml::from_str(&s).map_err(|e| crate::ProxyError::Config(e.to_string()))?;
        if cfg.peers.is_empty() {
            return Err(crate::ProxyError::Config(
                "peers list must not be empty".into(),
            ));
        }
        Ok(cfg)
    }

    /// A reasonable default for tests / smoke runs: a single local cairn-server
    /// on its default port.
    pub fn single_local_peer() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 7778,
            peers: vec![PeerEntry {
                name: "local".into(),
                base_url: "http://127.0.0.1:7777".into(),
                token: None,
                best_effort: true,
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_local_peer_has_one_upstream() {
        let cfg = ProxyConfig::single_local_peer();
        assert_eq!(cfg.peers.len(), 1);
        assert_eq!(cfg.peers[0].name, "local");
        assert!(cfg.peers[0].best_effort);
    }

    #[test]
    fn from_toml_file_rejects_empty_peers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.toml");
        std::fs::write(
            &path,
            r#"
host = "127.0.0.1"
port = 7778
peers = []
"#,
        )
        .unwrap();
        let err = ProxyConfig::from_toml_file(&path).unwrap_err();
        assert!(err.to_string().contains("peers list must not be empty"));
    }
}
