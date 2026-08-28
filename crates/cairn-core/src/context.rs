//! Briefing assembly (FR-027 – FR-031, D8; FR-441 – FR-448).
//!
//! Sections are admitted in a fixed priority order, each measured with the
//! Cairn token estimator *before* it is emitted. That makes budget compliance a
//! property of this loop rather than a statistic: the output can never exceed
//! the budget.
//!
//! # Three levels, and why Level 0 has two tiers
//!
//! ```text
//! LEVEL 0  minimum safe continuity   ── a reserved share the lower levels cannot take
//! LEVEL 1  relevant current knowledge ── the remaining budget, ranked
//! LEVEL 2  history and evidence       ── never automatic; explicit request only
//! ```
//!
//! A budget is finite. Criterion text, blocker descriptions and warning detail
//! are not. Guaranteeing that all of them fit would be a promise Cairn cannot
//! keep, so Level 0 splits:
//!
//! * **Tier 0a** — the guaranteed work state. Every item is O(1) in the size of
//!   the project and the task, so the tier has a bounded worst case that fits
//!   the documented minimum budget. After any number of compactions the agent
//!   still knows what it is doing, how far along it is, what is blocking it and
//!   that something is wrong (FR-443).
//! * **Tier 0b** — bounded detail, admitted in a documented order until the
//!   budget binds, with whatever does not fit counted by kind and given a
//!   retrieval path. Omission is never silent (FR-448).
//!
//! The reserve is a **cap on the lower levels, not a floor Level 0 must spend**.
//! Unspent reserve returns to the general pool, which is why a project with no
//! task, no warnings and no pins delivers exactly what it delivered before this
//! feature existed (FR-442).

use crate::budget::{estimate, Budget};
use crate::domain::*;
use crate::tasks::{self, BlockerFacts, CriterionFacts};
use crate::wire::*;

/// Sections, highest priority first (D8).
pub const SECTION_ORDER: &[&str] = &[
    "task",
    "repository",
    "previous_handoff",
    "known_failures",
    "decisions",
    "task_memory",
    "branch_memory",
    "project_memory",
    // Last, deliberately. A prior pattern from another project is the least
    // authoritative thing in a briefing, so it is the first thing a tight
    // budget drops (FR-398).
    "patterns",
    // Feature 004. Both after every project-scoped section, and personal
    // ahead of team: the same specificity gradient the order above already
    // expresses (task > branch > project) continues past "now" — personal
    // knowledge is specific to the one account asking, team guidance is the
    // server-wide default with no actor-specific claim at all (FR-476, D422,
    // `contracts/recall-composition.md` §4).
    "personal_notes",
    "team_guidance",
];

/// Sections whose loss means the briefing is materially degraded (SC-003).
pub const HIGH_PRIORITY_SECTIONS: &[&str] = &["task", "repository", "previous_handoff"];

/// Everything the briefing is assembled from. All recorded state.
pub struct ContextInputs<'a> {
    pub project: &'a Project,
    pub repository: RepositoryState,
    pub task: Option<&'a Task>,
    pub previous_handoff: Option<&'a Handoff>,
    pub decisions: &'a [String],
    pub known_failures: &'a [String],
    pub task_memory: &'a [String],
    pub branch_memory: &'a [String],
    pub project_memory: &'a [String],
    /// Signal-matched prior patterns, already capped and ordered by the caller.
    pub patterns: &'a [BriefingPattern],
    /// False for a project Cairn has never seen before (FR-031).
    pub has_history: bool,
    /// True when assembly ran out of time or storage was unavailable (FR-046).
    pub degraded: bool,
    /// Everything Feature 003 adds. Defaulted, so a caller that has none of it
    /// gets Feature 001's briefing unchanged.
    pub level0: Level0<'a>,
    /// Feature 004's personal candidates, already filtered by applicability and
    /// ranked within their own domain by the caller.
    ///
    /// Empty is the ordinary case and costs nothing: `admit_global` treats empty
    /// slices as "nothing to admit, zero spend", which is also how a
    /// `depth: "minimum"` request excludes both sections (FR-477).
    pub personal_notes: &'a [PersonalCandidate],
    /// Feature 004's team candidates, same treatment.
    pub team_guidance: &'a [TeamCandidate],
}

/// The Level 0 inputs. All plain data — this crate never reaches the store.
#[derive(Default)]
pub struct Level0<'a> {
    pub criteria: &'a [CriterionFacts],
    pub blockers: &'a [BlockerFacts],
    /// Blocker descriptions by id, so the most actionable one can be named.
    pub blocker_text: &'a [(uuid::Uuid, String)],
    pub warnings: &'a [ContextWarning],
    pub pins: &'a [PinnedConstraint],
    /// The recorded next action of a diverged checkpoint. Phase 9 supplies it;
    /// until then there are no checkpoints and it is absent, costing nothing.
    pub previous_next_action: Option<&'a str>,
    /// Emit the selection diagnostics (FR-461, FR-463).
    pub explain: bool,
    pub caps: Caps,
}

/// The bounds Level 0 admits within.
#[derive(Debug, Clone, Copy)]
pub struct Caps {
    pub goal_max_tokens: usize,
    pub warnings_in_context_max: usize,
    pub pins_in_context_max: usize,
    /// `floor(limit * min_safe_context_fraction)` is computed by the caller from
    /// config; this is the fraction already applied.
    pub reserve_fraction: f64,
    /// The ceiling on combined `personal_notes` + `team_guidance` spend, as a
    /// fraction of the **total** budget (D421, D450, FR-474).
    ///
    /// Independent of `reserve_fraction` on purpose: the reserve bounds what
    /// Level 1 and Level 2 may take from Level 0's share, while this bounds
    /// what global sections may take from *everyone's* share, including their
    /// own. A pinned constant rather than a caller-chosen number in practice —
    /// see [`GLOBAL_SHARE_MAX`] — but a `Caps` field, like `reserve_fraction`,
    /// so it travels with the rest of the budget configuration instead of
    /// being threaded through every call site separately.
    pub global_share_max: f64,
}

/// `global_share_max`, pinned rather than left to the caller (D450): an
/// unnamed "documented fraction" is a requirement no two implementations
/// could be tested against identically. `Caps::default()` uses this value;
/// nothing in this module reads a different one today.
pub const GLOBAL_SHARE_MAX: f64 = 0.15;

impl Default for Caps {
    fn default() -> Self {
        Self {
            goal_max_tokens: 60,
            warnings_in_context_max: 5,
            pins_in_context_max: 4,
            reserve_fraction: 0.40,
            global_share_max: GLOBAL_SHARE_MAX,
        }
    }
}

/// Assemble a briefing that fits `budget_tokens` estimated tokens.
pub fn assemble(input: &ContextInputs<'_>, budget_tokens: usize) -> ContextPayload {
    // This expression's only inputs are `budget_tokens` and a compile-time
    // fraction. Personal and team knowledge are not read anywhere above this
    // line, are not part of `Level0`, and `admit_global` — the function that
    // does read them — is not called until Level 1, after `release_reserve`
    // has already run. There is no arithmetic expression here that could
    // admit a global byte count into `reserve` (D420, FR-473).
    let reserve = (budget_tokens as f64 * input.level0.caps.reserve_fraction).floor() as usize;
    let mut budget = Budget::with_reserve(budget_tokens, reserve);
    let mut omitted: Vec<String> = Vec::new();
    let mut selection = Selection {
        budget: budget_tokens,
        reserve,
        ..Default::default()
    };

    // The project header is the frame everything else hangs on; it is charged
    // first and is small enough that dropping it would mean a useless briefing.
    // Charged against the reserve, so withholding one cannot make the frame fail
    // where it previously fit.
    let header_cost = estimate(&input.project.name) + 8;
    budget.try_spend_reserved(header_cost);

    let mut briefing = Briefing {
        project: ProjectSummary::from(input.project),
        repository: RepositoryState::default(),
        task: None,
        previous_handoff: None,
        decisions: Vec::new(),
        known_failures: Vec::new(),
        memory: BriefingMemory::default(),
        no_prior_history: !input.has_history,
        warnings: Vec::new(),
        constraints: Vec::new(),
        previous_next_action: None,
        patterns: Vec::new(),
        personal_notes: Vec::new(),
        team_guidance: Vec::new(),
    };

    // ---- Level 0 -----------------------------------------------------------
    //
    // Tier 0a first, then Tier 0b, both drawing on the reserve before the
    // general pool. Tier 0b can never displace Tier 0a because it is admitted
    // after it.
    if !admit_tier_0a(&mut briefing, input, &mut budget, &mut selection) {
        omitted.push("task".into());
    }
    admit_tier_0b(&mut briefing, input, &mut budget, &mut selection);

    // Whatever Level 0 did not spend goes back. This single call is what makes
    // the no-regression property true (FR-442).
    budget.release_reserve();
    selection.reserve_used = budget.reserve_used();
    selection.reserve_released = budget.reserve_released();

    // ---- Level 1 -----------------------------------------------------------
    // How many items each section had, so an omission can be counted rather
    // than merely named. FR-461 requires *every* omission to carry a reason,
    // and `omitted_sections` says only which section was cut short — not how
    // much of it went, nor why. The explain view is where that has to be
    // answerable.
    let section_total = |section: &str| -> usize {
        match section {
            "known_failures" => input.known_failures.len(),
            "decisions" => input.decisions.len(),
            "task_memory" => input.task_memory.len(),
            "branch_memory" => input.branch_memory.len(),
            "project_memory" => input.project_memory.len(),
            "patterns" => input.patterns.len(),
            _ => 0,
        }
    };
    let section_kept = |b: &Briefing, section: &str| -> usize {
        match section {
            "known_failures" => b.known_failures.len(),
            "decisions" => b.decisions.len(),
            "task_memory" => b.memory.task.len(),
            "branch_memory" => b.memory.branch.len(),
            "project_memory" => b.memory.project.len(),
            "patterns" => b.patterns.len(),
            _ => 0,
        }
    };

    for section in SECTION_ORDER {
        let admitted = match *section {
            // Admitted above, as Tier 0a.
            "task" | "repository" => continue,
            "previous_handoff" => admit_handoff(&mut briefing, input, &mut budget),
            "known_failures" => {
                briefing.known_failures = budget
                    .take_while_fits(input.known_failures.iter().cloned(), |s| estimate(s) + 1);
                briefing.known_failures.len() == input.known_failures.len()
            }
            "decisions" => {
                briefing.decisions =
                    budget.take_while_fits(input.decisions.iter().cloned(), |s| estimate(s) + 1);
                briefing.decisions.len() == input.decisions.len()
            }
            "task_memory" => {
                briefing.memory.task =
                    budget.take_while_fits(input.task_memory.iter().cloned(), |s| estimate(s) + 1);
                briefing.memory.task.len() == input.task_memory.len()
            }
            "branch_memory" => {
                briefing.memory.branch = budget
                    .take_while_fits(input.branch_memory.iter().cloned(), |s| estimate(s) + 1);
                briefing.memory.branch.len() == input.branch_memory.len()
            }
            "project_memory" => {
                briefing.memory.project = budget
                    .take_while_fits(input.project_memory.iter().cloned(), |s| estimate(s) + 1);
                briefing.memory.project.len() == input.project_memory.len()
            }
            "patterns" => {
                briefing.patterns =
                    budget.take_while_fits(input.patterns.iter().cloned(), pattern_cost);
                briefing.patterns.len() == input.patterns.len()
            }
            // The two global sections are admitted together, because their
            // shared cap is a property of the pair rather than of either one
            // (D421, D449). `personal_notes` runs the admission; `team_guidance`
            // reads what it already produced.
            "personal_notes" => {
                let (personal, team) = admit_global(
                    &mut budget,
                    &mut selection,
                    input.personal_notes,
                    input.team_guidance,
                    input.level0.caps.global_share_max,
                );
                let complete = personal.len() == input.personal_notes.len();
                briefing.personal_notes = personal;
                briefing.team_guidance = team;
                complete
            }
            "team_guidance" => briefing.team_guidance.len() == input.team_guidance.len(),
            _ => true,
        };
        if !admitted {
            omitted.push((*section).to_string());
            // Patterns arrive already capped and ordered by the caller, so
            // anything dropped here went for budget — the only reason this
            // loop can be the cause of.
            note_omission(
                &mut selection,
                if *section == "patterns" {
                    "pattern"
                } else {
                    "memory"
                },
                section_total(section).saturating_sub(section_kept(&briefing, section)),
                OmissionReason::BudgetExhausted,
                "cairn memory search",
            );
        }
    }

    ContextPayload {
        estimated_tokens: budget.spent(),
        budget: budget.limit(),
        truncated: !omitted.is_empty(),
        omitted_sections: omitted,
        degraded: input.degraded,
        briefing,
        selection: input.level0.explain.then_some(selection),
    }
}

/// What a suggestion costs, including the labelling that makes it honest.
///
/// The label is not optional and so is not free: a pattern rendered without
/// "unverified in this project" is a different, worse thing than a pattern that
/// did not fit.
fn pattern_cost(p: &BriefingPattern) -> usize {
    estimate(&p.title)
        + estimate(&p.approach)
        + p.applicability.iter().map(|a| estimate(a)).sum::<usize>()
        + p.constraints.iter().map(|c| estimate(c)).sum::<usize>()
        + p.alternative_cause.as_deref().map(estimate).unwrap_or(0)
        + p.check_this_first.as_deref().map(estimate).unwrap_or(0)
        // The label, the trust word and the field names.
        + 12
}

// ---------------------------------------------------------------------------
// Level 1 — the two global sections (`contracts/recall-composition.md`)
// ---------------------------------------------------------------------------

/// A personal-knowledge candidate for `personal_notes` (FR-476, D422).
///
/// Deliberately not `crate::global::PersonalKnowledge` directly: this module
/// never reaches the store (module doc), so it needs only what a candidate
/// costs and what it says, the same reduction `BriefingPattern` already
/// performs on a richer stored record.
///
/// `importance` is carried and **never read** by anything in this module. An
/// importance hint changes neither a section's position in `SECTION_ORDER`
/// nor its reserve eligibility (FR-482) — the field sits here, populated, so
/// that claim is checkable (`importance_hint_changes_nothing`, below) rather
/// than true only because nothing offered a hint to ignore.
#[derive(Debug, Clone)]
pub struct PersonalCandidate {
    pub id: uuid::Uuid,
    pub content: String,
    pub importance: Importance,
}

/// A team-knowledge candidate for `team_guidance`. See [`PersonalCandidate`];
/// the two are kept as distinct types rather than one shared struct because
/// nothing about this feature promises they stay identical — team knowledge
/// may grow fields (e.g. its ratification state) that a personal record must
/// never carry, and a shared type would be the thing that quietly let one
/// leak into the other.
#[derive(Debug, Clone)]
pub struct TeamCandidate {
    pub id: uuid::Uuid,
    pub content: String,
    pub importance: Importance,
}

/// One project result's rank within `memory_fts` — nothing else (D425, FR-471).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectRelevance {
    pub memory_id: uuid::Uuid,
    pub relevance: f64,
}

/// One personal result's rank within `personal_fts` — nothing else.
///
/// `PersonalRelevance`, `TeamRelevance` and [`ProjectRelevance`] each carry
/// only their own domain's score. A `4.1` here and a `4.1` there are not the
/// same claim — one is "well-matched against this user's own knowledge," the
/// other "well-matched against this project's" — and BM25's score is a
/// function of term statistics *within one corpus*, so there is no honest
/// conversion between them. Nothing normalizes one against the other, and
/// nothing could: no type exists in which a project score and a personal or
/// team score coexist, and no function accepts two of these three types at
/// once, so a cross-domain relevance comparison does not compile (D425,
/// FR-471, SC-468, `contracts/recall-composition.md` §8).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PersonalRelevance {
    pub knowledge_id: uuid::Uuid,
    pub relevance: f64,
}

/// One team result's rank within `team_fts` — nothing else. See
/// [`PersonalRelevance`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TeamRelevance {
    pub knowledge_id: uuid::Uuid,
    pub relevance: f64,
}

/// Admit `personal_notes` then `team_guidance`, the last two entries in
/// `SECTION_ORDER`, bounded by the two independent caps
/// `contracts/recall-composition.md` §3 describes (D421, D449, D450, FR-474,
/// FR-475, FR-584).
///
/// Callers must invoke this only after `release_reserve()` has already run
/// and every project-priority section has already spent what it is going to
/// spend — `Budget::remaining_non_reserve` only means what its name claims
/// once that is true. This function itself never spends via
/// `try_spend_reserved`; it draws exclusively on `try_spend`, the same call
/// every other Level 1 section uses, which is what makes "no personal or team
/// byte is ever admitted via the reserve" true regardless of when this runs
/// relative to `release_reserve` (D420, Invariant 2).
///
/// Personal is admitted before team, always (FR-476): the two share one
/// combined cap, and whichever runs out of room second is the one a tight
/// budget drops. Returns the admitted content as plain strings — the same
/// shape `decisions` and `known_failures` already render in, which is what
/// makes "the rendered form has no reason field" a property of the type
/// rather than a promise a renderer keeps (FR-478, D451): a `String` has
/// nowhere to put one. Every inclusion and exclusion is additionally recorded
/// on `sel` with a reason from the vocabulary project sections already use
/// (§4a) — `explain`-only, never reaching the returned content.
///
/// A future `depth: "minimum"` gate (FR-477, not this feature's task here —
/// see `contracts/recall-composition.md` §5) needs no change to this
/// signature: a caller that must exclude both sections unconditionally can
/// simply pass empty `personal`/`team` slices, which this function already
/// treats as "nothing to admit" with zero spend either way.
pub fn admit_global(
    budget: &mut Budget,
    sel: &mut Selection,
    personal: &[PersonalCandidate],
    team: &[TeamCandidate],
    global_share_max: f64,
) -> (Vec<String>, Vec<String>) {
    // Fixed the moment the budget is known; does not vary with how much any
    // other section spends (D421, Invariant 3).
    let global_cap = (budget.limit() as f64 * global_share_max).floor() as usize;
    // A snapshot, taken once, of the non-reserve pool *before either global
    // section has spent anything from it* — not a value re-read from `budget`
    // on every item.
    //
    // `Budget::remaining_non_reserve` is a live query: read again after this
    // function's own spending, it would report `general_remaining()` (which
    // keeps shrinking correctly) capped at `limit - reserve_initial` (which
    // does not shrink at all, since that quantity is fixed the moment the
    // budget is created). While `general_remaining()` still exceeds that
    // fixed cap — exactly the regime a large released reserve produces — a
    // second live read would not reflect what this function *itself* already
    // spent, and repeated admissions could walk straight past the ceiling
    // this function exists to enforce. Snapshotting once and decrementing by
    // an explicit running total keeps the ceiling honest across every item in
    // both sections.
    let non_reserve_ceiling = budget.remaining_non_reserve();
    let mut global_spent = 0usize;

    let personal_items: Vec<(uuid::Uuid, String)> =
        personal.iter().map(|c| (c.id, c.content.clone())).collect();
    let team_items: Vec<(uuid::Uuid, String)> =
        team.iter().map(|c| (c.id, c.content.clone())).collect();

    let notes = admit_global_section(
        budget,
        sel,
        "personal_notes",
        &personal_items,
        global_cap,
        non_reserve_ceiling,
        &mut global_spent,
    );
    let guidance = admit_global_section(
        budget,
        sel,
        "team_guidance",
        &team_items,
        global_cap,
        non_reserve_ceiling,
        &mut global_spent,
    );
    (notes, guidance)
}

/// One global section's admission loop. Stops at the first candidate that
/// does not fit rather than skipping ahead — the caller's order is a priority
/// order, exactly as `Budget::take_while_fits` already documents for every
/// other section.
fn admit_global_section(
    budget: &mut Budget,
    sel: &mut Selection,
    kind: &str,
    items: &[(uuid::Uuid, String)],
    global_cap: usize,
    non_reserve_ceiling: usize,
    global_spent: &mut usize,
) -> Vec<String> {
    let mut kept = Vec::new();
    for (id, content) in items {
        let cost = estimate(content) + 1;
        // The two independent limits (D421, FR-474): the fraction of the
        // whole budget not yet spent by global sections, and the pool that
        // excludes whatever `release_reserve()` returned — never
        // `general_remaining()` directly, which is exactly the D449 defect.
        // Both are measured against `global_spent`, this function's own
        // running total, not against a fresh `Budget` query (see the
        // snapshot note in `admit_global`).
        let cap_left = global_cap.saturating_sub(*global_spent);
        let pool_left = non_reserve_ceiling.saturating_sub(*global_spent);
        let allowance = cap_left.min(pool_left);
        if cost > allowance {
            note_omission(
                sel,
                kind,
                items.len() - kept.len(),
                // Whichever term was smaller is the one that actually bound —
                // stating one without the other is not enough (§3).
                if cap_left < pool_left {
                    OmissionReason::CapReached
                } else {
                    OmissionReason::BudgetExhausted
                },
                "cairn recall --domain personal|team",
            );
            return kept;
        }
        // `cost <= allowance <= pool_left <= general_remaining()`, so this
        // cannot fail — `try_spend` (never `try_spend_reserved`) is the call
        // that keeps this section off the reserve unconditionally.
        let spent = budget.try_spend(cost);
        debug_assert!(spent, "allowance already bounds cost by general_remaining");
        *global_spent += cost;
        kept.push(content.clone());
        sel.included.push(SelectedItem {
            level: ContextLevel::Relevant,
            kind: kind.to_string(),
            id: id.to_string(),
            // Reusing the vocabulary project sections already use (§4a):
            // admitted for the same reason project content is — it matched
            // the requester's scope, here "their own account" or "their
            // team" rather than "this project."
            reasons: vec![SelectionReason::ScopeMatch],
            cost,
        });
    }
    kept
}

// ---------------------------------------------------------------------------
// Tier 0a — the guaranteed work state
// ---------------------------------------------------------------------------

/// Admit the O(1) work state: repository, then the task's identity, bounded
/// goal, status, derived counts, readiness and the single most actionable
/// blocker, then the warning **kinds** with counts.
///
/// Returns false only when the task itself did not fit, which at any budget at
/// or above the documented minimum cannot happen — the tier's worst case is
/// bounded precisely so that it cannot.
fn admit_tier_0a(
    b: &mut Briefing,
    input: &ContextInputs<'_>,
    budget: &mut Budget,
    sel: &mut Selection,
) -> bool {
    // Repository state — fixed shape, Tier 0a item 7.
    let r = &input.repository;
    let repo_cost = estimate(&r.branch) + estimate(r.commit_sha.as_deref().unwrap_or("")) + 12;
    if budget.try_spend_reserved(repo_cost) {
        b.repository = r.clone();
        sel.included.push(SelectedItem {
            level: ContextLevel::MinimumSafe,
            kind: "repository".into(),
            id: r.branch.clone(),
            reasons: vec![SelectionReason::ScopeMatch],
            cost: repo_cost,
        });
    }

    // The warning kinds, with counts. Detail is Tier 0b; the *fact* that
    // something is wrong is guaranteed.
    if !input.level0.warnings.is_empty() {
        let kinds = warning_kind_counts(input.level0.warnings);
        let cost = estimate(&kinds) + 1;
        if budget.try_spend_reserved(cost) {
            b.warnings.push(ContextWarning {
                kind: "summary".into(),
                subject: kinds,
                detail: String::new(),
            });
            sel.included.push(SelectedItem {
                level: ContextLevel::MinimumSafe,
                kind: "warning".into(),
                id: "summary".into(),
                reasons: vec![SelectionReason::DriftWarning],
                cost,
            });
        }
    }

    // A diverged checkpoint's recorded action, always labelled as previous.
    if let Some(previous) = input.level0.previous_next_action {
        let cost = estimate(previous) + 2;
        if budget.try_spend_reserved(cost) {
            b.previous_next_action = Some(previous.to_string());
            sel.included.push(SelectedItem {
                level: ContextLevel::MinimumSafe,
                kind: "previous_next_action".into(),
                id: String::new(),
                reasons: vec![SelectionReason::CheckpointAssumption],
                cost,
            });
        }
    }

    let Some(t) = input.task else { return true };

    // The goal is bounded rather than dropped: the tier stays O(1) by
    // truncating, never by omitting (FR-443).
    let (goal, goal_truncated) = truncate_to_tokens(&t.goal, input.level0.caps.goal_max_tokens);
    let progress = tasks::progress(input.level0.criteria);
    let readiness = tasks::completion_readiness(input.level0.criteria, input.level0.blockers);
    let open_blockers = input
        .level0
        .blockers
        .iter()
        .filter(|x| !x.deleted && x.state == BlockerState::Open)
        .count();
    let blocker = most_actionable_blocker(input);

    // A fixed shape, so the cost does not grow with the project or the task.
    let cost =
        estimate(&t.title) + estimate(&goal) + estimate(blocker.as_deref().unwrap_or("")) + 24;
    if !budget.try_spend_reserved(cost) {
        return false;
    }

    b.task = Some(BriefingTask {
        id: t.id,
        title: t.title.clone(),
        goal,
        // Feature 001's array stays exactly as it was for its five readers; the
        // Level 0 rendering uses `criteria` below.
        acceptance_criteria: t.acceptance_criteria.clone(),
        status: t.status,
        progress: Some(progress),
        completion_readiness: Some(readiness),
        open_blockers: Some(open_blockers),
        blocker,
        goal_truncated,
        criteria: Vec::new(),
        criteria_omitted: None,
    });
    sel.included.push(SelectedItem {
        level: ContextLevel::MinimumSafe,
        kind: "task".into(),
        id: t.id.to_string(),
        reasons: vec![SelectionReason::TaskBinding],
        cost,
    });
    true
}

// ---------------------------------------------------------------------------
// Tier 0b — bounded detail
// ---------------------------------------------------------------------------

/// Warning detail, then pins, then criterion text in action order, then further
/// blockers — each until its cap or the budget binds (FR-444, FR-446, FR-448).
fn admit_tier_0b(
    b: &mut Briefing,
    input: &ContextInputs<'_>,
    budget: &mut Budget,
    sel: &mut Selection,
) {
    let caps = input.level0.caps;

    // 8 — warning detail, highest precedence first.
    let mut ordered: Vec<&ContextWarning> = input.level0.warnings.iter().collect();
    ordered.sort_by_key(|w| warning_precedence(&w.kind));
    let mut warnings_shown = 0usize;
    for w in ordered.iter().take(caps.warnings_in_context_max) {
        let cost = estimate(&w.subject) + estimate(&w.detail) + 2;
        if !budget.try_spend_reserved(cost) {
            break;
        }
        b.warnings.push((*w).clone());
        warnings_shown += 1;
        sel.included.push(SelectedItem {
            level: ContextLevel::MinimumSafe,
            kind: "warning".into(),
            id: format!("{}:{}", w.kind, w.subject),
            reasons: vec![warning_reason(&w.kind)],
            cost,
        });
    }
    note_omission(
        sel,
        "warning",
        input.level0.warnings.len().saturating_sub(warnings_shown),
        if warnings_shown >= caps.warnings_in_context_max {
            OmissionReason::CapReached
        } else {
            OmissionReason::BudgetExhausted
        },
        "",
    );

    // 9 — pinned constraints in force.
    let mut pins_shown = 0usize;
    for p in input.level0.pins.iter().take(caps.pins_in_context_max) {
        let cost = estimate(&p.text) + 2;
        if !budget.try_spend_reserved(cost) {
            break;
        }
        b.constraints.push(p.clone());
        pins_shown += 1;
        sel.included.push(SelectedItem {
            level: ContextLevel::MinimumSafe,
            kind: "constraint".into(),
            id: p.id.to_string(),
            reasons: vec![SelectionReason::Pinned],
            cost,
        });
    }
    note_omission(
        sel,
        "constraint",
        input.level0.pins.len().saturating_sub(pins_shown),
        if pins_shown >= caps.pins_in_context_max {
            OmissionReason::PinBudget
        } else {
            OmissionReason::BudgetExhausted
        },
        "",
    );

    // 10 — criterion text, in action order: blocked, then satisfied but
    // unverified, then pending, then verified, then waived. The ones an agent
    // must act on arrive first.
    let ordered = tasks::action_order(input.level0.criteria);
    let total = ordered.len();
    let mut shown = Vec::new();
    for c in ordered {
        let label = tasks::criterion_label(c.ordinal);
        let cost = estimate(&label) + estimate(&c.text) + 3;
        if !budget.try_spend_reserved(cost) {
            break;
        }
        sel.included.push(SelectedItem {
            level: ContextLevel::MinimumSafe,
            kind: "criterion".into(),
            id: c.id.to_string(),
            reasons: vec![SelectionReason::TaskBinding],
            cost,
        });
        shown.push(BriefingCriterion {
            label,
            text: c.text.clone(),
            state: c.state,
            verification: c.verification,
        });
    }
    let dropped = total.saturating_sub(shown.len());
    if let Some(task) = b.task.as_mut() {
        task.criteria = shown;
        task.criteria_omitted = (dropped > 0).then_some(dropped);
    }
    note_omission(
        sel,
        "criterion",
        dropped,
        OmissionReason::BudgetExhausted,
        "cairn task get <id>",
    );
}

/// `⚠ 1 conflict · 1 drift · checkpoint diverged` — the kinds and their counts,
/// which is what Tier 0a guarantees whatever the budget.
fn warning_kind_counts(warnings: &[ContextWarning]) -> String {
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for w in warnings {
        *counts.entry(w.kind.as_str()).or_default() += 1;
    }
    let parts: Vec<String> = counts
        .iter()
        // "2 conflict" reads as a truncated string rather than a count. The
        // kinds are single English nouns, so the plural is the suffix.
        .map(|(kind, n)| {
            if *n == 1 {
                format!("{n} {kind}")
            } else {
                format!("{n} {kind}s")
            }
        })
        .collect();
    parts.join(" · ")
}

/// divergence → task → conflict → drift (`contracts/continuity-context.md`).
fn warning_precedence(kind: &str) -> usize {
    match kind {
        "task_divergence" | "checkpoint" => 0,
        "task" => 1,
        "conflict" => 2,
        "drift" => 3,
        _ => 4,
    }
}

fn warning_reason(kind: &str) -> SelectionReason {
    match kind {
        "conflict" => SelectionReason::ConflictWarning,
        "task_divergence" | "checkpoint" => SelectionReason::CheckpointAssumption,
        "task" => SelectionReason::TaskBinding,
        _ => SelectionReason::DriftWarning,
    }
}

fn note_omission(
    sel: &mut Selection,
    kind: &str,
    count: usize,
    reason: OmissionReason,
    retrieval: &str,
) {
    if count == 0 {
        return;
    }
    sel.omitted.push(OmittedItem {
        kind: kind.to_string(),
        count,
        reason,
        retrieval: retrieval.to_string(),
    });
}

/// The blocker an agent should act on: the oldest still open.
///
/// One bounded line. Which one is "most actionable" is not a judgement Cairn can
/// make from a description, so it uses the one that has been in force longest —
/// deterministic, and explainable.
fn most_actionable_blocker(input: &ContextInputs<'_>) -> Option<String> {
    let open = input
        .level0
        .blockers
        .iter()
        .find(|b| !b.deleted && b.state == BlockerState::Open)?;
    let text = input
        .level0
        .blocker_text
        .iter()
        .find(|(id, _)| *id == open.id)
        .map(|(_, t)| t.as_str())
        .unwrap_or("");
    let (bounded, _) = truncate_to_tokens(text, 24);
    Some(bounded)
}

/// Truncate to at most `max_tokens` estimated tokens, on a character boundary.
///
/// Returns whether anything was dropped, so the briefing can say so rather than
/// presenting a cut sentence as the whole goal.
fn truncate_to_tokens(text: &str, max_tokens: usize) -> (String, bool) {
    if estimate(text) <= max_tokens {
        return (text.to_string(), false);
    }
    let max_chars = (max_tokens as f64 * crate::budget::CHARS_PER_TOKEN) as usize;
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    (out, true)
}

fn admit_handoff(b: &mut Briefing, input: &ContextInputs<'_>, budget: &mut Budget) -> bool {
    let Some(h) = input.previous_handoff else {
        return true;
    };

    // Remaining work and next step first: they are the part a successor
    // session actually acts on (D8).
    let head_cost = estimate(&h.next_step) + 4;
    if !budget.try_spend(head_cost) {
        return false;
    }
    let remaining = budget.take_while_fits(h.remaining_work.iter().cloned(), |s| estimate(s) + 1);
    let complete_remaining = remaining.len() == h.remaining_work.len();
    let changed = budget.take_while_fits(h.changed_files.iter().cloned(), |s| estimate(s) + 1);
    let complete_changed = changed.len() == h.changed_files.len();

    b.previous_handoff = Some(BriefingHandoff {
        session_id: h.session_id,
        next_step: h.next_step.clone(),
        remaining_work: remaining,
        changed_files: changed,
    });
    complete_remaining && complete_changed
}

/// True when nothing in `omitted` is a high-priority section (SC-003).
pub fn kept_all_high_priority(omitted: &[String]) -> bool {
    !omitted
        .iter()
        .any(|s| HIGH_PRIORITY_SECTIONS.contains(&s.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn project() -> Project {
        Project {
            id: new_id(),
            name: "cairn".into(),
            git_common_dir: "/tmp/repo/.git".into(),
            repository_remote: None,
            linked: false,
            server_project_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        }
    }

    fn inputs<'a>(p: &'a Project, mem: &'a [String]) -> ContextInputs<'a> {
        ContextInputs {
            project: p,
            repository: RepositoryState {
                branch: "main".into(),
                commit_sha: Some("abc1234".into()),
                staged: 1,
                unstaged: 2,
                untracked: 0,
            },
            task: None,
            previous_handoff: None,
            decisions: &[],
            known_failures: &[],
            task_memory: &[],
            branch_memory: &[],
            project_memory: mem,
            patterns: &[],
            has_history: true,
            degraded: false,
            level0: Level0::default(),
            personal_notes: &[],
            team_guidance: &[],
        }
    }

    #[test]
    fn never_exceeds_the_budget_even_with_far_more_memory_than_fits() {
        let p = project();
        let mem: Vec<String> = (0..5000)
            .map(|i| format!("project memory item number {i} with some prose"))
            .collect();
        for budget in [200usize, 800, 3000, 4000] {
            let out = assemble(&inputs(&p, &mem), budget);
            assert!(
                out.estimated_tokens <= budget,
                "spent {} over budget {budget}",
                out.estimated_tokens
            );
            assert!(out.truncated);
            assert!(out.omitted_sections.contains(&"project_memory".to_string()));
        }
    }

    #[test]
    fn reports_no_prior_history_and_still_succeeds() {
        let p = project();
        let mut i = inputs(&p, &[]);
        i.has_history = false;
        let out = assemble(&i, 3000);
        assert!(out.briefing.no_prior_history);
        assert!(!out.truncated);
    }

    #[test]
    fn repository_state_survives_a_tight_budget_before_memory_does() {
        let p = project();
        let mem: Vec<String> = (0..200).map(|i| format!("memory {i}")).collect();
        let out = assemble(&inputs(&p, &mem), 60);
        assert_eq!(out.briefing.repository.branch, "main");
        assert!(out.omitted_sections.contains(&"project_memory".to_string()));
        assert!(kept_all_high_priority(&out.omitted_sections));
    }

    #[test]
    fn task_goal_and_criteria_lead_the_briefing() {
        let p = project();
        let task = Task {
            id: new_id(),
            project_id: p.id,
            title: "Rate limiting".into(),
            goal: "Requests over the limit get 429".into(),
            acceptance_criteria: vec!["429 above threshold".into()],
            status: TaskStatus::InProgress,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        let mut i = inputs(&p, &[]);
        i.task = Some(&task);
        let out = assemble(&i, 3000);
        let bt = out.briefing.task.expect("task section");
        assert_eq!(bt.goal, "Requests over the limit get 429");
        assert_eq!(bt.acceptance_criteria.len(), 1);
    }

    #[test]
    fn previous_handoff_next_step_and_remaining_work_are_carried() {
        let p = project();
        let h = Handoff {
            id: new_id(),
            session_id: new_id(),
            trigger: HandoffTrigger::SessionEnd,
            goal: "g".into(),
            progress: "p".into(),
            completed_work: vec![],
            remaining_work: vec!["Open failure: cargo test".into()],
            changed_files: vec!["src/lib.rs".into()],
            decisions: vec![],
            failures: vec![],
            tests_executed: vec![],
            repository_state: RepositoryState::default(),
            next_step: "Fix the open failure".into(),
            agent_note: None,
            evidence: vec![],
            created_at: Utc::now(),
            deleted_at: None,
        };
        let mut i = inputs(&p, &[]);
        i.previous_handoff = Some(&h);
        let out = assemble(&i, 3000);
        let ph = out.briefing.previous_handoff.expect("handoff section");
        assert_eq!(ph.next_step, "Fix the open failure");
        assert_eq!(ph.remaining_work.len(), 1);
        assert_eq!(ph.changed_files, vec!["src/lib.rs".to_string()]);
    }

    #[test]
    fn degraded_flag_passes_through() {
        let p = project();
        let mut i = inputs(&p, &[]);
        i.degraded = true;
        assert!(assemble(&i, 3000).degraded);
    }

    // -- `admit_global`, Phase 8 (`contracts/recall-composition.md`) -------

    fn personal(content: &str) -> PersonalCandidate {
        PersonalCandidate {
            id: new_id(),
            content: content.to_string(),
            importance: Importance::Normal,
        }
    }

    fn team(content: &str) -> TeamCandidate {
        TeamCandidate {
            id: new_id(),
            content: content.to_string(),
            importance: Importance::Normal,
        }
    }

    #[test]
    fn global_spend_is_zero_when_project_sections_consume_the_entire_pool() {
        // Example C, `contracts/recall-composition.md` §6: real global
        // content is present and would otherwise be admitted, but the pool it
        // would draw from is empty before either domain is even considered
        // (FR-475, SC-418).
        let mut b = Budget::with_reserve(3000, 1200);
        assert!(b.try_spend_reserved(1200));
        b.release_reserve();
        assert!(
            b.try_spend(1800),
            "project sections spend the entire remainder"
        );
        assert_eq!(b.general_remaining(), 0);

        let mut sel = Selection::default();
        let personal = vec![personal("a durable personal note")];
        let team_items = vec![team("a ratified team default")];
        let (notes, guidance) =
            admit_global(&mut b, &mut sel, &personal, &team_items, GLOBAL_SHARE_MAX);

        assert!(notes.is_empty());
        assert!(guidance.is_empty());
        assert_eq!(b.spent(), 3000, "not one global byte was charged");
    }

    #[test]
    fn global_spend_excludes_released_reserve_d449() {
        // The defect this feature exists to prevent: a large, mostly unspent
        // reserve is released, inflating `general_remaining()`, but global
        // sections must be bounded by the non-reserve pool alone (D449,
        // FR-584, SC-451).
        let mut b = Budget::with_reserve(1000, 900);
        assert!(b.try_spend_reserved(10));
        b.release_reserve();
        assert_eq!(b.general_remaining(), 990);
        assert_eq!(b.remaining_non_reserve(), 100);

        let mut sel = Selection::default();
        // Sized to fit the (buggy) 990 general_remaining and the 150 cap, but
        // not the correct 100-token non-reserve pool.
        let big = "x".repeat(400); // ~115 estimated tokens
        let personal = vec![personal(&big)];
        let (notes, _) = admit_global(&mut b, &mut sel, &personal, &[], GLOBAL_SHARE_MAX);

        assert!(
            notes.is_empty(),
            "a defect that read general_remaining() would have admitted this"
        );
        assert_eq!(b.spent(), 10, "global spent nothing beyond Level 0");
    }

    #[test]
    fn the_fraction_binds_when_the_pool_is_roomy() {
        // Example A, §6: both sections fit — bounded by neither term.
        let mut b = Budget::with_reserve(3000, 1200);
        assert!(b.try_spend_reserved(350));
        b.release_reserve();
        assert!(b.try_spend(2000));
        assert_eq!(b.remaining_non_reserve(), 650);

        let mut sel = Selection::default();
        let personal = vec![personal(&"p".repeat(690))]; // ~200 tokens
        let team_items = vec![team(&"t".repeat(345))]; // ~100 tokens
        let (notes, guidance) =
            admit_global(&mut b, &mut sel, &personal, &team_items, GLOBAL_SHARE_MAX);

        assert_eq!(notes.len(), 1, "personal fits under the 450 cap");
        assert_eq!(guidance.len(), 1, "team fits in the cap's remainder");
        assert!(b.spent() <= 3000);
    }

    #[test]
    fn the_pool_binds_when_the_fraction_does_not() {
        // The sharper differentiator: the pool the never-reserved fraction of
        // the budget provides is *smaller* than the 0.15 cap, so it must bind
        // even though `general_remaining()` looks ample thanks to a large
        // released reserve sitting in it. A snapshot taken once and
        // decremented by this function's own running spend (not re-read from
        // `Budget` per item) is what keeps a second candidate from spending
        // past the 100-token ceiling the first one left almost none of.
        let mut b = Budget::with_reserve(1000, 900);
        assert!(b.try_spend_reserved(10));
        b.release_reserve();
        // global_cap = floor(1000 * 0.15) = 150; remaining_non_reserve = 100.
        assert_eq!(b.remaining_non_reserve(), 100);
        assert!(
            b.general_remaining() > 900,
            "the released reserve dwarfs the true pool"
        );

        let mut sel = Selection::default();
        let ninety = "p".repeat(315); // 90 estimated tokens
        let item_cost = estimate(&ninety) + 1;
        assert!(
            item_cost < 100,
            "each item alone must fit the 100-token pool"
        );
        assert!(item_cost * 2 > 100, "but two of them together must not");
        let personal = vec![personal(&ninety), personal(&ninety)];
        let (notes, _) = admit_global(&mut b, &mut sel, &personal, &[], GLOBAL_SHARE_MAX);

        assert_eq!(
            notes.len(),
            1,
            "the second item must not spend the 100-token pool a second time over"
        );
        assert_eq!(b.spent(), 10 + item_cost);
    }

    #[test]
    fn the_cap_binds_before_the_pool_does() {
        // Example D, §6: 2000 tokens of general pool remain unspent, and team
        // guidance is still truncated, because the 0.15-of-budget ceiling is
        // a property of the budget, not of how generous the remainder is.
        let mut b = Budget::with_reserve(3000, 1200);
        assert!(b.try_spend_reserved(200));
        b.release_reserve();
        assert!(b.try_spend(500));
        assert_eq!(b.remaining_non_reserve(), 1800);

        let mut sel = Selection::default();
        let personal = vec![personal(&"p".repeat(1035))]; // ~300 tokens
        let team_items = vec![team(&"t".repeat(1380))]; // ~400 tokens, wants more than the cap has left
        let (notes, guidance) =
            admit_global(&mut b, &mut sel, &personal, &team_items, GLOBAL_SHARE_MAX);

        assert_eq!(notes.len(), 1, "personal's 300 fits under the 450 cap");
        assert!(
            guidance.is_empty(),
            "team's 400 does not fit the cap's 150 remainder"
        );
        assert!(
            b.general_remaining() > 1500,
            "ample general pool remained — the cap bound independently of it"
        );
        let omission = sel
            .omitted
            .iter()
            .find(|o| o.kind == "team_guidance")
            .expect("team_guidance was omitted");
        assert_eq!(omission.reason, OmissionReason::CapReached);
    }

    #[test]
    fn personal_is_admitted_before_team_when_only_one_fits() {
        // FR-476, SC-462: the boundary that actually distinguishes the two
        // orderings — only enough combined room for one.
        let mut b = Budget::with_reserve(1000, 0);
        b.release_reserve();
        // global_cap = floor(1000 * 0.15) = 150.
        let content = "w".repeat(345); // ~100 tokens each
        let personal = vec![personal(&content)];
        let team_items = vec![team(&content)];

        let mut sel = Selection::default();
        let (notes, guidance) =
            admit_global(&mut b, &mut sel, &personal, &team_items, GLOBAL_SHARE_MAX);

        assert_eq!(notes.len(), 1, "personal is admitted");
        assert!(guidance.is_empty(), "team loses the tiebreak, not personal");
    }

    #[test]
    fn importance_hint_changes_nothing() {
        // FR-482, SC-464: every supported hint value leaves the assembled
        // output byte-identical. Nothing in `admit_global` reads
        // `importance`, so this is really asserting that fact rather than
        // discovering it.
        let mut baseline: Option<(Vec<String>, Vec<String>)> = None;
        for importance in [Importance::High, Importance::Normal, Importance::Low] {
            let mut b = Budget::with_reserve(3000, 1200);
            assert!(b.try_spend_reserved(350));
            b.release_reserve();
            assert!(b.try_spend(2000));
            let mut sel = Selection::default();
            let personal = vec![PersonalCandidate {
                id: new_id(),
                content: "identical content".into(),
                importance,
            }];
            let team_items = vec![TeamCandidate {
                id: new_id(),
                content: "identical guidance".into(),
                importance,
            }];
            let out = admit_global(&mut b, &mut sel, &personal, &team_items, GLOBAL_SHARE_MAX);
            match &baseline {
                None => baseline = Some(out),
                Some(expected) => assert_eq!(
                    &out, expected,
                    "importance {importance:?} changed the assembled output"
                ),
            }
        }
    }

    #[test]
    fn estimated_tokens_never_exceeds_budget_across_a_matrix() {
        // FR-480: the existing invariant, re-verified with both new terms in
        // play, across the documented minimum and default budgets and a
        // spread in between.
        for budget_tokens in [600usize, 900, 1500, 3000, 5000] {
            let reserve = (budget_tokens as f64 * 0.40).floor() as usize;
            let mut b = Budget::with_reserve(budget_tokens, reserve);
            // Simulate Level 0 taking a modest, varying share of the reserve.
            let _ = b.try_spend_reserved(reserve / 4);
            b.release_reserve();
            // Simulate project sections spending most, but not all, of what
            // remains.
            let _ = b.try_spend(b.general_remaining() * 3 / 4);

            let mut sel = Selection::default();
            let personal: Vec<PersonalCandidate> = (0..20)
                .map(|i| personal(&format!("personal note number {i} with some prose in it")))
                .collect();
            let team_items: Vec<TeamCandidate> = (0..20)
                .map(|i| team(&format!("team guidance number {i} with some prose in it")))
                .collect();
            admit_global(&mut b, &mut sel, &personal, &team_items, GLOBAL_SHARE_MAX);

            assert!(
                b.spent() <= budget_tokens,
                "budget {budget_tokens}: spent {} over limit",
                b.spent()
            );
        }
    }

    #[test]
    fn rendered_global_items_carry_no_reason_field() {
        // FR-478, D451: a reason is produced on the diagnostic path (`sel`)
        // and the rendered form (`Vec<String>`) has nowhere to hold one —
        // inspected field by field, which for a bare `String` is exhaustive.
        let mut b = Budget::with_reserve(1000, 0);
        b.release_reserve();
        let personal = vec![personal("keep this exact text")];
        let mut sel = Selection::default();
        let (notes, _) = admit_global(&mut b, &mut sel, &personal, &[], GLOBAL_SHARE_MAX);

        assert_eq!(notes, vec!["keep this exact text".to_string()]);
        assert_eq!(sel.included.len(), 1);
        assert_eq!(sel.included[0].reasons, vec![SelectionReason::ScopeMatch]);
    }

    #[test]
    fn global_never_draws_from_an_unreleased_reserve() {
        // Invariant 2: personal and team sections call only `try_spend`, so
        // this must hold even if invoked before `release_reserve` runs — a
        // defensive property of the function, not merely of call order.
        let mut b = Budget::with_reserve(1000, 900);
        // Deliberately no `release_reserve()` call.
        let mut sel = Selection::default();
        let personal = vec![personal(&"p".repeat(170))]; // ~49 tokens, fits the 100 non-reserve pool
        let (notes, _) = admit_global(&mut b, &mut sel, &personal, &[], GLOBAL_SHARE_MAX);

        assert_eq!(notes.len(), 1);
        assert_eq!(
            b.reserve_used(),
            0,
            "spend went through try_spend, never try_spend_reserved"
        );
        assert_eq!(b.remaining(), 1000 - (estimate(&"p".repeat(170)) + 1));
    }
}
