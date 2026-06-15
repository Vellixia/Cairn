//! The structured store.
//!
//! `Store` is a thin facade over a [`StoreBackend`] — Cairn's [`HelixBackend`](crate::helix) — plus
//! the content-addressed [`BlobStore`] that holds full-fidelity originals. Keeping the public
//! `Store` API stable means the backend never churns the engines, API, MCP, or CLI.

use crate::blob::BlobStore;
use cairn_core::{Config, DeviceToken, Error, Memory, Result};
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// The structured-storage operations Cairn needs from a backend, implemented by
/// [`HelixBackend`](crate::helix::HelixBackend).
pub(crate) trait StoreBackend: Send + Sync {
    fn insert_memory(&self, m: &Memory) -> Result<()>;
    fn get_memory(&self, id: &str) -> Result<Option<Memory>>;
    fn find_memory_by_content_hash(&self, hash: &str) -> Result<Option<Memory>>;
    fn all_memories(&self) -> Result<Vec<Memory>>;
    fn touch_memory(&self, id: &str) -> Result<()>;
    fn count_memories(&self) -> Result<i64>;
    fn upsert_memory(&self, m: &Memory) -> Result<bool>;
    fn memories_since(&self, since: DateTime<Utc>) -> Result<Vec<Memory>>;
    /// Semantic (vector) recall, newest-relevant first, if the backend has an embedding index.
    /// `Ok(None)` means the backend has no vectors — callers fall back to lexical ranking.
    fn semantic_recall(&self, query: &str, k: usize) -> Result<Option<Vec<Memory>>> {
        let _ = (query, k);
        Ok(None)
    }
    fn create_token(&self, name: &str) -> Result<DeviceToken>;
    /// Check whether a token id is valid (exists and has not been revoked).
    fn validate_token_id(&self, token_id: &str) -> Result<bool>;
    fn revoke_token(&self, token_id: &str) -> Result<bool>;
    fn list_tokens(&self) -> Result<Vec<DeviceToken>>;
    fn count_tokens(&self) -> Result<i64>;
    fn get_last_sync(&self, server: &str) -> Result<Option<DateTime<Utc>>>;
    fn set_last_sync(&self, server: &str, when: DateTime<Utc>) -> Result<()>;
    fn record_file_version(&self, path: &str, content_hash: &str, lines: i64) -> Result<()>;
    fn latest_file_version(&self, path: &str) -> Result<Option<(String, i64)>>;
    fn set_meta(&self, key: &str, value: &str) -> Result<()>;
    fn get_meta(&self, key: &str) -> Result<Option<String>>;
    fn all_file_versions(&self) -> Result<Vec<(String, String, i64)>>;
    fn insert_checkpoint(&self, id: &str, label: &str, created_at: &str, files: &str)
        -> Result<()>;
    /// `(label, created_at, files_json)`.
    fn get_checkpoint(&self, id: &str) -> Result<Option<(String, String, String)>>;
    /// `(id, label, created_at)`, newest first.
    fn list_checkpoints(&self) -> Result<Vec<(String, String, String)>>;
    fn record_guard_event(&self, ts: &str, kind: &str, risk: &str, path: &str) -> Result<()>;
    /// `(kind, risk, path, ts)`, newest first.
    fn recent_guard_events(&self, limit: usize) -> Result<Vec<(String, String, String, String)>>;
    fn create_pairing(&self, code: &str, token: &str, name: &str, expires_at: &str) -> Result<()>;
    /// Atomically claim a non-expired code (single-use): returns `(token, name)` and removes it.
    fn claim_pairing(&self, code: &str, now: &str) -> Result<Option<(String, String)>>;
}

/// The structured store plus the content-addressed blob store. Backend-agnostic public API.
pub struct Store {
    backend: Box<dyn StoreBackend>,
    blobs: BlobStore,
}

impl Store {
    /// Open the store described by `cfg`. Cairn uses **HelixDB** as its datastore, so
    /// `CAIRN_HELIX_URL` (`cfg.helix_url`) must be set; the bundled `docker compose` stack provides
    /// one. The content-addressed blob store lives under the data dir.
    pub fn open(cfg: &Config) -> Result<Self> {
        let url = cfg.helix_url.as_deref().ok_or_else(|| {
            Error::Invalid(
                "CAIRN_HELIX_URL is required — Cairn stores data in HelixDB. Run the docker compose \
                 stack (which starts one) or point CAIRN_HELIX_URL at a HelixDB server."
                    .into(),
            )
        })?;
        let backend: Box<dyn StoreBackend> =
            Box::new(crate::helix::HelixBackend::connect(url, cfg)?);
        Ok(Self {
            backend,
            blobs: BlobStore::new(cfg.blobs_dir()),
        })
    }

    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    pub fn insert_memory(&self, m: &Memory) -> Result<()> {
        self.backend.insert_memory(m)
    }
    pub fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        self.backend.get_memory(id)
    }
    pub fn find_memory_by_content_hash(&self, hash: &str) -> Result<Option<Memory>> {
        self.backend.find_memory_by_content_hash(hash)
    }
    pub fn all_memories(&self) -> Result<Vec<Memory>> {
        self.backend.all_memories()
    }
    /// Vector recall (HNSW) when the backend has embeddings; `Ok(None)` on lexical-only backends.
    pub fn semantic_recall(&self, query: &str, k: usize) -> Result<Option<Vec<Memory>>> {
        self.backend.semantic_recall(query, k)
    }
    pub fn touch_memory(&self, id: &str) -> Result<()> {
        self.backend.touch_memory(id)
    }
    pub fn count_memories(&self) -> Result<i64> {
        self.backend.count_memories()
    }
    pub fn upsert_memory(&self, m: &Memory) -> Result<bool> {
        self.backend.upsert_memory(m)
    }
    pub fn memories_since(&self, since: DateTime<Utc>) -> Result<Vec<Memory>> {
        self.backend.memories_since(since)
    }
    pub fn create_token(&self, name: &str) -> Result<DeviceToken> {
        self.backend.create_token(name)
    }
    pub fn validate_token_id(&self, token_id: &str) -> Result<bool> {
        self.backend.validate_token_id(token_id)
    }
    pub fn revoke_token(&self, token_id: &str) -> Result<bool> {
        self.backend.revoke_token(token_id)
    }
    pub fn list_tokens(&self) -> Result<Vec<DeviceToken>> {
        self.backend.list_tokens()
    }
    pub fn count_tokens(&self) -> Result<i64> {
        self.backend.count_tokens()
    }
    pub fn get_last_sync(&self, server: &str) -> Result<Option<DateTime<Utc>>> {
        self.backend.get_last_sync(server)
    }
    pub fn set_last_sync(&self, server: &str, when: DateTime<Utc>) -> Result<()> {
        self.backend.set_last_sync(server, when)
    }
    pub fn record_file_version(&self, path: &str, content_hash: &str, lines: i64) -> Result<()> {
        self.backend.record_file_version(path, content_hash, lines)
    }
    pub fn latest_file_version(&self, path: &str) -> Result<Option<(String, i64)>> {
        self.backend.latest_file_version(path)
    }
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.backend.set_meta(key, value)
    }
    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        self.backend.get_meta(key)
    }
    pub fn all_file_versions(&self) -> Result<Vec<(String, String, i64)>> {
        self.backend.all_file_versions()
    }
    pub fn insert_checkpoint(
        &self,
        id: &str,
        label: &str,
        created_at: &str,
        files: &str,
    ) -> Result<()> {
        self.backend.insert_checkpoint(id, label, created_at, files)
    }
    pub fn get_checkpoint(&self, id: &str) -> Result<Option<(String, String, String)>> {
        self.backend.get_checkpoint(id)
    }
    pub fn list_checkpoints(&self) -> Result<Vec<(String, String, String)>> {
        self.backend.list_checkpoints()
    }
    pub fn record_guard_event(&self, ts: &str, kind: &str, risk: &str, path: &str) -> Result<()> {
        self.backend.record_guard_event(ts, kind, risk, path)
    }
    pub fn recent_guard_events(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, String, String, String)>> {
        self.backend.recent_guard_events(limit)
    }
    pub fn create_pairing(
        &self,
        code: &str,
        token: &str,
        name: &str,
        expires_at: &str,
    ) -> Result<()> {
        self.backend.create_pairing(code, token, name, expires_at)
    }
    pub fn claim_pairing(&self, code: &str, now: &str) -> Result<Option<(String, String)>> {
        self.backend.claim_pairing(code, now)
    }

    /// Open an **isolated** store for tests against a HelixDB server.
    ///
    /// Returns `None` when `CAIRN_HELIX_URL` is unset, so the offline suite simply skips
    /// Helix-backed tests; when it *is* set but the server can't be reached, this panics so CI
    /// surfaces the failure rather than skipping silently. Each call gets a fresh label namespace
    /// (so concurrent tests never collide on the shared server) and the dependency-free `hashing`
    /// embedder (no model download, no network).
    #[doc(hidden)]
    pub fn open_for_test() -> Option<Self> {
        let cfg = Self::test_config()?;
        Some(Self::open(&cfg).expect("CAIRN_HELIX_URL is set but opening the Helix store failed"))
    }

    /// The isolated [`Config`] backing [`open_for_test`](Self::open_for_test) — a fresh label
    /// namespace + the `hashing` embedder, pointed at `CAIRN_HELIX_URL`. `None` when that is unset.
    /// Components built from a `Config` (the API/MCP servers) use this directly in their tests.
    /// Data/blob dirs are created so the store opens cleanly.
    #[doc(hidden)]
    pub fn test_config() -> Option<Config> {
        let url = std::env::var("CAIRN_HELIX_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let id = Uuid::new_v4().simple().to_string();
        let cfg = Config {
            data_dir: std::env::temp_dir().join(format!("cairn-test-{id}")),
            host: "127.0.0.1".into(),
            port: 7777,
            helix_url: Some(url),
            helix_ns: Some(format!("t{id}_")),
            default_server: None,
            secret_key: Some(b"test-secret-key-must-be-32-bytes!!".to_vec()),
            tls: None,
            embed: cairn_core::EmbedConfig {
                provider: "hashing".into(),
                model: None,
                url: None,
                api_key: None,
            },
        };
        std::fs::create_dir_all(cfg.blobs_dir()).expect("create test blob dir");
        Some(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::{MemoryKind, MemoryTier};
    use chrono::Duration;

    /// `None` when `CAIRN_HELIX_URL` is unset (offline runs skip these); otherwise an isolated
    /// Helix-backed store.
    fn store() -> Option<Store> {
        Store::open_for_test()
    }

    fn mem(id: &str, content: &str, updated: DateTime<Utc>) -> Memory {
        Memory {
            id: id.into(),
            kind: MemoryKind::Note,
            tier: MemoryTier::Working,
            content: content.into(),
            concepts: vec![],
            files: vec![],
            session_id: None,
            importance: 0.5,
            access_count: 0,
            created_at: updated,
            updated_at: updated,
        }
    }

    #[test]
    fn tokens_create_validate_revoke() {
        let Some(s) = store() else { return };
        assert_eq!(s.count_tokens().unwrap(), 0);
        let t = s.create_token("laptop").unwrap();
        assert!(s.validate_token_id(&t.id).unwrap());
        assert!(!s.validate_token_id("nope").unwrap());
        assert_eq!(s.count_tokens().unwrap(), 1);
        assert!(s.revoke_token(&t.id).unwrap());
        assert!(!s.validate_token_id(&t.id).unwrap());
    }

    #[test]
    fn upsert_is_last_write_wins() {
        let Some(s) = store() else { return };
        let t0 = Utc::now();
        assert!(s.upsert_memory(&mem("x", "old", t0)).unwrap());
        // An older write must be ignored.
        assert!(!s
            .upsert_memory(&mem("x", "stale", t0 - Duration::seconds(10)))
            .unwrap());
        assert_eq!(s.get_memory("x").unwrap().unwrap().content, "old");
        // A newer write wins.
        assert!(s
            .upsert_memory(&mem("x", "new", t0 + Duration::seconds(10)))
            .unwrap());
        assert_eq!(s.get_memory("x").unwrap().unwrap().content, "new");
    }

    #[test]
    fn memories_since_filters_by_time() {
        let Some(s) = store() else { return };
        let t0 = Utc::now();
        s.upsert_memory(&mem("a", "a", t0 - Duration::seconds(60)))
            .unwrap();
        s.upsert_memory(&mem("b", "b", t0 + Duration::seconds(60)))
            .unwrap();
        let changed = s.memories_since(t0).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].id, "b");
    }

    #[test]
    fn last_sync_roundtrips() {
        let Some(s) = store() else { return };
        assert!(s.get_last_sync("https://x").unwrap().is_none());
        let now = Utc::now();
        s.set_last_sync("https://x", now).unwrap();
        let got = s.get_last_sync("https://x").unwrap().unwrap();
        assert!((got - now).num_seconds().abs() < 2);
    }

    #[test]
    fn meta_roundtrips_and_overwrites() {
        let Some(s) = store() else { return };
        assert!(s.get_meta("task_anchor").unwrap().is_none());
        s.set_meta("task_anchor", "ship the release").unwrap();
        assert_eq!(
            s.get_meta("task_anchor").unwrap().unwrap(),
            "ship the release"
        );
        s.set_meta("task_anchor", "fix the bug").unwrap();
        assert_eq!(s.get_meta("task_anchor").unwrap().unwrap(), "fix the bug");
    }

    #[test]
    fn file_version_roundtrips_and_upserts() {
        let Some(s) = store() else { return };
        assert!(s.latest_file_version("/x.rs").unwrap().is_none());
        s.record_file_version("/x.rs", "abc123", 42).unwrap();
        let (hash, lines) = s.latest_file_version("/x.rs").unwrap().unwrap();
        assert_eq!(hash, "abc123");
        assert_eq!(lines, 42);
        s.record_file_version("/x.rs", "def456", 10).unwrap();
        assert_eq!(s.latest_file_version("/x.rs").unwrap().unwrap().0, "def456");
    }

    #[test]
    fn pairing_code_claims_once_and_respects_expiry() {
        let Some(s) = store() else { return };
        // A live code claims exactly once, returning its token + device name.
        s.create_pairing("ABCD2345", "tok-1", "laptop", "2999-01-01T00:00:00.000Z")
            .unwrap();
        let claimed = s
            .claim_pairing("ABCD2345", "2026-01-01T00:00:00.000Z")
            .unwrap();
        assert_eq!(claimed, Some(("tok-1".to_string(), "laptop".to_string())));
        // Single-use: a second claim finds nothing.
        assert!(s
            .claim_pairing("ABCD2345", "2026-01-01T00:00:00.000Z")
            .unwrap()
            .is_none());

        // An expired code never claims (RFC3339 strings compare lexicographically).
        s.create_pairing("EXPIRED1", "tok-2", "phone", "2000-01-01T00:00:00.000Z")
            .unwrap();
        assert!(s
            .claim_pairing("EXPIRED1", "2026-01-01T00:00:00.000Z")
            .unwrap()
            .is_none());
    }
}
