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
