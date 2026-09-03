//! Server-side retrieval, its trace, and the delivery outcome (T069, T070;
//! `contracts/retrieval-delivery.md`).
//!
//! # Why selection moved here
//!
//! Dedup is `relevant MINUS delivered PLUS changed`, and only the server holds
//! both sides of that comparison: a request carries a session, not the set of
//! what that session already received. A client-side `delivered_context` made
//! the rule uncomputable, and every prompt-time delivery would have restated
//! the session-open briefing.
//!
//! # The lifecycle is the point
//!
//! `requested → generated → transmitted | failed`, and each arrow is written by
//! a different party at a different moment. The server writes `requested`
//! before it selects anything, so a retrieval that dies mid-generation leaves a
//! record rather than nothing. Generation writes `generated` or a
//! generation-stage `failed`. **Only the daemon's later authenticated report
//! can write `transmitted`** — generating a briefing is not evidence that an
//! agent received one, and the two states exist precisely so that Cairn cannot
//! quietly claim the second on the strength of the first (FR-843, FR-854).
//!
//! `delivered_context` follows the same rule: a row is written when
//! transmission is reported and did not fail, never at selection. Writing it at
//! selection would suppress, for the life of the session, items the agent never
//! saw — dedup enforcing a delivery that did not happen.
//!
//! # What a trace may say
//!
//! Identities, budget accounting and a degradation level. Never the rendered
//! briefing: the text mixes domains and carries handoff-derived material, so
//! persisting it centrally would put one account's personal knowledge inside a
//! project-scoped record (FR-839).

use crate::auth::{self, ReaderContext, SessionBindingError, Visibility};
use crate::error::{ApiError, ApiResult};
use axum::http::StatusCode;
use cairn_core::budget::{estimate, Budget};
use cairn_core::domain::{KnowledgeDomain, KnowledgeRef, PatternRef, Reference};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::collections::BTreeMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Bounds and vocabulary
// ---------------------------------------------------------------------------

/// The prompt-time budget, as a fraction of the session-open one.
///
/// The two points cannot restate each other (FR-829, FR-830), and the smaller
/// number is the one that sits inside the model's turn.
pub const INCREMENTAL_FRACTION: f64 = 0.25;

/// The share of the budget Level 1 and Level 2 may not take from project truth.
pub const RESERVE_FRACTION: f64 = 0.40;

/// How long a trace and its items are kept (FR-847).
pub const TRACE_RETENTION_DAYS: i64 = 90;

/// How many traces one sweep deletes, so retention never becomes a long lock.
pub const SWEEP_BATCH: i64 = 500;

/// Soft targets, inside the hook's own `context_deadline_ms`.
///
/// Missing one degrades a level; missing the hook deadline is what the hook
/// itself enforces, and the agent proceeds regardless. This module introduces
/// no new deadline constant, because a second number would drift against the
/// one that is actually enforced.
pub const SOFT_TARGET_SESSION_OPEN_MS: u128 = 250;
pub const SOFT_TARGET_PROMPT_TIME_MS: u128 = 100;

/// How many candidates one domain contributes before budget arithmetic runs.
///
/// A bound rather than a page: selection is deterministic, so an unbounded read
/// would make its cost grow with the project's history for no gain — every item
/// past the budget is discarded anyway.
const CANDIDATES_PER_SECTION: i64 = 24;

/// The four levels, and no fifth (FR-836).
pub const LEVEL_FULL: &str = "full";
pub const LEVEL_REDUCED: &str = "reduced";
pub const LEVEL_MINIMAL: &str = "minimal";
pub const LEVEL_NONE: &str = "none";

/// The durable sections, in `SECTION_ORDER`.
///
/// Only stable-reference sections participate in dedup and in delivery
/// (`contracts/retrieval-delivery.md` §4); `task`, `repository`, `decisions`
/// and the rest are re-derived fresh every delivery and are the daemon's.
pub const DURABLE_SECTIONS: &[&str] = &[
    "task_memory",
    "branch_memory",
    "project_memory",
    "patterns",
    "personal_notes",
    "team_guidance",
];

/// Why an item was admitted, in terms a reviewer can reproduce (SC-711).
const RULE_PROJECT_RESERVE: &str = "project_reserve";
const RULE_GENERAL_POOL: &str = "general_pool";
const RULE_GLOBAL_SHARE: &str = "global_share";

/// Why an item was considered and not selected.
///
/// There is deliberately no `not_visible_to_reader` here. Every owner-scoped
/// read below is written as `owner_user_id = $reader`, so a record the caller
/// may not see never becomes a candidate and never reaches a rule — the filter
/// is the query, not a step after it. A rule for it would imply such rows are
/// gathered and then dropped, which is the arrangement one refactor away from
/// not dropping them.
const RULE_ALREADY_DELIVERED: &str = "already_delivered";
const RULE_BUDGET_EXHAUSTED: &str = "budget_exhausted";

// ---------------------------------------------------------------------------
// The request and its answer
// ---------------------------------------------------------------------------

/// Why retrieval ran.
///
/// `explicit` is what `cairn_context`/`cairn_search` produce: no push into the
/// agent's context stream, and exempt from dedup, because an explicit call is a
/// request to be told again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    SessionOpen,
    PromptSubmit,
    Explicit,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Trigger::SessionOpen => "session_open",
            Trigger::PromptSubmit => "prompt_submit",
            Trigger::Explicit => "explicit",
        }
    }

    /// Where the briefing is aimed, which is not the same question as why it
    /// ran: an explicit call is answered, never pushed.
    pub fn delivery_point(self) -> &'static str {
        match self {
            Trigger::SessionOpen => "session_open",
            Trigger::PromptSubmit => "prompt_time",
            Trigger::Explicit => "explicit",
        }
    }

    /// Whether this point restates what the session already has.
    fn dedups(self) -> bool {
        !matches!(self, Trigger::Explicit)
    }

    fn soft_target_ms(self) -> u128 {
        match self {
            Trigger::PromptSubmit => SOFT_TARGET_PROMPT_TIME_MS,
            _ => SOFT_TARGET_SESSION_OPEN_MS,
        }
    }
}

/// What a caller may ask for, and the whole of it.
///
/// No `project_id`, no `account_id`, no budget, no authority: the project and
/// the account are derived from the session and the credential, and a caller
/// that could name them could retrieve against a project it has nothing to do
/// with (FR-769, Principle XI).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrieveRequest {
    pub session_id: Uuid,
    pub trigger: Trigger,
    /// A **smaller** budget than the deployment's, where the caller wants one.
    ///
    /// Not authority: a caller may ask for less and never for more, and the
    /// server clamps it, so the worst a hostile value can do is starve the
    /// caller's own briefing. It exists because the budget is a property of the
    /// machine asking — `cairn context --budget` and a per-machine
    /// `context_budget_tokens` are both real — and a server that ignored it
    /// would hand back more than the caller can spend, which is how a briefing
    /// stops being within its stated budget (FR-029, SC-709).
    #[serde(default)]
    pub budget_tokens: Option<usize>,
    /// `session_open` only. `compact` is how post-compaction restoration is
    /// reached — there is no post-compaction delivery point of its own,
    /// because at least one committed vendor's post-compaction event cannot
    /// carry returned context at all (FR-838d).
    #[serde(default)]
    pub open_trigger: Option<String>,
}

/// One admitted item, with everything a reviewer needs to reproduce it.
///
/// SC-711 asks for the rule that admitted an item **and the budget that
/// remained at that point**, sufficient to redo the selection by hand. Both
/// travel per item rather than as a summary, because a total cannot be
/// unwound into the sequence that produced it.
#[derive(Debug, Clone, Serialize)]
pub struct SectionItem {
    pub reference_key: String,
    pub ref_kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<&'static str>,
    pub knowledge_id: Uuid,
    pub content: String,
    pub selection_rule: &'static str,
    pub rank: i32,
    pub cost: usize,
    /// What was left after this item was admitted.
    pub budget_remaining: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetrieveResponse {
    pub trace_id: Uuid,
    pub trigger: &'static str,
    pub delivery_point: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_trigger: Option<&'static str>,
    /// Whether this session-open is the post-compaction restoration point.
    pub restored_after_compaction: bool,
    pub degradation_level: &'static str,
    pub budget: BudgetReport,
    /// Always false here. The cache is the daemon's, and only the daemon can
    /// know it answered from one — a server that said so would be reporting a
    /// fact it does not have (FR-837).
    pub served_from_cache: bool,
    pub sections: BTreeMap<String, Vec<SectionItem>>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct BudgetReport {
    /// The whole briefing budget for this delivery point.
    pub tokens: usize,
    /// What the durable sections took of it.
    pub spent: usize,
    /// What is guaranteed to the Level 0 the daemon assembles.
    pub reserved_for_level0: usize,
}

// ---------------------------------------------------------------------------
// Candidates
// ---------------------------------------------------------------------------

/// One durable record retrieval may offer, before any budget is applied.
struct Candidate {
    reference: Reference,
    section: &'static str,
    content: String,
    source_updated_at: DateTime<Utc>,
}

impl Candidate {
    fn key(&self) -> String {
        self.reference.reference_key()
    }

    fn ref_kind(&self) -> &'static str {
        match self.reference {
            Reference::Knowledge(_) => "knowledge",
            Reference::Pattern(_) => "pattern",
        }
    }

    fn domain(&self) -> Option<&'static str> {
        self.reference.domain_slot().map(domain_str)
    }
}

fn domain_str(domain: KnowledgeDomain) -> &'static str {
    match domain {
        KnowledgeDomain::Project => "project",
        KnowledgeDomain::Personal => "personal",
        KnowledgeDomain::Team => "team",
    }
}

// ---------------------------------------------------------------------------
// Retrieval
// ---------------------------------------------------------------------------

/// Retrieve, trace it, and answer.
///
/// The trace row exists before selection begins, so a retrieval that fails
/// half-way is recorded rather than absent (FR-848). A non-member is refused
/// rather than handed an empty briefing, because a refusal and an empty
/// briefing must never be indistinguishable (FR-834, FR-894a).
pub async fn retrieve(
    pool: &PgPool,
    reader: &ReaderContext,
    request: &RetrieveRequest,
    budget_tokens: usize,
    // The hook's own context deadline. Passed in rather than defined here,
    // because a second deadline constant would drift against the one the hook
    // actually enforces.
    deadline_ms: u128,
) -> ApiResult<RetrieveResponse> {
    let started = std::time::Instant::now();

    let binding = match auth::bind_session(pool, reader, request.session_id).await? {
        Ok(binding) => binding,
        // One answer for "no such session" and "not your project", so a caller
        // cannot enumerate session ids one guess at a time.
        Err(SessionBindingError::Unresolvable) => {
            return Err(ApiError::forbidden("you are not a member of this project"))
        }
        Err(SessionBindingError::NotOwned) => {
            return Err(ApiError::forbidden(
                "this session belongs to another account",
            ))
        }
    };

    // Validated, not ignored. `compact` is the whole of post-compaction
    // restoration (FR-838d), so a caller that misspells it would silently get
    // an ordinary session-open briefing instead of a restoration — and an
    // unvalidated string here is a free-text field on an otherwise closed
    // boundary.
    let open_trigger = match (request.trigger, request.open_trigger.as_deref()) {
        (Trigger::SessionOpen, Some(raw)) => match raw.parse::<cairn_core::event::OpenTrigger>() {
            Ok(t) => Some(t),
            Err(_) => {
                return Err(ApiError::invalid(
                    "open_trigger must be one of startup, resume, clear, compact or fork",
                ))
            }
        },
        (Trigger::SessionOpen, None) => None,
        (_, Some(_)) => {
            return Err(ApiError::invalid(
                "open_trigger belongs to a session_open retrieval and to no other",
            ))
        }
        (_, None) => None,
    };

    // Clamped, not trusted. `min` is the whole guard: a caller asking for more
    // than the deployment allows gets the deployment's figure.
    let full = request
        .budget_tokens
        .unwrap_or(budget_tokens)
        .min(budget_tokens);
    let tokens = match request.trigger {
        Trigger::PromptSubmit => (full as f64 * INCREMENTAL_FRACTION).floor() as usize,
        _ => full,
    };

    let trace_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO retrieval_traces
            (trace_id, project_id, session_id, account_id, trigger, delivery_point,
             budget_tokens, delivery_state)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'requested')",
    )
    .bind(trace_id)
    .bind(binding.project_id)
    .bind(request.session_id)
    .bind(reader.user_id())
    .bind(request.trigger.as_str())
    .bind(request.trigger.delivery_point())
    .bind(tokens as i32)
    .execute(pool)
    .await?;

    match generate(
        pool,
        reader,
        request,
        &binding,
        trace_id,
        tokens,
        deadline_ms,
        open_trigger,
        started,
    )
    .await
    {
        Ok(response) => Ok(response),
        Err(e) => {
            // The failed attempt is observable rather than absent, and the
            // error carries the trace so a caller can point at it.
            let reason = failure_reason(&e);
            let latency = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
            let _ = sqlx::query(
                "UPDATE retrieval_traces
                    SET delivery_state = 'failed', failure_reason = $2, latency_ms = $3,
                        updated_at = now()
                  WHERE trace_id = $1",
            )
            .bind(trace_id)
            .bind(reason)
            .bind(latency)
            .execute(pool)
            .await;
            Err(e.with_detail(json!({ "trace_id": trace_id })))
        }
    }
}

/// A bounded reason, never a message. A failure reason is read by a health
/// report, and a free-text one would make two spellings of the same failure
/// count as two failures.
fn failure_reason(e: &ApiError) -> &'static str {
    match e.code {
        "store_unreachable" => "store_unreachable",
        _ => "store_unreachable",
    }
}

#[allow(clippy::too_many_arguments)]
async fn generate(
    pool: &PgPool,
    reader: &ReaderContext,
    request: &RetrieveRequest,
    binding: &auth::SessionBinding,
    trace_id: Uuid,
    tokens: usize,
    deadline_ms: u128,
    open_trigger: Option<cairn_core::event::OpenTrigger>,
    started: std::time::Instant,
) -> ApiResult<RetrieveResponse> {
    let candidates = gather(pool, reader, binding, request.session_id).await?;

    // What this session already has, and when it had it. Read once: the
    // comparison is `relevant MINUS delivered PLUS changed`, and re-reading per
    // item would let the set move underneath the arithmetic.
    let delivered: BTreeMap<String, DateTime<Utc>> = if request.trigger.dedups() {
        sqlx::query(
            "SELECT reference_key, delivered_at FROM delivered_context WHERE session_id = $1",
        )
        .bind(request.session_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|r| (r.get::<String, _>(0), r.get::<DateTime<Utc>, _>(1)))
        .collect()
    } else {
        BTreeMap::new()
    };

    // **The reserve stays withheld here, and that is the whole reason it
    // exists.** One budget is shared by two assemblers: this module selects the
    // durable sections, and the daemon adds the Level 0 it alone can see — the
    // task, the repository's working state, the previous handoff, the warnings
    // and the pins. Level 0 is the part a briefing may not lose.
    //
    // Releasing the reserve here would hand the whole budget to durable memory
    // and leave the assembler that owes the guaranteed minimum with whatever
    // happened to be left, which is exactly the displacement the reserve was
    // introduced to prevent. Nothing in this module is Level 0, so it never
    // spends *from* the reserve; it withholds it *for* the assembler that does,
    // and the two together therefore cannot exceed the budget.
    let mut budget =
        Budget::with_reserve(tokens, (tokens as f64 * RESERVE_FRACTION).floor() as usize);
    let global_cap = (tokens as f64 * cairn_core::context::GLOBAL_SHARE_MAX).floor() as usize;
    let mut global_spent = 0usize;

    let mut sections: BTreeMap<String, Vec<SectionItem>> = BTreeMap::new();
    let mut items: Vec<TraceItem> = Vec::new();
    let mut rank: i32 = 0;

    for section in DURABLE_SECTIONS {
        for candidate in candidates.iter().filter(|c| c.section == *section) {
            let key = candidate.key();

            // Dedup: withheld unless it changed since it was delivered. A
            // domain with no `updated_at` of its own cannot change in place —
            // a revision there is a new record with a new id — so its
            // `source_updated_at` is its creation, and it never re-enters.
            if let Some(delivered_at) = delivered.get(&key) {
                if candidate.source_updated_at <= *delivered_at {
                    items.push(TraceItem::considered(candidate, RULE_ALREADY_DELIVERED));
                    continue;
                }
            }

            let cost = estimate(&candidate.content);
            let global = matches!(*section, "personal_notes" | "team_guidance");
            let admitted = if global {
                // The combined ceiling is fixed the moment the budget is
                // known and does not grow with what other sections left
                // unspent: project-priority headroom is not something global
                // guidance earned.
                global_spent + cost <= global_cap && budget.try_spend(cost)
            } else {
                budget.try_spend(cost)
            };

            if !admitted {
                items.push(TraceItem::considered(candidate, RULE_BUDGET_EXHAUSTED));
                continue;
            }
            if global {
                global_spent += cost;
            }

            rank += 1;
            let rule = match *section {
                "task_memory" | "branch_memory" | "project_memory" => RULE_PROJECT_RESERVE,
                "personal_notes" | "team_guidance" => RULE_GLOBAL_SHARE,
                _ => RULE_GENERAL_POOL,
            };
            let remaining = budget.general_remaining();
            items.push(TraceItem::selected(candidate, rule, rank));
            sections
                .entry((*section).to_string())
                .or_default()
                .push(SectionItem {
                    reference_key: key,
                    ref_kind: candidate.ref_kind(),
                    domain: candidate.domain(),
                    knowledge_id: candidate.reference.record_id(),
                    content: candidate.content.clone(),
                    selection_rule: rule,
                    rank,
                    cost,
                    budget_remaining: remaining,
                });
        }
    }

    persist_items(pool, trace_id, &items).await?;

    let elapsed = started.elapsed().as_millis();
    let level = degradation(elapsed, request.trigger, deadline_ms);
    let latency = elapsed.min(i32::MAX as u128) as i32;

    sqlx::query(
        "UPDATE retrieval_traces
            SET delivery_state = 'generated', degradation_level = $2, budget_spent = $3,
                latency_ms = $4, updated_at = now()
          WHERE trace_id = $1",
    )
    .bind(trace_id)
    .bind(level)
    .bind(budget.spent() as i32)
    .bind(latency)
    .execute(pool)
    .await?;

    Ok(RetrieveResponse {
        trace_id,
        trigger: request.trigger.as_str(),
        delivery_point: request.trigger.delivery_point(),
        // Echoed so the caller can see the server read it as a restoration
        // rather than as an ordinary open. There is no post-compaction
        // delivery point of its own; this is how one is reached.
        open_trigger: open_trigger.map(|t| t.as_str()),
        restored_after_compaction: open_trigger == Some(cairn_core::event::OpenTrigger::Compact),
        degradation_level: level,
        budget: BudgetReport {
            tokens,
            spent: budget.spent(),
            // What this module withheld for the assembler that owes Level 0.
            // Reported rather than left to be re-derived, because a caller
            // recomputing the fraction is a second place for it to drift.
            reserved_for_level0: budget.reserve(),
        },
        served_from_cache: false,
        sections,
    })
}

/// Which of the four levels this retrieval reached.
///
/// **The level says how much of the pipeline ran, never how many items came
/// out.** §5's table and §4.1's worked example read differently on this — the
/// table lists "retrieval produced nothing" under `none`, and the example says
/// an empty prompt-time delivery is `full` with a spend of zero, "distinguished
/// from a failed retrieval: nothing was owed, not something broke". The example
/// is the more specific statement and it is the one that is reasoned, so it
/// governs: a delivery emptied by dedup is a complete delivery of nothing owed,
/// and reporting it as degraded would say the briefing was cut short when the
/// briefing was exactly right.
///
/// What emptiness means is already answered elsewhere and by better fields:
/// `delivery_state` separates a failure from a success (FR-849), and
/// `budget.spent = 0` says nothing was owed. A third field repeating either
/// would only be a third thing that can disagree.
///
/// `none` is therefore reached only by the deadline the hook itself enforces,
/// which is the one case where the guaranteed minimum genuinely did not
/// assemble.
fn degradation(elapsed_ms: u128, trigger: Trigger, deadline_ms: u128) -> &'static str {
    let soft = trigger.soft_target_ms();
    if elapsed_ms <= soft {
        LEVEL_FULL
    } else if elapsed_ms <= soft * 4 {
        LEVEL_REDUCED
    } else if elapsed_ms <= deadline_ms {
        LEVEL_MINIMAL
    } else {
        LEVEL_NONE
    }
}

/// One row of `retrieval_trace_items`.
struct TraceItem {
    ref_kind: &'static str,
    domain: Option<&'static str>,
    knowledge_id: Uuid,
    status: &'static str,
    selection_rule: &'static str,
    rank: Option<i32>,
    source_updated_at: DateTime<Utc>,
}

impl TraceItem {
    fn considered(candidate: &Candidate, rule: &'static str) -> Self {
        Self {
            ref_kind: candidate.ref_kind(),
            domain: candidate.domain(),
            knowledge_id: candidate.reference.record_id(),
            status: "considered",
            selection_rule: rule,
            rank: None,
            source_updated_at: candidate.source_updated_at,
        }
    }

    fn selected(candidate: &Candidate, rule: &'static str, rank: i32) -> Self {
        Self {
            ref_kind: candidate.ref_kind(),
            domain: candidate.domain(),
            knowledge_id: candidate.reference.record_id(),
            status: "selected",
            selection_rule: rule,
            rank: Some(rank),
            source_updated_at: candidate.source_updated_at,
        }
    }
}

async fn persist_items(pool: &PgPool, trace_id: Uuid, items: &[TraceItem]) -> ApiResult<()> {
    for item in items {
        sqlx::query(
            "INSERT INTO retrieval_trace_items
                (trace_id, ref_kind, domain, knowledge_id, status, selection_rule, rank,
                 source_updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (trace_id, reference_key) DO NOTHING",
        )
        .bind(trace_id)
        .bind(item.ref_kind)
        .bind(item.domain)
        .bind(item.knowledge_id)
        .bind(item.status)
        .bind(item.selection_rule)
        .bind(item.rank)
        .bind(item.source_updated_at)
        .execute(pool)
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Gathering candidates, per domain, in that domain's own terms
// ---------------------------------------------------------------------------

/// Every durable record this reader may be offered, by section.
///
/// Per domain because there is no single table to ask, and per domain's own
/// ownership rule because a project member is not the owner of a colleague's
/// personal record. The owner-scoped reads are written as `owner_user_id = $reader`
/// rather than filtered afterwards: a query that could return another account's
/// row and then dropped it is one refactor away from not dropping it.
async fn gather(
    pool: &PgPool,
    reader: &ReaderContext,
    binding: &auth::SessionBinding,
    session_id: Uuid,
) -> ApiResult<Vec<Candidate>> {
    let mut out = Vec::new();

    let session: Option<(Option<Uuid>, String)> =
        sqlx::query_as("SELECT task_id, branch FROM sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(pool)
            .await?;
    let (task_id, branch) = session.unwrap_or((None, String::new()));

    // Project knowledge, most specific scope first — the same task > branch >
    // project gradient the section order already expresses.
    if let Some(task_id) = task_id {
        out.extend(
            project_memory(
                pool,
                binding.project_id,
                "task",
                &task_id.to_string(),
                "task_memory",
            )
            .await?,
        );
    }
    if !branch.is_empty() {
        out.extend(
            project_memory(pool, binding.project_id, "branch", &branch, "branch_memory").await?,
        );
    }
    out.extend(
        project_memory(
            pool,
            binding.project_id,
            "project",
            &binding.project_id.to_string(),
            "project_memory",
        )
        .await?,
    );

    // Patterns are owner-only. `shared_patterns` describes where a pattern is
    // stored, not who may see it (data-model.md §6.2).
    let patterns = sqlx::query(
        "SELECT pattern_id, title, problem, approach, updated_at
           FROM shared_patterns
          WHERE owner_user_id = $1 AND forgotten_at IS NULL
          ORDER BY updated_at DESC, pattern_id
          LIMIT $2",
    )
    .bind(reader.user_id())
    .bind(CANDIDATES_PER_SECTION)
    .fetch_all(pool)
    .await?;
    for row in patterns {
        out.push(Candidate {
            reference: Reference::Pattern(PatternRef(row.get::<Uuid, _>(0))),
            section: "patterns",
            content: format!(
                "{} — {} Approach: {}",
                row.get::<String, _>(1),
                row.get::<String, _>(2),
                row.get::<String, _>(3)
            ),
            source_updated_at: row.get::<DateTime<Utc>, _>(4),
        });
    }

    // Personal knowledge, owner-only. There is no `updated_at` on this table
    // and that is not an omission: a personal record is not edited in place —
    // a revision supersedes it with a new id — so its creation *is* the last
    // moment its content changed, and dedup comparing against it is exact
    // rather than approximate.
    let personal = sqlx::query(
        "SELECT id, content, created_at
           FROM personal_knowledge
          WHERE owner_user_id = $1 AND forgotten_at IS NULL AND superseded_by_id IS NULL
          ORDER BY created_at DESC, id
          LIMIT $2",
    )
    .bind(reader.user_id())
    .bind(CANDIDATES_PER_SECTION)
    .fetch_all(pool)
    .await?;
    for row in personal {
        out.push(Candidate {
            reference: Reference::Knowledge(KnowledgeRef::personal(row.get::<Uuid, _>(0))),
            section: "personal_notes",
            content: row.get::<String, _>(1),
            source_updated_at: row.get::<DateTime<Utc>, _>(2),
        });
    }

    // Team guidance: authoritative only. A proposal is not guidance, and
    // delivering one would make consolidation's own proposals read as settled.
    let team = sqlx::query(
        "SELECT id, content, coalesce(ratified_at, created_at)
           FROM team_knowledge
          WHERE state = 'authoritative' AND superseded_by_id IS NULL
          ORDER BY coalesce(ratified_at, created_at) DESC, id
          LIMIT $1",
    )
    .bind(CANDIDATES_PER_SECTION)
    .fetch_all(pool)
    .await?;
    for row in team {
        out.push(Candidate {
            reference: Reference::Knowledge(KnowledgeRef::team(row.get::<Uuid, _>(0))),
            section: "team_guidance",
            content: row.get::<String, _>(1),
            source_updated_at: row.get::<DateTime<Utc>, _>(2),
        });
    }

    Ok(out)
}

async fn project_memory(
    pool: &PgPool,
    project_id: Uuid,
    scope: &str,
    scope_key: &str,
    section: &'static str,
) -> ApiResult<Vec<Candidate>> {
    let rows = sqlx::query(
        "SELECT id, content, updated_at
           FROM memories
          WHERE project_id = $1 AND scope = $2 AND scope_key = $3
            AND state = 'active' AND deleted_at IS NULL
            AND origin_kind IS DISTINCT FROM 'corroboration'
          ORDER BY updated_at DESC, id
          LIMIT $4",
    )
    .bind(project_id)
    .bind(scope)
    .bind(scope_key)
    .bind(CANDIDATES_PER_SECTION)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Candidate {
            reference: Reference::Knowledge(KnowledgeRef::project(row.get::<Uuid, _>(0))),
            section,
            content: row.get::<String, _>(1),
            source_updated_at: row.get::<DateTime<Utc>, _>(2),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// The transmission outcome (FR-842, FR-843, invariants 11 and 12)
// ---------------------------------------------------------------------------

/// What the daemon may report, and the whole of it.
///
/// No account, no project, no session, no reference, no acknowledgement, no
/// diagnostic field. Everything the server needs it already holds against the
/// trace, and everything else is authority a caller must not be able to assert.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransmissionReport {
    pub outcome: TransmissionOutcome,
    #[serde(default)]
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransmissionOutcome {
    Transmitted,
    Failed,
}

/// The two reasons a hook transmission can fail, and no others.
const HOOK_FAILURE_REASONS: &[&str] = &[
    "hook_transmission_failed",
    "hook_transmission_deadline_exceeded",
];

/// Record what actually happened to a generated briefing.
///
/// The account, project and session come from the stored trace and never from
/// the request, so a caller cannot report an outcome for somebody else's
/// delivery by naming their ids. The reader must own the trace **and** still be
/// authorized for its project: standing at the moment of the report is what is
/// checked, not standing at the moment of retrieval.
///
/// `generated → transmitted` and the `delivered_context` upsert happen in one
/// transaction. `generated → failed` writes no delivery rows at all: dedup must
/// never suppress an item the agent did not receive.
pub async fn report_transmission(
    pool: &PgPool,
    reader: &ReaderContext,
    trace_id: Uuid,
    report: &TransmissionReport,
) -> ApiResult<Value> {
    if let Some(reason) = &report.failure_reason {
        if !HOOK_FAILURE_REASONS.contains(&reason.as_str()) {
            return Err(ApiError::invalid(
                "failure_reason must be one of the declared hook transmission reasons",
            ));
        }
    }
    if report.outcome == TransmissionOutcome::Failed && report.failure_reason.is_none() {
        return Err(ApiError::invalid(
            "a failed transmission must name which of the declared reasons applied",
        ));
    }

    // Read the trace **inside** the transaction that will act on it, and lock
    // the row. Reading first and updating afterwards leaves a window in which a
    // concurrent report moves the state between the two: the guarded `UPDATE`
    // would then match nothing while the delivery insert still ran, writing
    // rows for a transition that did not happen. A retry after a lost response
    // is the ordinary case here, so two reports racing is not exotic.
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT project_id, session_id, account_id, delivery_state, failure_reason
           FROM retrieval_traces WHERE trace_id = $1 FOR UPDATE",
    )
    .bind(trace_id)
    .fetch_optional(&mut *tx)
    .await?;

    // One answer for "no such trace" and "not yours". A foreign account that
    // could tell them apart could enumerate which traces exist.
    let Some(row) = row else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "trace_not_found",
            "no such retrieval trace",
        ));
    };
    let project_id: Uuid = row.get(0);
    let session_id: Uuid = row.get(1);
    let account_id: Uuid = row.get(2);
    let state: String = row.get(3);
    let recorded_reason: Option<String> = row.get(4);

    if account_id != reader.user_id() || !reader.is_member_of(project_id) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "trace_not_found",
            "no such retrieval trace",
        ));
    }

    let wanted = match report.outcome {
        TransmissionOutcome::Transmitted => "transmitted",
        TransmissionOutcome::Failed => "failed",
    };

    match state.as_str() {
        // Repeating the identical terminal report is a no-op success. A retry
        // after a lost response is the ordinary case, and answering it with an
        // error would make the daemon choose between reporting twice and not
        // reporting at all.
        s if s == wanted && same_reason(&recorded_reason, report) => {
            return Ok(json!({ "status": "duplicate", "delivery_state": s }))
        }
        // An opposite terminal outcome is refused rather than overwritten:
        // whichever was true, one of the two reports is wrong, and silently
        // taking the later one would make the record agree with the last
        // caller instead of with what happened.
        "transmitted" | "failed" => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "outcome_conflict",
                "a different terminal transmission outcome is already recorded",
            ))
        }
        // Generation never completed, so there is nothing whose transmission
        // could be reported.
        "requested" => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "outcome_conflict",
                "this retrieval never produced a briefing to transmit",
            ))
        }
        _ => {}
    }

    match report.outcome {
        TransmissionOutcome::Transmitted => {
            sqlx::query(
                "UPDATE retrieval_traces
                    SET delivery_state = 'transmitted', transmission_reported_at = now(),
                        updated_at = now()
                  WHERE trace_id = $1 AND delivery_state = 'generated'",
            )
            .bind(trace_id)
            .execute(&mut *tx)
            .await?;

            // Derived from the trace's own selected items and their stored
            // `source_updated_at`. The daemon resubmits no references and no
            // timestamps, so it cannot widen what was delivered or backdate it.
            sqlx::query(
                "INSERT INTO delivered_context
                    (session_id, ref_kind, domain, knowledge_id, delivered_at,
                     source_updated_at, delivery_point)
                 SELECT $1, i.ref_kind, i.domain, i.knowledge_id, now(), i.source_updated_at,
                        t.delivery_point
                   FROM retrieval_trace_items i
                   JOIN retrieval_traces t ON t.trace_id = i.trace_id
                  WHERE i.trace_id = $2 AND i.status = 'selected'
                 ON CONFLICT (session_id, reference_key) DO UPDATE
                    SET delivered_at = EXCLUDED.delivered_at,
                        source_updated_at = EXCLUDED.source_updated_at,
                        delivery_point = EXCLUDED.delivery_point",
            )
            .bind(session_id)
            .bind(trace_id)
            .execute(&mut *tx)
            .await?;
        }
        TransmissionOutcome::Failed => {
            sqlx::query(
                "UPDATE retrieval_traces
                    SET delivery_state = 'failed', failure_reason = $2,
                        transmission_reported_at = now(), updated_at = now()
                  WHERE trace_id = $1 AND delivery_state = 'generated'",
            )
            .bind(trace_id)
            .bind(report.failure_reason.as_deref())
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;

    Ok(json!({
        "status": "recorded",
        "delivery_state": wanted,
        // Never anything else, for any agent, until a named vendor mechanism
        // establishes receipt. Returning context is transmission; it is not
        // evidence that the agent read it (FR-844, FR-854, SC-712).
        "acknowledgement_state": "unavailable",
    }))
}

fn same_reason(recorded: &Option<String>, report: &TransmissionReport) -> bool {
    match report.outcome {
        TransmissionOutcome::Transmitted => true,
        TransmissionOutcome::Failed => recorded.as_deref() == report.failure_reason.as_deref(),
    }
}

// ---------------------------------------------------------------------------
// Reading a trace back (§6.1, §12.2)
// ---------------------------------------------------------------------------

/// A trace, filtered to what this reader may see.
///
/// A row the reader may not see is **dropped**, never returned as an opaque
/// handle: a handle still discloses that some personal record existed and was
/// used, which is the enumeration FR-846a forbids regardless of content
/// visibility.
///
/// Ranks are re-assigned densely **after** the filter, and budget figures are
/// returned only to the trace's own account. Pre-filter ranks re-enumerate
/// exactly what was withheld — visible ranks 1, 2, 4 prove an item at 3 — and
/// `budget_spent` minus the visible items' cost yields the withheld items'
/// count and size.
pub async fn trace_detail(
    pool: &PgPool,
    reader: &ReaderContext,
    trace_id: Uuid,
) -> ApiResult<Value> {
    let row = sqlx::query(
        "SELECT project_id, session_id, account_id, trigger, delivery_point, degradation_level,
                budget_tokens, budget_spent, latency_ms, delivery_state, acknowledgement_state,
                failure_reason, created_at
           FROM retrieval_traces WHERE trace_id = $1",
    )
    .bind(trace_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "trace_not_found",
            "no such retrieval trace",
        )
    })?;

    let project_id: Uuid = row.get(0);
    let account_id: Uuid = row.get(2);
    if !reader.is_member_of(project_id) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "trace_not_found",
            "no such retrieval trace",
        ));
    }
    let own = account_id == reader.user_id();

    let items = sqlx::query(
        "SELECT ref_kind, domain, knowledge_id, reference_key, status, selection_rule,
                source_updated_at
           FROM retrieval_trace_items
          WHERE trace_id = $1
          ORDER BY status DESC, rank NULLS LAST, reference_key",
    )
    .bind(trace_id)
    .fetch_all(pool)
    .await?;

    let mut visible = Vec::new();
    let mut dense: i32 = 0;
    for item in items {
        let ref_kind: String = item.get(0);
        let domain: Option<String> = item.get(1);
        let knowledge_id: Uuid = item.get(2);
        let Some(reference) = rebuild_reference(&ref_kind, domain.as_deref(), knowledge_id) else {
            continue;
        };
        if auth::reference_visibility(pool, reader, reference).await? != Visibility::Visible {
            continue;
        }
        dense += 1;
        visible.push(json!({
            "ref_kind": ref_kind,
            "domain": domain,
            "knowledge_id": knowledge_id,
            "reference_key": item.get::<String, _>(3),
            "status": item.get::<String, _>(4),
            "selection_rule": item.get::<Option<String>, _>(5),
            "rank": dense,
            "source_updated_at": item.get::<DateTime<Utc>, _>(6),
        }));
    }

    let mut out = json!({
        "trace_id": trace_id,
        "session_id": row.get::<Uuid, _>(1),
        "trigger": row.get::<String, _>(3),
        "delivery_point": row.get::<String, _>(4),
        "degradation_level": row.get::<Option<String>, _>(5),
        "delivery_state": row.get::<String, _>(9),
        "acknowledgement_state": row.get::<String, _>(10),
        "failure_reason": row.get::<Option<String>, _>(11),
        "created_at": row.get::<DateTime<Utc>, _>(12),
        "items": visible,
    });
    if own {
        out["budget"] = json!({
            "tokens": row.get::<Option<i32>, _>(6),
            "spent": row.get::<Option<i32>, _>(7),
        });
        out["latency_ms"] = json!(row.get::<Option<i32>, _>(8));
    }
    Ok(out)
}

fn rebuild_reference(ref_kind: &str, domain: Option<&str>, id: Uuid) -> Option<Reference> {
    match (ref_kind, domain) {
        ("pattern", None) => Some(Reference::Pattern(PatternRef(id))),
        ("knowledge", Some("project")) => Some(Reference::Knowledge(KnowledgeRef::project(id))),
        ("knowledge", Some("personal")) => Some(Reference::Knowledge(KnowledgeRef::personal(id))),
        ("knowledge", Some("team")) => Some(Reference::Knowledge(KnowledgeRef::team(id))),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Retention (FR-847)
// ---------------------------------------------------------------------------

/// Delete traces past their retention window, oldest first, in one bounded
/// batch.
///
/// Bounded so retention never becomes a long lock on a table the request path
/// writes to. Items go with their trace through `ON DELETE CASCADE`.
pub async fn sweep_traces(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let deleted = sqlx::query(
        "DELETE FROM retrieval_traces
          WHERE trace_id IN (
                SELECT trace_id FROM retrieval_traces
                 WHERE created_at < now() - make_interval(days => $1)
                 ORDER BY created_at
                 LIMIT $2)",
    )
    .bind(TRACE_RETENTION_DAYS as i32)
    .bind(SWEEP_BATCH)
    .execute(pool)
    .await?;
    Ok(deleted.rows_affected())
}
