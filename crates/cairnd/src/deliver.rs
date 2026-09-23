//! Server-authoritative retrieval, merged with the daemon's own Level 0
//! assembly, and the account-bound outage cache (T072;
//! `contracts/retrieval-delivery.md` §1–§6, §12.3).
//!
//! # One budget, two assemblers
//!
//! The server selects the durable sections — `session_memory`, `branch_memory`,
//! `project_memory`, `patterns`, `personal_notes`, `team_guidance` — against
//! one delivery point's whole budget, and reports what it spent
//! (`budget.tokens`, `budget.spent`) plus what it withheld for whoever owes
//! Level 0 (`budget.reserved_for_level0`). This module gives the daemon's own
//! Level 0 / local-section assembly (`crate::briefing::build`) exactly what is
//! left — `tokens - spent`, which the server guarantees is never less than
//! `reserved_for_level0` — and never recomputes that fraction itself: a
//! second place computing it is a second place for it to drift.
//!
//! `patterns` is taken from the server like every other durable section, and
//! for a stricter reason than the others. The server selects a canonical
//! `shared_patterns` row, budgets it, and traces its `pattern_id` as
//! *selected*; on the transmission report it copies that same id into
//! `delivered_context`. So the id the server will record as delivered is
//! fixed before this module runs, and the only way that record can be true is
//! for the content rendered here to be that exact canonical pattern. The
//! server therefore sends the pattern's own fields alongside its id
//! (`cairn-server/src/retrieve.rs::SectionPattern`) and the merge below
//! renders those fields under that id.
//!
//! The daemon's own `crate::briefing::level1_patterns` reads local
//! `reusable_patterns` — this machine's promotions, matched against this
//! project's recorded signals. That is a different universe of rows with
//! different ids, so substituting one of them for a canonical selection would
//! make the server's `delivered_context` a record of something the agent never
//! saw. `briefing::build` runs it only under `Durable::Local`, for an unlinked
//! project that has no server selection to be faithful to.
//!
//! # The outage cache (§12.3, FR-789, FR-790a, SC-718)
//!
//! Retrieval moved server-side, so an outage means no fresh *durable*
//! knowledge. The cache below holds the server's last answer per session,
//! bound to the account it was assembled for, and is consulted only when the
//! server cannot be reached at all this call.
//!
//! **Level 0 is not always current, and saying so was the FR-790a defect.**
//! For a project whose briefing is server-side, a call with no fresh response
//! and no cache entry *for this account* serves nothing derived from the local
//! store — not Level 0 or previous handoff.
//! On a cache miss the server has not established what this caller may see, so
//! there is nothing to check them against, and the local store is one machine's
//! store shared by every account that signs in on it. An **unlinked** project
//! is the other case and keeps its local assembly: there is no server authority
//! to defer to, so its own store is the only authority there is.

use crate::state::{Daemon, Resolved};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Trigger
// ---------------------------------------------------------------------------

/// Why retrieval ran (`contracts/retrieval-delivery.md` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Parse the wire value `Request::Context::trigger` carries.
    ///
    /// Anything unrecognized — including absent, which is how every caller
    /// written before this field existed still parses — becomes `Explicit`,
    /// never an automatic trigger: `explicit` is the one value that asserts
    /// no push and permits no transmission report, so it is the safe
    /// direction to fall in when a value cannot be trusted (§3).
    pub fn parse(s: &str) -> Self {
        match s {
            "session_open" => Trigger::SessionOpen,
            "prompt_submit" => Trigger::PromptSubmit,
            _ => Trigger::Explicit,
        }
    }
}

// ---------------------------------------------------------------------------
// The outage cache
// ---------------------------------------------------------------------------

const CACHE_MAX_SESSIONS: usize = 200;
const CACHE_MAX_BYTES: usize = 64 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(300);

struct CachedResponse {
    account_id: Uuid,
    cached_at: std::time::Instant,
    /// The server's own answer, verbatim — `sections`, `degradation_level`,
    /// `budget`, `trace_id` and all. Read back through the same parser a
    /// fresh response goes through ([`ResponseMeta::extract`]), with
    /// `from_cache: true` so its `trace_id` is discarded rather than replayed
    /// against a report the server never asked for.
    response: Value,
}

/// Last briefing per session, account-bound, LRU-evicted at
/// [`CACHE_MAX_SESSIONS`] sessions, each entry capped at [`CACHE_MAX_BYTES`].
///
/// A cache, not durable state (Principle II): in-memory, lost on restart, and
/// rebuilt by the next successful retrieval. It exists solely so a server
/// outage degrades the durable half of a briefing rather than blanking it.
/// A *hit* is also the evidence that the server authorized this account for
/// this session, which is exactly what a miss does not have — see the module
/// header for what a miss may therefore serve.
#[derive(Default)]
pub struct OutageCache {
    /// Most-recently-used session id first.
    order: VecDeque<Uuid>,
    entries: HashMap<Uuid, CachedResponse>,
}

impl OutageCache {
    fn touch(&mut self, session_id: Uuid) {
        self.order.retain(|s| *s != session_id);
        self.order.push_front(session_id);
    }

    /// Refill on a successful retrieval (§12.3).
    ///
    /// An over-budget response is **rejected outright, never truncated**: a
    /// truncated durable section would misrepresent what the server actually
    /// said the last time it was reachable, which is worse than simply not
    /// caching it. The session keeps whatever entry it already had.
    fn put(&mut self, session_id: Uuid, account_id: Uuid, response: &Value) {
        let bytes = serde_json::to_vec(response)
            .map(|b| b.len())
            .unwrap_or(usize::MAX);
        if bytes > CACHE_MAX_BYTES {
            return;
        }
        self.entries.insert(
            session_id,
            CachedResponse {
                account_id,
                cached_at: std::time::Instant::now(),
                response: response.clone(),
            },
        );
        self.touch(session_id);
        while self.order.len() > CACHE_MAX_SESSIONS {
            if let Some(evicted) = self.order.pop_back() {
                self.entries.remove(&evicted);
            }
        }
    }

    /// Served only for the account it was assembled for (FR-790a) — an entry
    /// belonging to a different account is treated exactly as though none
    /// existed, never returned and never even inspected beyond the id check.
    fn get(&mut self, session_id: Uuid, account_id: Uuid) -> Option<Value> {
        let hit = self.entries.get(&session_id)?;
        if hit.account_id != account_id {
            return None;
        }
        if hit.cached_at.elapsed() > CACHE_TTL {
            self.entries.remove(&session_id);
            self.order.retain(|id| *id != session_id);
            return None;
        }
        let mut response = hit.response.clone();
        if let Some(object) = response.as_object_mut() {
            object.insert(
                "cache_age_seconds".into(),
                json!(hit.cached_at.elapsed().as_secs()),
            );
            object.insert("cache_account_id".into(), json!(account_id));
        }
        self.touch(session_id);
        Some(response)
    }

    fn invalidate(&mut self, session_id: Uuid, account_id: Uuid) {
        if self
            .entries
            .get(&session_id)
            .is_some_and(|entry| entry.account_id == account_id)
        {
            self.entries.remove(&session_id);
            self.order.retain(|id| *id != session_id);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

// ---------------------------------------------------------------------------
// Delivery
// ---------------------------------------------------------------------------

/// What one delivery produced, ready for a caller to render and — except for
/// [`Trigger::Explicit`], and except when [`Delivered::trace_id`] is `None` —
/// report the transmission outcome of.
pub struct Delivered {
    pub payload: Value,
}

/// Retrieve, merge with the daemon's own Level 0 assembly, and fall back to
/// the outage cache when the server cannot be reached within `deadline`
/// (`contracts/retrieval-delivery.md` §1–§6, §12.3). `deadline` is the
/// existing `context_deadline_ms` — this module introduces no deadline
/// constant of its own.
pub async fn deliver(
    d: &Daemon,
    _resolved: &Resolved,
    session_id: Uuid,
    trigger: Trigger,
    open_trigger: Option<&str>,
    // What this machine may spend. Sent to the server rather than applied
    // afterwards, because the two assemblers share one budget and only the side
    // that selects first can keep the total inside it — trimming the answer
    // here would already have exceeded it.
    budget_tokens: usize,
    deadline: Duration,
) -> Delivered {
    let account_id = d.account_identity().await;

    // A session created moments ago may still be in a typed durable lane.
    // Drain one bounded pass before retrieval so the server can bind it. This
    // deliberately does not invoke legacy entity sync or any pull path.
    let _ = tokio::time::timeout(deadline / 2, crate::sync::drain_typed_spools(d)).await;

    // A timeout is silence, exactly as a transport failure is.
    let remote = tokio::time::timeout(
        deadline,
        retrieve_remote(d, session_id, trigger, open_trigger, budget_tokens),
    )
    .await
    .unwrap_or(Answer::Unreachable);

    let (response, served_from_cache) = match remote {
        Answer::Answered(response) => {
            if let Some(account_id) = account_id {
                d.outage_cache
                    .lock()
                    .await
                    .put(session_id, account_id, &response);
            }
            (Some(response), false)
        }
        // Refused, by something that was there to refuse. No cache, because a
        // hit would claim an authorization this very call was denied.
        Answer::Refused => {
            if let Some(account_id) = account_id {
                d.outage_cache
                    .lock()
                    .await
                    .invalidate(session_id, account_id);
            }
            (None, false)
        }
        Answer::Rejected => (None, false),
        Answer::Unreachable => {
            let cached = match account_id {
                Some(account_id) => d.outage_cache.lock().await.get(session_id, account_id),
                None => None,
            };
            match cached {
                Some(cached) => (Some(cached), true),
                None => (None, false),
            }
        }
    };

    let meta = ResponseMeta::extract(response.as_ref(), served_from_cache);
    let mut payload = response.unwrap_or_else(|| {
        json!({
            "fresh_knowledge_unavailable": true,
            "degradation_level": "none",
            "sections": {},
        })
    });
    embed_meta(&mut payload, &meta, served_from_cache);

    Delivered { payload }
}

/// Report what actually happened to a generated briefing
/// (`contracts/retrieval-delivery.md` §3, §6.2).
///
/// Best-effort and idempotent by construction: the server answers a repeated
/// identical report with `duplicate` rather than an error (§3), so a caller
/// retrying after a dropped response needs no retry loop of its own here.
///
/// **Never call this with `transmitted: true` without having actually
/// written the context to the hook's return channel.** Generating a briefing
/// is not evidence that an agent received one (FR-843, FR-854) — that is the
/// entire reason this is a second call, made by the daemon after the caller
/// tells it what happened, rather than something `deliver` claims on its own.
pub async fn report_outcome(d: &Daemon, trace_id: Uuid, transmitted: bool, reason: Option<&str>) {
    let creds = d.server.read().await.clone();
    let (Some(base), Some(token)) = (creds.url, creds.token) else {
        return;
    };
    let Ok(http) = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    else {
        return;
    };
    let body = if transmitted {
        json!({ "outcome": "transmitted" })
    } else {
        json!({
            "outcome": "failed",
            "failure_reason": reason.unwrap_or("hook_transmission_failed"),
        })
    };
    let url = format!(
        "{}/api/retrieval-traces/{trace_id}/transmission",
        base.trim_end_matches('/')
    );
    if let Err(e) = http.post(url).bearer_auth(token).json(&body).send().await {
        tracing::debug!(error = %e, %trace_id, "transmission outcome not reported");
    }
}

/// `POST /api/retrieve`, resolved into the three outcomes the outage cache
/// turns on ([`Answer`]).
///
/// **The caller must tell these apart, and folding them together was a
/// defect.** Only [`Answer::Unreachable`] permits the cache to answer, so the
/// mapping is the whole authorization story of an outage:
///
/// - **no credential**, a client that will not build, a transport failure, or
///   a 2xx whose body will not parse → [`Answer::Unreachable`]. Nothing
///   answered, or nothing intelligible did, so a previously authorized entry
///   for this account and session may still stand in (§12.3).
/// - **401, 403, 404** → [`Answer::Refused`]. Authentication or existence was
///   denied, so matching cache is invalidated.
/// - **5xx** → [`Answer::Unreachable`]. Server failure is outage, not auth.
/// - **other 4xx** → [`Answer::Rejected`]. No cache this turn, no invalidation.
async fn retrieve_remote(
    d: &Daemon,
    session_id: Uuid,
    trigger: Trigger,
    open_trigger: Option<&str>,
    budget_tokens: usize,
) -> Answer {
    let creds = d.server.read().await.clone();
    let (Some(base), Some(token)) = (creds.url, creds.token) else {
        return Answer::Unreachable;
    };
    let Ok(http) = reqwest::Client::builder().build() else {
        return Answer::Unreachable;
    };

    let mut body = json!({
        "session_id": session_id,
        "trigger": trigger.as_str(),
        // The server clamps this to its own figure, so asking is always safe
        // and never widens anything.
        "budget_tokens": budget_tokens,
    });
    // `open_trigger` belongs to a `session_open` retrieval and to no other
    // (the server refuses it otherwise) — never sent for the other two.
    if trigger == Trigger::SessionOpen {
        if let Some(ot) = open_trigger {
            body["open_trigger"] = json!(ot);
        }
    }

    let url = format!("{}/api/retrieve", base.trim_end_matches('/'));
    let Ok(response) = http.post(url).bearer_auth(token).json(&body).send().await else {
        // Nothing answered. This is the outage the cache exists for.
        return Answer::Unreachable;
    };
    if response.status().is_server_error() {
        return Answer::Unreachable;
    }
    if !response.status().is_success() {
        // **Something answered, and it refused.**
        //
        // This used to be folded into "unreachable", and the fold was a
        // cross-deployment leak: replace the server at the same address and the
        // old token authenticates against nothing there, so every retrieval
        // came back `401` — which looked exactly like silence, so the daemon
        // served the *predecessor's* cached briefing and labelled it cached, as
        // though the new deployment had authorized it. A cache hit is supposed
        // to be evidence that the server authorized this account for this
        // session; a live refusal is evidence of the opposite, and it cannot be
        // allowed to produce one.
        return match response.status().as_u16() {
            401 | 403 | 404 => Answer::Refused,
            _ => Answer::Rejected,
        };
    }
    match response.json::<Value>().await {
        Ok(value) => Answer::Answered(value),
        // A 2xx whose body will not parse is a server that answered
        // incomprehensibly, not one that declined. Treated as silence.
        Err(_) => Answer::Unreachable,
    }
}

/// What `/api/retrieve` did, distinguished because the cache turns on it.
enum Answer {
    Answered(Value),
    /// Reachable, and it declined — a wrong deployment, a revoked token, a
    /// session it does not hold. The outage cache must not answer for it.
    Refused,
    Rejected,
    /// Nothing answered at all.
    Unreachable,
}

// ---------------------------------------------------------------------------
// Reading the server's answer
// ---------------------------------------------------------------------------

/// The parts of `/api/retrieve`'s response this module reasons about,
/// pulled out of the raw `Value` once so the rest of the module never
/// re-parses it.
struct ResponseMeta {
    trace_id: Option<Uuid>,
    degradation_level: String,
    #[cfg(test)]
    tokens: usize,
    #[cfg(test)]
    spent: usize,
}

impl ResponseMeta {
    /// No response at all — the server was unreachable and nothing was
    /// cached for this session and account. `none` here is not a claim that
    /// the briefing is empty (Level 0 never is): it says durable retrieval
    /// produced nothing, which is true because none was attempted (§5's
    /// `none` row: "retrieval produced nothing").
    fn unavailable() -> Self {
        Self {
            trace_id: None,
            degradation_level: "none".to_string(),
            #[cfg(test)]
            tokens: 0,
            #[cfg(test)]
            spent: 0,
        }
    }

    /// `from_cache` discards `trace_id`: a cached answer's trace was already
    /// resolved (`generated` → `transmitted` or `failed`) the call it was
    /// captured on, and replaying its id here would let a later transmission
    /// report land against a trace this call never asked the server to make
    /// (§3's idempotency is about *repeating* a report, not about reusing a
    /// stale identity for a new one).
    fn extract(response: Option<&Value>, from_cache: bool) -> Self {
        let Some(response) = response else {
            return Self::unavailable();
        };
        Self {
            trace_id: if from_cache {
                None
            } else {
                response
                    .get("trace_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
            },
            degradation_level: response
                .get("degradation_level")
                .and_then(|v| v.as_str())
                .unwrap_or("none")
                .to_string(),
            #[cfg(test)]
            tokens: response
                .get("budget")
                .and_then(|b| b.get("tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
            #[cfg(test)]
            spent: response
                .get("budget")
                .and_then(|b| b.get("spent"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
        }
    }

    /// What the daemon's own Level 0 / local-section assembly may spend.
    ///
    /// `tokens - spent` when the server answered (fresh or cached) — a number
    /// the server guarantees is never less than what it withheld for exactly
    /// this (`budget.reserved_for_level0`), so this never recomputes that
    /// fraction itself. The whole local budget when nothing durable was
    /// retrieved at all: nothing else claimed a share of it that time.
    #[cfg(test)]
    fn local_budget(&self, full: usize) -> usize {
        if self.tokens == 0 {
            full
        } else {
            self.tokens.saturating_sub(self.spent)
        }
    }
}

/// One durable section's admitted content, in the order the server admitted
/// it, discarding everything but the rendered text — reference keys, ranks
/// and costs are trace-only detail (`contracts/retrieval-delivery.md` §6),
/// not briefing content.
#[cfg(test)]
fn section_contents(sections: &Value, name: &str) -> Vec<String> {
    sections
        .get(name)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|it| {
                    it.get("content")
                        .and_then(|c| c.as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Replace the daemon's own (locally recomputed, undeduplicated) durable
/// section content with the server's. Selection and dedup happened
/// server-side against `delivered_context`, which the daemon's own read of
/// the same tables never sees (`contracts/retrieval-delivery.md` §4) — so the
/// server's answer, not the daemon's own read, is what a caller must be
/// shown. `patterns` is rendered from the canonical fields the server sent
/// with its selection, under the very id the server traced; see the module
/// docs for why that one is not merely a preference.
#[cfg(test)]
fn merge_durable_sections(payload: &mut Value, sections: &Value) {
    let Some(briefing) = payload.get_mut("briefing").and_then(|b| b.as_object_mut()) else {
        return;
    };
    if let Some(memory) = briefing.get_mut("memory").and_then(|m| m.as_object_mut()) {
        memory.insert(
            "session".into(),
            json!(section_contents(sections, "session_memory")),
        );
        memory.insert(
            "branch".into(),
            json!(section_contents(sections, "branch_memory")),
        );
        memory.insert(
            "project".into(),
            json!(section_contents(sections, "project_memory")),
        );
    }
    // `Briefing`'s own fields are `#[serde(skip_serializing_if =
    // "Vec::is_empty")]` (FR-481: byte-identical output for a caller with
    // nothing in either domain), which only governs serializing *from* the
    // struct. This is a raw JSON merge after that already happened, so an
    // empty section is dropped here rather than inserted as a present but
    // empty array.
    for key in ["personal_notes", "team_guidance"] {
        let items = section_contents(sections, key);
        if items.is_empty() {
            briefing.remove(key);
        } else {
            briefing.insert(key.into(), json!(items));
        }
    }

    // **The server's canonical patterns, and only those.**
    //
    // This section used not to be merged at all, and the omission was the
    // canonical pattern-delivery defect. The server selected a `PatternRef`,
    // spent budget on it and recorded it as selected; the daemon then rendered
    // whatever its *local* matcher found in `reusable_patterns` — a different
    // store, with different identities — and reported the transmission
    // successful, at which point the server copied its selected refs into
    // `delivered_context`. A reference could be recorded as delivered without
    // the pattern behind it ever having been rendered.
    //
    // Each item is rendered under the id the server traced, from the canonical
    // fields the server sent with it, so the reference, the budgeted content
    // and the text the agent reads are one record. `signal_overlap` is absent
    // because no signal comparison ran: the server selected by budget.
    let patterns: Vec<Value> = sections
        .get("patterns")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let id = item.get("knowledge_id")?.as_str()?;
                    let p = item.get("pattern")?;
                    Some(json!({
                        "id": id,
                        "title": p.get("title").and_then(Value::as_str).unwrap_or_default(),
                        "trust": p.get("trust").and_then(Value::as_str).unwrap_or("sanitized"),
                        "verified_in_this_project": false,
                        "applicability": p.get("applicability").cloned().unwrap_or(json!([])),
                        "approach": p.get("approach").and_then(Value::as_str).unwrap_or_default(),
                        "constraints": p.get("constraints").cloned().unwrap_or(json!([])),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    if patterns.is_empty() {
        briefing.remove("patterns");
    } else {
        briefing.insert("patterns".into(), json!(patterns));
    }
}

/// Add what a caller needs beyond the rendered briefing itself: whether this
/// answer is fresh or replayed, at what level, and — only when it is fresh —
/// the trace to report a transmission outcome against.
fn embed_meta(payload: &mut Value, meta: &ResponseMeta, served_from_cache: bool) {
    let Some(obj) = payload.as_object_mut() else {
        return;
    };
    obj.insert(
        "trace_id".into(),
        meta.trace_id.map(|id| json!(id)).unwrap_or(Value::Null),
    );
    obj.insert("degradation_level".into(), json!(meta.degradation_level));
    obj.insert("served_from_cache".into(), json!(served_from_cache));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ServerCredentials;
    use crate::testsupport as fx;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn response(trace: &str, level: &str, tokens: u64, spent: u64) -> Value {
        json!({
            "trace_id": trace,
            "degradation_level": level,
            "budget": { "tokens": tokens, "spent": spent, "reserved_for_level0": tokens * 4 / 10 },
            "sections": {
                "personal_notes": [{ "content": "p1" }],
                "team_guidance": [{ "content": "g1" }],
                "session_memory": [{ "content": "s1" }],
            },
        })
    }

    #[test]
    fn trigger_parses_the_three_wire_values_and_nothing_else_as_automatic() {
        assert_eq!(Trigger::parse("session_open"), Trigger::SessionOpen);
        assert_eq!(Trigger::parse("prompt_submit"), Trigger::PromptSubmit);
        // Absent, misspelled, or anything else Cairn has never declared: the
        // one direction that asserts no push (§3).
        assert_eq!(Trigger::parse("explicit"), Trigger::Explicit);
        assert_eq!(Trigger::parse("bogus"), Trigger::Explicit);
        assert_eq!(Trigger::parse(""), Trigger::Explicit);
    }

    #[test]
    fn trigger_as_str_round_trips_through_parse() {
        for t in [
            Trigger::SessionOpen,
            Trigger::PromptSubmit,
            Trigger::Explicit,
        ] {
            assert_eq!(Trigger::parse(t.as_str()), t);
        }
    }

    // -- OutageCache -----------------------------------------------------

    /// The invariant the caller specifically asked to see tested: a cached
    /// entry is bound to the account it was assembled for and is never
    /// served to a different one (FR-790a). Not "empty" or "an error" — the
    /// same as no entry existing at all, so a second account cannot even
    /// learn that a first account has a cached briefing here.
    #[test]
    fn a_cached_entry_never_crosses_accounts() {
        let mut cache = OutageCache::default();
        let session = Uuid::now_v7();
        let owner = Uuid::now_v7();
        let intruder = Uuid::now_v7();

        cache.put(session, owner, &response("t1", "full", 3000, 100));

        assert!(
            cache.get(session, intruder).is_none(),
            "a different account must not read the owner's cached briefing"
        );
        assert!(
            cache.get(session, owner).is_some(),
            "the owning account's own read must still succeed"
        );
    }

    /// Refilled on every successful retrieval, and the newest answer is what
    /// a later outage replays (§12.3).
    #[test]
    fn a_second_put_for_the_same_session_replaces_the_first() {
        let mut cache = OutageCache::default();
        let session = Uuid::now_v7();
        let owner = Uuid::now_v7();

        cache.put(session, owner, &response("t1", "full", 3000, 100));
        cache.put(session, owner, &response("t2", "reduced", 3000, 40));

        let got = cache.get(session, owner).expect("entry");
        assert_eq!(got["trace_id"], "t2");
        assert_eq!(got["cache_account_id"], owner.to_string());
        assert!(got["cache_age_seconds"].is_u64());
    }

    #[test]
    fn an_expired_entry_cannot_answer_an_outage() {
        let mut cache = OutageCache::default();
        let session = Uuid::now_v7();
        let owner = Uuid::now_v7();
        cache.put(session, owner, &response("t1", "full", 3000, 100));
        cache.entries.get_mut(&session).unwrap().cached_at =
            std::time::Instant::now() - CACHE_TTL - Duration::from_secs(1);
        assert!(cache.get(session, owner).is_none());
    }

    #[test]
    fn a_live_refusal_removes_the_entry_before_a_later_outage() {
        let mut cache = OutageCache::default();
        let session = Uuid::now_v7();
        let owner = Uuid::now_v7();
        cache.put(session, owner, &response("t1", "full", 3000, 100));
        cache.invalidate(session, owner);
        assert!(cache.get(session, owner).is_none());
    }

    #[tokio::test]
    async fn a_server_error_uses_an_eligible_cached_answer() {
        let repo = fx::Repo::with(cairn_core::CairnConfig::default()).await;
        let resolved = repo.daemon.resolve(&repo.cwd).await.unwrap();
        let session = Uuid::now_v7();
        let account = Uuid::now_v7();
        repo.daemon.outage_cache.lock().await.put(
            session,
            account,
            &response("cached", "full", 3000, 100),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await.unwrap();
            socket.write_all(b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await.unwrap();
        });
        *repo.daemon.server.write().await = ServerCredentials {
            url: Some(format!("http://{address}")),
            token: Some("token".into()),
            account_id: Some(account),
        };
        let delivered = deliver(
            &repo.daemon,
            &resolved,
            session,
            Trigger::Explicit,
            None,
            3000,
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(delivered.payload["served_from_cache"], true);
        assert_eq!(delivered.payload["cache_account_id"], account.to_string());
    }

    /// An over-budget entry is rejected outright, and whatever the session
    /// already had survives untouched — never silently truncated into a
    /// misrepresentation of what the server actually said.
    #[test]
    fn an_over_budget_entry_is_rejected_not_truncated() {
        let mut cache = OutageCache::default();
        let session = Uuid::now_v7();
        let owner = Uuid::now_v7();

        cache.put(session, owner, &response("t1", "full", 3000, 100));

        let huge_note = "x".repeat(CACHE_MAX_BYTES + 1024);
        let oversized = json!({
            "trace_id": "t2",
            "degradation_level": "full",
            "budget": { "tokens": 3000, "spent": 100, "reserved_for_level0": 1200 },
            "sections": { "personal_notes": [{ "content": huge_note }] },
        });
        cache.put(session, owner, &oversized);

        let got = cache
            .get(session, owner)
            .expect("the original entry survives");
        assert_eq!(
            got["trace_id"], "t1",
            "the oversized put must not have landed"
        );
    }

    /// LRU eviction at the session cap: the least recently touched session is
    /// the one that goes.
    #[test]
    fn the_least_recently_used_session_is_evicted_at_the_cap() {
        let mut cache = OutageCache::default();
        let owner = Uuid::now_v7();
        let sessions: Vec<Uuid> = (0..CACHE_MAX_SESSIONS).map(|_| Uuid::now_v7()).collect();

        for s in &sessions {
            cache.put(*s, owner, &response("t", "full", 3000, 0));
        }
        assert_eq!(cache.len(), CACHE_MAX_SESSIONS);

        // Touch every session but the first, so it is unambiguously the
        // least recently used one when the cap is next exceeded.
        for s in &sessions[1..] {
            assert!(cache.get(*s, owner).is_some());
        }

        let newcomer = Uuid::now_v7();
        cache.put(newcomer, owner, &response("t", "full", 3000, 0));

        assert_eq!(cache.len(), CACHE_MAX_SESSIONS);
        assert!(
            cache.get(sessions[0], owner).is_none(),
            "the session nothing touched again must be the one evicted"
        );
        assert!(cache.get(newcomer, owner).is_some());
    }

    // -- ResponseMeta ------------------------------------------------------

    #[test]
    fn local_budget_is_tokens_minus_spent_when_the_server_answered() {
        let meta = ResponseMeta::extract(Some(&response("t1", "full", 3000, 700)), false);
        assert_eq!(meta.local_budget(3000), 2300);
    }

    /// Guaranteed never less than what the server withheld — this is the
    /// property the coordinator's fix (`reserved_for_level0`) exists for,
    /// checked from the daemon's side of the same arithmetic.
    #[test]
    fn local_budget_never_falls_below_the_servers_own_reserve() {
        let response = response("t1", "full", 3000, 1799); // spends right up to the edge
        let meta = ResponseMeta::extract(Some(&response), false);
        let reserved = response["budget"]["reserved_for_level0"].as_u64().unwrap() as usize;
        assert!(meta.local_budget(3000) >= reserved);
    }

    #[test]
    fn local_budget_falls_back_to_the_full_local_budget_when_nothing_was_retrieved() {
        let meta = ResponseMeta::unavailable();
        assert_eq!(meta.local_budget(3000), 3000);
    }

    /// An empty durable selection is a complete delivery of nothing owed
    /// (§4.1's worked example), never treated as degraded here — this module
    /// only ever passes the server's own `degradation_level` through, never
    /// reinterprets it by how many items came back.
    #[test]
    fn an_empty_selection_still_reports_the_servers_own_level_untouched() {
        let empty = json!({
            "trace_id": "t1",
            "degradation_level": "full",
            "budget": { "tokens": 750, "spent": 0, "reserved_for_level0": 300 },
            "sections": {},
        });
        let meta = ResponseMeta::extract(Some(&empty), false);
        assert_eq!(meta.degradation_level, "full");
    }

    #[test]
    fn a_cached_answer_never_carries_a_reportable_trace_id() {
        let meta = ResponseMeta::extract(Some(&response("t1", "full", 3000, 0)), true);
        assert_eq!(meta.trace_id, None);
    }

    // -- merge_durable_sections --------------------------------------------

    fn bare_payload() -> Value {
        json!({
            "briefing": {
                "memory": { "session": [], "branch": [], "project": [] },
            },
            "estimated_tokens": 0,
        })
    }

    #[test]
    fn durable_sections_are_merged_into_the_matching_fields() {
        let mut payload = bare_payload();
        let sections = json!({
            "session_memory": [{ "content": "s1" }],
            "branch_memory": [{ "content": "b1" }],
            "project_memory": [{ "content": "p1" }],
            "personal_notes": [{ "content": "n1" }],
            "team_guidance": [{ "content": "g1" }],
        });
        merge_durable_sections(&mut payload, &sections);

        assert_eq!(payload["briefing"]["memory"]["session"], json!(["s1"]));
        assert_eq!(payload["briefing"]["memory"]["branch"], json!(["b1"]));
        assert_eq!(payload["briefing"]["memory"]["project"], json!(["p1"]));
        assert_eq!(payload["briefing"]["personal_notes"], json!(["n1"]));
        assert_eq!(payload["briefing"]["team_guidance"], json!(["g1"]));
    }

    /// FR-481: a caller with nothing in a global domain sees exactly what a
    /// caller who never touched that domain sees — the key absent, not
    /// present with an empty array.
    #[test]
    fn an_empty_global_section_is_dropped_not_inserted_empty() {
        let mut payload = bare_payload();
        merge_durable_sections(&mut payload, &json!({}));
        assert!(payload["briefing"].get("personal_notes").is_none());
        assert!(payload["briefing"].get("team_guidance").is_none());
    }

    #[test]
    fn embed_meta_nulls_the_trace_id_when_absent() {
        let mut payload = json!({});
        let meta = ResponseMeta::unavailable();
        embed_meta(&mut payload, &meta, false);
        assert!(payload["trace_id"].is_null());
        assert_eq!(payload["degradation_level"], "none");
        assert_eq!(payload["served_from_cache"], false);
    }
}
