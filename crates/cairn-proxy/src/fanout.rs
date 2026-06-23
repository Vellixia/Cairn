//! Fan-out: query every upstream registry in parallel and merge the results.

use crate::config::ProxyConfig;
use crate::ProxyError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// A merged pack metadata after dedup across upstreams. The `seen_at` field is
/// the wall-clock time we observed the pack at any upstream — useful for
/// freshness heuristics in the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedPack {
    pub pack: cairn_registry::PackMeta,
    pub seen_at: DateTime<Utc>,
    /// Which upstream peers reported this pack. Empty when the pack was loaded
    /// from a local JSON index (single-peer mode).
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanoutResult {
    pub packs: Vec<MergedPack>,
    /// Total number of upstreams queried (used for diagnostics).
    pub upstreams: usize,
    /// Number of upstreams that failed (best-effort mode still returns partial
    /// results).
    pub failed: usize,
}

/// Single-threaded helper for tests / unit fixtures. Production callers go
/// through the async `fanout_async` below.
pub fn merge_results(per_peer: Vec<(&str, Vec<cairn_registry::PackMeta>)>) -> FanoutResult {
    let mut by_id: HashMap<String, MergedPack> = HashMap::new();
    let now = Utc::now();
    let mut failed = 0;
    let upstreams = per_peer.len();
    for (peer_name, packs) in per_peer {
        if packs.is_empty() && peer_name.starts_with("failed:") {
            failed += 1;
            continue;
        }
        for p in packs {
            by_id
                .entry(p.id.clone())
                .and_modify(|existing| {
                    if !existing.sources.contains(&peer_name.to_string()) {
                        existing.sources.push(peer_name.to_string());
                    }
                    existing.seen_at = now;
                })
                .or_insert_with(|| MergedPack {
                    pack: p,
                    seen_at: now,
                    sources: vec![peer_name.to_string()],
                });
        }
    }
    FanoutResult {
        packs: by_id.into_values().collect(),
        upstreams,
        failed,
    }
}

/// Async fan-out: GET `/registry/packs` from every peer in parallel via ureq
/// in a blocking-task pool, then merge. Best-effort failures are dropped
/// silently; strict-mode failures bubble up as `ProxyError`.
pub async fn fanout_async(
    config: Arc<ProxyConfig>,
    path: &str,
) -> Result<FanoutResult, ProxyError> {
    let path = path.to_string();
    let results: Vec<(String, Result<Vec<cairn_registry::PackMeta>, ProxyError>)> =
        futures_util::future::join_all(config.peers.iter().map(|peer| {
            let peer = peer.clone();
            let path = path.clone();
            async move {
                let url = format!("{}{}", peer.base_url.trim_end_matches('/'), path);
                let join_res = tokio::task::spawn_blocking(move || {
                    let mut req = ureq::get(&url).set("Accept", "application/json");
                    if let Some(t) = &peer.token {
                        req = req.set("Authorization", &format!("Bearer {t}"));
                    }
                    req.call()
                        .map_err(|e| ProxyError::Upstream(format!("GET {url}: {e}")))
                        .and_then(|resp| {
                            resp.into_json::<Vec<cairn_registry::PackMeta>>()
                                .map_err(|e| ProxyError::Upstream(format!("invalid JSON: {e}")))
                        })
                })
                .await;
                let res = match join_res {
                    Ok(r) => r,
                    Err(e) => Err(ProxyError::Upstream(format!("join error: {e}"))),
                };
                let name = peer.name.clone();
                let best_effort = peer.best_effort;
                let mapped = match res {
                    Ok(p) => (name, Ok(p)),
                    Err(e) if best_effort => (format!("failed:{}", name), Err(e)),
                    Err(e) => (name, Err(e)),
                };
                mapped
            }
        }))
        .await;

    // Fold into merge_results.
    let per_peer: Vec<(&str, Vec<cairn_registry::PackMeta>)> = results
        .iter()
        .map(|(name, res)| match res {
            Ok(p) => (name.as_str(), p.clone()),
            Err(_) => (name.as_str(), Vec::new()),
        })
        .collect();
    let failed = results.iter().filter(|(_, r)| r.is_err()).count();
    let merged = merge_results(per_peer);
    Ok(FanoutResult {
        packs: merged.packs,
        upstreams: merged.upstreams,
        failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_registry::PackMeta;

    fn pack(id: &str, name: &str, version: &str) -> PackMeta {
        PackMeta {
            id: id.into(),
            name: name.into(),
            version: version.into(),
            author: "test".into(),
            description: "".into(),
            created_at: Utc::now(),
            stored_at: Utc::now(),
            size_bytes: 0,
            signer_pubkey: None,
            has_ed25519_signature: false,
            memory_count: 0,
            download_count: 0,
            scope: Default::default(),
            origin: None,
            provenance_edge_count: 0,
        }
    }

    #[test]
    fn merge_dedups_by_id_and_tracks_sources() {
        let a = pack("1", "alpha", "1.0.0");
        let b = pack("1", "alpha", "1.0.0");
        let c = pack("2", "beta", "2.0.0");
        let merged = merge_results(vec![("peer-a", vec![a]), ("peer-b", vec![b, c])]);
        assert_eq!(merged.packs.len(), 2);
        let alpha = merged
            .packs
            .iter()
            .find(|m| m.pack.name == "alpha")
            .unwrap();
        assert_eq!(alpha.sources.len(), 2);
    }

    #[test]
    fn merge_counts_failed_upstreams() {
        let merged = merge_results(vec![
            ("failed:peer-a", Vec::new()),
            ("peer-b", vec![pack("1", "alpha", "1.0.0")]),
        ]);
        assert_eq!(merged.packs.len(), 1);
        assert_eq!(merged.failed, 1);
    }
}
