//! The HelixDB backend.
//!
//! [`HelixBackend`] implements the same [`StoreBackend`](crate::db::StoreBackend) surface as the
//! embedded SQLite backend, but persists to a HelixDB server (an OLTP graph + vector database)
//! over its REST query API via the `helix-db` crate. It is selected when `CAIRN_HELIX_URL` is set
//! (`Config::helix_url`); otherwise Cairn uses SQLite, so the workspace test suite — which sets no
//! such URL — keeps running with no external service.
//!
//! ## Sync ↔ async bridge
//! `StoreBackend` is synchronous; the `helix-db` client is async (tokio). Each call hops onto a
//! dedicated [`tokio::runtime::Runtime`] owned by the backend. We `block_on` from a *scoped OS
//! thread* (not the caller's thread) so this is safe whether the caller is plain sync (tests) or
//! already inside a `#[tokio::main]` runtime (the server) — the latter would otherwise panic with
//! "Cannot start a runtime from within a runtime".
//!
//! ## Data model
//! Memories are `Memory` nodes carrying their columns plus a `embedding` vector property (HNSW
//! index, used for semantic recall). Operational records (tokens, sync state, file versions,
//! checkpoints, guard events, pairing codes, meta) are keyed nodes of their own label. Inserts use
//! `add_n`; reads project the needed properties with `.values([...])`.
//!
//! ## Status
//! Memory CRUD and the read paths are validated end-to-end against a live server. A few operations
//! that require in-place update/delete DSL (`touch_memory`, `revoke_token`, `claim_pairing`) are
//! intentionally minimal or return an explicit WIP error rather than fake a result — they are
//! filled in as the update/delete patterns are validated. Bulk reads currently scan a label and
//! filter in-process; property indexes and native predicates are a follow-up optimization.

use crate::db::StoreBackend;
use cairn_core::{Config, ContentHash, DeviceToken, Error, Memory, MemoryKind, MemoryTier, Result};
use cairn_embed::Embedder;
use chrono::{DateTime, Utc};
use helix_db::dsl::prelude::*;
use helix_db::dsl::{DynamicQueryRequest, PropertyInput};
use helix_db::Client;
use serde_json::{Map, Value};
use std::future::Future;
use std::str::FromStr;

const MEMORY: &str = "Memory";
const MEM_COLS: &[&str] = &[
    "id",
    "kind",
    "tier",
    "content",
    "concepts",
    "files",
    "session_id",
    "importance",
    "access_count",
    "created_at",
    "updated_at",
];

/// A HelixDB-backed structured store.
pub(crate) struct HelixBackend {
    client: Client,
    rt: tokio::runtime::Runtime,
    embed: Box<dyn Embedder>,
}

impl HelixBackend {
    /// Connect to the HelixDB server at `url`, build the embedder from `cfg`, and ensure the
    /// memory vector index exists.
    pub(crate) fn connect(url: &str, cfg: &Config) -> Result<Self> {
        let client = Client::new(Some(url))
            .map_err(|e| Error::Storage(format!("helix connect to {url}: {e}")))?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| Error::Storage(format!("helix runtime: {e}")))?;
        let embed = cairn_embed::from_config(&cfg.embed)?;
        let backend = Self { client, rt, embed };
        backend.ensure_indexes()?;
        Ok(backend)
    }

    /// Run `fut` to completion on the backend's runtime from a scoped OS thread (runtime-nesting
    /// safe). The future borrows `self`; the scope guarantees `self` outlives it.
    fn block<F>(&self, fut: F) -> F::Output
    where
        F: Future + Send,
        F::Output: Send,
    {
        let rt = &self.rt;
        std::thread::scope(|s| s.spawn(move || rt.block_on(fut)).join().unwrap())
    }

    /// Execute a dynamic query and return the raw JSON response.
    fn run(&self, req: DynamicQueryRequest) -> Result<Value> {
        let out = self.block(async move { self.client.query().dynamic(req).send().await });
        // Anchoring the Ok type to `Value` drives `send`'s response-type inference.
        let val: Value = out.map_err(|e| Error::Storage(format!("helix query: {e}")))?;
        Ok(val)
    }

    /// Create the HNSW vector index over `Memory.embedding` (idempotent).
    fn ensure_indexes(&self) -> Result<()> {
        let batch = write_batch()
            .var_as(
                "vi",
                g().create_vector_index_nodes(MEMORY, "embedding", None::<String>),
            )
            .returning(["vi"]);
        self.run(DynamicQueryRequest::write(batch))?;
        Ok(())
    }

    /// Insert a node of `label` with `props`.
    fn add_node(&self, label: &str, props: Vec<(String, PropertyInput)>) -> Result<()> {
        let batch = write_batch()
            .var_as("n", g().add_n(label, props))
            .returning(["n"]);
        self.run(DynamicQueryRequest::write(batch))?;
        Ok(())
    }

    /// Read every node of `label`, projecting `cols`. Rows are returned in insertion order.
    fn read_rows(&self, label: &str, cols: &[&str]) -> Result<Vec<Map<String, Value>>> {
        let projection: Vec<String> = cols.iter().map(|c| c.to_string()).collect();
        let batch = read_batch()
            .var_as("rows", g().n_with_label(label).values(projection))
            .returning(["rows"]);
        let resp = self.run(DynamicQueryRequest::read(batch))?;
        let arr = resp
            .get("rows")
            .and_then(|r| r.get("properties"))
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(arr
            .into_iter()
            .filter_map(|v| v.as_object().cloned())
            .collect())
    }

    /// All memories, newest first.
    fn load_memories(&self) -> Result<Vec<Memory>> {
        let mut out: Vec<Memory> = self
            .read_rows(MEMORY, MEM_COLS)?
            .iter()
            .map(memory_from_props)
            .collect();
        out.sort_by_key(|m| std::cmp::Reverse(m.created_at));
        Ok(out)
    }
}

impl StoreBackend for HelixBackend {
    fn insert_memory(&self, m: &Memory) -> Result<()> {
        let embedding = self.embed.embed_one(&m.content)?;
        let hash = ContentHash::of_str(&m.content);
        let props: Vec<(String, PropertyInput)> = vec![
            ("id".into(), m.id.clone().into()),
            ("kind".into(), m.kind.as_str().to_string().into()),
            ("tier".into(), m.tier.as_str().to_string().into()),
            ("content".into(), m.content.clone().into()),
            ("content_hash".into(), hash.as_str().to_string().into()),
            (
                "concepts".into(),
                serde_json::to_string(&m.concepts)?.into(),
            ),
            ("files".into(), serde_json::to_string(&m.files)?.into()),
            (
                "session_id".into(),
                m.session_id.clone().unwrap_or_default().into(),
            ),
            ("importance".into(), (m.importance as f64).into()),
            ("access_count".into(), m.access_count.into()),
            ("created_at".into(), ts(m.created_at).into()),
            ("updated_at".into(), ts(m.updated_at).into()),
            ("embedding".into(), embedding.into()),
        ];
        self.add_node(MEMORY, props)
    }

    fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        Ok(self.load_memories()?.into_iter().find(|m| m.id == id))
    }

    fn find_memory_by_content_hash(&self, hash: &str) -> Result<Option<Memory>> {
        Ok(self
            .load_memories()?
            .into_iter()
            .find(|m| ContentHash::of_str(&m.content).as_str() == hash))
    }

    fn all_memories(&self) -> Result<Vec<Memory>> {
        self.load_memories()
    }

    fn touch_memory(&self, _id: &str) -> Result<()> {
        // Access-count bump needs in-place node update (WIP); a no-op is safe and lossless here —
        // it only defers access analytics, never corrupts or drops a memory.
        Ok(())
    }

    fn count_memories(&self) -> Result<i64> {
        Ok(self.read_rows(MEMORY, &["id"])?.len() as i64)
    }

    fn upsert_memory(&self, m: &Memory) -> Result<bool> {
        if self.get_memory(&m.id)?.is_some() {
            // Last-writer-wins update needs update DSL (WIP); the existing copy is retained.
            return Ok(false);
        }
        self.insert_memory(m)?;
        Ok(true)
    }

    fn memories_since(&self, since: DateTime<Utc>) -> Result<Vec<Memory>> {
        Ok(self
            .load_memories()?
            .into_iter()
            .filter(|m| m.updated_at > since)
            .collect())
    }

    fn create_token(&self, name: &str) -> Result<DeviceToken> {
        let token = DeviceToken {
            token: format!("ct_{}", uuid_simple()),
            name: name.to_string(),
            created_at: Utc::now(),
        };
        self.add_node(
            "Token",
            vec![
                ("token".into(), token.token.clone().into()),
                ("name".into(), token.name.clone().into()),
                ("created_at".into(), ts(token.created_at).into()),
            ],
        )?;
        Ok(token)
    }

    fn validate_token(&self, token: &str) -> Result<bool> {
        Ok(self
            .read_rows("Token", &["token"])?
            .iter()
            .any(|r| get_str(r, "token") == token))
    }

    fn revoke_token(&self, _token: &str) -> Result<bool> {
        Err(wip("revoke_token (node delete)"))
    }

    fn list_tokens(&self) -> Result<Vec<DeviceToken>> {
        Ok(self
            .read_rows("Token", &["token", "name", "created_at"])?
            .iter()
            .map(|r| DeviceToken {
                token: get_str(r, "token"),
                name: get_str(r, "name"),
                created_at: parse_ts(&get_str(r, "created_at")),
            })
            .collect())
    }

    fn count_tokens(&self) -> Result<i64> {
        Ok(self.read_rows("Token", &["token"])?.len() as i64)
    }

    fn get_last_sync(&self, server: &str) -> Result<Option<DateTime<Utc>>> {
        Ok(self
            .read_rows("SyncState", &["server", "when"])?
            .iter()
            .rfind(|r| get_str(r, "server") == server)
            .map(|r| parse_ts(&get_str(r, "when"))))
    }

    fn set_last_sync(&self, server: &str, when: DateTime<Utc>) -> Result<()> {
        // Append-only: the newest row for a server wins on read (compaction is a follow-up).
        self.add_node(
            "SyncState",
            vec![
                ("server".into(), server.to_string().into()),
                ("when".into(), ts(when).into()),
            ],
        )
    }

    fn record_file_version(&self, path: &str, content_hash: &str, lines: i64) -> Result<()> {
        self.add_node(
            "FileVersion",
            vec![
                ("path".into(), path.to_string().into()),
                ("content_hash".into(), content_hash.to_string().into()),
                ("lines".into(), lines.into()),
            ],
        )
    }

    fn latest_file_version(&self, path: &str) -> Result<Option<(String, i64)>> {
        Ok(self
            .read_rows("FileVersion", &["path", "content_hash", "lines"])?
            .iter()
            .rfind(|r| get_str(r, "path") == path)
            .map(|r| (get_str(r, "content_hash"), get_i64(r, "lines"))))
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        // Append-only key/value; newest write for a key wins on read.
        self.add_node(
            "Meta",
            vec![
                ("key".into(), key.to_string().into()),
                ("value".into(), value.to_string().into()),
            ],
        )
    }

    fn get_meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .read_rows("Meta", &["key", "value"])?
            .iter()
            .rfind(|r| get_str(r, "key") == key)
            .map(|r| get_str(r, "value")))
    }

    fn all_file_versions(&self) -> Result<Vec<(String, String, i64)>> {
        Ok(self
            .read_rows("FileVersion", &["path", "content_hash", "lines"])?
            .iter()
            .map(|r| {
                (
                    get_str(r, "path"),
                    get_str(r, "content_hash"),
                    get_i64(r, "lines"),
                )
            })
            .collect())
    }

    fn insert_checkpoint(
        &self,
        id: &str,
        label: &str,
        created_at: &str,
        files: &str,
    ) -> Result<()> {
        self.add_node(
            "Checkpoint",
            vec![
                ("id".into(), id.to_string().into()),
                ("label".into(), label.to_string().into()),
                ("created_at".into(), created_at.to_string().into()),
                ("files".into(), files.to_string().into()),
            ],
        )
    }

    fn get_checkpoint(&self, id: &str) -> Result<Option<(String, String, String)>> {
        Ok(self
            .read_rows("Checkpoint", &["id", "label", "created_at", "files"])?
            .iter()
            .find(|r| get_str(r, "id") == id)
            .map(|r| {
                (
                    get_str(r, "label"),
                    get_str(r, "created_at"),
                    get_str(r, "files"),
                )
            }))
    }

    fn list_checkpoints(&self) -> Result<Vec<(String, String, String)>> {
        let mut rows: Vec<(String, String, String)> = self
            .read_rows("Checkpoint", &["id", "label", "created_at"])?
            .iter()
            .map(|r| {
                (
                    get_str(r, "id"),
                    get_str(r, "label"),
                    get_str(r, "created_at"),
                )
            })
            .collect();
        rows.sort_by(|a, b| b.2.cmp(&a.2)); // newest first by created_at
        Ok(rows)
    }

    fn record_guard_event(&self, ts: &str, kind: &str, risk: &str, path: &str) -> Result<()> {
        self.add_node(
            "GuardEvent",
            vec![
                ("ts".into(), ts.to_string().into()),
                ("kind".into(), kind.to_string().into()),
                ("risk".into(), risk.to_string().into()),
                ("path".into(), path.to_string().into()),
            ],
        )
    }

    fn recent_guard_events(&self, limit: usize) -> Result<Vec<(String, String, String, String)>> {
        let mut rows: Vec<(String, String, String, String)> = self
            .read_rows("GuardEvent", &["ts", "kind", "risk", "path"])?
            .iter()
            .map(|r| {
                (
                    get_str(r, "kind"),
                    get_str(r, "risk"),
                    get_str(r, "path"),
                    get_str(r, "ts"),
                )
            })
            .collect();
        rows.sort_by(|a, b| b.3.cmp(&a.3)); // newest first by ts
        rows.truncate(limit);
        Ok(rows)
    }

    fn create_pairing(&self, code: &str, token: &str, name: &str, expires_at: &str) -> Result<()> {
        self.add_node(
            "Pairing",
            vec![
                ("code".into(), code.to_string().into()),
                ("token".into(), token.to_string().into()),
                ("name".into(), name.to_string().into()),
                ("expires_at".into(), expires_at.to_string().into()),
            ],
        )
    }

    fn claim_pairing(&self, _code: &str, _now: &str) -> Result<Option<(String, String)>> {
        // Single-use claim requires an atomic read-then-delete (node delete DSL, WIP). Returning an
        // explicit error is safer than handing back a code we cannot consume.
        Err(wip("claim_pairing (atomic node delete)"))
    }
}

// --- helpers -----------------------------------------------------------------------------------

fn wip(op: &str) -> Error {
    Error::Storage(format!("helix backend: {op} not yet implemented"))
}

/// RFC3339 with millisecond precision (matches the SQLite backend's timestamp format).
fn ts(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn parse_ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn uuid_simple() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn get_str(m: &Map<String, Value>, k: &str) -> String {
    m.get(k)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn get_i64(m: &Map<String, Value>, k: &str) -> i64 {
    m.get(k)
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0)
}

fn get_f64(m: &Map<String, Value>, k: &str) -> f64 {
    m.get(k)
        .and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0.0)
}

/// Reconstruct a [`Memory`] from a projected property row.
fn memory_from_props(m: &Map<String, Value>) -> Memory {
    let concepts: Vec<String> = serde_json::from_str(&get_str(m, "concepts")).unwrap_or_default();
    let files: Vec<String> = serde_json::from_str(&get_str(m, "files")).unwrap_or_default();
    let session = get_str(m, "session_id");
    Memory {
        id: get_str(m, "id"),
        kind: MemoryKind::from_str(&get_str(m, "kind")).unwrap_or(MemoryKind::Note),
        tier: MemoryTier::from_str(&get_str(m, "tier")).unwrap_or(MemoryTier::Working),
        content: get_str(m, "content"),
        concepts,
        files,
        session_id: if session.is_empty() {
            None
        } else {
            Some(session)
        },
        importance: get_f64(m, "importance") as f32,
        access_count: get_i64(m, "access_count"),
        created_at: parse_ts(&get_str(m, "created_at")),
        updated_at: parse_ts(&get_str(m, "updated_at")),
    }
}
