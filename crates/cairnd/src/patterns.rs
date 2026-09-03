//! The pattern surface: promote, list, show, record an outcome, forget
//! (`contracts/patterns.md` §Surfaces, FR-406).
//!
//! Every refusal here is `ok: false`. Promotion is a configuration-class
//! operation and fails loudly rather than soft — a gate that quietly did
//! nothing would be worse than one that refused, because the developer would
//! believe the pattern exists.
//!
//! ## Two records, one pattern (Feature 005, FR-708)
//!
//! A pattern used to be local and nothing else. It now has a **canonical
//! server record** — an owner-scoped personal-domain row — and the local row
//! stays, because the two hold different things:
//!
//! - The server holds the **transferable knowledge**: title, problem, root
//!   cause, approach, constraints, applicability. That is what survives losing
//!   this machine (FR-703, SC-738).
//! - The local row holds the **machine-local evidence**: `signals`,
//!   `signal_digest`, the salted `origin_ref`, `sanitization_report`,
//!   `source_memory_id`, `origin_deleted` — the six names the privacy boundary
//!   refuses — and the `pattern_applications` that derive `validated` and
//!   `contested`. Those stay here under FR-707 and FR-708a, and they are
//!   *not* backed up. Losing this store loses them.
//!
//! So promotion writes the local row and **queues the safe shape as a command**;
//! it does not push a row. Forgetting does both halves, addressing the server
//! record by the identity both sides derive rather than by a mapping either
//! side could get wrong (`knowledge-commands.md` §3.3, FR-708f).
//!
//! Under `AuthorityMode::Feature004` none of that happens and the surface
//! behaves exactly as it did: the store is still the authority, and a command
//! queued against a server that owns nothing would be a command nothing ever
//! applies.

use crate::state::{storage_err, Daemon};
use cairn_core::domain::{PatternDiscovery, PatternOutcome, PatternTrust};
use cairn_core::wire::{codes, WireError};
use cairn_store::patterns::{self as store, Candidate, NewApplication, Promotion};
use serde_json::json;
use uuid::Uuid;

type Reply = Result<serde_json::Value, WireError>;

/// Whether this store's durable knowledge belongs to the server.
///
/// Read rather than assumed on every mutation, because the answer decides
/// whether a write is a local record or a request the server accepts or refuses
/// (FR-712). A store that cannot answer is treated as not authoritative: the
/// local path is the one that works without a server, and guessing the other
/// way would queue commands nothing will ever apply.
async fn server_owns_patterns(d: &Daemon) -> bool {
    cairn_store::authority::mode(&d.store)
        .await
        .map(|m| m.commands_are_authoritative())
        .unwrap_or(false)
}

/// The canonical identity of a local pattern's server record.
///
/// Derived on both sides from the same three fields, so neither side stores a
/// mapping that could go stale and a retried promotion converges on one record
/// (FR-708f, SC-760). `owner` is the authenticated account — the credential,
/// never anything the caller supplies (Principle XI).
fn server_pattern_id(owner: Uuid, p: &store::Pattern) -> Uuid {
    let content_key = cairn_core::eventid::content_key(&p.problem, &p.root_cause, &p.approach);
    cairn_core::eventid::pattern_id(owner, &content_key)
}

/// The safe shape, derived from the local row.
///
/// **Six fields are dropped, and dropping them here is the point.** `signals`,
/// `signal_digest`, `origin_ref`, `sanitization_report`, `source_memory_id` and
/// `origin_deleted` are refused names at the boundary; the server refuses a body
/// carrying any of them, and this is the reason it never has to. `trust` is
/// absent for the same class of reason — the server assigns `sanitized`, the one
/// level it can establish, and `validated` is a state this machine earned
/// privately from applications the server cannot see (FR-708g).
fn safe_promotion_payload(p: &store::Pattern) -> serde_json::Value {
    json!({
        "title": p.title,
        "problem": p.problem,
        "root_cause": p.root_cause,
        "approach": p.approach,
        "constraints": p.constraints,
        "applicability": p.applicability,
    })
}

/// What `cairn pattern promote` proposes.
pub struct PromoteRequest {
    pub memory_id: Uuid,
    pub title: Option<String>,
    pub problem: Option<String>,
    pub signals: Vec<String>,
    pub applicability: Vec<String>,
    pub root_cause: Option<String>,
    pub approach: Option<String>,
    pub constraints: Vec<String>,
    pub dry_run: bool,
}

pub async fn promote(d: &Daemon, cwd: &str, r: PromoteRequest) -> Reply {
    d.resolve(cwd).await?;
    let config = d.config.read().await.clone();

    // The source memory supplies what the caller did not. An agent proposing a
    // promotion has already written the memory; making it retype the content is
    // how the two drift apart.
    let memory = cairn_store::repo::memory(&d.store, r.memory_id)
        .await
        .map_err(storage_err)?;
    let title = r.title.unwrap_or_else(|| first_line(&memory.content));
    let problem = r.problem.unwrap_or_else(|| memory.content.clone());
    let root_cause = r.root_cause.unwrap_or_else(|| memory.content.clone());
    let approach = r.approach.unwrap_or_else(|| memory.content.clone());

    let outcome = store::promote(
        &d.store,
        r.memory_id,
        Candidate {
            title: &title,
            problem: &problem,
            signals: &r.signals,
            applicability: &r.applicability,
            root_cause: &root_cause,
            approach: &approach,
            constraints: &r.constraints,
        },
        config.pattern_signals_min,
        r.dry_run,
    )
    .await
    .map_err(storage_err)?;

    match outcome {
        Promotion::Promoted(p) => {
            // The safe shape goes to the server, and only after the local gate
            // passed. Sending first would let a candidate the local gate refuses
            // — "not yet fit to leave" is exactly what `candidate` trust means —
            // become durable somewhere this machine cannot retract it.
            //
            // A dry run sends nothing, for the reason a dry run exists.
            let durable = if r.dry_run || !server_owns_patterns(d).await {
                None
            } else {
                Some(queue_promotion(d, &p).await?)
            };
            Ok(json!({
                "pattern": p,
                // A dry run reports the same answer and writes nothing, so a
                // developer can ask before committing to the wording.
                "dry_run": r.dry_run,
                "counters": store::counters(&d.store, p.id).await.ok().map(|c| c.render()),
                // Truthfully: accepted for delivery, not yet durable (FR-815a).
                // `null` when this store is still its own authority, which is a
                // different statement from "queued and waiting".
                "server": durable,
            }))
        }
        Promotion::Refused { class, message } => Err(WireError::new(class, message)),
    }
}

/// Queue the promotion of one local pattern to the server.
///
/// The account is required rather than optional. A command with no account
/// cannot be claimed — the claim predicate matches an account exactly — so
/// queueing one would be a silent black hole, and `queue_knowledge_command`
/// refuses for that reason. Reading the identity here as well lets the reply
/// name the `pattern_id` the server will derive, which is what `cairn pattern
/// forget` addresses later.
async fn queue_promotion(d: &Daemon, p: &store::Pattern) -> Result<serde_json::Value, WireError> {
    let Some(owner) = d.account_identity().await else {
        return Err(WireError::new(
            codes::NOT_LINKED,
            "sign in before promoting a pattern: the server owns durable \
             knowledge now, and a command with no account could never be \
             delivered",
        ));
    };
    let mut queued = crate::handlers::queue_knowledge_command(
        d,
        None,
        None,
        cairn_store::spool::CommandKind::PatternPromote,
        &safe_promotion_payload(p),
    )
    .await?;
    // Named in the reply so the caller can address the canonical record without
    // deriving the identity a second time, in a second place, from the same
    // three fields.
    queued["pattern_id"] = json!(server_pattern_id(owner, p));
    Ok(queued)
}

pub async fn list(
    d: &Daemon,
    cwd: &str,
    trust: Option<PatternTrust>,
    signal: Option<String>,
) -> Reply {
    d.resolve(cwd).await?;
    let all = store::list(&d.store, trust).await.map_err(storage_err)?;

    let filtered: Vec<_> = match &signal {
        Some(token) => {
            let wanted = cairn_core::patterns::normalize_signals(std::slice::from_ref(token));
            all.into_iter()
                .filter(|p| cairn_core::patterns::signal_overlap(&p.signals, &wanted) > 0)
                .collect()
        }
        None => all,
    };

    // Who the durability question is asked on behalf of. Without an account
    // there is no owner, so no local row can have a canonical counterpart and
    // every one of them is honestly local-only.
    let owner = d.account_identity().await;
    let authoritative = server_owns_patterns(d).await;
    let durable = match (authoritative, owner) {
        (true, Some(owner)) => durable_pattern_ids(d, owner).await,
        _ => Durable::NotAsked,
    };

    let mut out = Vec::with_capacity(filtered.len());
    for p in filtered {
        let counters = store::counters(&d.store, p.id).await.map_err(storage_err)?;
        // **Cached and server-accepted are distinguishable here** (FR-710).
        // Every row in this list is local; what differs is whether losing this
        // store loses the knowledge in it, and a list that did not say so would
        // present a machine-local pattern and a backed-up one as the same thing.
        let canonical = owner.map(|owner| server_pattern_id(owner, &p));
        out.push(json!({
            "id": p.id,
            "title": p.title,
            "trust": p.trust,
            "signals": p.signals,
            // The one permitted phrasing. Never a number of verifications.
            "counts": counters.render(),
            "origin": store::origin_of(&p),
            "pattern_id": canonical,
            "durability": durable.classify(canonical),
        }));
    }
    Ok(json!({
        "patterns": out,
        "total": out.len(),
        // Where the durability answer came from, so a reader can tell "not
        // backed up" from "could not ask" (FR-710a).
        "durability_source": durable.source(),
    }))
}

/// Which of this owner's patterns the server is actually holding.
///
/// Read from the server when it answers, and from the local cache when it does
/// not. The distinction is kept rather than collapsed, because a cache that has
/// never refilled and a server that holds nothing produce the same empty set and
/// mean opposite things (FR-710a).
enum Durable {
    /// The server answered. This is the set it holds.
    Server(std::collections::BTreeSet<Uuid>),
    /// The server did not answer; this is what the cache last saw.
    Cache(std::collections::BTreeSet<Uuid>),
    /// Not server-authoritative, or not signed in. There is no canonical record
    /// to have, so the question does not arise.
    NotAsked,
}

impl Durable {
    fn classify(&self, canonical: Option<Uuid>) -> &'static str {
        let Some(id) = canonical else {
            return "local_only";
        };
        match self {
            Durable::Server(held) if held.contains(&id) => "server",
            Durable::Server(_) => "local_only",
            Durable::Cache(held) if held.contains(&id) => "server_cached",
            // The cache is the only witness and it has not seen this one. That
            // is not proof the server does not hold it, so it is reported as
            // unknown rather than as a loss the user has not actually taken.
            Durable::Cache(_) => "unknown",
            Durable::NotAsked => "local_only",
        }
    }

    fn source(&self) -> &'static str {
        match self {
            Durable::Server(_) => "server",
            Durable::Cache(_) => "cache",
            Durable::NotAsked => "local",
        }
    }
}

/// Ask the server what it holds, falling back to the cache.
///
/// A read, not a mutation, so an unreachable server degrades rather than fails:
/// `cairn pattern list` is an agent-facing operation and must not block on the
/// network (FR-781). What degrades is the *confidence* of the durability
/// column, and that is reported rather than hidden.
async fn durable_pattern_ids(d: &Daemon, owner: Uuid) -> Durable {
    if let Ok(client) = crate::sync::client(d).await {
        if let Ok(body) = client.get("/api/patterns").await {
            let held = body
                .get("patterns")
                .and_then(serde_json::Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(|r| r.get("pattern_id"))
                        .filter_map(serde_json::Value::as_str)
                        .filter_map(|s| Uuid::parse_str(s).ok())
                        .collect()
                })
                .unwrap_or_default();
            return Durable::Server(held);
        }
    }
    match cairn_store::global::cached_patterns(&d.store, owner).await {
        Ok(rows) => Durable::Cache(rows.into_iter().map(|r| r.pattern_id).collect()),
        Err(e) => {
            tracing::debug!(error = %e, "the pattern cache could not be read");
            Durable::Cache(std::collections::BTreeSet::new())
        }
    }
}

pub async fn show(d: &Daemon, cwd: &str, id: Uuid) -> Reply {
    d.resolve(cwd).await?;
    let p = store::pattern(&d.store, id).await.map_err(|e| match e {
        cairn_store::StoreError::NotFound(_) => {
            WireError::new(codes::PATTERN_NOT_FOUND, format!("no pattern {id}"))
        }
        other => storage_err(other),
    })?;
    let counters = store::counters(&d.store, id).await.map_err(storage_err)?;
    let causes = store::alternative_causes(&d.store, id)
        .await
        .map_err(storage_err)?;

    Ok(json!({
        "pattern": p,
        "counts": counters.render(),
        "applications": counters.applications,
        "distinct_projects": counters.distinct_projects_applied,
        "independently_validated_in": counters.qualifying_successes,
        "counterexamples": counters.counterexamples,
        "alternative_causes": causes,
        "origin": store::origin_of(&p),
    }))
}

#[allow(clippy::too_many_arguments)]
pub async fn record_outcome(
    d: &Daemon,
    cwd: &str,
    id: Uuid,
    outcome: PatternOutcome,
    signals: Vec<String>,
    alternative_cause: Option<String>,
    evidence_id: Option<Uuid>,
    session: Option<Uuid>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    // An application needs an origin session and only that — the same rule a
    // memory follows. Requiring a developer to open one before saying "this
    // pattern did not apply here" would cost the record most of the outcomes
    // worth having, and a counterexample nobody records is a pattern that stays
    // wrong (FR-404).
    let session_id = crate::handlers::ensure_session_for_memory(d, &r, session, None)
        .await?
        .id;

    // `discovery` is decided **here**, from whether this session was actually
    // shown the pattern — never taken from the caller. An agent cannot be asked
    // to report honestly on whether it was influenced by something it read
    // (FR-401, FR-403).
    let discovery = if suggested_to(d, session_id, id).await {
        PatternDiscovery::CairnSuggested
    } else {
        PatternDiscovery::Independent
    };

    let trust = store::record_outcome(
        &d.store,
        NewApplication {
            pattern_id: id,
            project_id: r.project.id,
            session_id,
            signals: &signals,
            outcome,
            discovery,
            alternative_cause: alternative_cause.as_deref(),
            evidence_id,
        },
    )
    .await
    .map_err(|e| match e {
        cairn_store::StoreError::Refused { code, message } => WireError::new(code, message),
        cairn_store::StoreError::NotFound(_) => {
            WireError::new(codes::PATTERN_NOT_FOUND, format!("no pattern {id}"))
        }
        other => storage_err(other),
    })?;

    let counters = store::counters(&d.store, id).await.map_err(storage_err)?;
    Ok(json!({
        "pattern_id": id,
        "outcome": outcome,
        "discovery": discovery,
        "trust": trust,
        "counts": counters.render(),
    }))
}

pub async fn forget(d: &Daemon, cwd: &str, id: Uuid) -> Reply {
    d.resolve(cwd).await?;
    // Reading first, so forgetting something that is not there is
    // `pattern_not_found` rather than a silent success. It is also the only
    // place the three fields the canonical identity is derived from are still
    // to hand: after the local row is gone there is nothing left to derive from,
    // so the server record would be unaddressable and would quietly outlive the
    // forget.
    let p = store::pattern(&d.store, id).await.map_err(|e| match e {
        cairn_store::StoreError::NotFound(_) => {
            WireError::new(codes::PATTERN_NOT_FOUND, format!("no pattern {id}"))
        }
        other => storage_err(other),
    })?;

    // The server first, and the local row only if that was accepted for
    // delivery. The other order forgets the copy this machine can retract and
    // leaves the durable one standing — with the local row gone, nothing can
    // name it again. A queue that refuses (no account, saturated) therefore
    // refuses the whole operation rather than half of it.
    let server = if server_owns_patterns(d).await {
        let Some(owner) = d.account_identity().await else {
            return Err(WireError::new(
                codes::NOT_LINKED,
                "sign in before forgetting a pattern: its durable record is the \
                 server's, and a command with no account could never be delivered",
            ));
        };
        let pattern_id = server_pattern_id(owner, &p);
        Some(
            crate::handlers::queue_knowledge_command(
                d,
                None,
                None,
                cairn_store::spool::CommandKind::PatternForget,
                &json!({ "target_id": pattern_id }),
            )
            .await?,
        )
    } else {
        None
    };

    store::forget(&d.store, id).await.map_err(storage_err)?;
    // The local applications go with it, and they are not recoverable: they are
    // machine-local evidence with no server table (FR-707). Said here because
    // this is the point the choice is made.
    Ok(json!({
        "forgotten": id,
        "server": server,
        "local_only_lost": ["pattern_applications", "signals", "origin_ref"],
    }))
}

/// Whether this session received the pattern in its context.
///
/// Recorded by the briefing rather than asked of the agent. Today the briefing
/// does not persist what it offered, so this is answered conservatively: a
/// session that could have been shown the pattern — because the pattern's
/// signals match what this project recorded — is treated as having been shown
/// it. That direction of error is the safe one: it withholds a validation that
/// might have been independent, rather than granting one that was not
/// (FR-403).
async fn suggested_to(d: &Daemon, session_id: Uuid, pattern_id: Uuid) -> bool {
    let Ok(pattern) = store::pattern(&d.store, pattern_id).await else {
        return false;
    };
    let errors = cairn_store::repo::recent_error_summaries(&d.store, session_id, 20)
        .await
        .unwrap_or_default();
    let config = d.config.read().await.clone();
    cairn_core::patterns::signal_overlap(&pattern.signals, &errors) >= config.pattern_signals_min
}

fn first_line(content: &str) -> String {
    content
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(128)
        .collect()
}
