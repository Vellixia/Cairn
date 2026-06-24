//! Runtime configuration and on-disk layout.
//!
//! Settings resolve with precedence (highest â†’ lowest):
//!
//! 1. **CLI flag** â€” e.g. `--host`, `--port`, `--data-dir`.
//! 2. **Real environment variable** â€” whatever the parent shell already exported.
//! 3. **Project `.env`** â€” `<repo>/.env` (or the `environment:` block in `docker-compose.yml`).
//! 4. **Global `.env`** â€” `~/.config/cairn/.env` on Linux,
//!    `$XDG_CONFIG_HOME/cairn/.env` elsewhere, `%APPDATA%\\cairn\\.env` on Windows
//!    (see [`global_env_path`]).
//! 5. **Built-in default** â€” the hard-coded fallback inside [`Config::resolve`].
//!
//! The split between CLI / core is intentional: the core crate reads raw `std::env::var`, and the
//! `cairn` binary loads both `.env` files at startup via `dotenvy` (see
//! `crates/cairn/src/main.rs`). `dotenvy` only fills variables that are not already set, so
//! real env always wins over a project `.env`, and a project `.env` always wins over the global
//! one. A machine-global `.env` lets you configure embeddings/Helix once for every project on the
//! device ("global cairn").

use std::path::{Path, PathBuf};

/// Single-admin account settings (web dashboard auth).
///
/// Resolution priority (highest â†’ lowest):
/// 1. `CAIRN_ADMIN_PASSWORD_HASH` â€” pre-hashed (Argon2id PHC).
/// 2. `CAIRN_ADMIN_PASSWORD` â€” plaintext; refused on non-loopback binds unless
///    `CAIRN_INSECURE=1` is set, mirroring the existing TLS gate.
/// 3. Server starts in setup mode â€” `/setup` wizard accepts the first admin.
///
/// Note: the *persisted* admin record (with its generation counter and hash) is stored in the
/// meta store under key `admin`, not in config. These fields only describe the *bootstrap*
/// inputs.
#[derive(Debug, Clone)]
pub struct AdminConfig {
    pub username: String,
    pub password_hash: Option<String>,
    pub password: Option<String>,
    pub session_ttl_hours: u64,
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            username: "admin".to_string(),
            password_hash: None,
            password: None,
            session_ttl_hours: 24,
        }
    }
}

/// Embedding-model settings (used to vectorize memories for Helix's vector search).
#[derive(Clone)]
pub struct EmbedConfig {
    /// `local` (default), `openai`, or `ollama`.
    pub provider: String,
    /// Model id; defaults per provider (local â†’ `all-MiniLM-L6-v2`).
    pub model: Option<String>,
    /// Base URL for `ollama` / OpenAI-compatible providers.
    pub url: Option<String>,
    /// API key for hosted providers.
    pub api_key: Option<String>,
}

impl std::fmt::Debug for EmbedConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbedConfig")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("url", &self.url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
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
/// (`~/.local/share/cairn`, `%APPDATA%\cairn`, â€¦); overridable via flags and env (`CAIRN_*`).
#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    /// Serve bind host (`CAIRN_HOST`, default `127.0.0.1`).
    pub host: String,
    /// Serve bind port (`CAIRN_PORT`, default `7777`).
    pub port: u16,
    /// HelixDB server URL (`CAIRN_HELIX_URL`).
    pub helix_url: Option<String>,
    /// HelixDB bearer API key (`CAIRN_HELIX_TOKEN`). Sent as `Authorization: Bearer <token>` on
    /// every HelixDB request. Optional â€” HelixDB instances without auth don't need it.
    pub helix_token: Option<String>,
    /// Label-namespace prefix for the HelixDB backend (`CAIRN_HELIX_NS`). Lets multiple Cairn
    /// instances â€” or isolated tests â€” share one Helix server without colliding. Default `cairn_`.
    pub helix_ns: Option<String>,
    /// Default remote Cairn server for `sync` / `pull` / `contribute` (`CAIRN_SERVER`).
    pub default_server: Option<String>,
    /// HMAC secret used to sign device-token JWTs (`CAIRN_SECRET_KEY`).
    pub secret_key: Option<Vec<u8>>,
    /// Optional TLS material for HTTPS serve (`CAIRN_TLS_CERT` + `CAIRN_TLS_KEY`).
    ///
    /// Network-exposed serve (`host` other than `127.0.0.1` / `localhost` / `::1`) requires this
    /// to be set unless `CAIRN_INSECURE=1` is also set; the API layer will refuse to start over
    /// plain HTTP on a non-loopback bind.
    pub tls: Option<TlsConfig>,
    /// When `true`, allow plain HTTP on a non-loopback bind (`CAIRN_INSECURE=1`). Intended only
    /// for local/private Docker Compose setups where TLS is handled by a reverse proxy or is
    /// genuinely unnecessary.
    pub insecure: bool,
    /// Optional project/workspace root used by context engines (`CAIRN_WORKSPACE_ROOT`).
    pub workspace_root: Option<PathBuf>,
    /// Allowed CORS origins (`CAIRN_CORS_ORIGINS`, comma-separated). Empty means same-origin only;
    /// `"*"` means permissive (with a startup warning). Default: empty.
    pub cors_origins: Vec<String>,
    /// Embedding settings.
    pub embed: EmbedConfig,
    /// Admin account settings.
    pub admin: AdminConfig,
    /// Multi-tenant mode (v0.5.0 Sprint 19). When `true`, every memory is
    /// tagged with the bearer token's org id; queries are scoped to the caller's
    /// org. When `false` (default for self-hosted installs), all memories share
    /// a single implicit org â€” `OrgId::default()` â€” so the on-disk schema doesn't
    /// change for existing users.
    pub multi_tenant: bool,
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
            helix_token: env_str("CAIRN_HELIX_TOKEN"),
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
            insecure: env_bool("CAIRN_INSECURE"),
            workspace_root: env_path("CAIRN_WORKSPACE_ROOT"),
            cors_origins: env_str("CAIRN_CORS_ORIGINS")
                .map(|s| {
                    s.split(',')
                        .map(|o| o.trim().to_string())
                        .filter(|o| !o.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            embed: EmbedConfig {
                provider: env_str("CAIRN_EMBED_PROVIDER").unwrap_or_else(|| "local".to_string()),
                model: env_str("CAIRN_EMBED_MODEL"),
                url: env_str("CAIRN_EMBED_URL"),
                api_key: env_str("CAIRN_EMBED_API_KEY"),
            },
            admin: AdminConfig {
                username: env_str("CAIRN_ADMIN_USERNAME").unwrap_or_else(|| "admin".to_string()),
                password_hash: env_str("CAIRN_ADMIN_PASSWORD_HASH"),
                password: env_str("CAIRN_ADMIN_PASSWORD"),
                session_ttl_hours: env_str("CAIRN_ADMIN_SESSION_TTL_HOURS")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(24),
            },
            multi_tenant: env_bool("CAIRN_MULTI_TENANT"),
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
        // If the host is a DNS name we can't prove loopback-ness â€” assume non-loopback so the
        // safe-by-default TLS gate kicks in.
        false
    }
}

/// Path to the machine-global `.env` (OS config dir) â€” loaded at CLI startup for "global cairn".
pub fn global_env_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "cairn", "cairn").map(|d| d.config_dir().join(".env"))
}

fn env_str(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn env_bool(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
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
            helix_token: None,
            helix_ns: None,
            default_server: None,
            secret_key: None,
            tls: None,
            insecure: false,
            workspace_root: None,
            cors_origins: vec![],
            embed: EmbedConfig {
                provider: "local".into(),
                model: None,
                url: None,
                api_key: None,
            },
            admin: AdminConfig::default(),
            multi_tenant: false,
        }
    }

    #[test]
    fn loopback_hosts_are_recognised() {
        for host in [
            "127.0.0.1",
            "::1",
            "localhost",
            "LOCALHOST",
            "  127.0.0.1  ",
        ] {
            assert!(
                cfg_with_host(host).is_loopback_host(),
                "{host} should be loopback"
            );
        }
    }

    #[test]
    fn non_loopback_hosts_are_rejected() {
        for host in [
            "0.0.0.0",
            "192.168.1.5",
            "10.0.0.1",
            "cairn.example.com",
            "",
        ] {
            assert!(
                !cfg_with_host(host).is_loopback_host(),
                "{host} should NOT be loopback"
            );
        }
    }

    #[test]
    fn embed_config_debug_redacts_api_key() {
        let cfg = EmbedConfig {
            provider: "openai".into(),
            model: Some("text-embedding-3-small".into()),
            url: Some("https://api.openai.com".into()),
            api_key: Some("sk-super-secret-key-12345".into()),
        };
        let debug = format!("{:?}", cfg);
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("sk-super-secret-key-12345"));
        assert!(debug.contains("openai"));
    }

    #[test]
    fn admin_config_default_username() {
        assert_eq!(AdminConfig::default().username, "admin");
        assert_eq!(AdminConfig::default().session_ttl_hours, 24);
    }
}
