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
