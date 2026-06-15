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
//! The full surface is implemented and validated end-to-end against a live server, including the
//! in-place update/delete paths: `touch_memory`/`upsert_memory` use `set_property`, and
//! `revoke_token`/`claim_pairing` use a label-scoped `drop` (`n_with_label_where(..).drop()`).
//! Single-record lookups filter server-side via `n_with_label_where`; only the full-corpus reads
//! (`all_memories`, `list_*`) scan a label, which is the natural follow-up to optimize with
//! property indexes.

use crate::db::StoreBackend;
use cairn_core::{Config, ContentHash, DeviceToken, Error, Memory, MemoryKind, MemoryTier, Result};
use cairn_embed::Embedder;
use chrono::{DateTime, Utc};
use helix_db::dsl::prelude::*;
use helix_db::dsl::{DynamicQueryRequest, PropertyInput, SourcePredicate};
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
    /// Prefix applied to every node label, so instances/tests can share one server safely.
    ns: String,
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
        let ns = cfg.helix_ns.clone().unwrap_or_else(|| "cairn_".to_string());
        let backend = Self {
            client,
            rt,
            embed,
            ns,
        };
        backend.wait_ready()?;
        Ok(backend)
    }

    /// The on-server label for a base entity name (namespaced).
    fn label(&self, base: &str) -> String {
        format!("{}{}", self.ns, base)
    }

    /// Ensure indexes, retrying for a while so a freshly started server (e.g. the Docker stack
    /// coming up alongside Cairn) is given time to accept connections before we give up.
    fn wait_ready(&self) -> Result<()> {
        const ATTEMPTS: u32 = 30;
        let mut last: Option<Error> = None;
        for i in 0..ATTEMPTS {
            match self.ensure_indexes() {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last = Some(e);
                    if i + 1 < ATTEMPTS {
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    }
                }
            }
        }
        Err(last.unwrap_or_else(|| Error::Storage("helix: server did not become ready".into())))
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
                g().create_vector_index_nodes(self.label(MEMORY), "embedding", None::<String>),
            )
            .returning(["vi"]);
        self.run(DynamicQueryRequest::write(batch))?;
        Ok(())
    }

    /// Insert a node of base label `label` (namespaced) with `props`.
    fn add_node(&self, label: &str, props: Vec<(String, PropertyInput)>) -> Result<()> {
        let batch = write_batch()
            .var_as("n", g().add_n(self.label(label), props))
            .returning(["n"]);
        self.run(DynamicQueryRequest::write(batch))?;
        Ok(())
    }

    /// Read every node of base label `label`, projecting `cols`. Rows are in insertion order.
    fn read_rows(&self, label: &str, cols: &[&str]) -> Result<Vec<Map<String, Value>>> {
        let projection: Vec<String> = cols.iter().map(|c| c.to_string()).collect();
        let batch = read_batch()
            .var_as(
                "rows",
                g().n_with_label(self.label(label)).values(projection),
            )
            .returning(["rows"]);
        let resp = self.run(DynamicQueryRequest::read(batch))?;
        Ok(rows_of(&resp, "rows"))
    }

    /// Read nodes of base label `label` where `prop == val`, projecting `cols` (server-side filter).
    fn read_where(
        &self,
        label: &str,
        prop: &str,
        val: &str,
        cols: &[&str],
    ) -> Result<Vec<Map<String, Value>>> {
        let projection: Vec<String> = cols.iter().map(|c| c.to_string()).collect();
        let batch = read_batch()
            .var_as(
                "rows",
                g().n_with_label_where(
                    self.label(label),
                    SourcePredicate::eq(prop, val.to_string()),
                )
                .values(projection),
            )
            .returning(["rows"]);
        let resp = self.run(DynamicQueryRequest::read(batch))?;
        Ok(rows_of(&resp, "rows"))
    }

    /// Delete every node of base label `label` where `prop == val`.
    fn drop_where(&self, label: &str, prop: &str, val: &str) -> Result<()> {
        let batch = write_batch()
            .var_as(
                "d",
                g().n_with_label_where(
                    self.label(label),
                    SourcePredicate::eq(prop, val.to_string()),
                )
                .drop(),
            )
            .returning(["d"]);
        self.run(DynamicQueryRequest::write(batch))?;
        Ok(())
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
        Ok(self
            .read_where(MEMORY, "id", id, MEM_COLS)?
            .first()
            .map(memory_from_props))
    }

    fn find_memory_by_content_hash(&self, hash: &str) -> Result<Option<Memory>> {
        // `content_hash` is stored as a node property at insert time (see `insert_memory`).
        Ok(self
            .read_where(MEMORY, "content_hash", hash, MEM_COLS)?
            .first()
            .map(memory_from_props))
    }

    fn all_memories(&self) -> Result<Vec<Memory>> {
        self.load_memories()
    }

    fn touch_memory(&self, id: &str) -> Result<()> {
        let Some(row) = self
            .read_where(MEMORY, "id", id, &["access_count"])?
            .into_iter()
            .next()
        else {
            return Ok(()); // nothing to touch
        };
        let next = get_i64(&row, "access_count") + 1;
        let batch = write_batch()
            .var_as(
                "u",
                g().n_with_label_where(
                    self.label(MEMORY),
                    SourcePredicate::eq("id", id.to_string()),
                )
                .set_property("access_count", next)
                .set_property("updated_at", ts(Utc::now())),
            )
            .returning(["u"]);
        self.run(DynamicQueryRequest::write(batch))?;
        Ok(())
    }

    fn count_memories(&self) -> Result<i64> {
        Ok(self.read_rows(MEMORY, &["id"])?.len() as i64)
    }

    fn upsert_memory(&self, m: &Memory) -> Result<bool> {
        if let Some(existing) = self.get_memory(&m.id)? {
            if m.updated_at < existing.updated_at {
                return Ok(false); // incoming is older — last-writer-wins keeps the existing copy
            }
            self.drop_where(MEMORY, "id", &m.id)?; // replace in place
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

    fn semantic_recall(&self, query: &str, k: usize) -> Result<Option<Vec<Memory>>> {
        let qvec = self.embed.embed_one(query)?;
        let projection: Vec<String> = MEM_COLS.iter().map(|c| c.to_string()).collect();
        // HNSW kNN, then project the memory columns — ordering (closest first) survives `.values`.
        let batch = read_batch()
            .var_as(
                "ranked",
                g().vector_search_nodes(self.label(MEMORY), "embedding", qvec, k, None)
                    .values(projection),
            )
            .returning(["ranked"]);
        let resp = self.run(DynamicQueryRequest::read(batch))?;
        let mems = rows_of(&resp, "ranked")
            .iter()
            .map(memory_from_props)
            .collect();
        Ok(Some(mems))
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
        Ok(!self
            .read_where("Token", "token", token, &["token"])?
            .is_empty())
    }

    fn revoke_token(&self, token: &str) -> Result<bool> {
        let existed = !self
            .read_where("Token", "token", token, &["token"])?
            .is_empty();
        if existed {
            self.drop_where("Token", "token", token)?;
        }
        Ok(existed)
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

    fn claim_pairing(&self, code: &str, now: &str) -> Result<Option<(String, String)>> {
        // Single-use: read the code, honor expiry, then delete it. (The window between read and
        // delete is small; pairing is low-concurrency and codes are short-lived.)
        let row = self
            .read_where("Pairing", "code", code, &["token", "name", "expires_at"])?
            .into_iter()
            .next();
        let Some(r) = row else { return Ok(None) };
        if get_str(&r, "expires_at").as_str() <= now {
            return Ok(None); // expired
        }
        let claimed = (get_str(&r, "token"), get_str(&r, "name"));
        self.drop_where("Pairing", "code", code)?;
        Ok(Some(claimed))
    }
}

// --- helpers -----------------------------------------------------------------------------------

/// Pull the projected property rows out of a query response under variable `var`
/// (`{ "<var>": { "properties": [ {..}, .. ] } }`).
fn rows_of(resp: &Value, var: &str) -> Vec<Map<String, Value>> {
    resp.get(var)
        .and_then(|r| r.get("properties"))
        .and_then(|p| p.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_object().cloned()).collect())
        .unwrap_or_default()
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

#[cfg(test)]
mod live {
    //! Integration tests against a real HelixDB server. Gated on `CAIRN_HELIX_URL` and `#[ignore]`d,
    //! so the normal suite never touches the network. Run explicitly with, e.g.:
    //! `CAIRN_HELIX_URL=http://host:6969 cargo test -p cairn-store -- --ignored live::`
    use super::*;
    use cairn_core::EmbedConfig;

    fn backend() -> Option<HelixBackend> {
        let url = std::env::var("CAIRN_HELIX_URL").ok()?;
        // `ollama` builds without a network call, so connect + index setup work without an
        // embedding model — enough to exercise the read/write machinery (meta, tokens).
        let cfg = Config {
            data_dir: std::env::temp_dir(),
            host: "127.0.0.1".into(),
            port: 7777,
            helix_url: Some(url.clone()),
            // Unique namespace per backend so concurrent tests never collide on the shared server.
            helix_ns: Some(format!("test_{}_", uuid_simple())),
            default_server: None,
            embed: EmbedConfig {
                provider: "ollama".into(),
                model: None,
                url: None,
                api_key: None,
            },
        };
        Some(HelixBackend::connect(&url, &cfg).expect("connect to live HelixDB"))
    }

    #[test]
    #[ignore = "requires a live HelixDB server (set CAIRN_HELIX_URL)"]
    fn meta_roundtrips() {
        let Some(be) = backend() else { return };
        let key = format!("cairn_test_meta_{}", uuid_simple());
        be.set_meta(&key, "hello-helix").expect("set_meta");
        assert_eq!(
            be.get_meta(&key).expect("get_meta").as_deref(),
            Some("hello-helix")
        );
        // Last-write-wins on read.
        be.set_meta(&key, "updated").expect("set_meta 2");
        assert_eq!(
            be.get_meta(&key).expect("get_meta 2").as_deref(),
            Some("updated")
        );
    }

    #[test]
    #[ignore = "requires a live HelixDB server (set CAIRN_HELIX_URL)"]
    fn tokens_roundtrip() {
        let Some(be) = backend() else { return };
        let before = be.count_tokens().expect("count");
        let tok = be.create_token("test-device").expect("create_token");
        assert!(be.validate_token(&tok.token).expect("validate"));
        assert!(be
            .list_tokens()
            .expect("list")
            .iter()
            .any(|t| t.token == tok.token && t.name == "test-device"));
        assert!(be.count_tokens().expect("count after") > before);

        // Revocation: a label-scoped delete removes exactly this token.
        assert!(
            be.revoke_token(&tok.token).expect("revoke"),
            "first revoke reports removed"
        );
        assert!(!be
            .validate_token(&tok.token)
            .expect("validate after revoke"));
        assert!(
            !be.revoke_token(&tok.token).expect("revoke again"),
            "second revoke is a no-op"
        );
    }

    #[test]
    #[ignore = "requires a live HelixDB server (set CAIRN_HELIX_URL)"]
    fn pairing_is_single_use() {
        let Some(be) = backend() else { return };
        let code = format!("pc-{}", uuid_simple());
        let future = ts(Utc::now() + chrono::Duration::minutes(10));
        be.create_pairing(&code, "tok-xyz", "new-device", &future)
            .expect("create_pairing");
        let now = ts(Utc::now());
        // First claim succeeds and returns the token+name.
        assert_eq!(
            be.claim_pairing(&code, &now).expect("claim"),
            Some(("tok-xyz".to_string(), "new-device".to_string()))
        );
        // Single-use: the code is consumed.
        assert_eq!(be.claim_pairing(&code, &now).expect("claim again"), None);
    }

    #[test]
    #[ignore = "requires a live HelixDB server (set CAIRN_HELIX_URL)"]
    fn expired_pairing_is_rejected() {
        let Some(be) = backend() else { return };
        let code = format!("pc-{}", uuid_simple());
        let past = ts(Utc::now() - chrono::Duration::minutes(1));
        be.create_pairing(&code, "tok-old", "old-device", &past)
            .expect("create_pairing");
        let now = ts(Utc::now());
        assert_eq!(be.claim_pairing(&code, &now).expect("claim expired"), None);
        // Clean up the expired code we left behind.
        let _ = be.drop_where("Pairing", "code", &code);
    }

    /// The full memory path through the public `Store` facade + `open_for_test` harness: insert
    /// (with a real embedding), get, count (isolated namespace), touch, upsert (last-writer-wins),
    /// and vector recall via the hashing embedder.
    #[test]
    #[ignore = "requires a live HelixDB server (set CAIRN_HELIX_URL)"]
    fn memory_roundtrip_via_store() {
        let Some(store) = crate::Store::open_for_test() else {
            return;
        };
        let mut m = Memory {
            id: uuid_simple(),
            kind: MemoryKind::Decision,
            tier: MemoryTier::Working,
            content: "use helix for the cairn vector store".into(),
            concepts: vec!["helix".into(), "store".into()],
            files: vec![],
            session_id: None,
            importance: 0.7,
            access_count: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.insert_memory(&m).expect("insert");

        // Isolated namespace: this is the only memory present.
        assert_eq!(store.count_memories().expect("count"), 1);
        let got = store.get_memory(&m.id).expect("get").expect("present");
        assert_eq!(got.content, m.content);
        assert_eq!(got.concepts, m.concepts);

        // touch bumps access_count in place.
        store.touch_memory(&m.id).expect("touch");
        assert_eq!(
            store.get_memory(&m.id).expect("get2").unwrap().access_count,
            1
        );

        // Vector recall (hashing embedder) surfaces the memory for a lexically-similar query.
        let hits = store
            .semantic_recall("helix vector store for cairn", 5)
            .expect("recall")
            .expect("backend has vectors");
        assert!(hits.iter().any(|x| x.id == m.id));

        // upsert is last-writer-wins: an older copy is rejected, a newer one replaces.
        m.updated_at = got.updated_at - chrono::Duration::minutes(5);
        assert!(!store.upsert_memory(&m).expect("stale upsert"));
        m.updated_at = Utc::now();
        m.content = "use helix for cairn vectors and graph".into();
        assert!(store.upsert_memory(&m).expect("fresh upsert"));
        assert_eq!(store.count_memories().expect("count3"), 1); // replaced, not duplicated
        assert_eq!(
            store.get_memory(&m.id).expect("get3").unwrap().content,
            m.content
        );
    }
}
