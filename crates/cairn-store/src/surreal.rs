//! The SurrealDB backend.
//!
//! [`SurrealStore`] is Cairn's [`StoreBackend`](crate::db::StoreBackend): it persists to a
//! SurrealDB server over its WebSocket RPC via the `surrealdb` crate. `CAIRN_DB_URL`
//! (`Config::db_url`, default `ws://localhost:8000`) names the server; `Config::db_ns` selects
//! the namespace, so instances/tests can share one server safely without colliding. The bundled
//! `docker compose` stack starts a server with no host port - it is reachable only over the
//! Docker-internal network, and Cairn always connects to it as a client.
//!
//! ## Sync <-> async bridge
//! `StoreBackend` is synchronous; the `surrealdb` client is async (tokio). Each call hops onto a
//! process-wide shared [`tokio::runtime::Runtime`] and `block_on`s from a *scoped OS thread* (not
//! the caller's thread), so this is safe whether the caller is plain sync (tests) or already inside
//! a `#[tokio::main]` runtime (the server) - the latter would otherwise panic with "Cannot start a
//! runtime from within a runtime". The runtime is shared (never dropped) so a backend can be
//! created and dropped inside an async context without the "drop a runtime in async" panic.
//!
//! ## Data model
//! Every table is `SCHEMALESS` - Cairn owns validation in Rust, the same trust boundary as the old
//! HelixDB property bags. Tables with a natural unique key (`memory`, `token`, `meta`,
//! `sync_state`, `checkpoint`, `pairing`, the `audit_counter` singleton) use that key directly as
//! the SurrealDB record id via `type::record(table, $key)`, so single-record reads/writes/deletes
//! are O(1) with no secondary index. Append-only logs with no natural key (`file_version`,
//! `guard_event`, `audit_event`) use an auto-generated record id and are queried by `ORDER BY`.
//! Vector search runs through a `DEFINE INDEX ... HNSW` index over `memory.embedding` (see
//! `schema.surql`), queried with the `<|k,ef|>` approximate-nearest-neighbor operator.

use crate::db::{AuditRecord, ProjectRecord, StoreBackend};
use cairn_core::{
    Config, ContentHash, DeviceToken, Error, Memory, MemoryKind, MemoryTier, OrgId, Result,
    ScopeType, TokenScope,
};
use cairn_embed::Embedder;
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value as Json};
use std::future::Future;
use std::str::FromStr;
use surrealdb::engine::remote::ws::{Client, Ws, Wss};
use surrealdb::opt::auth::Root;
use surrealdb::{IndexedResults, Surreal};

/// The fixed database name within a Cairn instance's namespace. `Config::db_ns` is the real
/// isolation boundary (one per instance/test); a single database inside it is enough.
const DB_NAME: &str = "cairn";

/// The idempotent schema, applied on every connect. `{dim}` is substituted with the active
/// embedder's output width before being sent to the server.
const SCHEMA_TEMPLATE: &str = include_str!("schema.surql");

/// A process-wide tokio runtime that drives the async SurrealDB client. Shared (and never
/// dropped) so that a `SurrealStore` can be created and dropped inside an async context (axum,
/// `#[tokio::test]`) without panicking - owning a `Runtime` and dropping it from async code is
/// not allowed.
fn shared_runtime() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build shared tokio runtime")
    })
}

/// Run `fut` to completion on the shared runtime from a scoped OS thread (runtime-nesting safe).
fn block_on<F>(fut: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send,
{
    let rt = shared_runtime();
    std::thread::scope(|s| s.spawn(move || rt.block_on(fut)).join().unwrap())
}

/// A SurrealDB-backed structured store.
pub(crate) struct SurrealStore {
    db: Surreal<Client>,
    embed: Box<dyn Embedder>,
    /// Per-query deadline. Read from `Config::db_timeout_secs` at connect time (default 10 s).
    query_timeout: std::time::Duration,
}

impl SurrealStore {
    /// Connect to the SurrealDB server described by `cfg`, build the embedder, and apply the
    /// idempotent schema (including the HNSW vector index, sized to the embedder's output width).
    pub(crate) fn connect(cfg: &Config) -> Result<Self> {
        let embed = cairn_embed::from_config(&cfg.embed)?;
        let dim = embed.dim();
        let query_timeout = std::time::Duration::from_secs(cfg.db_timeout_secs);
        let db = Self::connect_with_retry(cfg, dim)?;
        Ok(Self {
            db,
            embed,
            query_timeout,
        })
    }

    /// Ensure indexes, retrying for a while so a freshly started server (e.g. the Docker stack
    /// coming up alongside Cairn) is given time to accept connections before we give up.
    fn connect_with_retry(cfg: &Config, dim: usize) -> Result<Surreal<Client>> {
        const ATTEMPTS: u32 = 30;
        let mut last: Option<Error> = None;
        for i in 0..ATTEMPTS {
            match Self::try_connect(cfg, dim) {
                Ok(db) => return Ok(db),
                Err(e) => {
                    last = Some(e);
                    if i + 1 < ATTEMPTS {
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    }
                }
            }
        }
        Err(last.unwrap_or_else(|| Error::Storage("surrealdb: server did not become ready".into())))
    }

    fn try_connect(cfg: &Config, dim: usize) -> Result<Surreal<Client>> {
        if cfg.db_url.starts_with("ws://") {
            let is_loopback = cfg.db_url.contains("127.0.0.1")
                || cfg.db_url.contains("localhost")
                || cfg.db_url.contains("[::1]")
                // The Docker-internal network name used by the bundled compose stack.
                || cfg.db_url.contains("surreal:");
            if !is_loopback {
                tracing::warn!(
                    "SurrealDB URL is plain ws:// ({}) - credentials travel in cleartext. Use \
                     wss:// or a loopback/internal address.",
                    redact_url(&cfg.db_url)
                );
            }
        }
        let (is_wss, host_port) = split_endpoint(&cfg.db_url);
        let host_port = host_port.to_string();
        let user = cfg.db_user.clone();
        let pass = cfg.db_pass.clone();
        let ns = cfg.db_ns.clone();
        let schema = SCHEMA_TEMPLATE.replace("{dim}", &dim.to_string());

        block_on(async move {
            let db: Surreal<Client> = if is_wss {
                Surreal::new::<Wss>(host_port.as_str()).await
            } else {
                Surreal::new::<Ws>(host_port.as_str()).await
            }
            .map_err(|e| Error::Storage(format!("surrealdb connect: {e}")))?;
            if !user.is_empty() {
                db.signin(Root {
                    username: user,
                    password: pass,
                })
                .await
                .map_err(|e| Error::Storage(format!("surrealdb signin: {e}")))?;
            }
            db.use_ns(&ns)
                .use_db(DB_NAME)
                .await
                .map_err(|e| Error::Storage(format!("surrealdb use_ns/use_db: {e}")))?;
            db.query(schema)
                .await
                .map_err(|e| Error::Storage(format!("surrealdb schema: {e}")))?;
            Ok(db)
        })
    }

    /// Run `sql` (with `$`-bound `vars`) on the shared runtime under the query deadline.
    fn q(&self, sql: impl Into<String>, vars: Json) -> Result<IndexedResults> {
        let sql = sql.into();
        let timeout = self.query_timeout;
        block_on(async move {
            match tokio::time::timeout(timeout, self.db.query(sql).bind(vars)).await {
                Ok(inner) => inner.map_err(|e| Error::Storage(format!("surrealdb query: {e}"))),
                Err(_elapsed) => Err(Error::Storage(format!(
                    "surrealdb query timed out after {}s (set CAIRN_DB_TIMEOUT_SECS to override)",
                    timeout.as_secs()
                ))),
            }
        })
    }

    /// Run `sql` and decode the first statement's result as a list of JSON objects (rows).
    fn read_rows(&self, sql: impl Into<String>, vars: Json) -> Result<Vec<Map<String, Json>>> {
        let mut resp = self.q(sql, vars)?;
        let vals: Vec<Json> = resp
            .take(0)
            .map_err(|e| Error::Storage(format!("surrealdb decode: {e}")))?;
        Ok(vals.into_iter().filter_map(|v| v.as_object().cloned()).collect())
    }

    /// `RETURN array::len(SELECT VALUE id FROM <table>)` - a row count that never errors on an
    /// empty table (unlike `GROUP ALL`, which returns zero rows rather than a `count: 0` row).
    fn count_table(&self, table: &str) -> Result<i64> {
        let sql = format!("RETURN array::len(SELECT VALUE id FROM {table})");
        let mut resp = self.q(sql, json!({}))?;
        let n: Option<i64> = resp
            .take(0)
            .map_err(|e| Error::Storage(format!("surrealdb decode: {e}")))?;
        Ok(n.unwrap_or(0))
    }

    /// All memories, newest first.
    fn load_memories(&self) -> Result<Vec<Memory>> {
        let rows = self.read_rows(
            "SELECT *, record::id(id) AS rid FROM memory ORDER BY created_at DESC",
            json!({}),
        )?;
        Ok(rows.iter().map(memory_from_props).collect())
    }

    /// Read+increment the persistent `audit_counter:current` singleton in one atomic statement.
    /// Returns the post-increment value (the id assigned to the next appended audit event).
    fn bump_audit_counter(&self) -> Result<i64> {
        let rows = self.read_rows(
            "UPSERT audit_counter:current SET val = (val ?? 0) + 1 RETURN AFTER",
            json!({}),
        )?;
        Ok(rows.first().map(|r| get_i64(r, "val")).unwrap_or(1))
    }
}

impl StoreBackend for SurrealStore {
    fn insert_memory(&self, m: &Memory) -> Result<()> {
        let embedding = self.embed.embed_one(&m.content)?;
        let hash = ContentHash::of_str(&m.content);
        let vars = json!({
            "id": m.id,
            "data": {
                "kind": m.kind.as_str(),
                "tier": m.tier.as_str(),
                "content": m.content,
                "content_hash": hash.as_str(),
                "concepts": m.concepts,
                "files": m.files,
                "session_id": m.session_id.clone().unwrap_or_default(),
                "importance": m.importance,
                "access_count": m.access_count,
                "suspicious": m.suspicious,
                "confidence": m.confidence,
                "pinned": m.pinned,
                "derived_from": m.derived_from,
                "contradicts": m.contradicts,
                "supersedes": m.supersedes,
                "applies_to": m.applies_to,
                "scope_type": m.scope_type.as_str(),
                "scope_id": m.scope_id.clone().unwrap_or_default(),
                "created_at": ts(m.created_at),
                "updated_at": ts(m.updated_at),
                "embedding": embedding,
            },
        });
        self.q("CREATE type::record('memory', $id) CONTENT $data", vars)?;
        Ok(())
    }

    fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        let rows = self.read_rows(
            "SELECT *, record::id(id) AS rid FROM type::record('memory', $id)",
            json!({ "id": id }),
        )?;
        Ok(rows.first().map(memory_from_props))
    }

    fn find_memory_by_content_hash(&self, hash: &str) -> Result<Option<Memory>> {
        let rows = self.read_rows(
            "SELECT *, record::id(id) AS rid FROM memory WHERE content_hash = $hash LIMIT 1",
            json!({ "hash": hash }),
        )?;
        Ok(rows.first().map(memory_from_props))
    }

    fn all_memories(&self) -> Result<Vec<Memory>> {
        self.load_memories()
    }

    fn touch_memory(&self, id: &str) -> Result<()> {
        // `UPDATE` (unlike `UPSERT`) never creates a missing record, so this is a no-op when
        // `id` doesn't exist - matching the trait's "nothing to touch" contract.
        self.q(
            "UPDATE type::record('memory', $id) SET access_count += 1, updated_at = $now",
            json!({ "id": id, "now": ts(Utc::now()) }),
        )?;
        Ok(())
    }

    fn count_memories(&self) -> Result<i64> {
        self.count_table("memory")
    }

    fn upsert_memory(&self, m: &Memory) -> Result<bool> {
        if let Some(existing) = self.get_memory(&m.id)? {
            if m.updated_at < existing.updated_at {
                return Ok(false); // incoming is older - last-writer-wins keeps the existing copy
            }
            self.q(
                "DELETE type::record('memory', $id)",
                json!({ "id": &m.id }),
            )?;
        }
        self.insert_memory(m)?;
        Ok(true)
    }

    fn memories_since(&self, since: DateTime<Utc>) -> Result<Vec<Memory>> {
        let rows = self.read_rows(
            "SELECT *, record::id(id) AS rid FROM memory WHERE updated_at > $since ORDER BY created_at DESC",
            json!({ "since": ts(since) }),
        )?;
        Ok(rows.iter().map(memory_from_props).collect())
    }

    fn reinforce_memory(&self, id: &str) -> Result<()> {
        // Agentmemory reinforcement: c' = min(1.0, c + 0.1*(1.0 - c)), applied server-side so a
        // missing row is a no-op (same guard as `touch_memory`).
        self.q(
            "UPDATE type::record('memory', $id) SET \
             confidence = math::min([1.0, confidence + 0.1 * (1.0 - confidence)]), \
             access_count += 1, \
             updated_at = $now",
            json!({ "id": id, "now": ts(Utc::now()) }),
        )?;
        Ok(())
    }

    fn set_pinned(&self, id: &str, pinned: bool) -> Result<()> {
        self.q(
            "UPDATE type::record('memory', $id) SET pinned = $pinned, updated_at = $now",
            json!({ "id": id, "pinned": pinned, "now": ts(Utc::now()) }),
        )?;
        Ok(())
    }

    fn edit_memory(
        &self,
        id: &str,
        content: Option<String>,
        importance: Option<f32>,
        concepts: Option<Vec<String>>,
        files: Option<Vec<String>>,
    ) -> Result<bool> {
        let Some(existing) = self.get_memory(id)? else {
            return Ok(false);
        };
        let mut updated = existing;
        if let Some(c) = content {
            updated.content = c;
        }
        if let Some(i) = importance {
            updated.importance = i.clamp(0.0, 1.0);
        }
        if let Some(c) = concepts {
            updated.concepts = c;
        }
        if let Some(f) = files {
            updated.files = f;
        }
        updated.updated_at = Utc::now();
        // Drop + reinsert so all properties land together (and the vector index re-embeds the
        // new content), mirroring the prior backend's edit semantics.
        self.q(
            "DELETE type::record('memory', $id)",
            json!({ "id": &updated.id }),
        )?;
        self.insert_memory(&updated)?;
        Ok(true)
    }

    fn delete_memory(&self, id: &str) -> Result<bool> {
        let rows = self.read_rows(
            "DELETE type::record('memory', $id) RETURN BEFORE",
            json!({ "id": id }),
        )?;
        Ok(!rows.is_empty())
    }

    fn semantic_recall(&self, query: &str, k: usize) -> Result<Option<Vec<Memory>>> {
        let qvec = self.embed.embed_one(query)?;
        // `<|k,ef|>` is the HNSW approximate-nearest-neighbor operator; k/ef are internally
        // computed integers (not user text), so interpolating them into the query string carries
        // no injection risk - the SurrealQL grammar for this operator doesn't accept bind params.
        let ef = (k as u32).saturating_mul(4).max(40);
        let sql = format!(
            "SELECT *, record::id(id) AS rid FROM memory WHERE embedding <|{k},{ef}|> $qvec"
        );
        let rows = self.read_rows(sql, json!({ "qvec": qvec }))?;
        Ok(Some(rows.iter().map(memory_from_props).collect()))
    }

    fn create_token(
        &self,
        name: &str,
        scope: TokenScope,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<DeviceToken> {
        let id = uuid_simple();
        let now = Utc::now();
        let token = DeviceToken {
            id: id.clone(),
            token: None,
            name: name.to_string(),
            scope,
            expires_at,
            last_used_at: None,
            created_at: now,
        };
        let vars = json!({
            "id": id,
            "data": {
                "name": token.name,
                "scope": token.scope.as_str(),
                "created_at": ts(now),
                "expires_at": expires_at.map(ts).unwrap_or_default(),
                "last_used_at": "",
            },
        });
        self.q("CREATE type::record('token', $id) CONTENT $data", vars)?;
        Ok(token)
    }

    fn validate_token_id(&self, token_id: &str) -> Result<bool> {
        let rows = self.read_rows(
            "SELECT id FROM type::record('token', $id)",
            json!({ "id": token_id }),
        )?;
        Ok(!rows.is_empty())
    }

    fn record_token_usage(&self, token_id: &str) -> Result<()> {
        self.q(
            "UPDATE type::record('token', $id) SET last_used_at = $now",
            json!({ "id": token_id, "now": ts(Utc::now()) }),
        )?;
        Ok(())
    }

    fn revoke_token(&self, token_id: &str) -> Result<bool> {
        let rows = self.read_rows(
            "DELETE type::record('token', $id) RETURN BEFORE",
            json!({ "id": token_id }),
        )?;
        Ok(!rows.is_empty())
    }

    fn list_tokens(&self) -> Result<Vec<DeviceToken>> {
        let rows = self.read_rows("SELECT *, record::id(id) AS rid FROM token", json!({}))?;
        Ok(rows
            .iter()
            .map(|r| {
                let mut t = DeviceToken::meta(
                    get_str(r, "rid"),
                    get_str(r, "name"),
                    parse_ts(&get_str(r, "created_at")),
                );
                t.scope = get_str(r, "scope").parse().unwrap_or(TokenScope::Write);
                {
                    let exp = get_str(r, "expires_at");
                    if !exp.is_empty() {
                        t.expires_at = Some(parse_ts(&exp));
                    }
                }
                {
                    let lua = get_str(r, "last_used_at");
                    if !lua.is_empty() {
                        t.last_used_at = Some(parse_ts(&lua));
                    }
                }
                t
            })
            .collect())
    }

    fn count_tokens(&self) -> Result<i64> {
        self.count_table("token")
    }

    fn get_last_sync(&self, server: &str) -> Result<Option<DateTime<Utc>>> {
        let rows = self.read_rows(
            "SELECT at FROM type::record('sync_state', $server)",
            json!({ "server": server }),
        )?;
        Ok(rows.first().map(|r| parse_ts(&get_str(r, "at"))))
    }

    fn set_last_sync(&self, server: &str, when: DateTime<Utc>) -> Result<()> {
        self.q(
            "UPSERT type::record('sync_state', $server) SET at = $at",
            json!({ "server": server, "at": ts(when) }),
        )?;
        Ok(())
    }

    fn record_file_version(&self, path: &str, content_hash: &str, lines: i64) -> Result<()> {
        self.q(
            "CREATE file_version CONTENT $data",
            json!({
                "data": {
                    "path": path,
                    "content_hash": content_hash,
                    "lines": lines,
                    "created_at": ts(Utc::now()),
                },
            }),
        )?;
        Ok(())
    }

    fn latest_file_version(&self, path: &str) -> Result<Option<(String, i64)>> {
        let rows = self.read_rows(
            "SELECT content_hash, lines, created_at FROM file_version WHERE path = $path \
             ORDER BY created_at DESC LIMIT 1",
            json!({ "path": path }),
        )?;
        Ok(rows
            .first()
            .map(|r| (get_str(r, "content_hash"), get_i64(r, "lines"))))
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.q(
            "UPSERT type::record('meta', $key) SET val = $value",
            json!({ "key": key, "value": value }),
        )?;
        Ok(())
    }

    fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let rows = self.read_rows(
            "SELECT val FROM type::record('meta', $key)",
            json!({ "key": key }),
        )?;
        Ok(rows.first().map(|r| get_str(r, "val")))
    }

    fn all_file_versions(&self) -> Result<Vec<(String, String, i64)>> {
        let rows = self.read_rows(
            "SELECT path, content_hash, lines FROM file_version",
            json!({}),
        )?;
        Ok(rows
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
        self.q(
            "CREATE type::record('checkpoint', $id) CONTENT $data",
            json!({
                "id": id,
                "data": { "label": label, "created_at": created_at, "files": files },
            }),
        )?;
        Ok(())
    }

    fn get_checkpoint(&self, id: &str) -> Result<Option<(String, String, String)>> {
        let rows = self.read_rows(
            "SELECT label, created_at, files FROM type::record('checkpoint', $id)",
            json!({ "id": id }),
        )?;
        Ok(rows.first().map(|r| {
            (
                get_str(r, "label"),
                get_str(r, "created_at"),
                get_str(r, "files"),
            )
        }))
    }

    fn list_checkpoints(&self) -> Result<Vec<(String, String, String)>> {
        let rows = self.read_rows(
            "SELECT *, record::id(id) AS rid FROM checkpoint ORDER BY created_at DESC",
            json!({}),
        )?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    get_str(r, "rid"),
                    get_str(r, "label"),
                    get_str(r, "created_at"),
                )
            })
            .collect())
    }

    fn record_guard_event(&self, ts: &str, kind: &str, risk: &str, path: &str) -> Result<()> {
        self.q(
            "CREATE guard_event CONTENT $data",
            json!({ "data": { "ts": ts, "kind": kind, "risk": risk, "path": path } }),
        )?;
        Ok(())
    }

    fn recent_guard_events(&self, limit: usize) -> Result<Vec<(String, String, String, String)>> {
        let rows = self.read_rows(
            "SELECT kind, risk, path, ts FROM guard_event ORDER BY ts DESC LIMIT $limit",
            json!({ "limit": limit as i64 }),
        )?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    get_str(r, "kind"),
                    get_str(r, "risk"),
                    get_str(r, "path"),
                    get_str(r, "ts"),
                )
            })
            .collect())
    }

    fn create_pairing(&self, code: &str, token: &str, name: &str, expires_at: &str) -> Result<()> {
        self.q(
            "CREATE type::record('pairing', $code) CONTENT $data",
            json!({
                "code": code,
                "data": { "token": token, "name": name, "expires_at": expires_at },
            }),
        )?;
        Ok(())
    }

    fn claim_pairing(&self, code: &str, now: &str) -> Result<Option<(String, String)>> {
        // Atomic single-statement claim: the WHERE guard means the delete (and thus the
        // RETURN BEFORE payload) only fires when the code is still live.
        let rows = self.read_rows(
            "DELETE type::record('pairing', $code) WHERE expires_at > $now RETURN BEFORE",
            json!({ "code": code, "now": now }),
        )?;
        Ok(rows
            .first()
            .map(|r| (get_str(r, "token"), get_str(r, "name"))))
    }

    // -- audit log (v0.5.0 - Sprint 1) ----------------------------------------------------

    fn append_audit(&self, ts: i64, kind: &str, actor: &str, detail: &str) -> Result<String> {
        let next = self.bump_audit_counter()?;
        // `seq`, not `id` - every SurrealDB record already has a reserved `id` (the RecordId
        // identity column), and a schemaless `CONTENT` field literally named `id` gets coerced
        // into that column instead of staying the plain integer we need.
        self.q(
            "CREATE audit_event CONTENT $data",
            json!({
                "data": { "seq": next, "ts": ts, "kind": kind, "actor": actor, "detail": detail },
            }),
        )?;
        Ok(next.to_string())
    }

    fn recent_audit(&self, limit: usize, since_event_id: Option<&str>) -> Result<Vec<AuditRecord>> {
        let since = since_event_id.and_then(|s| s.parse::<i64>().ok());
        let rows = match since {
            Some(id) => self.read_rows(
                "SELECT seq, ts, kind, actor, detail FROM audit_event \
                 WHERE seq > $since ORDER BY seq DESC LIMIT $limit",
                json!({ "since": id, "limit": limit as i64 }),
            )?,
            None => self.read_rows(
                "SELECT seq, ts, kind, actor, detail FROM audit_event ORDER BY seq DESC LIMIT $limit",
                json!({ "limit": limit as i64 }),
            )?,
        };
        Ok(rows
            .iter()
            .map(|r| AuditRecord {
                id: get_i64(r, "seq"),
                ts: get_i64(r, "ts"),
                kind: get_str(r, "kind"),
                actor: get_str(r, "actor"),
                detail: get_str(r, "detail"),
            })
            .collect())
    }

    fn max_audit_event_id(&self) -> Result<i64> {
        let rows = self.read_rows("SELECT val FROM audit_counter:current", json!({}))?;
        Ok(rows.first().map(|r| get_i64(r, "val")).unwrap_or(0))
    }

    fn record_access_batch(
        &self,
        entries: &[(String, Option<String>, Option<String>)],
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let now = ts(Utc::now());
        let rows: Vec<Json> = entries
            .iter()
            .map(|(memory_id, project_id, session_id)| {
                json!({
                    "memory_id": memory_id,
                    "project_id": project_id.clone().unwrap_or_default(),
                    "session_id": session_id.clone().unwrap_or_default(),
                    "ts": now,
                })
            })
            .collect();
        // One statement, N rows - one round-trip regardless of how many memories recall
        // returned this call.
        self.q("INSERT INTO access_log $rows", json!({ "rows": rows }))?;
        Ok(())
    }

    fn upsert_project(&self, id: &str, name: &str, path: &str) -> Result<()> {
        let now = ts(Utc::now());
        // `first_seen ?? $now` preserves the original value across repeated upserts (the
        // field is absent on the very first upsert, when `??`'s left side is NONE).
        self.q(
            "UPSERT type::record('project', $id) SET \
             name = $name, path = $path, last_active = $now, first_seen = (first_seen ?? $now)",
            json!({ "id": id, "name": name, "path": path, "now": now }),
        )?;
        Ok(())
    }

    fn list_projects(&self) -> Result<Vec<ProjectRecord>> {
        let rows = self.read_rows(
            "SELECT *, record::id(id) AS rid FROM project ORDER BY last_active DESC",
            json!({}),
        )?;
        Ok(rows.iter().map(project_from_row).collect())
    }

    fn get_project(&self, id: &str) -> Result<Option<ProjectRecord>> {
        let rows = self.read_rows(
            "SELECT *, record::id(id) AS rid FROM type::record('project', $id)",
            json!({ "id": id }),
        )?;
        Ok(rows.first().map(project_from_row))
    }
}

// - helpers ---------------------------------------------------------------------------------

/// Split `ws://host:port` / `wss://host:port` into `(is_wss, host_port)`. Anything without a
/// recognized scheme is passed through unchanged (assumed plain `ws`).
fn split_endpoint(url: &str) -> (bool, &str) {
    if let Some(rest) = url.strip_prefix("wss://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("ws://") {
        (false, rest)
    } else {
        (false, url)
    }
}

/// RFC3339 with millisecond precision (matches the prior backend's timestamp format, and sorts
/// lexicographically the same as chronologically thanks to the fixed-width zero-padded format).
fn ts(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Strip userinfo (user:pass@) from a URL for safe logging.
fn redact_url(url: &str) -> String {
    if let Ok(mut parsed) = url::Url::parse(url) {
        if parsed.username() != "" || parsed.password().is_some() {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
        }
        parsed.to_string()
    } else {
        url.to_string()
    }
}

fn parse_ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn uuid_simple() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn project_from_row(m: &Map<String, Json>) -> ProjectRecord {
    ProjectRecord {
        id: get_str(m, "rid"),
        name: get_str(m, "name"),
        path: get_str(m, "path"),
        first_seen: parse_ts(&get_str(m, "first_seen")),
        last_active: parse_ts(&get_str(m, "last_active")),
    }
}

fn get_str(m: &Map<String, Json>, k: &str) -> String {
    m.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

fn get_i64(m: &Map<String, Json>, k: &str) -> i64 {
    m.get(k)
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        .unwrap_or(0)
}

fn get_f64(m: &Map<String, Json>, k: &str) -> f64 {
    m.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

fn get_bool(m: &Map<String, Json>, k: &str) -> bool {
    m.get(k).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn get_str_vec(m: &Map<String, Json>, k: &str) -> Vec<String> {
    m.get(k)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Reconstruct a [`Memory`] from a projected property row. Every memory-shaped query must
/// project `record::id(id) AS rid` alongside `*` so `rid` carries the app-level id string.
fn memory_from_props(m: &Map<String, Json>) -> Memory {
    let session = get_str(m, "session_id");
    Memory {
        id: get_str(m, "rid"),
        kind: MemoryKind::from_str(&get_str(m, "kind")).unwrap_or(MemoryKind::Note),
        tier: MemoryTier::from_str(&get_str(m, "tier")).unwrap_or(MemoryTier::Working),
        content: get_str(m, "content"),
        concepts: get_str_vec(m, "concepts"),
        files: get_str_vec(m, "files"),
        session_id: if session.is_empty() { None } else { Some(session) },
        importance: get_f64(m, "importance") as f32,
        access_count: get_i64(m, "access_count"),
        org_id: OrgId::default(),
        suspicious: get_bool(m, "suspicious"),
        confidence: get_f64(m, "confidence") as f32,
        pinned: get_bool(m, "pinned"),
        derived_from: get_str_vec(m, "derived_from"),
        contradicts: get_str_vec(m, "contradicts"),
        supersedes: get_str_vec(m, "supersedes"),
        applies_to: get_str_vec(m, "applies_to"),
        scope_type: ScopeType::from_str(&get_str(m, "scope_type")).unwrap_or(ScopeType::Global),
        scope_id: {
            let sid = get_str(m, "scope_id");
            if sid.is_empty() {
                None
            } else {
                Some(sid)
            }
        },
        created_at: parse_ts(&get_str(m, "created_at")),
        updated_at: parse_ts(&get_str(m, "updated_at")),
    }
}

#[cfg(test)]
mod live {
    //! Integration tests against a real SurrealDB server. Gated on `CAIRN_DB_URL` and `#[ignore]`d,
    //! so the normal suite never touches the network. Run explicitly with, e.g.:
    //! `CAIRN_DB_URL=ws://localhost:8000 cargo test -p cairn-store -- --ignored live::`
    use super::*;
    use cairn_core::EmbedConfig;

    fn backend() -> Option<SurrealStore> {
        let url = std::env::var("CAIRN_DB_URL").ok()?;
        let cfg = Config {
            data_dir: std::env::temp_dir(),
            host: "127.0.0.1".into(),
            port: 7777,
            db_url: url,
            db_user: std::env::var("CAIRN_DB_USER").unwrap_or_else(|_| "root".into()),
            db_pass: std::env::var("CAIRN_DB_PASS").unwrap_or_else(|_| "root".into()),
            db_ns: format!("test_{}", uuid_simple()),
            db_timeout_secs: 10,
            default_server: None,
            secret_key: None,
            tls: None,
            insecure: false,
            workspace_root: None,
            cors_origins: vec![],
            embed: EmbedConfig {
                provider: "hashing".into(),
                model: None,
                url: None,
                api_key: None,
            },
            llm_consolidation: cairn_core::LlmConsolidationConfig {
                enabled: false,
                url: "http://localhost:11434/v1/chat/completions".into(),
                model: "llama3.2".into(),
                api_key: None,
            },
            rerank: cairn_core::RerankConfig::default(),
            admin: cairn_core::AdminConfig::default(),
            multi_tenant: false,
        };
        Some(SurrealStore::connect(&cfg).expect("connect to live SurrealDB"))
    }

    #[test]
    #[ignore = "requires a live SurrealDB server (set CAIRN_DB_URL)"]
    fn meta_roundtrips() {
        let Some(be) = backend() else { return };
        let key = format!("cairn_test_meta_{}", uuid_simple());
        be.set_meta(&key, "hello-surreal").expect("set_meta");
        assert_eq!(
            be.get_meta(&key).expect("get_meta").as_deref(),
            Some("hello-surreal")
        );
        be.set_meta(&key, "updated").expect("set_meta 2");
        assert_eq!(
            be.get_meta(&key).expect("get_meta 2").as_deref(),
            Some("updated")
        );
    }

    #[test]
    #[ignore = "requires a live SurrealDB server (set CAIRN_DB_URL)"]
    fn tokens_roundtrip() {
        let Some(be) = backend() else { return };
        let before = be.count_tokens().expect("count");
        let tok = be
            .create_token("test-device", TokenScope::Write, None)
            .expect("create_token");
        assert!(be.validate_token_id(&tok.id).expect("validate"));
        assert!(be
            .list_tokens()
            .expect("list")
            .iter()
            .any(|t| t.id == tok.id && t.name == "test-device"));
        assert!(be.count_tokens().expect("count after") > before);

        assert!(
            be.revoke_token(&tok.id).expect("revoke"),
            "first revoke reports removed"
        );
        assert!(!be.validate_token_id(&tok.id).expect("validate after revoke"));
        assert!(
            !be.revoke_token(&tok.id).expect("revoke again"),
            "second revoke is a no-op"
        );
    }

    #[test]
    #[ignore = "requires a live SurrealDB server (set CAIRN_DB_URL)"]
    fn pairing_is_single_use() {
        let Some(be) = backend() else { return };
        let code = format!("pc-{}", uuid_simple());
        let future = ts(Utc::now() + chrono::Duration::minutes(10));
        be.create_pairing(&code, "tok-xyz", "new-device", &future)
            .expect("create_pairing");
        let now = ts(Utc::now());
        assert_eq!(
            be.claim_pairing(&code, &now).expect("claim"),
            Some(("tok-xyz".to_string(), "new-device".to_string()))
        );
        assert_eq!(be.claim_pairing(&code, &now).expect("claim again"), None);
    }

    #[test]
    #[ignore = "requires a live SurrealDB server (set CAIRN_DB_URL)"]
    fn expired_pairing_is_rejected() {
        let Some(be) = backend() else { return };
        let code = format!("pc-{}", uuid_simple());
        let past = ts(Utc::now() - chrono::Duration::minutes(1));
        be.create_pairing(&code, "tok-old", "old-device", &past)
            .expect("create_pairing");
        let now = ts(Utc::now());
        assert_eq!(be.claim_pairing(&code, &now).expect("claim expired"), None);
    }

    /// The full memory path through the public `Store` facade + `open_for_test` harness.
    #[test]
    #[ignore = "requires a live SurrealDB server (set CAIRN_DB_URL)"]
    fn memory_roundtrip_via_store() {
        let Some(store) = crate::Store::open_for_test() else {
            return;
        };
        let mut m = Memory {
            id: uuid_simple(),
            kind: MemoryKind::Decision,
            tier: MemoryTier::Working,
            content: "use surrealdb for the cairn vector store".into(),
            concepts: vec!["surrealdb".into(), "store".into()],
            files: vec![],
            session_id: None,
            importance: 0.7,
            access_count: 0,
            org_id: OrgId::default(),
            suspicious: false,
            confidence: 0.5,
            pinned: false,
            derived_from: vec![],
            contradicts: vec![],
            supersedes: vec![],
            applies_to: vec![],
            scope_type: ScopeType::Global,
            scope_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.insert_memory(&m).expect("insert");

        assert_eq!(store.count_memories().expect("count"), 1);
        let got = store.get_memory(&m.id).expect("get").expect("present");
        assert_eq!(got.content, m.content);
        assert_eq!(got.concepts, m.concepts);

        store.touch_memory(&m.id).expect("touch");
        assert_eq!(
            store.get_memory(&m.id).expect("get2").unwrap().access_count,
            1
        );

        let hits = store
            .semantic_recall("surrealdb vector store for cairn", 5)
            .expect("recall")
            .expect("backend has vectors");
        assert!(hits.iter().any(|x| x.id == m.id));

        m.updated_at = got.updated_at - chrono::Duration::minutes(5);
        assert!(!store.upsert_memory(&m).expect("stale upsert"));
        m.updated_at = Utc::now();
        m.content = "use surrealdb for cairn vectors and graph".into();
        assert!(store.upsert_memory(&m).expect("fresh upsert"));
        assert_eq!(store.count_memories().expect("count3"), 1);
        assert_eq!(
            store.get_memory(&m.id).expect("get3").unwrap().content,
            m.content
        );
    }
}
