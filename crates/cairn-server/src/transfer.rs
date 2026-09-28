//! Admin-only, versioned logical transfer.

use crate::auth::AdminUser;
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::State;
use axum::Json;
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Acquire, Postgres, Row, Transaction};
use std::collections::BTreeSet;
use uuid::Uuid;

const FORMAT: &str = "cairn-logical";
const VERSION: u32 = 1;
/// Maximum serialized logical bundle payload. Export is intentionally bounded
/// until streaming transfer has measured support; refusal happens before a
/// response is sent, never after a partial bundle.
pub(crate) const MAX_EXPORT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy)]
struct TableSpec {
    kind: &'static str,
    table: &'static str,
    id: &'static str,
    omit: &'static [&'static str],
}

const TABLES: &[TableSpec] = &[
    TableSpec {
        kind: "project",
        table: "projects",
        id: "t.id::text",
        omit: &[],
    },
    TableSpec {
        kind: "project_member",
        table: "project_members",
        id: "t.project_id::text || ':' || t.user_id::text",
        omit: &[],
    },
    TableSpec {
        kind: "session",
        table: "sessions",
        id: "t.id::text",
        omit: &[],
    },
    TableSpec {
        kind: "memory",
        table: "memories",
        id: "t.id::text",
        omit: &[],
    },
    TableSpec {
        kind: "handoff",
        table: "handoffs",
        id: "t.id::text",
        omit: &[],
    },
    TableSpec {
        kind: "memory_relation",
        table: "memory_relations",
        id: "t.from_memory_id::text || ':' || t.to_memory_id::text || ':' || t.kind",
        omit: &[],
    },
    TableSpec {
        kind: "safe_event",
        table: "safe_events",
        id: "t.event_id::text",
        omit: &[],
    },
    TableSpec {
        kind: "consolidation_session",
        table: "consolidation_session",
        id: "t.project_id::text || ':' || t.session_id::text",
        omit: &[],
    },
    TableSpec {
        kind: "consolidation_work",
        table: "consolidation_work",
        id: "t.event_id::text",
        omit: &[],
    },
    TableSpec {
        kind: "consolidation_run",
        table: "consolidation_runs",
        id: "t.run_id::text",
        omit: &[],
    },
    TableSpec {
        kind: "knowledge_candidate",
        table: "knowledge_candidates",
        id: "t.candidate_id::text",
        omit: &[],
    },
    TableSpec {
        kind: "candidate_source_event",
        table: "candidate_source_events",
        id: "t.candidate_id::text || ':' || t.event_id::text",
        omit: &[],
    },
    TableSpec {
        kind: "personal_knowledge",
        table: "personal_knowledge",
        id: "t.id::text",
        omit: &[],
    },
    TableSpec {
        kind: "personal_applicability",
        table: "personal_knowledge_applicability",
        id: "t.personal_id::text || ':' || t.kind || ':' || t.value",
        omit: &[],
    },
    TableSpec {
        kind: "personal_relation",
        table: "personal_knowledge_relations",
        id: "t.from_id::text || ':' || t.to_id::text || ':' || t.kind",
        omit: &[],
    },
    // Revision is a delivery cursor allocated by the destination's commit-order trigger.
    TableSpec {
        kind: "team_knowledge",
        table: "team_knowledge",
        id: "t.id::text",
        omit: &["revision"],
    },
    TableSpec {
        kind: "team_applicability",
        table: "team_knowledge_applicability",
        id: "t.team_id::text || ':' || t.kind || ':' || t.value",
        omit: &[],
    },
    TableSpec {
        kind: "team_relation",
        table: "team_knowledge_relations",
        id: "t.from_id::text || ':' || t.to_id::text || ':' || t.kind",
        omit: &[],
    },
    TableSpec {
        kind: "verification_report",
        table: "verification_reports",
        id: "t.report_id::text",
        omit: &["reference_key"],
    },
    TableSpec {
        kind: "knowledge_verification",
        table: "knowledge_verification",
        id: "t.reference_key",
        omit: &["reference_key"],
    },
    TableSpec {
        kind: "legacy_verification",
        table: "legacy_verification_audit",
        id: "t.domain || ':' || t.knowledge_id::text",
        omit: &[],
    },
    TableSpec {
        kind: "shared_pattern",
        table: "shared_patterns",
        id: "t.pattern_id::text",
        omit: &[],
    },
    TableSpec {
        kind: "applied_command",
        table: "applied_commands",
        id: "t.account_id::text || ':' || t.command_id::text",
        omit: &[],
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LogicalRecord {
    pub kind: String,
    pub source_id: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LogicalBundle {
    pub format: String,
    pub version: u32,
    pub bundle_id: Uuid,
    pub exported_at: String,
    pub records: Vec<LogicalRecord>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LogicalImportBody {
    pub import_id: Uuid,
    pub bundle: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RecordDisposition {
    pub kind: String,
    pub source_id: String,
    pub disposition: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LogicalImportReport {
    pub import_id: Uuid,
    pub accepted: u64,
    pub rejected: u64,
    pub retained: u64,
    pub unchanged: u64,
    pub dispositions: Vec<RecordDisposition>,
}

fn subtraction(omit: &[&str]) -> String {
    if omit.is_empty() {
        String::new()
    } else {
        let names = omit
            .iter()
            .map(|name| format!("'{}'", name.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");
        format!(" - ARRAY[{names}]")
    }
}

pub(crate) async fn logical_export(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> ApiResult<Json<LogicalBundle>> {
    let mut records = Vec::new();
    let bundle_id = Uuid::now_v7();
    let exported_at = chrono::Utc::now().to_rfc3339();
    let mut bytes = serde_json::to_vec(&LogicalBundle {
        format: FORMAT.to_owned(),
        version: VERSION,
        bundle_id,
        exported_at: exported_at.clone(),
        records: Vec::new(),
    })
    .map_err(|_| ApiError::internal("could not encode logical export"))?
    .len();
    // One repeatable-read transaction pins every table query to one database
    // snapshot. Pool reads per table could otherwise mix a parent before a
    // concurrent write with a child after it.
    let mut tx = state.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let mut accounts = sqlx::query(
        "SELECT id::text AS source_id,
                CASE WHEN octet_length(jsonb_build_object('id', id, 'email', email,
                                   'display_name', display_name, 'created_at', created_at)::text) > $1
                     THEN NULL ELSE jsonb_build_object('id', id, 'email', email,
                                   'display_name', display_name, 'created_at', created_at) END AS payload
           FROM users ORDER BY id",
    )
    .bind(MAX_EXPORT_BYTES as i32)
    .fetch(&mut *tx);
    while let Some(row) = accounts.try_next().await? {
        push_record(
            &mut records,
            &mut bytes,
            LogicalRecord {
                kind: "account".to_owned(),
                source_id: row.get("source_id"),
                payload: bounded_payload(&row)?,
            },
        )?;
    }
    drop(accounts);

    for spec in TABLES {
        let sql = format!(
            "SELECT ({}) AS source_id,
                    CASE WHEN octet_length((to_jsonb(t){})::text) > $1
                         THEN NULL ELSE to_jsonb(t){} END AS payload
               FROM {} t ORDER BY 1",
            spec.id,
            subtraction(spec.omit),
            subtraction(spec.omit),
            spec.table
        );
        let mut rows = sqlx::query(&sql)
            .bind(MAX_EXPORT_BYTES as i32)
            .fetch(&mut *tx);
        while let Some(row) = rows.try_next().await? {
            let mut payload = bounded_payload(&row)?;
            if spec.kind == "consolidation_session" && payload["state"] == "claimed" {
                payload["state"] = Value::String("pending".into());
                payload["claimed_by"] = Value::Null;
                payload["claim_expires_at"] = Value::Null;
            }
            if spec.kind == "consolidation_work" && payload["state"] == "pending" {
                // A source-side in-flight claim cannot consume retry capacity on transfer.
                payload["attempts"] = Value::from(0);
                payload["last_error"] = Value::Null;
            }
            push_record(
                &mut records,
                &mut bytes,
                LogicalRecord {
                    kind: spec.kind.to_owned(),
                    source_id: row.get("source_id"),
                    payload,
                },
            )?;
        }
        drop(rows);
    }

    let mut archived = sqlx::query(
        "SELECT feature AS source_id,
                CASE WHEN octet_length(to_jsonb(a)::text) > $1 THEN NULL
                     ELSE to_jsonb(a) END AS payload
           FROM removed_feature_archives a ORDER BY feature",
    )
    .bind(MAX_EXPORT_BYTES as i32)
    .fetch(&mut *tx);
    while let Some(row) = archived.try_next().await? {
        push_record(
            &mut records,
            &mut bytes,
            LogicalRecord {
                kind: "removed_feature".to_owned(),
                source_id: row.get("source_id"),
                payload: bounded_payload(&row)?,
            },
        )?;
    }
    drop(archived);

    tx.commit().await?;

    Ok(Json(LogicalBundle {
        format: FORMAT.to_owned(),
        version: VERSION,
        bundle_id,
        exported_at,
        records,
    }))
}

fn bounded_payload(row: &sqlx::postgres::PgRow) -> ApiResult<Value> {
    row.get::<Option<Value>, _>("payload").ok_or_else(|| {
        ApiError::new(
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            "logical_export_too_large",
            "logical export exceeds 32 MiB; use a physical backup or split deployment data",
        )
    })
}

fn push_record(
    records: &mut Vec<LogicalRecord>,
    bytes: &mut usize,
    record: LogicalRecord,
) -> ApiResult<()> {
    let size = serde_json::to_vec(&record)
        .map_err(|_| ApiError::internal("could not encode logical export record"))?
        .len();
    *bytes = bytes
        .checked_add(size + usize::from(!records.is_empty()))
        .ok_or_else(|| {
            ApiError::new(
                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                "logical_export_too_large",
                "logical export exceeds 32 MiB; use a physical backup or split deployment data",
            )
        })?;
    if *bytes > MAX_EXPORT_BYTES {
        return Err(ApiError::new(
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            "logical_export_too_large",
            "logical export exceeds 32 MiB; use a physical backup or split deployment data",
        ));
    }
    records.push(record);
    Ok(())
}

fn logical_records(value: &Value, import_id: Uuid) -> ApiResult<Vec<LogicalRecord>> {
    if value.get("format").and_then(Value::as_str) == Some(FORMAT) {
        let bundle: LogicalBundle = serde_json::from_value(value.clone())
            .map_err(|_| ApiError::invalid("logical bundle shape is invalid"))?;
        if bundle.version != VERSION {
            return Err(ApiError::invalid("logical bundle version is unsupported"));
        }
        if bundle.bundle_id != import_id {
            return Err(ApiError::invalid("import_id must match bundle_id"));
        }
        return Ok(bundle.records);
    }

    // Legacy removed-feature bundles are retained offline. Their records never
    // become live knowledge and therefore never gain a broader scope.
    if value.get("feature").and_then(Value::as_str) == Some("tasks")
        && value.get("version").and_then(Value::as_u64) == Some(1)
    {
        let rows = value
            .get("records")
            .and_then(Value::as_array)
            .ok_or_else(|| ApiError::invalid("removed-feature bundle records are missing"))?;
        return rows
            .iter()
            .map(|row| {
                Ok(LogicalRecord {
                    kind: "removed_feature".to_owned(),
                    source_id: format!(
                        "{}:{}",
                        row.get("source_table")
                            .and_then(Value::as_str)
                            .ok_or_else(|| ApiError::invalid(
                                "removed-feature source_table is missing"
                            ))?,
                        row.get("source_id")
                            .and_then(Value::as_str)
                            .ok_or_else(|| ApiError::invalid(
                                "removed-feature source_id is missing"
                            ))?
                    ),
                    payload: row.clone(),
                })
            })
            .collect();
    }

    Err(ApiError::invalid("bundle format is unsupported"))
}

async fn account_import(
    tx: &mut Transaction<'_, Postgres>,
    record: &LogicalRecord,
) -> Result<&'static str, sqlx::Error> {
    let inserted = sqlx::query_scalar::<_, bool>(
        "INSERT INTO users
             (id, email, display_name, password_hash, created_at,
              role, status, must_change_password)
         SELECT id, email, display_name, '!imported-disabled!', created_at,
                'member', 'disabled', true
           FROM jsonb_to_record($1)
                AS r(id uuid, email text, display_name text, created_at timestamptz)
         ON CONFLICT DO NOTHING RETURNING true",
    )
    .bind(&record.payload)
    .fetch_optional(&mut **tx)
    .await?;
    if inserted.is_some() {
        return Ok("accepted");
    }
    let same = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
             SELECT 1 FROM users
              WHERE id = ($1->>'id')::uuid
                AND email = $1->>'email'
                AND display_name = $1->>'display_name'
                AND created_at = ($1->>'created_at')::timestamptz)",
    )
    .bind(&record.payload)
    .fetch_one(&mut **tx)
    .await?;
    Ok(if same { "unchanged" } else { "rejected" })
}

async fn table_columns(
    tx: &mut Transaction<'_, Postgres>,
    table: &str,
) -> Result<String, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT string_agg(quote_ident(attname), ', ' ORDER BY attnum)
           FROM pg_attribute
          WHERE attrelid = $1::regclass AND attnum > 0
            AND NOT attisdropped AND attgenerated = ''",
    )
    .bind(table)
    .fetch_one(&mut **tx)
    .await
}

async fn table_import(
    tx: &mut Transaction<'_, Postgres>,
    spec: TableSpec,
    record: &LogicalRecord,
) -> Result<&'static str, sqlx::Error> {
    let columns = table_columns(tx, spec.table).await?;
    let insert = format!(
        "INSERT INTO {} ({columns}) SELECT {columns}
           FROM jsonb_populate_record(NULL::{}, $1) AS r
         ON CONFLICT DO NOTHING RETURNING true",
        spec.table, spec.table
    );
    let inserted = sqlx::query_scalar::<_, bool>(&insert)
        .bind(&record.payload)
        .fetch_optional(&mut **tx)
        .await?;
    if inserted.is_some() {
        return Ok("accepted");
    }
    let exact = format!(
        "SELECT EXISTS(SELECT 1 FROM {} t WHERE ({}) = $2 AND to_jsonb(t){} = $1)",
        spec.table,
        spec.id,
        subtraction(spec.omit)
    );
    let same = sqlx::query_scalar::<_, bool>(&exact)
        .bind(&record.payload)
        .bind(&record.source_id)
        .fetch_one(&mut **tx)
        .await?;
    Ok(if same { "unchanged" } else { "rejected" })
}

fn table(kind: &str) -> Option<(usize, TableSpec)> {
    TABLES
        .iter()
        .copied()
        .enumerate()
        .find(|(_, spec)| spec.kind == kind)
}

pub(crate) async fn logical_import(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<LogicalImportBody>,
) -> ApiResult<Json<LogicalImportReport>> {
    let mut records = logical_records(&body.bundle, body.import_id)?;
    let mut keys = BTreeSet::new();
    if records
        .iter()
        .any(|record| !keys.insert((record.kind.clone(), record.source_id.clone())))
    {
        return Err(ApiError::invalid(
            "bundle contains duplicate record identities",
        ));
    }
    records.sort_by_key(|record| match record.kind.as_str() {
        "account" => 0,
        "removed_feature" => usize::MAX - 1,
        kind => table(kind).map_or(usize::MAX, |(index, _)| index + 1),
    });

    let mut tx = state.pool.begin().await?;
    let reserved = sqlx::query_scalar::<_, bool>(
        "INSERT INTO logical_imports (import_id, imported_by, bundle)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING RETURNING true",
    )
    .bind(body.import_id)
    .bind(admin.id())
    .bind(&body.bundle)
    .fetch_optional(&mut *tx)
    .await?;
    if reserved.is_none() {
        let row = sqlx::query("SELECT bundle, report FROM logical_imports WHERE import_id = $1")
            .bind(body.import_id)
            .fetch_one(&mut *tx)
            .await?;
        if row.get::<Value, _>("bundle") != body.bundle {
            return Err(ApiError::invalid(
                "import_id already names a different bundle",
            ));
        }
        let report = row
            .get::<Option<Value>, _>("report")
            .ok_or_else(|| ApiError::internal("import receipt is incomplete"))?;
        tx.commit().await?;
        return serde_json::from_value(report)
            .map(Json)
            .map_err(|_| ApiError::internal("import receipt is corrupt"));
    }

    let mut report = LogicalImportReport {
        import_id: body.import_id,
        accepted: 0,
        rejected: 0,
        retained: 0,
        unchanged: 0,
        dispositions: Vec::with_capacity(records.len()),
    };
    for record in records {
        let (disposition, reason) = if record.kind == "removed_feature" {
            ("retained", Some("removed_feature".to_owned()))
        } else if record.kind == "account" || table(&record.kind).is_some() {
            let mut savepoint = tx.begin().await?;
            let result = if record.kind == "account" {
                account_import(&mut savepoint, &record).await
            } else {
                table_import(&mut savepoint, table(&record.kind).unwrap().1, &record).await
            };
            match result {
                Ok(value) => {
                    savepoint.commit().await?;
                    let reason = (value == "rejected").then(|| "identity_conflict".to_owned());
                    (value, reason)
                }
                Err(_) => {
                    savepoint.rollback().await?;
                    ("rejected", Some("constraint_refused".to_owned()))
                }
            }
        } else {
            ("rejected", Some("unsupported_record_kind".to_owned()))
        };
        match disposition {
            "accepted" => report.accepted += 1,
            "rejected" => report.rejected += 1,
            "retained" => report.retained += 1,
            "unchanged" => report.unchanged += 1,
            _ => unreachable!(),
        }
        report.dispositions.push(RecordDisposition {
            kind: record.kind,
            source_id: record.source_id,
            disposition: disposition.to_owned(),
            reason,
        });
    }

    let value = serde_json::to_value(&report)
        .map_err(|_| ApiError::internal("could not record import receipt"))?;
    sqlx::query("UPDATE logical_imports SET report = $2, finished_at = now() WHERE import_id = $1")
        .bind(body.import_id)
        .bind(value)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn transfer_allowlist_excludes_credentials_and_preserves_pending_work_provenance() {
        let tables = TABLES
            .iter()
            .map(|spec| spec.table)
            .collect::<BTreeSet<_>>();
        for refused in [
            "api_tokens",
            "web_sessions",
            "logical_imports",
            "retrieval_trace_items",
        ] {
            assert!(!tables.contains(refused));
        }
        for required in [
            "applied_commands",
            "safe_events",
            "consolidation_session",
            "consolidation_work",
            "consolidation_runs",
            "knowledge_candidates",
            "candidate_source_events",
        ] {
            assert!(tables.contains(required), "missing {required}");
        }
    }

    #[test]
    fn serialized_size_counts_the_bundle_envelope_commas_and_escaping() {
        let bundle_id = Uuid::nil();
        let exported_at = "2026-09-28T00:00:00+00:00".to_owned();
        let mut bundle = LogicalBundle {
            format: FORMAT.to_owned(),
            version: VERSION,
            bundle_id,
            exported_at,
            records: Vec::new(),
        };
        let mut bytes = serde_json::to_vec(&bundle).unwrap().len();
        for source_id in ["first", "quote-\""] {
            push_record(
                &mut bundle.records,
                &mut bytes,
                LogicalRecord {
                    kind: "test".to_owned(),
                    source_id: source_id.to_owned(),
                    payload: json!({"line": "one\ntwo"}),
                },
            )
            .unwrap();
        }
        assert_eq!(bytes, serde_json::to_vec(&bundle).unwrap().len());
    }

    #[test]
    fn legacy_tasks_are_retained_not_promoted() {
        let value = json!({
            "version": 1,
            "feature": "tasks",
            "records": [{"source_table": "tasks", "source_id": "t1", "payload": {}}]
        });
        let records = logical_records(&value, Uuid::nil()).unwrap();
        assert_eq!(records[0].kind, "removed_feature");
    }
}
