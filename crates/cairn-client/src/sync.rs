//! `cairn sync` — exchange memory with another Cairn server (last-write-wins).
//!
//! Pulls remote memories changed since our last sync, then pushes our local changes. The sync
//! watermark uses the server's clock to avoid drift; conflicts resolve by `updated_at` (newest
//! wins) via the store's upsert.

use anyhow::{Context, Result};
use cairn_core::Memory;
use cairn_store::Store;
use serde_json::{json, Value};

pub fn run(store: &Store, server: &str, token: Option<&str>) -> Result<()> {
    let server = server.trim_end_matches('/');
    // Fall back to a token stored at pairing time, so `cairn sync --server <url>` just works.
    let stored = store.get_meta(&format!("device_token:{server}"))?;
    let token = token.or(stored.as_deref());
    let since = store.get_last_sync(server)?;

    // ---- pull remote changes ----
    let mut req = ureq::get(&format!("{server}/api/sync/pull"));
    if let Some(s) = since {
        req = req.query("since", &s.to_rfc3339());
    }
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let pulled_body: Value = req
        .call()
        .context("pull request failed")?
        .into_json()
        .context("decoding pull response")?;
    let remote: Vec<Memory> =
        serde_json::from_value(pulled_body.get("memories").cloned().unwrap_or(Value::Null))
            .context("parsing remote memories")?;
    let server_now = pulled_body
        .get("now")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut pulled = 0usize;
    for m in &remote {
        if store.upsert_memory(m)? {
            pulled += 1;
        }
    }

    // ---- push local changes ----
    let local_changed = match since {
        Some(ts) => store.memories_since(ts)?,
        None => store.all_memories()?,
    };
    let mut preq = ureq::post(&format!("{server}/api/sync/push"));
    if let Some(t) = token {
        preq = preq.set("Authorization", &format!("Bearer {t}"));
    }
    let push_body: Value = preq
        .send_json(json!({ "memories": local_changed }))
        .context("push request failed")?
        .into_json()
        .unwrap_or(Value::Null);
    let pushed = push_body
        .get("applied")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    // ---- record the watermark using the server's clock ----
    match chrono::DateTime::parse_from_rfc3339(&server_now) {
        Ok(t) => store.set_last_sync(server, t.with_timezone(&chrono::Utc))?,
        Err(_) => store.set_last_sync(server, chrono::Utc::now())?,
    }

    println!(
        "sync with {server}: pulled {pulled}, pushed {pushed} (sent {})",
        local_changed.len()
    );
    Ok(())
}
