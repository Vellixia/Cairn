//! Versioned, integrity-checked portable-store transfer.
use crate::{diag, migrate, spool::CommandKind, Result, Store, StoreError};
use cairn_core::{
    event::{SafeCanonicalEvent, CONTRACT_VERSION},
    eventid,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Acquire, Row};
use std::collections::BTreeMap;
use std::io::Write;
use std::io::{Read, Seek};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path};
use std::str::FromStr;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyEdgeReport {
    pub status: String,
    pub pending_operations: u64,
    pub retained_operations: u64,
    pub backup: Option<String>,
    pub manifest: Option<String>,
    pub bundle: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct PendingImport {
    pending_operations: u64,
    retained_operations: u64,
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

async fn write_removed_feature_bundle(store: &Store, path: &Path) -> Result<RemovedFeatureBundle> {
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
    Ok(bundle)
}

pub async fn export_snapshot(store: &Store, snapshot: &Path) -> Result<MigrationManifest> {
    export_snapshot_inner(store, snapshot, true).await
}

async fn export_snapshot_inner(
    store: &Store,
    snapshot: &Path,
    checkpoint_source: bool,
) -> Result<MigrationManifest> {
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
    if checkpoint_source {
        store.checkpoint().await?;
    }
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
    let snapshot_pool =
        sqlx::SqlitePool::connect(&format!("sqlite://{}", snapshot.display())).await?;
    let schema_version =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(&snapshot_pool)
            .await?;
    snapshot_pool.close().await;
    Ok(MigrationManifest {
        version: MANIFEST_VERSION,
        schema_version,
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

/// Start V1 on a fresh thin edge database while retaining the pre-V1 database
/// byte-for-byte. Only immutable pending typed operations and their required
/// correlation/ownership rows cross into the edge.
pub async fn bootstrap_legacy_edge(
    legacy: &Path,
    edge: &Path,
    artifacts: &Path,
) -> Result<(Store, LegacyEdgeReport)> {
    let edge_store = Store::open(edge).await?;
    if !legacy.is_file() {
        return Ok((
            edge_store,
            LegacyEdgeReport {
                status: "not_pending".into(),
                pending_operations: 0,
                retained_operations: 0,
                backup: None,
                manifest: None,
                bundle: None,
                detail: None,
            },
        ));
    }

    let backup = artifacts.join("legacy.sqlite");
    let manifest_path = artifacts.join("legacy.manifest.json");
    let bundle_path = artifacts.join("removed_feature.json");
    let migrated = async {
        let existing = [
            backup.as_path(),
            manifest_path.as_path(),
            bundle_path.as_path(),
        ]
        .map(Path::is_file);
        if artifacts.exists() {
            if !existing.iter().all(|exists| *exists) {
                return Err(refused(
                    "incomplete_legacy_artifacts",
                    "legacy migration artifacts are incomplete",
                ));
            }
            let manifest = read_manifest(&manifest_path)?;
            let (size, hash) = digest(&backup)?;
            if manifest.snapshot != "legacy.sqlite"
                || manifest.snapshot_size != size
                || manifest.snapshot_sha256 != hash
                || manifest.lanes != inventory(&backup).await?
            {
                return Err(refused(
                    "legacy_artifact_mismatch",
                    "legacy snapshot no longer matches its manifest",
                ));
            }
            let bundle: RemovedFeatureBundle =
                serde_json::from_slice(&std::fs::read(&bundle_path)?)
                    .map_err(|error| StoreError::Corrupt(error.to_string()))?;
            if bundle.version != REMOVED_FEATURE_BUNDLE_VERSION || bundle.feature != "tasks" {
                return Err(refused(
                    "removed_feature_artifact_invalid",
                    "Task bundle is not a supported removed-feature artifact",
                ));
            }
            return Ok::<(PendingImport, bool), StoreError>((
                import_pending_edge_state(&edge_store, &backup).await?,
                true,
            ));
        }
        let parent = artifacts
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let staging = parent.join(format!(
            ".{}.{}.{}.tmp",
            artifacts.file_name().unwrap_or_default().to_string_lossy(),
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir(&staging)?;
        let publish = async {
            let options = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(legacy)
                .read_only(true);
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await?;
            let source = Store { pool };
            let staging_backup = staging.join("legacy.sqlite");
            let mut manifest = export_snapshot_inner(&source, &staging_backup, false).await?;
            manifest.snapshot = "legacy.sqlite".into();
            write_manifest(&manifest, &staging.join("legacy.manifest.json"))?;
            write_removed_feature_bundle(&source, &staging.join("removed_feature.json")).await?;
            source.close().await;
            std::fs::rename(&staging, artifacts)?;
            Ok::<(), StoreError>(())
        }
        .await;
        if publish.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        publish?;
        let imported = import_pending_edge_state(&edge_store, &backup).await?;
        Ok::<(PendingImport, bool), StoreError>((imported, false))
    }
    .await;

    let (status, pending_operations, retained_operations, detail) = match migrated {
        Ok((imported, _)) if imported.retained_operations != 0 => (
            "warning",
            imported.pending_operations,
            imported.retained_operations,
            Some(format!(
                "{} invalid pending operation(s) retained offline",
                imported.retained_operations
            )),
        ),
        Ok((imported, true)) => ("unchanged", imported.pending_operations, 0, None),
        Ok((imported, false)) => ("migrated", imported.pending_operations, 0, None),
        Err(error) => ("warning", 0, 0, Some(error.to_string())),
    };
    Ok((
        edge_store,
        LegacyEdgeReport {
            status: status.into(),
            pending_operations,
            retained_operations,
            backup: Some(backup.display().to_string()),
            manifest: Some(manifest_path.display().to_string()),
            bundle: Some(bundle_path.display().to_string()),
            detail,
        },
    ))
}

async fn import_pending_edge_state(store: &Store, snapshot: &Path) -> Result<PendingImport> {
    let mut connection = store.pool().acquire().await?;
    sqlx::query(&format!(
        "ATTACH DATABASE {} AS legacy_snapshot",
        ql(&snapshot.to_string_lossy())
    ))
    .execute(&mut *connection)
    .await?;
    let result = async {
        let mut tx = connection.begin().await?;
        let imported = validate_pending_operations(&mut tx).await?;
        copy_common_rows(
            &mut tx,
            "users",
            "source.id IN (
                SELECT s.user_id FROM legacy_snapshot.sessions s WHERE s.id IN (
                    SELECT e.session_id FROM legacy_snapshot.event_spool e JOIN legacy_valid_events v ON v.id = e.event_id
                    UNION SELECT c.session_id FROM legacy_snapshot.command_spool c JOIN legacy_valid_commands v ON v.id = c.command_id WHERE c.session_id IS NOT NULL
                )
            )",
            &[],
        )
        .await?;
        copy_common_rows(
            &mut tx,
            "projects",
            "source.id IN (
                SELECT e.project_id FROM legacy_snapshot.event_spool e JOIN legacy_valid_events v ON v.id = e.event_id
                UNION SELECT c.project_id FROM legacy_snapshot.command_spool c JOIN legacy_valid_commands v ON v.id = c.command_id WHERE c.project_id IS NOT NULL
            )",
            &[],
        )
        .await?;
        copy_common_rows(
            &mut tx,
            "sessions",
            "source.id IN (
                SELECT e.session_id FROM legacy_snapshot.event_spool e JOIN legacy_valid_events v ON v.id = e.event_id
                UNION SELECT c.session_id FROM legacy_snapshot.command_spool c JOIN legacy_valid_commands v ON v.id = c.command_id WHERE c.session_id IS NOT NULL
            )",
            &[("previous_session_id", "NULL")],
        )
        .await?;
        copy_common_rows(
            &mut tx,
            "agent_integrations",
            "source.agent IN (
                SELECT b.agent FROM legacy_snapshot.resource_bindings b
                JOIN legacy_snapshot.installed_resources r ON r.id = b.resource_id
                WHERE r.owner = 'direct'
            )",
            &[],
        )
        .await?;
        copy_common_rows(
            &mut tx,
            "installed_resources",
            "source.owner = 'direct'",
            &[],
        )
        .await?;
        copy_common_rows(
            &mut tx,
            "resource_bindings",
            "source.resource_id IN (
                SELECT id FROM legacy_snapshot.installed_resources WHERE owner = 'direct'
            )",
            &[],
        )
        .await?;
        copy_common_rows(
            &mut tx,
            "session_event_seq",
            "source.session_id IN (
                SELECT e.session_id FROM legacy_snapshot.event_spool e JOIN legacy_valid_events v ON v.id = e.event_id
            )",
            &[],
        )
        .await?;
        copy_common_rows(
            &mut tx,
            "command_seq",
            "EXISTS (
                SELECT 1 FROM legacy_snapshot.command_spool c JOIN legacy_valid_commands v ON v.id = c.command_id
                WHERE c.scope_kind = source.scope_kind AND c.scope_key = source.scope_key
            )",
            &[],
        )
        .await?;
        copy_common_rows(
            &mut tx,
            "event_spool",
            "source.event_id IN (SELECT id FROM legacy_valid_events)",
            &[
                ("state", "'pending'"),
                ("claimed_at", "NULL"),
                ("next_attempt_at", "NULL"),
                ("last_error_kind", "NULL"),
            ],
        )
        .await?;
        copy_common_rows(
            &mut tx,
            "command_spool",
            "source.command_id IN (SELECT id FROM legacy_valid_commands)",
            &[
                ("state", "'pending'"),
                ("claimed_at", "NULL"),
                ("next_attempt_at", "NULL"),
                ("last_error_kind", "NULL"),
            ],
        )
        .await?;
        sqlx::query("DROP TABLE legacy_valid_events; DROP TABLE legacy_valid_commands")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok::<PendingImport, StoreError>(imported)
    }
    .await;
    let detached = sqlx::query("DETACH DATABASE legacy_snapshot")
        .execute(&mut *connection)
        .await;
    match (result, detached) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
        (Ok(value), Ok(_)) => Ok(value),
    }
}

async fn validate_pending_operations(
    connection: &mut sqlx::SqliteConnection,
) -> Result<PendingImport> {
    sqlx::query(
        "CREATE TEMP TABLE legacy_valid_events (id TEXT PRIMARY KEY);
         CREATE TEMP TABLE legacy_valid_commands (id TEXT PRIMARY KEY)",
    )
    .execute(&mut *connection)
    .await?;
    let mut pending_operations = 0;
    let mut retained_operations = 0;
    for row in sqlx::query(
        "SELECT event_id, session_id, project_id, account_id, session_seq, kind,
                payload, payload_bytes, boundary_class
           FROM legacy_snapshot.event_spool WHERE state IN ('pending','in_flight')",
    )
    .fetch_all(&mut *connection)
    .await?
    {
        let event_id: String = row.try_get("event_id")?;
        let session_id: String = row.try_get("session_id")?;
        let project_id: String = row.try_get("project_id")?;
        let account_id: String = row.try_get("account_id")?;
        let session_seq: i64 = row.try_get("session_seq")?;
        let kind: String = row.try_get("kind")?;
        let payload: String = row.try_get("payload")?;
        let payload_bytes: i64 = row.try_get("payload_bytes")?;
        let boundary_class: i64 = row.try_get("boundary_class")?;
        let valid = (|| {
            let event_id = uuid::Uuid::parse_str(&event_id).ok()?;
            let session_id = uuid::Uuid::parse_str(&session_id).ok()?;
            uuid::Uuid::parse_str(&project_id).ok()?;
            uuid::Uuid::parse_str(&account_id).ok()?;
            let session_seq = u64::try_from(session_seq).ok()?;
            let event = serde_json::from_str::<SafeCanonicalEvent>(&payload).ok()?;
            event.validate().ok()?;
            (event.event_id == event_id
                && event.contract_version == CONTRACT_VERSION
                && event.session_id == session_id
                && event.session_seq == session_seq
                && event.kind.as_str() == kind
                && event.boundary_class() == boundary_class
                && payload.len() as i64 == payload_bytes
                && eventid::event_id(session_id, session_seq) == event_id)
                .then_some(())
        })()
        .is_some();
        if valid {
            sqlx::query("INSERT INTO legacy_valid_events VALUES (?)")
                .bind(event_id)
                .execute(&mut *connection)
                .await?;
            pending_operations += 1;
        } else {
            retained_operations += 1;
        }
    }
    for row in sqlx::query(
        "SELECT command_id, scope_kind, scope_key, session_id, project_id, account_id,
                command_seq, kind, payload
           FROM legacy_snapshot.command_spool WHERE state IN ('pending','in_flight')",
    )
    .fetch_all(&mut *connection)
    .await?
    {
        let command_id: String = row.try_get("command_id")?;
        let scope_kind: String = row.try_get("scope_kind")?;
        let scope_key: String = row.try_get("scope_key")?;
        let session_id: Option<String> = row.try_get("session_id")?;
        let project_id: Option<String> = row.try_get("project_id")?;
        let account_id: String = row.try_get("account_id")?;
        let command_seq: i64 = row.try_get("command_seq")?;
        let kind: String = row.try_get("kind")?;
        let payload: String = row.try_get("payload")?;
        let valid = (|| {
            let command_id = uuid::Uuid::parse_str(&command_id).ok()?;
            let scope_key_id = uuid::Uuid::parse_str(&scope_key).ok()?;
            uuid::Uuid::parse_str(&account_id).ok()?;
            project_id
                .as_deref()
                .map(uuid::Uuid::parse_str)
                .transpose()
                .ok()?;
            let session_id = session_id
                .as_deref()
                .map(uuid::Uuid::parse_str)
                .transpose()
                .ok()?;
            let command_seq = u64::try_from(command_seq).ok()?;
            CommandKind::from_str(&kind).ok()?;
            serde_json::from_str::<serde_json::Value>(&payload).ok()?;
            let scope_matches = match scope_kind.as_str() {
                "session" => session_id == Some(scope_key_id),
                "store" => session_id.is_none(),
                _ => false,
            };
            (scope_key_id.to_string() == scope_key
                && scope_matches
                && eventid::command_id(&scope_kind, &scope_key_id.to_string(), command_seq)
                    == command_id)
                .then_some(())
        })()
        .is_some();
        if valid {
            sqlx::query("INSERT INTO legacy_valid_commands VALUES (?)")
                .bind(command_id)
                .execute(&mut *connection)
                .await?;
            pending_operations += 1;
        } else {
            retained_operations += 1;
        }
    }
    Ok(PendingImport {
        pending_operations,
        retained_operations,
    })
}

async fn copy_common_rows(
    connection: &mut sqlx::SqliteConnection,
    table: &str,
    predicate: &str,
    replacements: &[(&str, &str)],
) -> Result<()> {
    let exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM legacy_snapshot.sqlite_master WHERE type='table' AND name=?)",
    )
    .bind(table)
    .fetch_one(&mut *connection)
    .await?;
    if exists == 0 {
        return Ok(());
    }
    let target: Vec<String> = sqlx::query(&format!("PRAGMA main.table_info({})", ql(table)))
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| row.try_get("name"))
        .collect::<std::result::Result<_, _>>()?;
    let source: std::collections::BTreeSet<String> =
        sqlx::query(&format!("PRAGMA legacy_snapshot.table_info({})", ql(table)))
            .fetch_all(&mut *connection)
            .await?
            .into_iter()
            .map(|row| row.try_get("name"))
            .collect::<std::result::Result<_, _>>()?;
    let columns: Vec<_> = target
        .into_iter()
        .filter(|column| source.contains(column))
        .collect();
    if columns.is_empty() {
        return Ok(());
    }
    let fields = columns
        .iter()
        .map(|column| qi(column))
        .collect::<Vec<_>>()
        .join(", ");
    let values = columns
        .iter()
        .map(|column| {
            replacements
                .iter()
                .find_map(|(name, value)| (*name == column).then_some((*value).to_owned()))
                .unwrap_or_else(|| format!("source.{}", qi(column)))
        })
        .collect::<Vec<_>>()
        .join(", ");
    sqlx::query(&format!(
        "INSERT OR IGNORE INTO main.{} ({fields}) SELECT {values} FROM legacy_snapshot.{} source WHERE {predicate}",
        qi(table),
        qi(table)
    ))
    .execute(&mut *connection)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::event::{EventAgent, EventKind};
    use cairn_core::eventid;
    use uuid::Uuid;

    #[tokio::test]
    async fn legacy_bootstrap_preserves_source_and_moves_only_pending_spool_rows() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("cairn.sqlite3");
        let edge = dir.path().join("edge.sqlite3");
        let artifacts = dir.path().join("removed_feature");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&legacy)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
        migrate::run_to(&pool, 12).await.unwrap();
        let user_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let project_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let session_id = Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap();
        let account_id = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
        let ignored_user = Uuid::parse_str("00000000-0000-0000-0000-000000000011").unwrap();
        let ignored_project = Uuid::parse_str("00000000-0000-0000-0000-000000000012").unwrap();
        let ignored_session = Uuid::parse_str("00000000-0000-0000-0000-000000000013").unwrap();
        sqlx::query("INSERT INTO users VALUES (?, NULL, 'user', 'now'), (?, NULL, 'user', 'now')")
            .bind(user_id.to_string())
            .bind(ignored_user.to_string())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO projects (id,name,git_common_dir,linked,created_at,updated_at) VALUES (?,'project','/repo',1,'now','now'), (?,'ignored','/ignored',1,'now','now')")
            .bind(project_id.to_string()).bind(ignored_project.to_string()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO tasks (id,project_id,title,goal,status,created_at,updated_at) VALUES ('t',?,'task','goal','todo','now','now')")
            .bind(project_id.to_string()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO sessions (id,project_id,task_id,user_id,agent,branch,worktree_path,agent_session_key,status,started_at,last_event_at,daemon_run_id) VALUES (?,?, 't',?,'codex','main','/repo','agent-key','active','now','now','run'), (?,?,NULL,?,'codex','main','/ignored','ignored-key','active','now','now','run')")
            .bind(session_id.to_string()).bind(project_id.to_string()).bind(user_id.to_string())
            .bind(ignored_session.to_string()).bind(ignored_project.to_string()).bind(ignored_user.to_string())
            .execute(&pool).await.unwrap();
        let pending_event = SafeCanonicalEvent {
            event_id: eventid::event_id(session_id, 1),
            contract_version: CONTRACT_VERSION,
            kind: EventKind::AgentQuiesced,
            agent: EventAgent::Codex,
            vendor_event: None,
            session_id,
            session_seq: 1,
            occurred_at: chrono::Utc::now(),
            content: None,
        };
        let delivered_event = SafeCanonicalEvent {
            event_id: eventid::event_id(session_id, 2),
            session_seq: 2,
            ..pending_event.clone()
        };
        for (event, state) in [(&pending_event, "pending"), (&delivered_event, "delivered")] {
            let payload = serde_json::to_string(event).unwrap();
            sqlx::query("INSERT INTO event_spool (event_id,session_id,project_id,account_id,session_seq,kind,payload,payload_bytes,boundary_class,state,created_at) VALUES (?,?,?,?,?,?,?,?,?,?, 'now')")
                .bind(event.event_id.to_string()).bind(session_id.to_string()).bind(project_id.to_string())
                .bind(account_id.to_string()).bind(event.session_seq as i64).bind(event.kind.as_str())
                .bind(&payload).bind(payload.len() as i64).bind(event.boundary_class()).bind(state)
                .execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO event_spool (event_id,session_id,project_id,account_id,session_seq,kind,payload,payload_bytes,boundary_class,state,created_at) VALUES (?,?,?,?,3,'agent_quiesced','{}',2,0,'pending','now')")
            .bind(eventid::event_id(ignored_session, 3).to_string()).bind(ignored_session.to_string())
            .bind(ignored_project.to_string()).bind(account_id.to_string()).execute(&pool).await.unwrap();
        for (seq, state) in [(1_u64, "in_flight"), (2, "delivered")] {
            sqlx::query("INSERT INTO command_spool (command_id,scope_kind,scope_key,session_id,project_id,account_id,command_seq,kind,payload,state,created_at) VALUES (?,'session',?,?,?,?,?,'remember','{}',?,'now')")
                .bind(eventid::command_id("session", &session_id.to_string(), seq).to_string())
                .bind(session_id.to_string()).bind(session_id.to_string()).bind(project_id.to_string())
                .bind(account_id.to_string()).bind(seq as i64).bind(state).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO command_spool (command_id,scope_kind,scope_key,session_id,project_id,account_id,command_seq,kind,payload,state,created_at) VALUES (?,'session',?,?,?,?,4,'remember','{}','pending','now')")
            .bind("00000000-0000-0000-0000-000000000099")
            .bind(ignored_session.to_string()).bind(ignored_session.to_string()).bind(ignored_project.to_string())
            .bind(account_id.to_string()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO agent_integrations (agent,adapter_version,compatibility,level,completion_guarantee,connected_at) VALUES ('codex',1,'supported','full','automatic','now'), ('claude_code',1,'supported','full','automatic','now')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO installed_resources (id,kind,owner,scope,location,activation,installed_at) VALUES ('00000000-0000-0000-0000-000000000021','hook','direct','user','/owned','active','now'), ('00000000-0000-0000-0000-000000000022','hook','manager','user','/manager','active','now')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO resource_bindings VALUES ('codex','hook','00000000-0000-0000-0000-000000000021','now'), ('claude_code','hook','00000000-0000-0000-0000-000000000022','now')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO manager_integrations (manager,compatibility,target_apps,connected_at) VALUES ('manager','supported','[]','now')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO capability_evidence (agent,capability,evidence,established_at) VALUES ('codex','capture','observation','now')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO migration_states (id,agent,kind,source_owner,source_scope,source_location,target_owner,target_scope,target_location,phase,overlap_permitted,started_at) VALUES ('00000000-0000-0000-0000-000000000023','codex','hook','direct','user','/old','manager','user','/new','copy',0,'now')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO recovery_artifacts VALUES ('00000000-0000-0000-0000-000000000024','codex','hook','/source','/artifact','hash','now')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO capture_disposition_counts VALUES (?, 'codex','agent_quiesced','captured','2026-09-24',1)").bind(project_id.to_string()).execute(&pool).await.unwrap();
        let before_db = std::fs::read(&legacy).unwrap();
        let wal = legacy.with_extension("sqlite3-wal");
        let before_wal = std::fs::read(&wal).unwrap();

        let (store, report) = bootstrap_legacy_edge(&legacy, &edge, &artifacts)
            .await
            .unwrap();

        assert_eq!(report.status, "warning");
        assert_eq!(report.pending_operations, 2);
        assert_eq!(report.retained_operations, 2);
        assert_eq!(std::fs::read(&legacy).unwrap(), before_db);
        assert_eq!(std::fs::read(&wal).unwrap(), before_wal);
        let ids: Vec<String> = sqlx::query_scalar("SELECT event_id FROM event_spool")
            .fetch_all(store.pool())
            .await
            .unwrap();
        assert_eq!(ids, vec![pending_event.event_id.to_string()]);
        let commands: Vec<(String, String)> =
            sqlx::query_as("SELECT command_id, state FROM command_spool")
                .fetch_all(store.pool())
                .await
                .unwrap();
        assert_eq!(
            commands,
            vec![(
                eventid::command_id("session", &session_id.to_string(), 1).to_string(),
                "pending".into()
            )]
        );
        for table in ["users", "projects", "sessions"] {
            let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(store.pool())
                .await
                .unwrap();
            assert_eq!(count, 1, "{table}");
        }
        for table in [
            "manager_integrations",
            "capability_evidence",
            "migration_states",
            "recovery_artifacts",
            "capture_disposition_counts",
        ] {
            let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(store.pool())
                .await
                .unwrap();
            assert_eq!(count, 0, "{table}");
        }
        let ownership: (String, String, String) = sqlx::query_as("SELECT a.agent, r.owner, r.location FROM agent_integrations a JOIN resource_bindings b USING (agent) JOIN installed_resources r ON r.id = b.resource_id")
            .fetch_one(store.pool()).await.unwrap();
        assert_eq!(
            ownership,
            ("codex".into(), "direct".into(), "/owned".into())
        );
        for removed in ["tasks", "memories", "outbox"] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
            )
            .bind(removed)
            .fetch_one(store.pool())
            .await
            .unwrap();
            assert_eq!(exists, 0, "{removed}");
        }
        assert!(artifacts.join("legacy.sqlite").is_file());
        assert!(artifacts.join("legacy.manifest.json").is_file());
        assert!(artifacts.join("removed_feature.json").is_file());
        store.close().await;

        let (reopened, repeated) = bootstrap_legacy_edge(&legacy, &edge, &artifacts)
            .await
            .unwrap();
        assert_eq!(repeated.status, "warning");
        assert_eq!(repeated.pending_operations, 2);
        assert_eq!(repeated.retained_operations, 2);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM event_spool")
            .fetch_one(reopened.pool())
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn failed_legacy_export_never_publishes_partial_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("cairn.sqlite3");
        let edge = dir.path().join("edge.sqlite3");
        let artifacts = dir.path().join("removed_feature");
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", legacy.display()))
            .await
            .unwrap();
        migrate::run_to(&pool, 12).await.unwrap();
        sqlx::query("INSERT INTO projects (id,name,git_common_dir,linked,created_at,updated_at) VALUES ('p','project','/repo',1,'now','now')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO tasks (id,project_id,title,goal,status,created_at,updated_at) VALUES ('t','p',X'80','goal','todo','now','now')").execute(&pool).await.unwrap();

        let (store, report) = bootstrap_legacy_edge(&legacy, &edge, &artifacts)
            .await
            .unwrap();

        assert_eq!(report.status, "warning");
        assert!(!artifacts.exists());
        let version: i64 = sqlx::query_scalar("SELECT MAX(version) FROM schema_migrations")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(version, migrate::latest_version());
    }
}
