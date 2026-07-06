---
title: "Finding: promo_score can never exceed 0.36 - promotion candidates, LLM tie-break, and full-auto promotion are all structurally unreachable"
type: finding
status: open
updated: 2026-07-03
severity: high
---

# Finding: promo_score can never exceed 0.36 - promotion candidates, LLM tie-break, and full-auto promotion are all structurally unreachable

**Flow:** 33-v0.8.0-llm-intelligence (discovered), also affects 35-v0.8.0-dashboard-promotion-panel and 36-v0.8.0-autopilot
**Severity:** high (not a crash or data-loss risk, but the entire automatic promotion pipeline
that Sprint 5 built and Sprint 8's autopilot depends on never fires against real data)
**Discovered:** 2026-07-03

## What happened
`run_promotion_scoring` (`crates/cairn-memory/src/lib.rs`) computes every memory's `promo_score`
as:

```rust
let mut score = llm_intelligence::fast_promotion_score(m.kind, cross_hits);
// score = kind_prior_score(kind) * 0.4 + cross_score * 0.6
// cross_score = (cross_project_hits / 3.0).min(1.0)
```

`cross_project_hits` comes from `Store::count_cross_project_access`, which counts `access_log`
rows where `project_id != <the memory's own project>`. There is exactly one place `access_log`
rows are ever written - `INSERT INTO access_log $rows` inside `record_access_batch`
(`crates/cairn-store/src/surreal.rs:736`), called from exactly one place -
`cairn-memory`'s `recall_for_org` (`crates/cairn-memory/src/lib.rs:395-411`), and *only* for
memories that already survived its own scope filter (`crates/cairn-memory/src/lib.rs:216-224`):

```rust
ScopeType::Project => scope.project_id.is_some() && m.scope_id == scope.project_id,
```

A `Project`-scoped memory is only ever recalled - and therefore only ever logged - under its
*own* project's context. There is no code path anywhere that recalls (and thus logs access to) a
`Project`-scoped memory from a *different* project. So every `access_log` row for such a memory
always has `project_id == its own scope_id`, `count_cross_project_access`'s `project_id !=
$exclude` condition can never match, and `cross_project_hits` is **always 0** for every memory
`run_promotion_scoring` will ever look at.

With `cross_project_hits = 0`, `cross_score = 0`, so `fast_score = kind_prior_score(kind) * 0.4`
exactly. The highest `kind_prior_score` is `Fact`'s `0.9`, giving a hard ceiling of **`0.36`** on
every `promo_score` that can ever be organically computed. That ceiling sits below *every*
threshold the rest of the pipeline depends on:

| Threshold | Value | Reachable? |
|---|---|---|
| LLM tie-break band (`run_promotion_scoring`) | `[0.40, 0.85]` | No - 0.36 < 0.40 |
| Human-review candidates (`promotion_candidates`) | `[0.70, 0.90]` | No |
| Full-auto promotion (`CAIRN_PROMOTE_THRESHOLD`, or the Sprint 9 self-tuned override) | `0.85` default, `[0.5, 0.95]` tunable range | No - even the tuner's floor of `0.5` is above the 0.36 ceiling |

Confirmed empirically in `docs/testing/live-e2e/33-v0.8.0-llm-intelligence.md` Step 3: a live
`Project`-scoped `Semantic` `Fact` scored via a real `llm-intelligence` run came back with
`promo_score: 0.35999998` - the pure, unblended `fast_score`, proving no downstream branch ever
engaged.

**Practical impact**: `GET /api/memory/promotion-candidates` will be empty forever in a real
deployment (no memory can reach `[0.70, 0.90]`), `run_auto_promote` will never promote anything
(no memory can reach `0.85`, or even the self-tuner's floor of `0.5`), and the LLM-judgment
refinement step - along with the Sprint 9 budget guard specifically protecting it - never
executes. Manual promotion (`POST /api/memory/:id/promote`, which doesn't consult `promo_score`
at all) is unaffected and still works as a direct action.

## Expected
A `Project`-scoped memory that's genuinely useful across projects should be able to accumulate
enough signal to cross into the review band and, eventually, the auto-promote threshold.

## Actual
No signal-accumulation path exists that a `Project`-scoped memory can ever actually traverse -
`promo_score` is capped at `0.36` for 100% of real memories, regardless of how long they exist
or how often they're used within their own project.

## Suggested fix
This needs a design decision, not a one-line patch, since the underlying signal
(`count_cross_project_access`) is unreachable *by design* (Sprint 2's scope isolation
intentionally prevents a `Project`-scoped memory from being visible outside its project - that's
not itself a bug). Two directions worth considering:
1. Log a signal Sprint 2's isolation *doesn't* block - e.g. `access_count`/reinforcement
   frequency within the memory's own project - and rebase `fast_promotion_score` on that instead
   of `cross_project_hits`.
2. Introduce a real cross-project signal deliberately, e.g. count how often a *similar*
   `Global`-scoped memory (or a differently-scoped memory with overlapping content) gets
   recalled from other projects, as a proxy for "this kind of knowledge is wanted elsewhere."

Either way, the current `[0.40, 0.85]`/`[0.70, 0.90]`/`0.85` threshold values were tuned against
a `fast_score` formula whose ceiling is `0.36` - fixing the signal alone would still need the
thresholds re-validated against real score distributions.
