//! Contention diagnostics and legacy-bundle table inventory.

use std::io::Write;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Begin,
    Body,
    Commit,
    Rollback,
    Autocommit,
    Unknown,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Begin => "begin_immediate",
            Self::Body => "statement",
            Self::Commit => "commit",
            Self::Rollback => "rollback",
            Self::Autocommit => "autocommit",
            Self::Unknown => "unknown",
        }
    }
}

fn sink() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        std::env::var("CAIRN_CONTENTION_LOG")
            .ok()
            .filter(|p| !p.is_empty())
    })
    .as_deref()
}

pub fn enabled() -> bool {
    sink().is_some()
}

pub fn codes(error: &sqlx::Error) -> Option<(i64, i64)> {
    let sqlx::Error::Database(database) = error else {
        return None;
    };
    let extended = database.code()?.parse().ok()?;
    Some((extended, extended & 0xff))
}

pub fn is_contention(error: &sqlx::Error) -> bool {
    matches!(codes(error), Some((_, 5 | 6)))
}

pub fn record(op: &str, stage: Stage, entity: &str, attempt: u32, error: &sqlx::Error) {
    let (Some(path), Some((extended, primary))) = (sink(), codes(error)) else {
        return;
    };
    let line = format!("op={op} stage={} entity={entity} extended={extended} primary={primary} attempt={attempt}\n", stage.as_str());
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

pub(crate) struct CategorySpec {
    pub(crate) category: &'static str,
    pub(crate) tables: &'static [&'static str],
}

/// Legacy tables remain exportable until fresh-DB setup cutover; they are not runtime authority.
pub(crate) const CATEGORIES: &[CategorySpec] = &[
    CategorySpec {
        category: "edge",
        tables: &[
            "users",
            "projects",
            "sessions",
            "agent_integrations",
            "manager_integrations",
            "installed_resources",
            "resource_bindings",
            "capability_evidence",
            "recovery_artifacts",
            "event_spool",
            "command_spool",
            "session_event_seq",
            "command_seq",
            "capture_disposition_counts",
        ],
    },
    CategorySpec {
        category: "removed_feature",
        tables: &[
            "memories",
            "memory_evidence",
            "memory_evidence_facts",
            "memory_relations",
            "evidence_facts",
            "verification_runs",
            "reusable_patterns",
            "pattern_applications",
            "personal_knowledge",
            "personal_knowledge_applicability",
            "personal_knowledge_relations",
            "team_knowledge",
            "team_knowledge_applicability",
            "team_knowledge_relations",
            "cached_patterns",
            "observations",
            "handoffs",
            "continuity_checkpoints",
            "outbox",
            "sync_cursor",
            "sync_meta",
            "sync_deferred",
            "authority_mode",
            "migration_state",
            "retained_local",
            "legacy_pattern_claims",
            "writer_identity",
            "project_traits",
        ],
    },
];
