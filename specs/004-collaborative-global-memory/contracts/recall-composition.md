# Contract: Recall Composition

**Feature**: `004-collaborative-global-memory`

This contract guarantees that personal and team knowledge can only ever add to a briefing,
never displace project truth from it: the guaranteed Level 0 reserve remains project-only by
construction, global sections spend only what project-priority sections leave over, that
leftover is additionally capped as a fraction of the whole budget, and a caller who has never
touched personal or team knowledge sees a byte-identical briefing to one who has. It also
guarantees that project and global search results are never merged into one ranked list,
because a relevance score computed against one corpus is not a number that means anything
against another.

## 1. The existing Level 0 / 1 / 2 model, exactly as implemented

```text
LEVEL 0  minimum safe continuity   — a reserved share the lower levels cannot take
LEVEL 1  relevant current knowledge — the remaining budget, ranked
LEVEL 2  history and evidence       — never automatic; explicit request only
```

Module doc, unchanged by this feature: `crates/cairn-core/src/context.rs:1-29`. Section
priority order, `SECTION_ORDER` (`context.rs:40-53`):

```rust
pub const SECTION_ORDER: &[&str] = &[
    "task", "repository", "previous_handoff", "known_failures", "decisions",
    "task_memory", "branch_memory", "project_memory",
    "patterns",   // last — the first thing a tight budget drops
];
```

Constants this feature must not change: `CHARS_PER_TOKEN = 3.5`
(`crates/cairn-core/src/budget.rs:16`); `reserve_fraction = 0.40`, i.e.
`reserve = floor(budget_tokens * 0.40)` (`crates/cairn-core/src/context.rs:103-121`,
`Caps::default`); default `context_budget_tokens = 3000`, `min_context_budget_tokens = 600`
(`crates/cairn-core/src/config.rs:90,97-98`), assembled by
`crates/cairnd/src/briefing.rs:19-98` (`briefing::build`), which reads the config, computes
`Caps`, and calls `cairn_core::context::assemble` (`context.rs`).

The mechanics of `Budget` (`crates/cairn-core/src/budget.rs:33-184`) matter for §3:

- `Budget::with_reserve(limit, reserve)` withholds `reserve` from `general_remaining()`
  (`budget.rs:104-108`).
- `try_spend_reserved` (Level 0 only) draws the reserve first, then the general pool
  (`budget.rs:129-138`).
- `release_reserve()` (`budget.rs:143-146`) zeroes the withheld amount — whatever Level 0
  did not spend returns to the general pool, which Level 1's `try_spend` then draws from.
- `try_spend` (`budget.rs:119-125`) refuses anything beyond `general_remaining()` — it can
  never touch a still-withheld reserve.

## 2. The structural guarantee: global cannot enter the reserve

**D420, FR-473.** `assemble`'s reserve computation is:

```rust
// crates/cairn-core/src/context.rs:121
let reserve = (budget_tokens as f64 * input.level0.caps.reserve_fraction).floor() as usize;
```

This expression's only inputs are `budget_tokens` (a `usize` argument) and
`caps.reserve_fraction` (a compile-time-configured `f64`, `0.40`). Neither a personal nor a
team fetch result appears anywhere in `Level0` (`context.rs:76-89`) or in this line. **The
global fetch is not called during reserve computation** — it is not merely filtered out of
the reserve's *content*, it is a function this feature never invokes before or during the
line that computes the reserve size. There is therefore no arithmetic expression anywhere
that could admit a global byte count into the number `reserve` — not a bug that must be
avoided by discipline, but an absence: the value the reserve formula could read from does not
exist at that point in the call graph. This is the same style of guarantee 003 already
relies on for "no timestamp is compared" in `derive_subject` — the property holds because the
function cannot see the thing that would violate it (`crates/cairn-core/src/knowledge.rs:
10-17`).

Personal and team fetches happen later, inside Level 1 admission (§4), by which point Level 0
has already been charged and `release_reserve()` has already run. There is no code path in
which a personal or team byte is charged via `try_spend_reserved`.

## 3. `global_share_max` — the second cap

**D421, D449, D450, FR-474, FR-584.** `global_share_max = 0.15`, a new `cairn-core` constant
alongside `reserve_fraction`. The value is pinned rather than left to design: an unnamed
"documented fraction" is an unimplementable requirement, because two implementations could both
claim conformance and no test could be written against either (D450). Global sections
(`personal_notes`, `team_guidance` — §4) are bounded by **two** independent limits, both of
which must hold:

```text
global_cap    = floor(total_budget_tokens * global_share_max)
global_spend  = min(remaining_non_reserve, global_cap - already_spent_by_global)
```

`global_cap` is fixed the moment the budget is known and does not depend on how much any other
section spent. `remaining_non_reserve` is the part that needs care, and it is **not** simply
`Budget::general_remaining()`.

**Released reserve is not available to global sections (D449, FR-584).** `release_reserve()`
(§1) zeroes the withheld amount once Level 0 has taken what it needs, returning the unspent
remainder to the general pool. After that call, `general_remaining()` includes tokens that were
reserved for critical project state and simply not needed. Global sections may not spend them.
`remaining_non_reserve` is therefore the general pool measured **net of whatever
`release_reserve()` returned** — the pool as it would have been had the reserve been spent in
full — not the pool as it stands.

The reason is Principle VIII rather than arithmetic. A project with little critical state
releases most of its 40%; if global sections could draw on that, exactly the projects with the
least established truth of their own would hand the largest share of their briefing to
project-independent guidance. That is the non-displacement failure wearing a budget's clothes
instead of a scope's. Released reserve returns to *project* ranked content, which is what it was
withheld for in the first place.

The two terms bind in different situations, which is why stating one is not enough: the `0.15`
fraction binds when the non-reserve pool is roomy, and the pool binds when project content has
nearly filled it. `SC-451` asserts global spend against the non-reserve pool alone, so an
implementation that reads `general_remaining()` after the release fails it; `SC-419` asserts the
fraction and the overall `estimated_tokens <= budget` invariant, which continues to hold under
both terms (D456/E3, re-derived rather than re-asserted).

**Where project sections consume the entire remainder, global contributes nothing** (FR-475):
if `remaining_non_reserve` is `0`, `global_spend` is `min(0, anything) = 0` regardless of
`global_cap`, regardless of how much personal or team content exists, and regardless of how
large a reserve was just released. The assembled briefing in that case is byte-identical to what
`assemble` would produce with no personal or team knowledge fetched at all, because no personal
or team `try_spend` call ever succeeds (SC-418).

**An importance hint changes none of this** (FR-482). A hint on a personal or team item alters
neither its section's position in the priority order (§4) nor its eligibility for reserved
context (§2) — the reserve is unreachable by construction, and the order is fixed at compile
time, so there is no expression a hint feeds into. `SC-464` asserts the assembled context is
byte-identical across every supported hint value, which is the falsifiable form: a hint that
began to matter would change some byte.

## 4. Section priority — the full order

**D422.** Appended to the existing order, at the end:

```text
task > repository > previous_handoff > known_failures > decisions >
task_memory > branch_memory > project_memory > patterns >
personal_notes > team_guidance
```

`personal_notes` and `team_guidance` are two new entries in `SECTION_ORDER`
(`crates/cairn-core/src/context.rs:40-53`), both after `patterns` — the least authoritative
project-scoped content still outranks the most relevant global content, because global
knowledge was never observed against *this* project at all.

**Personal before team, always (FR-476).** The specificity gradient the existing order
already expresses —
`task (this exact unit of work) > branch > project > *(now)* personal > team` — stays
monotone: personal knowledge is specific to the one account making the request; team
guidance is the server-wide default with no actor-specific claim at all. Where both compete
for the same remaining tokens, personal is admitted first, exactly as `task_memory` outranks
`branch_memory` today for the same reason (narrower beats broader when both are otherwise
eligible).

`SC-462` verifies this at the boundary that actually distinguishes the two orderings: a
personal item and a team item both eligible, and only enough room for one. Asserting that
personal appears *somewhere before* team in a briefing where both fit proves only that the
list is sorted; asserting which one survives when one must be dropped proves the order is
load-bearing.

## 4a. Selection reasons are explain-only, by construction (D451, FR-478)

Every personal or team item considered gets a selection reason from the vocabulary Feature 003
already defines for project sections — `applicability_match`, `budget_exhausted`,
`depth_excluded` and the rest — rather than a second vocabulary invented for this feature. The
requirement then says those reasons must never appear in the rendered briefing, and as
originally written that was two obligations reconcilable only by a convention: produce them,
then remember not to render them.

They are instead **explain-only, enforced by the type**. Reasons are produced on the diagnostic
path, returned in the structured `--explain` output, and the rendered-briefing type carries no
field they could occupy. A renderer cannot leak them by forgetting to omit them, because there
is nothing to omit — the same argument §2 makes for the reserve, applied to a different value.

`SC-463` asserts both halves: present in the diagnostic output, absent from the rendered form
inspected field by field. A reason reaching the briefing fails the test rather than being caught
in review.

## 5. `depth: "minimum"` excludes global entirely (D423, FR-477)

**This is also where this feature must finish wiring a field that exists but does nothing
today.** The MCP tool schema already declares `depth` with values `minimum | standard`
(`crates/cairn/src/mcp.rs:129`, description: "`minimum` is Level 0 only. Level 2 is never
automatic") — but the dispatch that builds the daemon request never forwards it:

```rust
// crates/cairn/src/mcp.rs:341-366 — the daemon call this feature must extend
let value = client::send(&Request::Context {
    cwd, agent_session_key: key, session_id: uuid_arg(args, "session_id").ok(),
    reason: args.get("reason")...,
    token_budget: args.get("token_budget")...,
    explain: false,
    // depth: absent — `Request::Context` carries no `depth` field today
})
```

`depth` is accepted by the JSON schema, described in its own text, and then silently dropped
before it reaches the daemon. This feature adds a `depth` field to `Request::Context`,
threads it through `crates/cairnd/src/handlers.rs:1332` (`context`) into
`crates/cairnd/src/briefing.rs:19` (`build`), and uses it for exactly one purpose: at
`depth = "minimum"`, `personal_notes` and `team_guidance` are never fetched and never
admitted, unconditionally. No configuration flag, importance hint, or explicit request
overrides this (FR-477) — the gate is a plain `if depth == Minimum { skip both sections }`
before either domain's search runs, not a budget outcome that happens to be zero. This is
deliberately a narrower repair than rewiring `depth` for every Level 0/1 distinction the
schema's own description implies; this feature wires only the one behavior FR-477 requires,
and existing project-section behavior under `depth: "minimum"` is unchanged (i.e., still
whatever it already is today, which does not distinguish minimum from standard for project
sections, because no requirement in this feature asks it to).

## 6. Worked budget arithmetic

Four examples, each showing `estimated_tokens <= budget` holds and demonstrating a different
interaction. "Spend" figures are illustrative round numbers in Cairn-estimated tokens, chosen
to make the arithmetic legible.

### Example A — default budget, everything fits

`budget = 3000`, `reserve = floor(3000 * 0.40) = 1200`, `global_cap = floor(3000 * 0.15) = 450`.

| Step | Spend | Running total | Source pool |
|---|---|---|---|
| Header + Level 0 (task, warnings, blockers) | 350 | 350 | reserve (1200 withheld, 350 used, 850 released) |
| `repository` … `project_memory` … `patterns` | 2000 | 2350 | general pool (now 3000 − 350 = 2650 available) |
| `personal_notes` | 200 | 2550 | general pool; `general_remaining = 2650 − 2000 = 650`; `global_cap = 450`; personal spend = `min(200, 450) = 200` |
| `team_guidance` | 100 | 2650 | general pool; remaining global allowance `450 − 200 = 250`; team spend = `min(100, 250) = 100` |

`estimated_tokens = 2650 <= 3000`. Both global sections fully included because neither the
remainder (650) nor the proportional cap (450) bound them.

### Example B — minimum budget, the global cap binds tightly

`budget = 600` (the documented floor), `reserve = floor(600*0.40) = 240`,
`global_cap = floor(600*0.15) = 90`.

| Step | Spend | Running total | Note |
|---|---|---|---|
| Header + Level 0 | 100 | 100 | reserve withheld 240, used 100, 140 released |
| `repository` … `patterns` (tight budget, most content trimmed) | 450 | 550 | general pool `= 600 − 100 = 500`; after spending, remaining `= 50` |
| `personal_notes` (30 tokens of matching content) | 30 | 580 | `min(50, 90) = 50` available; 30 fits |
| `team_guidance` (30 tokens wanted) | 0 | 580 | remaining after personal `= 50 − 30 = 20`; global allowance left `= 90 − 30 = 60`; actual available is `min(20, 60) = 20`, and 30 does not fit — omitted, `reason: budget_exhausted` |

`estimated_tokens = 580 <= 600`. Below the documented minimum the briefing is still produced
and never rejected for size (existing rule, `context.rs` module doc), and that holds
identically with global sections present.

### Example C — project sections consume the entire remainder

`budget = 3000`, `reserve = 1200`, `global_cap = 450`.

| Step | Spend | Running total | Note |
|---|---|---|---|
| Header + Level 0 (large task: many criteria, blockers, warnings) | 1200 | 1200 | reserve fully used via `try_spend_reserved` spending beyond the withheld amount; nothing released because nothing was withheld to release |
| `repository` … `patterns` (large project memory, patterns present) | 1800 | 3000 | general pool `= 3000 − 1200 = 1800`; fully spent |
| `personal_notes` | 0 | 3000 | `general_remaining = 0`; `min(0, 450) = 0` |
| `team_guidance` | 0 | 3000 | same |

`estimated_tokens = 3000 <= 3000`. The briefing is byte-identical to what `assemble` would
produce with no personal or team knowledge fetched at all (FR-475) — not because the content
was ranked low and lost a tiebreak, but because the pool it would have drawn from was empty
before either domain's fetch was even worth calling.

**This example, and SC-418's verification, require global records to actually exist in the
store — an empty global store proves nothing.** The property being demonstrated is that
project-priority sections fully consuming the budget leaves *zero room* for global content
that is otherwise present and would have been admitted if there had been space. A test that
runs Example C against a store holding no `personal_knowledge`/`team_knowledge` rows at all
would pass identically whether or not this contract's budget arithmetic (§2, §3) works
correctly, because `personal_notes`/`team_guidance` would spend `0` either way — from having
nothing to spend, not from being correctly refused space. The only test that actually
exercises FR-475/SC-418 seeds the store with matching `personal_knowledge` and
`team_knowledge` content for the requesting user and project *before* filling the budget, so a
regression that let global content leak into an exhausted budget would be visible as a
non-zero, incorrect spend rather than being masked by an empty fetch.

### Example D — the proportional cap binds before the remainder does

`budget = 3000`, `reserve = 1200`, `global_cap = 450`.

| Step | Spend | Running total | Note |
|---|---|---|---|
| Header + Level 0 (light task) | 200 | 200 | 1000 released from reserve |
| `repository` … `patterns` (small project, little memory) | 500 | 700 | general pool `= 3000 − 200 = 2800`; remaining after `= 2300` |
| `personal_notes` (300 tokens of matching content — all fits within remainder) | 300 | 1000 | `general_remaining = 2300`, ample; `global_cap = 450`; personal spend `= min(300, 450) = 300` |
| `team_guidance` (400 tokens wanted) | 150 | 1150 | remaining global allowance `= 450 − 300 = 150`; even though `general_remaining` is now `2000`, plenty, the proportional cap admits only `150`; the rest is omitted, `reason: cap_reached` |

`estimated_tokens = 1150 <= 3000`. This is the case that demonstrates `global_share_max`
actually binding independently of leftover space: 2000 tokens of general pool remained
unspent, and team guidance was still truncated, because the 0.15-of-total-budget ceiling is
a property of the *budget*, not of how generous the remainder happens to be.

## 7. The search contract

**FR-469, FR-470, FR-472, D424.** `SearchPayload` (`crates/cairn-core/src/wire.rs:1304-1308`)
is **unchanged**:

```rust
pub struct SearchPayload {
    pub results: Vec<MemoryResult>,
    pub total: usize,
}
```

`results[]` and `total` continue to describe **project** results only — `total` is not
redefined to mean "everything returned across all three domains." This feature adds two new
**sibling** top-level fields to the response envelope, spliced in by the handler exactly as
`patterns[]` already is (`crates/cairnd/src/handlers.rs:2280-2333`, which builds
`SearchPayload{results, total}` and then, only `if include_patterns`, inserts a separate
`"patterns"` array into the serialized object — never into `results`):

```json
{
  "results": [ ... ],
  "total": 7,
  "personal": [ { "id", "content", "topic_key", "value_key", "created_at", "applicability" } ],
  "team": [ { "id", "content", "topic_key", "value_key", "state", "applicability" } ]
}
```

Each result's `applicability` array holds `(kind, value)` pairs drawn from the closed
two-member vocabulary `language | tool` (`global-memory.md` §4, D439) — the same vocabulary
governs both `personal[]` and `team[]` entries, and neither a record's own `topic_key` nor any
third applicability kind ever appears in this field (`global-memory.md` §4's `topic_key`
distinction, FR-570).

`personal[]` and `team[]` are **never merged into `results[]`** — the same reasoning
`patterns[]` was built on ("a pattern is not this project's knowledge,"
`crates/cairn-core/src/wire.rs:1425-1430`) applied to two more domains. `total`'s value is
computed exactly as today, from `results.len()` before either sibling array exists, so every
existing assertion against `total` continues to hold unmodified (FR-470).

### `domains` filter

`MemoryQuery` gains an optional `domains: Vec<KnowledgeDomain>` field, defaulting to all
three (`project`, `personal`, `team`) when absent (FR-472). A caller who passes
`domains: ["project"]` gets `personal: []` and `team: []` back rather than the keys being
omitted — the shape is always present so a client does not need to branch on whether the
fields exist, only on whether they are empty.

## 8. Per-domain FTS5, and why no cross-domain comparator exists

**D425, FR-471.** Two new SQLite virtual tables, `personal_fts` and `team_fts`, each
mirroring the existing `memory_fts` construction (`0002_memory_fts.sql:6-26`: FTS5
external-content table plus `_ai`/`_ad`/`_au` triggers keeping it in sync with its base
table) — one over `personal_knowledge.content`, one over `team_knowledge.content`.

Ranking within each domain reuses the identical BM25 expression project search already uses:

```sql
-- crates/cairn-store/src/search.rs:36-41, the existing project query
SELECT m.*, ..., -bm25(memory_fts) AS relevance
  FROM memories m JOIN memory_fts ON memory_fts.rowid = m.rowid
 WHERE memory_fts MATCH ?
```

applied verbatim in shape against `personal_fts`/`personal_knowledge` and
`team_fts`/`team_knowledge`, each producing its own `relevance` column, ordered within its
own result set.

**No comparator exists that ranks a project result against a personal or team result, or a
personal result against a team one, by relevance.** This is not an oversight to be filled in
later — it is refused deliberately: BM25's score is a function of term statistics *within one
corpus* (document frequency, average document length, the specific set of documents the index
was built over). A score of `4.1` in `personal_fts` and a score of `4.1` in `memory_fts` do
not mean "equally relevant" — they mean "equally well-matched against two different, unrelated
populations of documents," which is not the same claim. Inventing a normalization to make them
comparable (min-max scaling, z-scores against each corpus's score distribution) would be
introducing a heuristic exactly where Constitution II already forbids one, to produce a
number whose only property would be looking comparable. §7's three-array shape exists
precisely so nothing downstream is ever tempted to sort the three together.

**And "refused deliberately" is not enough on its own** (SC-468). Everything above is a
rationale, and a rationale is exactly what a later contributor who has not read it will
override. So each domain's ranking input is a **distinct type** carrying only its own
`relevance` — there is no type in which a project score and a personal score coexist, and no
function that accepts both. A cross-domain comparison is not merely discouraged; it does not
compile. That is the same argument this feature uses everywhere it can reach it: the value is
visible, and the function permitted to decide anything is not allowed to see the other one.

## Invariants

1. `reserve = floor(budget_tokens * reserve_fraction)` reads no value derived from personal
   or team knowledge, at any point in its computation (D420, FR-473).
2. No personal or team byte is ever admitted via `try_spend_reserved`; personal and team
   sections call only the ordinary `try_spend` used by every other Level 1 section (D420).
3. `global_cap = floor(total_budget_tokens * 0.15)` is fixed once the budget is known and
   does not vary with how much any other section spends (D421).
4. Combined personal-plus-team spend never exceeds `min(remaining_non_reserve, global_cap)`
   (FR-474, FR-584). `remaining_non_reserve` excludes whatever `release_reserve()` returned to
   the general pool: a reserve Level 0 did not need is not thereby space global sections have
   earned (D449, SC-451).
5. When project-priority sections consume the entire general pool, personal and team
   sections spend exactly zero and the assembled briefing is byte-identical to one produced
   with no personal or team knowledge fetched at all (FR-475) — verified with matching
   personal and team content actually present in the store, never against an empty global
   store, which would prove nothing about this property (see Example C's note, §6).
6. Wherever personal and team sections compete for the same remaining space, personal is
   admitted first (FR-476).
7. `depth: "minimum"` excludes `personal_notes` and `team_guidance` unconditionally; no
   field, importance hint, or budget outcome can re-admit either at that depth (FR-477).
8. `estimated_tokens <= budget` holds for every assembled context, with or without personal
   or team content present (FR-480 — unchanged property of `Budget`, extended to the new
   sections by construction rather than by a new check).
9. A caller with no personal or team knowledge of their own receives an assembled context
   and a project search result identical, byte for byte, to one produced before this
   feature existed (FR-481).
10. An importance hint on a personal or team item changes neither its section's position in
    `SECTION_ORDER` nor its eligibility for the reserve (FR-482, SC-464).
10a. A selection reason for a personal or team item exists only on the diagnostic path; the
    rendered-briefing type has no field capable of holding one (FR-478, D451, SC-463).
10b. No type carries a relevance score from two domains at once, and no function accepts two
    domains' scores, so a cross-domain relevance comparison does not compile (FR-471, D425,
    SC-468).
11. `results[]` and `total` in a search response describe project results only, computed
    before `personal[]`/`team[]` are considered, exactly as `patterns[]` never contributes
    to either today (D424, FR-469, FR-470).
12. No code path computes a single ranked list, or a single relevance number, spanning more
    than one domain's FTS5 index (D425, FR-471).
13. Every `applicability` entry in `personal[]`/`team[]` search results is a `(kind, value)`
    pair from the closed two-member vocabulary `language | tool`; a record's own `topic_key`
    is a separate field, never conflated with applicability (D439, FR-570).
