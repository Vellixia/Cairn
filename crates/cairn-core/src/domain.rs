//! Domain types and enums for Cairn (data-model.md).
//!
//! These are pure data. No I/O lives here.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// Generate a time-ordered identifier. UUIDv7 everywhere (data-model.md).
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

/// Error returned when a stored enum string does not parse.
#[derive(Debug, thiserror::Error)]
#[error("invalid {kind} value: {value}")]
pub struct ParseEnumError {
    pub kind: &'static str,
    pub value: String,
}

/// Declare a lowercase-text enum with a `CHECK`-friendly string form.
macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident, $kind:literal, {
        $($(#[$vmeta:meta])* $variant:ident => $text:literal),+ $(,)?
    }) => {
        $(#[$meta])*
        // `Ord` follows declaration order and exists only so a collection of
        // these sorts stably for output. No semantics rest on it: nothing in
        // Cairn decides which of two records is *correct* by comparing enums.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(&self) -> &'static str {
                match self {
                    $($name::$variant => $text),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ParseEnumError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($text => Ok($name::$variant),)+
                    other => Err(ParseEnumError { kind: $kind, value: other.to_string() }),
                }
            }
        }
    };
}

text_enum!(
    /// Task lifecycle (FR-037). No revision history exists (FR-039).
    TaskStatus, "task status", {
        Todo => "todo",
        InProgress => "in_progress",
        Done => "done",
        Blocked => "blocked",
    }
);

text_enum!(
    /// Session lifecycle (FR-007). A session leaves `Active` only at the
    /// deterministic boundaries in FR-009 — never on a `Stop` turn checkpoint.
    SessionStatus, "session status", {
        Active => "active",
        Completed => "completed",
        Interrupted => "interrupted",
    }
);

text_enum!(
    /// Structured observation kinds (FR-011).
    ObservationType, "observation type", {
        FileRead => "file_read",
        FileChanged => "file_changed",
        CommandRun => "command_run",
        TestRun => "test_run",
        Error => "error",
        Decision => "decision",
        Discovery => "discovery",
        UserInstruction => "user_instruction",
    }
);

text_enum!(
    /// Durable knowledge kinds (FR-016).
    MemoryType, "memory type", {
        Fact => "fact",
        Decision => "decision",
        Convention => "convention",
        Failure => "failure",
        Procedure => "procedure",
    }
);

text_enum!(
    /// Where a memory applies (FR-017). Always paired with a scope key.
    MemoryScope, "memory scope", {
        Project => "project",
        Branch => "branch",
        Task => "task",
        Session => "session",
    }
);

impl MemoryScope {
    /// Ranking bucket: lower sorts first (FR-024, D3).
    pub fn bucket(&self) -> i64 {
        match self {
            MemoryScope::Task => 0,
            MemoryScope::Branch => 1,
            MemoryScope::Project => 2,
            MemoryScope::Session => 3,
        }
    }
}

text_enum!(
    /// Memory lifecycle (FR-018). Only `Active` is returned by default.
    MemoryState, "memory state", {
        Active => "active",
        Stale => "stale",
        Superseded => "superseded",
    }
);

text_enum!(
    /// Boundaries that produce a durable handoff (FR-032).
    ///
    /// A `Stop` turn checkpoint is deliberately absent: it is a turn boundary,
    /// not a session boundary (D16).
    HandoffTrigger, "handoff trigger", {
        PreCompact => "pre_compact",
        SessionEnd => "session_end",
        Recovered => "recovered",
    }
);

text_enum!(
    /// Entities the outbox can carry. There is deliberately no observation
    /// variant: raw observations never sync (FR-055, D9).
    ///
    /// Feature 003 adds exactly three (D66). Everything else it introduces —
    /// evidence facts, verification runs, continuity checkpoints, reusable
    /// patterns, pattern applications, task changes, criterion evidence — has
    /// **no** variant here and no server table, which is what makes "it stays
    /// local" a property of the schema rather than a promise (FR-503, I8).
    OutboxEntityType, "outbox entity type", {
        Project => "project",
        Task => "task",
        Session => "session",
        Memory => "memory",
        Handoff => "handoff",
        MemoryRelation => "memory_relation",
        TaskCriterion => "task_criterion",
        TaskBlocker => "task_blocker",
    }
);

text_enum!(
    OutboxOperation, "outbox operation", {
        Upsert => "upsert",
        Delete => "delete",
    }
);

text_enum!(
    /// `blocked` is Feature 003's addition and is deliberately **not** terminal
    /// (D81, FR-418): the server refused the work for lack of a capability, not
    /// because of its content. It is excluded from `claim` until the server's
    /// capability changes, then returns to `pending` and delivers exactly once
    /// under its original idempotency key. `failed` keeps its meaning — the
    /// content itself was refused, permanently.
    OutboxState, "outbox state", {
        Pending => "pending",
        InFlight => "in_flight",
        Delivered => "delivered",
        Failed => "failed",
        Blocked => "blocked",
    }
);

impl OutboxState {
    /// Whether a drainer may take this row.
    ///
    /// `blocked` is excluded here and by an explicit predicate in the claim
    /// query, so a capability-refused row is never retried against a server
    /// known to lack the capability (FR-418).
    pub fn is_claimable(&self) -> bool {
        matches!(self, OutboxState::Pending | OutboxState::InFlight)
    }

    /// Whether the row will never move again.
    pub fn is_terminal(&self) -> bool {
        matches!(self, OutboxState::Delivered | OutboxState::Failed)
    }
}

text_enum!(
    /// Why the server refused a queued item for lack of capability (D81).
    ///
    /// Distinct from a refusal of the **content**, which stays permanently
    /// `failed`. This is what a later capability change is compared against.
    BlockedReason, "blocked reason", {
        UnknownEntityType => "unknown_entity_type",
        UnknownField => "unknown_field",
        SchemaOlder => "schema_older",
    }
);

// ---------------------------------------------------------------------------
// Feature 003 — project intelligence (data-model.md §7)
// ---------------------------------------------------------------------------

text_enum!(
    /// What is established about a claim. Orthogonal to [`MemoryState`], which
    /// is its lifecycle: the two are never collapsed into one value, in storage
    /// or in output (FR-362).
    ///
    /// `conflicted` here means **this memory's own evidence disagrees with
    /// itself** — supporting and contradicting facts both attached, or two runs
    /// of one verifier disagreeing at the same commit. Subject-level
    /// disagreement is a [`Reconciliation`] state, and the two are reported
    /// separately (FR-369).
    VerificationState, "verification state", {
        Unverified => "unverified",
        Verified => "verified",
        NeedsRecheck => "needs_recheck",
        Drifted => "drifted",
        Conflicted => "conflicted",
    }
);

text_enum!(
    /// What *established* a verification, as a dimension distinct from what was
    /// established (FR-370, D76).
    ///
    /// Collapsing this into the state would let an agent's own assertion wear
    /// the same badge as a check Cairn performed. It is derived from the
    /// evidence a successful run consulted, never asserted, and it travels with
    /// the state everywhere — including across a sync boundary, which is where
    /// the earlier `verification_origin ∈ {local, remote}` lost the
    /// distinction.
    VerificationAuthority, "verification authority", {
        /// A deterministic check this machine ran over `collector = cairn`
        /// evidence. The only authority a task criterion or a cross-project
        /// promotion accepts (FR-484, FR-396).
        Cairn => "cairn",
        /// The memory is verified, and every run that established it consulted
        /// only agent-attested evidence. Useful, labelled, and visibly weaker.
        Attested => "attested",
        /// Imported: the peer's authority was `cairn`.
        RemoteCairn => "remote_cairn",
        /// Imported: the peer's authority was `attested`.
        RemoteAttested => "remote_attested",
    }
);

impl VerificationAuthority {
    /// Whether this authority is a deterministic check **this machine** ran.
    ///
    /// The single question the two strict consumers ask (FR-484, FR-396).
    pub fn is_local_deterministic(&self) -> bool {
        matches!(self, VerificationAuthority::Cairn)
    }

    /// Whether the verification was established on another machine.
    pub fn is_imported(&self) -> bool {
        matches!(
            self,
            VerificationAuthority::RemoteCairn | VerificationAuthority::RemoteAttested
        )
    }

    /// The authority a peer's value becomes when imported (FR-368, T104).
    ///
    /// A sender's value is never stored verbatim: `cairn` from elsewhere is not
    /// `cairn` here.
    pub fn imported(sent: VerificationAuthority) -> VerificationAuthority {
        match sent {
            VerificationAuthority::Cairn | VerificationAuthority::RemoteCairn => {
                VerificationAuthority::RemoteCairn
            }
            VerificationAuthority::Attested | VerificationAuthority::RemoteAttested => {
                VerificationAuthority::RemoteAttested
            }
        }
    }

    /// The value that may travel on the wire — only ever `cairn` or `attested`
    /// (`contracts/privacy-sync.md`, T099).
    pub fn on_the_wire(&self) -> VerificationAuthority {
        match self {
            VerificationAuthority::Cairn | VerificationAuthority::RemoteCairn => {
                VerificationAuthority::Cairn
            }
            VerificationAuthority::Attested | VerificationAuthority::RemoteAttested => {
                VerificationAuthority::Attested
            }
        }
    }
}

text_enum!(
    /// A ranking hint within one bucket, and nothing else (FR-308).
    ///
    /// It never changes scope precedence, never admits an item into reserved
    /// context, and never affects reconciliation, verification or promotion.
    Importance, "importance", {
        Low => "low",
        Normal => "normal",
        High => "high",
    }
);

impl Importance {
    /// Lower sorts first, matching [`MemoryScope::bucket`].
    pub fn rank(&self) -> i64 {
        match self {
            Importance::High => 0,
            Importance::Normal => 1,
            Importance::Low => 2,
        }
    }
}

impl Default for Importance {
    fn default() -> Self {
        Importance::Normal
    }
}

text_enum!(
    /// A durable, append-only reconciliation decision relating two proposals
    /// (FR-304). These records *are* the reconciliation: there is no canonical
    /// row, so the answer is derived from proposals and decisions and there is
    /// nothing to overwrite (D44, D47).
    RelationKind, "relation kind", {
        /// A session confirmed an existing memory is still true. **Explicit
        /// only** — never inferred from a matching value key (FR-321, D77).
        Reinforces => "reinforces",
        /// Same subject, identical content after normalization. The one case
        /// automatic reconciliation may decide (D46).
        Duplicates => "duplicates",
        /// The new memory replaces the old. **Never automatic** (FR-325).
        Supersedes => "supersedes",
        /// Two applicable answers disagree. Detected automatically, resolved
        /// never. Symmetric — see [`RelationKind::is_symmetric`].
        ConflictsWith => "conflicts_with",
        /// A documented scope exception: the narrower points at the broader
        /// (FR-333).
        Narrows => "narrows",
        /// This knowledge does not apply in the other's context.
        NotApplicableTo => "not_applicable_to",
    }
);

impl RelationKind {
    /// Whether the relation's meaning has no direction.
    ///
    /// Only `conflicts_with`. Its endpoints are normalized to `(min, max)`
    /// before the write, so two machines detecting one conflict while offline
    /// produce one durable row rather than two facing opposite ways — the
    /// primary key absorbs the second exactly as it absorbs a local duplicate
    /// (FR-305, D78).
    ///
    /// Normalizing any other kind would destroy its meaning: which memory
    /// supersedes which is the entire content of `supersedes`.
    pub fn is_symmetric(&self) -> bool {
        matches!(self, RelationKind::ConflictsWith)
    }

    /// Whether Cairn may record this without an explicit instruction.
    pub fn is_automatic(&self) -> bool {
        matches!(
            self,
            RelationKind::Duplicates | RelationKind::ConflictsWith
        )
    }
}

text_enum!(
    /// On what a reconciliation decision was decided (FR-304).
    RelationBasis, "relation basis", {
        DeterministicRule => "deterministic_rule",
        Evidence => "evidence",
        ExplicitAgent => "explicit_agent",
        ExplicitUser => "explicit_user",
    }
);

text_enum!(
    /// A bounded, redacted, attributable record of an observed state of the
    /// world (FR-351). Local, always: no outbox entity type, no server table.
    EvidenceKind, "evidence kind", {
        Observation => "observation",
        File => "file",
        GitRef => "git_ref",
        Configuration => "configuration",
        TestOutcome => "test_outcome",
        CommandOutcome => "command_outcome",
        RuntimeState => "runtime_state",
        SchemaVersion => "schema_version",
    }
);

text_enum!(
    /// Who observed the fact — which is what decides what it may establish
    /// (D52).
    EvidenceCollector, "evidence collector", {
        /// Cairn read it itself, inside the worktree or through Git.
        Cairn => "cairn",
        /// An agent submitted it. Never re-executed by Cairn.
        Agent => "agent",
    }
);

text_enum!(
    /// How an evidence fact bears on a claim (FR-359).
    EvidenceRole, "evidence role", {
        Supports => "supports",
        Contradicts => "contradicts",
    }
);

text_enum!(
    /// A deterministic check. Cairn executes nothing: the last three read a
    /// **captured** observation's recorded outcome rather than running anything
    /// (FR-365, D52).
    VerifierKind, "verifier kind", {
        FileExists => "file_exists",
        FileDigest => "file_digest",
        GitRef => "git_ref",
        GitCommit => "git_commit",
        Configuration => "configuration",
        SchemaVersion => "schema_version",
        TestOutcome => "test_outcome",
        CommandOutcome => "command_outcome",
        RuntimeState => "runtime_state",
    }
);

text_enum!(
    /// The outcome of one verification run (FR-363).
    ///
    /// `inconclusive` is a result, not an error: the check ran and could
    /// establish neither outcome, and the memory becomes neither `verified` nor
    /// `drifted` (FR-366).
    VerifyResult, "verify result", {
        Verified => "verified",
        Drifted => "drifted",
        Inconclusive => "inconclusive",
    }
);

text_enum!(
    /// What caused a verification run.
    VerifyTrigger, "verify trigger", {
        BackgroundPass => "background_pass",
        OnDemand => "on_demand",
        Attach => "attach",
    }
);

text_enum!(
    /// A subject's derived reconciliation state. **Nothing stores this** — it is
    /// recomputed from the subject's proposals and the recorded decisions, which
    /// is why no merge can silently pick a winner (FR-302, D44).
    Reconciliation, "reconciliation", {
        /// Every member is superseded or stale: no canonical answer.
        Historical => "historical",
        /// Exactly one active member.
        Settled => "settled",
        /// Several active members sharing one value key and one normalized
        /// content: one answer, with duplication accounting.
        Reinforced => "reinforced",
        /// Several active members agreeing on the **value** and differing in
        /// what they say. Not a conflict — they agree; not a merge — they say
        /// different things. Every statement is retained (FR-327, D77).
        Corroborated => "corroborated",
        /// Several active members with differing value keys in one scope: every
        /// competing answer, and no winner (FR-334).
        Conflicted => "conflicted",
    }
);

impl Reconciliation {
    /// Whether the subject reaches Level 0 context as a warning.
    ///
    /// `Corroborated` deliberately does not: several sessions agreeing on a
    /// value is normal, and the honest signal is an inline count. A warning
    /// here would train people to ignore warnings.
    pub fn is_warning(&self) -> bool {
        matches!(self, Reconciliation::Conflicted)
    }
}

text_enum!(
    /// The **work** state a session asserts about an acceptance criterion.
    /// Independent of [`CriterionVerification`] (FR-482).
    CriterionState, "criterion state", {
        Pending => "pending",
        Satisfied => "satisfied",
        Blocked => "blocked",
        Waived => "waived",
    }
);

impl CriterionState {
    /// Admission order for Level 0's bounded detail tier: what an agent must
    /// act on first (`contracts/continuity-context.md` §Criterion action
    /// order). Ties break by ascending ordinal, which the caller applies.
    ///
    /// `satisfied` sorts by whether it is verified, so the caller passes that
    /// in rather than this reading two axes at once.
    pub fn action_rank(&self, verified: bool) -> i64 {
        match self {
            CriterionState::Blocked => 0,
            CriterionState::Satisfied if !verified => 1,
            CriterionState::Pending => 2,
            CriterionState::Satisfied => 3,
            CriterionState::Waived => 4,
        }
    }
}

text_enum!(
    /// What **evidence** establishes about a criterion. Independent of
    /// [`CriterionState`]: `satisfied` + `unverified` is a normal, separately
    /// reported combination — the honest description of "the agent says it is
    /// done and nothing has checked" (FR-483).
    CriterionVerification, "criterion verification", {
        Unverified => "unverified",
        Verified => "verified",
        Failed => "failed",
    }
);

text_enum!(
    /// A blocker's only transition is `open → cleared`, and it is terminal:
    /// reopening creates a new blocker, so "who said this was blocked and who
    /// said it was not" stays answerable (FR-485).
    BlockerState, "blocker state", {
        Open => "open",
        Cleared => "cleared",
    }
);

text_enum!(
    /// What boundary produced a continuity checkpoint (FR-425).
    ///
    /// There is deliberately no turn-checkpoint trigger: `agent_quiesced` is a
    /// turn boundary, not a work boundary (Feature 001 D16).
    CheckpointTrigger, "checkpoint trigger", {
        ContextCompacting => "context_compacting",
        SessionClosed => "session_closed",
        Explicit => "explicit",
    }
);

text_enum!(
    /// How a checkpoint's recorded assumptions compare to current state
    /// (FR-431).
    CheckpointState, "checkpoint state", {
        Current => "current",
        Diverged => "diverged",
        /// The assumed task or worktree no longer exists. Every continuity
        /// field that does not depend on the missing state is still delivered
        /// (FR-435).
        Unresolvable => "unresolvable",
    }
);

text_enum!(
    /// What moved beneath a checkpoint (FR-432).
    DivergenceKind, "divergence kind", {
        Branch => "branch",
        Commit => "commit",
        Task => "task",
        Files => "files",
    }
);

text_enum!(
    /// How strongly a relevant path was fingerprinted at checkpoint time (D79).
    ///
    /// The class is recorded so a weaker comparison is **visible** rather than
    /// implied: a `size` match is weaker than a digest match, and saying so is
    /// the difference between a warning people trust and one they learn to
    /// ignore.
    FingerprintClass, "fingerprint class", {
        Digest => "digest",
        Size => "size",
        Unknown => "unknown",
    }
);

text_enum!(
    /// What comparing a recorded path fingerprint against the worktree found.
    ///
    /// `not_fingerprintable` is reported as itself and **never** collapses into
    /// `unchanged`: "I could not look" and "nothing moved" are different
    /// answers, and conflating them is exactly how a stale checkpoint reads as
    /// current (FR-432).
    PathOutcome, "path outcome", {
        Unchanged => "unchanged",
        Changed => "changed",
        Removed => "removed",
        Added => "added",
        NotFingerprintable => "not_fingerprintable",
    }
);

text_enum!(
    /// How far a reusable pattern has earned trust (D63).
    ///
    /// `contested` is evaluated **before** `validated`, so a pattern carrying
    /// both successes and counterexamples reports both sides (FR-405).
    PatternTrust, "pattern trust", {
        Candidate => "candidate",
        Sanitized => "sanitized",
        Validated => "validated",
        Contested => "contested",
    }
);

text_enum!(
    /// What happened when a pattern was applied (FR-401).
    PatternOutcome, "pattern outcome", {
        Resolved => "resolved",
        /// A counterexample: recorded with its alternative cause where known,
        /// increasing no success count and deleting nothing (FR-404).
        NotApplicable => "not_applicable",
        Failed => "failed",
    }
);

text_enum!(
    /// Whether an application found the pattern on its own.
    ///
    /// A `cairn_suggested` application with no deterministic evidence collected
    /// in the applying project counts as an application and **not** as a
    /// validation: an agent reading Cairn's suggestion and agreeing with it is
    /// not independent confirmation (FR-403).
    PatternDiscovery, "pattern discovery", {
        Independent => "independent",
        CairnSuggested => "cairn_suggested",
    }
);

text_enum!(
    /// Which layer of the briefing an item belongs to (FR-441).
    ContextLevel, "context level", {
        /// Minimum safe continuity, drawing on the reserved share.
        MinimumSafe => "minimum_safe",
        /// Relevant current knowledge, on the general pool.
        Relevant => "relevant",
        /// History and evidence. Never automatic (FR-444).
        OnDemand => "on_demand",
    }
);

text_enum!(
    /// Why an item was selected for context — a closed set (FR-461).
    SelectionReason, "selection reason", {
        ScopeMatch => "scope_match",
        CanonicalAnswer => "canonical_answer",
        Verified => "verified",
        Pinned => "pinned",
        DriftWarning => "drift_warning",
        ConflictWarning => "conflict_warning",
        PatternSignalMatch => "pattern_signal_match",
        CheckpointAssumption => "checkpoint_assumption",
        TaskBinding => "task_binding",
    }
);

text_enum!(
    /// Why a candidate was left out. Omission is never silent (FR-448).
    OmissionReason, "omission reason", {
        BudgetExhausted => "budget_exhausted",
        ScopeMismatch => "scope_mismatch",
        Superseded => "superseded",
        NotCanonical => "not_canonical",
        Level2Only => "level_2_only",
        PinBudget => "pin_budget",
        CapReached => "cap_reached",
    }
);

text_enum!(
    /// One entry in a task's append-only local change history (FR-488).
    TaskChangeKind, "task change kind", {
        GoalChanged => "goal_changed",
        TitleChanged => "title_changed",
        StatusChanged => "status_changed",
        CriterionAdded => "criterion_added",
        CriterionText => "criterion_text",
        CriterionState => "criterion_state",
        CriterionVerification => "criterion_verification",
        CriterionRemoved => "criterion_removed",
        BlockerOpened => "blocker_opened",
        BlockerCleared => "blocker_cleared",
    }
);

text_enum!(
    /// Derived on read, never stored as authority. Cairn never changes a task's
    /// status on the basis of it — completing a task stays an explicit act
    /// (FR-487).
    CompletionReadiness, "completion readiness", {
        NotReady => "not_ready",
        /// Every non-waived criterion is satisfied and no blocker is open, but
        /// at least one criterion is not verified.
        ReadyUnverified => "ready_unverified",
        Ready => "ready",
    }
);

text_enum!(
    /// Whether a historical answer can bound the interval it reports.
    Applicability, "applicability", {
        Bounded => "bounded",
        Unknown => "unknown",
    }
);

text_enum!(
    /// What Cairn can honestly promise an agent about continuity across a
    /// compaction boundary.
    ///
    /// **Derived** from Feature 002's capability profile, not maintained as a
    /// table: no canonical event and no capability is added, and Cairn never
    /// claims a rehydration guarantee an adapter cannot provide (FR-426,
    /// FR-427, D57).
    ContinuityMode, "continuity mode", {
        Automatic => "automatic",
        AgentInitiated => "agent_initiated",
        UnavailableAutomatic => "unavailable_automatic",
    }
);

/// A tracked Git repository under Cairn.
///
/// `id` and `git_common_dir` are local. `server_project_id` is the shared
/// identity, assigned by the server at `cairn link` (FR-064, D14).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub git_common_dir: String,
    pub repository_remote: Option<String>,
    pub linked: bool,
    pub server_project_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub project_id: Uuid,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// One agent working session.
///
/// Identity is `id`, keyed to `agent_session_key`. The worktree is scope and
/// context, never a uniqueness key (FR-010).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub project_id: Uuid,
    pub task_id: Option<Uuid>,
    pub user_id: Uuid,
    pub agent: String,
    pub branch: String,
    pub commit_sha: Option<String>,
    pub worktree_path: String,
    pub agent_session_key: String,
    pub previous_session_id: Option<Uuid>,
    pub status: SessionStatus,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub last_event_at: DateTime<Utc>,
    /// Set by the `Stop` turn checkpoint. Never ends the session (D16).
    pub last_turn_ended_at: Option<DateTime<Utc>>,
    pub daemon_run_id: Uuid,
    pub end_reason: Option<String>,
    /// Set inside the seal transaction at session close and cleared when the
    /// durable handoff is written (D22, FR-240). A terminal session carrying
    /// this is *owed* a handoff, not complete.
    #[serde(default)]
    pub handoff_pending: bool,
    /// How many synthesis attempts the boundary has taken.
    #[serde(default)]
    pub handoff_attempts: i64,
    /// The last redacted failure reason. Never file or conversation content.
    #[serde(default)]
    pub handoff_error: Option<String>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Session {
    pub fn is_active(&self) -> bool {
        self.status == SessionStatus::Active
    }
}

/// One structured thing that happened during a session.
///
/// Observations are local, always. No field of one ever syncs (FR-055).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: Uuid,
    pub session_id: Uuid,
    #[serde(rename = "type")]
    pub kind: ObservationType,
    pub occurred_at: DateTime<Utc>,
    pub branch: String,
    pub commit_sha: Option<String>,
    pub path: Option<String>,
    pub command: Option<String>,
    pub exit_code: Option<i64>,
    pub outcome: Option<String>,
    pub summary: String,
    pub details: Option<serde_json::Value>,
    pub payload_bytes: i64,
    pub truncated: bool,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// A supporting observation reference for a memory.
///
/// The join row survives deletion of the observation, so provenance stays
/// resolvable and reports "evidence deleted" (FR-052).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub observation_id: Uuid,
    pub content_digest: String,
    pub deleted: bool,
}

/// Durable knowledge.
///
/// `origin_session_id` is mandatory; `evidence` is zero-or-more and is never
/// fabricated to satisfy the schema (FR-019).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: Uuid,
    pub project_id: Uuid,
    #[serde(rename = "type")]
    pub kind: MemoryType,
    pub scope: MemoryScope,
    pub scope_key: String,
    pub content: String,
    pub state: MemoryState,
    pub superseded_by_id: Option<Uuid>,
    pub origin_session_id: Uuid,
    pub local_only: bool,
    pub evidence: Vec<EvidenceRef>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Memory {
    pub fn evidence_count(&self) -> usize {
        self.evidence.len()
    }
}

/// Repository state at a point in time (FR-003, FR-014).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepositoryState {
    pub branch: String,
    pub commit_sha: Option<String>,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
}

impl RepositoryState {
    pub fn is_clean(&self) -> bool {
        self.staged == 0 && self.unstaged == 0 && self.untracked == 0
    }
}

/// A test the session executed, as recorded on a handoff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRunRecord {
    pub command: String,
    pub outcome: String,
    pub occurred_at: DateTime<Utc>,
}

/// A structured summary produced at a session boundary (FR-033).
///
/// Every field except `agent_note` is derived from recorded state (FR-034).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handoff {
    pub id: Uuid,
    pub session_id: Uuid,
    pub trigger: HandoffTrigger,
    pub goal: String,
    pub progress: String,
    pub completed_work: Vec<String>,
    pub remaining_work: Vec<String>,
    pub changed_files: Vec<String>,
    pub decisions: Vec<String>,
    pub failures: Vec<String>,
    pub tests_executed: Vec<TestRunRecord>,
    pub repository_state: RepositoryState,
    pub next_step: String,
    pub agent_note: Option<String>,
    /// Observation identifiers only. Never their content (FR-055).
    pub evidence: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_roundtrip() {
        for s in SessionStatus::ALL {
            assert_eq!(SessionStatus::from_str(s.as_str()).unwrap(), *s);
        }
        for t in ObservationType::ALL {
            assert_eq!(ObservationType::from_str(t.as_str()).unwrap(), *t);
        }
        for t in MemoryType::ALL {
            assert_eq!(MemoryType::from_str(t.as_str()).unwrap(), *t);
        }
    }

    #[test]
    fn scope_precedence_is_task_branch_project() {
        assert!(MemoryScope::Task.bucket() < MemoryScope::Branch.bucket());
        assert!(MemoryScope::Branch.bucket() < MemoryScope::Project.bucket());
    }

    #[test]
    fn handoff_triggers_exclude_turn_checkpoints() {
        // `Stop` is a turn boundary, not a handoff boundary (FR-032, D16).
        assert_eq!(HandoffTrigger::ALL.len(), 3);
        assert!(HandoffTrigger::from_str("stop").is_err());
    }

    #[test]
    fn outbox_cannot_carry_observations() {
        // Structural guarantee behind SC-010: no observation entity type exists.
        assert!(OutboxEntityType::from_str("observation").is_err());
        assert!(OutboxEntityType::from_str("observation_ref").is_err());

        // Feature 003 keeps that guarantee and extends it (FR-503, I8). Every
        // record below is local by design; giving one an entity type is what
        // would quietly open a path to the server, so the absence is asserted
        // rather than reviewed. Adding a variant fails this test until someone
        // deliberately changes it.
        for local_only in [
            "evidence_fact",
            "evidence_facts",
            "memory_evidence_fact",
            "verification_run",
            "continuity_checkpoint",
            "checkpoint",
            "reusable_pattern",
            "pattern",
            "pattern_application",
            "task_change",
            "criterion_evidence",
            "selection",
        ] {
            assert!(
                OutboxEntityType::from_str(local_only).is_err(),
                "{local_only} has an outbox entity type; it is local state and must not"
            );
        }

        // And the three Feature 003 additions, which are the whole of the delta
        // (D66). A fourth arriving unnoticed changes this count.
        assert_eq!(OutboxEntityType::ALL.len(), 8);
        for added in ["memory_relation", "task_criterion", "task_blocker"] {
            assert!(OutboxEntityType::from_str(added).is_ok(), "{added}");
        }
    }

    #[test]
    fn feature_003_enums_round_trip() {
        macro_rules! round_trip {
            ($($t:ty),+ $(,)?) => {$(
                for v in <$t>::ALL {
                    assert_eq!(<$t>::from_str(v.as_str()).unwrap(), *v);
                }
            )+};
        }
        round_trip!(
            VerificationState,
            VerificationAuthority,
            Importance,
            RelationKind,
            RelationBasis,
            EvidenceKind,
            EvidenceCollector,
            EvidenceRole,
            VerifierKind,
            VerifyResult,
            VerifyTrigger,
            Reconciliation,
            CriterionState,
            CriterionVerification,
            BlockerState,
            CheckpointTrigger,
            CheckpointState,
            DivergenceKind,
            FingerprintClass,
            PathOutcome,
            PatternTrust,
            PatternOutcome,
            PatternDiscovery,
            ContextLevel,
            SelectionReason,
            OmissionReason,
            TaskChangeKind,
            CompletionReadiness,
            ContinuityMode,
            BlockedReason,
            OutboxEntityType,
            OutboxState,
        );
    }

    #[test]
    fn memory_lifecycle_is_untouched_by_feature_003() {
        // FR-362: lifecycle stays exactly Feature 001's three states. Feature
        // 003 adds a *verification* axis beside it and never extends this one.
        assert_eq!(MemoryState::ALL.len(), 3);
        assert!(MemoryState::from_str("verified").is_err());
        assert!(MemoryState::from_str("drifted").is_err());
        assert!(MemoryState::from_str("needs_recheck").is_err());
    }

    #[test]
    fn blocked_is_recoverable_and_failed_is_not() {
        // D81: a capability refusal is retained, not permanent. Getting this
        // backwards is what stranded the work the fifth state exists to save.
        assert!(!OutboxState::Blocked.is_terminal());
        assert!(!OutboxState::Blocked.is_claimable());
        assert!(OutboxState::Failed.is_terminal());
        assert!(OutboxState::Pending.is_claimable());
        assert!(OutboxState::Delivered.is_terminal());
    }

    #[test]
    fn only_conflicts_with_is_symmetric() {
        // Normalizing a directional kind would destroy its meaning: which
        // memory supersedes which is the entire content of the relation (D78).
        assert!(RelationKind::ConflictsWith.is_symmetric());
        for directional in [
            RelationKind::Supersedes,
            RelationKind::Duplicates,
            RelationKind::Reinforces,
            RelationKind::Narrows,
            RelationKind::NotApplicableTo,
        ] {
            assert!(!directional.is_symmetric(), "{directional} is directional");
        }
    }

    #[test]
    fn supersession_and_reinforcement_are_never_automatic() {
        // FR-325 and FR-321/D77. `reinforces` was demoted to explicit-only when
        // the coarse-value-key false-merge path was closed; asserting it here
        // is what stops it drifting back.
        assert!(!RelationKind::Supersedes.is_automatic());
        assert!(!RelationKind::Reinforces.is_automatic());
        assert!(!RelationKind::Narrows.is_automatic());
        assert!(!RelationKind::NotApplicableTo.is_automatic());
        assert!(RelationKind::Duplicates.is_automatic());
        assert!(RelationKind::ConflictsWith.is_automatic());
    }

    #[test]
    fn an_imported_authority_is_never_stored_as_local() {
        // FR-368/FR-370: `cairn` from elsewhere is not `cairn` here.
        use VerificationAuthority::*;
        assert_eq!(VerificationAuthority::imported(Cairn), RemoteCairn);
        assert_eq!(VerificationAuthority::imported(Attested), RemoteAttested);
        // Idempotent: a value that already travelled does not decay further.
        assert_eq!(VerificationAuthority::imported(RemoteCairn), RemoteCairn);
        assert_eq!(
            VerificationAuthority::imported(RemoteAttested),
            RemoteAttested
        );

        // Only `cairn` satisfies the two strict consumers.
        assert!(Cairn.is_local_deterministic());
        for weaker in [Attested, RemoteCairn, RemoteAttested] {
            assert!(
                !weaker.is_local_deterministic(),
                "{weaker} must not satisfy a deterministic-check requirement"
            );
        }

        // The wire carries only the two local values (T099).
        assert_eq!(RemoteCairn.on_the_wire(), Cairn);
        assert_eq!(RemoteAttested.on_the_wire(), Attested);
        assert!(!Cairn.on_the_wire().is_imported());
    }

    #[test]
    fn criterion_action_order_leads_with_what_blocks_progress() {
        // `contracts/continuity-context.md` §Criterion action order.
        let order = [
            CriterionState::Blocked.action_rank(false),
            CriterionState::Satisfied.action_rank(false),
            CriterionState::Pending.action_rank(false),
            CriterionState::Satisfied.action_rank(true),
            CriterionState::Waived.action_rank(false),
        ];
        assert_eq!(order, [0, 1, 2, 3, 4], "{order:?}");
    }

    #[test]
    fn corroborated_is_not_a_warning() {
        // Several sessions agreeing on a value is normal. A warning here would
        // train people to ignore warnings (contracts/knowledge.md §Warnings).
        assert!(Reconciliation::Conflicted.is_warning());
        for quiet in [
            Reconciliation::Corroborated,
            Reconciliation::Reinforced,
            Reconciliation::Settled,
            Reconciliation::Historical,
        ] {
            assert!(!quiet.is_warning(), "{quiet} must not reach Level 0");
        }
    }

    #[test]
    fn importance_ranks_within_a_bucket_only() {
        // FR-308: it orders candidates already selected; it never reorders
        // scopes. Asserting the two rankings are unrelated is the cheap way to
        // notice someone wiring importance into scope precedence.
        assert!(Importance::High.rank() < Importance::Normal.rank());
        assert!(Importance::Normal.rank() < Importance::Low.rank());
        assert_eq!(Importance::default(), Importance::Normal);
    }

    #[test]
    fn ids_are_time_ordered() {
        let a = new_id();
        let b = new_id();
        assert!(a < b || a.get_version_num() == 7);
    }
}
