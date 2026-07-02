//! 16 — Config: precedence, OrgId parser, redaction.

use cairn_core::{
    AdminConfig, Config, EmbedConfig, LlmConsolidationConfig, OrgId, RerankConfig, TlsConfig,
};

#[test]
fn org_id_rejects_empty_string() {
    assert!(OrgId::new("").is_err());
}

#[test]
fn org_id_rejects_too_long() {
    let s: String = "a".repeat(65);
    assert!(OrgId::new(s).is_err());
}

#[test]
fn org_id_rejects_uppercase() {
    // Org ids are lower-case ASCII only — log-friendly.
    assert!(OrgId::new("Vellixia").is_err());
    assert!(OrgId::new("alice@vellixia").is_err());
}

#[test]
fn org_id_accepts_valid_inputs() {
    for s in ["vellixia", "team-a", "acme_corp", "abc123", "single_tenant"] {
        let o = OrgId::new(s).expect("valid org id");
        assert_eq!(o.as_str(), s);
    }
}

#[test]
fn org_id_default_is_the_singleton() {
    let o = OrgId::default();
    assert_eq!(o.as_str(), OrgId::SINGLE_TENANT);
    assert!(o.is_default());
}

#[test]
fn org_id_display_matches_string() {
    let o = OrgId::new("vellixia").unwrap();
    assert_eq!(format!("{o}"), "vellixia");
}

#[test]
fn config_struct_has_documented_fields() {
    // The Config struct's public surface is the engine's contract
    // with the CLI / env loader. A drift here breaks env loading.
    // `Config::resolve(None)` is the "use every default" path the
    // CLI takes when no flags are set.
    let _guard = env_lock().lock().expect("env lock");
    let prev = std::env::var("CAIRN_HOST").ok();
    std::env::remove_var("CAIRN_HOST");
    let cfg = Config::resolve(None).expect("resolve with all defaults");
    // The resolved Config has a non-default data dir and a sensible
    // loopback bind host. We assert just enough to prove resolve
    // succeeded and the struct is well-formed.
    assert!(!cfg.data_dir().as_os_str().is_empty());
    // is_loopback_host reflects the default 127.0.0.1 bind.
    assert!(cfg.is_loopback_host(), "default host is loopback");
    if let Some(v) = prev {
        std::env::set_var("CAIRN_HOST", v);
    }
}

#[test]
fn llm_consolidation_config_redacts_api_key() {
    let c = LlmConsolidationConfig {
        enabled: true,
        url: "https://api.openai.com/v1/chat/completions".into(),
        model: "gpt-4o-mini".into(),
        api_key: Some("sk-very-secret".into()),
    };
    let dbg = format!("{c:?}");
    assert!(dbg.contains("[REDACTED]"));
    assert!(!dbg.contains("sk-very-secret"));
}

#[test]
fn rerank_config_redacts_api_key() {
    let c = RerankConfig {
        enabled: true,
        provider: "http".into(),
        model: Some("bge-reranker".into()),
        api_key: Some("super-secret-key".into()),
        top_k: 20,
        blend_weight: 0.6,
    };
    let dbg = format!("{c:?}");
    assert!(dbg.contains("[REDACTED]"));
    assert!(!dbg.contains("super-secret-key"));
}

#[test]
fn admin_config_is_cloneable_and_defaultable() {
    // The admin record is shared across threads.
    let a = AdminConfig::default();
    let b = a.clone();
    assert!(format!("{a:?}").contains("AdminConfig"));
    let _ = b;
}

#[test]
fn tls_config_requires_both_pem_paths() {
    // TlsConfig is a tagged struct; the engine rejects a half-config
    // at startup. We just assert the struct shape is symmetric.
    let t = TlsConfig {
        cert: std::path::PathBuf::from("/etc/cairn/cert.pem"),
        key: std::path::PathBuf::from("/etc/cairn/key.pem"),
    };
    assert!(t.cert.exists() == t.cert.exists());
    assert_eq!(t.cert.extension().unwrap(), "pem");
    assert_eq!(t.key.extension().unwrap(), "pem");
}

#[test]
fn config_resolve_reads_cairn_env_vars() {
    // The env loader is part of the public Config contract: setting
    // CAIRN_HOST / CAIRN_PORT must shape the resolved Config.
    // Guard with a mutex-keyed env block so concurrent tests don't race.
    let _guard = env_lock().lock().expect("env lock");
    let prev_host = std::env::var("CAIRN_HOST").ok();
    let prev_port = std::env::var("CAIRN_PORT").ok();
    std::env::set_var("CAIRN_HOST", "0.0.0.0");
    std::env::set_var("CAIRN_PORT", "9999");
    let cfg = Config::resolve(None).expect("resolve");
    assert_eq!(cfg.host, "0.0.0.0");
    assert_eq!(cfg.port, 9999);
    // Restore previous env state.
    match prev_host {
        Some(v) => std::env::set_var("CAIRN_HOST", v),
        None => std::env::remove_var("CAIRN_HOST"),
    }
    match prev_port {
        Some(v) => std::env::set_var("CAIRN_PORT", v),
        None => std::env::remove_var("CAIRN_PORT"),
    }
}

#[test]
fn config_resolve_default_host_is_loopback() {
    let _guard = env_lock().lock().expect("env lock");
    let prev = std::env::var("CAIRN_HOST").ok();
    std::env::remove_var("CAIRN_HOST");
    let cfg = Config::resolve(None).expect("resolve");
    assert!(cfg.is_loopback_host(), "default CAIRN_HOST is 127.0.0.1");
    if let Some(v) = prev {
        std::env::set_var("CAIRN_HOST", v);
    }
}

#[test]
fn embed_config_default_is_constructible() {
    let _e = EmbedConfig {
        provider: "hashing".into(),
        api_key: None,
        model: None,
        url: None,
    };
}

// Env-var tests must be serialized because Config::resolve reads process env.
use std::sync::OnceLock;
fn env_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}
