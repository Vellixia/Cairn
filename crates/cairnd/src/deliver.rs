//! Server-authoritative retrieval, merged with the daemon's own Level 0
//! assembly, and the account-bound outage cache (T072;
//! `contracts/retrieval-delivery.md` §1–§6, §12.3).
//!
//! # One budget, two assemblers
//!
//! The server selects the durable sections — `task_memory`, `branch_memory`,
//! `project_memory`, `patterns`, `personal_notes`, `team_guidance` — against
//! one delivery point's whole budget, and reports what it spent
//! (`budget.tokens`, `budget.spent`) plus what it withheld for whoever owes
//! Level 0 (`budget.reserved_for_level0`). This module gives the daemon's own
//! Level 0 / local-section assembly (`crate::briefing::build`) exactly what is
//! left — `tokens - spent`, which the server guarantees is never less than
//! `reserved_for_level0` — and never recomputes that fraction itself: a
//! second place computing it is a second place for it to drift.
//!
//! `patterns` is the one durable section this module does **not** take from
//! the server. The server's `patterns` candidates are this account's own
//! patterns, most-recent-first (`cairn-server/src/retrieve.rs::gather`); the
//! daemon's own `crate::briefing::level1_patterns` matches them against
//! *this project's* recorded signals instead — a materially richer selection
//! the server's bare content string cannot reconstruct. Local patterns are
//! kept as they were before Feature 005 US2.
//!
//! # The outage cache (§12.3, FR-789, FR-790a, SC-718)
//!
//! Retrieval moved server-side, so an outage means no fresh *durable*
//! knowledge — Level 0 is always current, because it never left the local
//! store. The cache below holds the server's last answer per session, bound
//! to the account it was assembled for, and is consulted only when the server
//! cannot be reached at all this call.

use crate::state::{Daemon, Resolved};
use cairn_core::wire::ContextDepth;
use serde_json::{json, Value};
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

struct CachedResponse {
    account_id: Uuid,
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
/// outage degrades the durable half of a briefing rather than blanking it —
/// Level 0 is unaffected either way, because it is always assembled fresh
/// from the local store.
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
        let response = hit.response.clone();
        self.touch(session_id);
        Some(response)
    }

    /// Invalidated on sign-out, on credential change, and on any change of
    /// authenticated account (FR-790a) — called from
    /// [`Daemon::mutate_credentials`], the single door for all three.
    pub fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
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
    /// `None` when there is nothing to report an outcome against: an explicit
    /// local-only fallback (no session, or the server never answered) and a
    /// cache hit both carry no fresh `generated` trace (§3, §6.2 — only a
    /// `generated` trace can become `transmitted`).
    pub trace_id: Option<Uuid>,
    /// The rendered briefing, in the same shape `crate::briefing::build`
    /// already produces, with the durable sections merged in and
    /// `trace_id`/`degradation_level`/`served_from_cache` added at the top
    /// level for a caller that reads the raw value rather than the struct.
    pub payload: Value,
    pub degradation_level: String,
    pub served_from_cache: bool,
}

/// Retrieve, merge with the daemon's own Level 0 assembly, and fall back to
/// the outage cache when the server cannot be reached within `deadline`
/// (`contracts/retrieval-delivery.md` §1–§6, §12.3). `deadline` is the
/// existing `context_deadline_ms` — this module introduces no deadline
/// constant of its own.
pub async fn deliver(
    d: &Daemon,
    resolved: &Resolved,
    session_id: Uuid,
    trigger: Trigger,
    open_trigger: Option<&str>,
    deadline: Duration,
) -> Delivered {
    let account_id = d.account_identity().await;

    let remote = tokio::time::timeout(
        deadline,
        retrieve_remote(d, session_id, trigger, open_trigger),
    )
    .await
    .ok()
    .flatten();

    let (response, served_from_cache) = match remote {
        Some(response) => {
            if let Some(account_id) = account_id {
                d.outage_cache
                    .lock()
                    .await
                    .put(session_id, account_id, &response);
            }
            (Some(response), false)
        }
        None => {
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
    let full_budget = d.config.read().await.context_budget_tokens;
    let local_budget = meta.local_budget(full_budget);

    let session = cairn_store::repo::session(&d.store, session_id).await.ok();
    let local = crate::briefing::build(
        d,
        resolved,
        session.as_ref(),
        local_budget,
        false,
        false,
        ContextDepth::Standard,
    )
    .await;
    let mut payload = match local {
        Ok(built) => serde_json::to_value(built).unwrap_or_else(|_| json!({})),
        Err(e) => json!({ "error": e.message }),
    };

    match response.as_ref().and_then(|r| r.get("sections")) {
        Some(sections) => merge_durable_sections(&mut payload, sections),
        // Nothing fresh and nothing cached for this session and account:
        // said, not silently served as though there were nothing to say
        // (§12.3's closing bullet).
        None => {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("fresh_knowledge_unavailable".into(), json!(true));
            }
        }
    }
    embed_meta(&mut payload, &meta, served_from_cache);

    Delivered {
        trace_id: meta.trace_id,
        payload,
        degradation_level: meta.degradation_level,
        served_from_cache,
    }
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

/// `POST /api/retrieve`, or `None` on anything short of a successful answer —
/// no credential, a transport failure, or a non-2xx status. The caller cannot
/// tell those apart and does not need to: every one of them means the same
/// thing here, fall back to the cache.
async fn retrieve_remote(
    d: &Daemon,
    session_id: Uuid,
    trigger: Trigger,
    open_trigger: Option<&str>,
) -> Option<Value> {
    let creds = d.server.read().await.clone();
    let base = creds.url?;
    let token = creds.token?;
    let http = reqwest::Client::builder().build().ok()?;

    let mut body = json!({
        "session_id": session_id,
        "trigger": trigger.as_str(),
    });
    // `open_trigger` belongs to a `session_open` retrieval and to no other
    // (the server refuses it otherwise) — never sent for the other two.
    if trigger == Trigger::SessionOpen {
        if let Some(ot) = open_trigger {
            body["open_trigger"] = json!(ot);
        }
    }

    let url = format!("{}/api/retrieve", base.trim_end_matches('/'));
    let response = http
        .post(url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Value>().await.ok()
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
    tokens: usize,
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
            tokens: 0,
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
            tokens: response
                .get("budget")
                .and_then(|b| b.get("tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
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
/// shown. `patterns` is deliberately left as `crate::briefing::build` left
/// it; see the module docs.
fn merge_durable_sections(payload: &mut Value, sections: &Value) {
    let Some(briefing) = payload.get_mut("briefing").and_then(|b| b.as_object_mut()) else {
        return;
    };
    if let Some(memory) = briefing.get_mut("memory").and_then(|m| m.as_object_mut()) {
        memory.insert(
            "task".into(),
            json!(section_contents(sections, "task_memory")),
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

    fn response(trace: &str, level: &str, tokens: u64, spent: u64) -> Value {
        json!({
            "trace_id": trace,
            "degradation_level": level,
            "budget": { "tokens": tokens, "spent": spent, "reserved_for_level0": tokens * 4 / 10 },
            "sections": {
                "personal_notes": [{ "content": "p1" }],
                "team_guidance": [{ "content": "g1" }],
                "task_memory": [{ "content": "t1" }],
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

    /// Sign-out, a credential change, and any account change all invalidate
    /// the whole cache (FR-790a) — this is the primitive `mutate_credentials`
    /// calls; the wiring itself is exercised in `state.rs`.
    #[test]
    fn clear_drops_every_entry() {
        let mut cache = OutageCache::default();
        let session = Uuid::now_v7();
        let owner = Uuid::now_v7();
        cache.put(session, owner, &response("t1", "full", 3000, 0));
        assert!(cache.get(session, owner).is_some());

        cache.clear();

        assert!(cache.get(session, owner).is_none());
        assert_eq!(cache.len(), 0);
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
                "memory": { "task": [], "branch": [], "project": [] },
            },
            "estimated_tokens": 0,
        })
    }

    #[test]
    fn durable_sections_are_merged_into_the_matching_fields() {
        let mut payload = bare_payload();
        let sections = json!({
            "task_memory": [{ "content": "t1" }],
            "branch_memory": [{ "content": "b1" }],
            "project_memory": [{ "content": "p1" }],
            "personal_notes": [{ "content": "n1" }],
            "team_guidance": [{ "content": "g1" }],
        });
        merge_durable_sections(&mut payload, &sections);

        assert_eq!(payload["briefing"]["memory"]["task"], json!(["t1"]));
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
