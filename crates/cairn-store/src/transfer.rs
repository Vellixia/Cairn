//! Versioned, integrity-checked portable-store transfer.
use crate::{diag, migrate, Result, Store, StoreError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Acquire, Row};
use std::collections::BTreeMap;
use std::io::Write;
use std::io::{Read, Seek};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path};

pub const MANIFEST_VERSION: u32 = 1;
/// Schema for the one-way, offline export of the removed Task feature.
pub const REMOVED_FEATURE_BUNDLE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemovedFeatureBundle {
    pub version: u32,
    pub feature: String,
    pub exported_at: String,
    pub records: Vec<RemovedFeatureRecord>,
    /// All disposition keys are present so a conservation report is explicit
    /// before any later import chooses accepted or rejected records.
    pub dispositions: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemovedFeatureRecord {
    pub source_table: String,
    pub source_id: String,
    pub disposition: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationManifest {
    pub version: u32,
    pub schema_version: i64,
    pub exported_at: String,
    /// Exactly one relative filename; manifest/payload move together.
    pub snapshot: String,
    pub snapshot_size: u64,
    pub snapshot_sha256: String,
    /// Exhaustive `diag` durability categories, table-by-table, from snapshot.
    pub lanes: BTreeMap<String, BTreeMap<String, u64>>,
    pub tombstones: BTreeMap<String, u64>,
    pub local_only: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportReport {
    pub accepted: u64,
    pub rejected: u64,
    pub retained: u64,
    pub checkpoint: String,
}

fn refused(code: &'static str, message: impl Into<String>) -> StoreError {
    StoreError::Refused {
        code,
        message: message.into(),
    }
}
fn qi(v: &str) -> String {
    format!("\"{}\"", v.replace('"', "\"\""))
}
fn ql(v: &str) -> String {
    format!("'{}'", v.replace('\'', "''"))
}
fn relative_filename(v: &str) -> Result<()> {
    let p = Path::new(v);
    if p.components().count() != 1 || !matches!(p.components().next(), Some(Component::Normal(_))) {
        return Err(refused(
            "unsafe_snapshot_path",
            "snapshot must be one relative filename",
        ));
    }
    Ok(())
}
fn digest(path: &Path) -> Result<(u64, String)> {
    let m = std::fs::symlink_metadata(path)?;
    if m.file_type().is_symlink() || !m.is_file() {
        return Err(refused(
            "unsafe_snapshot_path",
            "snapshot must be a regular file",
        ));
    }
    Ok((
        m.len(),
        format!("{:x}", Sha256::digest(std::fs::read(path)?)),
    ))
}

/// Read one stable, no-follow artifact then make a private immutable input.
/// Import never attaches caller-controlled bytes after this boundary.
struct PrivateArtifact(std::path::PathBuf);
impl PrivateArtifact {
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for PrivateArtifact {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn verified_private_copy(path: &Path) -> Result<(PrivateArtifact, u64, String)> {
    let before = std::fs::symlink_metadata(path)?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(refused(
            "unsafe_snapshot_path",
            "snapshot must be a regular file",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut input = options.open(path)?;
    let opened = input.metadata()?;
    #[cfg(windows)]
    if opened.file_type().is_symlink() {
        return Err(refused(
            "unsafe_snapshot_path",
            "snapshot reparse point is refused",
        ));
    }
    if opened.len() != before.len() {
        return Err(refused(
            "artifact_changed",
            "snapshot changed while opening",
        ));
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    input.read_to_end(&mut bytes)?;
    input.seek(std::io::SeekFrom::Start(0))?;
    if input.metadata()?.len() != opened.len() {
        return Err(refused(
            "artifact_changed",
            "snapshot changed while reading",
        ));
    }
    let private =
        std::env::temp_dir().join(format!("cairn-import-{}.sqlite", uuid::Uuid::now_v7()));
    let guard = PrivateArtifact(private.clone());
    let mut output_options = std::fs::OpenOptions::new();
    output_options.write(true).create_new(true);
    #[cfg(unix)]
    output_options.mode(0o600);
    let mut output = output_options.open(&private)?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    Ok((
        guard,
        bytes.len() as u64,
        format!("{:x}", Sha256::digest(bytes)),
    ))
}

async fn inventory(path: &Path) -> Result<BTreeMap<String, BTreeMap<String, u64>>> {
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", path.display())).await?;
    let mut lanes = BTreeMap::new();
    for c in diag::CATEGORIES {
        let mut tables = BTreeMap::new();
        for table in c.tables {
            if sqlx::query_scalar::<_, String>(
                "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_optional(&pool)
            .await?
            .is_some()
            {
                let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {}", qi(table)))
                    .fetch_one(&pool)
                    .await?;
                tables.insert((*table).to_string(), n as u64);
            }
        }
        lanes.insert(c.category.to_string(), tables);
    }
    pool.close().await;
    Ok(lanes)
}

async fn subset_counts(path: &Path, queries: &[(&str, &str)]) -> Result<BTreeMap<String, u64>> {
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", path.display())).await?;
    let mut counts = BTreeMap::new();
    for (name, query) in queries {
        counts.insert(
            (*name).to_string(),
            sqlx::query_scalar::<_, i64>(query).fetch_one(&pool).await? as u64,
        );
    }
    pool.close().await;
    Ok(counts)
}

async fn manifest_summaries(path: &Path) -> Result<(BTreeMap<String, u64>, BTreeMap<String, u64>)> {
    let tombstones = subset_counts(
        path,
        &[
            (
                "memories",
                "SELECT count(*) FROM memories WHERE deleted_at IS NOT NULL",
            ),
            (
                "memory_relations",
                "SELECT count(*) FROM memory_relations WHERE deleted_at IS NOT NULL",
            ),
            (
                "tasks",
                "SELECT count(*) FROM tasks WHERE deleted_at IS NOT NULL",
            ),
            (
                "sessions",
                "SELECT count(*) FROM sessions WHERE deleted_at IS NOT NULL",
            ),
            (
                "observations",
                "SELECT count(*) FROM observations WHERE deleted_at IS NOT NULL",
            ),
            (
                "handoffs",
                "SELECT count(*) FROM handoffs WHERE deleted_at IS NOT NULL",
            ),
            (
                "evidence_facts",
                "SELECT count(*) FROM evidence_facts WHERE deleted_at IS NOT NULL",
            ),
            (
                "continuity_checkpoints",
                "SELECT count(*) FROM continuity_checkpoints WHERE deleted_at IS NOT NULL",
            ),
            (
                "reusable_patterns",
                "SELECT count(*) FROM reusable_patterns WHERE deleted_at IS NOT NULL",
            ),
            (
                "personal_knowledge",
                "SELECT count(*) FROM personal_knowledge WHERE forgotten_at IS NOT NULL",
            ),
            (
                "team_knowledge",
                "SELECT count(*) FROM team_knowledge WHERE state IN ('retired', 'superseded')",
            ),
            (
                "cached_patterns",
                "SELECT count(*) FROM cached_patterns WHERE forgotten_at IS NOT NULL",
            ),
        ],
    )
    .await?;
    let local = subset_counts(
        path,
        &[(
            "memories",
            "SELECT count(*) FROM memories WHERE local_only = 1",
        )],
    )
    .await?;
    Ok((tombstones, local))
}

async fn removed_feature_rows(
    store: &Store,
    table: &str,
    predicate: &str,
    source_id: &str,
) -> Result<Vec<RemovedFeatureRecord>> {
    let columns: Vec<String> = sqlx::query(&format!("PRAGMA table_info({})", ql(table)))
        .fetch_all(store.pool())
        .await?
        .into_iter()
        .map(|row| row.try_get("name"))
        .collect::<std::result::Result<_, _>>()?;
    let json_args = columns
        .iter()
        .flat_map(|column| [ql(column), qi(column)])
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {source_id} AS source_id, json_object({json_args}) AS payload FROM {} WHERE {predicate}",
        qi(table)
    );
    sqlx::query(&sql)
        .fetch_all(store.pool())
        .await?
        .into_iter()
        .map(|row| {
            let payload: String = row.try_get("payload")?;
            Ok(RemovedFeatureRecord {
                source_table: table.to_owned(),
                source_id: row.try_get("source_id")?,
                disposition: "retained".to_owned(),
                payload: serde_json::from_str(&payload)
                    .map_err(|error| StoreError::Corrupt(error.to_string()))?,
            })
        })
        .collect()
}

/// Rows whose values cleanup removes, or whose table/column cleanup drops.
/// Export and cleanup share this inventory so no new deletion path can silently
/// escape the offline artifact.
const REMOVED_FEATURE_TASK_SELECTIONS: &[(&str, &str, &str)] = &[
    ("tasks", "1 = 1", "id"),
    ("sessions", "task_id IS NOT NULL", "id"),
    ("memories", "scope = 'task'", "id"),
    (
        "memory_evidence",
        "memory_id IN (SELECT id FROM memories WHERE scope = 'task')",
        "memory_id || ':' || observation_id",
    ),
    (
        "memory_evidence_facts",
        "memory_id IN (SELECT id FROM memories WHERE scope = 'task')",
        "memory_id || ':' || evidence_id || ':' || role",
    ),
    (
        "evidence_facts",
        "id IN (
            SELECT evidence_id FROM memory_evidence_facts
             WHERE memory_id IN (SELECT id FROM memories WHERE scope = 'task')
            UNION
            SELECT evidence_id FROM criterion_evidence
             WHERE criterion_id IN (SELECT id FROM task_criteria)
            UNION
            SELECT evidence_id FROM verification_runs
             WHERE (criterion_id IN (SELECT id FROM task_criteria)
                 OR memory_id IN (SELECT id FROM memories WHERE scope = 'task'))
               AND evidence_id IS NOT NULL
        )",
        "id",
    ),
    (
        "memory_relations",
        "from_memory_id IN (SELECT id FROM memories WHERE scope = 'task') OR to_memory_id IN (SELECT id FROM memories WHERE scope = 'task')",
        "from_memory_id || ':' || to_memory_id || ':' || kind",
    ),
    ("task_criteria", "1 = 1", "id"),
    ("task_blockers", "1 = 1", "id"),
    ("task_changes", "1 = 1", "id"),
    (
        "criterion_evidence",
        "criterion_id IN (SELECT id FROM task_criteria)",
        "criterion_id || ':' || evidence_id",
    ),
    (
        "verification_runs",
        "criterion_id IN (SELECT id FROM task_criteria) OR memory_id IN (SELECT id FROM memories WHERE scope = 'task')",
        "id",
    ),
    (
        "continuity_checkpoints",
        "assumed_task_id IS NOT NULL",
        "id",
    ),
    (
        "outbox",
        "entity_type IN ('task', 'task_criterion', 'task_blocker')",
        "id",
    ),
];

async fn removed_feature_task_records(store: &Store) -> Result<Vec<RemovedFeatureRecord>> {
    let mut records = Vec::new();
    for (table, predicate, source_id) in REMOVED_FEATURE_TASK_SELECTIONS {
        records.extend(removed_feature_rows(store, table, predicate, source_id).await?);
    }
    Ok(records)
}

/// Write a versioned, offline-only bundle before any operator considers
/// deleting legacy Task data. This never changes source rows; a failed export
/// therefore leaves capture and its local spool available.
pub async fn export_removed_feature_tasks(
    store: &Store,
    path: &Path,
) -> Result<RemovedFeatureBundle> {
    let state: Option<String> = sqlx::query_scalar(
        "SELECT disposition FROM removed_feature_manifest WHERE feature = 'tasks'",
    )
    .fetch_optional(store.pool())
    .await?;
    if state.as_deref() != Some("retained_pending_export") {
        return Err(refused(
            "removed_feature_export_not_pending",
            "Task export is already recorded",
        ));
    }
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(refused(
            "removed_feature_bundle_exists",
            format!("refusing to overwrite {}", path.display()),
        ));
    }
    let records = removed_feature_task_records(store).await?;
    let dispositions = BTreeMap::from([
        ("accepted".to_owned(), 0),
        ("rejected".to_owned(), 0),
        ("retained".to_owned(), records.len() as u64),
    ]);
    // Retaining the explicit zero avoids an ambiguous report while the bundle
    // remains offline, before a server import can decide accepted/rejected.
    let bundle = RemovedFeatureBundle {
        version: REMOVED_FEATURE_BUNDLE_VERSION,
        feature: "tasks".to_owned(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        records,
        dispositions,
    };
    let bytes = serde_json::to_vec_pretty(&bundle)
        .map_err(|error| StoreError::Corrupt(error.to_string()))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut output = options.open(path)?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    let (_, artifact_sha256) = digest(path)?;
    sqlx::query("UPDATE removed_feature_manifest SET disposition = 'exported_pending_cleanup', artifact_path = ?, artifact_sha256 = ? WHERE feature = 'tasks'")
        .bind(path.to_string_lossy().as_ref())
        .bind(artifact_sha256)
        .execute(store.pool())
        .await?;
    Ok(bundle)
}

/// Whether Task export or its resumable cleanup remains outstanding.
pub async fn removed_feature_tasks_pending(store: &Store) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT disposition FROM removed_feature_manifest WHERE feature = 'tasks'",
    )
    .fetch_optional(store.pool())
    .await?
    .as_deref()
        != Some("exported_cleaned"))
}

/// Artifact paths are manifest-owned, never reconstructed by a retrying caller.
pub async fn removed_feature_tasks_exported_pending_cleanup(
    store: &Store,
) -> Result<Option<(String, String)>> {
    sqlx::query_as(
        "SELECT artifact_path, artifact_sha256 FROM removed_feature_manifest WHERE feature = 'tasks' AND disposition = 'exported_pending_cleanup'",
    )
    .fetch_optional(store.pool())
    .await
    .map_err(Into::into)
}

/// Remove legacy Task schema only after its immutable external bundle exists.
/// This is deliberately an explicit setup action, never an open-store action.
pub async fn cleanup_removed_feature_tasks(store: &Store) -> Result<()> {
    let artifact: Option<(String, String)> = sqlx::query_as(
        "SELECT artifact_path, artifact_sha256 FROM removed_feature_manifest WHERE feature = 'tasks' AND disposition = 'exported_pending_cleanup'",
    )
    .fetch_optional(store.pool())
    .await?;
    let (path, expected_hash) = artifact.ok_or_else(|| {
        refused(
            "removed_feature_cleanup_not_pending",
            "Task bundle export must finish before cleanup",
        )
    })?;
    let path = Path::new(&path);
    let (_, actual_hash) = digest(path)?;
    if actual_hash != expected_hash {
        return Err(refused(
            "removed_feature_artifact_changed",
            "Task bundle hash no longer matches manifest",
        ));
    }
    let bundle: RemovedFeatureBundle = serde_json::from_slice(&std::fs::read(path)?)
        .map_err(|e| StoreError::Corrupt(e.to_string()))?;
    if bundle.version != REMOVED_FEATURE_BUNDLE_VERSION || bundle.feature != "tasks" {
        return Err(refused(
            "removed_feature_artifact_invalid",
            "Task bundle is not a supported removed-feature artifact",
        ));
    }
    let expected = removed_feature_task_records(store).await?;
    // Identity alone cannot conserve a changed row. Compare the complete
    // canonical record set immediately before destructive work; inserts,
    // deletes, payload updates, and duplicate artifact entries all refuse.
    let keyed = |records: &[RemovedFeatureRecord]| {
        records
            .iter()
            .map(|record| {
                (
                    (record.source_table.clone(), record.source_id.clone()),
                    (record.disposition.clone(), record.payload.clone()),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let expected_keyed = keyed(&expected);
    let exported_keyed = keyed(&bundle.records);
    if expected.len() != expected_keyed.len()
        || bundle.records.len() != exported_keyed.len()
        || expected_keyed != exported_keyed
    {
        return Err(refused(
            "removed_feature_bundle_incomplete",
            "Task bundle does not conserve every row cleanup deletes",
        ));
    }
    let mut tx = store.pool().begin().await?;
    for statement in [
        "CREATE TEMP TABLE task_only_evidence_facts AS SELECT evidence_id FROM memory_evidence_facts WHERE memory_id IN (SELECT id FROM memories WHERE scope = 'task') UNION SELECT evidence_id FROM criterion_evidence WHERE criterion_id IN (SELECT id FROM task_criteria) UNION SELECT evidence_id FROM verification_runs WHERE (criterion_id IN (SELECT id FROM task_criteria) OR memory_id IN (SELECT id FROM memories WHERE scope = 'task')) AND evidence_id IS NOT NULL",
        "DELETE FROM verification_runs WHERE criterion_id IN (SELECT id FROM task_criteria) OR memory_id IN (SELECT id FROM memories WHERE scope = 'task')",
        "DELETE FROM criterion_evidence",
        "DELETE FROM memory_evidence WHERE memory_id IN (SELECT id FROM memories WHERE scope = 'task')",
        "DELETE FROM memory_evidence_facts WHERE memory_id IN (SELECT id FROM memories WHERE scope = 'task')",
        "DELETE FROM memory_relations WHERE from_memory_id IN (SELECT id FROM memories WHERE scope = 'task') OR to_memory_id IN (SELECT id FROM memories WHERE scope = 'task')",
        "DELETE FROM evidence_facts WHERE id IN (SELECT evidence_id FROM task_only_evidence_facts) AND NOT EXISTS (SELECT 1 FROM memory_evidence_facts WHERE evidence_id = evidence_facts.id) AND NOT EXISTS (SELECT 1 FROM verification_runs WHERE evidence_id = evidence_facts.id) AND NOT EXISTS (SELECT 1 FROM memory_relations WHERE basis_evidence_id = evidence_facts.id) AND NOT EXISTS (SELECT 1 FROM pattern_applications WHERE evidence_id = evidence_facts.id)",
        "DROP TABLE task_only_evidence_facts",
        "DELETE FROM outbox WHERE entity_type IN ('task', 'task_criterion', 'task_blocker')",
        "DELETE FROM memories WHERE scope = 'task'",
        "DROP TABLE IF EXISTS criterion_evidence",
        "DROP TABLE IF EXISTS task_changes",
        "DROP TABLE IF EXISTS task_blockers",
        "DROP TABLE IF EXISTS task_criteria",
        "DROP INDEX IF EXISTS sessions_task_recent",
        "ALTER TABLE sessions DROP COLUMN task_id",
        "ALTER TABLE sessions DROP COLUMN task_snapshot_at_bind",
        "ALTER TABLE continuity_checkpoints DROP COLUMN assumed_task_id",
        "ALTER TABLE continuity_checkpoints DROP COLUMN assumed_task_state_digest",
        "ALTER TABLE continuity_checkpoints DROP COLUMN criteria_snapshot",
        "ALTER TABLE continuity_checkpoints DROP COLUMN open_blockers",
        "DROP TABLE IF EXISTS tasks",
        "DROP INDEX IF EXISTS verification_runs_criterion",
        "CREATE TABLE verification_runs_new (id TEXT PRIMARY KEY, memory_id TEXT NOT NULL, project_id TEXT NOT NULL REFERENCES projects(id), verifier TEXT NOT NULL CHECK (verifier IN ('file_exists', 'file_digest', 'git_ref', 'git_commit', 'configuration', 'schema_version', 'test_outcome', 'command_outcome', 'runtime_state')), evidence_id TEXT, expected_digest TEXT, observed_digest TEXT, result TEXT NOT NULL CHECK (result IN ('verified', 'drifted', 'inconclusive')), detail TEXT, repo_branch TEXT NOT NULL, repo_commit TEXT, checked_at TEXT NOT NULL, triggered_by TEXT NOT NULL CHECK (triggered_by IN ('background_pass', 'on_demand', 'attach')))",
        "INSERT INTO verification_runs_new (id, memory_id, project_id, verifier, evidence_id, expected_digest, observed_digest, result, detail, repo_branch, repo_commit, checked_at, triggered_by) SELECT id, memory_id, project_id, verifier, evidence_id, expected_digest, observed_digest, result, detail, repo_branch, repo_commit, checked_at, triggered_by FROM verification_runs",
        "DROP TABLE verification_runs",
        "ALTER TABLE verification_runs_new RENAME TO verification_runs",
        "CREATE INDEX verification_runs_memory ON verification_runs (memory_id, checked_at DESC)",
        "CREATE INDEX verification_runs_result ON verification_runs (project_id, result)",
        "CREATE TABLE memory_evidence_staged (memory_id TEXT NOT NULL, observation_id TEXT NOT NULL, content_digest TEXT NOT NULL, PRIMARY KEY (memory_id, observation_id))",
        "INSERT INTO memory_evidence_staged SELECT * FROM memory_evidence",
        "DROP TABLE memory_evidence",
        "CREATE TABLE memory_evidence_facts_staged (memory_id TEXT NOT NULL, evidence_id TEXT NOT NULL, role TEXT NOT NULL CHECK (role IN ('supports', 'contradicts')), attached_at TEXT NOT NULL, attached_by_session TEXT NOT NULL, PRIMARY KEY (memory_id, evidence_id, role))",
        "INSERT INTO memory_evidence_facts_staged SELECT * FROM memory_evidence_facts",
        "DROP TABLE memory_evidence_facts",
        "CREATE TABLE memories_new (id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id), type TEXT NOT NULL CHECK (type IN ('fact', 'decision', 'convention', 'failure', 'procedure')), scope TEXT NOT NULL CHECK (scope IN ('project', 'branch', 'session')), scope_key TEXT NOT NULL, content TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'stale', 'superseded')), superseded_by_id TEXT REFERENCES memories(id), origin_session_id TEXT NOT NULL, local_only INTEGER NOT NULL DEFAULT 0 CHECK (local_only IN (0, 1)), created_at TEXT NOT NULL, updated_at TEXT NOT NULL, deleted_at TEXT, topic_key TEXT, value_key TEXT, content_norm_digest TEXT, importance TEXT NOT NULL DEFAULT 'normal', verification TEXT NOT NULL DEFAULT 'unverified', verification_authority TEXT, last_verified_at TEXT, effective_from TEXT, superseded_at TEXT, stale_at TEXT, pinned INTEGER NOT NULL DEFAULT 0, pinned_at TEXT, pinned_by_session TEXT, pin_reason TEXT, reinforcement_count INTEGER NOT NULL DEFAULT 0, distinct_origin_count INTEGER NOT NULL DEFAULT 1)",
        "INSERT INTO memories_new SELECT * FROM memories",
        "DROP TABLE memories",
        "ALTER TABLE memories_new RENAME TO memories",
        "CREATE INDEX memories_scope ON memories (project_id, scope, scope_key, state)",
        "CREATE INDEX memories_topic ON memories (project_id, topic_key, state) WHERE topic_key IS NOT NULL",
        "CREATE INDEX memories_subject ON memories (project_id, scope, scope_key, topic_key) WHERE topic_key IS NOT NULL",
        "CREATE INDEX memories_verification ON memories (project_id, verification) WHERE verification <> 'unverified'",
        "CREATE INDEX memories_pinned ON memories (project_id, scope, scope_key) WHERE pinned = 1",
        "CREATE INDEX memories_temporal ON memories (project_id, effective_from, superseded_at)",
        "CREATE INDEX memories_content_norm ON memories (project_id, content_norm_digest) WHERE content_norm_digest IS NOT NULL",
        "CREATE TRIGGER memories_fts_ai AFTER INSERT ON memories BEGIN INSERT INTO memory_fts(rowid, content) VALUES (new.rowid, new.content); END",
        "CREATE TRIGGER memories_fts_ad AFTER DELETE ON memories BEGIN INSERT INTO memory_fts(memory_fts, rowid, content) VALUES ('delete', old.rowid, old.content); END",
        "CREATE TRIGGER memories_fts_au AFTER UPDATE ON memories BEGIN INSERT INTO memory_fts(memory_fts, rowid, content) VALUES ('delete', old.rowid, old.content); INSERT INTO memory_fts(rowid, content) VALUES (new.rowid, new.content); END",
        "INSERT INTO memory_fts(memory_fts) VALUES ('rebuild')",
        "CREATE TABLE memory_evidence (memory_id TEXT NOT NULL REFERENCES memories(id), observation_id TEXT NOT NULL, content_digest TEXT NOT NULL, PRIMARY KEY (memory_id, observation_id))",
        "INSERT INTO memory_evidence SELECT * FROM memory_evidence_staged",
        "DROP TABLE memory_evidence_staged",
        "CREATE TABLE memory_evidence_facts (memory_id TEXT NOT NULL REFERENCES memories(id), evidence_id TEXT NOT NULL, role TEXT NOT NULL CHECK (role IN ('supports', 'contradicts')), attached_at TEXT NOT NULL, attached_by_session TEXT NOT NULL, PRIMARY KEY (memory_id, evidence_id, role))",
        "INSERT INTO memory_evidence_facts SELECT * FROM memory_evidence_facts_staged",
        "DROP TABLE memory_evidence_facts_staged",
        "CREATE INDEX memory_evidence_facts_evidence ON memory_evidence_facts (evidence_id)",
        "CREATE TABLE outbox_new (id TEXT PRIMARY KEY, project_id TEXT REFERENCES projects(id), server_project_id TEXT, entity_type TEXT NOT NULL CHECK (entity_type IN ('project', 'session', 'memory', 'handoff', 'memory_relation', 'personal_knowledge', 'personal_knowledge_relation', 'team_knowledge', 'team_knowledge_relation')), entity_id TEXT NOT NULL, operation TEXT NOT NULL CHECK (operation IN ('upsert', 'delete')), idempotency_key TEXT NOT NULL UNIQUE, payload TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'in_flight', 'delivered', 'failed', 'blocked')), attempts INTEGER NOT NULL DEFAULT 0, last_error TEXT, created_at TEXT NOT NULL, delivered_at TEXT, claimed_at TEXT, blocked_reason TEXT, blocked_at_capability TEXT, namespace TEXT NOT NULL, authored_by_user_id TEXT, CHECK ((entity_type IN ('personal_knowledge', 'personal_knowledge_relation', 'team_knowledge', 'team_knowledge_relation')) = (project_id IS NULL)), CHECK ((entity_type IN ('personal_knowledge', 'personal_knowledge_relation', 'team_knowledge', 'team_knowledge_relation')) = (authored_by_user_id IS NOT NULL)))",
        "INSERT INTO outbox_new SELECT * FROM outbox",
        "DROP TABLE outbox",
        "ALTER TABLE outbox_new RENAME TO outbox",
        "CREATE INDEX outbox_pending ON outbox (state, created_at)",
        "CREATE INDEX outbox_claimable ON outbox (namespace, state, created_at)",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE removed_feature_manifest SET disposition = 'exported_cleaned' WHERE feature = 'tasks'")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn export_snapshot(store: &Store, snapshot: &Path) -> Result<MigrationManifest> {
    if std::fs::symlink_metadata(snapshot).is_ok() {
        return Err(refused(
            "snapshot_exists",
            format!("refusing to overwrite {}", snapshot.display()),
        ));
    }
    let name = snapshot
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| refused("unsafe_snapshot_path", "snapshot needs UTF-8 filename"))?
        .to_string();
    relative_filename(&name)?;
    store.checkpoint().await?;
    sqlx::query(&format!("VACUUM INTO {}", ql(&snapshot.to_string_lossy())))
        .execute(store.pool())
        .await?;
    let (snapshot_size, snapshot_sha256) = digest(snapshot)?;
    let tombstones = subset_counts(
        snapshot,
        &[
            (
                "memories",
                "SELECT count(*) FROM memories WHERE deleted_at IS NOT NULL",
            ),
            (
                "memory_relations",
                "SELECT count(*) FROM memory_relations WHERE deleted_at IS NOT NULL",
            ),
            (
                "tasks",
                "SELECT count(*) FROM tasks WHERE deleted_at IS NOT NULL",
            ),
            (
                "sessions",
                "SELECT count(*) FROM sessions WHERE deleted_at IS NOT NULL",
            ),
            (
                "observations",
                "SELECT count(*) FROM observations WHERE deleted_at IS NOT NULL",
            ),
            (
                "handoffs",
                "SELECT count(*) FROM handoffs WHERE deleted_at IS NOT NULL",
            ),
            (
                "evidence_facts",
                "SELECT count(*) FROM evidence_facts WHERE deleted_at IS NOT NULL",
            ),
            (
                "continuity_checkpoints",
                "SELECT count(*) FROM continuity_checkpoints WHERE deleted_at IS NOT NULL",
            ),
            (
                "reusable_patterns",
                "SELECT count(*) FROM reusable_patterns WHERE deleted_at IS NOT NULL",
            ),
            (
                "personal_knowledge",
                "SELECT count(*) FROM personal_knowledge WHERE forgotten_at IS NOT NULL",
            ),
            (
                "team_knowledge",
                "SELECT count(*) FROM team_knowledge WHERE state IN ('retired', 'superseded')",
            ),
            (
                "cached_patterns",
                "SELECT count(*) FROM cached_patterns WHERE forgotten_at IS NOT NULL",
            ),
        ],
    )
    .await?;
    let local_only = subset_counts(
        snapshot,
        &[(
            "memories",
            "SELECT count(*) FROM memories WHERE local_only = 1",
        )],
    )
    .await?;
    Ok(MigrationManifest {
        version: MANIFEST_VERSION,
        schema_version: migrate::latest_version(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        snapshot: name,
        snapshot_size,
        snapshot_sha256,
        lanes: inventory(snapshot).await?,
        tombstones,
        local_only,
    })
}
pub fn write_manifest(m: &MigrationManifest, path: &Path) -> Result<()> {
    relative_filename(&m.snapshot)?;
    let body = serde_json::to_vec_pretty(m)
        .map_err(|e| StoreError::Corrupt(format!("manifest serialization: {e}")))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                refused(
                    "manifest_exists",
                    format!("refusing to overwrite {}", path.display()),
                )
            } else {
                StoreError::Io(e)
            }
        })?;
    file.write_all(&body)?;
    file.sync_all()?;
    Ok(())
}
pub fn read_manifest(path: &Path) -> Result<MigrationManifest> {
    let before = std::fs::symlink_metadata(path)?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(refused(
            "unsafe_manifest_path",
            "manifest must be a regular file",
        ));
    }
    let m: MigrationManifest = serde_json::from_slice(&std::fs::read(path)?)
        .map_err(|e| StoreError::Corrupt(format!("manifest parse: {e}")))?;
    let after = std::fs::symlink_metadata(path)?;
    if before.len() != after.len() || after.file_type().is_symlink() {
        return Err(refused(
            "artifact_changed",
            "manifest changed while being read",
        ));
    }
    relative_filename(&m.snapshot)?;
    Ok(m)
}
struct Plan {
    table: String,
    fields: String,
    rows: u64,
}

/// Refuse a same-stable-ID row unless every stored field agrees. `INSERT OR
/// IGNORE` alone silently keeps the target row, which can make newly imported
/// dependents point at a different canonical record.
async fn reject_divergent_stable_ids(
    c: &mut sqlx::SqliteConnection,
    table: &str,
    cols: &[String],
) -> Result<()> {
    // This is the destination machine's singleton identity, not a portable
    // canonical record. Its fixed `id = 1` must remain destination-local.
    if matches!(
        table,
        "writer_identity" | "authority_mode" | "removed_feature_manifest"
    ) {
        return Ok(());
    }
    let info = sqlx::query(&format!("PRAGMA main.table_info({})", ql(table)))
        .fetch_all(&mut *c)
        .await?;
    let mut primary_key: Vec<(i64, String)> = info
        .iter()
        .filter_map(|row| {
            let position: i64 = row.try_get("pk").ok()?;
            (position > 0).then(|| Ok((position, row.try_get("name")?)))
        })
        .collect::<std::result::Result<_, sqlx::Error>>()?;
    primary_key.sort_by_key(|(position, _)| *position);
    if primary_key.is_empty() {
        return Ok(());
    }
    let join = primary_key
        .iter()
        .map(|(_, name)| format!("source.{} IS target.{}", qi(name), qi(name)))
        .collect::<Vec<_>>()
        .join(" AND ");
    let differs = cols
        .iter()
        .map(|name| format!("source.{} IS NOT target.{}", qi(name), qi(name)))
        .collect::<Vec<_>>()
        .join(" OR ");
    let divergent: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM import_snapshot.{table} source JOIN main.{table} target ON {join} WHERE {differs}",
        table = qi(table),
    ))
    .fetch_one(&mut *c)
    .await?;
    if divergent != 0 {
        return Err(refused(
            "divergent_stable_id",
            format!("import rejected: {divergent} divergent stable-ID row(s) in {table}"),
        ));
    }
    Ok(())
}

async fn preflight(
    c: &mut sqlx::SqliteConnection,
    m: &MigrationManifest,
    snapshot: &Path,
) -> Result<Vec<Plan>> {
    let version: i64 =
        sqlx::query_scalar("SELECT MAX(version) FROM import_snapshot.schema_migrations")
            .fetch_one(&mut *c)
            .await?;
    if version != m.schema_version {
        return Err(refused(
            "manifest_snapshot_mismatch",
            "snapshot schema does not match manifest",
        ));
    }
    let source = sqlx::query("SELECT name, sql FROM import_snapshot.sqlite_master WHERE type='table' AND sql NOT LIKE 'CREATE VIRTUAL TABLE%' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '%_fts_%' AND name <> 'schema_migrations'").fetch_all(&mut *c).await?;
    let mut source_names: Vec<String> = source
        .iter()
        .map(|r| r.try_get("name"))
        .collect::<std::result::Result<_, _>>()?;
    let mut target_names: Vec<String> = sqlx::query_scalar("SELECT name FROM main.sqlite_master WHERE type='table' AND sql NOT LIKE 'CREATE VIRTUAL TABLE%' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '%_fts_%' AND name <> 'schema_migrations'").fetch_all(&mut *c).await?;
    source_names.sort();
    target_names.sort();
    if source_names != target_names {
        return Err(refused(
            "incompatible_snapshot_schema",
            "snapshot nonvirtual table set differs from current schema",
        ));
    }
    if m.lanes != inventory(snapshot).await? {
        return Err(refused(
            "manifest_inventory_mismatch",
            "manifest lane counts differ from attached snapshot",
        ));
    }
    let mut plans = Vec::new();
    for row in source {
        let table: String = row.try_get("name")?;
        let source_sql: String = row.try_get("sql")?;
        let target_sql: Option<String> =
            sqlx::query_scalar("SELECT sql FROM main.sqlite_master WHERE type='table' AND name=?")
                .bind(&table)
                .fetch_optional(&mut *c)
                .await?;
        if target_sql.as_deref() != Some(source_sql.as_str()) {
            return Err(refused(
                "incompatible_snapshot_schema",
                format!("table {table} differs from current schema"),
            ));
        }
        let cols: Vec<String> = sqlx::query(&format!(
            "PRAGMA import_snapshot.table_info({})",
            ql(&table)
        ))
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|r| r.try_get("name"))
        .collect::<std::result::Result<_, _>>()?;
        let target_cols: Vec<String> =
            sqlx::query(&format!("PRAGMA main.table_info({})", ql(&table)))
                .fetch_all(&mut *c)
                .await?
                .into_iter()
                .map(|r| r.try_get("name"))
                .collect::<std::result::Result<_, _>>()?;
        if cols != target_cols {
            return Err(refused(
                "incompatible_snapshot_schema",
                format!("columns for {table} differ from current schema"),
            ));
        }
        reject_divergent_stable_ids(c, &table, &cols).await?;
        let rows: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM import_snapshot.{}",
            qi(&table)
        ))
        .fetch_one(&mut *c)
        .await?;
        plans.push(Plan {
            table,
            fields: cols.iter().map(|x| qi(x)).collect::<Vec<_>>().join(", "),
            rows: rows as u64,
        });
    }
    Ok(plans)
}
pub async fn import_snapshot(
    store: &Store,
    m: &MigrationManifest,
    snapshot: &Path,
) -> Result<ImportReport> {
    if m.version != MANIFEST_VERSION || m.schema_version != migrate::latest_version() {
        return Err(refused(
            "unsupported_manifest",
            format!(
                "manifest v{} schema {} is not supported",
                m.version, m.schema_version
            ),
        ));
    }
    relative_filename(&m.snapshot)?;
    let (private, size, hash) = verified_private_copy(snapshot)?;
    if size != m.snapshot_size || hash != m.snapshot_sha256 {
        return Err(refused(
            "snapshot_integrity_mismatch",
            "snapshot size or SHA-256 differs from manifest",
        ));
    }
    let mut p = store.pool().acquire().await?;
    sqlx::query("ATTACH DATABASE ? AS import_snapshot")
        .bind(private.path().to_string_lossy().as_ref())
        .execute(&mut *p)
        .await
        .map_err(StoreError::from)?;
    let result = async {
        if m.lanes != inventory(private.path()).await? {
            return Err(refused(
                "manifest_inventory_mismatch",
                "manifest lane counts differ from private snapshot",
            ));
        }
        let (tombstones, local_only) = manifest_summaries(private.path()).await?;
        if m.tombstones != tombstones || m.local_only != local_only {
            return Err(refused(
                "manifest_summary_mismatch",
                "manifest tombstone or local-only counts differ from private snapshot",
            ));
        }
        let plans = preflight(&mut p, m, private.path()).await?;
        let mut tx = p.begin().await?;
        let mut out = ImportReport {
            checkpoint: "preflight_verified".into(),
            ..Default::default()
        };
        for plan in plans {
            let changed = sqlx::query(&format!(
                "INSERT OR IGNORE INTO main.{} ({}) SELECT {} FROM import_snapshot.{}",
                qi(&plan.table),
                plan.fields,
                plan.fields,
                qi(&plan.table)
            ))
            .execute(&mut *tx)
            .await?
            .rows_affected();
            out.accepted += changed;
            out.retained += plan.rows.saturating_sub(changed);
        }
        let bad: i64 = sqlx::query_scalar("SELECT count(*) FROM pragma_foreign_key_check")
            .fetch_one(&mut *tx)
            .await?;
        if bad != 0 {
            return Err(refused(
                "foreign_key_violation",
                "import would violate foreign keys",
            ));
        }
        tx.commit().await?;
        out.checkpoint = "committed".into();
        Ok(out)
    }
    .await;
    let detached = sqlx::query("DETACH DATABASE import_snapshot")
        .execute(&mut *p)
        .await;
    match (result, detached) {
        (Err(e), _) => Err(e),
        (Ok(_), Err(e)) => Err(e.into()),
        (Ok(v), Ok(_)) => Ok(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn legacy_task_store() -> Store {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(":memory:")
            .foreign_keys(true)
            .shared_cache(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        migrate::run_to(&pool, 14).await.unwrap();
        Store { pool }
    }

    #[tokio::test]
    async fn v13_manifest_upgrades_before_task_export_and_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v13.sqlite");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
        migrate::run_to(&pool, 13).await.unwrap();
        sqlx::query("UPDATE removed_feature_manifest SET disposition = 'exported_retained' WHERE feature = 'tasks'")
            .execute(&pool).await.unwrap();
        pool.close().await;

        let store = Store::open(&path).await.unwrap();
        let columns: Vec<String> = sqlx::query("PRAGMA table_info(removed_feature_manifest)")
            .fetch_all(store.pool())
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.try_get("name").unwrap())
            .collect();
        assert!(columns.contains(&"artifact_path".to_owned()));
        assert!(columns.contains(&"artifact_sha256".to_owned()));
        let state: String = sqlx::query_scalar(
            "SELECT disposition FROM removed_feature_manifest WHERE feature = 'tasks'",
        )
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(state, "retained_pending_export");

        let bundle_path = dir.path().join("tasks.removed_feature.json");
        export_removed_feature_tasks(&store, &bundle_path)
            .await
            .unwrap();
        cleanup_removed_feature_tasks(&store).await.unwrap();
    }

    #[tokio::test]
    async fn restored_pre_artifact_v13_manifest_upgrades_to_fourteen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("restored-v13.sqlite");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
        migrate::run_to(&pool, 12).await.unwrap();
        sqlx::query("CREATE TABLE removed_feature_manifest (feature TEXT PRIMARY KEY, bundle_version INTEGER NOT NULL, disposition TEXT NOT NULL, created_at TEXT NOT NULL)")
            .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO removed_feature_manifest VALUES ('tasks', 1, 'exported_retained', 'now')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO schema_migrations (version, name, applied_at) VALUES (13, 'remove_task_runtime', 'now')")
            .execute(&pool).await.unwrap();
        pool.close().await;

        let store = Store::open(&path).await.unwrap();
        let columns: Vec<String> = sqlx::query("PRAGMA table_info(removed_feature_manifest)")
            .fetch_all(store.pool())
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.try_get("name").unwrap())
            .collect();
        assert!(columns.contains(&"artifact_path".to_owned()));
        assert!(columns.contains(&"artifact_sha256".to_owned()));
        assert_eq!(
            migrate::latest_version(),
            sqlx::query_scalar::<_, i64>("SELECT max(version) FROM schema_migrations")
                .fetch_one(store.pool())
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn artifact_bearing_v13_manifest_preserves_identity_then_exports_and_cleans() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad-e6-v13.sqlite");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
        migrate::run_to(&pool, 12).await.unwrap();
        sqlx::query("CREATE TABLE removed_feature_manifest (feature TEXT PRIMARY KEY, bundle_version INTEGER NOT NULL, disposition TEXT NOT NULL CHECK (disposition IN ('retained_pending_export', 'exported_pending_cleanup', 'exported_cleaned')), created_at TEXT NOT NULL, artifact_path TEXT, artifact_sha256 TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO removed_feature_manifest (feature, bundle_version, disposition, created_at, artifact_path, artifact_sha256) VALUES ('tasks', 1, 'retained_pending_export', 'now', '/retained/bundle.json', 'retained-hash')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_migrations (version, name, applied_at) VALUES (13, 'remove_task_runtime', 'now')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        let store = Store::open(&path).await.unwrap();
        let retained: (String, String, String) = sqlx::query_as("SELECT disposition, artifact_path, artifact_sha256 FROM removed_feature_manifest WHERE feature = 'tasks'")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(
            retained,
            (
                "retained_pending_export".to_owned(),
                "/retained/bundle.json".to_owned(),
                "retained-hash".to_owned(),
            ),
        );

        let bundle_path = dir.path().join("tasks.removed_feature.json");
        export_removed_feature_tasks(&store, &bundle_path)
            .await
            .unwrap();
        cleanup_removed_feature_tasks(&store).await.unwrap();
    }

    async fn task_bundle_fixture() -> (Store, tempfile::TempDir, std::path::PathBuf) {
        let store = legacy_task_store().await;
        sqlx::query("INSERT INTO projects (id, name, git_common_dir, linked, created_at, updated_at) VALUES ('p', 'p', '/p', 0, 'now', 'now')")
            .execute(store.pool()).await.unwrap();
        sqlx::query("INSERT INTO tasks (id, project_id, title, goal, status, created_at, updated_at) VALUES ('t', 'p', 'title', 'goal', 'todo', 'now', 'now')")
            .execute(store.pool()).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.removed_feature.json");
        export_removed_feature_tasks(&store, &path).await.unwrap();
        (store, dir, path)
    }

    #[tokio::test]
    async fn cleanup_refuses_changed_or_added_or_deleted_task_records() {
        let (store, _dir, _) = task_bundle_fixture().await;
        sqlx::query("UPDATE tasks SET title = 'changed' WHERE id = 't'")
            .execute(store.pool())
            .await
            .unwrap();
        assert!(matches!(
            cleanup_removed_feature_tasks(&store).await,
            Err(StoreError::Refused {
                code: "removed_feature_bundle_incomplete",
                ..
            })
        ));

        let (store, _dir, _) = task_bundle_fixture().await;
        sqlx::query("INSERT INTO tasks (id, project_id, title, goal, status, created_at, updated_at) VALUES ('added', 'p', 'title', 'goal', 'todo', 'now', 'now')").execute(store.pool()).await.unwrap();
        assert!(matches!(
            cleanup_removed_feature_tasks(&store).await,
            Err(StoreError::Refused {
                code: "removed_feature_bundle_incomplete",
                ..
            })
        ));

        let (store, _dir, _) = task_bundle_fixture().await;
        sqlx::query("DELETE FROM tasks WHERE id = 't'")
            .execute(store.pool())
            .await
            .unwrap();
        assert!(matches!(
            cleanup_removed_feature_tasks(&store).await,
            Err(StoreError::Refused {
                code: "removed_feature_bundle_incomplete",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn cleanup_conserves_criterion_and_task_verification_evidence() {
        let store = legacy_task_store().await;
        let pool = store.pool();
        for sql in [
            "INSERT INTO projects (id, name, git_common_dir, linked, created_at, updated_at) VALUES ('p', 'p', '/p', 0, 'now', 'now')",
            "INSERT INTO tasks (id, project_id, title, goal, status, created_at, updated_at) VALUES ('t', 'p', 'title', 'goal', 'todo', 'now', 'now')",
            "INSERT INTO sessions (id, project_id, task_id, user_id, agent, branch, worktree_path, agent_session_key, status, started_at, last_event_at, daemon_run_id) VALUES ('s', 'p', 't', 'u', 'a', 'main', '/p', 'key', 'active', 'now', 'now', 'run')",
            "INSERT INTO memories (id, project_id, type, scope, scope_key, content, origin_session_id, created_at, updated_at) VALUES ('task-memory', 'p', 'fact', 'task', 't', 'task memory', 's', 'now', 'now')",
            "INSERT INTO memories (id, project_id, type, scope, scope_key, content, origin_session_id, created_at, updated_at) VALUES ('live-memory', 'p', 'fact', 'project', 'p', 'live memory', 's', 'now', 'now')",
            "INSERT INTO task_criteria (id, task_id, ordinal, label, text, state, verification, revision, created_at, updated_at) VALUES ('c', 't', 1, 'AC-1', 'criterion', 'pending', 'unverified', 1, 'now', 'now')",
            "INSERT INTO evidence_facts (id, project_id, kind, collector, subject, repo_branch, collected_at, collected_by_session) VALUES ('criterion-only', 'p', 'observation', 'cairn', 'criterion', 'main', 'now', 's')",
            "INSERT INTO evidence_facts (id, project_id, kind, collector, subject, repo_branch, collected_at, collected_by_session) VALUES ('verification-only', 'p', 'observation', 'cairn', 'verification', 'main', 'now', 's')",
            "INSERT INTO evidence_facts (id, project_id, kind, collector, subject, repo_branch, collected_at, collected_by_session) VALUES ('shared', 'p', 'observation', 'cairn', 'shared', 'main', 'now', 's')",
            "INSERT INTO criterion_evidence (criterion_id, evidence_id, attached_at, attached_by_session) VALUES ('c', 'criterion-only', 'now', 's')",
            "INSERT INTO criterion_evidence (criterion_id, evidence_id, attached_at, attached_by_session) VALUES ('c', 'shared', 'now', 's')",
            "INSERT INTO memory_evidence_facts (memory_id, evidence_id, role, attached_at, attached_by_session) VALUES ('live-memory', 'shared', 'supports', 'now', 's')",
            "INSERT INTO verification_runs (id, memory_id, project_id, verifier, evidence_id, result, repo_branch, checked_at, triggered_by) VALUES ('verification-only-run', 'task-memory', 'p', 'test_outcome', 'verification-only', 'verified', 'main', 'now', 'attach')",
            "INSERT INTO verification_runs (id, criterion_id, project_id, verifier, evidence_id, result, repo_branch, checked_at, triggered_by) VALUES ('shared-run', 'c', 'p', 'test_outcome', 'shared', 'verified', 'main', 'now', 'attach')",
        ] {
            sqlx::query(sql).execute(pool).await.unwrap();
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.removed_feature.json");
        export_removed_feature_tasks(&store, &path).await.unwrap();
        cleanup_removed_feature_tasks(&store).await.unwrap();

        for id in ["criterion-only", "verification-only"] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM evidence_facts WHERE id = ?")
                    .bind(id)
                    .fetch_one(pool)
                    .await
                    .unwrap(),
                0,
                "{id} has no live reference",
            );
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM evidence_facts WHERE id = 'shared'")
                .fetch_one(pool)
                .await
                .unwrap(),
            1,
            "surviving memory keeps shared evidence",
        );
    }

    #[tokio::test]
    async fn removed_feature_bundle_conserves_task_records_and_dependencies() {
        let store = legacy_task_store().await;
        let pool = store.pool();
        sqlx::query("INSERT INTO projects (id, name, git_common_dir, linked, created_at, updated_at) VALUES ('p', 'p', '/p', 0, 'now', 'now')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO tasks (id, project_id, title, goal, status, created_at, updated_at) VALUES ('t', 'p', 'title', 'goal', 'todo', 'now', 'now')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO sessions (id, project_id, task_id, user_id, agent, branch, worktree_path, agent_session_key, status, started_at, last_event_at, daemon_run_id) VALUES ('s', 'p', 't', 'u', 'a', 'main', '/p', 'key', 'active', 'now', 'now', 'run')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO memories (id, project_id, type, scope, scope_key, content, origin_session_id, created_at, updated_at) VALUES ('m', 'p', 'fact', 'task', 't', 'task memory', 's', 'now', 'now')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO memory_evidence (memory_id, observation_id, content_digest) VALUES ('m', 'o', 'digest')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO evidence_facts (id, project_id, kind, collector, subject, repo_branch, collected_at, collected_by_session) VALUES ('e', 'p', 'observation', 'cairn', 'subject', 'main', 'now', 's')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO evidence_facts (id, project_id, kind, collector, subject, repo_branch, collected_at, collected_by_session) VALUES ('task-only-e', 'p', 'observation', 'cairn', 'task subject', 'main', 'now', 's')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO memory_evidence_facts (memory_id, evidence_id, role, attached_at, attached_by_session) VALUES ('m', 'e', 'supports', 'now', 's')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO memory_evidence_facts (memory_id, evidence_id, role, attached_at, attached_by_session) VALUES ('m', 'task-only-e', 'supports', 'now', 's')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO memories (id, project_id, type, scope, scope_key, content, origin_session_id, created_at, updated_at) VALUES ('keep', 'p', 'fact', 'project', 'p', 'surviving memory', 's', 'now', 'now')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO memory_evidence_facts (memory_id, evidence_id, role, attached_at, attached_by_session) VALUES ('keep', 'e', 'supports', 'now', 's')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO memory_relations (from_memory_id, to_memory_id, kind, project_id, decided_by_session, decided_at, basis) VALUES ('m', 'm', 'reinforces', 'p', 's', 'now', 'explicit_agent')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO task_criteria (id, task_id, ordinal, label, text, state, verification, revision, created_at, updated_at) VALUES ('c', 't', 1, 'AC-1', 'criterion', 'pending', 'unverified', 1, 'now', 'now')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO task_blockers (id, task_id, description, opened_by_session, opened_at) VALUES ('b', 't', 'blocker', 's', 'now')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO task_changes (id, task_id, local_revision, kind, session_id, changed_at) VALUES ('ch', 't', 1, 'title_changed', 's', 'now')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO criterion_evidence (criterion_id, evidence_id, attached_at, attached_by_session) VALUES ('c', 'e', 'now', 's')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO verification_runs (id, criterion_id, project_id, verifier, result, repo_branch, checked_at, triggered_by) VALUES ('vr-c', 'c', 'p', 'test_outcome', 'verified', 'main', 'now', 'attach')")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO verification_runs (id, memory_id, project_id, verifier, result, repo_branch, checked_at, triggered_by) VALUES ('vr-m', 'm', 'p', 'test_outcome', 'verified', 'main', 'now', 'attach')")
            .execute(pool).await.unwrap();
        for (id, entity_type) in [
            ("o-task", "task"),
            ("o-criterion", "task_criterion"),
            ("o-blocker", "task_blocker"),
        ] {
            sqlx::query("INSERT INTO outbox (id, project_id, entity_type, entity_id, operation, idempotency_key, payload, created_at, namespace) VALUES (?, 'p', ?, 't', 'upsert', ?, '{}', 'now', 'project:p')")
                .bind(id).bind(entity_type).bind(format!("key-{id}")).execute(pool).await.unwrap();
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.removed_feature.json");
        let bundle = export_removed_feature_tasks(&store, &path).await.unwrap();

        assert_eq!(bundle.version, REMOVED_FEATURE_BUNDLE_VERSION);
        assert_eq!(bundle.feature, "tasks");
        assert_eq!(bundle.dispositions["accepted"], 0);
        assert_eq!(bundle.dispositions["rejected"], 0);
        assert_eq!(bundle.dispositions["retained"], bundle.records.len() as u64);
        for table in [
            "tasks",
            "memories",
            "memory_evidence",
            "evidence_facts",
            "memory_evidence_facts",
            "memory_relations",
            "task_criteria",
            "task_blockers",
            "task_changes",
            "criterion_evidence",
            "verification_runs",
            "outbox",
        ] {
            assert!(
                bundle.records.iter().any(|r| r.source_table == table),
                "{table}"
            );
        }
        assert!(bundle.records.iter().all(|r| r.disposition == "retained"));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tasks")
                .fetch_one(pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memories WHERE scope = 'task'")
                .fetch_one(pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memory_evidence")
                .fetch_one(pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memory_relations")
                .fetch_one(pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            serde_json::to_string_pretty(&bundle).unwrap()
        );
        cleanup_removed_feature_tasks(&store).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memories WHERE id = 'm'")
                .fetch_one(pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM memory_evidence WHERE memory_id = 'm'"
            )
            .fetch_one(pool)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM memory_evidence_facts WHERE memory_id = 'm'"
            )
            .fetch_one(pool)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM evidence_facts WHERE id = 'e'")
                .fetch_one(pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM evidence_facts WHERE id = 'task-only-e'"
            )
            .fetch_one(pool)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memory_relations")
                .fetch_one(pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM verification_runs")
                .fetch_one(pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM outbox")
                .fetch_one(pool)
                .await
                .unwrap(),
            0
        );
        assert!(sqlx::query("INSERT INTO memories (id, project_id, type, scope, scope_key, content, origin_session_id, created_at, updated_at) VALUES ('bad', 'p', 'fact', 'task', 't', 'bad', 's', 'now', 'now')").execute(pool).await.is_err());
        assert!(sqlx::query("INSERT INTO outbox (id, project_id, entity_type, entity_id, operation, idempotency_key, payload, created_at, namespace) VALUES ('bad', 'p', 'task', 't', 'upsert', 'bad', '{}', 'now', 'project:p')").execute(pool).await.is_err());
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'tasks'"
            )
            .fetch_one(pool)
            .await
            .unwrap(),
            0
        );
        let indexes: Vec<String> = sqlx::query(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'memories'",
        )
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get("name").unwrap())
        .collect();
        for name in [
            "memories_scope",
            "memories_topic",
            "memories_subject",
            "memories_verification",
            "memories_pinned",
            "memories_temporal",
            "memories_content_norm",
        ] {
            assert!(indexes.contains(&name.to_owned()), "{name}");
        }
        let mut triggers: Vec<String> = sqlx::query(
            "SELECT name FROM sqlite_master WHERE type = 'trigger' AND tbl_name = 'memories'",
        )
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get("name").unwrap())
        .collect();
        triggers.sort();
        assert_eq!(
            triggers,
            vec!["memories_fts_ad", "memories_fts_ai", "memories_fts_au"]
        );
        sqlx::query("INSERT INTO memories (id, project_id, type, scope, scope_key, content, origin_session_id, created_at, updated_at) VALUES ('lex', 'p', 'fact', 'project', 'p', 'needle one', 's', 'now', 'now')")
            .execute(pool).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'needle'"
            )
            .fetch_one(pool)
            .await
            .unwrap(),
            1
        );
        sqlx::query("UPDATE memories SET content = 'needle two' WHERE id = 'lex'")
            .execute(pool)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'two'"
            )
            .fetch_one(pool)
            .await
            .unwrap(),
            1
        );
        sqlx::query("DELETE FROM memories WHERE id = 'lex'")
            .execute(pool)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'two'"
            )
            .fetch_one(pool)
            .await
            .unwrap(),
            0
        );
        assert!(!removed_feature_tasks_pending(&store).await.unwrap());
    }
}
