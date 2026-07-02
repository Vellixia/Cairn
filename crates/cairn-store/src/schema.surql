-- Cairn's SurrealDB schema (v0.8.0 Sprint 1). Applied idempotently on every connect via
-- `SurrealStore::ensure_schema` - `IF NOT EXISTS` means a restart never rebuilds the HNSW index.
-- Tables are SCHEMALESS: Cairn owns validation in Rust, same trust boundary as the old HelixDB
-- property bags. `{dim}` is substituted at connect time with the active embedder's output width
-- (`Embedder::dim()`) before this file is sent to the server.

DEFINE TABLE IF NOT EXISTS memory SCHEMALESS;
DEFINE INDEX IF NOT EXISTS memory_content_hash ON memory FIELDS content_hash;
DEFINE INDEX IF NOT EXISTS memory_session_id ON memory FIELDS session_id;
DEFINE INDEX IF NOT EXISTS memory_embedding_hnsw ON memory FIELDS embedding
    HNSW DIMENSION {dim} DIST COSINE TYPE F32;

DEFINE TABLE IF NOT EXISTS token SCHEMALESS;

DEFINE TABLE IF NOT EXISTS sync_state SCHEMALESS;

DEFINE TABLE IF NOT EXISTS file_version SCHEMALESS;
DEFINE INDEX IF NOT EXISTS file_version_path ON file_version FIELDS path;

DEFINE TABLE IF NOT EXISTS meta SCHEMALESS;

DEFINE TABLE IF NOT EXISTS checkpoint SCHEMALESS;

DEFINE TABLE IF NOT EXISTS guard_event SCHEMALESS;

DEFINE TABLE IF NOT EXISTS pairing SCHEMALESS;

DEFINE TABLE IF NOT EXISTS audit_event SCHEMALESS;
DEFINE INDEX IF NOT EXISTS audit_event_seq ON audit_event FIELDS seq;

DEFINE TABLE IF NOT EXISTS audit_counter SCHEMALESS;

-- v0.8.0 Sprint 2: recall access log. Written as one batched multi-row INSERT per recall
-- call (never one row-per-memory query). Pruned by the S4 access-log-prune cron job
-- (CAIRN_ACCESS_LOG_RETENTION_DAYS, default 90) so unbounded write volume never grows the
-- table without bound.
DEFINE TABLE IF NOT EXISTS access_log SCHEMALESS;
DEFINE INDEX IF NOT EXISTS access_log_ts ON access_log FIELDS ts;

-- v0.8.0 Sprint 3: registered projects, auto-detected by the SessionStart hook.
DEFINE TABLE IF NOT EXISTS project SCHEMALESS;
DEFINE INDEX IF NOT EXISTS project_last_active ON project FIELDS last_active;

-- v0.8.0 Sprint 6: RAG document chunks (`cairn documents ingest <file|url>`). `source` is the
-- literal path/URL the caller gave; re-ingesting the same source replaces its chunks (delete +
-- reinsert), same idempotent-upsert idea as `project`'s `first_seen ?? $now`. Embedded and
-- HNSW-indexed exactly like `memory` so document search reuses the same vector infrastructure
-- instead of a second one.
DEFINE TABLE IF NOT EXISTS document_chunk SCHEMALESS;
DEFINE INDEX IF NOT EXISTS document_chunk_source ON document_chunk FIELDS source;
DEFINE INDEX IF NOT EXISTS document_chunk_embedding_hnsw ON document_chunk FIELDS embedding
    HNSW DIMENSION {dim} DIST COSINE TYPE F32;
