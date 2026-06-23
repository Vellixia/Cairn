//! Signed cost-savings ledger (v0.5.0 Sprint 5).
//!
//! Every API handler that saves context budget appends a [`LedgerEntry`] to an in-process
//! ring buffer plus an HMAC-SHA256 signature (key = `CAIRN_SECRET_KEY`). The dashboard's
//! `/dashboard/savings` page reads this through `/api/ledger`; `/api/ledger/verify` lets a
//! user re-check an entry's signature offline (deterministic JSON serialization is what gets
//! signed, so the verifier doesn't need access to the runtime).
//!
//! Persistence: in-memory by default. A future iteration can mirror the audit log to
//! HelixDB — the signature scheme is the same so the two backends stay interchangeable.

use crate::AppState;
use axum::{
    extract::{Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

const LEDGER_CAPACITY: usize = 5_000;

/// What gets saved, in human-readable units. USD is computed at snapshot time using
/// `$0.00003` per input token (typical Sonnet-class pricing — the same constant
/// [`SavingsCounter`] uses).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub id: i64,
    pub ts: DateTime<Utc>,
    pub source: String,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub tokens_saved: i64,
    pub cost_usd_saved: f64,
    /// Lower-case hex HMAC-SHA256 over the canonical JSON of exactly six fields:
    /// (id, ts, source, bytes_in, bytes_out, tokens_saved). `cost_usd_saved` is
    /// **deliberately excluded** from the signed payload — it's computed at display
    /// time from `tokens_saved` and the current `$0.00003 / 4` price constant. If the
    /// price constant ever changes, historical entries will recompute their
    /// `cost_usd_saved` against the new price while the signature still verifies; this
    /// is intentional (the price is treated as a runtime parameter, not as part of the
    /// tamper-evidence scope). To reproduce historical USD exactly across price
    /// updates, sign `price_usd_per_million_tokens_at_sign_time` alongside the other
    /// fields — tracked for v0.6 (FIXME below).
    pub signature: String,
    // FIXME: v0.6 — add `price_usd_per_million_tokens_at_sign_time: f64` and include it
    // in `canonical_json` / `sign_ledger` so historical `cost_usd_saved` values are
    // reproducible after the price constant changes. This is a breaking change to the
    // signed payload, so it ships alongside the v0.6 ledger schema migration.
}

#[derive(Default)]
pub struct Ledger {
    inner: Mutex<VecDeque<LedgerEntry>>,
    next_id: Mutex<i64>,
}

impl Ledger {
    pub fn append(
        &self,
        source: &str,
        bytes_in: u64,
        bytes_out: u64,
        signing_key: &[u8],
    ) -> LedgerEntry {
        let tokens_saved = (bytes_out as i64) - (bytes_in as i64);
        let cost = (tokens_saved.max(0) as f64) * 0.00003 / 4.0; // 4 bytes/token
        let id = {
            let mut n = self.next_id.lock().expect("ledger next_id mutex");
            let cur = *n;
            *n += 1;
            cur
        };
        let ts = Utc::now();
        let signature = sign_ledger(
            id,
            ts,
            source,
            bytes_in,
            bytes_out,
            tokens_saved,
            signing_key,
        );
        let entry = LedgerEntry {
            id,
            ts,
            source: source.to_string(),
            bytes_in,
            bytes_out,
            tokens_saved,
            cost_usd_saved: cost,
            signature,
        };
        let mut q = self.inner.lock().expect("ledger ring mutex");
        q.push_front(entry.clone());
        while q.len() > LEDGER_CAPACITY {
            q.pop_back();
        }
        entry
    }

    pub fn snapshot(&self) -> Vec<LedgerEntry> {
        self.inner
            .lock()
            .expect("ledger ring mutex")
            .iter()
            .cloned()
            .collect()
    }
}

/// Add the ledger to AppState (cheap to clone — Arc inside).
#[derive(Clone, Default)]
pub struct LedgerState(pub Arc<Ledger>);

impl std::ops::Deref for LedgerState {
    type Target = Ledger;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Canonical serialization for signing. MUST stay stable across versions or every signature
/// already in the wild becomes unverifiable. The format is the JSON object of all fields
/// except `signature`, sorted by key.
fn canonical_json(
    id: i64,
    ts: DateTime<Utc>,
    source: &str,
    bytes_in: u64,
    bytes_out: u64,
    tokens_saved: i64,
) -> String {
    // serde_json::Map's default insertion order is "preserve insertion order" but BTreeMap
    // gives us deterministic ordering for free.
    let mut m: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();
    m.insert("bytes_in", bytes_in.to_string());
    m.insert("bytes_out", bytes_out.to_string());
    m.insert("id", id.to_string());
    m.insert("source", source.to_string());
    m.insert("tokens_saved", tokens_saved.to_string());
    m.insert(
        "ts",
        ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    );
    // Hand-roll the JSON to avoid serde_json's whitespace variability.
    let mut out = String::from("{");
    let mut first = true;
    for (k, v) in &m {
        if !first {
            out.push(',');
        }
        first = false;
        out.push('"');
        out.push_str(k);
        out.push_str("\":\"");
        out.push_str(&escape(v));
        out.push('"');
    }
    out.push('}');
    out
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn sign_ledger(
    id: i64,
    ts: DateTime<Utc>,
    source: &str,
    bytes_in: u64,
    bytes_out: u64,
    tokens_saved: i64,
    key: &[u8],
) -> String {
    let canonical = canonical_json(id, ts, source, bytes_in, bytes_out, tokens_saved);
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key");
    mac.update(canonical.as_bytes());
    let bytes = mac.finalize().into_bytes();
    bytes
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}

/// Re-verify a ledger entry's signature offline.
pub fn verify_entry(entry: &LedgerEntry, key: &[u8]) -> bool {
    let expected = sign_ledger(
        entry.id,
        entry.ts,
        &entry.source,
        entry.bytes_in,
        entry.bytes_out,
        entry.tokens_saved,
        key,
    );
    // Constant-time compare — entry.signature is user-controlled.
    if expected.len() != entry.signature.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (a, b) in expected.bytes().zip(entry.signature.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

// ---- HTTP handlers ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LedgerQuery {
    #[serde(default)]
    pub limit: Option<usize>,
}

/// GET `/api/ledger` — recent savings entries (newest first).
pub async fn get_ledger(
    State(s): State<AppState>,
    Query(q): Query<LedgerQuery>,
) -> Json<Vec<LedgerEntry>> {
    let mut entries = s.ledger.snapshot();
    if let Some(limit) = q.limit {
        entries.truncate(limit);
    }
    Json(entries)
}

#[derive(Deserialize)]
pub struct VerifyQuery {
    pub id: i64,
}

/// GET `/api/ledger/verify?id=N` — re-check entry N's HMAC. Returns `{ valid: bool }`.
pub async fn verify_ledger(
    State(s): State<AppState>,
    Query(q): Query<VerifyQuery>,
) -> Json<serde_json::Value> {
    let entry = s.ledger.snapshot().into_iter().find(|e| e.id == q.id);
    let Some(entry) = entry else {
        return Json(serde_json::json!({ "valid": false, "error": "no such entry" }));
    };
    let Some(key) = s.cfg.secret_key.as_ref() else {
        return Json(serde_json::json!({ "valid": false, "error": "no signing key configured" }));
    };
    Json(serde_json::json!({ "valid": verify_entry(&entry, key) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> Vec<u8> {
        b"test-secret-key-must-be-32-bytes-long!!".to_vec()
    }

    #[test]
    fn ledger_signs_and_verifies() {
        let l = Ledger::default();
        let e1 = l.append("context.read", 200, 1000, &key());
        let e2 = l.append("context.expand", 50, 100, &key());
        let snap = l.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].id, e2.id);
        assert_eq!(snap[1].id, e1.id);
        assert!(verify_entry(&e1, &key()));
        assert!(verify_entry(&e2, &key()));
    }

    #[test]
    fn ledger_detects_tampered_field() {
        let l = Ledger::default();
        let mut entry = l.append("assembler", 100, 500, &key());
        // Tamper with bytes_in.
        entry.bytes_in = 999;
        assert!(!verify_entry(&entry, &key()));
        // And tampering with the signature doesn't help (wrong key).
        entry = l.append("assembler", 100, 500, &key());
        let wrong = entry.signature.replace('a', "f");
        entry.signature = wrong;
        assert!(!verify_entry(&entry, &key()));
    }

    #[test]
    fn ledger_caps_at_capacity() {
        let l = Ledger::default();
        for i in 0..(LEDGER_CAPACITY + 5) {
            l.append("bench", i as u64, i as u64 * 10, &key());
        }
        assert_eq!(l.snapshot().len(), LEDGER_CAPACITY);
    }

    #[test]
    fn canonical_json_is_deterministic() {
        let ts = DateTime::parse_from_rfc3339("2026-01-02T03:04:05.000Z")
            .unwrap()
            .with_timezone(&Utc);
        let a = canonical_json(1, ts, "src", 100, 200, 50);
        let b = canonical_json(1, ts, "src", 100, 200, 50);
        assert_eq!(a, b);
        // Stable, key-sorted.
        assert!(a.starts_with("{\"bytes_in\":"));
    }

    #[test]
    fn escape_handles_control_chars() {
        assert_eq!(escape("a\"b"), "a\\\"b");
        assert_eq!(escape("a\nb"), "a\\nb");
        assert_eq!(escape("a\\b"), "a\\\\b");
    }
}
