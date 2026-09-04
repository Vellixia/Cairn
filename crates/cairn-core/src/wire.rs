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

    // What a server says when it cannot hold the work — as opposed to when it
    // will not (`contracts/privacy-sync.md` §Mixed versions, D81).
    //
    // These are **server** codes, and deliberately not part of
    // `INTELLIGENCE_CODES`: they never reach a CLI exit status. They classify a
    // rejection so the daemon can tell "upgrade the server and this delivers"
    // from "this will never be acceptable", which is the difference between
    // retained work and lost work (FR-418).
    pub const UNKNOWN_ENTITY_TYPE: &str = "unknown_entity_type";
    pub const UNKNOWN_FIELD: &str = "unknown_field";
    pub const SCHEMA_OLDER: &str = "schema_older";

    /// The refusals that mean *not yet*, rather than *never*.
    ///
    /// A rejection outside this set is a content rejection and stays a
    /// permanent failure exactly as it is today — which is what stops a privacy
    /// refusal from being retained as a pending delivery.
    pub const CAPABILITY_REFUSALS: &[&str] = &[UNKNOWN_ENTITY_TYPE, UNKNOWN_FIELD, SCHEMA_OLDER];

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

    // -----------------------------------------------------------------------
    // Feature 004 identity and administration
    // (`contracts/identity-administration.md` §10).
    //
    // These are **server** codes, named here so a caller compares against a
    // constant rather than a string literal it typed by hand — the daemon
    // never constructs them itself, only forwards what the server already
    // said (`cairnd::sync::decode`).
    // -----------------------------------------------------------------------
    pub const FORBIDDEN: &str = "forbidden";
    pub const PASSWORD_CHANGE_REQUIRED: &str = "password_change_required";
    pub const PASSWORD_TOO_SHORT: &str = "password_too_short";
    pub const INVALID_CREDENTIALS: &str = "invalid_credentials";
    pub const EMAIL_TAKEN: &str = "email_taken";
    pub const LAST_ADMIN: &str = "last_admin";
    /// The one route this feature's server code actually returns (patching or
    /// resetting the environment-named account), rather than the
    /// `env_admin_reset_refused` name the prose contract used before the code
    /// was written — `crates/cairn-server/src/api.rs`'s
    /// `environment_account_refusal()` is the source of truth.
    pub const ENVIRONMENT_ACCOUNT: &str = "environment_account";

    /// Every Feature 004 identity/administration code, for the uniqueness
    /// test below.
    pub const IDENTITY_CODES: &[&str] = &[
        FORBIDDEN,
        PASSWORD_CHANGE_REQUIRED,
        PASSWORD_TOO_SHORT,
        INVALID_CREDENTIALS,
        EMAIL_TAKEN,
        LAST_ADMIN,
        ENVIRONMENT_ACCOUNT,
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
    /// The agent is back from a compaction and is asking for its checkpoint.
    /// This is where a checkpoint is **restored**; it is never where one is
    /// written (`contracts/continuity-context.md` §When one is written).
    PostCompaction,
}

/// How much of the briefing to assemble
/// (`contracts/recall-composition.md` §5, FR-477).
///
/// `Minimum` is Level 0 only: `personal_notes` and `team_guidance` are never
/// fetched and never admitted, unconditionally — no importance hint, budget
/// outcome, or configuration overrides this. `Standard` is today's full
/// assembly, and it is also what an absent `depth` means, which is what makes
/// a caller that has never named this field see no change (FR-481).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextDepth {
    Minimum,
    Standard,
}

impl ContextDepth {
    pub fn is_minimum(&self) -> bool {
        matches!(self, ContextDepth::Minimum)
    }
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

    // Feature 003. Every one defaults to absent, so a caller that omits them
    // all receives Feature 001 behaviour exactly (FR-497).
    /// Exact or prefix match on the normalized subject identity. A trailing
    /// `.` makes it a prefix — `infrastructure.` matches every subject beneath
    /// it. Matched by SQL, never by FTS: a topic key is an identity, not text.
    #[serde(default)]
    pub topic_key: Option<String>,
    /// What was effective at an instant (FR-342).
    ///
    /// Reconstructs proposal effectiveness and explicit supersession
    /// intervals, which is the whole of what Cairn stores authoritatively. The
    /// lifecycle filter is relaxed when this is set, because a historical
    /// answer is precisely the set of proposals that are no longer current.
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    /// Only memories whose subject is `Conflicted`.
    #[serde(default)]
    pub conflicted: bool,
    /// Only memories whose subject is `Corroborated`.
    #[serde(default)]
    pub corroborated: bool,
    /// Filter by verification state. A `drifted` memory is still returned by
    /// default, because it stays lifecycle-`active` (FR-373).
    #[serde(default)]
    pub verification: Option<VerificationState>,
    /// Filter by what established the verification (FR-370).
    #[serde(default)]
    pub authority: Option<VerificationAuthority>,
    /// Also return signal-matched prior patterns, in a **separate** array.
    ///
    /// Never merged into `results`: a pattern is not this project's knowledge,
    /// and a caller that did not ask for one must not be handed one among its
    /// own memories (FR-406, SC-312).
    #[serde(default)]
    pub include_patterns: bool,

    /// Which knowledge domains to search. Absent means all three —
    /// `project`, `personal`, `team` — so a caller that has never named this
    /// field sees no change (FR-472). `results`/`total` still describe
    /// `project` alone; `personal`/`team` ride the response as sibling
    /// arrays the handler splices in, never merged into `results`
    /// (`contracts/recall-composition.md` §7).
    #[serde(default)]
    pub domains: Option<Vec<KnowledgeDomain>>,
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
        /// The Feature 005 events this same vendor event produced.
        ///
        /// Carried here rather than sent as a second request because the hot
        /// path is one hook invocation per tool call, and two connects and two
        /// writes where one would do is the largest cost Cairn adds to a
        /// session (SC-007). It also fixes the order for free: the lifecycle
        /// half creates or resumes the session the safe events bind to, and one
        /// request cannot arrive out of order with itself.
        #[serde(default)]
        capture: Option<crate::event::CaptureOutput>,
    },

    /// The session vocabulary a semantic signal must justify its tokens
    /// against (`contracts/extraction.md` §13.3).
    ///
    /// The hook asks for this because it holds the transient vendor text and
    /// the daemon holds the event stream, and neither can do the mapping
    /// alone. Sending the text to the daemon instead would put a prompt
    /// fragment across the capture-process boundary, which FR-730 forbids; so
    /// the derived set travels the other way. It discloses nothing new — every
    /// token in it is a file segment, a command verb, a test identifier or an
    /// established key already visible to anyone who can read the project.
    CaptureVocabulary {
        cwd: String,
        agent: String,
        agent_session_key: String,
    },

    /// Approved Feature 005 events from one vendor event, ready to spool.
    ///
    /// Additive to [`Request::CanonicalEvent`], which still drives sessions,
    /// handoffs and context delivery. This carries the richer safe-event
    /// stream, and carries no raw vendor payload: what arrives here has already
    /// been parsed, relativized, redacted and screened on the far side of the
    /// process boundary (FR-730, SC-741).
    CaptureEvents {
        cwd: String,
        agent: String,
        agent_session_key: String,
        #[serde(default)]
        output: crate::event::CaptureOutput,
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
        /// Return the selection diagnostics. Costs no budget when false, which
        /// is why it is opt-in rather than always present (FR-463).
        #[serde(default)]
        explain: bool,
        /// `minimum` excludes `personal_notes`/`team_guidance` entirely,
        /// unconditionally (FR-477). Absent means `standard`, today's full
        /// assembly (`contracts/recall-composition.md` §5).
        #[serde(default)]
        depth: Option<ContextDepth>,
        /// Which delivery point this retrieval is for, server-side
        /// (`contracts/retrieval-delivery.md` §1–§3): `"session_open"` |
        /// `"prompt_submit"`. Absent is an **explicit** pull —
        /// `cairn_context`/`cairn_search`'s companion call, and the CLI — which
        /// is also the safe default for any caller written before this field
        /// existed: nothing is pushed and nothing is ever reported
        /// `transmitted` on an absent trigger's behalf.
        #[serde(default)]
        trigger: Option<String>,
        /// `session_open` only: the vendor's own reason the session opened —
        /// `startup`/`resume`/`clear`/`compact`/`fork` — forwarded to
        /// `/api/retrieve` verbatim so the server can recognize a
        /// post-compaction restoration (`contracts/retrieval-delivery.md` §2).
        #[serde(default)]
        open_trigger: Option<String>,
    },

    /// Report what actually happened to a briefing `/api/retrieve` generated
    /// (`contracts/retrieval-delivery.md` §3, §6.2). Never sent for a
    /// `trigger` of `explicit` (absent on the [`Request::Context`] that
    /// produced it): an explicit call is answered, not pushed, and there is no
    /// transport to have succeeded or failed (FR-843, FR-854).
    RetrievalOutcome {
        trace_id: Uuid,
        transmitted: bool,
        #[serde(default)]
        failure_reason: Option<String>,
    },

    SessionCheckpoint {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
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

    // -----------------------------------------------------------------------
    // Feature 003 task work state (`contracts/task-model.md`).
    //
    // Note what is absent from every one of these: any field in which a caller
    // could store a completion percentage, and any sprint, epic, point,
    // assignee, estimate, board or inter-task dependency (FR-486, FR-491).
    // -----------------------------------------------------------------------
    TaskCriterionAdd {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        task_id: Uuid,
        text: String,
    },
    TaskCriterionSet {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        criterion_id: Uuid,
        #[serde(default)]
        state: Option<CriterionState>,
        #[serde(default)]
        text: Option<String>,
        /// What the caller read. Supplying it is how a caller is protected;
        /// omitting it applies the write and records `blind_write` (FR-490).
        #[serde(default)]
        expected_revision: Option<i64>,
    },
    /// Ask Cairn to verify a criterion from its evidence. There is no field in
    /// which a caller can *assert* a verification — that is the whole point.
    TaskCriterionVerify {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        criterion_id: Uuid,
        #[serde(default)]
        evidence_id: Option<Uuid>,
    },
    TaskCriterionRemove {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        criterion_id: Uuid,
    },
    TaskBlockerOpen {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        task_id: Uuid,
        description: String,
    },
    TaskBlockerClear {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        blocker_id: Uuid,
    },
    TaskReadiness {
        cwd: String,
        task_id: Uuid,
    },
    TaskHistory {
        cwd: String,
        task_id: Uuid,
        #[serde(default)]
        limit: Option<i64>,
    },

    MemoryPin {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        memory_id: Uuid,
        pinned: bool,
        #[serde(default)]
        reason: Option<String>,
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
        /// The subject this proposal concerns (FR-318). Optional: a free-form
        /// memory is fully valid and behaves exactly as it does in Feature 001.
        #[serde(default)]
        topic_key: Option<String>,
        #[serde(default)]
        value_key: Option<String>,
        #[serde(default)]
        importance: Option<Importance>,
        /// `project` (the default an absent field means) or `personal`.
        /// `team` is refused outright — no MCP action authors team knowledge
        /// directly; team is reached only by proposal or promotion (FR-431,
        /// FR-455, FR-517, FR-527).
        #[serde(default)]
        domain: Option<KnowledgeDomain>,
    },
    MemorySupersede {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        /// The session to attribute the replacement to, when the key is not to
        /// hand. Without it a worktree running two agents cannot say which one
        /// superseded the memory — the same ambiguity `MemoryCreate` resolves.
        #[serde(default)]
        session_id: Option<Uuid>,
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
        /// The subject this proposal concerns (FR-318). Optional: a free-form
        /// memory is fully valid and behaves exactly as it does in Feature 001.
        #[serde(default)]
        topic_key: Option<String>,
        #[serde(default)]
        value_key: Option<String>,
        #[serde(default)]
        importance: Option<Importance>,
    },
    MemoryForget {
        cwd: String,
        memory_id: Uuid,
        /// `project` (the default) or `personal` — forgets that domain's
        /// tombstone-only mutation (FR-441). `team` is refused: a team
        /// entry's lifecycle only advances through `cairn team retire`, by
        /// an admin, never through this tool.
        #[serde(default)]
        domain: Option<KnowledgeDomain>,
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

    // ---- Reusable cross-project patterns (`contracts/patterns.md`) --------
    //
    // A pattern is local to the machine and has no project identity, so none of
    // these carries a project — `cwd` is here only to resolve *this* project for
    // the promotion source and for an application's attribution.
    /// Recompute every derived value and report what differed (FR-478,
    /// FR-518).
    ///
    /// A release where a derived value disagrees with its rebuild ships a known
    /// inconsistency, so this exits non-zero when any of them does.
    RebuildDerived {
        cwd: String,
    },

    /// What this local store would lose if it were deleted, and what it would
    /// not (FR-705, FR-710a, SC-714).
    ///
    /// A question about the store, not about a project, but `cwd` is still
    /// carried: every request resolves a project, and a durability report that
    /// silently answered for whichever store the daemon happened to have open
    /// would be answering a question nobody asked.
    Durability {
        cwd: String,
    },

    /// List promoted patterns, with their counters.
    PatternList {
        cwd: String,
        #[serde(default)]
        trust: Option<PatternTrust>,
        #[serde(default)]
        signal: Option<String>,
    },
    /// One pattern in full: text, applications, counterexamples, and the
    /// sanitization report.
    PatternShow {
        cwd: String,
        id: Uuid,
    },
    /// Propose a promotion. Runs the ten-check gate; `dry_run` reports the
    /// outcome without writing (FR-395).
    PatternPromote {
        cwd: String,
        memory_id: Uuid,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        problem: Option<String>,
        #[serde(default)]
        signals: Vec<String>,
        #[serde(default)]
        applicability: Vec<String>,
        #[serde(default)]
        root_cause: Option<String>,
        #[serde(default)]
        approach: Option<String>,
        #[serde(default)]
        constraints: Vec<String>,
        #[serde(default)]
        dry_run: bool,
        /// What this promotes into. Absent means `pattern`, so a caller
        /// naming none gets today's behaviour unchanged (FR-506, D415).
        #[serde(default)]
        target: Option<PromotionTarget>,
        /// `(kind, value)` pairs the promoted record applies to, as
        /// `"kind=value"` strings — `language=rust`, `tool=docker` — screened
        /// against the closed `language | tool` vocabulary before this
        /// reaches the promotion gate; a value naming neither is refused
        /// rather than silently dropped (FR-434, FR-514). Meaningful only
        /// when `target` is `personal` or `team`; ignored for `target:
        /// pattern`, which keeps using `applicability` above for its own
        /// free-text conditions.
        #[serde(default)]
        applicability_facts: Vec<String>,
    },
    /// Record what happened when a pattern was applied here (FR-401, FR-404).
    PatternOutcome {
        cwd: String,
        id: Uuid,
        outcome: PatternOutcome,
        #[serde(default)]
        signals: Vec<String>,
        #[serde(default)]
        alternative_cause: Option<String>,
        #[serde(default)]
        evidence_id: Option<Uuid>,
        #[serde(default)]
        session: Option<Uuid>,
    },
    /// Tombstone a pattern. Its applications survive as history.
    PatternForget {
        cwd: String,
        id: Uuid,
    },

    // ---- Team knowledge (`contracts/global-memory.md` §5b, T133) ---------
    //
    // `list`/`propose` are reachable by any member; `ratify`/`retire` are
    // reachable only by an admin, and only through this CLI surface or the
    // server's own administration path — never through `cairn_remember`
    // (FR-455). Only `propose` carries a `cwd`, and it carries one for a
    // single reason: T123 requires the proposal to be screened against the
    // identities of the project the proposer is working in, and there is no
    // other way to learn which that is. Every *authorization* check on these
    // four (project membership, admin standing) is still answered by the
    // server from the caller's token, the same as `AdminUserCreate` and its
    // siblings above — the `cwd` is an input to the privacy screen, never to a
    // permission decision.
    //
    // Screening against an empty identity set would not have been a smaller
    // version of this. `validate_global_content` passes the
    // `project_identifying` class when it has no identities to compare against
    // (FR-580, the one documented fail-open), so a proposal made with no `cwd`
    // would be the one entry point of five at which naming a project is
    // allowed — and content only has to get in once.
    /// `authoritative` entries, plus the caller's own `proposed` ones. `all`
    /// additionally asks for every state, honored only for an admin caller
    /// (FR-464).
    TeamList {
        #[serde(default)]
        all: bool,
    },
    /// Any member of at least one project may propose; the row lands
    /// `proposed` and nothing else — no path from here ever reaches
    /// `authoritative` (FR-451, FR-455).
    TeamPropose {
        cwd: String,
        content: String,
        #[serde(default)]
        knowledge_type: Option<MemoryType>,
        #[serde(default)]
        topic_key: Option<String>,
        #[serde(default)]
        value_key: Option<String>,
        /// `"kind=value"` strings, screened against the closed
        /// `language | tool` vocabulary (FR-446).
        #[serde(default)]
        applicability: Vec<String>,
    },
    /// Admin only. Moves `proposed` to `authoritative` by compare-and-swap on
    /// the entry's current state, never last-write-wins (D409, FR-454).
    TeamRatify {
        id: Uuid,
        /// This ratification's own explicit `supersedes` relation (T127, §6,
        /// D431) — the one place that relation kind is ever recorded, and
        /// only on the ratifying admin's say-so, never inferred.
        #[serde(default)]
        supersedes: Option<Uuid>,
    },
    /// Admin only. Moves `authoritative` to `retired`; never reversible by
    /// re-ratifying (FR-465).
    TeamRetire {
        id: Uuid,
    },

    // ---- Personal knowledge (`contracts/global-memory.md` §5a, T082) ------
    //
    // Reads and the one tombstone mutation, over `personal_knowledge`. No
    // `cwd`: like team knowledge, personal knowledge follows the account,
    // not any one project — `recall_personal` filters by applicability at
    // read time inside briefing/search composition, but a caller's own
    // listing here shows everything they hold, unfiltered by the project
    // they happen to be standing in.
    /// This account's own personal entries (T082, FR-434–FR-436).
    PersonalList {
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        limit: Option<i64>,
    },
    /// Tombstone one entry: content cleared, nothing else touched (FR-440,
    /// FR-441). Scoped to the caller's own account by the store call this
    /// forwards to — a caller cannot forget another account's entry, or
    /// even learn that one exists at that id.
    PersonalForget {
        id: Uuid,
    },

    /// This project's derived stack traits — the same set applicability
    /// matching reads at recall time (D413, FR-437, T082).
    ProjectTraits {
        cwd: String,
    },

    // ---- Shared-project membership (`contracts/identity-
    // administration.md` §9a, T063) ---------------------------------------
    //
    // Every route below is addressed by email, resolved to a server-side row
    // id the same way `AdminUserPatch` is (FR-418–FR-427) — the CLI never
    // learns or holds a project member's uuid.
    /// `POST /api/projects/{id}/members`. Grants membership; refused for
    /// anyone but an existing member or a server admin (FR-418, FR-419).
    ProjectMemberAdd {
        project_id: Uuid,
        email: String,
    },
    /// `DELETE /api/projects/{id}/members` (FR-420, FR-421).
    ProjectMemberRemove {
        project_id: Uuid,
        email: String,
    },
    /// `GET /api/projects/{id}/members` (FR-427).
    ProjectMemberList {
        project_id: Uuid,
    },

    /// Inspect a subject: its members, its canonical answer or answers, its
    /// reconciliation state, and the decisions that produced it (FR-307).
    MemorySubject {
        cwd: String,
        topic_key: String,
        #[serde(default)]
        scope: Option<MemoryScope>,
        #[serde(default)]
        scope_key: Option<String>,
        /// Which domain's subject to read. Absent means `project`, so every
        /// pre-004 caller is unaffected.
        ///
        /// A domain is not a scope, and this field is why: `scope`/`scope_key`
        /// describe how long a *project* memory stays relevant and mean nothing
        /// to the other two domains, which have no scope at all. Naming a domain
        /// selects which corpus and which relations table the derivation reads,
        /// and nothing else about the read changes (FR-442, FR-462; T078, T127).
        #[serde(default)]
        domain: Option<KnowledgeDomain>,
    },
    /// A session confirms an existing memory is still true (FR-321).
    ///
    /// Explicit only. Cairn never infers a reinforcement from a matching value
    /// key — that inference was the false-merge path this feature closed.
    MemoryReinforce {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        memory_id: Uuid,
        /// The memory carrying the confirming session's statement. When absent
        /// the confirmation is recorded against the session itself.
        #[serde(default)]
        from_memory_id: Option<Uuid>,
    },
    /// Attach a bounded, redacted, attributable evidence fact (FR-351).
    ///
    /// Local, always: there is no outbox entity type and no server table for
    /// one, which is what makes "evidence content never leaves the machine" a
    /// property of the schema rather than a promise.
    EvidenceAdd {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        kind: EvidenceKind,
        /// `cairn` when Cairn read it; `agent` when an agent attested it. An
        /// attested fact is usable, labelled everywhere, and refused by the two
        /// strict consumers (FR-355, FR-370).
        #[serde(default)]
        collector: Option<EvidenceCollector>,
        subject: String,
        observed_value: String,
        /// Repository-relative, or a Git ref. Never absolute (FR-353).
        source_locator: String,
        #[serde(default)]
        observation_id: Option<Uuid>,
        /// The memory this supports or contradicts, when it is being attached.
        #[serde(default)]
        memory_id: Option<Uuid>,
        #[serde(default)]
        role: Option<EvidenceRole>,
    },
    EvidenceList {
        cwd: String,
        #[serde(default)]
        memory_id: Option<Uuid>,
    },
    EvidenceShow {
        cwd: String,
        evidence_id: Uuid,
    },
    /// Run verification on demand. Same caps, same verifiers, reported
    /// synchronously (FR-472).
    Verify {
        cwd: String,
        #[serde(default)]
        memory_id: Option<Uuid>,
        /// Every memory in the project that owes a check.
        #[serde(default)]
        all: bool,
        /// Return the run history rather than only the current state.
        #[serde(default)]
        explain: bool,
    },
    /// Record an explicit reconciliation decision (FR-335).
    MemoryReconcile {
        cwd: String,
        #[serde(default)]
        agent_session_key: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        from_memory_id: Uuid,
        to_memory_id: Uuid,
        relation: RelationKind,
        basis: RelationBasis,
        #[serde(default)]
        basis_evidence_id: Option<Uuid>,
        #[serde(default)]
        rationale: Option<String>,
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
    /// Change the caller's own password (FR-405, `contracts/identity-
    /// administration.md` §5). Reachable regardless of `must_change_password`
    /// — it is the one route that is.
    AuthChangePassword {
        new_password: String,
    },
    SyncStatus {
        cwd: String,
    },
    SyncNow {
        cwd: String,
    },

    // -----------------------------------------------------------------------
    // Administration (`contracts/identity-administration.md` §2, §2a, §9).
    //
    // Every one of these is daemon-mediated exactly like `Link`/`AuthTokenSet`
    // above: the CLI never holds a bearer token, and the daemon is what makes
    // the HTTP call. None of these carries `cwd` — an account is server-wide,
    // not project-scoped, so there is no repository to resolve.
    // -----------------------------------------------------------------------
    /// `POST /api/admin/users` (FR-401). Admin-only; the server enforces that,
    /// this only carries the request. The response's `temporary_password` is
    /// shown to the operator exactly once — there is no route that reads it
    /// back (FR-403).
    AdminUserCreate {
        email: String,
        display_name: String,
    },
    /// `GET /api/admin/users`: every account, its role and its status
    /// (FR-411).
    AdminUserList,
    /// `PATCH /api/admin/users/{id}`: promote, demote, disable or enable one
    /// account (FR-402, FR-408, FR-412), addressed by email — the CLI never
    /// has to learn or hold a server-side row id.
    AdminUserPatch {
        email: String,
        #[serde(default)]
        role: Option<ServerRole>,
        #[serde(default)]
        status: Option<UserStatus>,
    },
    /// `POST /api/admin/users/{id}/reset-password` (FR-553–FR-559). The new
    /// temporary password is returned exactly once, and never again by any
    /// route (FR-554).
    ResetPassword {
        email: String,
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
    /// How far subject identity has actually reached in this project, and what
    /// the mechanism is currently reporting (FR-499). Defaulted, so a Feature
    /// 001 consumer reading an older payload still parses.
    #[serde(default)]
    pub knowledge: Option<KnowledgeHealth>,
    /// Boundaries whose synthesis has failed, with the redacted reason. They
    /// stay retryable and actionable; this is not a terminal outcome.
    #[serde(default)]
    pub handoff_synthesis_failures: Vec<HandoffFailure>,
    /// What capture did, and where its events currently are (FR-740, FR-749c).
    ///
    /// Defaulted so a client reading an older payload still parses.
    #[serde(default)]
    pub capture: Option<CaptureHealth>,
}

/// Capture's own state, reported truthfully rather than reassuringly.
///
/// Fail-soft describes what the agent experiences: a capture-class event that
/// misses its deadline is dropped and the hook still exits successfully. It
/// does not describe what Cairn is allowed to know about itself, and this is
/// where the difference is visible (FR-749c, SC-706).
///
/// Every number here is a count. No field carries a path, a command, a token or
/// any part of an event — a disposition record has nothing of the payload it
/// was processing (FR-749d, FR-741), and neither does its summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CaptureHealth {
    /// How often each disposition was recorded, keyed by its fixed name.
    ///
    /// A map rather than named fields, because the vocabulary is closed and
    /// declared once in `data-model.md` §4; thirteen fields here would be a
    /// second declaration of it that could fall behind the first.
    #[serde(default)]
    pub dispositions: std::collections::BTreeMap<String, i64>,
    /// Where this machine's undelivered events are.
    #[serde(default)]
    pub events: SpoolHealth,
    /// Where this machine's undelivered commands are.
    #[serde(default)]
    pub commands: SpoolHealth,
}

/// One spool, as a partition (`data-model.md` §3).
///
/// A queued row is in exactly one of these conditions and they cover every row
/// the spool still holds, so the five sum to the table. `undelivered` is
/// derived from the four non-terminal ones rather than counted separately, so
/// it cannot disagree with them, and `terminal_retry_exhausted` is a subset of
/// `terminal` rather than a sixth condition.
/// No longer `Copy`: FR-792's two additions are an instant and a reason, and
/// both are text on the wire. A status type that had to stay `Copy` would be a
/// type that could never carry a reason.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpoolHealth {
    pub waiting: i64,
    pub in_flight: i64,
    pub retrying: i64,
    /// Waiting for a server that cannot yet hold this contract version or kind.
    /// Not a failure, and it spends no attempt budget.
    pub deferred: i64,
    pub terminal: i64,
    pub terminal_retry_exhausted: i64,
    pub undelivered: i64,
    /// Whether the spool is at its bound and refusing new work.
    pub saturated: bool,
    /// Undelivered rows queued for a different server instance (FR-791).
    ///
    /// Intact, visible, and undeliverable under the deployment this store is
    /// talking to now. Zero on any ordinary machine; a non-zero value means the
    /// endpoint now answers as a server that did not queue this work.
    #[serde(default)]
    pub other_instance: i64,
    /// When the oldest undelivered row was created (FR-792), RFC 3339, or
    /// absent when nothing is waiting.
    ///
    /// A depth on its own does not say whether anything is wrong. Fifty rows
    /// spooled in the last second is a busy minute; one row spooled last week is
    /// an outage nobody noticed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_at: Option<String>,
    /// Why delivery is not progressing (FR-792), or absent when it is.
    ///
    /// One of `no_account`, `server_unreachable`, `server_instance_mismatch`,
    /// `saturated`, `retry_exhausted`, `refused_by_server`,
    /// `awaiting_capability`, `backing_off` — most severe first,
    /// because a spool can be several at once and this reports one. A closed
    /// vocabulary rather than a message, so a caller can branch on it and a
    /// reader is not asked to parse prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
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

    // Feature 003 read-only fields. A Feature 001 caller sees its existing
    // fields unchanged and simply gains these (FR-497).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporal: Option<Temporal>,
    /// Ranks within a bucket and nothing more (FR-308).
    pub importance: Importance,
    pub pinned: bool,
    pub verification: VerificationInfo,
    pub reinforcement: Reinforcement,
    /// Where this result stands in its subject. Absent on a free-form memory,
    /// which belongs to no subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<SubjectInfo>,
}

/// The verification a **local** reader may see.
///
/// Deliberately not `VerificationSummary`, which is what the outbox transmits.
/// That one collapses `remote_cairn` to `cairn`, because a peer must not learn
/// that *this* machine imported a state rather than checking it (T104, FR-502).
/// A local caller has the opposite need: without the `remote_` prefix it cannot
/// tell a check this machine ran from one it was merely told about, which is
/// the distinction FR-370 exists to preserve. Two readers, two truths, two
/// types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationInfo {
    pub state: VerificationState,
    /// Always present when the state is `verified` (FR-370).
    pub authority: Option<VerificationAuthority>,
    pub last_verified_at: Option<String>,
    /// A count of supporting evidence facts. Never the facts themselves.
    pub fact_count: usize,
    /// Verifier **kinds** only — never a subject, value, locator or digest
    /// (FR-502).
    pub basis: Vec<VerifierKind>,
}

/// How many sessions confirmed a memory is still true.
///
/// A reinforcement is an explicit act by a session that read the memory
/// (FR-321). It is **never** a verification, and must never be presented as one
/// (FR-406) — which is why it is a field of its own rather than a number folded
/// into `verification`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reinforcement {
    pub count: i64,
    pub distinct_origins: i64,
}

/// Where a result stands among the other answers to its subject.
///
/// The two arrays are the same distinction the subject state rests on: an
/// answer that asserts a *different* value competes, and one that asserts the
/// *same* value with different words corroborates. Cairn never merges the
/// second (D46), so a caller that wants one answer has to be told there are
/// others and which kind they are.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubjectInfo {
    pub reconciliation: Reconciliation,
    pub is_canonical_answer: bool,
    pub competing_answers: Vec<Uuid>,
    pub corroborating_answers: Vec<Uuid>,
}

/// What Cairn can say about when a proposal applied.
///
/// The claim is bounded by what Cairn stores authoritatively: an
/// `effective_from` it recorded, and a `superseded_at` set with the
/// supersession relation. A lifecycle transition with no authoritative instant
/// reports `applicability: unknown` rather than an unbounded interval (FR-342,
/// D82).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Temporal {
    pub effective_from: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_at: Option<DateTime<Utc>>,
    /// NULL means **unknown**, never "not stale".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_at: Option<DateTime<Utc>>,
    /// `bounded` when every transition this memory underwent has an
    /// authoritative instant; `unknown` when one does not.
    pub applicability: Applicability,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPayload {
    pub results: Vec<MemoryResult>,
    pub total: usize,
}

/// One personal-knowledge search result (`contracts/recall-composition.md`
/// §7). Spliced onto a search response as a **sibling** `personal` array,
/// exactly as `patterns[]` already is — never merged into `SearchPayload`'s
/// own `results`, so `total` continues to describe project results alone
/// (FR-469, FR-470).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalSearchResult {
    pub id: Uuid,
    pub content: String,
    pub topic_key: Option<String>,
    pub value_key: Option<String>,
    pub created_at: DateTime<Utc>,
    /// `(kind, value)` pairs from the closed `language | tool` vocabulary —
    /// never this record's own `topic_key`, a different question entirely
    /// (D439, FR-570).
    pub applicability: Vec<ApplicabilityFact>,
}

/// One team-knowledge search result. See [`PersonalSearchResult`].
///
/// Only ever `authoritative`, or `proposed` and owned by (or visible to an
/// admin alongside) the caller — the same visibility predicate every
/// team-knowledge read carries (`contracts/global-memory.md` §5b, FR-452).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamSearchResult {
    pub id: Uuid,
    pub content: String,
    pub topic_key: Option<String>,
    pub value_key: Option<String>,
    pub state: TeamState,
    pub applicability: Vec<ApplicabilityFact>,
}

/// What a `create` turned out to mean for the subject it joined
/// (`contracts/mcp-tools.md` §`cairn_remember`).
///
/// The internal decision is `ProposalOutcome`, a tagged enum. This is its wire
/// form: one flat object a caller can read field by field without matching on a
/// tag, which is what the contract fixes and what an agent reading JSON needs.
///
/// Two fields are deliberately *not* what a lookup table would produce.
/// `relation_recorded` is the kind the write actually recorded, carried out of
/// the transaction that wrote it — never re-derived from the outcome, so it
/// cannot drift from what is in the database. And `matched_memory_id` is null
/// for a conflict: a conflict is intrinsically several, and naming one of them
/// as *the* match would be arbitration by identifier, which nothing in Cairn
/// does (FR-334). The full set is in `competing_memory_ids`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconciliationReport {
    /// `created | duplicate | corroborating | conflict_detected | deferred`.
    pub outcome: String,
    /// The single member this proposal matched, where there is exactly one.
    pub matched_memory_id: Option<Uuid>,
    /// That member's value key.
    pub matched_value_key: Option<String>,
    /// The subject, normalized. Absent on a free-form memory.
    pub subject: Option<String>,
    /// The relation this write recorded, if any. `corroborating` records
    /// nothing, and says so (FR-327).
    pub relation_recorded: Option<RelationKind>,
    pub conflict_detected: bool,
    /// The one call that would settle this, where a caller — which can read
    /// both statements — is the only party able to decide. Null where there is
    /// nothing to settle.
    pub next_step: Option<String>,
    /// Every member the proposal disagrees with, in identifier order for a
    /// stable rendering. Empty unless `conflict_detected`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub competing_memory_ids: Vec<Uuid>,
}

/// The verbatim prompt FR-327 relies on. A corroborating write merges nothing;
/// this is how the party that *can* read both statements is told it may.
pub const CORROBORATING_NEXT_STEP: &str =
    "if this is the same claim, call action=reinforce with memory_id";

/// A conflict is recorded and never resolved (FR-334). The caller is pointed at
/// the explicit decision, not at a merge.
pub const CONFLICT_NEXT_STEP: &str =
    "both answers stand; record a decision with action=reconcile when you know which applies";

impl ReconciliationReport {
    /// Build the wire form from the decision and the facts the write held.
    ///
    /// `relation_recorded` and `matched_value_key` come from the caller because
    /// only the write knows them: the first is what was inserted, the second is
    /// a column of a member the classifier already had in hand.
    pub fn build(
        outcome: &crate::knowledge::ProposalOutcome,
        subject: Option<&str>,
        relation_recorded: Option<RelationKind>,
        matched_value_key: Option<String>,
    ) -> Self {
        use crate::knowledge::ProposalOutcome as P;
        let (matched, competing, next_step) = match outcome {
            P::Duplicate { of } => (Some(*of), Vec::new(), None),
            P::Corroborating { member } => (
                Some(*member),
                Vec::new(),
                Some(CORROBORATING_NEXT_STEP.to_string()),
            ),
            P::ConflictDetected { with } => {
                let mut with = with.clone();
                with.sort();
                (None, with, Some(CONFLICT_NEXT_STEP.to_string()))
            }
            P::Created | P::Deferred => (None, Vec::new(), None),
        };
        Self {
            outcome: outcome.as_str().to_string(),
            matched_memory_id: matched,
            matched_value_key: matched.and(matched_value_key),
            subject: subject.map(str::to_string),
            relation_recorded,
            conflict_detected: matches!(outcome, P::ConflictDetected { .. }),
            next_step,
            competing_memory_ids: competing,
        }
    }
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
    // ---- Feature 004's two global sections, last in the priority order.
    //
    // `Vec<String>` deliberately, exactly like `decisions` and `known_failures`:
    // rendered lines and nothing else. There is **no field here for a selection
    // reason** (FR-478, D451), and that absence is the enforcement — reasons are
    // produced on the diagnostic path and the rendered type has nowhere to put
    // one, so a renderer cannot leak them by forgetting to omit them.
    //
    // Skipped when empty, so a caller with no personal or team knowledge gets
    // byte-identical output to one that never touched either domain (FR-481).
    /// Personal knowledge admitted to this briefing (FR-476).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub personal_notes: Vec<String>,
    /// Authoritative team guidance admitted to this briefing (FR-476).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub team_guidance: Vec<String>,
    pub no_prior_history: bool,
    // ---- Feature 003. Every one is skipped when empty, so a project with no
    // Level 0 content produces exactly the bytes Feature 001 produced (FR-442).
    /// Critical warning kinds with counts (Tier 0a), then their detail (Tier 0b).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<ContextWarning>,
    /// Pinned constraints in force for this scope.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<PinnedConstraint>,
    /// The recorded next action of a diverged checkpoint, **never** presented as
    /// `next_action` (FR-434). Absent until Phase 9 records checkpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_next_action: Option<String>,
    /// Signal-matched prior patterns from other projects (FR-398, SC-312).
    ///
    /// A **separate array**, never merged into `memory`. A pattern is not this
    /// project's knowledge and must not be readable as though it were; keeping
    /// it in its own field is what makes that structural rather than a matter of
    /// how it happens to be rendered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<BriefingPattern>,
}

/// A prior pattern offered to this project, with everything needed to rule it
/// out cheaply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingPattern {
    pub id: Uuid,
    pub title: String,
    /// `sanitized`, `validated` or `contested`. Stated, never summarized into a
    /// score.
    pub trust: PatternTrust,
    /// **Always false.** A pattern is never verified in the project being
    /// briefed; it is offered, not asserted (SC-312).
    pub verified_in_this_project: bool,
    /// The conditions under which it applies, so the agent can rule it out
    /// without trying it.
    pub applicability: Vec<String>,
    pub approach: String,
    /// What the approach does *not* do.
    pub constraints: Vec<String>,
    /// A cause someone else found behind the same symptom. Present only when a
    /// counterexample recorded one (FR-405).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alternative_cause: Option<String>,
    /// What to rule out first, derived from the alternative cause.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_this_first: Option<String>,
    /// How many of this project's own signals matched.
    pub signal_overlap: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingTask {
    pub id: Uuid,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub status: TaskStatus,
    // ---- Feature 003 Tier 0a. Every one is O(1) in the size of the task, which
    // is what makes the tier's guarantee keepable (FR-443).
    /// Counts by state. Never a percentage — there is no field for one (FR-486).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<crate::tasks::Progress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_readiness: Option<CompletionReadiness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_blockers: Option<usize>,
    /// The single most actionable open blocker, summarized to one line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    /// True when the goal was truncated to `goal_max_tokens`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub goal_truncated: bool,
    /// Criterion labels admitted as Tier 0b detail, in action order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub criteria: Vec<BriefingCriterion>,
    /// How many criteria did not fit, with the path that retrieves them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria_omitted: Option<usize>,
}

/// One criterion as Level 0 renders it — both axes named, never collapsed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingCriterion {
    pub label: String,
    pub text: String,
    pub state: CriterionState,
    pub verification: CriterionVerification,
}

/// A Level 0 warning. Content, not diagnostics: present whether or not
/// `explain` was requested (FR-464).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextWarning {
    /// `task_divergence` | `checkpoint` | `task` | `conflict` | `drift`.
    pub kind: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// A pinned constraint in force for this scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinnedConstraint {
    pub id: Uuid,
    pub text: String,
    /// A pin whose claim no longer holds keeps its pin and carries its warning —
    /// a constraint that stopped being true is exactly what must be said
    /// (FR-456).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub drifted: bool,
}

/// Why each admitted item was chosen, and why each omission was left out.
///
/// Returned only when `explain` was requested, so it costs no budget otherwise
/// (FR-463).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Selection {
    pub budget: usize,
    pub reserve: usize,
    pub reserve_used: usize,
    pub reserve_released: usize,
    pub included: Vec<SelectedItem>,
    pub omitted: Vec<OmittedItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedItem {
    pub level: ContextLevel,
    pub kind: String,
    pub id: String,
    pub reasons: Vec<SelectionReason>,
    pub cost: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmittedItem {
    pub kind: String,
    pub count: usize,
    pub reason: OmissionReason,
    /// How to retrieve what was left out. Omission is never silent (FR-448).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub retrieval: String,
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
    /// Present only when `explain` was requested (FR-463).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Selection>,
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
    /// Work retained for a server that cannot hold it yet (FR-415, FR-418).
    ///
    /// Reported apart from `pending` and from `failed` because it is neither.
    /// Defaulted so a Feature 001 consumer reading an older payload still
    /// parses, and so this payload still satisfies an older consumer.
    #[serde(default)]
    pub degradation: Option<SyncDegradation>,
}

/// One synchronization namespace's outbox standing (FR-487,
/// `contracts/sync-namespaces.md`).
///
/// Spliced onto `cairn sync status`'s response as a sibling `namespaces`
/// array — the same way `patterns[]` rides alongside `SearchPayload` — rather
/// than added to [`SyncStatusPayload`] itself, so that struct's existing
/// project-scoped `pending`/`failed` keep meaning exactly what they meant
/// before this feature. A project always has exactly one `project` row here;
/// `personal`/`team` rows are present only once this store has ever queued
/// something in that namespace (D426, D427, FR-486).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceSyncStatus {
    /// The cursor key, e.g. `project:<id>`, `personal:<instance>:<user>`,
    /// `team:<instance>` (`SyncNamespace::key`).
    pub namespace: String,
    pub kind: KnowledgeDomain,
    pub pending: i64,
    pub failed: i64,
    pub blocked: i64,
    /// Holes in one writer's own sequence, as this store observes them
    /// (FR-492, SC-450).
    ///
    /// **Diagnostic only, and reported here because nowhere else would say it.**
    /// A gap nobody surfaces is indistinguishable from a stream that had no gap,
    /// which is the whole reason `writer_seq` crosses the wire at all — it is
    /// useless to the store that minted it and useful only to whoever receives
    /// it. Nothing in recall, reconciliation or ordering reads this: `MemoryFacts`
    /// has no `writer_seq` field, so a tiebreak that consulted one would not
    /// compile (FR-583).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<WriterSequenceGap>,
}

/// One writer's missing sequence numbers, in the shape `cairn sync status`
/// renders.
///
/// Carries no content and no identity beyond the opaque writer id: a gap says
/// that something did not arrive, not what it was.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriterSequenceGap {
    pub writer_id: Uuid,
    pub missing: Vec<i64>,
    pub highest_seen: i64,
}

/// What a server cannot hold, and what happens next.
/// What `cairn status` says about the subject mechanism's reach.
///
/// Reported as counts and one share, never as a score. `subject_share_percent`
/// is absent when the project has no project-scoped memory to have adopted
/// anything.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeHealth {
    pub project_memories: i64,
    pub with_subject: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_share_percent: Option<i64>,
    pub conflicted_subjects: i64,
    pub needs_recheck: i64,
    pub drifted: i64,
    /// Present only when this project is retaining work for an older server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_degradation: Option<SyncDegradation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncDegradation {
    pub blocked: i64,
    /// What the server last said it could do.
    pub server_capability: String,
    /// The capabilities the retained work is waiting for, named.
    pub missing_capabilities: Vec<String>,
    /// One line for a person: what is still syncing, and what happens on
    /// upgrade. Degradation must never read as data loss (FR-415).
    pub note: String,
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

// `REJECTED_OBSERVATION_FIELDS` used to live here: seven names from Feature
// 001, presented as if it were the same boundary as the server's live,
// enforced `FORBIDDEN_OBSERVATION_FIELDS` (`crates/cairn-server/src/sync.rs`,
// twenty-seven names as of Feature 003). Nothing in the workspace read it —
// grepping for the identifier turns up only its own declaration and this
// note — so it was documentation-shaped code that happened to compile, and a
// twenty-item-short list presented as the boundary is worse than no list at
// all. The server's list is the single source of truth for what the wire
// rejects; see `cairn_server::sync::FORBIDDEN_OBSERVATION_FIELDS` (FR-534).

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
        all.extend_from_slice(IDENTITY_CODES);
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
        assert!(!INTELLIGENCE_CODES
            .iter()
            .any(|c| c.contains("budget_exceeded")));
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
            assert!(
                INTELLIGENCE_CODES.contains(code),
                "{code} is not in the set"
            );
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
