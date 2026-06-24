//! `cairn contribute` / `cairn pull` — federate sanitized knowledge with a shared Cairn pool.
//!
//! Privacy-first by construction: `contribute` sanitizes every local memory and only ever sends
//! the shareable, redacted forms (the server re-sanitizes again as a hard trust boundary). `pull`
//! ingests a pool into local memory, tagged with `shared` provenance.

use anyhow::{Context, Result};
use cairn_memory::MemoryEngine;
use cairn_share::{Sanitizer, ShareBundle};
use cairn_store::Store;
use serde_json::Value;

/// Sanitize all local memories and contribute the shareable ones to a server's pool.
pub fn contribute(store: &Store, server: &str, token: Option<&str>) -> Result<()> {
    let server = server.trim_end_matches('/');
    let mems = store.all_memories()?;
    let (bundle, stats) = Sanitizer::new().bundle(&mems);

    let mut req = ureq::post(&format!("{server}/api/pool/contribute"));
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let body: Value = req
        .send_json(serde_json::to_value(&bundle)?)
        .context("contribute request failed")?
        .into_json()
        .unwrap_or(Value::Null);

    let accepted = body.get("accepted").and_then(Value::as_u64).unwrap_or(0);
    let rejected = body.get("rejected").and_then(Value::as_u64).unwrap_or(0);
    println!(
        "contributed to {server}: {accepted} accepted, {rejected} rejected \
         ({} shareable of {} scanned, {} withheld locally)",
        stats.shared, stats.total, stats.withheld
    );
    Ok(())
}

/// Pull a server's pool and ingest it into local memory (tagged `shared`, deduplicated).
pub fn pull(mem: &MemoryEngine, server: &str, token: Option<&str>) -> Result<()> {
    let server = server.trim_end_matches('/');
    let mut req = ureq::get(&format!("{server}/api/pool"));
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let body: Value = req
        .call()
        .context("pull request failed")?
        .into_json()
        .context("decoding pool response")?;
    let bundle: ShareBundle = serde_json::from_value(body).context("parsing the pool bundle")?;

    let news = bundle.into_new_memories();
    let total = news.len();
    for nm in news {
        mem.remember(nm)?;
    }
    println!("pulled from {server}: ingested {total} shared memories (deduplicated)");
    Ok(())
}
