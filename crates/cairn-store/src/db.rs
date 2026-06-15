//! The structured store.
//!
//! `Store` is a backend-agnostic facade: it dispatches to a `StoreBackend` implementation (SQLite
//! today; a HelixDB backend next, per the plan) and owns the content-addressed [`BlobStore`] that
//! holds full-fidelity originals. Keeping the public `Store` API stable means swapping the backend
//! never churns the engines, API, MCP, or CLI — and tests/CI run against the embedded SQLite
//! backend with no external service.

use crate::blob::BlobStore;
use cairn_core::{Config, ContentHash, DeviceToken, Error, Memory, MemoryKind, MemoryTier, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;
use uuid::Uuid;

/// Map any storage-backend error into the shared error type.
fn stor<E: std::fmt::Display>(e: E) -> Error {
    Error::Storage(e.to_string())
}

const SELECT_COLS: &str = "id,kind,tier,content,content_hash,concepts,files,session_id,importance,access_count,created_at,updated_at";

/// The structured-storage operations Cairn needs from a backend. Implemented by [`SqliteBackend`]
/// today; a HelixDB backend implements the same surface so [`Store`] can dispatch to either.
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
    fn validate_token(&self, token: &str) -> Result<bool>;
    fn revoke_token(&self, token: &str) -> Result<bool>;
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
    /// Open (and migrate) the store described by `cfg`. Uses HelixDB when `CAIRN_HELIX_URL` is set
    /// (`cfg.helix_url`), otherwise the embedded SQLite backend.
    pub fn open(cfg: &Config) -> Result<Self> {
        let backend: Box<dyn StoreBackend> = match &cfg.helix_url {
            Some(url) => Box::new(crate::helix::HelixBackend::connect(url, cfg)?),
            None => Box::new(SqliteBackend::open(&cfg.db_path())?),
        };
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
    pub fn validate_token(&self, token: &str) -> Result<bool> {
        self.backend.validate_token(token)
    }
    pub fn revoke_token(&self, token: &str) -> Result<bool> {
        self.backend.revoke_token(token)
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

/// Embedded SQLite backend — the default; zero external services, ideal for dev/tests/CI.
struct SqliteBackend {
    conn: Mutex<Connection>,
}

impl SqliteBackend {
    fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(stor)?;
        let backend = Self {
            conn: Mutex::new(conn),
        };
        backend.migrate()?;
        Ok(backend)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id           TEXT PRIMARY KEY,
                kind         TEXT NOT NULL,
                tier         TEXT NOT NULL,
                content      TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                concepts     TEXT NOT NULL,
                files        TEXT NOT NULL,
                session_id   TEXT,
                importance   REAL NOT NULL,
                access_count INTEGER NOT NULL DEFAULT 0,
                created_at   TEXT NOT NULL,
                updated_at   TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_memories_hash ON memories(content_hash);
            CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at);
            CREATE TABLE IF NOT EXISTS device_tokens (
                token TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sync_state (
                server TEXT PRIMARY KEY,
                last_sync TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS file_versions (
                path TEXT PRIMARY KEY,
                content_hash TEXT NOT NULL,
                lines INTEGER NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS checkpoints (
                id TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                created_at TEXT NOT NULL,
                files TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS guard_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts TEXT NOT NULL,
                kind TEXT NOT NULL,
                risk TEXT NOT NULL,
                path TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS pairing_codes (
                code TEXT PRIMARY KEY,
                token TEXT NOT NULL,
                name TEXT NOT NULL,
                expires_at TEXT NOT NULL
            );",
        )
        .map_err(stor)?;
        Ok(())
    }
}

impl StoreBackend for SqliteBackend {
    fn insert_memory(&self, m: &Memory) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let hash = ContentHash::of_str(&m.content);
        conn.execute(
            "INSERT INTO memories (id,kind,tier,content,content_hash,concepts,files,session_id,importance,access_count,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                m.id,
                m.kind.as_str(),
                m.tier.as_str(),
                m.content,
                hash.as_str(),
                serde_json::to_string(&m.concepts)?,
                serde_json::to_string(&m.files)?,
                m.session_id,
                m.importance,
                m.access_count,
                ts(m.created_at),
                ts(m.updated_at),
            ],
        )
        .map_err(stor)?;
        Ok(())
    }

    fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!("SELECT {SELECT_COLS} FROM memories WHERE id=?1");
        let mut stmt = conn.prepare(&sql).map_err(stor)?;
        stmt.query_row(params![id], row_to_memory)
            .optional()
            .map_err(stor)
    }

    fn find_memory_by_content_hash(&self, hash: &str) -> Result<Option<Memory>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!("SELECT {SELECT_COLS} FROM memories WHERE content_hash=?1 LIMIT 1");
        let mut stmt = conn.prepare(&sql).map_err(stor)?;
        stmt.query_row(params![hash], row_to_memory)
            .optional()
            .map_err(stor)
    }

    fn all_memories(&self) -> Result<Vec<Memory>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!("SELECT {SELECT_COLS} FROM memories ORDER BY created_at DESC");
        let mut stmt = conn.prepare(&sql).map_err(stor)?;
        let rows = stmt.query_map([], row_to_memory).map_err(stor)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(stor)?);
        }
        Ok(out)
    }

    fn touch_memory(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE memories SET access_count = access_count + 1, updated_at = ?2 WHERE id = ?1",
            params![id, ts(Utc::now())],
        )
        .map_err(stor)?;
        Ok(())
    }

    fn count_memories(&self) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
            .map_err(stor)
    }

    fn upsert_memory(&self, m: &Memory) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let existing: Option<String> = conn
            .query_row(
                "SELECT updated_at FROM memories WHERE id=?1",
                params![m.id],
                |r| r.get(0),
            )
            .optional()
            .map_err(stor)?;
        if let Some(existing_ts) = existing {
            if m.updated_at < parse_ts(&existing_ts) {
                return Ok(false);
            }
        }
        let hash = ContentHash::of_str(&m.content);
        conn.execute(
            "INSERT OR REPLACE INTO memories (id,kind,tier,content,content_hash,concepts,files,session_id,importance,access_count,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                m.id,
                m.kind.as_str(),
                m.tier.as_str(),
                m.content,
                hash.as_str(),
                serde_json::to_string(&m.concepts)?,
                serde_json::to_string(&m.files)?,
                m.session_id,
                m.importance,
                m.access_count,
                ts(m.created_at),
                ts(m.updated_at),
            ],
        )
        .map_err(stor)?;
        Ok(true)
    }

    fn memories_since(&self, since: DateTime<Utc>) -> Result<Vec<Memory>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {SELECT_COLS} FROM memories WHERE updated_at > ?1 ORDER BY updated_at ASC"
        );
        let mut stmt = conn.prepare(&sql).map_err(stor)?;
        let rows = stmt
            .query_map(params![ts(since)], row_to_memory)
            .map_err(stor)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(stor)?);
        }
        Ok(out)
    }

    fn create_token(&self, name: &str) -> Result<DeviceToken> {
        let conn = self.conn.lock().unwrap();
        let token = format!("cairn_{}", Uuid::new_v4().simple());
        let created_at = Utc::now();
        conn.execute(
            "INSERT INTO device_tokens (token,name,created_at) VALUES (?1,?2,?3)",
            params![token, name, ts(created_at)],
        )
        .map_err(stor)?;
        Ok(DeviceToken {
            token,
            name: name.to_string(),
            created_at,
        })
    }

    fn validate_token(&self, token: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let found: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM device_tokens WHERE token=?1",
                params![token],
                |r| r.get(0),
            )
            .optional()
            .map_err(stor)?;
        Ok(found.is_some())
    }

    fn revoke_token(&self, token: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn
            .execute("DELETE FROM device_tokens WHERE token=?1", params![token])
            .map_err(stor)?;
        Ok(n > 0)
    }

    fn list_tokens(&self) -> Result<Vec<DeviceToken>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT token,name,created_at FROM device_tokens ORDER BY created_at ASC")
            .map_err(stor)?;
        let rows = stmt
            .query_map([], |row| {
                let created: String = row.get("created_at")?;
                Ok(DeviceToken {
                    token: row.get("token")?,
                    name: row.get("name")?,
                    created_at: parse_ts(&created),
                })
            })
            .map_err(stor)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(stor)?);
        }
        Ok(out)
    }

    fn count_tokens(&self) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM device_tokens", [], |r| r.get(0))
            .map_err(stor)
    }

    fn get_last_sync(&self, server: &str) -> Result<Option<DateTime<Utc>>> {
        let conn = self.conn.lock().unwrap();
        let ts_str: Option<String> = conn
            .query_row(
                "SELECT last_sync FROM sync_state WHERE server=?1",
                params![server],
                |r| r.get(0),
            )
            .optional()
            .map_err(stor)?;
        Ok(ts_str.map(|s| parse_ts(&s)))
    }

    fn set_last_sync(&self, server: &str, when: DateTime<Utc>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sync_state (server,last_sync) VALUES (?1,?2)
             ON CONFLICT(server) DO UPDATE SET last_sync=excluded.last_sync",
            params![server, ts(when)],
        )
        .map_err(stor)?;
        Ok(())
    }

    fn record_file_version(&self, path: &str, content_hash: &str, lines: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO file_versions (path,content_hash,lines,updated_at) VALUES (?1,?2,?3,?4)
             ON CONFLICT(path) DO UPDATE SET content_hash=excluded.content_hash, lines=excluded.lines, updated_at=excluded.updated_at",
            params![path, content_hash, lines, ts(Utc::now())],
        )
        .map_err(stor)?;
        Ok(())
    }

    fn latest_file_version(&self, path: &str) -> Result<Option<(String, i64)>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT content_hash, lines FROM file_versions WHERE path=?1",
            params![path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(stor)
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO meta (key,value) VALUES (?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )
        .map_err(stor)?;
        Ok(())
    }

    fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT value FROM meta WHERE key=?1", params![key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(stor)
    }

    fn all_file_versions(&self) -> Result<Vec<(String, String, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT path, content_hash, lines FROM file_versions")
            .map_err(stor)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(stor)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(stor)?);
        }
        Ok(out)
    }

    fn insert_checkpoint(
        &self,
        id: &str,
        label: &str,
        created_at: &str,
        files: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO checkpoints (id,label,created_at,files) VALUES (?1,?2,?3,?4)",
            params![id, label, created_at, files],
        )
        .map_err(stor)?;
        Ok(())
    }

    fn get_checkpoint(&self, id: &str) -> Result<Option<(String, String, String)>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT label, created_at, files FROM checkpoints WHERE id=?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(stor)
    }

    fn list_checkpoints(&self) -> Result<Vec<(String, String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, label, created_at FROM checkpoints ORDER BY created_at DESC")
            .map_err(stor)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(stor)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(stor)?);
        }
        Ok(out)
    }

    fn record_guard_event(&self, ts: &str, kind: &str, risk: &str, path: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO guard_events (ts,kind,risk,path) VALUES (?1,?2,?3,?4)",
            params![ts, kind, risk, path],
        )
        .map_err(stor)?;
        Ok(())
    }

    fn recent_guard_events(&self, limit: usize) -> Result<Vec<(String, String, String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT kind, risk, path, ts FROM guard_events ORDER BY id DESC LIMIT ?1")
            .map_err(stor)?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .map_err(stor)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(stor)?);
        }
        Ok(out)
    }

    fn create_pairing(&self, code: &str, token: &str, name: &str, expires_at: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO pairing_codes (code,token,name,expires_at) VALUES (?1,?2,?3,?4)",
            params![code, token, name, expires_at],
        )
        .map_err(stor)?;
        Ok(())
    }

    fn claim_pairing(&self, code: &str, now: &str) -> Result<Option<(String, String)>> {
        // Single connection under a mutex, so select-then-delete is effectively atomic.
        let conn = self.conn.lock().unwrap();
        let found: Option<(String, String)> = conn
            .query_row(
                "SELECT token, name FROM pairing_codes WHERE code=?1 AND expires_at > ?2",
                params![code, now],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(stor)?;
        if found.is_some() {
            conn.execute("DELETE FROM pairing_codes WHERE code=?1", params![code])
                .map_err(stor)?;
        }
        Ok(found)
    }
}

fn row_to_memory(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    let kind: String = row.get("kind")?;
    let tier: String = row.get("tier")?;
    let concepts: String = row.get("concepts")?;
    let files: String = row.get("files")?;
    let created: String = row.get("created_at")?;
    let updated: String = row.get("updated_at")?;
    Ok(Memory {
        id: row.get("id")?,
        kind: kind.parse().unwrap_or(MemoryKind::Note),
        tier: tier.parse().unwrap_or(MemoryTier::Working),
        content: row.get("content")?,
        concepts: serde_json::from_str(&concepts).unwrap_or_default(),
        files: serde_json::from_str(&files).unwrap_or_default(),
        session_id: row.get("session_id")?,
        importance: row.get("importance")?,
        access_count: row.get("access_count")?,
        created_at: parse_ts(&created),
        updated_at: parse_ts(&updated),
    })
}

fn parse_ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

/// Fixed-width, lexicographically-sortable RFC 3339 (millis + `Z`) so SQL string comparisons on
/// timestamps match chronological order.
fn ts(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::resolve(Some(dir.path().join("data"))).unwrap();
        (Store::open(&cfg).unwrap(), dir)
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
        let (s, _d) = store();
        assert_eq!(s.count_tokens().unwrap(), 0);
        let t = s.create_token("laptop").unwrap();
        assert!(s.validate_token(&t.token).unwrap());
        assert!(!s.validate_token("nope").unwrap());
        assert_eq!(s.count_tokens().unwrap(), 1);
        assert!(s.revoke_token(&t.token).unwrap());
        assert!(!s.validate_token(&t.token).unwrap());
    }

    #[test]
    fn upsert_is_last_write_wins() {
        let (s, _d) = store();
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
        let (s, _d) = store();
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
        let (s, _d) = store();
        assert!(s.get_last_sync("https://x").unwrap().is_none());
        let now = Utc::now();
        s.set_last_sync("https://x", now).unwrap();
        let got = s.get_last_sync("https://x").unwrap().unwrap();
        assert!((got - now).num_seconds().abs() < 2);
    }

    #[test]
    fn meta_roundtrips_and_overwrites() {
        let (s, _d) = store();
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
        let (s, _d) = store();
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
        let (s, _d) = store();
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
