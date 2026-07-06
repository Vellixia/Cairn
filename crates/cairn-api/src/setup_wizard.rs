//! Setup wizard (Sprint 6, consolidated to 2 steps).
//!
//! `POST /api/auth/setup` takes username + password and returns the session cookie plus a
//! `health` block the dashboard uses to render the final "all-green" page. The dashboard's
//! `/setup` route walks: 1) admin credentials, 2) health check (database reachability, embed
//! provider, admin record round-tripped) + finish. Embed provider is configured entirely via
//! `CAIRN_EMBED_PROVIDER`/`CAIRN_EMBED_MODEL`/`CAIRN_EMBED_URL`/`CAIRN_EMBED_API_KEY` env vars
//! (`cairn_core::Config`) - there is no in-wizard picker; an earlier version had one, but it
//! persisted to a store key nothing ever read back at startup, so it was removed rather than
//! wired up for a setting that already has a working env-var path.

use crate::AppState;
use axum::{extract::State, Json};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HealthCheck {
    pub db_reachable: bool,
    pub admin_exists: bool,
    pub embedder_loaded: bool,
    pub secret_key_configured: bool,
}

#[derive(Debug, Serialize)]
pub struct SetupHealth {
    pub health: HealthCheck,
    pub embed_provider: String,
}

/// `GET /api/setup/health` - the wizard's final "all green" check.
pub async fn setup_health(State(s): State<AppState>) -> Json<SetupHealth> {
    let admin_exists = crate::admin::load_admin(&s)
        .map(|r| r.is_some())
        .unwrap_or(false);
    let db_reachable = s.store.count_memories().is_ok();
    let embedder_loaded = cairn_embed::from_config(&s.cfg.embed).is_ok();
    let secret_key_configured = s.cfg.secret_key.is_some();
    Json(SetupHealth {
        health: HealthCheck {
            db_reachable,
            admin_exists,
            embedder_loaded,
            secret_key_configured,
        },
        embed_provider: s.cfg.embed.provider.clone(),
    })
}
