//! `SafeCanonicalEvent` — the one record that crosses the machine boundary
//! (`data-model.md` §1, `contracts/safe-events.md`).
//!
//! Everything about this module is shaped by one rule: **raw material does not
//! leave the machine that saw it.** There is no field here for a transcript, a
//! prompt, a diff, a tool's output, or a vendor's original JSON, and there is
//! deliberately no `summary` — a per-event human-readable gist is precisely the
//! field into which conversation content would leak, and kind plus typed
//! content already carries the meaning.
//!
//! Three consequences worth stating up front, because each looks like an
//! omission until you know why it is not:
//!
//! - **The union is closed.** Twenty-one kinds, each with a fixed content
//!   shape. A kind cannot carry another kind's fields, and content that does
//!   not match its kind is refused rather than ignored — ignoring it would let
//!   a client smuggle a field past the type system and into a JSONB column.
//! - **`project_id` and `account_id` are not envelope fields.** They are bound
//!   server-side from the authenticated credential and the verified session
//!   (FR-769, FR-769a). A client that could name them could attribute another
//!   account's work.
//! - **`agent_session_key` is not an envelope field and must not become one.**
//!   It is on `FORBIDDEN_SESSION_FIELDS`, the server has never had a column for
//!   it, and putting it here would weaken a standing refusal (FR-777a, SC-751).
//!   The synced session UUID travels instead.
//!
//! Bounds live here as numbers, because SC-743 and SC-733 have to be able to
//! fail (FR-773). Over-bound values are **refused, never truncated**:
//! truncating a `repo_file` could turn a path outside the repository into one
//! that looks inside it.
//!
//! This module does structure and bounds. Privacy screening — redaction
//! outcomes, secret detection, path attacks — is `validate.rs`, applied
//! identically on both sides of the boundary. Neither is sufficient alone: a
//! well-shaped event can still carry a secret, and a screened string can still
//! be twice as long as the column allows.

use crate::domain::text_enum;
use crate::domain::ParseEnumError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Bounds (data-model.md §3, contracts/safe-events.md §5)
// ---------------------------------------------------------------------------

/// The contract version this build produces. Starts at 1 (FR-742).
pub const CONTRACT_VERSION: u16 = 1;

/// `repo_file`, in bytes of UTF-8.
pub const REPO_FILE_MAX_BYTES: usize = 1024;
/// `repo_file`, in path segments.
pub const REPO_FILE_MAX_SEGMENTS: usize = 64;
/// `command_line`, `test_command`, `failure_note` — the only three free-text
/// fields in the model, each in bytes.
pub const FREE_TEXT_MAX_BYTES: usize = 512;
/// `vendor_event`, `vendor_tool`, `subagent_kind`, in characters.
pub const VENDOR_TOKEN_MAX_CHARS: usize = 64;
/// `subagent_ref`, in characters.
pub const SUBAGENT_REF_MAX_CHARS: usize = 128;
/// `subject_token`, in characters. Matches `topic_key`'s existing bound.
pub const SUBJECT_TOKEN_MAX_CHARS: usize = 128;
/// `object_token`, in characters. Matches `value_key`'s existing bound.
pub const OBJECT_TOKEN_MAX_CHARS: usize = 64;
/// Serialized event `content`.
pub const CONTENT_MAX_BYTES: usize = 8 * 1024;
/// The whole serialized event.
pub const EVENT_MAX_BYTES: usize = 16 * 1024;
/// Events per ingest batch. Server-enforced.
pub const BATCH_MAX_EVENTS: usize = 256;
/// Ingest request body. Server-enforced.
pub const BODY_MAX_BYTES: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// Closed vocabularies
// ---------------------------------------------------------------------------

text_enum!(
    /// The agents Feature 005 captures from (FR-838a).
    ///
    /// OpenCode is here as a capture agent. Cairn declines to guarantee
    /// automatic *delivery* for it — a decision about an unstable vendor
    /// surface, recorded as `declined_by_cairn` rather than as a vendor
    /// absence, which would be untrue (FR-838b).
    EventAgent, "event agent", {
        ClaudeCode => "claude_code",
        Codex => "codex",
        OpenCode => "opencode",
    }
);

text_enum!(
    /// Why a session opened.
    OpenTrigger, "open trigger", {
        Startup => "startup",
        Resume => "resume",
        Clear => "clear",
        /// A session opened immediately after a compaction. This is the
        /// post-compaction delivery opportunity for both committed agents:
        /// they re-open a session, and session open is where context is
        /// delivered.
        Compact => "compact",
        Fork => "fork",
    }
);

text_enum!(
    /// Whether a compaction was asked for or happened on its own.
    CompactionTrigger, "compaction trigger", {
        Manual => "manual",
        Auto => "auto",
    }
);

text_enum!(
    /// What happened to a file.
    ChangeKind, "change kind", {
        Created => "created",
        Modified => "modified",
        Deleted => "deleted",
        Renamed => "renamed",
    }
);

text_enum!(
    /// Whether a repository-relative file identity could be established
    /// (data-model.md §2, FR-777e–g).
    ///
    /// Three answers, and the distinction between the last two is the honest
    /// part: a path the vendor never gave us and a path that pointed outside
    /// the repository are different facts, and reporting both as "no file"
    /// would hide that Cairn is watching a tool it cannot place.
    FileIdentity, "file identity", {
        Present => "present",
        OutOfRepository => "out_of_repository",
        UnavailableFromVendor => "unavailable_from_vendor",
    }
);

text_enum!(
    /// What a tool does, independent of what a vendor calls it.
    ///
    /// The members are exactly the classes `tools::classify_tool` can already
    /// produce, so a tool's class on this boundary and its observation kind in
    /// the local store cannot disagree. `vendor_tool` carries the vendor's own
    /// name alongside, as bounded provenance.
    ToolClass, "tool class", {
        Read => "read",
        Edit => "edit",
        Execute => "execute",
        Test => "test",
        Research => "research",
        Other => "other",
    }
);

text_enum!(
    /// How a tool failed, classified rather than described.
    ///
    /// A closed classification and not free text, because consolidation counts
    /// repeats of *the same* failure — three `tool_failed` for one tool with
    /// one `failure_kind` and no subsequent success is what makes a failure
    /// worth remembering (`contracts/extraction.md` §5). Two spellings of one
    /// English sentence would never count as the same failure; two occurrences
    /// of `permission_denied` always do.
    ///
    /// `failure_note` carries the redacted detail alongside, for a human.
    FailureKind, "failure kind", {
        NotFound => "not_found",
        PermissionDenied => "permission_denied",
        Timeout => "timeout",
        NonZeroExit => "non_zero_exit",
        InvalidInput => "invalid_input",
        Unavailable => "unavailable",
        Interrupted => "interrupted",
        Unknown => "unknown",
    }
);

text_enum!(
    /// What a test run concluded.
    ///
    /// `unknown` is a first-class answer: a runner whose verdict Cairn could
    /// not read is not a pass.
    TestOutcome, "test outcome", {
        Passed => "passed",
        Failed => "failed",
        Unknown => "unknown",
    }
);

text_enum!(
    /// What kind of resource an agent went looking at.
    ///
    /// Deliberately coarse. A URL is a locator, and a locator is exactly the
    /// sort of value this boundary does not carry.
    ResourceKind, "resource kind", {
        Docs => "docs",
        Web => "web",
        Repository => "repository",
        Other => "other",
    }
);

text_enum!(
    /// The shape of a user instruction, without its words.
    InstructionKind, "instruction kind", {
        Require => "require",
        Forbid => "forbid",
        Prefer => "prefer",
        Scope => "scope",
        Correct => "correct",
    }
);

text_enum!(
    /// The shape of a decision, without its words.
    DecisionKind, "decision kind", {
        Adopt => "adopt",
        Reject => "reject",
        Defer => "defer",
        Constrain => "constrain",
        Prefer => "prefer",
        Revert => "revert",
    }
);

text_enum!(
    /// What happened to an attempted capture (data-model.md §4).
    ///
    /// One closed vocabulary shared by the local counts table, the server's
    /// funnel and the health report, so the three cannot describe the same
    /// event three ways.
    ///
    /// `capture_deadline_exceeded` is the honest one: the agent saw success and
    /// Cairn dropped the event (FR-749c). Fail-soft describes what the agent
    /// experiences, not what Cairn is allowed to know about itself.
    Disposition, "disposition", {
        Captured => "captured",
        DeclinedByPolicy => "declined_by_policy",
        CaptureDeadlineExceeded => "capture_deadline_exceeded",
        RedactionFailed => "redaction_failed",
        PrivacyRefused => "privacy_refused",
        NoSafeSemanticMapping => "no_safe_semantic_mapping",
        Spooled => "spooled",
        SpoolOverflowDropped => "spool_overflow_dropped",
        SpoolSaturatedDropped => "spool_saturated_dropped",
        Transmitted => "transmitted",
        Accepted => "accepted",
        RejectedByServer => "rejected_by_server",
        Persisted => "persisted",
    }
);

text_enum!(
    /// Where in the pipeline something happened (FR-851, FR-858).
    ///
    /// A capture failure must be visible *with the stage it failed at*: "capture
    /// is broken" and "the server refused what capture produced" call for
    /// different actions, and a single health flag conflates them.
    ///
    /// The delivery stages are in the same vocabulary because health is one
    /// matrix. A cell has to be able to say "context was generated but never
    /// transmitted", which needs both stages nameable side by side.
    PipelineStage, "pipeline stage", {
        Configured => "configured",
        Installed => "installed",
        RuntimeHookFired => "runtime_hook_fired",
        EventReceived => "event_received",
        EventParsed => "event_parsed",
        SafeEventAccepted => "safe_event_accepted",
        ServerPersistedEvent => "server_persisted_event",
        ContextGenerated => "context_generated",
        ContextTransmitted => "context_transmitted",
        ContextReceiptConfirmed => "context_receipt_confirmed",
    }
);

text_enum!(
    /// Why a capture was declined or failed, as a reason rather than a message.
    DeclineReason, "decline reason", {
        NoSafeSemanticMapping => "no_safe_semantic_mapping",
        AmbiguousClassification => "ambiguous_classification",
        InsufficientVocabulary => "insufficient_vocabulary",
        VendorUnavailable => "vendor_unavailable",
        PolicyExcluded => "policy_excluded",
    }
);

text_enum!(
    /// The twenty-one canonical event kinds (data-model.md §1.2).
    ///
    /// Closed and versioned. Adding a kind does not invalidate stored events
    /// (FR-743); a server that cannot store a kind refuses it in a way the
    /// client recognises and defers, the way the existing capability mechanism
    /// already handles entity types (FR-775).
    ///
    /// The seven Feature 001–003 lifecycle events map onto this set without
    /// loss, so handoff generation, checkpointing and context delivery keep
    /// working (FR-744). See `lifecycle.rs`.
    EventKind, "event kind", {
        SessionOpened => "session_opened",
        SessionClosed => "session_closed",
        ContextCompacting => "context_compacting",
        ContextCompacted => "context_compacted",
        SubagentStarted => "subagent_started",
        SubagentCompleted => "subagent_completed",
        ToolStarted => "tool_started",
        ToolSucceeded => "tool_succeeded",
        ToolFailed => "tool_failed",
        FileRead => "file_read",
        FileChanged => "file_changed",
        CommandExecuted => "command_executed",
        TestExecuted => "test_executed",
        TestResult => "test_result",
        ResearchActivity => "research_activity",
        UserInstructionSignal => "user_instruction_signal",
        DecisionSignal => "decision_signal",
        CaptureDeclined => "capture_declined",
        CaptureFailed => "capture_failed",
        SessionResumed => "session_resumed",
        AgentQuiesced => "agent_quiesced",
    }
);

impl EventKind {
    /// Whether the capacity policy may shed this event under overflow
    /// (`data-model.md` §5, FR-785).
    ///
    /// Session open, close, resume and the two compaction events are
    /// **boundary class** and are never dropped. Every other event is
    /// interpreted relative to the session structure those establish, so
    /// shedding one would not lose an event — it would corrupt the reading of
    /// everything still queued.
    pub fn is_boundary_class(&self) -> bool {
        matches!(
            self,
            EventKind::SessionOpened
                | EventKind::SessionClosed
                | EventKind::SessionResumed
                | EventKind::ContextCompacting
                | EventKind::ContextCompacted
        )
    }

    /// The `boundary_class` column's value.
    pub fn boundary_class(&self) -> i64 {
        i64::from(self.is_boundary_class())
    }
}

// ---------------------------------------------------------------------------
// Per-kind content (data-model.md §1.3)
// ---------------------------------------------------------------------------

/// A token drawn from the session's derived vocabulary.
///
/// **Not free text, and the distinction is the whole reason a decision can
/// cross this boundary at all.** A token is key-shaped and must additionally
/// appear in the vocabulary the session's own events establish — its file and
/// module tokens, command verbs, test identifiers and project keys. A
/// sentence's words are not in that set, and neither is a credential
/// (`contracts/extraction.md` §13).
///
/// This type enforces the *shape*. Vocabulary membership is checked against the
/// session, by the client when constructing and independently by the server, in
/// `validate.rs` — a shape check here cannot know what a session has seen.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VocabToken(String);

impl VocabToken {
    /// Accept a token if it is key-shaped and within `max_chars`.
    ///
    /// Key-shaped means `[a-z0-9_]` segments separated by dots, with no empty
    /// segment and no leading or trailing dot — the same shape `topic_key` and
    /// `value_key` already use, so a token and the key it becomes cannot
    /// disagree about what is legal.
    pub fn new(raw: &str, max_chars: usize) -> Result<Self, EventRefusal> {
        if raw.is_empty() {
            return Err(EventRefusal::TokenEmpty);
        }
        if raw.chars().count() > max_chars {
            return Err(EventRefusal::TokenTooLong { max: max_chars });
        }
        for segment in raw.split('.') {
            if segment.is_empty() {
                return Err(EventRefusal::TokenNotKeyShaped);
            }
            if !segment
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                return Err(EventRefusal::TokenNotKeyShaped);
            }
        }
        Ok(Self(raw.to_string()))
    }

    pub fn subject(raw: &str) -> Result<Self, EventRefusal> {
        Self::new(raw, SUBJECT_TOKEN_MAX_CHARS)
    }

    pub fn object(raw: &str) -> Result<Self, EventRefusal> {
        Self::new(raw, OBJECT_TOKEN_MAX_CHARS)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VocabToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The content of one event, by kind.
///
/// Every variant is named for the kind or kinds it belongs to, and
/// [`EventContent::kinds`] is the one place that mapping lives. Content and
/// kind are checked against each other on construction and again on ingest;
/// mismatched content is refused, never ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum EventContent {
    /// `session_opened`, `session_resumed`
    SessionOpen { open_trigger: OpenTrigger },
    /// `session_closed`
    SessionClose { close_reason: String },
    /// `context_compacting`, `context_compacted`
    Compaction {
        compaction_trigger: CompactionTrigger,
    },
    /// `subagent_started`, `subagent_completed`
    Subagent {
        subagent_ref: String,
        subagent_kind: String,
        parent_session_seq: u64,
    },
    /// `tool_started`, `tool_succeeded`
    Tool {
        vendor_tool: String,
        tool_class: ToolClass,
    },
    /// `tool_failed`
    ToolFailure {
        vendor_tool: String,
        tool_class: ToolClass,
        failure_kind: FailureKind,
        failure_note: Option<String>,
        exit_status: Option<i32>,
    },
    /// `file_read`, `file_changed`
    File {
        /// Absent unless `file_identity` is `present` — a path outside the
        /// repository, or one the vendor never supplied, has no
        /// repository-relative identity to record.
        repo_file: Option<String>,
        /// Present only for a rename.
        repo_file_from: Option<String>,
        /// Absent for `file_read`, which does not change anything.
        change_kind: Option<ChangeKind>,
        file_identity: FileIdentity,
    },
    /// `command_executed`
    Command {
        command_line: String,
        exit_status: Option<i32>,
    },
    /// `test_executed`
    TestInvocation { test_command: String },
    /// `test_result`
    TestVerdict {
        test_outcome: TestOutcome,
        exit_status: Option<i32>,
        tests_total: Option<u32>,
        tests_failed: Option<u32>,
    },
    /// `research_activity`
    Research { resource_kind: ResourceKind },
    /// `user_instruction_signal`
    Instruction {
        instruction_kind: InstructionKind,
        subject_token: VocabToken,
        object_token: VocabToken,
        justified_by_seq: Option<u64>,
        lexicon_version: u16,
    },
    /// `decision_signal`
    Decision {
        decision_kind: DecisionKind,
        subject_token: VocabToken,
        object_token: VocabToken,
        justified_by_seq: Option<u64>,
        lexicon_version: u16,
    },
    /// `capture_declined`, `capture_failed`
    CaptureOutcome {
        disposition: Disposition,
        stage: PipelineStage,
        decline_reason: DeclineReason,
    },
    /// `agent_quiesced` — carries nothing, and that is the whole content.
    None,
}

impl EventContent {
    /// The kinds this content shape is legal for.
    ///
    /// One function rather than a match at each call site, so "which kinds
    /// share a shape" has exactly one answer.
    pub fn kinds(&self) -> &'static [EventKind] {
        match self {
            EventContent::SessionOpen { .. } => {
                &[EventKind::SessionOpened, EventKind::SessionResumed]
            }
            EventContent::SessionClose { .. } => &[EventKind::SessionClosed],
            EventContent::Compaction { .. } => {
                &[EventKind::ContextCompacting, EventKind::ContextCompacted]
            }
            EventContent::Subagent { .. } => {
                &[EventKind::SubagentStarted, EventKind::SubagentCompleted]
            }
            EventContent::Tool { .. } => &[EventKind::ToolStarted, EventKind::ToolSucceeded],
            EventContent::ToolFailure { .. } => &[EventKind::ToolFailed],
            EventContent::File { .. } => &[EventKind::FileRead, EventKind::FileChanged],
            EventContent::Command { .. } => &[EventKind::CommandExecuted],
            EventContent::TestInvocation { .. } => &[EventKind::TestExecuted],
            EventContent::TestVerdict { .. } => &[EventKind::TestResult],
            EventContent::Research { .. } => &[EventKind::ResearchActivity],
            EventContent::Instruction { .. } => &[EventKind::UserInstructionSignal],
            EventContent::Decision { .. } => &[EventKind::DecisionSignal],
            EventContent::CaptureOutcome { .. } => {
                &[EventKind::CaptureDeclined, EventKind::CaptureFailed]
            }
            EventContent::None => &[EventKind::AgentQuiesced],
        }
    }

    /// Whether this content may accompany `kind`.
    pub fn is_legal_for(&self, kind: EventKind) -> bool {
        self.kinds().contains(&kind)
    }

    /// Check every bound this content's fields are subject to.
    ///
    /// Structure and size only. Whether a string carries a secret, and whether
    /// a `repo_file` escapes the repository, are `validate.rs`'s questions.
    pub fn check_bounds(&self) -> Result<(), EventRefusal> {
        match self {
            EventContent::SessionOpen { .. }
            | EventContent::Compaction { .. }
            | EventContent::Research { .. }
            | EventContent::TestVerdict { .. }
            | EventContent::CaptureOutcome { .. }
            | EventContent::None => Ok(()),

            EventContent::SessionClose { close_reason } => {
                bounded_chars("close_reason", close_reason, VENDOR_TOKEN_MAX_CHARS)
            }
            EventContent::Subagent {
                subagent_ref,
                subagent_kind,
                ..
            } => {
                bounded_chars("subagent_ref", subagent_ref, SUBAGENT_REF_MAX_CHARS)?;
                bounded_chars("subagent_kind", subagent_kind, VENDOR_TOKEN_MAX_CHARS)
            }
            EventContent::Tool { vendor_tool, .. } => {
                bounded_chars("vendor_tool", vendor_tool, VENDOR_TOKEN_MAX_CHARS)
            }
            EventContent::ToolFailure {
                vendor_tool,
                failure_note,
                ..
            } => {
                bounded_chars("vendor_tool", vendor_tool, VENDOR_TOKEN_MAX_CHARS)?;
                match failure_note {
                    Some(note) => bounded_bytes("failure_note", note, FREE_TEXT_MAX_BYTES),
                    None => Ok(()),
                }
            }
            EventContent::File {
                repo_file,
                repo_file_from,
                change_kind,
                file_identity,
            } => {
                for (field, value) in [("repo_file", repo_file), ("repo_file_from", repo_file_from)]
                {
                    if let Some(v) = value {
                        bounded_bytes(field, v, REPO_FILE_MAX_BYTES)?;
                        if v.split('/').count() > REPO_FILE_MAX_SEGMENTS {
                            return Err(EventRefusal::TooManySegments {
                                field,
                                max: REPO_FILE_MAX_SEGMENTS,
                            });
                        }
                    }
                }
                // A file identity that is not `present` has no path, and a
                // `present` one must have exactly one. Either mismatch would
                // leave a reader unable to tell "outside the repository" from
                // "we forgot to record it".
                match (file_identity, repo_file) {
                    (FileIdentity::Present, None) => {
                        return Err(EventRefusal::FileIdentityMismatch)
                    }
                    (FileIdentity::OutOfRepository, Some(_))
                    | (FileIdentity::UnavailableFromVendor, Some(_)) => {
                        return Err(EventRefusal::FileIdentityMismatch)
                    }
                    _ => {}
                }
                // A rename source without a rename is a field nothing reads.
                if repo_file_from.is_some() && *change_kind != Some(ChangeKind::Renamed) {
                    return Err(EventRefusal::RenameSourceWithoutRename);
                }
                Ok(())
            }
            EventContent::Command { command_line, .. } => {
                bounded_bytes("command_line", command_line, FREE_TEXT_MAX_BYTES)
            }
            EventContent::TestInvocation { test_command } => {
                bounded_bytes("test_command", test_command, FREE_TEXT_MAX_BYTES)
            }
            // The tokens were bounded when they were constructed; a
            // `VocabToken` cannot exist over its limit.
            EventContent::Instruction { .. } | EventContent::Decision { .. } => Ok(()),
        }
    }
}

fn bounded_chars(field: &'static str, value: &str, max: usize) -> Result<(), EventRefusal> {
    if value.chars().count() > max {
        return Err(EventRefusal::TooLong { field, max });
    }
    Ok(())
}

fn bounded_bytes(field: &'static str, value: &str, max: usize) -> Result<(), EventRefusal> {
    if value.len() > max {
        return Err(EventRefusal::TooLong { field, max });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The event
// ---------------------------------------------------------------------------

/// The single record that crosses the machine boundary.
///
/// `occurred_at` is the client's clock and is **advisory only**. Nothing orders
/// by it and nothing derives identity from it (FR-780); the server's
/// `received_at` is what anything time-ordered actually uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeCanonicalEvent {
    pub event_id: Uuid,
    pub contract_version: u16,
    pub kind: EventKind,
    pub agent: EventAgent,
    /// The vendor's own event name, sanitized. Provenance only (FR-724):
    /// nothing routes on it.
    pub vendor_event: Option<String>,
    /// The **synced** Cairn session id, which the server already holds. Never
    /// the vendor's session key.
    pub session_id: Uuid,
    /// Per-session ordinal, assigned by the daemon inside the transaction that
    /// spools the event. See [`crate::eventid`].
    pub session_seq: u64,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    /// Absent for kinds that carry none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<EventContent>,
}

impl SafeCanonicalEvent {
    /// Every structural rule this event is subject to, applied in one place.
    ///
    /// Run by the client when constructing and by the server independently on
    /// ingest. Independently is the operative word: a client's validation is a
    /// courtesy, and the server that trusted it would be trusting whoever
    /// wrote the client.
    pub fn validate(&self) -> Result<(), EventRefusal> {
        if self.contract_version == 0 {
            return Err(EventRefusal::ContractVersionUnsupported {
                found: self.contract_version,
            });
        }
        if let Some(v) = &self.vendor_event {
            bounded_chars("vendor_event", v, VENDOR_TOKEN_MAX_CHARS)?;
        }

        match (&self.content, self.kind) {
            // `agent_quiesced` carries nothing, and an absent `content` is the
            // same statement as `EventContent::None`. Both are accepted so a
            // wire form that omits the field and one that spells it out agree.
            (None, EventKind::AgentQuiesced) => {}
            (None, kind) => return Err(EventRefusal::ContentMissing { kind }),
            (Some(content), kind) => {
                if !content.is_legal_for(kind) {
                    return Err(EventRefusal::ContentKindMismatch { kind });
                }
                content.check_bounds()?;
            }
        }

        let serialized = serde_json::to_vec(self).map_err(|_| EventRefusal::NotSerializable)?;
        if serialized.len() > EVENT_MAX_BYTES {
            return Err(EventRefusal::TooLong {
                field: "event",
                max: EVENT_MAX_BYTES,
            });
        }
        if let Some(content) = &self.content {
            let bytes = serde_json::to_vec(content).map_err(|_| EventRefusal::NotSerializable)?;
            if bytes.len() > CONTENT_MAX_BYTES {
                return Err(EventRefusal::TooLong {
                    field: "content",
                    max: CONTENT_MAX_BYTES,
                });
            }
        }
        Ok(())
    }

    pub fn boundary_class(&self) -> i64 {
        self.kind.boundary_class()
    }
}

/// Why an event was refused.
///
/// **Carries no payload content, at any variant.** A refusal names the field
/// and the bound, never the value that broke it — a rejection record that
/// quoted the offending string would carry across the boundary exactly the
/// material the refusal exists to keep on this side (FR-741, FR-749d).
///
/// `Debug` and `Display` are both structurally incapable of leaking, because
/// there is nothing in the type to leak: no variant holds a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EventRefusal {
    #[error("{field} exceeds its bound of {max}")]
    TooLong { field: &'static str, max: usize },
    #[error("{field} has more than {max} path segments")]
    TooManySegments { field: &'static str, max: usize },
    #[error("a {kind} event carries no content")]
    ContentMissing { kind: EventKind },
    #[error("the content does not belong to a {kind} event")]
    ContentKindMismatch { kind: EventKind },
    #[error("file_identity does not agree with whether a repo_file is present")]
    FileIdentityMismatch,
    #[error("a rename source was given for a change that is not a rename")]
    RenameSourceWithoutRename,
    #[error("a vocabulary token is empty")]
    TokenEmpty,
    #[error("a vocabulary token exceeds its bound of {max}")]
    TokenTooLong { max: usize },
    #[error("a vocabulary token is not key-shaped")]
    TokenNotKeyShaped,
    #[error("contract version {found} is not supported")]
    ContractVersionUnsupported { found: u16 },
    #[error("the event could not be serialized")]
    NotSerializable,
}

impl EventRefusal {
    /// The wire vocabulary this refusal reports as
    /// (`contracts/safe-events.md` §7.2).
    pub fn code(&self) -> &'static str {
        match self {
            EventRefusal::TooLong { .. } | EventRefusal::TooManySegments { .. } => "bound_exceeded",
            EventRefusal::ContentMissing { .. }
            | EventRefusal::ContentKindMismatch { .. }
            | EventRefusal::FileIdentityMismatch
            | EventRefusal::RenameSourceWithoutRename
            | EventRefusal::NotSerializable => "malformed_event",
            EventRefusal::TokenEmpty
            | EventRefusal::TokenTooLong { .. }
            | EventRefusal::TokenNotKeyShaped => "token_not_in_vocabulary",
            EventRefusal::ContractVersionUnsupported { .. } => "contract_version_unsupported",
        }
    }
}
