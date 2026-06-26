//! Setup wizard v2 (Sprint 6).
//!
//! v1's `/api/auth/setup` accepted only username + password. v2 layers an embed-provider
//! picker, optional device-pair via a QR code, and a green-health check that verifies
//! HelixDB reachability, the embed provider, and that the admin record round-tripped.
//!
//! The wizard flow is:
//! 1. `POST /api/auth/setup` with the embed fields + credentials. Returns the session cookie
//!    and a `health` block the dashboard uses to render the final "all-green" page.
//! 2. The dashboard renders the wizard steps; the existing `/setup` route is kept as a
//!    fallback (deprecation banner pointing to `/setup/wizard`).
//! 3. After the wizard, the embed config is read at server startup - the runtime picks up
//!    the persisted choice on next launch.

use crate::AppState;
use axum::{extract::State, Json};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HealthCheck {
    pub helix_reachable: bool,
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
    let helix_reachable = s.store.count_memories().is_ok();
    let embedder_loaded = cairn_embed::from_config(&s.cfg.embed).is_ok();
    let secret_key_configured = s.cfg.secret_key.is_some();
    Json(SetupHealth {
        health: HealthCheck {
            helix_reachable,
            admin_exists,
            embedder_loaded,
            secret_key_configured,
        },
        embed_provider: s.cfg.embed.provider.clone(),
    })
}
