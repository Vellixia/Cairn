//! In-memory [`StoreBackend`](crate::db::StoreBackend) implementation.
//!
//! Used by the hermetic test bucket (`crates/cairn-tests/`) and any other
//! caller that wants a `Store` without standing up a database. The semantics
//! match the SurrealDB backend wherever it matters for engine correctness:
//!
//! - last-write-wins on `upsert_memory` (older `updated_at` is dropped).
//! - monotonic `AuditRecord` ids so SSE replay works the same.
//! - `set_meta_if_absent` honors the `__deleted__` tombstone.
//!
//! No vector index: `semantic_recall` returns `Ok(None)` so `MemoryEngine`
//! falls back to lexical ranking, identical to the offline behaviour of
//! the production server when `CAIRN_DB_URL` is unset.

use crate::db::{
    AuditRecord, DocumentChunkRecord, DocumentSummary, ProjectRecord, PromotionLogEntry,
    StoreBackend,
};
use crate::Store;
use cairn_core::{ContentHash, DeviceToken, Error, Memory, Result, TokenScope};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Mutex;

pub struct MemoryBackend {
    inner: Mutex<Inner>,
}

struct Inner {
    /// id -> Memory
    memories: HashMap<String, Memory>,
    /// content_hash -> memory id (dedup lookup)
    by_hash: HashMap<String, String>,
    /// id -> DeviceToken
    tokens: HashMap<String, DeviceToken>,
    /// server -> last_sync ts
    last_sync: HashMap<String, DateTime<Utc>>,
    /// path -> (content_hash, lines)
    file_versions: HashMap<String, (String, i64)>,
    /// key -> value (raw; tombs of "__deleted__" are visible to get_meta)
    meta: HashMap<String, String>,
    /// id -> (label, created_at, files_json)
    checkpoints: HashMap<String, (String, String, String)>,
    /// append-only audit (Vec, oldest first; ids are monotonic)
    audit: Vec<AuditRecord>,
    /// monotonically-increasing audit id counter (max ever assigned)
    next_audit_id: i64,
    /// (ts, kind, risk, path) append-only, newest first when read
    guard_events: Vec<(String, String, String, String)>,
    /// id -> ProjectRecord (v0.8.0 Sprint 3)
    projects: HashMap<String, ProjectRecord>,
    /// chunk id -> DocumentChunkRecord (v0.8.0 Sprint 6)
    document_chunks: HashMap<String, DocumentChunkRecord>,
    /// append-only promotion log, oldest first (v0.8.0 Sprint 8)
    promotion_log: Vec<PromotionLogEntry>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                memories: HashMap::new(),
                by_hash: HashMap::new(),
                tokens: HashMap::new(),
                last_sync: HashMap::new(),
                file_versions: HashMap::new(),
                meta: HashMap::new(),
                checkpoints: HashMap::new(),
                audit: Vec::new(),
                next_audit_id: 1,
                guard_events: Vec::new(),
                projects: HashMap::new(),
                document_chunks: HashMap::new(),
                promotion_log: Vec::new(),
            }),
        }
    }
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl StoreBackend for MemoryBackend {
    fn insert_memory(&self, m: &Memory) -> Result<()> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        let hash = ContentHash::of_str(&m.content);
        g.memories.insert(m.id.clone(), m.clone());
        g.by_hash.insert(hash.as_str().to_string(), m.id.clone());
        Ok(())
    }

    fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.memories.get(id).cloned())
    }

    fn find_memory_by_content_hash(&self, hash: &str) -> Result<Option<Memory>> {
        let g = self.inner.lock().map_err(poisoned)?;
        let Some(id) = g.by_hash.get(hash) else {
            return Ok(None);
        };
        Ok(g.memories.get(id).cloned())
    }

    fn all_memories(&self) -> Result<Vec<Memory>> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.memories.values().cloned().collect())
    }

    fn touch_memory(&self, id: &str) -> Result<()> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        if let Some(m) = g.memories.get_mut(id) {
            m.access_count += 1;
            m.updated_at = Utc::now();
        }
        Ok(())
    }

    fn count_memories(&self) -> Result<i64> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.memories.len() as i64)
    }

    fn upsert_memory(&self, m: &Memory) -> Result<bool> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        if let Some(existing) = g.memories.get(&m.id) {
            if m.updated_at < existing.updated_at {
                return Ok(false); // LWW: incoming is older
            }
            // Replace: drop the old hash index entry if the content changed.
            if existing.content != m.content {
                let old_hash = ContentHash::of_str(&existing.content);
                g.by_hash.remove(old_hash.as_str());
            }
        }
        let hash = ContentHash::of_str(&m.content);
        g.memories.insert(m.id.clone(), m.clone());
        g.by_hash.insert(hash.as_str().to_string(), m.id.clone());
        Ok(true)
    }

    fn memories_since(&self, since: DateTime<Utc>) -> Result<Vec<Memory>> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.memories
            .values()
            .filter(|m| m.updated_at > since)
            .cloned()
            .collect())
    }

    fn reinforce_memory(&self, id: &str) -> Result<()> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        if let Some(m) = g.memories.get_mut(id) {
            m.access_count += 1;
            m.confidence = (m.confidence + 0.1 * (1.0 - m.confidence)).clamp(0.0, 1.0);
            m.updated_at = Utc::now();
        }
        Ok(())
    }

    fn set_pinned(&self, id: &str, pinned: bool) -> Result<()> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        if let Some(m) = g.memories.get_mut(id) {
            m.pinned = pinned;
            m.updated_at = Utc::now();
        }
        Ok(())
    }

    fn reassign_scope(
        &self,
        id: &str,
        scope_type: cairn_core::ScopeType,
        scope_id: Option<String>,
    ) -> Result<bool> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        let Some(m) = g.memories.get_mut(id) else {
            return Ok(false);
        };
        m.scope_type = scope_type;
        m.scope_id = scope_id;
        m.updated_at = Utc::now();
        Ok(true)
    }

    fn edit_memory(
        &self,
        id: &str,
        content: Option<String>,
        importance: Option<f32>,
        concepts: Option<Vec<String>>,
        files: Option<Vec<String>>,
        title: Option<String>,
        reasoning: Option<String>,
    ) -> Result<bool> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        let Some(existing) = g.memories.get(id).cloned() else {
            return Ok(false);
        };
        let mut updated = existing.clone();
        if let Some(c) = content {
            if c != existing.content {
                let old_hash = ContentHash::of_str(&existing.content);
                g.by_hash.remove(old_hash.as_str());
            }
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
        if let Some(t) = title {
            updated.title = Some(t);
        }
        if let Some(r) = reasoning {
            updated.reasoning = Some(r);
        }
        updated.updated_at = Utc::now();
        let hash = ContentHash::of_str(&updated.content);
        g.memories.insert(id.to_string(), updated.clone());
        g.by_hash
            .insert(hash.as_str().to_string(), updated.id.clone());
        Ok(true)
    }

    fn delete_memory(&self, id: &str) -> Result<bool> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        let Some(existing) = g.memories.remove(id) else {
            return Ok(false);
        };
        let hash = ContentHash::of_str(&existing.content);
        g.by_hash.remove(hash.as_str());
        Ok(true)
    }

    fn semantic_recall(&self, _query: &str, _k: usize) -> Result<Option<Vec<Memory>>> {
        // No vector index in this backend.
        Ok(None)
    }

    fn create_token(
        &self,
        name: &str,
        scope: TokenScope,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<DeviceToken> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let token = DeviceToken {
            id: id.clone(),
            token: None, // caller mints the bearer outside
            name: name.to_string(),
            scope,
            expires_at,
            last_used_at: None,
            created_at: Utc::now(),
        };
        g.tokens.insert(id, token.clone());
        Ok(token)
    }

    fn validate_token_id(&self, token_id: &str) -> Result<bool> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.tokens.contains_key(token_id))
    }

    fn record_token_usage(&self, token_id: &str) -> Result<()> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        if let Some(t) = g.tokens.get_mut(token_id) {
            t.last_used_at = Some(Utc::now());
        }
        Ok(())
    }

    fn revoke_token(&self, token_id: &str) -> Result<bool> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        Ok(g.tokens.remove(token_id).is_some())
    }

    fn list_tokens(&self) -> Result<Vec<DeviceToken>> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.tokens.values().cloned().collect())
    }

    fn count_tokens(&self) -> Result<i64> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.tokens.len() as i64)
    }

    fn get_last_sync(&self, server: &str) -> Result<Option<DateTime<Utc>>> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.last_sync.get(server).copied())
    }

    fn set_last_sync(&self, server: &str, when: DateTime<Utc>) -> Result<()> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        g.last_sync.insert(server.to_string(), when);
        Ok(())
    }

    fn record_file_version(&self, path: &str, content_hash: &str, lines: i64) -> Result<()> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        g.file_versions
            .insert(path.to_string(), (content_hash.to_string(), lines));
        Ok(())
    }

    fn latest_file_version(&self, path: &str) -> Result<Option<(String, i64)>> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.file_versions.get(path).cloned())
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        g.meta.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.meta.get(key).cloned())
    }

    fn all_file_versions(&self) -> Result<Vec<(String, String, i64)>> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.file_versions
            .iter()
            .map(|(p, (h, l))| (p.clone(), h.clone(), *l))
            .collect())
    }

    fn insert_checkpoint(
        &self,
        id: &str,
        label: &str,
        created_at: &str,
        files: &str,
    ) -> Result<()> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        g.checkpoints.insert(
            id.to_string(),
            (label.to_string(), created_at.to_string(), files.to_string()),
        );
        Ok(())
    }

    fn get_checkpoint(&self, id: &str) -> Result<Option<(String, String, String)>> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.checkpoints.get(id).cloned())
    }

    fn list_checkpoints(&self) -> Result<Vec<(String, String, String)>> {
        let g = self.inner.lock().map_err(poisoned)?;
        // The real backend returns newest first; match that ordering.
        let mut v: Vec<_> = g
            .checkpoints
            .iter()
            .map(|(id, (l, t, _))| (id.clone(), l.clone(), t.clone()))
            .collect();
        v.sort_by(|a, b| b.2.cmp(&a.2));
        Ok(v)
    }

    fn record_guard_event(&self, ts: &str, kind: &str, risk: &str, path: &str) -> Result<()> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        g.guard_events.push((
            ts.to_string(),
            kind.to_string(),
            risk.to_string(),
            path.to_string(),
        ));
        Ok(())
    }

    fn recent_guard_events(&self, limit: usize) -> Result<Vec<(String, String, String, String)>> {
        let g = self.inner.lock().map_err(poisoned)?;
        // newest first
        let mut v = g.guard_events.clone();
        v.reverse();
        v.truncate(limit);
        Ok(v)
    }

    fn append_audit(&self, ts: i64, kind: &str, actor: &str, detail: &str) -> Result<String> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        let id = g.next_audit_id;
        g.next_audit_id += 1;
        g.audit.push(AuditRecord {
            id,
            ts,
            kind: kind.to_string(),
            actor: actor.to_string(),
            detail: detail.to_string(),
        });
        Ok(id.to_string())
    }

    fn recent_audit(&self, limit: usize, since_event_id: Option<&str>) -> Result<Vec<AuditRecord>> {
        let g = self.inner.lock().map_err(poisoned)?;
        // Newest first, then optionally drop everything <= since_event_id.
        let mut v = g.audit.clone();
        v.reverse();
        if let Some(since) = since_event_id {
            let since: i64 = since
                .parse()
                .map_err(|_| Error::Invalid(format!("since_event_id not an integer: {since}")))?;
            v.retain(|r| r.id > since);
        }
        v.truncate(limit);
        Ok(v)
    }

    fn max_audit_event_id(&self) -> Result<i64> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.next_audit_id - 1)
    }

    fn upsert_project(&self, id: &str, name: &str, path: &str) -> Result<()> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        let now = Utc::now();
        g.projects
            .entry(id.to_string())
            .and_modify(|p| {
                p.name = name.to_string();
                p.path = path.to_string();
                p.last_active = now;
            })
            .or_insert_with(|| ProjectRecord {
                id: id.to_string(),
                name: name.to_string(),
                path: path.to_string(),
                first_seen: now,
                last_active: now,
            });
        Ok(())
    }

    fn list_projects(&self) -> Result<Vec<ProjectRecord>> {
        let g = self.inner.lock().map_err(poisoned)?;
        let mut v: Vec<ProjectRecord> = g.projects.values().cloned().collect();
        v.sort_by_key(|p| std::cmp::Reverse(p.last_active));
        Ok(v)
    }

    fn get_project(&self, id: &str) -> Result<Option<ProjectRecord>> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.projects.get(id).cloned())
    }

    fn replace_document(
        &self,
        source: &str,
        title: &str,
        chunks: &[String],
        project_id: Option<&str>,
    ) -> Result<usize> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        g.document_chunks.retain(|_, c| c.source != source);
        let base_id = ContentHash::of_str(source).short().to_string();
        let now = Utc::now();
        for (i, content) in chunks.iter().enumerate() {
            let id = format!("{base_id}-{i}");
            g.document_chunks.insert(
                id.clone(),
                DocumentChunkRecord {
                    id,
                    source: source.to_string(),
                    title: title.to_string(),
                    chunk_index: i,
                    content: content.clone(),
                    project_id: project_id.map(str::to_string),
                    created_at: now,
                },
            );
        }
        Ok(chunks.len())
    }

    fn list_documents(&self, project_filter: Option<&str>) -> Result<Vec<DocumentSummary>> {
        let g = self.inner.lock().map_err(poisoned)?;
        let mut by_source: HashMap<String, DocumentSummary> = HashMap::new();
        for c in g.document_chunks.values() {
            if let Some(want) = project_filter {
                if c.project_id.as_deref() != Some(want) && c.project_id.is_some() {
                    continue;
                }
            }
            by_source
                .entry(c.source.clone())
                .and_modify(|d| {
                    d.chunk_count += 1;
                    if c.created_at > d.updated_at {
                        d.updated_at = c.created_at;
                    }
                })
                .or_insert_with(|| DocumentSummary {
                    id: ContentHash::of_str(&c.source).short().to_string(),
                    source: c.source.clone(),
                    title: c.title.clone(),
                    chunk_count: 1,
                    project_id: c.project_id.clone(),
                    updated_at: c.created_at,
                });
        }
        let mut out: Vec<DocumentSummary> = by_source.into_values().collect();
        out.sort_by_key(|d| std::cmp::Reverse(d.updated_at));
        Ok(out)
    }

    /// No vector index in the hermetic backend - falls back to a naive lexical match count
    /// (case-insensitive substring hits per query word), same "good enough offline" spirit as
    /// `semantic_recall` returning `None` for lexical-only fallback elsewhere.
    fn search_documents(
        &self,
        query: &str,
        k: usize,
        project_filter: Option<&str>,
    ) -> Result<Vec<DocumentChunkRecord>> {
        let g = self.inner.lock().map_err(poisoned)?;
        let terms: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut scored: Vec<(usize, &DocumentChunkRecord)> = g
            .document_chunks
            .values()
            .filter(|c| match project_filter {
                Some(want) => c.project_id.as_deref() == Some(want) || c.project_id.is_none(),
                None => true,
            })
            .filter_map(|c| {
                let lower = c.content.to_lowercase();
                let hits = terms.iter().filter(|t| lower.contains(t.as_str())).count();
                (hits > 0).then_some((hits, c))
            })
            .collect();
        scored.sort_by_key(|(hits, _)| std::cmp::Reverse(*hits));
        Ok(scored.into_iter().take(k).map(|(_, c)| c.clone()).collect())
    }

    fn delete_document(&self, source: &str) -> Result<bool> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        let before = g.document_chunks.len();
        g.document_chunks.retain(|_, c| c.source != source);
        Ok(g.document_chunks.len() < before)
    }

    fn record_promotion_event(&self, entry: &PromotionLogEntry) -> Result<()> {
        let mut g = self.inner.lock().map_err(poisoned)?;
        g.promotion_log.push(entry.clone());
        Ok(())
    }

    fn list_promotion_log(&self, limit: usize) -> Result<Vec<PromotionLogEntry>> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.promotion_log.iter().rev().take(limit).cloned().collect())
    }

    fn last_promotion_event(
        &self,
        memory_id: &str,
        action: &str,
    ) -> Result<Option<PromotionLogEntry>> {
        let g = self.inner.lock().map_err(poisoned)?;
        Ok(g.promotion_log
            .iter()
            .rev()
            .find(|e| e.memory_id == memory_id && e.action == action)
            .cloned())
    }

    // `count_any_access` keeps the trait's no-op default (`Ok(0)`) - same hermetic-backend
    // boundary as `count_cross_project_access`: the in-memory backend never stores access_log
    // rows in the first place.
}

fn poisoned(e: std::sync::PoisonError<std::sync::MutexGuard<'_, Inner>>) -> Error {
    Error::Other(format!("memory backend mutex poisoned: {e}"))
}

/// (Implementation detail — called by `Store::open_in_memory`.)
pub fn build(blobs_dir: std::path::PathBuf) -> Result<Store> {
    use crate::blob::BlobStore;
    let backend: Box<dyn StoreBackend> = Box::new(MemoryBackend::new());
    if let Some(parent) = blobs_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(&blobs_dir)?;
    Ok(Store {
        backend,
        blobs: BlobStore::new(blobs_dir),
    })
}
