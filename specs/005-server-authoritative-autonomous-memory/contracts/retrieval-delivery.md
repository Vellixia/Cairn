# Contract — Retrieval and Delivery

Getting a briefing into an agent's session without anyone asking, exactly once per fact,
server-side, within a deadline that never blocks the agent.

This contract does not redefine Level 0 / Level 1 / Level 2 or the budget mechanics —
`continuity-context.md` and `recall-composition.md` own those, unchanged. It adds: **where**
delivery happens per agent, **how** a second delivery in one session avoids repeating the
first, **what Cairn does when the deadline is tight**, and **what is recorded**.

## 1. Per-agent delivery matrix

Reproduced from `spec.md`'s "Supported agents and delivery points" (FR-838a–f), checked
against each vendor's official documentation on 2026-08-30. No implementation may narrow it.

| Capability | Claude Code | Codex CLI | OpenCode |
|---|---|---|---|
| Capture events | documented, stable | documented, stable | documented (v1 bus + plugin hooks) |
| Session-open delivery | documented, stable | documented, stable | no stable documented injection point |
| Prompt-time delivery | documented, stable | documented, stable | exists in v2 beta; **declined** |
| Post-compaction opportunity | via session-open, trigger `compact` | via session-open, trigger `compact` | pre-compaction only |
| Receipt acknowledgement | `unavailable / no evidence` | `unavailable / no evidence` | `unavailable / no evidence` |

**Committed automatic delivery: Claude Code and Codex CLI only** (FR-838a). Both document a
prompt-submit hook that fires before the model processes the user's prompt and accepts
`hookSpecificOutput.additionalContext` (FR-838c), and a session-start `source` value of
`compact`, which is how post-compaction restoration is reached (§2).

**OpenCode is capture-only**, reported `declined_by_cairn`, never `unsupported_by_vendor`
(FR-838b). OpenCode 2 exposes prompt/context hooks, but they are beta and the vendor states its
plugin APIs may change; OpenCode v1's injection points exist only in undocumented,
experimentally-named type definitions. Cairn declines to build an automatic-delivery guarantee
on a surface the vendor itself calls unstable — a Cairn decision, stated as one, not a vendor
absence. FR-828 states delivery by capability, not by vendor, so the surface can be added later
without a specification change. OpenCode remains fully supported for capture and manual
`cairn_context`/`cairn_search`.

**Receipt acknowledgement is `unavailable / no evidence` for every agent**, never "vendor does
not support it" (FR-838e): no named mechanism was found in any committed vendor's official
documentation reviewed on 2026-08-30. The entry changes only when a named mechanism is found,
never on the strength of "we didn't see one, so assume it's impossible."

## 2. Delivery points, and the one that does not exist

| Delivery point | When it fires | Population |
|---|---|---|
| **session open** | session-start, for every `open_trigger` (`startup`, `resume`, `clear`, `compact`, `fork`) | every agent whose matrix records session-open delivery |
| **prompt time** | prompt-submit, before the model sees the prompt | agents whose matrix records prompt-time delivery |

**Post-compaction delivery does not exist as its own point** (FR-838d). At least one committed
vendor documents its post-compaction event as unable to carry returned context at all —
the hook has already returned by the time compaction finishes. Restoration reaches the agent
only through the **next session open**, distinguished by `open_trigger = compact`, mirroring
`continuity-context.md`'s "capture is not delivery." Feature 005 never describes post-compaction
delivery as an existing behavior it merely continues.

**Prompt-time selection is never driven by the prompt's text.** No `SafeCanonicalEvent` field is
derived from a user or assistant message (data-model.md §1.3); the prompt-submit hook is
consumed purely as a *delivery point* — a moment Cairn is asked for context — never as query
content. FR-828's "driven by the prompt" is satisfied at the level of *when* retrieval runs, not
by transmitting *what* the prompt says. Selection at both points draws on the same
server-recorded session context: project, branch, bound task, and accumulated safe events for
this session.

## 3. Selection: server-side, membership-gated

`project_id` comes from the session record the daemon presents (bound to an authenticated
credential at session-open, `safe-events.md` §3), never from a client-supplied field.
Membership is checked exactly as every other project-scoped server route
(`crates/cairn-server/src/auth.rs:536`); a non-member is refused —

```
403 forbidden — "you are not a member of this project"
```

— never given an empty result, so a refusal and an empty briefing are never indistinguishable
(mirrors FR-894a for every other project-scoped read API).

```
POST /api/retrieve                              →  { "trace_id", "delivery_point",
{ "session_id", "trigger": "session_open"            "degradation_level", "budget": {tokens,
    | "prompt_submit" | "explicit",                  spent}, "served_from_cache",
  "open_trigger": "…"  // trigger=session_open }      "sections": {…SECTION_ORDER…} }
```

`account_id`/`project_id` are absent from the request — bound server-side, the discipline
`safe-events.md` §3 states for the ingest envelope. `trigger = explicit` is what
`cairn_context`/`cairn_search` produce: no push into the agent's context stream, always
`delivery_point = explicit`, exempt from dedup (§4) — an explicit call is a request to be told
again. `sections` is exactly the shape `continuity-context.md`/`recall-composition.md` already
define (`SECTION_ORDER`: task, repository, previous_handoff, known_failures, decisions,
task_memory, branch_memory, project_memory, patterns, personal_notes, team_guidance) — Feature
005 changes where this is assembled, not its shape.

## 4. Budget arithmetic and de-duplication

Two budgets (data-model.md §7, FR-829, FR-830), so the two points cannot restate each other:

| Delivery point | Budget | At default `context_budget_tokens = 3000` |
|---|---|---|
| session open | the full briefing budget | 3000 |
| prompt time | 25% of the briefing budget | 750 |

Dedup table `delivered_context (session_id, domain, knowledge_id, delivered_at, delivery_point)`,
`PRIMARY KEY (session_id, domain, knowledge_id)`. Only memory-domain sections participate —
`task_memory`, `branch_memory`, `project_memory`, `patterns`, `personal_notes`,
`team_guidance` — since only those carry a stable `KnowledgeRef`; `task`, `repository`,
`decisions` and the other non-memory sections are re-derived fresh every delivery.

```
relevant(session, prompt)
  MINUS delivered_context[session]   -- matched on (domain, knowledge_id)
  PLUS  any delivered item whose updated_at > its delivered_at
```

Already-delivered items drop from later selections unless they changed since delivery, in
which case they re-qualify — the "plus" clause is what lets an edited fact reach the agent
again without waiting for the next session.

### 4.1 Worked example — session open then two prompts

`budget = 3000`, `reserve = floor(3000*0.40) = 1200`, `global_cap = floor(3000*0.15) = 450`
(mechanics per `recall-composition.md` §1–§3, unchanged).

**t0 — session open** (`startup`). Header + Level 0: 300 (reserve 1200 withheld, 300 used, 900
released). `task_memory`/`branch_memory`/`project_memory` `M1`(120) `M2`(90) `M3`(70) = 280,
general pool `3000−300=2700`, remaining `2420`. `personal_notes` `P1`(50): `min(2420,450)=450`
room, fits. `team_guidance` `G1`(40): remaining global allowance `450−50=400`, fits. Total
spend 670. `delivered_context` gets five rows: `{M1,M2,M3,P1,G1}`, `delivery_point=session_open,
delivered_at=t0`.

**t1 — prompt 1**, `incremental_budget=750`. Relevant set recomputes to the same
`{M1,M2,M3,P1,G1}` — nothing new, nothing changed:
`{M1,M2,M3,P1,G1} MINUS {M1,M2,M3,P1,G1} PLUS {} = {}`. Empty selection. Trace:
`degradation_level: full` (deadline met), `budget.spent: 0` — distinguished from a failed
retrieval (§7, FR-849): nothing was owed, not something broke.

**t2 — prompt 2.** A new `M4`(60) was created; `M1` was edited, so
`M1.updated_at > M1.delivered_at(t0)`. `relevant = {M1,M2,M3,M4,P1,G1}`, so:
`{M1,M2,M3,M4,P1,G1} MINUS {M1,M2,M3,P1,G1} PLUS {M1} = {M4, M1}`. `M2,M3,P1,G1` withheld
(delivered, unchanged); `M4` new; `M1` re-enters on its updated timestamp. Cost
`60+120=180 <= 750`, both admitted. `delivered_context` is upserted: `M4` inserted, `M1`'s row
updated to `delivered_at=t2, delivery_point=prompt_time` — the primary key
`(session_id, domain, knowledge_id)` makes this an update, not a second row.

## 5. Degradation — four levels, stated once

**FR-836.** The deadline is the **existing context-class hook deadline**,
`context_deadline_ms`, whose shipped default is **1500 ms**
(`crates/cairn-core/src/config.rs:107`). Both delivery points are context-class, so both are
governed by it; this contract introduces no new deadline constant and must not, because a
second number would drift against the one the hook actually enforces.

Within that budget, retrieval targets **250 ms at session open** and **100 ms at prompt time**
as internal soft targets — prompt-time is tighter because it sits inside the model's turn.
Missing a soft target degrades a level; missing `context_deadline_ms` is what the hook itself
enforces, and the agent proceeds regardless (FR-781).

Exceeding the deadline degrades to exactly one of four pre-declared levels. Latency is never itself an input to content — only the level reached is,
recorded on the briefing and in its trace, so **identical inputs at the same declared level
produce an identical briefing** (FR-835).

| Level | Contains | When reached |
|---|---|---|
| `full` | Tier 0a + Tier 0b of Level 0, all of Level 1 within budget | assembled inside the deadline |
| `reduced` | Level 0 in full. Level 1 **project-domain only** — `task_memory`, `branch_memory`, `project_memory`, `patterns`; `personal_notes`/`team_guidance` never fetched | Level 0 fit; global fetch/admission skipped |
| `minimal` | Level 0 **Tier 0a only** — goal/status, progress counts, `completion_readiness`, single most actionable blocker, `next_action`, critical warning kinds, `repository` | even Tier 0b's bounded reads risked the deadline |
| `none` | Nothing; the briefing is empty and says so | Tier 0a itself missed the deadline, or retrieval produced nothing |

The order — global first, then Tier 0b, then everything but the guaranteed minimum — is
Principle VIII applied under time pressure exactly as `recall-composition.md` applies it under
budget pressure: personal/team is the first casualty of either scarcity, project truth
(Tier 0a) the last.

`none` is a **degraded result**, not a failure. A retrieval that errors outright (store
unreachable, unhandled exception) is reported through `delivery_state = failed` with a
`failure_reason` (§7) — its `degradation_level` is still written `none` to satisfy the schema's
`NOT NULL`, but the two are told apart by `delivery_state`, never conflated (FR-849).

Prompt-time delivery applies the same vocabulary to the incremental selection (§4) — a
degraded prompt-time delivery is `reduced` if only the global increment was dropped, and so
on; there is no fifth, prompt-time-only level. Where a briefing is served from cache,
`served_from_cache: true` is stated (FR-837); caching changes no reported level.

## 6. Traces

`retrieval_traces` / `retrieval_trace_items` (data-model.md §6): `trigger` (why retrieval ran —
`session_open`/`prompt_submit`/`explicit`), `delivery_point` (where it was aimed), the
`degradation_level` and budget accounting from §4–§5, `latency_ms`, `delivery_state`
(`generated`|`transmitted`|`acknowledged`|`unavailable`|`failed`), `failure_reason` (set only on
`failed`), and per-item `(domain, knowledge_id, status ∈ considered|selected, selection_rule,
rank)`.

**The rendered briefing text is never in a trace** (FR-839): text mixes domains and carries
handoff-derived material, so persisting it centrally would place one account's personal
knowledge inside a project-scoped record. A trace records *identities* — which records, why, at
what cost — never the prose a session saw. `cairn_context --explain` continues to answer "why,"
rendered on demand, never stored as text. A failed retrieval is recorded, not omitted (FR-848):
the row exists with `delivery_state=failed` and a reason, so "never happened" and "happened and
produced nothing" stay distinguishable.

### 6.1 Trace readership — withheld, not opaque (FR-846a)

A briefing spans project, personal and team domains, so its trace names records from all three.
Readership is the session's project members, filtered per item at read time:

| Item's domain | Visible to |
|---|---|
| `task_memory`, `branch_memory`, `project_memory`, `patterns` | any project member (unchanged) |
| `team_guidance` | any project member (project/server-wide domain) |
| `personal_notes` | **only** the account owning the referenced record (`personal_knowledge.owner_user_id`) |

A reader who may not see a `personal_notes` item gets that row **dropped from the list**, never
returned as a redacted or opaque reference — an opaque handle still discloses that *some*
personal record existed and was used, exactly the enumeration FR-846a forbids regardless of
content visibility. The filter resolves each `KnowledgeRef` in its own domain table and
compares ownership there; failing rows are excluded entirely, and surviving rows are re-ranked
densely so a gap cannot betray a withheld one (§12.2). `degradation_level` returns to every
reader; `budget_tokens` and `budget_spent` return **only to the trace's own account**, because
the spent total minus the visible items' cost would otherwise yield the withheld items' count
and size. A project member
therefore cannot enumerate a colleague's personal knowledge via traces, regardless of shared
project membership.

### 6.2 Delivery stages, observable per agent (FR-843)

`retrieval_requested` → `context_generated` → `context_transmitted` → `context_acknowledged`.

| Stage | Claude Code | Codex CLI | OpenCode |
|---|---|---|---|
| `retrieval_requested` / `context_generated` | observed (Cairn-side) | observed | n/a — capture only, no request is ever made |
| `context_transmitted` | observed — hook returned `additionalContext` | observed, same shape | n/a |
| `context_acknowledged` | `unavailable / no evidence` | `unavailable / no evidence` | n/a |

Writing context to the hook's return channel is `context_transmitted`, never
`context_acknowledged` (FR-854): Cairn claims only that transmission was attempted and its
outcome, never that the agent consumed it.

## 7. Refusal and error vocabulary

| Code | Meaning |
|---|---|
| `forbidden` (403, "you are not a member of this project") | non-member caller; never an empty briefing (§3) |
| `session_not_found` | `session_id` does not resolve to a session bound to the caller's credential |
| `retrieval_deadline_exceeded` | internal signal driving a degradation (§5); never surfaced to the agent as an error |
| `store_unreachable` | `delivery_state = failed`, `failure_reason = store_unreachable` |

## Invariants

1. Automatic delivery is committed for Claude Code and Codex CLI only; OpenCode's delivery is
   `declined_by_cairn`, never `unsupported_by_vendor` (FR-838a, FR-838b).
2. Post-compaction restoration is reached only through a session opened with `open_trigger =
   compact`; no context is ever returned from a post-compaction event itself (FR-838d).
3. Receipt acknowledgement is `unavailable / no evidence` for every agent until a named vendor
   mechanism is found (FR-838e).
4. Retrieval enforces project membership itself; a non-member is refused, never given an empty
   result (FR-834, mirrors FR-894a).
5. Session-open uses the full briefing budget; prompt-time uses 25% of it, applied to whichever
   items survive dedup (FR-829, FR-830, data-model.md §7).
6. Prompt-time selection is `relevant MINUS delivered_context[session]   -- matched on (domain, knowledge_id) PLUS {items updated
   since their delivery}` — never a resend of what the session already has unless it changed.
7. Retrieval degrades to exactly one of `full`, `reduced`, `minimal`, `none` on a deadline; the
   level is recorded on the briefing and in its trace; wall-clock latency is never itself part
   of briefing content (FR-836, FR-835).
8. A trace never carries rendered briefing text — only identities, budget accounting and the
   degradation level (FR-839, FR-846).
9. A trace reader never receives a reference to a `personal_notes` item they may not see — the
   row is dropped, never rendered as an opaque handle (FR-846a).
10. A failed retrieval is recorded with its failure reason, and is distinguishable from a
    degraded-to-`none` retrieval and from a briefing that was legitimately empty (FR-848,
    FR-849).

---

## 12. Cross-domain references, dedup placement, trace scoping and the outage cache

These four rules govern wherever an earlier section is less specific. They are stated here once
rather than as amendments to §§4-6.

### 12.0 Every knowledge reference is a `KnowledgeRef`

Project, personal and team knowledge live in three different tables. A bare `memory_id` names
only the first. Traces, `delivered_context`, authorization and web rendering all use
`KnowledgeRef = (domain, knowledge_id)` (`data-model.md` §6.1). `updated_at` is read from the
referenced record's own table, and ownership is checked in that domain's own terms — a project
member is not an owner of a colleague's personal record.

### 12.1 `delivered_context` is a server table

Selection is server-side, so the server must hold both sides of the dedup comparison. A
client-side `delivered_context` made the rule uncomputable: the request carries a session, not
the set of what that session already received, and every prompt-time delivery would restate the
session-open briefing (FR-830).

`delivered_context` therefore lives in PostgreSQL (`data-model.md` §6). Two consequences:

- Deleting the local store does **not** cause re-delivery, and the table is not in the
  durability-loss list.
- A row is written **when transmission is attempted and did not fail** — never at selection.
  Writing at selection suppresses, for the life of the session, items the agent never received.

### 12.2 Ranks are assigned after filtering, and budget figures are scoped

§6.1 withholds trace items the reader may not see, but dense pre-filter ranks and unfiltered
budget arithmetic re-enumerate exactly what was withheld: visible ranks 1, 2, 4, 5 prove an item
at rank 3, and `budget_spent` minus the visible items' cost yields the withheld items' count and
size. FR-846a forbids enumeration "directly or by inference from identifiers".

Correction:

- `rank` is assigned **after** the authorization filter, densely, per reader. A reader sees
  1..n with no gaps.
- `budget_tokens` and `budget_spent` are returned **only to the trace's own account**. Other
  readers get the item list and the degradation level, with budget figures omitted rather than
  approximated.
- Item counts shown to another reader are counts of what that reader can see.

### 12.3 Outage behaviour of a server-side briefing

Retrieval moved server-side, so an outage means no fresh briefing. FR-789, FR-790a and SC-718
still bind:

- The daemon keeps a **bounded local briefing cache**: the last briefing per session, capped at
  64 KiB per session and 200 sessions, refilled on every successful retrieval, invalidated on
  sign-out, on credential change, and on any change of authenticated account.
- A cached briefing is **bound to the account it was assembled for** and is never served to a
  different account (FR-790a).
- It is served only when the server is unreachable, and is labelled cached and possibly stale
  (FR-789, FR-837).
- If no cache entry exists for the session and account, Cairn reports fresh knowledge as
  unavailable rather than serving nothing silently.

This is the cache Principle II permits, and its bound, refill and invalidation policy is
stated here as that principle requires.
