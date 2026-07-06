//! Runtime configuration and on-disk layout.
//!
//! This crate reads `std::env::var` directly - there is no in-process `.env`-file loader (no
//! `dotenvy` or equivalent dependency) in either binary that embeds `cairn-core`
//! (`cairn-server`, `crates/cairn-api/src/bin/cairn_server.rs`). A real environment variable is
//! the only input this crate's [`Config::resolve`] ever sees. `.env` files still work in
//! practice, but the loading happens OUTSIDE this crate: `docker-compose.yml`'s `env_file:`
//! directive reads `.env` and exports its contents as real environment variables before the
//! `cairn` container process starts. Running the server bare-metal (no compose) means exporting
//! the variables yourself (e.g. `set -a; source .env; set +a` before launching `cairn-server`) -
//! nothing here reads the file for you.
//!
//! Settings resolve with precedence (highest -> lowest):
//!
//! 1. **CLI flag** - e.g. `--host`, `--port`, `--data-dir`.
//! 2. **Environment variable** - however it got set (real shell export, or `docker-compose`'s
//!    `env_file:` per above).
//! 3. **Built-in default** - the hard-coded fallback inside [`Config::resolve`].
//!
//! `cairn-client` (the `cairn` CLI agents run) is a separate binary with its own, separate
//! config layer - `~/.cairn/config.toml`, written by `cairn pair`/`onboard`/`setup` - documented
//! in `crates/cairn-client/src/config.rs`, not here.

use std::path::{Path, PathBuf};

/// Single-admin account settings (web dashboard auth).
///
/// Resolution priority (highest -> lowest):
/// 1. `CAIRN_ADMIN_PASSWORD_HASH` - pre-hashed (Argon2id PHC).
/// 2. `CAIRN_ADMIN_PASSWORD` - plaintext; refused on non-loopback binds unless
///    `CAIRN_INSECURE=1` is set, mirroring the existing TLS gate.
/// 3. Server starts in setup mode - `/setup` wizard accepts the first admin.
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

/// Embedding-model settings (used to vectorize memories for the store's vector search).
#[derive(Clone)]
pub struct EmbedConfig {
    /// `local` (default), `openai`, or `ollama`.
    pub provider: String,
    /// Model id; defaults per provider (local -> `all-MiniLM-L6-v2`).
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

/// LLM-driven consolidation settings (P1.4). Gated behind `CAIRN_LLM_CONSOLIDATION=true`
/// (opt-in due to LLM call cost). When enabled, consolidation runs an LLM over session
/// summaries to extract stable facts, reusable procedures, and cross-cutting insights.
#[derive(Clone)]
pub struct LlmConsolidationConfig {
    /// Master gate. `CAIRN_LLM_CONSOLIDATION=true`. Off by default.
    pub enabled: bool,
    /// OpenAI-compatible chat completion endpoint (`CAIRN_LLM_CONSOLIDATION_URL`).
    pub url: String,
    /// Model name (`CAIRN_LLM_CONSOLIDATION_MODEL`).
    pub model: String,
    /// API key for hosted providers (`CAIRN_LLM_CONSOLIDATION_API_KEY`).
    pub api_key: Option<String>,
}

impl std::fmt::Debug for LlmConsolidationConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmConsolidationConfig")
            .field("enabled", &self.enabled)
            .field("url", &self.url)
            .field("model", &self.model)
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

/// Cross-encoder reranking settings (P4.2). The default backend is `none` (no-op pass-through);
/// set `provider=local` + `enabled=true` to enable in-process fastembed-based reranking.
#[derive(Clone)]
pub struct RerankConfig {
    /// Reranking backend. `none` (no-op), `local` (fastembed cross-encoder, gated by
    /// `cairn-rerank/local` feature).
    pub provider: String,
    /// Model id (used by `local` backend). Default: `jina-reranker-v1-turbo-en`.
    pub model: Option<String>,
    /// API key for hosted rerank providers (e.g. Cohere). Reserved for future `http` provider.
    pub api_key: Option<String>,
    /// Master gate (`CAIRN_RERANKER_ENABLED`).
    pub enabled: bool,
    /// How many of the top-MMR results to rerank (`CAIRN_RERANKER_TOP_K`, default 20).
    pub top_k: usize,
    /// Blend weight for cross-encoder vs hybrid score, in [0, 1]
    /// (`CAIRN_RERANKER_BLEND_WEIGHT`, default 0.6). 0 = pure hybrid, 1 = pure cross-encoder.
    pub blend_weight: f32,
}

impl std::fmt::Debug for RerankConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RerankConfig")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("enabled", &self.enabled)
            .field("top_k", &self.top_k)
            .field("blend_weight", &self.blend_weight)
            .finish()
    }
}

impl Default for RerankConfig {
    fn default() -> Self {
        Self {
            provider: "none".to_string(),
            model: None,
            api_key: None,
            enabled: false,
            top_k: 20,
            blend_weight: 0.6,
        }
    }
}

/// Where Cairn keeps its data and how it's reached. Defaults to the OS data dir
/// (`~/.local/share/cairn`, `%APPDATA%\cairn`, ...); overridable via flags and env (`CAIRN_*`).
#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    /// Serve bind host (`CAIRN_HOST`, default `127.0.0.1`).
    pub host: String,
    /// Serve bind port (`CAIRN_PORT`, default `7777`).
    pub port: u16,
    /// SurrealDB connection URL (`CAIRN_DB_URL`, default `ws://localhost:8000`). In the bundled
    /// docker-compose stack this is `ws://surreal:8000` (Docker-internal network name; SurrealDB
    /// is never exposed to the host).
    pub db_url: String,
    /// SurrealDB root/auth username (`CAIRN_DB_USER`, default `root`).
    pub db_user: String,
    /// SurrealDB root/auth password (`CAIRN_DB_PASS`).
    pub db_pass: String,
    /// SurrealDB namespace (`CAIRN_DB_NS`, default `cairn`). Lets multiple Cairn instances - or
    /// isolated tests - share one SurrealDB server without colliding (SurrealDB's namespace is
    /// the natural isolation boundary, replacing HelixDB's label-prefix scheme).
    pub db_ns: String,
    /// Per-query deadline for the SurrealDB client (`CAIRN_DB_TIMEOUT_SECS`, default `10`).
    pub db_timeout_secs: u64,
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
    /// LLM-driven consolidation settings (P1.4). Disabled by default.
    pub llm_consolidation: LlmConsolidationConfig,
    /// Cross-encoder reranking settings (P4.2). Disabled by default.
    pub rerank: RerankConfig,
    /// Admin account settings.
    pub admin: AdminConfig,
    /// Multi-tenant mode (v0.5.0 Sprint 19). When `true`, every memory is
    /// tagged with the bearer token's org id; queries are scoped to the caller's
    /// org. When `false` (default for self-hosted installs), all memories share
    /// a single implicit org - `OrgId::default()` - so the on-disk schema doesn't
    /// change for existing users.
    pub multi_tenant: bool,
    /// Days a `Session`-scoped memory can go untouched before the nightly `session-gc` cron
    /// job promotes it to `Global` scope (`CAIRN_SESSION_TTL_DAYS`, default `2`). `0` disables
    /// the job - Session-scoped memories are then kept forever unless promoted manually.
    pub session_ttl_days: u32,
    /// Confidence half-life for the weekly `memory-decay` cron job
    /// (`CAIRN_DECAY_PERIOD_DAYS`, default `30`) - see
    /// [`cairn_memory::apply_decay`](../../cairn_memory/fn.apply_decay.html).
    pub decay_period_days: u32,
    /// How long `access_log` rows (v0.8.0 Sprint 2) are kept before the monthly
    /// `access-log-prune` cron job deletes them (`CAIRN_ACCESS_LOG_RETENTION_DAYS`, default
    /// `90`).
    pub access_log_retention_days: u32,
    /// Whether the in-process cron scheduler runs at all (`CAIRN_CRON_ENABLED`, default
    /// `true`). Set `false` to disable every background job - useful for a horizontally-scaled
    /// deployment where only one replica should run cron.
    pub cron_enabled: bool,
    /// Promotion auto-threshold (v0.8.0 Sprint 8). A `Project`-scoped memory whose
    /// `promo_score` exceeds this auto-promotes to `Global` on the next `llm-intelligence`
    /// cron run - the `[0.70, 0.90]` human-review band from Sprint 5 only ever applies below
    /// this threshold. (`CAIRN_PROMOTE_THRESHOLD`, default `0.85`).
    pub promote_threshold: f32,
    /// Days an auto-promoted `Global` memory can go unused (by any project) before the nightly
    /// `memory-demote` cron job reverts it back to its original `Project` scope
    /// (`CAIRN_DEMOTE_IDLE_DAYS`, default `45`). Human-promoted/pinned memories are exempt.
    pub demote_idle_days: u32,
    /// Drift auto-approval policy (v0.8.0 Sprint 8): `"safe"` (default - `ok` always
    /// auto-approves, `warn` auto-approves only under `drift_safe_globs`, `danger` never
    /// auto-approves), `"off"` (fully manual), or `"all"` (`ok`+`warn` always auto-approve;
    /// `danger` still never does). (`CAIRN_DRIFT_AUTOPILOT`)
    pub drift_autopilot: String,
    /// Glob patterns considered low-stakes enough for a `warn`-risk drift event to auto-approve
    /// under the `"safe"` policy (`CAIRN_DRIFT_SAFE_GLOBS`, comma-separated, default
    /// `docs/**,*.md,**/tests/**,**/*.test.*`).
    pub drift_safe_globs: Vec<String>,
    /// Whether a session with no manually-set anchor gets one derived automatically from its
    /// first substantive prompt (`CAIRN_AUTO_ANCHOR`, default `true`).
    pub auto_anchor: bool,
    /// Daily token budget for background LLM calls (v0.8.0 Sprint 9) - concept extraction,
    /// contradiction detection, and promotion-scoring's borderline-case judgment step all
    /// check remaining budget first and fall back to their non-LLM heuristic once it's spent,
    /// rather than overspending or hard-failing. `0` means unlimited.
    /// (`CAIRN_LLM_DAILY_BUDGET`, default `200_000`).
    pub llm_daily_budget: u64,
    /// Whether the weekly `tune` cron job nudges `promote_threshold` from measured retrieval
    /// quality (the followup-rate signal - see `cairn_memory::FollowupTracker`). Bounded
    /// (±0.05/week, clamped to `[0.5, 0.95]`) and logged; `false` pins every threshold at its
    /// configured value exactly as in Sprints 1-8. (`CAIRN_SELFTUNE`, default `true`).
    pub selftune: bool,
    /// Cap on Working-tier memories kept per project (v0.8.0 Sprint 9). Beyond this, the
    /// oldest lowest-confidence rows are deleted by the `memory-decay` cron job - a genuine,
    /// permanent deletion via the same `delete_memory` path `DELETE /api/memory/:id` already
    /// uses (Cairn's lossless-retention guarantee covers compressed *file reads*, not the
    /// memory lifecycle - deleting a memory has always been permanent, autopilot or not).
    /// (`CAIRN_MAX_WORKING_PER_PROJECT`, default `500`).
    pub max_working_per_project: usize,
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
            db_url: env_str("CAIRN_DB_URL").unwrap_or_else(|| "ws://localhost:8000".to_string()),
            db_user: env_str("CAIRN_DB_USER").unwrap_or_else(|| "root".to_string()),
            db_pass: env_str("CAIRN_DB_PASS").unwrap_or_default(),
            db_ns: env_str("CAIRN_DB_NS").unwrap_or_else(|| "cairn".to_string()),
            db_timeout_secs: env_str("CAIRN_DB_TIMEOUT_SECS")
                .and_then(|s| s.parse().ok())
                .unwrap_or(10),
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
            llm_consolidation: LlmConsolidationConfig {
                enabled: env_str("CAIRN_LLM_CONSOLIDATION").as_deref() == Some("true"),
                url: env_str("CAIRN_LLM_CONSOLIDATION_URL")
                    .unwrap_or_else(|| "http://localhost:11434/v1/chat/completions".to_string()),
                model: env_str("CAIRN_LLM_CONSOLIDATION_MODEL")
                    .unwrap_or_else(|| "llama3.2".to_string()),
                api_key: env_str("CAIRN_LLM_CONSOLIDATION_API_KEY"),
            },
            rerank: RerankConfig {
                provider: env_str("CAIRN_RERANKER_PROVIDER").unwrap_or_else(|| "none".to_string()),
                model: env_str("CAIRN_RERANKER_MODEL"),
                api_key: env_str("CAIRN_RERANKER_API_KEY"),
                enabled: env_bool("CAIRN_RERANKER_ENABLED"),
                top_k: env_str("CAIRN_RERANKER_TOP_K")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(20),
                blend_weight: env_str("CAIRN_RERANKER_BLEND_WEIGHT")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.6),
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
            session_ttl_days: env_str("CAIRN_SESSION_TTL_DAYS")
                .and_then(|s| s.parse().ok())
                .unwrap_or(2),
            decay_period_days: env_str("CAIRN_DECAY_PERIOD_DAYS")
                .and_then(|s| s.parse().ok())
                .unwrap_or(30),
            access_log_retention_days: env_str("CAIRN_ACCESS_LOG_RETENTION_DAYS")
                .and_then(|s| s.parse().ok())
                .unwrap_or(90),
            cron_enabled: std::env::var("CAIRN_CRON_ENABLED")
                .ok()
                .map(|s| !matches!(s.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no"))
                .unwrap_or(true),
            promote_threshold: env_str("CAIRN_PROMOTE_THRESHOLD")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.85),
            demote_idle_days: env_str("CAIRN_DEMOTE_IDLE_DAYS")
                .and_then(|s| s.parse().ok())
                .unwrap_or(45),
            drift_autopilot: env_str("CAIRN_DRIFT_AUTOPILOT").unwrap_or_else(|| "safe".to_string()),
            drift_safe_globs: env_str("CAIRN_DRIFT_SAFE_GLOBS")
                .map(|s| s.split(',').map(|g| g.trim().to_string()).collect())
                .unwrap_or_else(|| {
                    ["docs/**", "*.md", "**/tests/**", "**/*.test.*"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect()
                }),
            auto_anchor: std::env::var("CAIRN_AUTO_ANCHOR")
                .ok()
                .map(|s| !matches!(s.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no"))
                .unwrap_or(true),
            llm_daily_budget: env_str("CAIRN_LLM_DAILY_BUDGET")
                .and_then(|s| s.parse().ok())
                .unwrap_or(200_000),
            selftune: std::env::var("CAIRN_SELFTUNE")
                .ok()
                .map(|s| {
                    !matches!(
                        s.trim().to_ascii_lowercase().as_str(),
                        "0" | "false" | "off" | "no"
                    )
                })
                .unwrap_or(true),
            max_working_per_project: env_str("CAIRN_MAX_WORKING_PER_PROJECT")
                .and_then(|s| s.parse().ok())
                .unwrap_or(500),
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
        // If the host is a DNS name we can't prove loopback-ness - assume non-loopback so the
        // safe-by-default TLS gate kicks in.
        false
    }
}

/// Path to the machine-global `.env` (OS config dir) - loaded at CLI startup for "global cairn".
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
            db_url: "ws://localhost:8000".into(),
            db_user: "root".into(),
            db_pass: String::new(),
            db_ns: "cairn".into(),
            db_timeout_secs: 10,
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
            llm_consolidation: LlmConsolidationConfig {
                enabled: false,
                url: "http://localhost:11434/v1/chat/completions".into(),
                model: "llama3.2".into(),
                api_key: None,
            },
            rerank: RerankConfig {
                provider: "none".into(),
                model: None,
                api_key: None,
                enabled: false,
                top_k: 20,
                blend_weight: 0.6,
            },
            admin: AdminConfig::default(),
            multi_tenant: false,
            session_ttl_days: 2,
            decay_period_days: 30,
            access_log_retention_days: 90,
            cron_enabled: true,
            promote_threshold: 0.85,
            demote_idle_days: 45,
            drift_autopilot: "safe".to_string(),
            drift_safe_globs: ["docs/**", "*.md", "**/tests/**", "**/*.test.*"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            auto_anchor: true,
            llm_daily_budget: 200_000,
            selftune: true,
            max_working_per_project: 500,
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
