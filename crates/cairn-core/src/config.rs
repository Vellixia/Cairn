//! Runtime configuration and on-disk layout.
//!
//! Settings resolve with precedence **CLI flag > environment (incl. `.env`) > default**. A
//! machine-global `.env` in the OS config dir lets you configure embeddings/Helix once for every
//! project on the device ("global cairn"); a project `.env` overrides it; real environment
//! variables override both. The CLI loads those `.env` files at startup (see [`global_env_path`]).

use std::path::{Path, PathBuf};

/// Embedding-model settings (used to vectorize memories for Helix's vector search).
#[derive(Debug, Clone)]
pub struct EmbedConfig {
    /// `local` (default), `openai`, or `ollama`.
    pub provider: String,
    /// Model id; defaults per provider (local → `all-MiniLM-L6-v2`).
    pub model: Option<String>,
    /// Base URL for `ollama` / OpenAI-compatible providers.
    pub url: Option<String>,
    /// API key for hosted providers.
    pub api_key: Option<String>,
}

/// TLS material for HTTPS serve. Both `cert` and `key` must be present to enable TLS; partial
/// configuration (e.g. cert without key) is rejected by the API layer at startup.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// PEM-encoded TLS certificate chain (`CAIRN_TLS_CERT`).
    pub cert: PathBuf,
    /// PEM-encoded TLS private key (`CAIRN_TLS_KEY`).
    pub key: PathBuf,
}

/// Where Cairn keeps its data and how it's reached. Defaults to the OS data dir
/// (`~/.local/share/cairn`, `%APPDATA%\cairn`, …); overridable via flags and env (`CAIRN_*`).
#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    /// Serve bind host (`CAIRN_HOST`, default `127.0.0.1`).
    pub host: String,
    /// Serve bind port (`CAIRN_PORT`, default `7777`).
    pub port: u16,
    /// HelixDB server URL (`CAIRN_HELIX_URL`).
    pub helix_url: Option<String>,
    /// Label-namespace prefix for the HelixDB backend (`CAIRN_HELIX_NS`). Lets multiple Cairn
    /// instances — or isolated tests — share one Helix server without colliding. Default `cairn_`.
    pub helix_ns: Option<String>,
    /// Default remote Cairn server for `sync` / `pull` / `contribute` (`CAIRN_SERVER`).
    pub default_server: Option<String>,
    /// HMAC secret used to sign device-token JWTs (`CAIRN_SECRET_KEY`).
    pub secret_key: Option<Vec<u8>>,
    /// Optional TLS material for HTTPS serve (`CAIRN_TLS_CERT` + `CAIRN_TLS_KEY`).
    ///
    /// Network-exposed serve (`host` other than `127.0.0.1` / `localhost` / `::1`) requires this
    /// to be set; the API layer will refuse to start over plain HTTP on a non-loopback bind.
    pub tls: Option<TlsConfig>,
    /// Optional project/workspace root used by context engines (`CAIRN_WORKSPACE_ROOT`).
    pub workspace_root: Option<PathBuf>,
    /// Embedding settings.
    pub embed: EmbedConfig,
}

impl Config {
    /// Resolve config (creating the data dir). `data_dir` is the `--data-dir` flag, taking
    /// precedence over `CAIRN_DATA_DIR`, then the OS default.
    pub fn resolve(data_dir: Option<PathBuf>) -> crate::Result<Self> {
        let data_dir = data_dir
            .or_else(|| env_path("CAIRN_DATA_DIR"))
            .unwrap_or_else(default_data_dir);
        std::fs::create_dir_all(&data_dir)?;

        let cfg = Self {
            host: env_str("CAIRN_HOST").unwrap_or_else(|| "127.0.0.1".to_string()),
            port: env_str("CAIRN_PORT")
                .and_then(|p| p.parse().ok())
                .unwrap_or(7777),
            helix_url: env_str("CAIRN_HELIX_URL"),
            helix_ns: env_str("CAIRN_HELIX_NS"),
            default_server: env_str("CAIRN_SERVER"),
            secret_key: env_str("CAIRN_SECRET_KEY").map(|s| s.into_bytes()),
            tls: match (env_path("CAIRN_TLS_CERT"), env_path("CAIRN_TLS_KEY")) {
                (Some(cert), Some(key)) => Some(TlsConfig { cert, key }),
                (None, None) => None,
                // Partial TLS config is almost always a misconfiguration that would later fail
                // obscurely at handshake time. Surface it loudly here so it can't be missed.
                _ => {
                    return Err(crate::Error::Invalid(
                        "CAIRN_TLS_CERT and CAIRN_TLS_KEY must be set together".into(),
                    ));
                }
            },
            workspace_root: env_path("CAIRN_WORKSPACE_ROOT"),
            embed: EmbedConfig {
                provider: env_str("CAIRN_EMBED_PROVIDER").unwrap_or_else(|| "local".to_string()),
                model: env_str("CAIRN_EMBED_MODEL"),
                url: env_str("CAIRN_EMBED_URL"),
                api_key: env_str("CAIRN_EMBED_API_KEY"),
            },
            data_dir,
        };
        std::fs::create_dir_all(cfg.blobs_dir())?;
        Ok(cfg)
    }

    pub fn blobs_dir(&self) -> PathBuf {
        self.data_dir.join("blobs")
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// True if the configured serve bind host is a loopback address (`127.0.0.1`, `::1`, or
    /// `localhost`). Used by the API layer to gate TLS enforcement: non-loopback binds MUST serve
    /// HTTPS, loopback binds are still allowed to serve plain HTTP for local dev.
    pub fn is_loopback_host(&self) -> bool {
        let h = self.host.trim();
        if h.eq_ignore_ascii_case("localhost") {
            return true;
        }
        if let Ok(ip) = h.parse::<std::net::IpAddr>() {
            return ip.is_loopback();
        }
        // If the host is a DNS name we can't prove loopback-ness — assume non-loopback so the
        // safe-by-default TLS gate kicks in.
        false
    }
}

/// Path to the machine-global `.env` (OS config dir) — loaded at CLI startup for "global cairn".
pub fn global_env_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "cairn", "cairn").map(|d| d.config_dir().join(".env"))
}

fn env_str(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn env_path(key: &str) -> Option<PathBuf> {
    env_str(key).map(PathBuf::from)
}

fn default_data_dir() -> PathBuf {
    if let Some(dirs) = directories::ProjectDirs::from("dev", "cairn", "cairn") {
        dirs.data_dir().to_path_buf()
    } else {
        PathBuf::from(".cairn")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_host(host: &str) -> Config {
        Config {
            host: host.to_string(),
            // The rest of these fields are not relevant to is_loopback_host(); populate with
            // placeholders so the struct literal stays exhaustive.
            data_dir: std::env::temp_dir(),
            port: 7777,
            helix_url: None,
            helix_ns: None,
            default_server: None,
            secret_key: None,
            tls: None,
            workspace_root: None,
            embed: EmbedConfig {
                provider: "local".into(),
                model: None,
                url: None,
                api_key: None,
            },
        }
    }

    #[test]
    fn loopback_hosts_are_recognised() {
        for host in ["127.0.0.1", "::1", "localhost", "LOCALHOST", "  127.0.0.1  "] {
            assert!(cfg_with_host(host).is_loopback_host(), "{host} should be loopback");
        }
    }

    #[test]
    fn non_loopback_hosts_are_rejected() {
        for host in ["0.0.0.0", "192.168.1.5", "10.0.0.1", "cairn.example.com", ""] {
            assert!(!cfg_with_host(host).is_loopback_host(), "{host} should NOT be loopback");
        }
    }
}
