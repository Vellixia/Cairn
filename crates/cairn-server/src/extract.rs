//! Semantic extraction (`contracts/extraction.md`; FR-763a–b, FR-805a–f).
//!
//! The extractor **proposes**; Cairn governs (`consolidate.rs`). Everything in
//! this module is a pure function of events that already crossed the privacy
//! boundary: no network, no model, no configuration that changes an output.
//!
//! # What a proposal cannot say
//!
//! [`CandidateProposal`] has no field for durability, verification,
//! supersession, scope, authorization or a privacy verdict. That is the design,
//! not an omission (FR-805b): an extractor cannot assert what it has no field
//! to assert, and structural prevention beats a procedural rule that somebody
//! has to keep remembering. The same reasoning puts the *governance* decisions
//! in `consolidate.rs` and not behind this trait.
//!
//! # Two rule tiers, and why the aggregator is not an extractor
//!
//! R1, R2, R4 and R7 are **session rules**: they read one session's ordered
//! events and [`DeterministicV1`] evaluates them from an [`ExtractionInput`].
//!
//! R3, R5, R6 and R8 need evidence across sessions — "≥3 sessions", "≥2
//! sessions". A session-scoped `ExtractionInput` cannot see that, and widening
//! it is not available: FR-805a1 confines an extraction request to one project
//! and one account context, and SC-749 tests it. So those four are evaluated by
//! [`aggregate`], **Cairn's own deterministic aggregator**, over data
//! `consolidate.rs` reads for one project. No extractor ever sees a
//! cross-session corpus, and the rules that most resemble policy claims about a
//! project are the ones no extractor influences at all.
//!
//! # Digests, not command text, in a project rule's value key
//!
//! `VALUE_KEY_MAX_CHARS` is 64 and `command_line` is up to 512 bytes, so R3,
//! R5 and R6 keying on the command itself would make a realistic command refuse
//! its own candidate as `key_normalization_failed`. They key on a bounded
//! digest instead (`contracts/extraction.md` §4).

use cairn_core::domain::{KnowledgeDomain, MemoryType};
use cairn_core::event::{
    EventContent, EventKind, FailureKind, InstructionKind, SafeCanonicalEvent, TestOutcome,
};
use cairn_core::knowledge::normalize_topic_key;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Scoping newtypes
// ---------------------------------------------------------------------------

/// The one project an extraction request is confined to (FR-805a1).
///
/// A newtype rather than a bare `Uuid` because the three identifiers an
/// extraction request carries are all UUIDs and all mean different things; a
/// signature taking three bare `Uuid`s is one transposition away from handing
/// an extractor another project's scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectRef(pub Uuid);

/// The one account context an extraction request is confined to (FR-805a1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountRef(pub Uuid);

/// The session whose ordered events a session rule reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionRef(pub Uuid);

// ---------------------------------------------------------------------------
// Bounds (`data-model.md` §3)
// ---------------------------------------------------------------------------

/// Events one extraction request may carry.
pub const EXTRACTION_MAX_EVENTS: usize = 200;

/// Serialized size one extraction request may carry.
pub const EXTRACTION_MAX_BYTES: usize = 256 * 1024;

/// A candidate's content bound (`contracts/extraction.md` §2).
pub const CANDIDATE_CONTENT_MAX_BYTES: usize = 2048;

/// Source events one proposal may cite.
///
/// Evidence is additive across re-executions, so a bound here costs nothing —
/// a later pass that saw more events adds rows to `candidate_source_events`
/// without changing the candidate. What it buys is a ceiling on what one
/// malformed proposal can make the governance loop verify.
pub const PROPOSAL_MAX_SOURCE_EVENTS: usize = 64;

/// Sessions the project aggregator reads.
pub const AGGREGATE_MAX_SESSIONS: i64 = 50;

/// Events the project aggregator reads, across all of those sessions.
pub const AGGREGATE_MAX_EVENTS: i64 = 5_000;

/// How many commands a repeated-procedure sequence may contain (R5).
///
/// A longer sequence would not fit a candidate's content bound, and a sequence
/// that long repeating byte-for-byte across sessions is not the pattern R5 is
/// looking for.
const PROCEDURE_MAX_STEPS: usize = 12;

/// How many file names R1 and R4 name in their content.
const CONTENT_MAX_FILES: usize = 8;

/// Characters of digest a project rule's value key carries.
///
/// Sixty-four bits of SHA-256, hex-encoded. Far inside `VALUE_KEY_MAX_CHARS`,
/// and this is a naming function rather than a security one: two different
/// commands must get two different keys, which 64 bits gives comfortably for
/// the number of distinct commands one project runs.
const DIGEST_CHARS: usize = 16;

// ---------------------------------------------------------------------------
// The interface (`contracts/extraction.md` §2)
// ---------------------------------------------------------------------------

/// Why an extraction request or its output was rejected before governance ran.
///
/// Both variants name a term from the consolidation refusal vocabulary
/// (`contracts/consolidation.md` §9), and neither carries the material that
/// caused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExtractError {
    /// More events, or more bytes, than one request may carry.
    #[error("the extraction request exceeds a declared bound")]
    BoundExceeded,
    /// The extractor returned something that is not a usable proposal.
    #[error("the extractor returned output Cairn cannot use")]
    MalformedOutput,
}

impl ExtractError {
    /// The fixed refusal term this error is recorded under.
    pub fn reason(&self) -> &'static str {
        match self {
            ExtractError::BoundExceeded => "bound_exceeded",
            ExtractError::MalformedOutput => "extractor_malformed_output",
        }
    }
}

/// **The complete description of what any extractor may see** (FR-805a).
///
/// Only approved safe events, already scoped to one project and one account
/// context. Extraction is not exempt from the membership guard every other read
/// path has, and there is no field here through which a wider corpus could
/// arrive.
#[derive(Debug, Clone)]
pub struct ExtractionInput {
    pub project_ref: ProjectRef,
    pub account_ref: AccountRef,
    pub session_ref: SessionRef,
    events: Vec<SafeCanonicalEvent>,
}

impl ExtractionInput {
    /// Build a request, enforcing both bounds.
    ///
    /// The constructor is the only way in and the field is private, so a bound
    /// cannot be skipped by assembling the struct literally. The byte bound is
    /// measured on the serialized form because that is the thing a hosted
    /// extractor would be sent; measuring the in-memory size would be a
    /// different number under the same name.
    pub fn new(
        project_ref: ProjectRef,
        account_ref: AccountRef,
        session_ref: SessionRef,
        events: Vec<SafeCanonicalEvent>,
    ) -> Result<Self, ExtractError> {
        if events.len() > EXTRACTION_MAX_EVENTS {
            return Err(ExtractError::BoundExceeded);
        }
        let serialized = serde_json::to_vec(&events).map_err(|_| ExtractError::MalformedOutput)?;
        if serialized.len() > EXTRACTION_MAX_BYTES {
            return Err(ExtractError::BoundExceeded);
        }
        Ok(Self {
            project_ref,
            account_ref,
            session_ref,
            events,
        })
    }

    /// The events, in the order they happened.
    pub fn events(&self) -> &[SafeCanonicalEvent] {
        &self.events
    }

    /// The one project this request is confined to (FR-805a1).
    pub fn project(&self) -> ProjectRef {
        self.project_ref
    }

    /// The one account context this request is confined to (FR-805a1).
    ///
    /// Read by governance rather than by extraction: the owner of anything
    /// personal comes from the account the server bound at ingest, never from a
    /// proposal, and this is where that binding travels (FR-810a).
    pub fn account(&self) -> AccountRef {
        self.account_ref
    }
}

/// What an extractor proposes, and the whole of what it may propose.
///
/// No durability, no verification, no supersession, no scope, no authorization,
/// no privacy verdict (FR-805b). `proposed_domain` is **advisory**: Cairn
/// resolves the domain from the project and session context and may resolve it
/// differently, or refuse it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateProposal {
    pub kind: MemoryType,
    pub content: String,
    /// PROPOSED — normalized by Cairn before any use.
    pub topic_key: String,
    /// PROPOSED — normalized by Cairn before any use.
    pub value_key: String,
    /// Verified by Cairn before any use (FR-805c).
    pub source_event_ids: Vec<Uuid>,
    /// Advisory only.
    pub proposed_domain: KnowledgeDomain,
}

impl CandidateProposal {
    /// The shape checks Cairn runs before governance sees a proposal.
    ///
    /// Shape only: whether the content is safe and whether the keys name
    /// anything are later gates, and are not this function's business.
    pub fn check_shape(&self) -> Result<(), ExtractError> {
        if self.content.trim().is_empty()
            || self.content.len() > CANDIDATE_CONTENT_MAX_BYTES
            || self.topic_key.trim().is_empty()
            || self.value_key.trim().is_empty()
            || self.source_event_ids.is_empty()
            || self.source_event_ids.len() > PROPOSAL_MAX_SOURCE_EVENTS
        {
            return Err(ExtractError::MalformedOutput);
        }
        Ok(())
    }
}

/// The replaceable extraction step (FR-805f).
///
/// Nothing in Feature 005 depends on a particular extractor or on a hosted one
/// existing. Replacing the implementation changes extraction quality and
/// nothing else in the pipeline.
pub trait SemanticExtractor: Send + Sync {
    /// What `consolidation_runs.extractor_kind` records for a pass this ran.
    fn kind(&self) -> &'static str;

    fn extract(&self, input: &ExtractionInput) -> Result<Vec<CandidateProposal>, ExtractError>;
}

// ---------------------------------------------------------------------------
// Tokens and digests
// ---------------------------------------------------------------------------

/// A single key-shaped token, or nothing.
///
/// The lenient normalizer is right here: this builds a *proposed* key, and the
/// strict normalizer refuses rather than repairs when Cairn later decides what
/// the key actually is (`consolidate.rs` gate 2). Dropping a character while
/// proposing and refusing it while deciding are two different jobs.
fn token(raw: &str) -> Option<String> {
    normalize_topic_key(raw).filter(|t| !t.is_empty())
}

/// The last path component, without its extension.
fn file_token(path: &str) -> Option<String> {
    let last = path.rsplit('/').next().unwrap_or(path);
    let stem = last.rsplit_once('.').map_or(last, |(stem, _)| stem);
    token(stem)
}

/// The area a path belongs to: its first directory, or the file itself when
/// there is no directory to name.
fn module_token(path: &str) -> Option<String> {
    match path.split_once('/') {
        Some((head, _)) => token(head),
        None => file_token(path),
    }
}

/// Non-flag words of a command line, leading component first, each reduced to
/// a path-free, extension-free stem.
///
/// A flag says how, not what. A path-shaped invocation contributes its final
/// component, so `./scripts/deploy.sh` is `deploy`. This mirrors the session
/// vocabulary's own derivation (`cairn_core::vocabulary`) deliberately: a rule
/// that named things differently from the vocabulary would produce keys the
/// vocabulary could never justify.
fn command_words(line: &str, how_many: usize) -> Vec<String> {
    line.split_whitespace()
        .take(how_many)
        .filter(|w| !w.starts_with('-'))
        .filter_map(|word| {
            let last = word.rsplit('/').next().unwrap_or(word);
            let stem = last.rsplit_once('.').map_or(last, |(stem, _)| stem);
            token(stem)
        })
        .collect()
}

/// The verb a command is known by: its leading non-flag word.
fn command_verb(line: &str) -> Option<String> {
    command_words(line, 2).into_iter().next()
}

/// The suite a test invocation names.
///
/// The **last** non-flag word among the first four, because a suite identifier
/// is usually the third or fourth word — `cargo test -p cairn-core` names
/// `cairn_core`, `pytest tests/unit` names `unit`. A runner invoked with no
/// suite at all (`npm test`) names its subcommand, which is the honest answer:
/// the project has one unnamed suite and this is it.
fn suite_token(test_command: &str) -> Option<String> {
    command_words(test_command, 4).pop()
}

/// A bounded, key-shaped digest of some text (R3, R5, R6).
fn digest_token(parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hasher.update([0x1f]);
        }
        hasher.update(part.as_bytes());
    }
    hex::encode(hasher.finalize())[..DIGEST_CHARS].to_string()
}

/// Render a list of names for a candidate's content, bounded.
fn name_list(names: &[String]) -> String {
    if names.len() <= CONTENT_MAX_FILES {
        return names
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ");
    }
    let shown = names[..CONTENT_MAX_FILES]
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{shown} and {} more", names.len() - CONTENT_MAX_FILES)
}

/// The most frequent entry, ties broken by first appearance.
///
/// Both halves matter: a count alone leaves ties to iteration order, and an
/// order alone ignores the evidence. Together they are total.
fn dominant(items: &[String]) -> Option<String> {
    let mut counts: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (position, item) in items.iter().enumerate() {
        let entry = counts.entry(item.as_str()).or_insert((0, position));
        entry.0 += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, (count, first))| (*count, std::cmp::Reverse(*first)))
        .map(|(item, _)| item.to_string())
}

/// The distinct entries, in first-seen order.
fn distinct(items: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    items
        .iter()
        .filter(|item| seen.insert((*item).clone()))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// The baseline extractor (`contracts/extraction.md` §3, §4, §13.5)
// ---------------------------------------------------------------------------

/// The deterministic, rule-based baseline: the default and only supported
/// extractor Feature 005 ships.
///
/// Pure. No network, no model, no configuration that changes its output. It
/// produces fewer and blunter claims than a model would, and that is the
/// accepted trade: the knowledge it produces is *checkable against the events
/// that produced it*, which is what makes its provenance mean anything.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicV1;

impl SemanticExtractor for DeterministicV1 {
    fn kind(&self) -> &'static str {
        "deterministic_v1"
    }

    fn extract(&self, input: &ExtractionInput) -> Result<Vec<CandidateProposal>, ExtractError> {
        Ok(session_rules(input.session_ref, input.events()))
    }
}

/// R1, R2, R4 and R7 over one session's ordered events.
///
/// Exposed to `consolidate.rs` as well as to the trait implementation, because
/// gate 5a re-derives a proposal's key pair from the events it cites using
/// **Cairn's own** rules — and "Cairn's own rules" has to be the same function,
/// or the gate would be checking the extractor against a second opinion nobody
/// maintains.
pub fn session_rules(session: SessionRef, events: &[SafeCanonicalEvent]) -> Vec<CandidateProposal> {
    let mut out = Vec::new();
    out.extend(rule_1_fix_confirmed_by_tests(events));
    out.extend(rule_2_persistent_failure(events));
    out.extend(rule_4_decision_near_change(session, events));
    out.extend(rule_7_recorded_decision(events));
    out.retain(|p| p.check_shape().is_ok());
    out
}

/// A `file_changed` that established a repository-relative identity.
///
/// A path the vendor never supplied and a path outside the repository are both
/// real answers and neither names a file, so neither contributes to a rule.
fn changed_file(event: &SafeCanonicalEvent) -> Option<&str> {
    if event.kind != EventKind::FileChanged {
        return None;
    }
    match event.content.as_ref()? {
        EventContent::File {
            repo_file: Some(path),
            file_identity: cairn_core::event::FileIdentity::Present,
            ..
        } => Some(path.as_str()),
        _ => None,
    }
}

/// **R1 — Fix confirmed by tests.** `test_result(failed) … file_changed(F)+ …
/// test_result(passed)`.
///
/// The strongest signal in the model: a failure, a change, and evidence the
/// change worked. This is the rule that carries the end-to-end acceptance
/// scenario.
///
/// The suite is named by the nearest preceding `test_executed`, because
/// `test_result` carries no suite of its own. A verdict with no invocation
/// before it names no suite, and the rule emits nothing rather than inventing a
/// key that two unrelated suites would then share. **The invocation is cited as
/// a source event**, which matters for more than provenance: gate 5a
/// re-derives the key pair from the cited events alone, so a citation that
/// omitted the invocation would refuse its own candidate.
fn rule_1_fix_confirmed_by_tests(events: &[SafeCanonicalEvent]) -> Vec<CandidateProposal> {
    let mut out = Vec::new();
    let mut suite: Option<(String, Uuid)> = None;
    let mut open: Option<(Uuid, String, Uuid)> = None;
    let mut changed: Vec<(String, Uuid)> = Vec::new();

    for event in events {
        match (&event.kind, event.content.as_ref()) {
            (EventKind::TestExecuted, Some(EventContent::TestInvocation { test_command })) => {
                if let Some(name) = suite_token(test_command) {
                    suite = Some((name, event.event_id));
                }
            }
            (
                EventKind::TestResult,
                Some(EventContent::TestVerdict {
                    test_outcome: TestOutcome::Failed,
                    ..
                }),
            ) => {
                changed.clear();
                open = suite
                    .as_ref()
                    .map(|(name, at)| (event.event_id, name.clone(), *at));
            }
            (
                EventKind::TestResult,
                Some(EventContent::TestVerdict {
                    test_outcome: TestOutcome::Passed,
                    ..
                }),
            ) => {
                if let Some((failed_at, name, suite_event)) = open.take() {
                    if let Some(proposal) =
                        r1_candidate(&name, suite_event, failed_at, event.event_id, &changed)
                    {
                        out.push(proposal);
                    }
                }
                changed.clear();
            }
            _ => {
                if open.is_some() {
                    if let Some(path) = changed_file(event) {
                        if let Some(name) = file_token(path) {
                            changed.push((name, event.event_id));
                        }
                    }
                }
            }
        }
    }
    out
}

fn r1_candidate(
    suite: &str,
    suite_event: Uuid,
    failed_at: Uuid,
    passed_at: Uuid,
    changed: &[(String, Uuid)],
) -> Option<CandidateProposal> {
    if changed.is_empty() {
        return None;
    }
    let names: Vec<String> = changed.iter().map(|(name, _)| name.clone()).collect();
    let primary = dominant(&names)?;
    let mut sources = vec![suite_event, failed_at];
    sources.extend(changed.iter().map(|(_, id)| *id));
    sources.push(passed_at);
    sources.dedup();
    sources.truncate(PROPOSAL_MAX_SOURCE_EVENTS);
    Some(CandidateProposal {
        kind: MemoryType::Failure,
        content: format!(
            "Tests were failing and passed after changes to {}.",
            name_list(&distinct(&names))
        ),
        topic_key: format!("test.{suite}"),
        value_key: format!("fixed_by.{primary}"),
        source_event_ids: sources,
        proposed_domain: KnowledgeDomain::Project,
    })
}

/// **R2 — Persistent failure.** `tool_failed(K) ≥3 times, same failure_kind, no
/// subsequent success`.
///
/// "No subsequent success" is read against the *tools that failed*: a different
/// tool succeeding afterwards says nothing about whether this failure was
/// resolved, and letting it clear the rule would silence the most common shape
/// — one broken tool inside an otherwise working session.
fn rule_2_persistent_failure(events: &[SafeCanonicalEvent]) -> Vec<CandidateProposal> {
    struct Streak {
        events: Vec<Uuid>,
        tools: BTreeSet<String>,
        last: usize,
    }
    let mut streaks: BTreeMap<FailureKind, Streak> = BTreeMap::new();
    let mut succeeded: BTreeMap<String, usize> = BTreeMap::new();

    for (position, event) in events.iter().enumerate() {
        match (&event.kind, event.content.as_ref()) {
            (
                EventKind::ToolFailed,
                Some(EventContent::ToolFailure {
                    vendor_tool,
                    failure_kind,
                    ..
                }),
            ) => {
                let streak = streaks.entry(*failure_kind).or_insert_with(|| Streak {
                    events: Vec::new(),
                    tools: BTreeSet::new(),
                    last: position,
                });
                streak.events.push(event.event_id);
                streak.tools.insert(vendor_tool.clone());
                streak.last = position;
            }
            (EventKind::ToolSucceeded, Some(EventContent::Tool { vendor_tool, .. })) => {
                succeeded.insert(vendor_tool.clone(), position);
            }
            _ => {}
        }
    }

    streaks
        .into_iter()
        .filter(|(_, streak)| streak.events.len() >= 3)
        .filter(|(_, streak)| {
            !streak
                .tools
                .iter()
                .any(|tool| succeeded.get(tool).is_some_and(|at| *at > streak.last))
        })
        .map(|(kind, mut streak)| {
            streak.events.truncate(PROPOSAL_MAX_SOURCE_EVENTS);
            CandidateProposal {
                kind: MemoryType::Failure,
                content: format!("`{}` fails repeatedly in this project.", kind.as_str()),
                topic_key: format!("failure.{}", kind.as_str()),
                value_key: "unresolved".to_string(),
                source_event_ids: streak.events,
                proposed_domain: KnowledgeDomain::Project,
            }
        })
        .collect()
}

/// **R4 — Decision near change.** `decision_signal … file_changed(F)+ within the
/// same session`.
///
/// Deliberately weak. `decision_signal` carries no prose — carrying prompt text
/// would be a transcript — so the claim asserts only what the events establish.
/// R4 exists to make the decision *locatable*, not to state its content.
///
/// One candidate per session, for the area most of the work landed in. The
/// contract phrases the claim in the singular, and emitting one per touched
/// directory would turn a weak signal into a wide one.
fn rule_4_decision_near_change(
    session: SessionRef,
    events: &[SafeCanonicalEvent],
) -> Vec<CandidateProposal> {
    let Some(decision_at) = events
        .iter()
        .position(|e| e.kind == EventKind::DecisionSignal)
    else {
        return Vec::new();
    };
    let decision_event = events[decision_at].event_id;

    let mut modules: Vec<String> = Vec::new();
    let mut per_module: BTreeMap<String, (Vec<String>, Vec<Uuid>)> = BTreeMap::new();
    for event in &events[decision_at + 1..] {
        let Some(path) = changed_file(event) else {
            continue;
        };
        let (Some(area), Some(file)) = (module_token(path), file_token(path)) else {
            continue;
        };
        modules.push(area.clone());
        let entry = per_module.entry(area).or_default();
        entry.0.push(file);
        entry.1.push(event.event_id);
    }

    let Some(area) = dominant(&modules) else {
        return Vec::new();
    };
    let Some((files, mut sources)) = per_module.remove(&area) else {
        return Vec::new();
    };
    sources.insert(0, decision_event);
    sources.dedup();
    sources.truncate(PROPOSAL_MAX_SOURCE_EVENTS);

    vec![CandidateProposal {
        kind: MemoryType::Decision,
        content: format!(
            "Work in {} followed a decision point in session `{}`.",
            name_list(&distinct(&files)),
            session.0
        ),
        topic_key: format!("area.{area}"),
        value_key: format!("changed.{}", session.0.simple()),
        source_event_ids: sources,
        proposed_domain: KnowledgeDomain::Project,
    }]
}

/// **R7 — Recorded decision.** `decision_signal{kind, subject, object}`.
///
/// The `decision.` prefix on the topic key is load-bearing. R1–R6 derive their
/// keys from structural evidence; R7 and R8 take theirs from a token the client
/// supplied, so an unprefixed key would let one crafted signal name an existing
/// high-value topic and register a `conflicts_with` against it — a poisoning
/// primitive, and one gate 5a cannot catch, because the cited event *is* the
/// key. Namespacing confines client-originated claims to their own key space,
/// where they still reinforce, conflict and supersede among themselves but
/// cannot collide with structurally-derived knowledge.
fn rule_7_recorded_decision(events: &[SafeCanonicalEvent]) -> Vec<CandidateProposal> {
    events
        .iter()
        .filter(|e| e.kind == EventKind::DecisionSignal)
        .filter_map(|event| {
            let Some(EventContent::Decision {
                decision_kind,
                subject_token,
                object_token,
                ..
            }) = event.content.as_ref()
            else {
                return None;
            };
            Some(CandidateProposal {
                kind: MemoryType::Decision,
                content: format!(
                    "This project `{}` `{}` for `{}`.",
                    decision_kind.as_str(),
                    object_token.as_str(),
                    subject_token.as_str()
                ),
                topic_key: format!("decision.{}", subject_token.as_str()),
                value_key: object_token.as_str().to_string(),
                source_event_ids: vec![event.event_id],
                proposed_domain: KnowledgeDomain::Project,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The project aggregator (`contracts/extraction.md` §4.0)
// ---------------------------------------------------------------------------

/// One session's events, as the aggregator reads them.
#[derive(Debug, Clone)]
pub struct SessionEvents {
    pub session_ref: SessionRef,
    pub events: Vec<SafeCanonicalEvent>,
}

/// R3, R5, R6 and R8 over one project's recent sessions.
///
/// **Not part of [`SemanticExtractor`]**, and that is the point: an extractor
/// must never be handed a cross-session corpus (FR-805a1, SC-749). This is
/// Cairn's own deterministic function over data `consolidate.rs` read for one
/// project, under the same scoping every other read path uses.
pub fn aggregate(project: ProjectRef, sessions: &[SessionEvents]) -> Vec<CandidateProposal> {
    let _ = project;
    let mut out = Vec::new();
    out.extend(rule_3_established_command(sessions));
    out.extend(rule_5_repeated_procedure(sessions));
    out.extend(rule_6_test_suite_identity(sessions));
    out.extend(rule_8_standing_instruction(sessions));
    out.retain(|p| p.check_shape().is_ok());
    out
}

/// A session's successful commands, in order.
fn successful_commands(session: &SessionEvents) -> Vec<(&str, Uuid)> {
    session
        .events
        .iter()
        .filter_map(|event| match (&event.kind, event.content.as_ref()) {
            (
                EventKind::CommandExecuted,
                Some(EventContent::Command {
                    command_line,
                    exit_status: Some(0),
                }),
            ) => Some((command_line.trim(), event.event_id)),
            _ => None,
        })
        .collect()
}

/// **R3 — Established command.** `command_executed(C) ≥3 sessions, exit_status = 0`.
fn rule_3_established_command(sessions: &[SessionEvents]) -> Vec<CandidateProposal> {
    let mut seen: BTreeMap<&str, (BTreeSet<Uuid>, Vec<Uuid>)> = BTreeMap::new();
    for session in sessions {
        for (line, event_id) in successful_commands(session) {
            let entry = seen.entry(line).or_default();
            entry.0.insert(session.session_ref.0);
            entry.1.push(event_id);
        }
    }
    seen.into_iter()
        .filter(|(_, (in_sessions, _))| in_sessions.len() >= 3)
        .filter_map(|(line, (_, mut events))| {
            let verb = command_verb(line)?;
            events.truncate(PROPOSAL_MAX_SOURCE_EVENTS);
            Some(CandidateProposal {
                kind: MemoryType::Convention,
                content: format!("`{line}` is the established command for this project."),
                topic_key: format!("command.{verb}"),
                value_key: digest_token(&[line]),
                source_event_ids: events,
                proposed_domain: KnowledgeDomain::Project,
            })
        })
        .filter(|p| p.content.len() <= CANDIDATE_CONTENT_MAX_BYTES)
        .collect()
}

/// **R5 — Repeated procedure.** Identical ordered `command_executed` sequence in
/// ≥2 sessions, all `exit_status = 0`.
fn rule_5_repeated_procedure(sessions: &[SessionEvents]) -> Vec<CandidateProposal> {
    let mut seen: BTreeMap<Vec<String>, (BTreeSet<Uuid>, Vec<Uuid>)> = BTreeMap::new();
    for session in sessions {
        let run = successful_commands(session);
        if run.len() < 2 || run.len() > PROCEDURE_MAX_STEPS {
            continue;
        }
        let sequence: Vec<String> = run.iter().map(|(line, _)| (*line).to_string()).collect();
        let entry = seen.entry(sequence).or_default();
        entry.0.insert(session.session_ref.0);
        entry.1.extend(run.iter().map(|(_, id)| *id));
    }
    seen.into_iter()
        .filter(|(_, (in_sessions, _))| in_sessions.len() >= 2)
        .filter_map(|(sequence, (_, mut events))| {
            let verb = command_verb(sequence.first()?)?;
            events.truncate(PROPOSAL_MAX_SOURCE_EVENTS);
            let rendered = sequence
                .iter()
                .map(|c| format!("`{c}`"))
                .collect::<Vec<_>>()
                .join(" → ");
            let parts: Vec<&str> = sequence.iter().map(String::as_str).collect();
            Some(CandidateProposal {
                kind: MemoryType::Procedure,
                content: format!("The sequence {rendered} is used to accomplish work here."),
                topic_key: format!("procedure.{verb}"),
                value_key: digest_token(&parts),
                source_event_ids: events,
                proposed_domain: KnowledgeDomain::Project,
            })
        })
        // A sequence whose rendering will not fit the content bound is dropped
        // rather than truncated: half a procedure is a wrong procedure.
        .filter(|p| p.content.len() <= CANDIDATE_CONTENT_MAX_BYTES)
        .collect()
}

/// **R6 — Test suite identity.** `test_executed(T) ≥2 with a consistent
/// test_command`.
///
/// *Consistent* is read strictly: every invocation the aggregator can see names
/// the same command. A project whose invocations disagree has no single test
/// command, and emitting the most popular one would manufacture a fact — and,
/// under gate 7, a permanent `conflicts_with` against the runner-up on every
/// pass.
fn rule_6_test_suite_identity(sessions: &[SessionEvents]) -> Vec<CandidateProposal> {
    let invocations: Vec<(&str, Uuid)> = sessions
        .iter()
        .flat_map(|s| s.events.iter())
        .filter_map(|event| match (&event.kind, event.content.as_ref()) {
            (EventKind::TestExecuted, Some(EventContent::TestInvocation { test_command })) => {
                Some((test_command.trim(), event.event_id))
            }
            _ => None,
        })
        .collect();

    if invocations.len() < 2 {
        return Vec::new();
    }
    let command = invocations[0].0;
    if invocations.iter().any(|(other, _)| *other != command) {
        return Vec::new();
    }
    let mut events: Vec<Uuid> = invocations.iter().map(|(_, id)| *id).collect();
    events.truncate(PROPOSAL_MAX_SOURCE_EVENTS);

    vec![CandidateProposal {
        kind: MemoryType::Fact,
        content: format!("`{command}` is the test command for this project."),
        topic_key: "test.command".to_string(),
        value_key: digest_token(&[command]),
        source_event_ids: events,
        proposed_domain: KnowledgeDomain::Project,
    }]
    .into_iter()
    .filter(|p| p.content.len() <= CANDIDATE_CONTENT_MAX_BYTES)
    .collect()
}

/// **R8 — Standing instruction.** `user_instruction_signal{require|forbid,
/// subject, object}` observed in ≥2 sessions.
///
/// An aggregator rule because a standing convention should rest on repetition,
/// not on one instruction in one session. `instruction.` namespaces it for the
/// same reason R7's `decision.` does.
fn rule_8_standing_instruction(sessions: &[SessionEvents]) -> Vec<CandidateProposal> {
    // The standing instruction, the sessions that carried it, and the events
    // that evidenced it. Named because the shape is what R8 counts over.
    type Standing = BTreeMap<(InstructionKind, String, String), (BTreeSet<Uuid>, Vec<Uuid>)>;
    let mut seen: Standing = BTreeMap::new();
    for session in sessions {
        for event in &session.events {
            let (
                EventKind::UserInstructionSignal,
                Some(EventContent::Instruction {
                    instruction_kind,
                    subject_token,
                    object_token,
                    ..
                }),
            ) = (&event.kind, event.content.as_ref())
            else {
                continue;
            };
            if !matches!(
                instruction_kind,
                InstructionKind::Require | InstructionKind::Forbid
            ) {
                continue;
            }
            let entry = seen
                .entry((
                    *instruction_kind,
                    subject_token.as_str().to_string(),
                    object_token.as_str().to_string(),
                ))
                .or_default();
            entry.0.insert(session.session_ref.0);
            entry.1.push(event.event_id);
        }
    }
    seen.into_iter()
        .filter(|(_, (in_sessions, _))| in_sessions.len() >= 2)
        .map(|((kind, subject, object), (_, mut events))| {
            events.truncate(PROPOSAL_MAX_SOURCE_EVENTS);
            let verb = match kind {
                InstructionKind::Forbid => "forbidden",
                _ => "required",
            };
            CandidateProposal {
                kind: MemoryType::Convention,
                content: format!("`{object}` is {verb} for `{subject}` here."),
                topic_key: format!("instruction.{subject}"),
                value_key: object,
                source_event_ids: events,
                proposed_domain: KnowledgeDomain::Project,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Gate 5a — key ↔ evidence correspondence
// ---------------------------------------------------------------------------

/// Every key pair Cairn's own session rules derive from exactly these events.
///
/// This is the re-derivation gate 5a runs (`contracts/consolidation.md` §5).
/// Without it the extractor chooses which existing record gets reinforced: a
/// well-formed proposal whose keys happen to match a high-value record would
/// produce a durable reinforcement a null extractor would not, which is
/// precisely the difference SC-742 measures.
///
/// The pairs are **as proposed**, not normalized. The caller normalizes both
/// sides with the same strict function before comparing, so a candidate cannot
/// pass or fail on a difference normalization would have erased.
pub fn rederive_session_keys(
    session: SessionRef,
    events: &[SafeCanonicalEvent],
) -> BTreeSet<(String, String)> {
    session_rules(session, events)
        .into_iter()
        .map(|p| (p.topic_key, p.value_key))
        .collect()
}

/// Every key pair Cairn's own project rules derive from exactly these events.
pub fn rederive_project_keys(
    project: ProjectRef,
    sessions: &[SessionEvents],
) -> BTreeSet<(String, String)> {
    aggregate(project, sessions)
        .into_iter()
        .map(|p| (p.topic_key, p.value_key))
        .collect()
}

// ---------------------------------------------------------------------------
// The hosted-extractor compliance gate (`contracts/extraction.md` §5)
// ---------------------------------------------------------------------------

/// What a deployment must have established before a hosted extractor may run.
///
/// Every field is an assertion the operator makes about **the actual provider,
/// model and endpoint in this deployment**, established from that provider's
/// current official documentation. There is no `Default` implementation and
/// there is deliberately no way to build a compliant configuration by omission:
/// FR-805e says a default configuration must not be assumed acceptable, and a
/// `Default` that produced one would be exactly that assumption in code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedExtractorConfig {
    /// §5.1 — provider, model and endpoint identity. All three, non-empty.
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    /// §5.2 — customer-content retention has been established: whether, where,
    /// how long.
    pub retention_established: bool,
    /// §5.3 — submitted content is not used for training or model improvement.
    pub training_excluded: bool,
    /// §5.4 — a zero-retention or no-training mode is available **and enabled**.
    pub zero_retention_enabled: bool,
    /// §5.5 — prompt and application-state caching behaviour has been
    /// established.
    pub caching_established: bool,
    /// §5.6 — the provider isolates projects and accounts.
    pub isolation_established: bool,
    /// §5.7 — the disclosure the provider requires has been made to end users,
    /// as plainly as the Cairn server connection itself (FR-805d).
    pub disclosed_to_users: bool,
    /// §5.8 — the behaviour when a compliant mode is unavailable or silently
    /// disabled has been established, and it is to stop rather than to proceed.
    pub fails_closed_when_noncompliant: bool,
}

/// Why a hosted extractor was not admitted.
///
/// One variant per row of `contracts/extraction.md` §5, so a blocker report
/// names the check that failed rather than saying "not compliant".
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ComplianceBlocker {
    #[error("no hosted extractor is configured")]
    NotConfigured,
    #[error("provider, model and endpoint identity is not established")]
    IdentityUnestablished,
    #[error("customer-content retention behaviour is not established")]
    RetentionUnestablished,
    #[error("training or model-improvement use of submitted content is not excluded")]
    TrainingNotExcluded,
    #[error("no zero-retention or no-training mode is enabled")]
    ZeroRetentionNotEnabled,
    #[error("prompt and application-state caching behaviour is not established")]
    CachingUnestablished,
    #[error("project and account isolation is not established")]
    IsolationUnestablished,
    #[error("the required disclosure has not been made to affected users")]
    NotDisclosed,
    #[error("behaviour when a compliant mode is unavailable is not fail-closed")]
    NotFailClosed,
}

impl ComplianceBlocker {
    /// A fixed term for a report, never a message assembled from configuration.
    pub fn reason(&self) -> &'static str {
        match self {
            ComplianceBlocker::NotConfigured => "hosted_extractor_not_configured",
            ComplianceBlocker::IdentityUnestablished => "provider_identity_unestablished",
            ComplianceBlocker::RetentionUnestablished => "retention_unestablished",
            ComplianceBlocker::TrainingNotExcluded => "training_not_excluded",
            ComplianceBlocker::ZeroRetentionNotEnabled => "zero_retention_not_enabled",
            ComplianceBlocker::CachingUnestablished => "caching_unestablished",
            ComplianceBlocker::IsolationUnestablished => "isolation_unestablished",
            ComplianceBlocker::NotDisclosed => "not_disclosed_to_users",
            ComplianceBlocker::NotFailClosed => "not_fail_closed",
        }
    }
}

/// The eight §5 checks, fail-closed.
///
/// The reasoning "the material had already left the machine, so no new egress
/// occurs" is **not available** here and has no field to be expressed in.
/// Constitution v1.2.1 Principle V names it as the derivation-as-loophole
/// argument it refuses: being permitted to reach the user's own server does not
/// permit forwarding anywhere else.
pub fn admit_hosted(config: &HostedExtractorConfig) -> Result<(), ComplianceBlocker> {
    if config.provider.trim().is_empty()
        || config.model.trim().is_empty()
        || config.endpoint.trim().is_empty()
    {
        return Err(ComplianceBlocker::IdentityUnestablished);
    }
    if !config.retention_established {
        return Err(ComplianceBlocker::RetentionUnestablished);
    }
    if !config.training_excluded {
        return Err(ComplianceBlocker::TrainingNotExcluded);
    }
    if !config.zero_retention_enabled {
        return Err(ComplianceBlocker::ZeroRetentionNotEnabled);
    }
    if !config.caching_established {
        return Err(ComplianceBlocker::CachingUnestablished);
    }
    if !config.isolation_established {
        return Err(ComplianceBlocker::IsolationUnestablished);
    }
    if !config.disclosed_to_users {
        return Err(ComplianceBlocker::NotDisclosed);
    }
    if !config.fails_closed_when_noncompliant {
        return Err(ComplianceBlocker::NotFailClosed);
    }
    Ok(())
}

/// Which extractor a deployment runs, and what stopped a hosted one.
///
/// **No provider is preselected**, and there is no path through this function
/// that enables a hosted extractor by default: `None` is the only configuration
/// a deployment has unless an operator supplied one, and it returns the
/// baseline with [`ComplianceBlocker::NotConfigured`]. Where compliance cannot
/// be established the deterministic baseline runs and the blocker is reported;
/// the privacy contract is not traded for extraction quality (FR-805e).
///
/// The compliant branch still returns the baseline, because Feature 005 ships
/// no hosted client. That is the honest state of the code: the gate exists, it
/// is fail-closed, and nothing behind it is implemented yet.
pub fn select_extractor(
    hosted: Option<&HostedExtractorConfig>,
) -> (Box<dyn SemanticExtractor>, Option<ComplianceBlocker>) {
    let blocker = match hosted {
        None => Some(ComplianceBlocker::NotConfigured),
        Some(config) => admit_hosted(config).err(),
    };
    (Box::new(DeterministicV1), blocker)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::event::{ChangeKind, DecisionKind};
    use cairn_core::event::{EventAgent, FileIdentity, ToolClass, VocabToken, CONTRACT_VERSION};

    fn event(seq: u64, kind: EventKind, content: Option<EventContent>) -> SafeCanonicalEvent {
        let session = Uuid::from_u128(7);
        SafeCanonicalEvent {
            event_id: cairn_core::eventid::event_id(session, seq),
            contract_version: CONTRACT_VERSION,
            kind,
            agent: EventAgent::ClaudeCode,
            vendor_event: Some("PostToolUse".to_string()),
            session_id: session,
            session_seq: seq,
            occurred_at: chrono::Utc::now(),
            content,
        }
    }

    fn changed(seq: u64, path: &str) -> SafeCanonicalEvent {
        event(
            seq,
            EventKind::FileChanged,
            Some(EventContent::File {
                repo_file: Some(path.to_string()),
                repo_file_from: None,
                change_kind: Some(ChangeKind::Modified),
                file_identity: FileIdentity::Present,
            }),
        )
    }

    fn verdict(seq: u64, outcome: TestOutcome) -> SafeCanonicalEvent {
        event(
            seq,
            EventKind::TestResult,
            Some(EventContent::TestVerdict {
                test_outcome: outcome,
                exit_status: None,
                tests_total: None,
                tests_failed: None,
            }),
        )
    }

    fn invoked(seq: u64, command: &str) -> SafeCanonicalEvent {
        event(
            seq,
            EventKind::TestExecuted,
            Some(EventContent::TestInvocation {
                test_command: command.to_string(),
            }),
        )
    }

    fn ran(seq: u64, line: &str, status: i32) -> SafeCanonicalEvent {
        event(
            seq,
            EventKind::CommandExecuted,
            Some(EventContent::Command {
                command_line: line.to_string(),
                exit_status: Some(status),
            }),
        )
    }

    fn failed(seq: u64, tool: &str, kind: FailureKind) -> SafeCanonicalEvent {
        event(
            seq,
            EventKind::ToolFailed,
            Some(EventContent::ToolFailure {
                vendor_tool: tool.to_string(),
                tool_class: ToolClass::Execute,
                failure_kind: kind,
                failure_note: None,
                exit_status: None,
            }),
        )
    }

    fn session() -> SessionRef {
        SessionRef(Uuid::from_u128(7))
    }

    fn input(events: Vec<SafeCanonicalEvent>) -> ExtractionInput {
        ExtractionInput::new(
            ProjectRef(Uuid::from_u128(1)),
            AccountRef(Uuid::from_u128(2)),
            session(),
            events,
        )
        .expect("within bounds")
    }

    #[test]
    fn r1_reads_a_fix_the_tests_confirmed() {
        let events = vec![
            invoked(1, "cargo test -p cairn-core"),
            verdict(2, TestOutcome::Failed),
            changed(3, "crates/cairn-core/src/knowledge.rs"),
            changed(4, "crates/cairn-core/src/knowledge.rs"),
            changed(5, "crates/cairn-core/src/domain.rs"),
            verdict(6, TestOutcome::Passed),
        ];
        let out = DeterministicV1.extract(&input(events)).expect("extracted");
        let r1 = out
            .iter()
            .find(|p| p.topic_key.starts_with("test."))
            .expect("R1 fired");
        assert_eq!(r1.topic_key, "test.cairn_core");
        assert_eq!(r1.value_key, "fixed_by.knowledge");
        assert_eq!(r1.kind, MemoryType::Failure);
        // The invocation that named the suite is cited, so gate 5a can
        // re-derive the topic key from the evidence alone.
        assert_eq!(r1.source_event_ids.len(), 6);
    }

    #[test]
    fn r1_says_nothing_when_no_invocation_named_the_suite() {
        // A verdict with no invocation before it names no suite. Emitting
        // `test.<something>` anyway would give two unrelated suites one key.
        let events = vec![
            verdict(1, TestOutcome::Failed),
            changed(2, "src/a.rs"),
            verdict(3, TestOutcome::Passed),
        ];
        let out = DeterministicV1.extract(&input(events)).expect("extracted");
        assert!(out.iter().all(|p| !p.topic_key.starts_with("test.")));
    }

    #[test]
    fn r2_needs_three_failures_and_no_later_success_from_the_same_tool() {
        let three = vec![
            failed(1, "bash", FailureKind::PermissionDenied),
            failed(2, "bash", FailureKind::PermissionDenied),
            failed(3, "bash", FailureKind::PermissionDenied),
        ];
        let out = session_rules(session(), &three);
        let r2 = out
            .iter()
            .find(|p| p.topic_key == "failure.permission_denied")
            .expect("R2 fired");
        assert_eq!(r2.value_key, "unresolved");

        // Two is not a pattern.
        assert!(session_rules(session(), &three[..2])
            .iter()
            .all(|p| !p.topic_key.starts_with("failure.")));

        // The same tool succeeding afterwards resolves it.
        let mut resolved = three.clone();
        resolved.push(event(
            4,
            EventKind::ToolSucceeded,
            Some(EventContent::Tool {
                vendor_tool: "bash".to_string(),
                tool_class: ToolClass::Execute,
            }),
        ));
        assert!(session_rules(session(), &resolved)
            .iter()
            .all(|p| !p.topic_key.starts_with("failure.")));

        // A *different* tool succeeding says nothing about this failure.
        let mut unrelated = three;
        unrelated.push(event(
            4,
            EventKind::ToolSucceeded,
            Some(EventContent::Tool {
                vendor_tool: "grep".to_string(),
                tool_class: ToolClass::Execute,
            }),
        ));
        assert!(session_rules(session(), &unrelated)
            .iter()
            .any(|p| p.topic_key == "failure.permission_denied"));
    }

    #[test]
    fn r4_and_r7_read_one_decision_signal_two_ways() {
        let signal = event(
            1,
            EventKind::DecisionSignal,
            Some(EventContent::Decision {
                decision_kind: DecisionKind::Adopt,
                subject_token: VocabToken::subject("storage_authority").unwrap(),
                object_token: VocabToken::object("postgresql").unwrap(),
                justified_by_seq: Some(0),
                lexicon_version: 1,
            }),
        );
        let events = vec![
            signal,
            changed(2, "crates/server.rs"),
            changed(3, "crates/db.rs"),
        ];
        let out = session_rules(session(), &events);

        let r4 = out
            .iter()
            .find(|p| p.topic_key.starts_with("area."))
            .expect("R4 fired");
        assert_eq!(r4.topic_key, "area.crates");
        assert_eq!(
            r4.value_key,
            format!("changed.{}", session().0.simple()),
            "R4's value key locates the decision in its own session"
        );

        let r7 = out
            .iter()
            .find(|p| p.topic_key.starts_with("decision."))
            .expect("R7 fired");
        assert_eq!(r7.topic_key, "decision.storage_authority");
        assert_eq!(r7.value_key, "postgresql");
        assert_eq!(
            r7.content,
            "This project `adopt` `postgresql` for `storage_authority`."
        );
    }

    #[test]
    fn r7_cannot_collide_with_structurally_derived_knowledge() {
        // A crafted signal naming an existing high-value topic. The `decision.`
        // prefix confines it to its own key space, where it can conflict with
        // other decisions but not with `test.command`.
        let signal = event(
            1,
            EventKind::DecisionSignal,
            Some(EventContent::Decision {
                decision_kind: DecisionKind::Adopt,
                subject_token: VocabToken::subject("test.command").unwrap(),
                object_token: VocabToken::object("nothing").unwrap(),
                justified_by_seq: Some(0),
                lexicon_version: 1,
            }),
        );
        let out = session_rules(session(), &[signal]);
        let r7 = out.first().expect("R7 fired");
        assert_eq!(r7.topic_key, "decision.test.command");
        assert_ne!(r7.topic_key, "test.command");
    }

    fn sessions_running(runs: &[(u128, &[&str])]) -> Vec<SessionEvents> {
        runs.iter()
            .map(|(id, lines)| SessionEvents {
                session_ref: SessionRef(Uuid::from_u128(*id)),
                events: lines
                    .iter()
                    .enumerate()
                    .map(|(i, line)| ran(i as u64 + 1, line, 0))
                    .collect(),
            })
            .collect()
    }

    #[test]
    fn r3_needs_three_sessions_and_keys_on_a_digest() {
        let two = sessions_running(&[(1, &["cargo build"]), (2, &["cargo build"])]);
        assert!(aggregate(ProjectRef(Uuid::from_u128(9)), &two).is_empty());

        let three = sessions_running(&[
            (1, &["cargo build --release"]),
            (2, &["cargo build --release"]),
            (3, &["cargo build --release"]),
        ]);
        let out = aggregate(ProjectRef(Uuid::from_u128(9)), &three);
        let r3 = out
            .iter()
            .find(|p| p.topic_key == "command.cargo")
            .expect("R3 fired");
        // A digest, not the command: the command is up to 512 bytes and a value
        // key is 64 characters, so keying on the text would refuse the
        // candidate the rule just produced.
        assert_eq!(r3.value_key.len(), DIGEST_CHARS);
        assert!(r3.value_key.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(
            r3.content,
            "`cargo build --release` is the established command for this project."
        );
    }

    #[test]
    fn r5_needs_the_same_ordered_sequence_in_two_sessions() {
        let same = sessions_running(&[
            (1, &["cargo fmt", "cargo clippy"]),
            (2, &["cargo fmt", "cargo clippy"]),
        ]);
        let out = aggregate(ProjectRef(Uuid::from_u128(9)), &same);
        let r5 = out
            .iter()
            .find(|p| p.topic_key == "procedure.cargo")
            .expect("R5 fired");
        assert_eq!(r5.kind, MemoryType::Procedure);

        // Reordered is a different procedure, and neither now repeats.
        let reordered = sessions_running(&[
            (1, &["cargo fmt", "cargo clippy"]),
            (2, &["cargo clippy", "cargo fmt"]),
        ]);
        assert!(aggregate(ProjectRef(Uuid::from_u128(9)), &reordered)
            .iter()
            .all(|p| !p.topic_key.starts_with("procedure.")));
    }

    #[test]
    fn r6_stays_silent_when_the_invocations_disagree() {
        let consistent = vec![
            SessionEvents {
                session_ref: SessionRef(Uuid::from_u128(1)),
                events: vec![invoked(1, "cargo test")],
            },
            SessionEvents {
                session_ref: SessionRef(Uuid::from_u128(2)),
                events: vec![invoked(1, "cargo test")],
            },
        ];
        assert!(aggregate(ProjectRef(Uuid::from_u128(9)), &consistent)
            .iter()
            .any(|p| p.topic_key == "test.command"));

        let inconsistent = vec![
            SessionEvents {
                session_ref: SessionRef(Uuid::from_u128(1)),
                events: vec![invoked(1, "cargo test")],
            },
            SessionEvents {
                session_ref: SessionRef(Uuid::from_u128(2)),
                events: vec![invoked(1, "pytest")],
            },
        ];
        assert!(
            aggregate(ProjectRef(Uuid::from_u128(9)), &inconsistent)
                .iter()
                .all(|p| p.topic_key != "test.command"),
            "two disagreeing runners would have produced a permanent conflict"
        );
    }

    #[test]
    fn r8_needs_two_sessions_of_the_same_standing_instruction() {
        let instruction = |seq: u64| {
            event(
                seq,
                EventKind::UserInstructionSignal,
                Some(EventContent::Instruction {
                    instruction_kind: InstructionKind::Forbid,
                    subject_token: VocabToken::subject("migrations").unwrap(),
                    object_token: VocabToken::object("rewrite").unwrap(),
                    justified_by_seq: Some(0),
                    lexicon_version: 1,
                }),
            )
        };
        let one = vec![SessionEvents {
            session_ref: SessionRef(Uuid::from_u128(1)),
            events: vec![instruction(1)],
        }];
        assert!(aggregate(ProjectRef(Uuid::from_u128(9)), &one).is_empty());

        let two = vec![
            SessionEvents {
                session_ref: SessionRef(Uuid::from_u128(1)),
                events: vec![instruction(1)],
            },
            SessionEvents {
                session_ref: SessionRef(Uuid::from_u128(2)),
                events: vec![instruction(1)],
            },
        ];
        let out = aggregate(ProjectRef(Uuid::from_u128(9)), &two);
        let r8 = out.first().expect("R8 fired");
        assert_eq!(r8.topic_key, "instruction.migrations");
        assert_eq!(r8.value_key, "rewrite");
        assert_eq!(r8.content, "`rewrite` is forbidden for `migrations` here.");
    }

    #[test]
    fn extraction_never_consults_provenance() {
        // FR-723–725, FR-779: `agent` and `vendor_event` are provenance. They
        // are persisted, and nothing in extraction or ranking may read them.
        let build = |agent: EventAgent, vendor: &str| {
            vec![
                invoked(1, "cargo test -p cairn-core"),
                verdict(2, TestOutcome::Failed),
                SafeCanonicalEvent {
                    agent,
                    vendor_event: Some(vendor.to_string()),
                    ..changed(3, "crates/cairn-core/src/knowledge.rs")
                },
                verdict(4, TestOutcome::Passed),
            ]
        };
        let a = session_rules(session(), &build(EventAgent::ClaudeCode, "PostToolUse"));
        let b = session_rules(
            session(),
            &build(EventAgent::OpenCode, "tool.execute.after"),
        );
        assert_eq!(a, b, "extraction output varied with vendor provenance");
    }

    #[test]
    fn an_extraction_request_names_exactly_one_project_and_one_account() {
        // SC-749's unit-level half. The scoping is on the request itself and
        // there is no field through which a second project or a second account
        // could arrive, so a cross-project corpus is not something an extractor
        // has to be trusted to refuse — it is something it cannot be handed.
        let project = Uuid::now_v7();
        let account = Uuid::now_v7();
        let session = Uuid::now_v7();
        let input = ExtractionInput::new(
            ProjectRef(project),
            AccountRef(account),
            SessionRef(session),
            Vec::new(),
        )
        .expect("an empty request is within every bound");
        assert_eq!(input.project_ref.0, project);
        assert_eq!(input.account_ref.0, account);
        assert_eq!(input.session_ref.0, session);
    }

    #[test]
    fn the_input_refuses_more_than_it_may_carry() {
        let too_many: Vec<SafeCanonicalEvent> = (0..=EXTRACTION_MAX_EVENTS as u64)
            .map(|seq| event(seq, EventKind::AgentQuiesced, Some(EventContent::None)))
            .collect();
        assert_eq!(
            ExtractionInput::new(
                ProjectRef(Uuid::from_u128(1)),
                AccountRef(Uuid::from_u128(2)),
                session(),
                too_many
            )
            .unwrap_err(),
            ExtractError::BoundExceeded
        );
    }

    #[test]
    fn a_hosted_extractor_is_never_admitted_by_default() {
        // There is no `Default` for the configuration and no path that enables
        // a hosted extractor without one. Absence is a blocker, not a pass.
        let (extractor, blocker) = select_extractor(None);
        assert_eq!(extractor.kind(), "deterministic_v1");
        assert_eq!(blocker, Some(ComplianceBlocker::NotConfigured));
    }

    #[test]
    fn every_one_of_the_eight_checks_can_block_on_its_own() {
        let compliant = HostedExtractorConfig {
            provider: "acme".into(),
            model: "m-1".into(),
            endpoint: "https://example.invalid/v1".into(),
            retention_established: true,
            training_excluded: true,
            zero_retention_enabled: true,
            caching_established: true,
            isolation_established: true,
            disclosed_to_users: true,
            fails_closed_when_noncompliant: true,
        };
        assert_eq!(admit_hosted(&compliant), Ok(()));

        let cases: Vec<(HostedExtractorConfig, ComplianceBlocker)> = vec![
            (
                HostedExtractorConfig {
                    provider: "  ".into(),
                    ..compliant.clone()
                },
                ComplianceBlocker::IdentityUnestablished,
            ),
            (
                HostedExtractorConfig {
                    model: String::new(),
                    ..compliant.clone()
                },
                ComplianceBlocker::IdentityUnestablished,
            ),
            (
                HostedExtractorConfig {
                    endpoint: String::new(),
                    ..compliant.clone()
                },
                ComplianceBlocker::IdentityUnestablished,
            ),
            (
                HostedExtractorConfig {
                    retention_established: false,
                    ..compliant.clone()
                },
                ComplianceBlocker::RetentionUnestablished,
            ),
            (
                HostedExtractorConfig {
                    training_excluded: false,
                    ..compliant.clone()
                },
                ComplianceBlocker::TrainingNotExcluded,
            ),
            (
                HostedExtractorConfig {
                    zero_retention_enabled: false,
                    ..compliant.clone()
                },
                ComplianceBlocker::ZeroRetentionNotEnabled,
            ),
            (
                HostedExtractorConfig {
                    caching_established: false,
                    ..compliant.clone()
                },
                ComplianceBlocker::CachingUnestablished,
            ),
            (
                HostedExtractorConfig {
                    isolation_established: false,
                    ..compliant.clone()
                },
                ComplianceBlocker::IsolationUnestablished,
            ),
            (
                HostedExtractorConfig {
                    disclosed_to_users: false,
                    ..compliant.clone()
                },
                ComplianceBlocker::NotDisclosed,
            ),
            (
                HostedExtractorConfig {
                    fails_closed_when_noncompliant: false,
                    ..compliant.clone()
                },
                ComplianceBlocker::NotFailClosed,
            ),
        ];
        for (config, expected) in cases {
            assert_eq!(admit_hosted(&config), Err(expected));
            // And whatever the blocker, the baseline is what runs.
            let (extractor, blocker) = select_extractor(Some(&config));
            assert_eq!(extractor.kind(), "deterministic_v1");
            assert_eq!(blocker, Some(expected));
        }
    }

    #[test]
    fn gate_5a_re_derives_exactly_what_the_rules_produced() {
        let events = vec![
            invoked(1, "cargo test -p cairn-core"),
            verdict(2, TestOutcome::Failed),
            changed(3, "crates/cairn-core/src/knowledge.rs"),
            verdict(4, TestOutcome::Passed),
        ];
        let derived = rederive_session_keys(session(), &events);
        assert!(derived.contains(&(
            "test.cairn_core".to_string(),
            "fixed_by.knowledge".to_string()
        )));
        // A key pair nobody's evidence supports is not in the set, which is
        // what refuses a hostile proposal at gate 5a.
        assert!(!derived.contains(&("storage.authority".to_string(), "server".to_string())));
    }
}
