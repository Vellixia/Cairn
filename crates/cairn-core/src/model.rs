//! Core domain model: memories and their tiers/kinds.

use crate::OrgId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Human-memory-inspired tiers. New observations land in `Working` and are consolidated upward
/// (episodic events -> semantic facts -> procedural how-to) over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryTier {
    Working,
    Episodic,
    Semantic,
    Procedural,
}

impl MemoryTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryTier::Working => "working",
            MemoryTier::Episodic => "episodic",
            MemoryTier::Semantic => "semantic",
            MemoryTier::Procedural => "procedural",
        }
    }
}

impl std::str::FromStr for MemoryTier {
    type Err = crate::Error;
    fn from_str(s: &str) -> crate::Result<Self> {
        Ok(match s {
            "working" => Self::Working,
            "episodic" => Self::Episodic,
            "semantic" => Self::Semantic,
            "procedural" => Self::Procedural,
            other => {
                return Err(crate::Error::Invalid(format!(
                    "unknown memory tier: {other}"
                )))
            }
        })
    }
}

/// What a memory represents. Drives recall ranking and consolidation rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    Fact,
    Decision,
    Task,
    Preference,
    Gotcha,
    Note,
}

impl MemoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryKind::Fact => "fact",
            MemoryKind::Decision => "decision",
            MemoryKind::Task => "task",
            MemoryKind::Preference => "preference",
            MemoryKind::Gotcha => "gotcha",
            MemoryKind::Note => "note",
        }
    }
}

impl std::str::FromStr for MemoryKind {
    type Err = crate::Error;
    fn from_str(s: &str) -> crate::Result<Self> {
        Ok(match s {
            "fact" => Self::Fact,
            "decision" => Self::Decision,
            "task" => Self::Task,
            "preference" => Self::Preference,
            "gotcha" => Self::Gotcha,
            "note" => Self::Note,
            other => {
                return Err(crate::Error::Invalid(format!(
                    "unknown memory kind: {other}"
                )))
            }
        })
    }
}

/// A memory's visibility boundary (v0.8.0 Sprint 2). Recall always includes `Global`, plus
/// `Project`/`Session` memories whose `scope_id` matches the caller's current project/session -
/// this is an access-control boundary, distinct from `Memory::session_id` (which just records
/// provenance/diversity metadata and never hides a memory from other sessions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScopeType {
    #[default]
    Global,
    Project,
    Session,
}

impl ScopeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScopeType::Global => "global",
            ScopeType::Project => "project",
            ScopeType::Session => "session",
        }
    }
}

impl std::str::FromStr for ScopeType {
    type Err = crate::Error;
    fn from_str(s: &str) -> crate::Result<Self> {
        Ok(match s {
            "global" => Self::Global,
            "project" => Self::Project,
            "session" => Self::Session,
            other => {
                return Err(crate::Error::Invalid(format!(
                    "unknown scope type: {other}"
                )))
            }
        })
    }
}

/// The caller's current project/session context for scope-aware recall (v0.8.0 Sprint 2).
/// Built by `cairn-api` from `X-Cairn-Project`/`X-Cairn-Session` request headers today;
/// `cairn-mcp`'s stdio server has no per-request headers, so it passes `ScopeCtx::default()`
/// (Global-only) until Sprint 3's auto project detection wires a real value through.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeCtx {
    pub project_id: Option<String>,
    pub session_id: Option<String>,
}

/// A persisted memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub kind: MemoryKind,
    pub tier: MemoryTier,
    /// Short scannable label (web redesign v2 follow-up). `None` for memories written before
    /// this field existed or by callers that don't set one - the dashboard falls back to the
    /// first line of `content` as a display title in that case.
    #[serde(default)]
    pub title: Option<String>,
    pub content: String,
    /// Why this memory matters / the reasoning behind it, separate from the memory's own
    /// content - e.g. "chose X because Y broke last time". `None` when not provided.
    #[serde(default)]
    pub reasoning: Option<String>,
    pub concepts: Vec<String>,
    pub files: Vec<String>,
    pub session_id: Option<String>,
    pub importance: f32,
    pub access_count: i64,
    /// Tenant scope (v0.5.0 Sprint 19). Defaults to `OrgId::default()` for
    /// self-hosted installs; ignored when `Config::multi_tenant = false`.
    #[serde(default)]
    pub org_id: OrgId,
    #[serde(default)]
    pub suspicious: bool,
    /// Confidence score `[0.0, 1.0]` - evolves over time via the agentmemory reinforcement
    /// curve `c' = min(1.0, c + 0.1*(1.0 - c))` on each successful `recall` hit. Defaults to 0.5
    /// for new memories (neutral).
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    /// Pinned memories are kept around even when their confidence decays - they bypass the
    /// "candidate for review" cutoff so the user can keep a memory they've explicitly marked
    /// important. Defaults to false.
    #[serde(default)]
    pub pinned: bool,
    // -- v0.5.0 Sprint 3: provenance edges -----------------------------------------------
    /// Edges to memory ids this one was derived from (crystallized from, summarized, combined).
    #[serde(default)]
    pub derived_from: Vec<String>,
    /// Edges to memory ids this one contradicts (used to surface "these two memories disagree").
    #[serde(default)]
    pub contradicts: Vec<String>,
    /// Edges to memory ids this one supersedes (newer replaces older).
    #[serde(default)]
    pub supersedes: Vec<String>,
    /// Edges to file paths / symbols / projects this memory applies to (code-graph-style relevance).
    #[serde(default)]
    pub applies_to: Vec<String>,
    /// Edges to memory ids this one is related to (shared concepts, same topic).
    /// Auto-derived from shared `concepts` at write time (Bug #6 fix).
    #[serde(default)]
    pub related_to: Vec<String>,
    /// Edges to file paths this memory depends on (imported modules, required files).
    /// Auto-derived from simple import analysis of `files` at write time (Bug #6 fix).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Visibility boundary (v0.8.0 Sprint 2). Defaults to `Global` so pre-Sprint-2 records and
    /// callers that don't care about scoping keep working unchanged.
    #[serde(default)]
    pub scope_type: ScopeType,
    /// `None` for `Global`; the project id or session id for `Project`/`Session` respectively.
    #[serde(default)]
    pub scope_id: Option<String>,
    /// `Project`-scope promotion-worthiness, `[0.0, 1.0]` (v0.8.0 Sprint 5). Written by the
    /// `llm-intelligence` cron job's promotion-scoring pass; `0.0` for a memory that has never
    /// been scored. A score in `[0.70, 0.90]` surfaces the memory at
    /// `GET /api/memory/promotion-candidates` for human review.
    #[serde(default)]
    pub promo_score: f32,
    /// Once `true`, this memory is permanently excluded from promotion scoring/suggestions -
    /// set by `POST /api/memory/:id/dismiss-promotion` ("don't ask again") or implicitly once
    /// `POST /api/memory/:id/promote` has already promoted it. Defaults to `false`.
    #[serde(default)]
    pub promo_locked: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Typed provenance edge between memories (or memory -> file/symbol/project).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    DerivedFrom,
    Contradicts,
    Supersedes,
    AppliesTo,
    RelatedTo,
    DependsOn,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::DerivedFrom => "derived_from",
            EdgeKind::Contradicts => "contradicts",
            EdgeKind::Supersedes => "supersedes",
            EdgeKind::AppliesTo => "applies_to",
            EdgeKind::RelatedTo => "related_to",
            EdgeKind::DependsOn => "depends_on",
        }
    }
}

/// Default confidence for a brand-new memory. The agentmemory project's reinforcement curve
/// starts from a neutral midpoint so neither new memories nor old ones bias the recall mix.
fn default_confidence() -> f32 {
    0.5
}

/// Input for creating a memory. Optional fields fall back to sensible defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewMemory {
    pub content: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub kind: Option<MemoryKind>,
    #[serde(default)]
    pub tier: Option<MemoryTier>,
    #[serde(default)]
    pub concepts: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub importance: Option<f32>,
    #[serde(default)]
    pub suspicious: Option<bool>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub pinned: Option<bool>,
    /// Tenant scope for the new memory (v0.5.0 Sprint 19). If absent, the
    /// storage layer uses `OrgId::default()` so single-tenant installs don't
    /// need to pass it.
    #[serde(default)]
    pub org_id: Option<OrgId>,
    // v0.5.0 Sprint 3: optional edge inputs so callers can create a memory already wired
    // into the provenance graph (e.g. a crystallization step that knows which memories it's
    // summarizing).
    #[serde(default)]
    pub derived_from: Vec<String>,
    #[serde(default)]
    pub contradicts: Vec<String>,
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default)]
    pub applies_to: Vec<String>,
    #[serde(default)]
    pub related_to: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Visibility boundary for the new memory (v0.8.0 Sprint 2). Defaults to `Global`.
    #[serde(default)]
    pub scope_type: ScopeType,
    /// Required (and meaningful) only when `scope_type` is `Project` or `Session`.
    #[serde(default)]
    pub scope_id: Option<String>,
}

impl NewMemory {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            ..Default::default()
        }
    }

    /// Materialize into a full [`Memory`] with id and timestamps.
    pub fn into_memory(self) -> Memory {
        let now = Utc::now();
        Memory {
            id: Uuid::new_v4().to_string(),
            kind: self.kind.unwrap_or(MemoryKind::Note),
            tier: self.tier.unwrap_or(MemoryTier::Working),
            title: self.title,
            content: self.content,
            reasoning: self.reasoning,
            concepts: self.concepts,
            files: self.files,
            session_id: self.session_id,
            importance: self.importance.unwrap_or(0.5).clamp(0.0, 1.0),
            access_count: 0,
            org_id: self.org_id.unwrap_or_default(),
            suspicious: self.suspicious.unwrap_or(false),
            confidence: self.confidence.unwrap_or(0.5).clamp(0.0, 1.0),
            pinned: self.pinned.unwrap_or(false),
            derived_from: self.derived_from,
            contradicts: self.contradicts,
            supersedes: self.supersedes,
            applies_to: self.applies_to,
            related_to: self.related_to,
            depends_on: self.depends_on,
            scope_type: self.scope_type,
            scope_id: self.scope_id,
            promo_score: 0.0,
            promo_locked: false,
            created_at: now,
            updated_at: now,
        }
    }

    /// Materialize into a [`Memory`] tagged with `org_id` - used by the multi-tenant
    /// cairn-server to scope every memory at write time.
    pub fn into_memory_for_org(self, org_id: OrgId) -> Memory {
        let mut mem = self.into_memory();
        mem.org_id = org_id;
        mem
    }
}

/// A per-device access token for authenticating to a Cairn server.
/// `id` is the token identifier (stored in the backend). `token` is the opaque bearer value
/// (a signed JWT) returned to the user once, only at creation time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceToken {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    pub name: String,
    #[serde(default)]
    pub scope: TokenScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl DeviceToken {
    /// Metadata-only token (used by list/revoke flows that must never re-emit the bearer).
    pub fn meta(id: String, name: String, created_at: DateTime<Utc>) -> Self {
        Self {
            id,
            token: None,
            name,
            scope: TokenScope::Write,
            expires_at: None,
            last_used_at: None,
            created_at,
        }
    }
}

/// What a device token is allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TokenScope {
    /// Full access: read, write, revoke, rollback, admin operations.
    Admin,
    /// Read and write: create/read memories, checkpoints, preferences. Default scope.
    #[default]
    Write,
    /// Read-only: recall, wakeup, stats, expand, assemble.
    Read,
}

impl TokenScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenScope::Admin => "admin",
            TokenScope::Write => "write",
            TokenScope::Read => "read",
        }
    }

    /// Whether this scope allows the given HTTP method on the given path.
    pub fn allows(&self, method: &str, path: &str) -> bool {
        match self {
            TokenScope::Admin => true,
            TokenScope::Write => {
                // Write allows everything except destructive admin ops.
                !is_admin_only(path, method)
            }
            TokenScope::Read => {
                // Read-only: GET requests and POST to read-like endpoints.
                method == "GET"
                    || path == "/api/guard/verify"
                    || path == "/api/share/sanitize"
                    || path == "/api/context/assemble"
                    || path == "/api/shell/compress"
            }
        }
    }
}

fn is_admin_only(path: &str, method: &str) -> bool {
    method == "POST" && (path == "/api/guard/rollback" || path.starts_with("/api/pool"))
}

impl std::str::FromStr for TokenScope {
    type Err = crate::Error;
    fn from_str(s: &str) -> crate::Result<Self> {
        Ok(match s {
            "admin" => Self::Admin,
            "write" => Self::Write,
            "read" => Self::Read,
            other => {
                return Err(crate::Error::Invalid(format!(
                    "unknown token scope: {other}"
                )))
            }
        })
    }
}

impl std::fmt::Display for TokenScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
