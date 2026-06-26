//! The structured store.
//!
//! `Store` is a thin facade over a [`StoreBackend`] - Cairn's [`HelixBackend`](crate::helix) - plus
//! the content-addressed [`BlobStore`] that holds full-fidelity originals. Keeping the public
//! `Store` API stable means the backend never churns the engines, API, MCP, or CLI.

use crate::blob::BlobStore;
use cairn_core::{Config, DeviceToken, Error, Memory, Result};
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Tombstone value written by [`Store::reset_meta`]. HelixDB's append-only schema can't
/// physically remove rows, so this sentinel signals "logically absent" to readers.
pub(crate) const META_TOMBSTONE: &str = "__deleted__";

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
    /// Apply the agentmemory reinforcement curve `c' = min(1.0, c + 0.1*(1-c))` and bump
    /// `access_count`. Idempotent on missing rows (no-op).
    fn reinforce_memory(&self, id: &str) -> Result<()> {
        let _ = id;
        Ok(())
    }
    /// Set `pinned` to the given value. Idempotent; missing rows are no-ops.
    fn set_pinned(&self, id: &str, pinned: bool) -> Result<()> {
        let _ = (id, pinned);
        Ok(())
    }
    /// Edit a memory's mutable fields. Returns `Ok(true)` if the row was updated, `Ok(false)`
    /// if no row exists. Only the fields that are `Some` are applied - the rest are kept.
    fn edit_memory(
        &self,
        id: &str,
        content: Option<String>,
        importance: Option<f32>,
        concepts: Option<Vec<String>>,
        files: Option<Vec<String>>,
    ) -> Result<bool> {
        let _ = (id, content, importance, concepts, files);
        Ok(false)
    }
    /// Delete a memory by id. Returns `Ok(true)` if a row was removed, `Ok(false)` otherwise.
    fn delete_memory(&self, id: &str) -> Result<bool> {
        let _ = id;
        Ok(false)
    }
    /// Semantic (vector) recall, newest-relevant first, if the backend has an embedding index.
    /// `Ok(None)` means the backend has no vectors - callers fall back to lexical ranking.
    fn semantic_recall(&self, query: &str, k: usize) -> Result<Option<Vec<Memory>>> {
        let _ = (query, k);
        Ok(None)
    }
    fn create_token(
        &self,
        name: &str,
        scope: cairn_core::TokenScope,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<DeviceToken>;
    /// Check whether a token id is valid (exists and has not been revoked).
    fn validate_token_id(&self, token_id: &str) -> Result<bool>;
    /// Record that a token was used successfully, updating its `last_used_at` timestamp.
    fn record_token_usage(&self, token_id: &str) -> Result<()> {
        let _ = token_id;
        Ok(())
    }
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

    // -- audit log (v0.5.0 - Sprint 1) ----------------------------------------------------
    /// Append an audit event to durable storage. Returns the assigned event id (a monotonically
    /// increasing integer encoded as a string, suitable for SSE `Last-Event-ID` resync).
    /// Implementations must surface failures so a torn audit trail can't go silently unrecorded.
    fn append_audit(&self, ts: i64, kind: &str, actor: &str, detail: &str) -> Result<String> {
        // Default no-op for backends that haven't implemented durable audit yet. Returns a
        // pseudo-id derived from the timestamp so callers always get *something* back.
        let _ = (ts, kind, actor, detail);
        Ok(format!("inmem-{ts}"))
    }
    /// Recent audit events, newest first. `since_event_id=None` returns the most recent
    /// `limit`; `since_event_id=Some(id)` returns events strictly newer than `id` (for SSE
    /// reconnect replay).
    fn recent_audit(&self, limit: usize, since_event_id: Option<&str>) -> Result<Vec<AuditRecord>> {
        let _ = (limit, since_event_id);
        Ok(Vec::new())
    }
    /// Maximum event id ever assigned (used to confirm no events have been pruned when a client
    /// asks for events older than the ring's window).
    fn max_audit_event_id(&self) -> Result<i64> {
        Ok(0)
    }
}

/// A single audit event read from durable storage.
#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub id: i64,
    pub ts: i64,
    pub kind: String,
    pub actor: String,
    pub detail: String,
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
                "CAIRN_HELIX_URL is required - Cairn stores data in HelixDB. Run the docker compose \
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
    /// Materialize a `NewMemory` (with id + timestamps) and insert it. Used by
    /// the API layer where a handler doesn't want to depend on `MemoryEngine`.
    pub fn insert_memory_for(&self, new_mem: &cairn_core::NewMemory) -> Result<Memory> {
        let mem = new_mem.clone().into_memory();
        self.backend.insert_memory(&mem)?;
        Ok(mem)
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
    /// Apply the reinforcement curve on a memory's confidence and bump access_count.
    pub fn reinforce_memory(&self, id: &str) -> Result<()> {
        self.backend.reinforce_memory(id)
    }
    /// Set a memory's `pinned` flag.
    pub fn set_pinned(&self, id: &str, pinned: bool) -> Result<()> {
        self.backend.set_pinned(id, pinned)
    }
    /// Edit a memory's mutable fields. Only the `Some` values are applied.
    pub fn edit_memory(
        &self,
        id: &str,
        content: Option<String>,
        importance: Option<f32>,
        concepts: Option<Vec<String>>,
        files: Option<Vec<String>>,
    ) -> Result<bool> {
        self.backend
            .edit_memory(id, content, importance, concepts, files)
    }
    /// Delete a memory by id.
    pub fn delete_memory(&self, id: &str) -> Result<bool> {
        self.backend.delete_memory(id)
    }
    pub fn create_token(
        &self,
        name: &str,
        scope: cairn_core::TokenScope,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<DeviceToken> {
        self.backend.create_token(name, scope, expires_at)
    }
    pub fn validate_token_id(&self, token_id: &str) -> Result<bool> {
        self.backend.validate_token_id(token_id)
    }
    pub fn record_token_usage(&self, token_id: &str) -> Result<()> {
        self.backend.record_token_usage(token_id)
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
    /// Atomically set `key=value` only if `key` is currently absent (or present only as the
    /// tombstone sentinel `__deleted__`, written by [`reset_meta`]). Returns `Ok(true)` on
    /// insert, `Ok(false)` otherwise. Used for first-run admin creation so two concurrent
    /// setup requests can't both win.
    pub fn set_meta_if_absent(&self, key: &str, value: &str) -> Result<bool> {
        if let Some(existing) = self.backend.get_meta(key)? {
            if existing != META_TOMBSTONE {
                return Ok(false);
            }
        }
        self.backend.set_meta(key, value)?;
        Ok(true)
    }

    /// Append a durable audit event. Returns the assigned event id so callers (SSE broadcaster)
    /// can include it in the `id:` line for `Last-Event-ID` resync.
    pub fn append_audit(&self, ts: i64, kind: &str, actor: &str, detail: &str) -> Result<String> {
        self.backend.append_audit(ts, kind, actor, detail)
    }

    /// Recent audit events, newest first. `since_event_id=None` returns up to `limit`; `Some(id)`
    /// returns events strictly newer than `id` (used by the SSE endpoint to replay missed events
    /// after a disconnect).
    pub fn recent_audit(
        &self,
        limit: usize,
        since_event_id: Option<&str>,
    ) -> Result<Vec<AuditRecord>> {
        self.backend.recent_audit(limit, since_event_id)
    }

    /// Maximum audit event id ever assigned (used by SSE clients to detect pruning).
    pub fn max_audit_event_id(&self) -> Result<i64> {
        self.backend.max_audit_event_id()
    }

    /// Mark `key` as deleted by appending the tombstone sentinel. The append-only HelixDB
    /// schema can't physically remove rows, so future reads will see this as absent via
    /// [`get_meta`] and [`set_meta_if_absent`]. Returns `Ok(true)` if a record existed.
    pub fn reset_meta(&self, key: &str) -> Result<bool> {
        let existed = self.backend.get_meta(key)?.is_some();
        self.backend.set_meta(key, META_TOMBSTONE)?;
        Ok(existed)
    }

    /// Read meta, treating the tombstone sentinel as absent.
    pub fn get_meta_live(&self, key: &str) -> Result<Option<String>> {
        Ok(self.backend.get_meta(key)?.filter(|v| v != META_TOMBSTONE))
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
        Self::open(&cfg).ok()
    }

    /// The isolated [`Config`] backing [`open_for_test`](Self::open_for_test) - a fresh label
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
            helix_token: None,
            helix_ns: Some(format!("t{id}_")),
            default_server: None,
            secret_key: Some(b"test-secret-key-must-be-32-bytes!!".to_vec()),
            tls: None,
            insecure: false,
            workspace_root: None,
            cors_origins: vec![],
            embed: cairn_core::EmbedConfig {
                provider: "hashing".into(),
                model: None,
                url: None,
                api_key: None,
            },
            admin: cairn_core::AdminConfig::default(),
            multi_tenant: false,
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
            org_id: cairn_core::OrgId::default(),
            suspicious: false,
            confidence: 0.5,
            pinned: false,
            derived_from: vec![],
            contradicts: vec![],
            supersedes: vec![],
            applies_to: vec![],
            created_at: updated,
            updated_at: updated,
        }
    }

    #[test]
    fn tokens_create_validate_revoke() {
        let Some(s) = store() else { return };
        assert_eq!(s.count_tokens().unwrap(), 0);
        let t = s
            .create_token("laptop", cairn_core::TokenScope::Write, None)
            .unwrap();
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

    #[test]
    fn set_meta_if_absent_blocks_concurrent_setup() {
        let Some(s) = store() else { return };
        let k = "first_admin_slot";
        assert!(s.set_meta_if_absent(k, "first").unwrap());
        assert!(!s.set_meta_if_absent(k, "second").unwrap());
        // The first writer's value persists.
        assert_eq!(s.get_meta_live(k).unwrap().as_deref(), Some("first"));
    }

    #[test]
    fn reset_meta_tombstones_and_unblocks_setup() {
        let Some(s) = store() else { return };
        let k = "deletable";
        s.set_meta(k, "original").unwrap();
        assert_eq!(s.get_meta_live(k).unwrap().as_deref(), Some("original"));

        assert!(s.reset_meta(k).unwrap());
        // get_meta_live sees the tombstone as absent.
        assert!(s.get_meta_live(k).unwrap().is_none());
        // Raw get_meta still sees the tombstone string (so the audit chain is intact).
        assert_eq!(s.get_meta(k).unwrap().as_deref(), Some(META_TOMBSTONE));

        // A subsequent set_meta_if_absent succeeds because the tombstone counts as absent.
        assert!(s.set_meta_if_absent(k, "fresh").unwrap());
        assert_eq!(s.get_meta_live(k).unwrap().as_deref(), Some("fresh"));
    }

    #[test]
    fn reset_meta_on_missing_key_reports_no_prior_record() {
        let Some(s) = store() else { return };
        assert!(!s.reset_meta("never_set").unwrap());
    }

    // -- v0.5.0 Sprint 1: durable audit log tests ----------------------------------------

    #[test]
    fn audit_append_assigns_monotonic_ids_and_reads_back() {
        let Some(s) = store() else { return };
        let id1 = s.append_audit(1000, "login_ok", "alice", "").unwrap();
        let id2 = s
            .append_audit(2000, "token_issued", "alice", "laptop")
            .unwrap();
        let id3 = s
            .append_audit(3000, "login_failed", "bob", "bad password")
            .unwrap();
        // Ids are monotonically increasing as integers.
        let i1: i64 = id1.parse().unwrap();
        let i2: i64 = id2.parse().unwrap();
        let i3: i64 = id3.parse().unwrap();
        assert!(i1 < i2 && i2 < i3);

        // recent_audit returns newest first by default.
        let recent = s.recent_audit(10, None).unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].id, i3);
        assert_eq!(recent[2].id, i1);
        assert_eq!(recent[0].kind, "login_failed");
        assert_eq!(recent[1].actor, "alice");
        assert_eq!(recent[2].detail, "");
    }

    #[test]
    fn audit_replay_since_id_returns_only_newer_events() {
        let Some(s) = store() else { return };
        s.append_audit(1000, "a", "x", "").unwrap();
        s.append_audit(2000, "b", "x", "").unwrap();
        s.append_audit(3000, "c", "x", "").unwrap();

        // Take the id of the middle event and ask for everything newer.
        let all = s.recent_audit(10, None).unwrap();
        let middle_id = all.iter().find(|r| r.kind == "b").unwrap().id.to_string();

        let newer = s.recent_audit(10, Some(&middle_id)).unwrap();
        // Only "c" remains.
        assert_eq!(newer.len(), 1);
        assert_eq!(newer[0].kind, "c");
    }

    #[test]
    fn audit_survives_replay_via_max_event_id() {
        let Some(s) = store() else { return };
        assert_eq!(s.max_audit_event_id().unwrap(), 0);
        s.append_audit(100, "x", "y", "").unwrap();
        s.append_audit(200, "x", "y", "").unwrap();
        let max = s.max_audit_event_id().unwrap();
        assert!(
            max >= 2,
            "max audit id should be at least 2 after two appends; got {max}"
        );
    }

    #[test]
    fn audit_survives_round_trip_after_a_store_drop_and_reopen() {
        // Append a few events to one store, then close it and open a fresh one. The events
        // should still be readable - that's the whole point of the Sprint 1 migration away
        // from the in-memory ring buffer.
        let Some(s1) = store() else { return };
        s1.append_audit(100, "login_ok", "alice", "").unwrap();
        s1.append_audit(200, "token_issued", "alice", "laptop")
            .unwrap();
        drop(s1);

        let Some(s2) = store() else { return };
        let recent = s2.recent_audit(10, None).unwrap();
        let kinds: Vec<&str> = recent.iter().map(|r| r.kind.as_str()).collect();
        assert!(
            kinds.contains(&"login_ok") && kinds.contains(&"token_issued"),
            "audit events should survive reopen; got kinds {kinds:?}"
        );
    }
}
