//! Canonical knowledge: subject identity, scope overlap and the derivation
//! (`contracts/knowledge.md`).
//!
//! Pure. No I/O, no clock, no randomness, no database. That is not an
//! aesthetic preference — it is what makes the whole reconciliation model
//! testable against a JSON corpus, and what makes it survive any merge from
//! any device: there is no canonical row, so after two stores exchange
//! proposals and decisions the answer is simply recomputed (D44).
//!
//! # What this module may not read
//!
//! [`derive_subject`] never reads `created_at`, `updated_at`, `effective_from`,
//! a relation's `decided_at`, or the timestamp embedded in a UUIDv7 **to
//! choose between competing proposals** (FR-303, D49). Identifier order sorts
//! output for stability and nothing else. The types here carry no timestamp at
//! all, so the rule is enforced by what the function can see rather than by
//! review.

use crate::domain::{
    Importance, MemoryScope, MemoryState, Reconciliation, RelationBasis, RelationKind,
    VerificationAuthority, VerificationState,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Normalization (FR-311, FR-312, FR-326)
// ---------------------------------------------------------------------------

/// Longest normalized topic key, in characters.
pub const TOPIC_KEY_MAX_CHARS: usize = 128;
/// Most dot-separated segments a topic key may carry.
pub const TOPIC_KEY_MAX_SEGMENTS: usize = 6;
/// Longest normalized value key, in characters.
pub const VALUE_KEY_MAX_CHARS: usize = 64;

/// Normalize a proposed topic key, or report that it cannot be represented.
///
/// Total: unrepresentable input yields `None` and never an error, because
/// FR-312 requires the memory to be stored regardless — free-form rather than
/// rejected. The caller reports `invalid_topic_key` as a note on an `ok: true`
/// envelope.
///
/// ```text
/// 1. Unicode NFC, then lower-case
/// 2. split on '.'
/// 3. each segment: keep [a-z0-9_]; map '-', ' ' and '/' to '_';
///    collapse repeats; trim '_'
/// 4. drop empty segments
/// 5. reject if: 0 segments, > 6 segments, or total length > 128
/// ```
///
/// `/` deliberately maps to `_` rather than separating: a topic key is not a
/// path, and accepting path syntax would invite absolute paths into a column
/// that synchronizes.
pub fn normalize_topic_key(input: &str) -> Option<String> {
    let lowered: String = input.nfc().collect::<String>().to_lowercase();

    let segments: Vec<String> = lowered
        .split('.')
        .map(normalize_segment)
        .filter(|s| !s.is_empty())
        .collect();

    if segments.is_empty() || segments.len() > TOPIC_KEY_MAX_SEGMENTS {
        return None;
    }
    let key = segments.join(".");
    if key.chars().count() > TOPIC_KEY_MAX_CHARS {
        return None;
    }
    Some(key)
}

fn normalize_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for ch in segment.chars() {
        let mapped = match ch {
            'a'..='z' | '0'..='9' | '_' => ch,
            '-' | ' ' | '/' => '_',
            _ => continue,
        };
        // Collapse runs of '_' as we go rather than in a second pass.
        if mapped == '_' && out.ends_with('_') {
            continue;
        }
        out.push(mapped);
    }
    out.trim_matches('_').to_string()
}

/// Normalize a proposed value key.
///
/// A value key states a **value**, not a whole proposition, and it is accepted
/// only alongside a topic key — the caller enforces that pairing, because it is
/// a storage constraint rather than a normalization one (FR-311).
pub fn normalize_value_key(input: &str) -> Option<String> {
    let collapsed: String = input
        .nfc()
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.is_empty() || collapsed.chars().count() > VALUE_KEY_MAX_CHARS {
        return None;
    }
    Some(collapsed)
}

/// The digest exact-duplicate detection compares (D46).
///
/// `SHA-256( NFC → lower-case → collapse whitespace → strip trailing .,;:!? )`
///
/// This is **not** a similarity measure and must never be compared for partial
/// equality. Two digests are equal or they are unrelated; there is no third
/// answer, which is precisely why automatic reconciliation can rest on it
/// without inference (FR-326, FR-511).
pub fn content_norm_digest(content: &str) -> String {
    crate::digest(&normalize_content(content))
}

/// The normalized form behind [`content_norm_digest`], exposed so a test can
/// show *why* two contents collided rather than only that they did.
pub fn normalize_content(content: &str) -> String {
    let collapsed: String = content
        .nfc()
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    collapsed
        .trim_end_matches(['.', ',', ';', ':', '!', '?'])
        .to_string()
}

// ---------------------------------------------------------------------------
// Scope overlap (FR-332, FR-333, FR-381, FR-385)
// ---------------------------------------------------------------------------

/// Whether a single working context would select both memories.
///
/// This is the precondition for conflict, and it is why the two cases people
/// expect to be conflicts are not: a project-scoped answer and a task-scoped
/// one are never simultaneously applicable, and neither are two branches
/// (D48).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeOverlap {
    /// Same scope, same scope key: both apply at once, so they may conflict.
    Simultaneous,
    /// Different precedence ranks on one topic: the narrower applies in its own
    /// context and the broader is the answer it narrows (FR-333).
    ScopeException,
    /// Same scope kind, different key — `branch:main` against
    /// `branch:feature/x`. Never simultaneously applicable, and they do not
    /// interact at all.
    Disjoint,
}

/// Classify two scopes for the purposes of conflict and narrowing.
///
/// Precedence is Feature 001's [`MemoryScope::bucket`], unchanged: task 0,
/// branch 1, project 2, session 3.
pub fn scope_overlap(
    a_scope: MemoryScope,
    a_key: &str,
    b_scope: MemoryScope,
    b_key: &str,
) -> ScopeOverlap {
    if a_scope == b_scope {
        return if a_key == b_key {
            ScopeOverlap::Simultaneous
        } else {
            ScopeOverlap::Disjoint
        };
    }
    if a_scope.bucket() == b_scope.bucket() {
        // Unreachable with Feature 001's four scopes, which have distinct
        // buckets. Stated rather than assumed, so adding a scope that shares a
        // rank has to decide this deliberately.
        return ScopeOverlap::Disjoint;
    }
    ScopeOverlap::ScopeException
}

/// Which of two scopes applies, where they form a scope exception.
///
/// Lower bucket wins, which is Feature 001's existing precedence.
pub fn narrower_scope(a: MemoryScope, b: MemoryScope) -> MemoryScope {
    if a.bucket() <= b.bucket() {
        a
    } else {
        b
    }
}

// ---------------------------------------------------------------------------
// Relations
// ---------------------------------------------------------------------------

/// A reconciliation decision, as the derivation sees it.
///
/// Deliberately carries no `decided_at`: the derivation never reads it, and a
/// type that cannot express a timestamp cannot accidentally arbitrate on one
/// (D49).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Relation {
    pub from: Uuid,
    pub to: Uuid,
    pub kind: RelationKind,
    pub basis: RelationBasis,
}

impl Relation {
    /// Build a relation with symmetric endpoints already normalized.
    pub fn new(kind: RelationKind, from: Uuid, to: Uuid, basis: RelationBasis) -> Self {
        let (from, to) = normalize_relation_endpoints(kind, from, to);
        Self {
            from,
            to,
            kind,
            basis,
        }
    }

    /// Whether this relation touches `id` at either end.
    pub fn touches(&self, id: Uuid) -> bool {
        self.from == id || self.to == id
    }
}

/// Order the endpoints of a symmetric relation deterministically.
///
/// `conflicts_with` is the one kind whose meaning has no direction, so its
/// endpoints become `(min, max)` lexicographically before the write. Two
/// machines detecting one conflict while offline would otherwise produce `A→B`
/// and `B→A` — two durable rows for one fact, both syncing, and the same
/// conflict reported twice. With normalization the primary key absorbs the
/// second machine's record exactly as it absorbs a local duplicate (FR-305,
/// D78, SC-324).
///
/// Every other kind is directional and is returned untouched. Normalizing
/// `supersedes` would destroy the relation's entire content.
pub fn normalize_relation_endpoints(kind: RelationKind, a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    if kind.is_symmetric() && b < a {
        (b, a)
    } else {
        (a, b)
    }
}

// ---------------------------------------------------------------------------
// Automatic reconciliation (D46, FR-316, FR-321, FR-327)
// ---------------------------------------------------------------------------

/// What a new proposal turned out to be, relative to the subject it joins.
///
/// Returned to the writer so the party that can read both statements can decide
/// what Cairn deliberately will not: whether a corroborating member is the same
/// claim (FR-327).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ProposalOutcome {
    /// Nothing in the subject matched.
    Created,
    /// Identical after normalization to an existing member. A `duplicates`
    /// relation is recorded and the proposal stays individually retrievable
    /// with its own provenance (FR-321).
    Duplicate { of: Uuid },
    /// Agrees on the value with an existing member and differs in content. **No
    /// relation is recorded** — the value is agreed and the statements are
    /// several. The matched member is named so the writer can collapse them
    /// with one explicit call if they really are one claim (FR-327, D77).
    Corroborating { member: Uuid },
    /// Incompatible with one or more simultaneously applicable members. The
    /// conflict is recorded; it is never resolved (FR-334).
    ConflictDetected { with: Vec<Uuid> },
    /// The subject has more members than the per-write bound allows. The write
    /// completes, the relation is deferred to the maintenance tick, and the
    /// response says so (FR-474).
    Deferred,
}

/// Decide what a new proposal is, and which relations follow automatically.
///
/// **Exactly one merging case** exists, and it is the only one Cairn can decide
/// without inference: content that is identical after normalization. Everything
/// else is surfaced and never merged (D46).
///
/// | Condition | Outcome | Relation |
/// |---|---|---|
/// | same subject, equal `content_norm_digest` | duplicate | `duplicates` new→existing |
/// | same subject, equal `value_key`, differing content | corroboration | **none** |
/// | same subject, differing `value_key`, same scope and scope key | conflict | `conflicts_with` |
/// | different scope precedence rank | scope exception | none |
/// | same scope, different scope key | unrelated | none |
/// | either side has no `topic_key` | nothing | none |
///
/// The last row is why a free-form proposal never merges. FR-321 scopes
/// duplication to "an existing member of **the same subject**", and a subject
/// requires a topic key (FR-315); FR-313 requires a free-form memory to behave
/// exactly as it does in Feature 001, where two identical memories are two
/// memories. `plan.md`'s risk table describes exact-content duplication as
/// working "without any key at all", which overstates what the requirements
/// permit — and the conservative reading is the one that cannot suppress a
/// claim.
///
/// `members` are the existing members of the subject the proposal joins.
/// Bounded by `reconcile_members_max`: beyond it the write completes and the
/// decision defers rather than scanning (FR-474).
pub fn classify_proposal(
    proposal: &MemoryFacts,
    members: &[MemoryFacts],
    reconcile_members_max: usize,
) -> (ProposalOutcome, Vec<Relation>) {
    // Without a subject there is nothing to reconcile against. The proposal is
    // stored, searchable and briefable exactly as in Feature 001.
    let Some(topic) = proposal.topic_key.as_deref() else {
        return (ProposalOutcome::Created, Vec::new());
    };

    let applicable: Vec<&MemoryFacts> = members
        .iter()
        .filter(|m| m.id != proposal.id)
        .filter(|m| m.state == MemoryState::Active)
        .filter(|m| m.topic_key.as_deref() == Some(topic))
        .filter(|m| {
            scope_overlap(proposal.scope, &proposal.scope_key, m.scope, &m.scope_key)
                == ScopeOverlap::Simultaneous
        })
        .collect();

    if applicable.len() > reconcile_members_max {
        return (ProposalOutcome::Deferred, Vec::new());
    }

    // 1. Identical content. The one case decidable without inference.
    if let Some(digest) = proposal.content_norm_digest.as_deref() {
        let mut identical: Vec<&MemoryFacts> = applicable
            .iter()
            .filter(|m| m.content_norm_digest.as_deref() == Some(digest))
            .copied()
            .collect();
        if !identical.is_empty() {
            // The existing memory a duplicate points at is the
            // highest-precedence active member; where several are equally
            // applicable, the lowest identifier, for stability.
            identical.sort_by_key(|m| (m.scope.bucket(), m.id));
            let target = identical[0];
            return (
                ProposalOutcome::Duplicate { of: target.id },
                vec![Relation::new(
                    RelationKind::Duplicates,
                    proposal.id,
                    target.id,
                    RelationBasis::DeterministicRule,
                )],
            );
        }
    }

    // 2. A shared value key with differing content. Agreement about the value,
    //    and nothing more — so nothing is written.
    if let Some(value) = proposal.value_key.as_deref() {
        let mut agreeing: Vec<&MemoryFacts> = applicable
            .iter()
            .filter(|m| m.value_key.as_deref() == Some(value))
            .copied()
            .collect();
        if !agreeing.is_empty() {
            agreeing.sort_by_key(|m| m.id);
            return (
                ProposalOutcome::Corroborating {
                    member: agreeing[0].id,
                },
                Vec::new(),
            );
        }

        // 3. A different value key in an overlapping scope: they disagree.
        let mut disagreeing: Vec<&MemoryFacts> = applicable
            .iter()
            .filter(|m| m.value_key.is_some() && m.value_key.as_deref() != Some(value))
            .copied()
            .collect();
        if !disagreeing.is_empty() {
            disagreeing.sort_by_key(|m| m.id);
            let relations = disagreeing
                .iter()
                .map(|m| {
                    Relation::new(
                        RelationKind::ConflictsWith,
                        proposal.id,
                        m.id,
                        RelationBasis::DeterministicRule,
                    )
                })
                .collect();
            return (
                ProposalOutcome::ConflictDetected {
                    with: disagreeing.iter().map(|m| m.id).collect(),
                },
                relations,
            );
        }
    }

    (ProposalOutcome::Created, Vec::new())
}

// ---------------------------------------------------------------------------
// The derivation (FR-302, FR-303, FR-327, FR-334)
// ---------------------------------------------------------------------------

/// Everything the derivation reads about one proposal.
///
/// Note what is absent: every timestamp. The derivation cannot read a clock
/// because the type it reads does not carry one (FR-303).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryFacts {
    pub id: Uuid,
    pub state: MemoryState,
    pub scope: MemoryScope,
    pub scope_key: String,
    pub topic_key: Option<String>,
    pub value_key: Option<String>,
    pub content_norm_digest: Option<String>,
    pub verification: VerificationState,
    pub verification_authority: Option<VerificationAuthority>,
    pub evidence_fact_count: usize,
    pub pinned: bool,
    pub importance: Importance,
    /// Attribution. Read only for distinct-origin accounting, never for
    /// arbitration (FR-322).
    pub origin_session_id: Uuid,
}

impl MemoryFacts {
    /// A minimal active free-form proposal, for tests and fixtures.
    pub fn active(id: Uuid, scope: MemoryScope, scope_key: impl Into<String>) -> Self {
        Self {
            id,
            state: MemoryState::Active,
            scope,
            scope_key: scope_key.into(),
            topic_key: None,
            value_key: None,
            content_norm_digest: None,
            verification: VerificationState::Unverified,
            verification_authority: None,
            evidence_fact_count: 0,
            pinned: false,
            importance: Importance::Normal,
            origin_session_id: Uuid::nil(),
        }
    }

    /// Where this proposal sorts when a partition needs a representative.
    ///
    /// A deterministic check outranks an attestation, which is what stops an
    /// attested claim becoming the face of a subject over a checked one
    /// (`contracts/knowledge.md` §derive_subject).
    fn verification_rank(&self) -> u8 {
        match (self.verification, self.verification_authority) {
            (VerificationState::Verified, Some(a)) if a.is_local_deterministic() => 0,
            (VerificationState::Verified, Some(VerificationAuthority::RemoteCairn)) => 1,
            (VerificationState::Verified, _) => 2,
            (VerificationState::NeedsRecheck, _) => 3,
            (VerificationState::Unverified, _) => 4,
            (VerificationState::Drifted, _) => 5,
            (VerificationState::Conflicted, _) => 6,
        }
    }

    /// Ordering key for a representative: most evidence, then strongest
    /// verification, then lowest identifier.
    ///
    /// Every tiebreak is a property of the record. None is a property of time.
    fn representative_key(&self) -> (std::cmp::Reverse<usize>, u8, Uuid) {
        (
            std::cmp::Reverse(self.evidence_fact_count),
            self.verification_rank(),
            self.id,
        )
    }
}

/// What one canonical answer stands for.
///
/// Kept separate from [`SubjectView::answers`] because a duplicate is dropped
/// from the answer set but stays individually retrievable and still counts
/// toward distinct-origin accounting (FR-321, FR-322). Collapsing the two would
/// lose exactly the accounting FR-406 forbids misreporting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerAccounting {
    /// The proposal that is the answer.
    pub memory_id: Uuid,
    /// Proposals dropped as duplicates of it. Each remains retrievable with its
    /// own provenance.
    pub duplicates: Vec<Uuid>,
    /// Distinct origin sessions across the answer and its duplicates.
    ///
    /// **Never** presented as a number of independent verifications (FR-406).
    pub distinct_origins: usize,
}

/// The derived state of one subject. Nothing stores this (D44).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectView {
    pub reconciliation: Reconciliation,
    /// One when settled or reinforced, two or more when conflicted or
    /// corroborated, none when historical. Sorted for stable output.
    pub answers: Vec<Uuid>,
    /// Duplication and origin accounting, aligned to `answers`.
    pub accounting: Vec<AnswerAccounting>,
    /// Proposals at a narrower scope that a recorded `narrows` decision says
    /// are exceptions to this subject's answer (FR-333).
    pub narrowed_by: Vec<Uuid>,
    /// The decisions that produced this outcome, so `cairn memory subject` can
    /// answer "why" (FR-307).
    pub decisions: Vec<Relation>,
}

impl SubjectView {
    /// A subject with no canonical answer.
    fn historical(decisions: Vec<Relation>, narrowed_by: Vec<Uuid>) -> Self {
        Self {
            reconciliation: Reconciliation::Historical,
            answers: Vec::new(),
            accounting: Vec::new(),
            narrowed_by,
            decisions,
        }
    }

    /// Whether the subject has exactly one current answer.
    pub fn is_settled(&self) -> bool {
        matches!(
            self.reconciliation,
            Reconciliation::Settled | Reconciliation::Reinforced
        )
    }
}

/// Derive a subject's canonical answer from its proposals and the recorded
/// decisions.
///
/// `members` are the proposals of one subject — one `(project, scope,
/// scope_key, topic_key)` — including superseded and stale ones, so the
/// historical case is decidable. `relations` are every decision touching them.
///
/// ```text
/// 1. keep only lifecycle-active members
/// 2. drop members a `supersedes` or `duplicates` relation points at
/// 3. if none remain → Historical
///    unless every member was dropped by a supersession *cycle*, which
///    resolves nothing → Conflicted
/// 4. partition by value_key; a member with none is its own partition
/// 5. one partition  → Settled | Reinforced | Corroborated
///    several        → Conflicted, no winner
/// ```
pub fn derive_subject(members: &[MemoryFacts], relations: &[Relation]) -> SubjectView {
    let member_ids: BTreeSet<Uuid> = members.iter().map(|m| m.id).collect();
    let decisions: Vec<Relation> = {
        let mut d: Vec<Relation> = relations
            .iter()
            .filter(|r| member_ids.contains(&r.from) || member_ids.contains(&r.to))
            .copied()
            .collect();
        d.sort();
        d.dedup();
        d
    };

    // A recorded scope exception: the narrower proposal points at the broader.
    let narrowed_by: Vec<Uuid> = {
        let mut n: Vec<Uuid> = decisions
            .iter()
            .filter(|r| r.kind == RelationKind::Narrows && member_ids.contains(&r.to))
            .map(|r| r.from)
            .collect();
        n.sort();
        n.dedup();
        n
    };

    // Step 1 — only a lifecycle-active proposal can be a current answer. A
    // stale or superseded one is history, and history has no canonical answer.
    let active: Vec<&MemoryFacts> = members
        .iter()
        .filter(|m| m.state == MemoryState::Active)
        .collect();
    if active.is_empty() {
        return SubjectView::historical(decisions, narrowed_by);
    }
    let active_ids: BTreeSet<Uuid> = active.iter().map(|m| m.id).collect();

    // Step 2 — a proposal another *active* proposal supersedes or duplicates is
    // no longer a candidate. Restricting to active endpoints matters: a
    // duplicate of a memory that has since been superseded is still a
    // candidate in its own right.
    let mut supersedes_edges: Vec<(Uuid, Uuid)> = Vec::new();
    let mut duplicates_edges: Vec<(Uuid, Uuid)> = Vec::new();
    for r in &decisions {
        if !active_ids.contains(&r.from) || !active_ids.contains(&r.to) {
            continue;
        }
        // A relation from a proposal to itself says nothing. Acting on it would
        // let one malformed row delete the only answer a subject has.
        if r.from == r.to {
            continue;
        }
        match r.kind {
            RelationKind::Supersedes => supersedes_edges.push((r.from, r.to)),
            // `from` duplicates `to`: the newer proposal points at the member it
            // duplicates, so `from` is the one that drops out.
            RelationKind::Duplicates => duplicates_edges.push((r.from, r.to)),
            _ => {}
        }
    }

    // Two machines that each recorded the opposite supersession — the ordinary
    // offline case (D78) — decided nothing between them. Applying both drops
    // both members, and any third value nobody argued about is left standing as
    // the sole settled answer: a winner nobody chose. Inside a cycle the
    // supersessions cancel, the members stay, and the subject reports the
    // disagreement it actually is (FR-303, SC-302).
    let supersedes_cycles = mutually_superseding(&supersedes_edges);
    let mut superseded_targets: BTreeSet<Uuid> = BTreeSet::new();
    for (from, to) in &supersedes_edges {
        if !in_one_cycle(&supersedes_cycles, *from, *to) {
            superseded_targets.insert(*to);
        }
    }

    // A duplicate cycle is not a disagreement: the members say the same thing,
    // and each store recorded that fact pointing the other way. Dropping both
    // would report two identical claims as conflicting. One is the answer and
    // the rest are its duplicates; which one is arbitrary, so it is settled by
    // identifier — the one choice that cannot depend on arrival order.
    let duplicate_cycles = mutually_superseding(&duplicates_edges);
    let mut duplicate_of: BTreeMap<Uuid, Uuid> = BTreeMap::new();
    for (from, to) in &duplicates_edges {
        if !in_one_cycle(&duplicate_cycles, *from, *to) {
            duplicate_of.insert(*from, *to);
        }
    }
    for group in &duplicate_cycles {
        let Some(survivor) = group.iter().next().copied() else {
            continue;
        };
        for member in group.iter().skip(1) {
            duplicate_of.insert(*member, survivor);
        }
    }

    let remaining: Vec<&MemoryFacts> = active
        .iter()
        .filter(|m| !superseded_targets.contains(&m.id) && !duplicate_of.contains_key(&m.id))
        .copied()
        .collect();

    // Step 3 — nothing remains. Two very different reasons, and conflating them
    // would silently resolve a disagreement.
    if remaining.is_empty() {
        // A supersession cycle — A supersedes B and B supersedes A — resolves
        // nothing. Reporting it as history would let two machines' mutually
        // exclusive decisions annihilate a subject. It is a disagreement about
        // which replaces which, so it is reported as one (T034).
        let mut answers: Vec<Uuid> = active.iter().map(|m| m.id).collect();
        answers.sort();
        let accounting = answers
            .iter()
            .map(|id| AnswerAccounting {
                memory_id: *id,
                duplicates: Vec::new(),
                distinct_origins: 1,
            })
            .collect();
        return SubjectView {
            reconciliation: Reconciliation::Conflicted,
            answers,
            accounting,
            narrowed_by,
            decisions,
        };
    }

    // Step 4 — partition by value key. A member with no value key never merges
    // with anything: it forms a partition of its own, keyed by its identifier.
    let mut partitions: BTreeMap<String, Vec<&MemoryFacts>> = BTreeMap::new();
    for m in &remaining {
        let key = match &m.value_key {
            Some(v) => format!("v:{v}"),
            None => format!("m:{}", m.id),
        };
        partitions.entry(key).or_default().push(m);
    }

    // Duplicates that were dropped still belong to the answer they duplicate,
    // for reinforcement and distinct-origin accounting.
    let duplicates_by_target = |target: Uuid| -> Vec<Uuid> {
        let mut v: Vec<Uuid> = duplicate_of
            .iter()
            .filter(|(_, to)| **to == target)
            .map(|(from, _)| *from)
            .collect();
        v.sort();
        v
    };
    let origins_for = |answer: &MemoryFacts, dropped: &[Uuid]| -> usize {
        let mut origins: BTreeSet<Uuid> = BTreeSet::new();
        origins.insert(answer.origin_session_id);
        for id in dropped {
            if let Some(m) = members.iter().find(|m| m.id == *id) {
                origins.insert(m.origin_session_id);
            }
        }
        origins.len()
    };

    if partitions.len() == 1 {
        let partition = partitions.values().next().expect("one partition");

        if partition.len() == 1 {
            let answer = partition[0];
            let dropped = duplicates_by_target(answer.id);
            let distinct_origins = origins_for(answer, &dropped);
            // A single remaining member that absorbed duplicates is
            // `Reinforced` — the duplication is what the state reports. With no
            // duplicates it is simply `Settled`.
            let reconciliation = if dropped.is_empty() {
                Reconciliation::Settled
            } else {
                Reconciliation::Reinforced
            };
            return SubjectView {
                reconciliation,
                answers: vec![answer.id],
                accounting: vec![AnswerAccounting {
                    memory_id: answer.id,
                    duplicates: dropped,
                    distinct_origins,
                }],
                narrowed_by,
                decisions,
            };
        }

        // Several members share one value key. Whether that is one claim or
        // several is decided by content, never by the key (FR-327, D77).
        let one_content = single_content_digest(partition);
        if one_content {
            let answer = representative(partition);
            let mut dropped = duplicates_by_target(answer.id);
            for m in partition {
                if m.id != answer.id {
                    dropped.push(m.id);
                }
            }
            dropped.sort();
            dropped.dedup();
            let distinct_origins = origins_for(answer, &dropped);
            return SubjectView {
                reconciliation: Reconciliation::Reinforced,
                answers: vec![answer.id],
                accounting: vec![AnswerAccounting {
                    memory_id: answer.id,
                    duplicates: dropped,
                    distinct_origins,
                }],
                narrowed_by,
                decisions,
            };
        }

        // Corroborated: they agree on the value and differ in what they say.
        // Not a conflict — the members agree. Not a merge — they say different
        // things. Every distinct statement is an answer.
        let (answers, accounting) =
            distinct_content_answers(partition, &duplicates_by_target, &origins_for);
        return SubjectView {
            reconciliation: Reconciliation::Corroborated,
            answers,
            accounting,
            narrowed_by,
            decisions,
        };
    }

    // Step 5 — several value keys in one scope: every competing answer, and no
    // winner. Nothing here picks one, which is why there is no branch that
    // could (FR-334, I4).
    //
    // Within a partition the same rule applies as in the corroborated branch:
    // one answer per *distinct statement*, not one answer per value key. Keeping
    // a single representative per key and recording only byte-identical members
    // as its duplicates loses any member that shares the key and says something
    // else — `jwt`/HS256 beside `jwt`/RS256 — which would make a statement
    // disappear from a subject whose whole purpose is to show every competing
    // one (FR-334, metric 2b).
    let mut answers: Vec<Uuid> = Vec::new();
    let mut accounting: Vec<AnswerAccounting> = Vec::new();
    for partition in partitions.values() {
        let (part_answers, part_accounting) =
            distinct_content_answers(partition, &duplicates_by_target, &origins_for);
        answers.extend(part_answers);
        accounting.extend(part_accounting);
    }
    // Sorted by identifier for stable rendering — and for nothing else.
    let mut paired: Vec<(Uuid, AnswerAccounting)> = answers.into_iter().zip(accounting).collect();
    paired.sort_by_key(|(id, _)| *id);
    let (answers, accounting): (Vec<Uuid>, Vec<AnswerAccounting>) = paired.into_iter().unzip();

    SubjectView {
        reconciliation: Reconciliation::Conflicted,
        answers,
        accounting,
        narrowed_by,
        decisions,
    }
}

/// Whether every member of a partition says the same thing after
/// normalization.
///
/// A member with no recorded digest cannot be shown equal to anything, so it
/// makes the partition several statements rather than one. That is the
/// conservative direction: a missed duplicate costs a line of context, a false
/// merge suppresses a claim.
fn single_content_digest(partition: &[&MemoryFacts]) -> bool {
    let mut seen: Option<&str> = None;
    for m in partition {
        let Some(d) = m.content_norm_digest.as_deref() else {
            return false;
        };
        match seen {
            None => seen = Some(d),
            Some(first) if first == d => {}
            Some(_) => return false,
        }
    }
    seen.is_some()
}

/// The member of a partition that stands for it.
///
/// Most supporting evidence, then strongest verification, then lowest
/// identifier.
fn representative<'a>(partition: &[&'a MemoryFacts]) -> &'a MemoryFacts {
    partition
        .iter()
        .min_by_key(|m| m.representative_key())
        .copied()
        .expect("a partition is never empty")
}

/// The groups of proposals that reach each other in both directions.
///
/// A relation graph built from two machines' independent decisions has no
/// ordering authority behind it, so it can contain cycles that no single
/// machine ever created. Every member of one of these groups points, directly or
/// through others, at every other member — which is precisely the shape that
/// carries no decision.
///
/// Only groups of two or more are returned: a single proposal is not a cycle,
/// and self-relations are filtered out before this is called.
///
/// Kosaraju, in two linear passes, rather than asking "does each pair reach each
/// other" — `rebuild_supersession` runs this over *every* supersession in a
/// project, and pairwise reachability would make `doctor --rebuild-derived`
/// quadratic in the size of the knowledge base. Both passes iterate ordered
/// collections, so the grouping is deterministic and does not depend on the
/// order relations arrived in.
pub fn mutually_superseding(edges: &[(Uuid, Uuid)]) -> Vec<BTreeSet<Uuid>> {
    if edges.is_empty() {
        return Vec::new();
    }
    let mut forward: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
    let mut backward: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
    let mut nodes: BTreeSet<Uuid> = BTreeSet::new();
    for (from, to) in edges {
        forward.entry(*from).or_default().push(*to);
        backward.entry(*to).or_default().push(*from);
        nodes.insert(*from);
        nodes.insert(*to);
    }

    // Pass one: finishing order on the forward graph. The `bool` marks a frame
    // as already expanded, which is how an explicit stack records postorder —
    // recursion here would be bounded only by the size of the project.
    let mut visited: BTreeSet<Uuid> = BTreeSet::new();
    let mut finished: Vec<Uuid> = Vec::new();
    for start in &nodes {
        if visited.contains(start) {
            continue;
        }
        let mut stack: Vec<(Uuid, bool)> = vec![(*start, false)];
        while let Some((node, expanded)) = stack.pop() {
            if expanded {
                finished.push(node);
                continue;
            }
            if !visited.insert(node) {
                continue;
            }
            stack.push((node, true));
            if let Some(next) = forward.get(&node) {
                for m in next {
                    if !visited.contains(m) {
                        stack.push((*m, false));
                    }
                }
            }
        }
    }

    // Pass two: on the reversed graph, in reverse finishing order. Each tree is
    // one strongly connected component — a set of proposals that all reach each
    // other, which is exactly a set whose relations decide nothing.
    let mut assigned: BTreeSet<Uuid> = BTreeSet::new();
    let mut groups: Vec<BTreeSet<Uuid>> = Vec::new();
    for root in finished.iter().rev() {
        if assigned.contains(root) {
            continue;
        }
        let mut group: BTreeSet<Uuid> = BTreeSet::new();
        let mut stack = vec![*root];
        while let Some(node) = stack.pop() {
            if !assigned.insert(node) {
                continue;
            }
            group.insert(node);
            if let Some(next) = backward.get(&node) {
                for m in next {
                    if !assigned.contains(m) {
                        stack.push(*m);
                    }
                }
            }
        }
        if group.len() > 1 {
            groups.push(group);
        }
    }
    groups
}

/// Whether both ends of a relation sit in the same cycle, which is what makes
/// that relation cancel.
pub fn in_one_cycle(groups: &[BTreeSet<Uuid>], from: Uuid, to: Uuid) -> bool {
    groups.iter().any(|g| g.contains(&from) && g.contains(&to))
}

/// One answer per distinct normalized content, ranked then sorted.
///
/// Two members that say the same thing inside a corroborated partition collapse
/// to one answer; the third, differing member keeps its own. The subject is
/// still `Corroborated` — the value is agreed and the statements are several.
fn distinct_content_answers(
    partition: &[&MemoryFacts],
    duplicates_by_target: &dyn Fn(Uuid) -> Vec<Uuid>,
    origins_for: &dyn Fn(&MemoryFacts, &[Uuid]) -> usize,
) -> (Vec<Uuid>, Vec<AnswerAccounting>) {
    // Group by digest; a member with no digest is its own group, since it
    // cannot be shown equal to anything.
    let mut groups: BTreeMap<String, Vec<&MemoryFacts>> = BTreeMap::new();
    for m in partition {
        let key = match &m.content_norm_digest {
            Some(d) => format!("d:{d}"),
            None => format!("m:{}", m.id),
        };
        groups.entry(key).or_default().push(m);
    }

    let mut ranked: Vec<(std::cmp::Reverse<usize>, u8, Uuid, AnswerAccounting)> = Vec::new();
    for group in groups.values() {
        let answer = representative(group);
        let mut dropped = duplicates_by_target(answer.id);
        for m in group {
            if m.id != answer.id {
                dropped.push(m.id);
            }
        }
        dropped.sort();
        dropped.dedup();
        let accounting = AnswerAccounting {
            memory_id: answer.id,
            duplicates: dropped.clone(),
            distinct_origins: origins_for(answer, &dropped),
        };
        let (evidence, verification, id) = answer.representative_key();
        ranked.push((evidence, verification, id, accounting));
    }
    ranked.sort_by_key(|r| (r.0, r.1, r.2));

    let answers = ranked.iter().map(|(_, _, id, _)| *id).collect();
    let accounting = ranked.into_iter().map(|(_, _, _, acc)| acc).collect();
    (answers, accounting)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn keyed(n: u128, topic: &str, value: &str, content: &str) -> MemoryFacts {
        MemoryFacts {
            topic_key: normalize_topic_key(topic),
            value_key: normalize_value_key(value),
            content_norm_digest: Some(content_norm_digest(content)),
            origin_session_id: id(1000 + n),
            ..MemoryFacts::active(id(n), MemoryScope::Project, "p1")
        }
    }

    // -- normalization ----------------------------------------------------

    #[test]
    fn topic_key_normalization_table() {
        // `contracts/knowledge.md` §Normalization, verbatim.
        let cases: &[(&str, Option<&str>)] = &[
            (
                "Infrastructure.Production_Database",
                Some("infrastructure.production_database"),
            ),
            ("infra/prod-db", Some("infra_prod_db")),
            ("a.b.c.d.e.f.g", None),
            ("\"; DROP TABLE memories;--", Some("drop_table_memories")),
            ("데이터베이스", None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                normalize_topic_key(input).as_deref(),
                *expected,
                "topic key {input:?}"
            );
        }
    }

    #[test]
    fn a_slash_is_not_a_separator() {
        // Accepting path syntax would invite absolute paths into a column that
        // synchronizes, so `/` becomes `_` rather than splitting.
        assert_eq!(
            normalize_topic_key("a/b/c/d/e/f/g/h").as_deref(),
            Some("a_b_c_d_e_f_g_h"),
            "eight slash-separated parts are one segment, not eight"
        );
        assert_eq!(
            normalize_topic_key("/Users/dev/repo").as_deref(),
            Some("users_dev_repo"),
            "an absolute path normalizes to a key with no leading separator"
        );
    }

    #[test]
    fn topic_key_bounds_are_enforced() {
        assert_eq!(
            normalize_topic_key("a.b.c.d.e.f").as_deref(),
            Some("a.b.c.d.e.f")
        );
        assert_eq!(normalize_topic_key("a.b.c.d.e.f.g"), None);
        assert_eq!(normalize_topic_key(""), None);
        assert_eq!(normalize_topic_key("..."), None);
        assert_eq!(normalize_topic_key("___"), None, "trims to nothing");

        let long = "a".repeat(TOPIC_KEY_MAX_CHARS);
        assert_eq!(normalize_topic_key(&long).as_deref(), Some(long.as_str()));
        assert_eq!(
            normalize_topic_key(&"a".repeat(TOPIC_KEY_MAX_CHARS + 1)),
            None
        );
    }

    #[test]
    fn underscore_runs_collapse_and_edges_are_trimmed() {
        assert_eq!(
            normalize_topic_key("  service - - api  port  ").as_deref(),
            Some("service_api_port")
        );
        assert_eq!(
            normalize_topic_key("_leading.trailing_").as_deref(),
            Some("leading.trailing")
        );
    }

    #[test]
    fn empty_segments_are_dropped_not_kept() {
        assert_eq!(normalize_topic_key("a..b").as_deref(), Some("a.b"));
        assert_eq!(normalize_topic_key(".a.").as_deref(), Some("a"));
    }

    #[test]
    fn value_key_normalization() {
        assert_eq!(normalize_value_key("  JWT  ").as_deref(), Some("jwt"));
        assert_eq!(
            normalize_value_key("PostgreSQL\t16").as_deref(),
            Some("postgresql 16")
        );
        assert_eq!(normalize_value_key("   "), None);
        assert_eq!(normalize_value_key(""), None);
        assert_eq!(
            normalize_value_key(&"v".repeat(VALUE_KEY_MAX_CHARS)).as_deref(),
            Some("v".repeat(VALUE_KEY_MAX_CHARS).as_str())
        );
        assert_eq!(
            normalize_value_key(&"v".repeat(VALUE_KEY_MAX_CHARS + 1)),
            None
        );
    }

    #[test]
    fn content_digest_ignores_case_whitespace_and_trailing_punctuation() {
        let base = content_norm_digest("The production database is PostgreSQL.");
        for equivalent in [
            "the production database is postgresql",
            "  The   production database  is PostgreSQL!  ",
            "THE PRODUCTION DATABASE IS POSTGRESQL???",
            "The production\tproduction"
                .replace(
                    "production\tproduction",
                    "production database is PostgreSQL;",
                )
                .as_str(),
        ] {
            assert_eq!(
                content_norm_digest(equivalent),
                base,
                "should normalize equal: {equivalent:?}"
            );
        }
        assert_ne!(
            base,
            content_norm_digest("The production database is CockroachDB."),
            "different claims must not collide"
        );
    }

    #[test]
    fn content_digest_is_canonical_across_unicode_forms() {
        // The reason `unicode-normalization` is a dependency at all: without
        // NFC these two byte-different strings produce different digests and
        // exact-duplicate detection misses them (FR-326).
        let composed = "the caf\u{e9} service is deployed";
        let decomposed = "the cafe\u{301} service is deployed";
        assert_ne!(composed, decomposed, "the inputs really do differ in bytes");
        assert_eq!(
            content_norm_digest(composed),
            content_norm_digest(decomposed)
        );
    }

    #[test]
    fn the_digest_is_not_a_similarity_measure() {
        // Stated as a test because the failure mode is someone comparing
        // prefixes. Two nearly identical claims produce unrelated digests.
        let a = content_norm_digest("the api listens on 8080");
        let b = content_norm_digest("the api listens on 8081");
        assert_ne!(a, b);
        assert_ne!(a[..8], b[..8]);
    }

    // -- scope overlap -----------------------------------------------------

    #[test]
    fn scope_overlap_table() {
        use MemoryScope::*;
        use ScopeOverlap::*;
        let cases: &[(MemoryScope, &str, MemoryScope, &str, ScopeOverlap)] = &[
            (Project, "P", Project, "P", Simultaneous),
            (Branch, "main", Branch, "main", Simultaneous),
            (Task, "T1", Task, "T1", Simultaneous),
            (Session, "S1", Session, "S1", Simultaneous),
            (Project, "P", Task, "T1", ScopeException),
            (Project, "P", Branch, "main", ScopeException),
            (Branch, "main", Branch, "feature/x", Disjoint),
            (Task, "T1", Task, "T2", Disjoint),
        ];
        for (a, ak, b, bk, expected) in cases {
            assert_eq!(
                scope_overlap(*a, ak, *b, bk),
                *expected,
                "{a:?}:{ak} vs {b:?}:{bk}"
            );
            // Classification is symmetric: which side is named first cannot
            // change whether a conflict is possible.
            assert_eq!(scope_overlap(*b, bk, *a, ak), *expected);
        }
    }

    #[test]
    fn a_narrower_scope_wins_by_feature_001_precedence() {
        use MemoryScope::*;
        assert_eq!(narrower_scope(Project, Task), Task);
        assert_eq!(narrower_scope(Branch, Task), Task);
        assert_eq!(narrower_scope(Project, Branch), Branch);
        assert_eq!(narrower_scope(Project, Session), Project);
    }

    // -- symmetric normalization -------------------------------------------

    #[test]
    fn only_conflicts_with_normalizes_its_endpoints() {
        let (a, b) = (id(2), id(1));
        let (from, to) = normalize_relation_endpoints(RelationKind::ConflictsWith, a, b);
        assert_eq!((from, to), (id(1), id(2)), "min, max");

        // Recording it the other way round produces the identical row, which
        // is what makes the primary key absorb a second machine's record.
        assert_eq!(
            normalize_relation_endpoints(RelationKind::ConflictsWith, id(1), id(2)),
            normalize_relation_endpoints(RelationKind::ConflictsWith, id(2), id(1))
        );

        for directional in [
            RelationKind::Supersedes,
            RelationKind::Duplicates,
            RelationKind::Reinforces,
            RelationKind::Narrows,
            RelationKind::NotApplicableTo,
        ] {
            assert_eq!(
                normalize_relation_endpoints(directional, a, b),
                (a, b),
                "{directional} is directional and must be untouched"
            );
        }
    }

    // -- derive_subject ----------------------------------------------------

    #[test]
    fn a_lone_active_member_is_settled() {
        let m = keyed(
            1,
            "infra.db",
            "postgresql",
            "The production database is PostgreSQL.",
        );
        let v = derive_subject(std::slice::from_ref(&m), &[]);
        assert_eq!(v.reconciliation, Reconciliation::Settled);
        assert_eq!(v.answers, vec![m.id]);
        assert_eq!(v.accounting[0].distinct_origins, 1);
    }

    #[test]
    fn every_member_superseded_or_stale_is_historical() {
        let mut a = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
        a.state = MemoryState::Superseded;
        let mut b = keyed(2, "infra.db", "mysql", "MySQL.");
        b.state = MemoryState::Stale;
        let v = derive_subject(&[a, b], &[]);
        assert_eq!(v.reconciliation, Reconciliation::Historical);
        assert!(v.answers.is_empty());
    }

    #[test]
    fn identical_content_reinforces_into_one_answer() {
        // Three sessions recording the same thing (US1 scenario A).
        let a = keyed(
            1,
            "infra.db",
            "postgresql",
            "The production database is PostgreSQL.",
        );
        let b = keyed(
            2,
            "infra.db",
            "postgresql",
            "the production   database is postgresql!",
        );
        let c = keyed(
            3,
            "infra.db",
            "postgresql",
            "THE PRODUCTION DATABASE IS POSTGRESQL",
        );
        let v = derive_subject(&[a.clone(), b.clone(), c.clone()], &[]);
        assert_eq!(v.reconciliation, Reconciliation::Reinforced);
        assert_eq!(v.answers.len(), 1);
        assert_eq!(
            v.accounting[0].distinct_origins, 3,
            "three distinct origins"
        );
        assert_eq!(v.accounting[0].duplicates.len(), 2);
    }

    #[test]
    fn an_equal_value_key_with_differing_content_corroborates_and_never_merges() {
        // The false-merge path R12 closed. Both statements are honest and
        // materially different; merging would suppress one and report a
        // reinforcement that never happened (FR-327, D77).
        let hs = keyed(
            1,
            "auth.strategy",
            "jwt",
            "JWT uses HS256 with a shared secret.",
        );
        let rs = keyed(
            2,
            "auth.strategy",
            "jwt",
            "JWT uses RS256 with rotating public keys.",
        );
        let v = derive_subject(&[hs.clone(), rs.clone()], &[]);
        assert_eq!(v.reconciliation, Reconciliation::Corroborated);
        assert_eq!(v.answers.len(), 2, "every statement is retained");
        assert!(v.answers.contains(&hs.id) && v.answers.contains(&rs.id));
        assert!(
            v.decisions.is_empty(),
            "corroboration records no relation at all"
        );
    }

    #[test]
    fn corroboration_collapses_only_identical_statements() {
        // Two members say the same thing, a third differs. The subject is still
        // Corroborated — the value is agreed, the statements are several — and
        // the identical pair becomes one answer.
        let a = keyed(
            1,
            "auth.strategy",
            "jwt",
            "JWT uses HS256 with a shared secret.",
        );
        let b = keyed(
            2,
            "auth.strategy",
            "jwt",
            "jwt uses hs256 with a shared secret",
        );
        let c = keyed(
            3,
            "auth.strategy",
            "jwt",
            "JWT uses RS256 with rotating public keys.",
        );
        let v = derive_subject(&[a.clone(), b.clone(), c.clone()], &[]);
        assert_eq!(v.reconciliation, Reconciliation::Corroborated);
        assert_eq!(
            v.answers.len(),
            2,
            "distinct *content*, not distinct member"
        );
        let collapsed = v
            .accounting
            .iter()
            .find(|acc| !acc.duplicates.is_empty())
            .expect("one answer stands for two members");
        assert_eq!(collapsed.distinct_origins, 2);
    }

    #[test]
    fn differing_value_keys_in_one_scope_conflict_with_no_winner() {
        let pg = keyed(
            1,
            "infra.db",
            "postgresql",
            "The production database is PostgreSQL.",
        );
        let cr = keyed(
            2,
            "infra.db",
            "cockroachdb",
            "The production database is CockroachDB.",
        );
        let v = derive_subject(&[pg.clone(), cr.clone()], &[]);
        assert_eq!(v.reconciliation, Reconciliation::Conflicted);
        assert_eq!(v.answers, vec![pg.id, cr.id], "sorted by id, both returned");
        assert!(!v.is_settled());
    }

    #[test]
    fn a_supersession_removes_its_target_and_settles_the_subject() {
        let old = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
        let new = keyed(2, "infra.db", "cockroachdb", "CockroachDB.");
        let r = Relation::new(
            RelationKind::Supersedes,
            new.id,
            old.id,
            RelationBasis::ExplicitUser,
        );
        let v = derive_subject(&[old.clone(), new.clone()], &[r]);
        assert_eq!(v.reconciliation, Reconciliation::Settled);
        assert_eq!(v.answers, vec![new.id]);
    }

    #[test]
    fn a_mutual_supersession_is_reported_not_resolved() {
        // Two machines each decided the other's proposal was replaced. Dropping
        // both would let mutually exclusive decisions annihilate the subject;
        // it is a disagreement, so it is reported as one (T034).
        let a = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
        let b = keyed(2, "infra.db", "cockroachdb", "CockroachDB.");
        let relations = [
            Relation::new(
                RelationKind::Supersedes,
                a.id,
                b.id,
                RelationBasis::ExplicitAgent,
            ),
            Relation::new(
                RelationKind::Supersedes,
                b.id,
                a.id,
                RelationBasis::ExplicitAgent,
            ),
        ];
        let v = derive_subject(&[a.clone(), b.clone()], &relations);
        assert_eq!(v.reconciliation, Reconciliation::Conflicted);
        assert_eq!(v.answers, vec![a.id, b.id]);
    }

    #[test]
    fn a_recorded_narrowing_is_reported_as_a_scope_exception() {
        let broad = keyed(
            1,
            "infra.db",
            "postgresql",
            "The production database is PostgreSQL.",
        );
        let narrow_id = id(2);
        let r = Relation::new(
            RelationKind::Narrows,
            narrow_id,
            broad.id,
            RelationBasis::ExplicitAgent,
        );
        let v = derive_subject(std::slice::from_ref(&broad), &[r]);
        assert_eq!(v.reconciliation, Reconciliation::Settled);
        assert_eq!(v.narrowed_by, vec![narrow_id]);
    }

    #[test]
    fn the_representative_prefers_evidence_then_a_deterministic_check() {
        let mut plain = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
        let mut attested = keyed(2, "infra.db", "postgresql", "PostgreSQL.");
        let mut checked = keyed(3, "infra.db", "postgresql", "PostgreSQL.");

        plain.evidence_fact_count = 0;
        attested.evidence_fact_count = 2;
        attested.verification = VerificationState::Verified;
        attested.verification_authority = Some(VerificationAuthority::Attested);
        checked.evidence_fact_count = 2;
        checked.verification = VerificationState::Verified;
        checked.verification_authority = Some(VerificationAuthority::Cairn);

        let v = derive_subject(&[plain, attested.clone(), checked.clone()], &[]);
        assert_eq!(v.reconciliation, Reconciliation::Reinforced);
        assert_eq!(
            v.answers,
            vec![checked.id],
            "a deterministic check outranks an attestation with equal evidence"
        );
    }

    #[test]
    fn no_clock_and_no_identifier_order_decides_a_winner() {
        // The mutation-style proof: identifiers are the only ordering input the
        // type can express, and reversing them must not change *which* answers
        // are returned — only their order (FR-303, D49).
        let pg_low = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
        let cr_high = keyed(9, "infra.db", "cockroachdb", "CockroachDB.");
        let a = derive_subject(&[pg_low.clone(), cr_high.clone()], &[]);

        let mut pg_high = pg_low.clone();
        pg_high.id = id(9);
        let mut cr_low = cr_high.clone();
        cr_low.id = id(1);
        let b = derive_subject(&[pg_high, cr_low], &[]);

        assert_eq!(a.reconciliation, b.reconciliation);
        assert_eq!(a.answers.len(), b.answers.len());
        assert_eq!(a.answers.len(), 2, "neither ordering produced a winner");
    }

    #[test]
    fn the_derivation_does_not_depend_on_relation_order() {
        let a = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
        let b = keyed(2, "infra.db", "mysql", "MySQL.");
        let c = keyed(3, "infra.db", "cockroachdb", "CockroachDB.");
        let relations = vec![
            Relation::new(
                RelationKind::Supersedes,
                c.id,
                b.id,
                RelationBasis::ExplicitUser,
            ),
            Relation::new(
                RelationKind::Supersedes,
                b.id,
                a.id,
                RelationBasis::ExplicitUser,
            ),
        ];
        let forward = derive_subject(&[a.clone(), b.clone(), c.clone()], &relations);
        let reversed: Vec<Relation> = relations.iter().rev().copied().collect();
        let backward = derive_subject(&[c.clone(), a.clone(), b.clone()], &reversed);
        assert_eq!(forward, backward);
        assert_eq!(forward.answers, vec![c.id]);
    }

    #[test]
    fn a_free_form_member_never_merges_with_anything() {
        // No topic key, no value key, no digest: it can never be shown equal to
        // another member, so it stands alone (FR-313, FR-317).
        let mut a = MemoryFacts::active(id(1), MemoryScope::Project, "p1");
        let mut b = MemoryFacts::active(id(2), MemoryScope::Project, "p1");
        a.origin_session_id = id(101);
        b.origin_session_id = id(102);
        let v = derive_subject(&[a, b], &[]);
        assert_eq!(
            v.reconciliation,
            Reconciliation::Conflicted,
            "two unkeyed proposals are two separate partitions"
        );
        assert_eq!(v.answers.len(), 2, "neither is dropped");
        assert!(v.decisions.is_empty(), "and no relation was invented");
    }

    #[test]
    fn a_duplicate_is_dropped_from_answers_but_stays_counted() {
        // FR-321/FR-322: individually retrievable, and still an origin.
        let existing = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
        let newer = keyed(2, "infra.db", "postgresql", "PostgreSQL.");
        let r = Relation::new(
            RelationKind::Duplicates,
            newer.id,
            existing.id,
            RelationBasis::DeterministicRule,
        );
        let v = derive_subject(&[existing.clone(), newer.clone()], &[r]);
        assert_eq!(v.answers, vec![existing.id]);
        assert_eq!(v.accounting[0].duplicates, vec![newer.id]);
        assert_eq!(v.accounting[0].distinct_origins, 2);
        assert_eq!(v.reconciliation, Reconciliation::Reinforced);
    }
}

#[cfg(test)]
mod proposal_tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn keyed(n: u128, topic: &str, value: &str, content: &str) -> MemoryFacts {
        MemoryFacts {
            topic_key: normalize_topic_key(topic),
            value_key: normalize_value_key(value),
            content_norm_digest: Some(content_norm_digest(content)),
            origin_session_id: id(1000 + n),
            ..MemoryFacts::active(id(n), MemoryScope::Project, "p1")
        }
    }

    const MAX: usize = 64;

    #[test]
    fn identical_content_is_the_one_automatic_merge() {
        let existing = keyed(
            1,
            "infra.db",
            "postgresql",
            "The production database is PostgreSQL.",
        );
        let proposal = keyed(
            2,
            "infra.db",
            "postgresql",
            "the production   database is postgresql!",
        );
        let (outcome, relations) =
            classify_proposal(&proposal, std::slice::from_ref(&existing), MAX);
        assert_eq!(outcome, ProposalOutcome::Duplicate { of: existing.id });
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].kind, RelationKind::Duplicates);
        assert_eq!(relations[0].from, proposal.id, "new points at existing");
        assert_eq!(relations[0].basis, RelationBasis::DeterministicRule);
    }

    #[test]
    fn an_equal_value_key_with_differing_content_records_nothing() {
        // Metric 2a: zero unrequested `reinforces` relations, ever. And metric
        // 2b: the writer is told which member it matched, so the party that can
        // read both statements can decide.
        let existing = keyed(
            1,
            "auth.strategy",
            "jwt",
            "JWT uses HS256 with a shared secret.",
        );
        let proposal = keyed(
            2,
            "auth.strategy",
            "jwt",
            "JWT uses RS256 with rotating public keys.",
        );
        let (outcome, relations) =
            classify_proposal(&proposal, std::slice::from_ref(&existing), MAX);
        assert_eq!(
            outcome,
            ProposalOutcome::Corroborating {
                member: existing.id
            }
        );
        assert!(relations.is_empty(), "corroboration writes nothing");
    }

    #[test]
    fn a_differing_value_key_in_one_scope_detects_a_conflict() {
        let existing = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
        let proposal = keyed(2, "infra.db", "cockroachdb", "CockroachDB.");
        let (outcome, relations) =
            classify_proposal(&proposal, std::slice::from_ref(&existing), MAX);
        assert_eq!(
            outcome,
            ProposalOutcome::ConflictDetected {
                with: vec![existing.id]
            }
        );
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].kind, RelationKind::ConflictsWith);
        // Symmetric endpoints are normalized at construction, so the row two
        // machines write independently is the same row.
        assert_eq!(
            (relations[0].from, relations[0].to),
            (existing.id, proposal.id)
        );
    }

    #[test]
    fn a_scope_exception_is_never_a_conflict() {
        // Scenario B: project PostgreSQL, task SQLite fixture.
        let project = keyed(
            1,
            "infra.db",
            "postgresql",
            "The production database is PostgreSQL.",
        );
        let mut task = keyed(
            2,
            "infra.db",
            "sqlite",
            "This integration fixture uses SQLite.",
        );
        task.scope = MemoryScope::Task;
        task.scope_key = "T1".into();
        let (outcome, relations) = classify_proposal(&task, &[project], MAX);
        assert_eq!(outcome, ProposalOutcome::Created);
        assert!(relations.is_empty());
    }

    #[test]
    fn two_branches_never_interact() {
        let mut main = keyed(1, "api.style", "rest", "The API is REST.");
        main.scope = MemoryScope::Branch;
        main.scope_key = "main".into();
        let mut feature = keyed(2, "api.style", "graphql", "The API is GraphQL.");
        feature.scope = MemoryScope::Branch;
        feature.scope_key = "feature/graphql".into();
        let (outcome, relations) = classify_proposal(&feature, &[main], MAX);
        assert_eq!(outcome, ProposalOutcome::Created);
        assert!(relations.is_empty());
    }

    #[test]
    fn a_free_form_proposal_reconciles_against_nothing() {
        // FR-313 and FR-317. Even with byte-identical content: a subject
        // requires a topic key, and without one there is no subject to join.
        let mut existing = MemoryFacts::active(id(1), MemoryScope::Project, "p1");
        existing.content_norm_digest = Some(content_norm_digest("The same sentence."));
        let mut proposal = MemoryFacts::active(id(2), MemoryScope::Project, "p1");
        proposal.content_norm_digest = Some(content_norm_digest("The same sentence."));

        let (outcome, relations) = classify_proposal(&proposal, &[existing], MAX);
        assert_eq!(outcome, ProposalOutcome::Created);
        assert!(
            relations.is_empty(),
            "no relation is invented for a free-form pair"
        );
    }

    #[test]
    fn a_superseded_member_is_not_reconciled_against() {
        let mut old = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
        old.state = MemoryState::Superseded;
        let proposal = keyed(2, "infra.db", "cockroachdb", "CockroachDB.");
        let (outcome, relations) = classify_proposal(&proposal, &[old], MAX);
        assert_eq!(
            outcome,
            ProposalOutcome::Created,
            "history does not conflict"
        );
        assert!(relations.is_empty());
    }

    #[test]
    fn an_oversized_subject_defers_rather_than_scanning() {
        // FR-474: the write completes and the relation is deferred to the
        // maintenance tick. Never unbounded work on a write path.
        let members: Vec<MemoryFacts> = (10..40)
            .map(|n| keyed(n, "infra.db", "postgresql", &format!("statement {n}")))
            .collect();
        let proposal = keyed(1, "infra.db", "cockroachdb", "CockroachDB.");
        let (outcome, relations) = classify_proposal(&proposal, &members, 8);
        assert_eq!(outcome, ProposalOutcome::Deferred);
        assert!(relations.is_empty());
        // And within the bound it decides normally.
        let (decided, _) = classify_proposal(&proposal, &members, MAX);
        assert!(matches!(decided, ProposalOutcome::ConflictDetected { .. }));
    }

    #[test]
    fn duplication_outranks_corroboration_and_conflict() {
        // Order matters: a proposal identical to one member and differing from
        // another is a duplicate of the first, not a conflict with the second.
        let same = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
        let other = keyed(2, "infra.db", "cockroachdb", "CockroachDB.");
        let proposal = keyed(3, "infra.db", "postgresql", "postgresql");
        let (outcome, _) = classify_proposal(&proposal, &[same.clone(), other], MAX);
        assert_eq!(outcome, ProposalOutcome::Duplicate { of: same.id });
    }

    #[test]
    fn a_proposal_with_no_value_key_conflicts_with_nothing() {
        // A value key is what makes two claims comparable. Without one there is
        // nothing to disagree about that Cairn could decide.
        let existing = keyed(1, "infra.db", "postgresql", "PostgreSQL.");
        let mut proposal = keyed(2, "infra.db", "x", "Something else entirely.");
        proposal.value_key = None;
        let (outcome, relations) = classify_proposal(&proposal, &[existing], MAX);
        assert_eq!(outcome, ProposalOutcome::Created);
        assert!(relations.is_empty());
    }
}
