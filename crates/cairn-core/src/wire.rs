//! IPC and sync wire types (contracts/agent-integration.md, contracts/server-api.md).
//!
//! The CLI, the MCP server and the daemon all speak these types over a local
//! socket as newline-delimited JSON. The same envelope shape is what `--json`
//! prints.

use crate::domain::*;
use crate::lifecycle::CanonicalLifecycleEvent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable error codes (contracts/mcp-tools.md).
///
/// There is deliberately no `budget_exceeded`: a briefing is truncated to fit,
/// never rejected for size (FR-029).
pub mod codes {
    pub const NOT_A_REPOSITORY: &str = "not_a_repository";
    pub const NO_ACTIVE_SESSION: &str = "no_active_session";
    pub const AMBIGUOUS_SESSION: &str = "ambiguous_session";
    pub const NOT_FOUND: &str = "not_found";
    pub const INVALID_REQUEST: &str = "invalid_request";
    pub const STORAGE_UNAVAILABLE: &str = "storage_unavailable";
    pub const DAEMON_UNAVAILABLE: &str = "daemon_unavailable";
    pub const NOT_LINKED: &str = "not_linked";
    pub const SERVER_UNAVAILABLE: &str = "server_unavailable";
    pub const UNAUTHORIZED: &str = "unauthorized";

    // Feature 002's integration codes (contracts/integration-cli.md §Error
    // codes). A closed set, not ad-hoc strings (FR-167). Every one of them
    // exits 1; `daemon_unavailable` and `storage_unavailable` keep exit 2.
    //
    // Configuration operations fail loudly — none of them is fail-soft
    // (FR-196). Only the hook path fails soft, and it is unchanged (FR-193).
    pub const AGENT_NOT_DETECTED: &str = "agent_not_detected";
    pub const AGENT_UNSUPPORTED: &str = "agent_unsupported";
    pub const MALFORMED_CONFIG: &str = "malformed_config";
    pub const PERMISSION_DENIED: &str = "permission_denied";
    pub const DAMAGED_MARKERS: &str = "damaged_markers";
    pub const RESOURCE_MODIFIED: &str = "resource_modified";
    pub const DUPLICATE_RESOURCE: &str = "duplicate_resource";
    pub const CONFLICTING_OWNER: &str = "conflicting_owner";
    pub const INSTALLED_NOT_ACTIVATED: &str = "installed_not_activated";
    pub const MIGRATION_IN_PROGRESS: &str = "migration_in_progress";
    pub const MIGRATION_UNSAFE: &str = "migration_unsafe";
    pub const MANAGER_ACTION_REQUIRED: &str = "manager_action_required";
    pub const VERIFICATION_FAILED: &str = "verification_failed";
    pub const CONFIRMATION_REQUIRED: &str = "confirmation_required";
    /// A manager Skill import was requested from a build whose embedded Skill
    /// revision has no published `skill-release` branch. Emitting an
    /// unpublished ref would make CC Switch silently install `main`.
    pub const UNPUBLISHED_SKILL_REF: &str = "unpublished_skill_ref";
    pub const PARTIAL_APPLY: &str = "partial_apply";

    /// Every Feature 002 code, for the exit-code mapping and its test.
    pub const INTEGRATION_CODES: &[&str] = &[
        AGENT_NOT_DETECTED,
        AGENT_UNSUPPORTED,
        MALFORMED_CONFIG,
        PERMISSION_DENIED,
        DAMAGED_MARKERS,
        RESOURCE_MODIFIED,
        DUPLICATE_RESOURCE,
        CONFLICTING_OWNER,
        INSTALLED_NOT_ACTIVATED,
        MIGRATION_IN_PROGRESS,
        MIGRATION_UNSAFE,
        MANAGER_ACTION_REQUIRED,
        VERIFICATION_FAILED,
        CONFIRMATION_REQUIRED,
        UNPUBLISHED_SKILL_REF,
        PARTIAL_APPLY,
    ];

    // -----------------------------------------------------------------------
    // Feature 003 (FR-499). Added to the same stable set, with the same exit
    // mapping. Deliberately **no** `budget_exceeded`: a briefing is truncated
    // to fit and never rejected for size (FR-445).
    // -----------------------------------------------------------------------

    // Knowledge (`contracts/knowledge.md` §Error codes)
    /// The proposed key did not normalize; the memory was stored free-form.
    pub const INVALID_TOPIC_KEY: &str = "invalid_topic_key";
    pub const VALUE_WITHOUT_TOPIC: &str = "value_without_topic";
    pub const SUBJECT_NOT_FOUND: &str = "subject_not_found";
    pub const NOT_CONFLICTED: &str = "not_conflicted";
    /// The requested relation contradicts an existing one — a mutual
    /// supersession, for instance.
    pub const RELATION_CONFLICT: &str = "relation_conflict";
    /// The write succeeded; the relation exceeded `reconcile_members_max`.
    pub const RECONCILIATION_DEFERRED: &str = "reconciliation_deferred";
    /// The write succeeded and agrees on the value with a named existing
    /// member while differing in content. Not a failure — the prompt for an
    /// explicit decision (FR-327).
    pub const CORROBORATING_MEMBER: &str = "corroborating_member";

    // Evidence and verification (`contracts/evidence-verification.md`)
    pub const EVIDENCE_EXCLUDED: &str = "evidence_excluded";
    pub const EVIDENCE_OUTSIDE_WORKTREE: &str = "evidence_outside_worktree";
    pub const EVIDENCE_TOO_LARGE: &str = "evidence_too_large";
    pub const ABSOLUTE_LOCATOR: &str = "absolute_locator";
    pub const VERIFIER_UNAVAILABLE: &str = "verifier_unavailable";
    /// The check ran and could not establish either outcome (FR-366).
    pub const VERIFICATION_INCONCLUSIVE: &str = "verification_inconclusive";
    /// Attested evidence was offered where a deterministic check is required —
    /// a criterion's verification, or promotion (FR-370).
    pub const ATTESTED_NOT_SUFFICIENT: &str = "attested_not_sufficient";
    /// An imported verification was offered for a criterion; readiness is a
    /// local claim (FR-368).
    pub const IMPORTED_NOT_SUFFICIENT: &str = "imported_not_sufficient";
    /// The bounded pass hit a cap; remaining work is queued.
    pub const VERIFY_PASS_YIELDED: &str = "verify_pass_yielded";

    // Continuity and context (`contracts/continuity-context.md`)
    pub const PIN_BUDGET_EXHAUSTED: &str = "pin_budget_exhausted";
    pub const CHECKPOINT_NOT_FOUND: &str = "checkpoint_not_found";
    pub const CHECKPOINT_UNRESOLVABLE: &str = "checkpoint_unresolvable";
    /// A relevant path could not be fingerprinted — excluded, unreadable, or
    /// over the cap. Reported per path, never as unchanged (FR-432).
    pub const PATH_NOT_FINGERPRINTABLE: &str = "path_not_fingerprintable";
    pub const NO_BOUNDARY_RECORD: &str = "no_boundary_record";

    // Task work state (`contracts/task-model.md`)
    pub const REVISION_CONFLICT: &str = "revision_conflict";
    pub const CRITERION_NOT_FOUND: &str = "criterion_not_found";
    pub const BLOCKER_NOT_FOUND: &str = "blocker_not_found";
    pub const BLOCKER_ALREADY_CLEARED: &str = "blocker_already_cleared";
    pub const CRITERION_WAIVED: &str = "criterion_waived";

    // The ten promotion refusals, in the gate's fixed order so the reported
    // reason is stable (`contracts/patterns.md` §The promotion gate).
    pub const SOURCE_NOT_ACTIVE: &str = "source_not_active";
    pub const SOURCE_UNVERIFIED: &str = "source_unverified";
    pub const NO_EVIDENCE: &str = "no_evidence";
    pub const LOCAL_ONLY_MEMORY: &str = "local_only_memory";
    pub const SOURCE_CONFLICTED: &str = "source_conflicted";
    pub const NOT_TRANSFERABLE: &str = "not_transferable";
    pub const POSSIBLE_SECRET: &str = "possible_secret";
    pub const PROJECT_IDENTIFYING: &str = "project_identifying";
    pub const INSUFFICIENT_SPECIFICITY: &str = "insufficient_specificity";
    pub const DUPLICATE_PATTERN: &str = "duplicate_pattern";
    pub const PATTERN_NOT_FOUND: &str = "pattern_not_found";
    pub const OUTCOME_ALREADY_RECORDED: &str = "outcome_already_recorded";

    /// The ten gate refusals **in gate order**. The order is the contract: it
    /// is what makes the reported reason stable when a candidate violates more
    /// than one check.
    pub const PROMOTION_REFUSALS: &[&str] = &[
        SOURCE_NOT_ACTIVE,
        SOURCE_UNVERIFIED,
        NO_EVIDENCE,
        LOCAL_ONLY_MEMORY,
        SOURCE_CONFLICTED,
        NOT_TRANSFERABLE,
        POSSIBLE_SECRET,
        PROJECT_IDENTIFYING,
        INSUFFICIENT_SPECIFICITY,
        DUPLICATE_PATTERN,
    ];

    /// Feature 003 codes that are **not failures**.
    ///
    /// Each rides an `ok: true` envelope in a `notes` array, because the
    /// operation succeeded and the note is what the caller needs to know:
    /// FR-312 requires a memory with an unrepresentable topic key to be stored
    /// regardless, FR-366 makes an inconclusive check an outcome rather than an
    /// error, and FR-435 makes partial continuity a result.
    pub const FEATURE_003_NOTES: &[&str] = &[
        INVALID_TOPIC_KEY,
        RECONCILIATION_DEFERRED,
        CORROBORATING_MEMBER,
        VERIFICATION_INCONCLUSIVE,
        VERIFY_PASS_YIELDED,
        CHECKPOINT_UNRESOLVABLE,
        PATH_NOT_FINGERPRINTABLE,
    ];

    /// Every Feature 003 code, for the exit-code mapping and its test.
    pub const INTELLIGENCE_CODES: &[&str] = &[
        INVALID_TOPIC_KEY,
        VALUE_WITHOUT_TOPIC,
        SUBJECT_NOT_FOUND,
        NOT_CONFLICTED,
        RELATION_CONFLICT,
        RECONCILIATION_DEFERRED,
        CORROBORATING_MEMBER,
        EVIDENCE_EXCLUDED,
        EVIDENCE_OUTSIDE_WORKTREE,
        EVIDENCE_TOO_LARGE,
        ABSOLUTE_LOCATOR,
        VERIFIER_UNAVAILABLE,
        VERIFICATION_INCONCLUSIVE,
        ATTESTED_NOT_SUFFICIENT,
        IMPORTED_NOT_SUFFICIENT,
        VERIFY_PASS_YIELDED,
        PIN_BUDGET_EXHAUSTED,
        CHECKPOINT_NOT_FOUND,
        CHECKPOINT_UNRESOLVABLE,
        PATH_NOT_FINGERPRINTABLE,
        NO_BOUNDARY_RECORD,
        REVISION_CONFLICT,
        CRITERION_NOT_FOUND,
        BLOCKER_NOT_FOUND,
        BLOCKER_ALREADY_CLEARED,
        CRITERION_WAIVED,
        SOURCE_NOT_ACTIVE,
        SOURCE_UNVERIFIED,
        NO_EVIDENCE,
        LOCAL_ONLY_MEMORY,
        SOURCE_CONFLICTED,
        NOT_TRANSFERABLE,
        POSSIBLE_SECRET,
        PROJECT_IDENTIFYING,
        INSUFFICIENT_SPECIFICITY,
        DUPLICATE_PATTERN,
        PATTERN_NOT_FOUND,
        OUTCOME_ALREADY_RECORDED,
    ];
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WireError {
    pub code: String,
    pub message: String,
}

impl WireError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
    pub fn not_found(what: impl Into<String>) -> Self {
        Self::new(codes::NOT_FOUND, what)
    }
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::new(codes::INVALID_REQUEST, msg)
    }
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for WireError {}

/// The stable `--json` envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

impl Envelope {
    pub fn ok(data: serde_json::Value) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }
    pub fn err(error: WireError) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error),
        }
    }
    pub fn into_result(self) -> Result<serde_json::Value, WireError> {
        if self.ok {
            Ok(self.data.unwrap_or(serde_json::Value::Null))
        } else {
            Err(self
                .error
                .unwrap_or_else(|| WireError::new("internal", "unspecified failure")))
        }
    }
}

/// Why a briefing is being assembled (contracts/mcp-tools.md).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextReason {
    SessionStart,
    Continuation,
    Refresh,
}

/// What a capture hook observed, before the daemon filters and stores it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationInput {
    pub kind: ObservationType,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub outcome: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
    /// The raw vendor tool name, kept as bounded provenance only (FR-122,
    /// D36). Normalized and truncated at construction; never consulted by
    /// ranking, handoff synthesis or context assembly (FR-121).
    #[serde(default)]
    pub vendor_tool: Option<String>,
}

/// A request that does not say defaults to waiting, which is Feature 001's
/// behavior and the safe answer for anything that is not a budgeted hook.
fn default_wait_for_handoff() -> bool {
    true
}

/// What to do with an ownership migration record.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MigrationAction {
    Start,
    Advance,
    Fail,
    Clear,
    Read,
}

/// Which entity a delete targets (FR-052).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeleteTarget {
    Observation,
    Memory,
    Session,
    Handoff,
}

/// Search parameters for memory recall (FR-022, FR-023).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryQuery {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub scope: Option<MemoryScope>,
    #[serde(default)]
    pub scope_key: Option<String>,
    #[serde(default)]
    pub kind: Option<MemoryType>,
    #[serde(default)]
    pub state: Option<MemoryState>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Everything the daemon can be asked to do.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    DaemonStatus,
    DaemonShutdown,

    Init {
        cwd: String,
    },
    Status {
        cwd: String,
    },

    SessionStart {
        cwd: String,
        agent: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        task_id: Option<Uuid>,
    },
    SessionList {
        cwd: String,
    },
    SessionShow {
        cwd: String,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        agent_session_key: Option<String>,
    },
    SessionBindTask {
        cwd: String,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        agent_session_key: Option<String>,
        task_id: Uuid,
    },
    SessionEnd {
        cwd: String,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        agent_session_key: Option<String>,
        status: SessionStatus,
        #[serde(default)]
        reason: Option<String>,
        /// Whether the caller waits for the durable handoff.
        ///
        /// True for `cairn session end` from the command line, which keeps
        /// Feature 001's behavior — nothing holds a deadline over it. False
        /// for a hook-driven boundary, where the vendor's own handler budget
        /// does, and the seal is what is acknowledged (D22, FR-240).
        #[serde(default = "default_wait_for_handoff")]
        wait_for_handoff: bool,
    },
    /// `Stop`: a turn boundary. Never ends the session (FR-032, D16).
    TurnCheckpoint {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
    },

    /// One canonical lifecycle event, from an adapter (FR-112).
    ///
    /// The daemon's single lifecycle entry point. No vendor event name,
    /// payload shape or ordering assumption reaches it: an adapter translates
    /// first, and this is what it translates into.
    CanonicalEvent {
        event: CanonicalLifecycleEvent,
        /// Whether the caller waits for a boundary's durable handoff. False
        /// for a hook under a vendor deadline (D22).
        #[serde(default)]
        wait_for_handoff: bool,
        /// The token budget for the context a `session_opened` delivers.
        #[serde(default)]
        token_budget: Option<usize>,
    },

    /// Read the local integration record for this machine (FR-182).
    ///
    /// Local only: nothing here has an outbox entity type and none of it ever
    /// reaches the server (FR-183, FR-184).
    IntegrationSnapshot {
        cwd: String,
    },
    /// Record an agent's integration row.
    IntegrationUpsertAgent {
        cwd: String,
        agent: String,
        adapter_version: i64,
        detected_version: Option<String>,
        compatibility: String,
        level: String,
        completion_guarantee: String,
    },
    /// Record that this agent depends on a physical resource.
    IntegrationBind {
        cwd: String,
        agent: String,
        kind: String,
        owner: String,
        scope: String,
        location: String,
        #[serde(default)]
        content_hash: Option<String>,
        #[serde(default)]
        artifact_schema: Option<i64>,
        #[serde(default)]
        artifact_revision: Option<String>,
        activation: String,
        #[serde(default)]
        container_single_line: bool,
        #[serde(default)]
        created_container: bool,
    },
    /// Drop this agent's dependency on one resource kind.
    ///
    /// The resource itself goes only when no binding remains (FR-243).
    IntegrationUnbind {
        cwd: String,
        agent: String,
        kind: String,
    },
    /// Remove an agent's record, but only once its last binding is gone
    /// (FR-244).
    IntegrationForgetAgent {
        cwd: String,
        agent: String,
    },
    /// Record that a capability was established here (FR-242, D19a).
    IntegrationEvidence {
        cwd: String,
        agent: String,
        capability: String,
        evidence: String,
        #[serde(default)]
        agent_version: Option<String>,
        #[serde(default)]
        degraded: Option<bool>,
    },
    /// Discard observation evidence a detected version change invalidated
    /// (FR-245).
    IntegrationInvalidateEvidence {
        cwd: String,
        agent: String,
        #[serde(default)]
        detected_version: Option<String>,
    },
    /// Record, resume, abort or complete an ownership migration (FR-228).
    IntegrationMigration {
        cwd: String,
        agent: String,
        kind: String,
        action: MigrationAction,
        #[serde(default)]
        source_owner: Option<String>,
        #[serde(default)]
        source_scope: Option<String>,
        #[serde(default)]
        source_location: Option<String>,
        #[serde(default)]
        target_owner: Option<String>,
        #[serde(default)]
        target_scope: Option<String>,
        #[serde(default)]
        target_location: Option<String>,
        #[serde(default)]
        overlap_permitted: bool,
        #[serde(default)]
        phase: Option<String>,
        #[serde(default)]
        last_error: Option<String>,
    },
    /// Record a preserved recovery artifact's metadata (FR-222).
    IntegrationRecovery {
        cwd: String,
        agent: String,
        kind: String,
        source_path: String,
        artifact_path: String,
        content_hash: String,
    },

    Observe {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        observation: ObservationInput,
    },

    Context {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        reason: Option<ContextReason>,
        #[serde(default)]
        token_budget: Option<usize>,
    },

    HandoffGenerate {
        cwd: String,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        agent_session_key: Option<String>,
        trigger: HandoffTrigger,
    },
    HandoffLatest {
        cwd: String,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        agent_session_key: Option<String>,
    },
    HandoffAnnotate {
        cwd: String,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(default)]
        agent_session_key: Option<String>,
        note: String,
    },

    TaskList {
        cwd: String,
        #[serde(default)]
        status: Option<TaskStatus>,
    },
    TaskGet {
        cwd: String,
        task_id: Uuid,
    },
    TaskCreate {
        cwd: String,
        title: String,
        goal: String,
        #[serde(default)]
        acceptance_criteria: Vec<String>,
    },
    TaskUpdate {
        cwd: String,
        task_id: Uuid,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        goal: Option<String>,
        #[serde(default)]
        acceptance_criteria: Option<Vec<String>>,
        #[serde(default)]
        status: Option<TaskStatus>,
    },

    MemoryCreate {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        /// The session to attribute this to, when the key is not to hand.
        #[serde(default)]
        session_id: Option<Uuid>,
        kind: MemoryType,
        #[serde(default)]
        scope: Option<MemoryScope>,
        #[serde(default)]
        scope_key: Option<String>,
        content: String,
        /// Zero or more. Never fabricated (FR-019).
        #[serde(default)]
        evidence_observation_ids: Vec<Uuid>,
        #[serde(default)]
        local_only: bool,
    },
    MemorySupersede {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        memory_id: Uuid,
        kind: MemoryType,
        #[serde(default)]
        scope: Option<MemoryScope>,
        #[serde(default)]
        scope_key: Option<String>,
        content: String,
        #[serde(default)]
        evidence_observation_ids: Vec<Uuid>,
        #[serde(default)]
        local_only: bool,
    },
    MemoryForget {
        cwd: String,
        memory_id: Uuid,
    },
    MemoryGet {
        cwd: String,
        memory_id: Uuid,
    },
    MemorySearch {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(flatten)]
        query: MemoryQuery,
    },

    PrivacyExclude {
        cwd: String,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        command: Option<String>,
    },
    PrivacyUnexclude {
        cwd: String,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        command: Option<String>,
    },
    PrivacyList {
        cwd: String,
    },

    Delete {
        cwd: String,
        target: DeleteTarget,
        id: Uuid,
        #[serde(default)]
        with_memories: bool,
    },

    Link {
        cwd: String,
        #[serde(default)]
        server_project_id: Option<Uuid>,
        #[serde(default)]
        create: bool,
    },
    Unlink {
        cwd: String,
    },
    AuthTokenSet {
        token: String,
        #[serde(default)]
        server_url: Option<String>,
    },
    AuthLogout,
    AuthStatus,
    SyncStatus {
        cwd: String,
    },
    SyncNow {
        cwd: String,
    },
}

/// Repository and project state for `cairn status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusPayload {
    pub project: ProjectSummary,
    pub repository: RepositoryState,
    pub worktree_path: String,
    pub sessions: Vec<SessionSummary>,
    pub integration_mode: String,
    pub daemon: String,
    pub observation_count: i64,
    pub memory_count: i64,
    /// The server a linked project syncs to, when one is configured.
    #[serde(default)]
    pub server_url: Option<String>,
    /// Whether a credential is stored for it.
    #[serde(default)]
    pub authenticated: bool,
    /// The build answering this, so a bug report can name it.
    #[serde(default)]
    pub version: Option<String>,
    /// The local database's schema version, so doctor can report it without
    /// linking the store into the CLI.
    #[serde(default)]
    pub local_schema_version: i64,
    /// Boundaries that acknowledged but have not produced their handoff yet.
    ///
    /// A terminal session never sits silently owing one: this is what makes
    /// the debt visible (FR-240 clause 3).
    #[serde(default)]
    pub sessions_awaiting_handoff: i64,
    /// Boundaries whose synthesis has failed, with the redacted reason. They
    /// stay retryable and actionable; this is not a terminal outcome.
    #[serde(default)]
    pub handoff_synthesis_failures: Vec<HandoffFailure>,
}

/// One boundary still owing a handoff, and why (FR-240 clause 3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffFailure {
    pub session_id: Uuid,
    /// Redacted and bounded; never file or conversation content.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: Uuid,
    pub name: String,
    pub linked: bool,
    pub server_project_id: Option<Uuid>,
}

impl From<&Project> for ProjectSummary {
    fn from(p: &Project) -> Self {
        Self {
            id: p.id,
            name: p.name.clone(),
            linked: p.linked,
            server_project_id: p.server_project_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: Uuid,
    pub status: SessionStatus,
    pub agent: String,
    pub branch: String,
    pub commit: Option<String>,
    pub task_id: Option<Uuid>,
    pub previous_session_id: Option<Uuid>,
    pub worktree_path: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub last_turn_ended_at: Option<DateTime<Utc>>,
    /// Reported only. Idleness never reclassifies a session (FR-009).
    pub idle_seconds: i64,
}

impl SessionSummary {
    pub fn from_session(s: &Session, now: DateTime<Utc>) -> Self {
        Self {
            id: s.id,
            status: s.status,
            agent: s.agent.clone(),
            branch: s.branch.clone(),
            commit: s.commit_sha.clone(),
            task_id: s.task_id,
            previous_session_id: s.previous_session_id,
            worktree_path: s.worktree_path.clone(),
            started_at: s.started_at,
            ended_at: s.ended_at,
            last_turn_ended_at: s.last_turn_ended_at,
            idle_seconds: (now - s.last_event_at).num_seconds().max(0),
        }
    }
}

/// Provenance carried on every search result (FR-026).
///
/// A mandatory session, zero or more observation ids, and a count that may be
/// zero. Observation *content* is resolved locally and never travels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub session_id: Uuid,
    /// The agent whose session produced this. US6 requires a retrieved item to
    /// name the agent *and* the session; the session id alone made a reader
    /// join by hand to find out which agent learned it. Optional so a record
    /// whose session has since been deleted still returns.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent: Option<String>,
    pub observation_ids: Vec<Uuid>,
    pub evidence_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub deleted_observation_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankInfo {
    pub scope_bucket: i64,
    pub relevance: f64,
    pub age_days: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryResult {
    pub id: Uuid,
    #[serde(rename = "type")]
    pub kind: MemoryType,
    pub scope: MemoryScope,
    pub scope_key: String,
    pub content: String,
    pub state: MemoryState,
    pub local_only: bool,
    pub superseded_by_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub provenance: Provenance,
    pub rank: RankInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPayload {
    pub results: Vec<MemoryResult>,
    pub total: usize,
}

/// The bounded briefing (FR-028).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Briefing {
    pub project: ProjectSummary,
    pub repository: RepositoryState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<BriefingTask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_handoff: Option<BriefingHandoff>,
    pub decisions: Vec<String>,
    pub known_failures: Vec<String>,
    pub memory: BriefingMemory,
    pub no_prior_history: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingTask {
    pub id: Uuid,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingHandoff {
    pub session_id: Uuid,
    pub next_step: String,
    pub remaining_work: Vec<String>,
    pub changed_files: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BriefingMemory {
    pub task: Vec<String>,
    pub branch: Vec<String>,
    pub project: Vec<String>,
}

/// A briefing plus its budget accounting.
///
/// `estimated_tokens` and `budget` are both denominated in Cairn-estimated
/// tokens, not any model's tokenizer (FR-029, D8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPayload {
    pub briefing: Briefing,
    pub estimated_tokens: usize,
    pub budget: usize,
    pub truncated: bool,
    pub omitted_sections: Vec<String>,
    /// True when the briefing could not be fully assembled in time (FR-046).
    pub degraded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusPayload {
    pub linked: bool,
    pub server_project_id: Option<Uuid>,
    pub server_url: Option<String>,
    pub pending: i64,
    pub failed: i64,
    pub last_success_at: Option<DateTime<Utc>>,
    pub failures: Vec<SyncFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncFailure {
    pub entity_type: OutboxEntityType,
    pub entity_id: Uuid,
    pub error: String,
}

// ---------------------------------------------------------------------------
// Server sync wire types (contracts/server-api.md)
// ---------------------------------------------------------------------------

/// One item in a sync batch. There is no observation variant, so a payload
/// carrying observation content cannot be constructed (FR-055, D9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncItem {
    pub idempotency_key: String,
    pub entity_type: OutboxEntityType,
    pub entity_id: Uuid,
    pub operation: OutboxOperation,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncBatch {
    pub project_id: Uuid,
    pub items: Vec<SyncItem>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncItemStatus {
    Applied,
    Duplicate,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncItemResult {
    pub idempotency_key: String,
    pub status: SyncItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncBatchResponse {
    pub results: Vec<SyncItemResult>,
}

/// Fields the server rejects on a session payload: local-only, always
/// (contracts/server-api.md).
pub const REJECTED_SESSION_FIELDS: &[&str] = &[
    "worktree_path",
    "agent_session_key",
    "daemon_run_id",
    "last_event_at",
    "last_turn_ended_at",
];

/// Fields that would carry observation content. Rejected everywhere (FR-055).
pub const REJECTED_OBSERVATION_FIELDS: &[&str] = &[
    "summary",
    "path",
    "command",
    "details",
    "observations",
    "outcome",
    "exit_code",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let e = Envelope::ok(serde_json::json!({"a": 1}));
        let s = serde_json::to_string(&e).unwrap();
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert!(back.ok);
        assert!(back.into_result().is_ok());
    }

    #[test]
    fn error_envelope_has_no_data_key() {
        let s = serde_json::to_string(&Envelope::err(WireError::not_found("task"))).unwrap();
        assert!(!s.contains("\"data\""));
        assert!(s.contains("not_found"));
    }

    #[test]
    fn request_is_tagged_by_op() {
        let r = Request::Status { cwd: "/tmp".into() };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"op\":\"status\""));
        let back: Request = serde_json::from_str(&s).unwrap();
        matches!(back, Request::Status { .. });
    }

    #[test]
    fn provenance_allows_zero_evidence() {
        // Manual MCP mode records memory with no observations (FR-019).
        let p = Provenance {
            session_id: crate::domain::new_id(),
            agent: None,
            observation_ids: vec![],
            evidence_count: 0,
            deleted_observation_ids: vec![],
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"evidence_count\":0"));
    }

    /// US6: a retrieved item names the agent *and* the session that produced
    /// it. The session id alone left a reader to join by hand to find out which
    /// agent learned the thing.
    #[test]
    fn provenance_names_the_producing_agent() {
        let p = Provenance {
            session_id: crate::domain::new_id(),
            agent: Some("claude-code".into()),
            observation_ids: vec![],
            evidence_count: 0,
            deleted_observation_ids: vec![],
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"agent\":\"claude-code\""));
        let back: Provenance = serde_json::from_str(&s).unwrap();
        assert_eq!(back.agent.as_deref(), Some("claude-code"));
    }

    /// A record whose origin session is gone still returns, and an older peer
    /// that never sends the field still deserializes.
    #[test]
    fn provenance_without_an_agent_round_trips() {
        let id = crate::domain::new_id();
        let json = format!(r#"{{"session_id":"{id}","observation_ids":[],"evidence_count":0}}"#);
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert!(back.agent.is_none());
        assert!(!serde_json::to_string(&back).unwrap().contains("agent"));
    }
}

#[cfg(test)]
mod feature_003_code_tests {
    use super::codes::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_code_is_unique_across_the_whole_stable_set() {
        // One set, not three: a code that means two things on two surfaces is
        // a code an agent cannot act on.
        let mut all: Vec<&str> = Vec::new();
        all.extend_from_slice(INTEGRATION_CODES);
        all.extend_from_slice(INTELLIGENCE_CODES);
        all.extend_from_slice(&[
            NOT_A_REPOSITORY,
            NO_ACTIVE_SESSION,
            AMBIGUOUS_SESSION,
            NOT_FOUND,
            INVALID_REQUEST,
            STORAGE_UNAVAILABLE,
            DAEMON_UNAVAILABLE,
            NOT_LINKED,
            SERVER_UNAVAILABLE,
            UNAUTHORIZED,
        ]);
        let unique: BTreeSet<&str> = all.iter().copied().collect();
        assert_eq!(
            unique.len(),
            all.len(),
            "duplicate code in the stable set: {all:?}"
        );
    }

    #[test]
    fn there_is_no_budget_exceeded_code() {
        // FR-445: a briefing is truncated to fit, never rejected for size. A
        // code for the rejection would invite one.
        assert!(!INTELLIGENCE_CODES.contains(&"budget_exceeded"));
        assert!(!INTELLIGENCE_CODES.iter().any(|c| c.contains("budget_exceeded")));
    }

    #[test]
    fn the_promotion_gate_order_is_the_contract() {
        // The reported reason must be stable when a candidate violates several
        // checks, and the order is what makes it so (FR-396, FR-397).
        assert_eq!(PROMOTION_REFUSALS.len(), 10);
        assert_eq!(PROMOTION_REFUSALS[0], SOURCE_NOT_ACTIVE);
        assert_eq!(PROMOTION_REFUSALS[1], SOURCE_UNVERIFIED);
        assert_eq!(
            PROMOTION_REFUSALS[6], POSSIBLE_SECRET,
            "the secret scan runs before the identifier scan"
        );
        assert_eq!(PROMOTION_REFUSALS[7], PROJECT_IDENTIFYING);
        assert_eq!(PROMOTION_REFUSALS[9], DUPLICATE_PATTERN);
        for code in PROMOTION_REFUSALS {
            assert!(INTELLIGENCE_CODES.contains(code), "{code} is not in the set");
        }
    }

    #[test]
    fn notes_are_a_subset_of_the_codes_and_are_not_failures() {
        for note in FEATURE_003_NOTES {
            assert!(
                INTELLIGENCE_CODES.contains(note),
                "{note} is a note for a code that does not exist"
            );
        }
        // The four the contracts call out explicitly as `ok: true`.
        for note in [
            INVALID_TOPIC_KEY,
            RECONCILIATION_DEFERRED,
            VERIFICATION_INCONCLUSIVE,
            CHECKPOINT_UNRESOLVABLE,
        ] {
            assert!(FEATURE_003_NOTES.contains(&note), "{note}");
        }
        // And a refusal is never one of them.
        for refusal in PROMOTION_REFUSALS {
            assert!(
                !FEATURE_003_NOTES.contains(refusal),
                "{refusal} must fail loudly"
            );
        }
    }

    #[test]
    fn the_two_strict_consumer_refusals_exist_and_are_distinct() {
        // They are told apart because they mean different things: one says
        // "an agent said so", the other says "another machine checked it".
        assert_ne!(ATTESTED_NOT_SUFFICIENT, IMPORTED_NOT_SUFFICIENT);
        assert!(INTELLIGENCE_CODES.contains(&ATTESTED_NOT_SUFFICIENT));
        assert!(INTELLIGENCE_CODES.contains(&IMPORTED_NOT_SUFFICIENT));
    }
}
