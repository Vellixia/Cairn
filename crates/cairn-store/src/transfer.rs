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
    if matches!(table, "writer_identity" | "authority_mode") {
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
        .map_err(|e| StoreError::from(e))?;
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
