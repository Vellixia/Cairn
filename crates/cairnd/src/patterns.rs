//! The pattern surface: promote, list, show, record an outcome, forget
//! (`contracts/patterns.md` §Surfaces, FR-406).
//!
//! Every refusal here is `ok: false`. Promotion is a configuration-class
//! operation and fails loudly rather than soft — a gate that quietly did
//! nothing would be worse than one that refused, because the developer would
//! believe the pattern exists.

use crate::state::{storage_err, Daemon};
use cairn_core::domain::{PatternDiscovery, PatternOutcome, PatternTrust};
use cairn_core::wire::{codes, WireError};
use cairn_store::patterns::{self as store, Candidate, NewApplication, Promotion};
use serde_json::json;
use uuid::Uuid;

type Reply = Result<serde_json::Value, WireError>;

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
        Promotion::Promoted(p) => Ok(json!({
            "pattern": p,
            // A dry run reports the same answer and writes nothing, so a
            // developer can ask before committing to the wording.
            "dry_run": r.dry_run,
            "counters": store::counters(&d.store, p.id).await.ok().map(|c| c.render()),
        })),
        Promotion::Refused { class, message } => Err(WireError::new(class, message)),
    }
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

    let mut out = Vec::with_capacity(filtered.len());
    for p in filtered {
        let counters = store::counters(&d.store, p.id).await.map_err(storage_err)?;
        out.push(json!({
            "id": p.id,
            "title": p.title,
            "trust": p.trust,
            "signals": p.signals,
            // The one permitted phrasing. Never a number of verifications.
            "counts": counters.render(),
            "origin": store::origin_of(&p),
        }));
    }
    Ok(json!({ "patterns": out, "total": out.len() }))
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
    // `pattern_not_found` rather than a silent success.
    store::pattern(&d.store, id).await.map_err(|e| match e {
        cairn_store::StoreError::NotFound(_) => {
            WireError::new(codes::PATTERN_NOT_FOUND, format!("no pattern {id}"))
        }
        other => storage_err(other),
    })?;
    store::forget(&d.store, id).await.map_err(storage_err)?;
    Ok(json!({ "forgotten": id }))
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
