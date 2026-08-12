//! The local integration record (FR-182–FR-184, D28, D28a, D19a).
//!
//! Everything in this module is machine-local. **No function here enqueues an
//! outbox row**, and there is deliberately no outbox entity type for any of
//! these tables: an agent configuration path, a content hash, or an
//! integration health detail must never reach the shared server (SC-120).
//!
//! The reference counting is the part worth reading. An `installed_resource`
//! is one *physical* thing — a file, a managed block, an entry. A
//! `resource_binding` is one agent's dependency on it. Connect ensures a
//! binding exists; disconnect ensures it does not; and a resource is deleted
//! only when its last binding goes. That is what keeps the shared `AGENTS.md`
//! block alive for OpenCode when Codex disconnects.

use crate::{Result, Store};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

fn now() -> String {
    Utc::now().to_rfc3339()
}

/// One connected agent's record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentIntegration {
    pub agent: String,
    pub adapter_version: i64,
    pub detected_version: Option<String>,
    pub compatibility: String,
    pub level: String,
    pub completion_guarantee: String,
    pub connected_at: String,
    pub last_verified_at: Option<String>,
}

/// One physical installed thing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledResource {
    pub id: Uuid,
    pub kind: String,
    pub owner: String,
    pub scope: String,
    pub location: String,
    pub content_hash: Option<String>,
    pub artifact_schema: Option<i64>,
    pub artifact_revision: Option<String>,
    pub activation: String,
    pub installed_at: String,
    pub last_verified_at: Option<String>,
    /// One bit about Cairn's own edit, never a copy of the developer's file.
    pub container_single_line: bool,
    pub created_container: bool,
}

/// What Cairn has established about one capability here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityEvidence {
    pub agent: String,
    pub capability: String,
    pub evidence: String,
    pub established_at: String,
    pub agent_version: Option<String>,
    pub degraded: Option<bool>,
}

/// A resource plus every agent bound to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundResource {
    #[serde(flatten)]
    pub resource: InstalledResource,
    /// Sorted, so diagnostics report a stable `serves` list.
    pub serves: Vec<String>,
}

/// Record or refresh an agent's integration row.
pub async fn upsert_agent(store: &Store, a: &AgentIntegration) -> Result<()> {
    sqlx::query(
        "INSERT INTO agent_integrations
            (agent, adapter_version, detected_version, compatibility, level,
             completion_guarantee, connected_at, last_verified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (agent) DO UPDATE SET
             adapter_version      = excluded.adapter_version,
             detected_version     = excluded.detected_version,
             compatibility        = excluded.compatibility,
             level                = excluded.level,
             completion_guarantee = excluded.completion_guarantee,
             last_verified_at     = excluded.last_verified_at",
    )
    .bind(&a.agent)
    .bind(a.adapter_version)
    .bind(&a.detected_version)
    .bind(&a.compatibility)
    .bind(&a.level)
    .bind(&a.completion_guarantee)
    .bind(&a.connected_at)
    .bind(&a.last_verified_at)
    .execute(store.pool())
    .await?;
    Ok(())
}

pub async fn agent(store: &Store, agent: &str) -> Result<Option<AgentIntegration>> {
    let row = sqlx::query("SELECT * FROM agent_integrations WHERE agent = ?1")
        .bind(agent)
        .fetch_optional(store.pool())
        .await?;
    Ok(row.map(|r| AgentIntegration {
        agent: r.get("agent"),
        adapter_version: r.get("adapter_version"),
        detected_version: r.get("detected_version"),
        compatibility: r.get("compatibility"),
        level: r.get("level"),
        completion_guarantee: r.get("completion_guarantee"),
        connected_at: r.get("connected_at"),
        last_verified_at: r.get("last_verified_at"),
    }))
}

pub async fn list_agents(store: &Store) -> Result<Vec<String>> {
    let rows = sqlx::query("SELECT agent FROM agent_integrations ORDER BY agent")
        .fetch_all(store.pool())
        .await?;
    Ok(rows.iter().map(|r| r.get("agent")).collect())
}

/// Ensure a physical resource exists at this location, and that this agent is
/// bound to it.
///
/// Idempotent in both halves: connecting twice changes nothing (FR-157). When
/// the resource already exists — because another agent installed it — this
/// adds only the binding, which is how one `AGENTS.md` block comes to serve
/// two agents (FR-144, FR-243).
pub async fn bind(store: &Store, agent: &str, resource: &InstalledResource) -> Result<Uuid> {
    let mut tx = store.pool().begin().await?;

    let existing: Option<String> =
        sqlx::query("SELECT id FROM installed_resources WHERE kind = ?1 AND location = ?2")
            .bind(&resource.kind)
            .bind(&resource.location)
            .fetch_optional(&mut *tx)
            .await?
            .map(|r| r.get("id"));

    let id = match existing {
        Some(id) => {
            sqlx::query(
                "UPDATE installed_resources SET
                     owner = ?2, scope = ?3, content_hash = ?4, artifact_schema = ?5,
                     artifact_revision = ?6, activation = ?7, last_verified_at = ?8,
                     container_single_line = ?9, created_container = ?10
                 WHERE id = ?1",
            )
            .bind(&id)
            .bind(&resource.owner)
            .bind(&resource.scope)
            .bind(&resource.content_hash)
            .bind(resource.artifact_schema)
            .bind(&resource.artifact_revision)
            .bind(&resource.activation)
            .bind(now())
            .bind(resource.container_single_line as i64)
            .bind(resource.created_container as i64)
            .execute(&mut *tx)
            .await?;
            Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::now_v7())
        }
        None => {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO installed_resources
                    (id, kind, owner, scope, location, content_hash, artifact_schema,
                     artifact_revision, activation, installed_at, last_verified_at,
                     container_single_line, created_container)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            )
            .bind(id.to_string())
            .bind(&resource.kind)
            .bind(&resource.owner)
            .bind(&resource.scope)
            .bind(&resource.location)
            .bind(&resource.content_hash)
            .bind(resource.artifact_schema)
            .bind(&resource.artifact_revision)
            .bind(&resource.activation)
            .bind(now())
            .bind(now())
            .bind(resource.container_single_line as i64)
            .bind(resource.created_container as i64)
            .execute(&mut *tx)
            .await?;
            id
        }
    };

    sqlx::query(
        "INSERT INTO resource_bindings (agent, kind, resource_id, bound_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (agent, kind) DO UPDATE SET resource_id = excluded.resource_id",
    )
    .bind(agent)
    .bind(&resource.kind)
    .bind(id.to_string())
    .bind(now())
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(id)
}

/// What happened when a binding was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unbound {
    /// There was no binding to drop.
    Nothing,
    /// The binding went; the resource stays for the agents still using it.
    ResourceKept { remaining: usize },
    /// The last binding went, so the resource goes too.
    ResourceRemoved,
}

/// Drop this agent's dependency on one resource kind.
///
/// The resource itself is deleted **only if no binding remains** (FR-243). The
/// last consumer's disconnect is what removes the file, the block, or the
/// entry — and the caller does the filesystem half only when this says
/// `ResourceRemoved`.
pub async fn unbind(store: &Store, agent: &str, kind: &str) -> Result<Unbound> {
    let mut tx = store.pool().begin().await?;

    let resource_id: Option<String> =
        sqlx::query("SELECT resource_id FROM resource_bindings WHERE agent = ?1 AND kind = ?2")
            .bind(agent)
            .bind(kind)
            .fetch_optional(&mut *tx)
            .await?
            .map(|r| r.get("resource_id"));

    let Some(resource_id) = resource_id else {
        tx.commit().await?;
        return Ok(Unbound::Nothing);
    };

    sqlx::query("DELETE FROM resource_bindings WHERE agent = ?1 AND kind = ?2")
        .bind(agent)
        .bind(kind)
        .execute(&mut *tx)
        .await?;

    let remaining: i64 =
        sqlx::query("SELECT COUNT(*) AS n FROM resource_bindings WHERE resource_id = ?1")
            .bind(&resource_id)
            .fetch_one(&mut *tx)
            .await?
            .get("n");

    let outcome = if remaining == 0 {
        // A row with zero bindings is deleted in the same transaction that
        // removes its last binding.
        sqlx::query("DELETE FROM installed_resources WHERE id = ?1")
            .bind(&resource_id)
            .execute(&mut *tx)
            .await?;
        Unbound::ResourceRemoved
    } else {
        Unbound::ResourceKept {
            remaining: remaining as usize,
        }
    };

    tx.commit().await?;
    Ok(outcome)
}

/// Every resource this agent is bound to, with the full consumer list.
pub async fn bound_resources(store: &Store, agent: &str) -> Result<Vec<BoundResource>> {
    let rows = sqlx::query(
        "SELECT r.*, b.kind AS binding_kind
         FROM resource_bindings b
         JOIN installed_resources r ON r.id = b.resource_id
         WHERE b.agent = ?1
         ORDER BY b.kind",
    )
    .bind(agent)
    .fetch_all(store.pool())
    .await?;

    let mut out = Vec::new();
    for r in rows {
        let id: String = r.get("id");
        let serves: Vec<String> = sqlx::query(
            "SELECT agent FROM resource_bindings WHERE resource_id = ?1 ORDER BY agent",
        )
        .bind(&id)
        .fetch_all(store.pool())
        .await?
        .iter()
        .map(|s| s.get("agent"))
        .collect();

        out.push(BoundResource {
            resource: InstalledResource {
                id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::now_v7()),
                kind: r.get("kind"),
                owner: r.get("owner"),
                scope: r.get("scope"),
                location: r.get("location"),
                content_hash: r.get("content_hash"),
                artifact_schema: r.get("artifact_schema"),
                artifact_revision: r.get("artifact_revision"),
                activation: r.get("activation"),
                installed_at: r.get("installed_at"),
                last_verified_at: r.get("last_verified_at"),
                container_single_line: r.get::<i64, _>("container_single_line") != 0,
                created_container: r.get::<i64, _>("created_container") != 0,
            },
            serves,
        });
    }
    Ok(out)
}

/// Remove an agent's record — but only once its last binding is gone.
///
/// An agent whose only remaining resource is manager-owned keeps its record so
/// the withdrawal stays verifiable (FR-244, D28a).
pub async fn remove_agent_if_unbound(store: &Store, agent: &str) -> Result<bool> {
    let mut tx = store.pool().begin().await?;
    let remaining: i64 =
        sqlx::query("SELECT COUNT(*) AS n FROM resource_bindings WHERE agent = ?1")
            .bind(agent)
            .fetch_one(&mut *tx)
            .await?
            .get("n");
    if remaining > 0 {
        tx.commit().await?;
        return Ok(false);
    }
    sqlx::query("DELETE FROM capability_evidence WHERE agent = ?1")
        .bind(agent)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM agent_integrations WHERE agent = ?1")
        .bind(agent)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}

/// Record that a capability was established here.
///
/// Rows are created as a byproduct of work that already happens — writing a
/// resource, or receiving a canonical event. Cairn never synthesizes an event
/// or calls an undocumented interface to create one (D19a).
pub async fn record_evidence(store: &Store, e: &CapabilityEvidence) -> Result<()> {
    sqlx::query(
        "INSERT INTO capability_evidence
            (agent, capability, evidence, established_at, agent_version, degraded)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (agent, capability) DO UPDATE SET
             evidence       = excluded.evidence,
             established_at = excluded.established_at,
             agent_version  = excluded.agent_version,
             degraded       = excluded.degraded",
    )
    .bind(&e.agent)
    .bind(&e.capability)
    .bind(&e.evidence)
    .bind(&e.established_at)
    .bind(&e.agent_version)
    .bind(e.degraded.map(|d| d as i64))
    .execute(store.pool())
    .await?;
    Ok(())
}

pub async fn evidence(store: &Store, agent: &str) -> Result<Vec<CapabilityEvidence>> {
    let rows =
        sqlx::query("SELECT * FROM capability_evidence WHERE agent = ?1 ORDER BY capability")
            .bind(agent)
            .fetch_all(store.pool())
            .await?;
    Ok(rows
        .iter()
        .map(|r| CapabilityEvidence {
            agent: r.get("agent"),
            capability: r.get("capability"),
            evidence: r.get("evidence"),
            established_at: r.get("established_at"),
            agent_version: r.get("agent_version"),
            degraded: r.get::<Option<i64>, _>("degraded").map(|d| d != 0),
        })
        .collect())
}

/// Discard evidence a version change invalidated (FR-245).
///
/// `observation` evidence is version-bound: what a previous build did is not
/// evidence about this one, so it goes. `introspection` evidence proves a fact
/// about a resource Cairn itself wrote, so a version change re-derives it in
/// place rather than discarding it.
///
/// Returns how many rows were discarded.
pub async fn invalidate_observation_evidence(
    store: &Store,
    agent: &str,
    detected_version: Option<&str>,
) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM capability_evidence
         WHERE agent = ?1
           AND evidence = 'observation'
           AND (agent_version IS NOT ?2)",
    )
    .bind(agent)
    .bind(detected_version)
    .execute(store.pool())
    .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> Store {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cairn.db");
        // The directory must outlive the pool.
        std::mem::forget(dir);
        Store::open(&path).await.unwrap()
    }

    fn agent_row(agent: &str) -> AgentIntegration {
        AgentIntegration {
            agent: agent.into(),
            adapter_version: 1,
            detected_version: Some("1.0.0".into()),
            compatibility: "compatible_unverified".into(),
            level: "mcp_plus".into(),
            completion_guarantee: "not_demonstrated".into(),
            connected_at: now(),
            last_verified_at: None,
        }
    }

    fn resource(kind: &str, location: &str) -> InstalledResource {
        InstalledResource {
            id: Uuid::now_v7(),
            kind: kind.into(),
            owner: "direct".into(),
            scope: "project_shared".into(),
            location: location.into(),
            content_hash: Some("abc123abc123".into()),
            artifact_schema: Some(1),
            artifact_revision: Some("def456def456".into()),
            activation: "not_applicable".into(),
            installed_at: now(),
            last_verified_at: None,
            container_single_line: false,
            created_container: false,
        }
    }

    #[tokio::test]
    async fn a_shared_resource_survives_until_its_last_binding_goes() {
        // FR-243, SC-137 — the failure the old `satisfied_by` design caused.
        let s = store().await;
        upsert_agent(&s, &agent_row("codex")).await.unwrap();
        upsert_agent(&s, &agent_row("opencode")).await.unwrap();

        let block = resource("instructions", "/repo/AGENTS.md");
        let a = bind(&s, "codex", &block).await.unwrap();
        let b = bind(&s, "opencode", &block).await.unwrap();
        assert_eq!(a, b, "two agents must bind to one physical resource");

        assert_eq!(
            unbind(&s, "codex", "instructions").await.unwrap(),
            Unbound::ResourceKept { remaining: 1 }
        );
        let still = bound_resources(&s, "opencode").await.unwrap();
        assert_eq!(still.len(), 1, "OpenCode lost the block Codex disconnected");

        assert_eq!(
            unbind(&s, "opencode", "instructions").await.unwrap(),
            Unbound::ResourceRemoved
        );
    }

    #[tokio::test]
    async fn doctor_can_see_who_a_resource_serves() {
        // FR-243: the `serves` list is mandatory on a multi-binding resource.
        let s = store().await;
        upsert_agent(&s, &agent_row("claude-code")).await.unwrap();
        upsert_agent(&s, &agent_row("opencode")).await.unwrap();
        let skill = resource("skill", "/home/dev/.claude/skills/cairn");
        bind(&s, "claude-code", &skill).await.unwrap();
        bind(&s, "opencode", &skill).await.unwrap();

        let bound = bound_resources(&s, "opencode").await.unwrap();
        assert_eq!(bound[0].serves, vec!["claude-code", "opencode"]);
    }

    #[tokio::test]
    async fn binding_is_idempotent() {
        // FR-157: connecting twice changes nothing.
        let s = store().await;
        upsert_agent(&s, &agent_row("codex")).await.unwrap();
        let r = resource("mcp", "/home/dev/.codex/config.toml");
        let first = bind(&s, "codex", &r).await.unwrap();
        let second = bind(&s, "codex", &r).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(bound_resources(&s, "codex").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn unbinding_something_absent_is_not_an_error() {
        let s = store().await;
        upsert_agent(&s, &agent_row("codex")).await.unwrap();
        assert_eq!(
            unbind(&s, "codex", "skill").await.unwrap(),
            Unbound::Nothing
        );
    }

    #[tokio::test]
    async fn an_agent_record_outlives_a_native_disconnect_while_a_manager_owns_something() {
        // FR-244, D28a: otherwise there is nothing left to verify the
        // withdrawal against.
        let s = store().await;
        upsert_agent(&s, &agent_row("codex")).await.unwrap();
        let mut manager_owned = resource("mcp", "/home/dev/.codex/config.toml");
        manager_owned.owner = "manager".into();
        manager_owned.content_hash = None;
        bind(&s, "codex", &manager_owned).await.unwrap();
        bind(
            &s,
            "codex",
            &resource("lifecycle", "/home/dev/.codex/hooks.json"),
        )
        .await
        .unwrap();

        unbind(&s, "codex", "lifecycle").await.unwrap();
        assert!(
            !remove_agent_if_unbound(&s, "codex").await.unwrap(),
            "the record went while a manager-owned resource remained"
        );
        assert!(agent(&s, "codex").await.unwrap().is_some());

        unbind(&s, "codex", "mcp").await.unwrap();
        assert!(remove_agent_if_unbound(&s, "codex").await.unwrap());
        assert!(agent(&s, "codex").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_version_change_discards_observation_evidence_and_only_that() {
        // FR-245, SC-138.
        let s = store().await;
        upsert_agent(&s, &agent_row("claude-code")).await.unwrap();
        for (capability, kind) in [
            ("lifecycle_session_open", "observation"),
            ("lifecycle_tool_success", "observation"),
            ("mcp", "introspection"),
            ("instructions", "introspection"),
        ] {
            record_evidence(
                &s,
                &CapabilityEvidence {
                    agent: "claude-code".into(),
                    capability: capability.into(),
                    evidence: kind.into(),
                    established_at: now(),
                    agent_version: Some("2.1.220".into()),
                    degraded: None,
                },
            )
            .await
            .unwrap();
        }
        assert_eq!(evidence(&s, "claude-code").await.unwrap().len(), 4);

        // Same version: nothing is discarded.
        assert_eq!(
            invalidate_observation_evidence(&s, "claude-code", Some("2.1.220"))
                .await
                .unwrap(),
            0
        );

        // A new version discards the two observation rows and keeps the two
        // introspection rows.
        assert_eq!(
            invalidate_observation_evidence(&s, "claude-code", Some("2.2.0"))
                .await
                .unwrap(),
            2
        );
        let left = evidence(&s, "claude-code").await.unwrap();
        assert_eq!(left.len(), 2);
        assert!(left.iter().all(|e| e.evidence == "introspection"));
    }

    #[tokio::test]
    async fn degraded_context_delivery_is_recorded_as_such() {
        // D19a: a degraded delivery establishes the capability and says so.
        let s = store().await;
        upsert_agent(&s, &agent_row("claude-code")).await.unwrap();
        record_evidence(
            &s,
            &CapabilityEvidence {
                agent: "claude-code".into(),
                capability: "context_at_session_open".into(),
                evidence: "observation".into(),
                established_at: now(),
                agent_version: Some("2.1.220".into()),
                degraded: Some(true),
            },
        )
        .await
        .unwrap();
        let e = evidence(&s, "claude-code").await.unwrap();
        assert_eq!(e[0].degraded, Some(true));
    }

    #[tokio::test]
    async fn integration_state_creates_no_outbox_rows() {
        // FR-183, SC-120: no outbox entity type, and the enqueue path is never
        // called for any of this.
        let s = store().await;
        upsert_agent(&s, &agent_row("codex")).await.unwrap();
        bind(
            &s,
            "codex",
            &resource("mcp", "/home/dev/.codex/config.toml"),
        )
        .await
        .unwrap();
        record_evidence(
            &s,
            &CapabilityEvidence {
                agent: "codex".into(),
                capability: "mcp".into(),
                evidence: "introspection".into(),
                established_at: now(),
                agent_version: None,
                degraded: None,
            },
        )
        .await
        .unwrap();

        let count: i64 = sqlx::query("SELECT COUNT(*) AS n FROM outbox")
            .fetch_one(s.pool())
            .await
            .unwrap()
            .get("n");
        assert_eq!(count, 0, "integration state reached the outbox");
    }
}

/// An ownership migration in flight (FR-228).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationState {
    pub id: Uuid,
    pub agent: String,
    pub kind: String,
    pub source_owner: String,
    pub source_scope: String,
    pub source_location: String,
    pub target_owner: String,
    pub target_scope: String,
    pub target_location: String,
    pub phase: String,
    pub overlap_permitted: bool,
    pub started_at: String,
    /// Redacted; never carries file content.
    pub last_error: Option<String>,
}

/// Metadata for content preserved before a forced repair (FR-222, D39).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryArtifact {
    pub id: Uuid,
    pub agent: String,
    pub kind: String,
    pub source_path: String,
    pub artifact_path: String,
    pub content_hash: String,
    pub created_at: String,
}

fn migration_from_row(r: &sqlx::sqlite::SqliteRow) -> MigrationState {
    MigrationState {
        id: Uuid::parse_str(&r.get::<String, _>("id")).unwrap_or_else(|_| Uuid::now_v7()),
        agent: r.get("agent"),
        kind: r.get("kind"),
        source_owner: r.get("source_owner"),
        source_scope: r.get("source_scope"),
        source_location: r.get("source_location"),
        target_owner: r.get("target_owner"),
        target_scope: r.get("target_scope"),
        target_location: r.get("target_location"),
        phase: r.get("phase"),
        overlap_permitted: r.get::<i64, _>("overlap_permitted") != 0,
        started_at: r.get("started_at"),
        last_error: r.get("last_error"),
    }
}

/// Begin an ownership migration.
///
/// At most one per `(agent, kind)`: a second attempt while one is in flight is
/// `migration_in_progress`, not a second row.
pub async fn start_migration(store: &Store, m: &MigrationState) -> Result<()> {
    sqlx::query(
        "INSERT INTO migration_states
            (id, agent, kind, source_owner, source_scope, source_location,
             target_owner, target_scope, target_location, phase, overlap_permitted,
             started_at, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL)
         ON CONFLICT (agent, kind) DO NOTHING",
    )
    .bind(m.id.to_string())
    .bind(&m.agent)
    .bind(&m.kind)
    .bind(&m.source_owner)
    .bind(&m.source_scope)
    .bind(&m.source_location)
    .bind(&m.target_owner)
    .bind(&m.target_scope)
    .bind(&m.target_location)
    .bind(&m.phase)
    .bind(m.overlap_permitted as i64)
    .bind(&m.started_at)
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Move a migration to its next phase, or record its failure.
///
/// A failed migration keeps its row so the developer can resume or reverse it
/// rather than being left with something indistinguishable from accidental
/// duplication (FR-228).
pub async fn set_migration_phase(
    store: &Store,
    agent: &str,
    kind: &str,
    phase: &str,
    last_error: Option<&str>,
) -> Result<()> {
    let redacted = last_error.map(cairn_core::redact::redact);
    sqlx::query(
        "UPDATE migration_states SET phase = ?3, last_error = ?4
         WHERE agent = ?1 AND kind = ?2",
    )
    .bind(agent)
    .bind(kind)
    .bind(phase)
    .bind(redacted)
    .execute(store.pool())
    .await?;
    Ok(())
}

pub async fn migration(store: &Store, agent: &str, kind: &str) -> Result<Option<MigrationState>> {
    let row = sqlx::query("SELECT * FROM migration_states WHERE agent = ?1 AND kind = ?2")
        .bind(agent)
        .bind(kind)
        .fetch_optional(store.pool())
        .await?;
    Ok(row.as_ref().map(migration_from_row))
}

pub async fn list_migrations(store: &Store) -> Result<Vec<MigrationState>> {
    let rows = sqlx::query("SELECT * FROM migration_states ORDER BY agent, kind")
        .fetch_all(store.pool())
        .await?;
    Ok(rows.iter().map(migration_from_row).collect())
}

/// The migration completed: exactly one owner and one resource remain.
pub async fn clear_migration(store: &Store, agent: &str, kind: &str) -> Result<()> {
    sqlx::query("DELETE FROM migration_states WHERE agent = ?1 AND kind = ?2")
        .bind(agent)
        .bind(kind)
        .execute(store.pool())
        .await?;
    Ok(())
}

/// Record a preserved recovery artifact, keeping the ten most recent per
/// `(agent, kind)`.
///
/// Metadata only: the artifact's content lives on disk and is never logged,
/// never entered into diagnostics, and never stored here (FR-239).
pub async fn record_recovery_artifact(store: &Store, a: &RecoveryArtifact) -> Result<()> {
    sqlx::query(
        "INSERT INTO recovery_artifacts
            (id, agent, kind, source_path, artifact_path, content_hash, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(a.id.to_string())
    .bind(&a.agent)
    .bind(&a.kind)
    .bind(&a.source_path)
    .bind(&a.artifact_path)
    .bind(&a.content_hash)
    .bind(&a.created_at)
    .execute(store.pool())
    .await?;
    sqlx::query(
        "DELETE FROM recovery_artifacts
         WHERE agent = ?1 AND kind = ?2 AND id NOT IN (
             SELECT id FROM recovery_artifacts
             WHERE agent = ?1 AND kind = ?2
             ORDER BY created_at DESC LIMIT 10
         )",
    )
    .bind(&a.agent)
    .bind(&a.kind)
    .execute(store.pool())
    .await?;
    Ok(())
}
