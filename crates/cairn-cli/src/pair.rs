//! `cairn pair` / `cairn pair-code` — frictionless device onboarding via a short code.
//!
//! The host mints a short-lived, single-use code (here or from the web dashboard). A new device
//! claims it over the network to receive its device token — so long secrets never have to be
//! copied by hand. After claiming, the device stores the token and runs an initial sync.

use anyhow::{Context, Result};
use cairn_api::AppState;
use chrono::{Duration, SecondsFormat, Utc};
use serde_json::{json, Value};

/// Host side: mint a device token + short-lived pairing code on the local store.
pub fn generate(state: &AppState, name: Option<&str>) -> Result<()> {
    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("device");
    let token = state.store.create_token(name)?;
    let code = cairn_api::pairing_code();
    let expires = (Utc::now() + Duration::minutes(10)).to_rfc3339_opts(SecondsFormat::Millis, true);
    state
        .store
        .create_pairing(&code, &token.id, name, &expires)?;

    println!("Pairing code for '{name}':  {code}");
    println!("On the new device, run:");
    println!("    cairn pair {code} --server http://<this-host>:7777");
    println!("(valid for 10 minutes · single use)");
    Ok(())
}

/// New-device side: claim a code from a server, store the device token, and run an initial sync.
pub fn claim(state: &AppState, server: &str, code: &str) -> Result<()> {
    let server = server.trim_end_matches('/');
    let body: Value = ureq::post(&format!("{server}/api/pair/claim"))
        .send_json(json!({ "code": code.trim() }))
        .map_err(|e| anyhow::anyhow!("pairing failed (is the code valid and unexpired?): {e}"))?
        .into_json()
        .context("decoding the pairing response")?;

    let token = body
        .get("token")
        .and_then(Value::as_str)
        .context("the server returned no token")?;
    let name = body.get("name").and_then(Value::as_str).unwrap_or("device");

    // Remember the token so future `cairn sync --server <url>` works without re-entering it.
    state
        .store
        .set_meta(&format!("device_token:{server}"), token)?;

    println!("paired as '{name}'. syncing…");
    crate::sync::run(&state.store, server, Some(token))
}
