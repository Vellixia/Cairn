# Contract: Continuity and Context

**Feature**: `003-project-intelligence`

Two things that must hold together: what an agent keeps across a compaction boundary, and how the
budget is spent so the important part survives.

```text
LEVEL 0  minimum safe continuity   ── reserved budget the lower levels cannot take
LEVEL 1  relevant current knowledge ── the remaining budget, ranked
LEVEL 2  history and evidence       ── never automatic; explicit request only
```

## Part 1 — Continuity checkpoints

### What a checkpoint is

A checkpoint is **derived work state plus the assumptions it was taken under**. It is not a summary
of conversation, and it does not depend on any provider's compression quality (FR-421).

It anchors to the handoff Cairn already derives at that boundary (D55, FR-423). The handoff carries
goal, progress, completed work, remaining work, changed files, decisions, failures, tests and next
step. The checkpoint adds only what the handoff cannot:

| Checkpoint field | Purpose |
|---|---|
| `handoff_id` | Everything the handoff already derives, by reference |
| `assumed_branch`, `assumed_commit` | Staleness detection |
| `assumed_task_id`, `assumed_task_revision` | Staleness detection |
| `relevant_paths` (≤32) | Staleness detection — the changed and read paths of this session |
| `criteria_snapshot` | Criterion states at that instant, for the divergence diff |
| `open_blockers` | Blockers in force |
| `pinned_constraints` | Pinned memory ids in force |
| `next_action` | Derived, bounded |
| `restore_count`, `restored_at` | Evidence for the ten-compaction test |

### When one is written

| Boundary | Canonical event | Written? |
|---|---|---|
| Pre-compaction | `context_compacting` | yes — where the adapter provides the event |
| Session close | `session_closed` | yes |
| Explicit | `cairn session checkpoint` / `cairn_session action=checkpoint` | yes |
| Turn checkpoint | `agent_quiesced` | **no** — it is a turn boundary, not a work boundary (Feature 001 D16) |
| Post-compaction | `context_compacted` | no; this is where a checkpoint is **restored** |

Writing a checkpoint reuses the sealed-close discipline where it applies: at `session_closed` the
checkpoint is written inside the same synthesis step that produces the handoff, so a boundary that
owes a handoff owes its checkpoint with it and the existing sweep covers both (Feature 002 D22).

### Continuity mode — derived, honest

No new canonical event and no new capability (FR-427). The mode is a **derived read** over
Feature 002's capability profile (D57):

| `LifecyclePreCompaction` | `LifecyclePostCompaction` | `continuity_mode` | What Cairn says |
|---|---|---|---|
| present | present | `automatic` | Continuity is restored automatically after compaction |
| present | absent / conditional | `agent_initiated` | A checkpoint is written before compaction; call `cairn_context(reason=post_compaction)` to restore it |
| absent | any | `unavailable_automatic` | Compression-safe continuity is not automatic for this agent; a checkpoint exists at session close and on demand |

Under currently verified vendor behaviour: Claude Code `automatic`, Codex `automatic`, OpenCode
`agent_initiated` (its compaction hook is experimental), generic MCP `unavailable_automatic`. These
are outputs of the rule, not entries in a maintained table.

Reported by `cairn agents`, `cairn doctor` and in the `cairn_context` response. Cairn never reports a
rehydration guarantee an adapter cannot provide (FR-426, SC-311 companion).

### Restoration and staleness

On restore, compare the assumption set against current state (FR-431):

| Divergence | Comparison | Source |
|---|---|---|
| `branch` | `assumed_branch` vs current branch | `cairn-git` |
| `commit` | `assumed_commit` vs current head | `cairn-git` |
| `task` | `assumed_task_revision` vs `tasks.revision` | store; `task_changes` supplies the diff |
| `files` | a `relevant_paths` entry with a `file_changed` observation from **another session**, `occurred_at > created_at` | store |

Outcome:

| State | Condition |
|---|---|
| `current` | no divergence |
| `diverged` | one or more divergences |
| `unresolvable` | the assumed task or the worktree no longer exists |

`unresolvable` still delivers every continuity field that does not depend on the missing state
(FR-435).

### What a diverged checkpoint must say

The recorded next action is emitted as **`previous_next_action`**, never as `next_action`
(FR-434). Rendered:

```text
⚠ CHECKPOINT DIVERGED
  recorded at commit abc123 on main, task revision 7
  current:        commit def456 on main, task revision 8
  task changed:   criterion AC-3 added; blocker "staging credentials" opened
  files changed by another session: src/config.rs (session 0192f4…, claude-code)
  previous next action (may be stale): "finish the retry backoff in config.rs"
```

The recorded action appears, because throwing it away loses information; it appears **labelled**,
because presenting it as the instruction is the failure mode the brief names (US6 #2, SC-311).

### Repeated compaction

Every restoration increments `restore_count`. Each new `context_compacting` writes a **new**
checkpoint anchored to that boundary's handoff — checkpoints are append-only, so ten cycles leave ten
records and the tenth restoration reads the tenth. The fields in FR-422 that exist in recorded state
are present after every cycle, because they are derived from the store each time rather than copied
forward from the previous checkpoint (FR-428, SC-310).

Nothing is carried in the conversation, so nothing degrades with each pass. That is the whole point.

## Part 2 — Layered context

### Level 0 — minimum safe continuity

Contents (FR-443), and the admission order applied when even the reserve binds (FR-446):

| # | Item | Notes |
|---|---|---|
| 1 | Task identity, goal, status | The frame everything else hangs on |
| 2 | `next_action`, or `previous_next_action` + divergence statement | Never both |
| 3 | Open blockers | |
| 4 | Critical warnings | Order: checkpoint divergence → task divergence → conflict → drift. Cap `warnings_in_context_max` (5) |
| 5 | Pinned constraints in force | Cap `pins_in_context_max` (4) |
| 6 | Repository state | Feature 001's `repository` section |
| 7 | Acceptance criteria with states, then derived progress | The largest item; last so a tiny budget still gets 1–6 |

### Level 1 — relevant current knowledge

Feature 001's sections, plus what Feature 003 adds:

| Item | Cap |
|---|---|
| Previous handoff remainder (remaining work, changed files) | Feature 001's behaviour |
| Known failures, decisions | Feature 001's behaviour |
| Canonical answers for applicable subjects, verified first | `MEMORY_PER_SCOPE` (12) per scope |
| Task, branch, project scoped memory, scope-first ranked | Feature 001's behaviour |
| Applicable reusable patterns, labelled unverified here | `patterns_in_context_max` (2) |
| `remote_verification` notes | within the warning cap |

### Level 2 — on demand only

Never in the automatic briefing (FR-444). Reached by explicit request:

| Item | How |
|---|---|
| Superseded chains, historical answers | `cairn_search state=superseded`, `as_of` |
| Verification run history | `cairn verify --explain` |
| Evidence facts and their values | `cairn evidence show` |
| Full pattern text and applications | `cairn pattern show` |
| Selection diagnostics | `cairn_context explain=true`, `cairn context --explain` |
| Task change log | `cairn task history` |

### The budget reserve

```rust
// cairn-core/src/budget.rs
impl Budget {
    pub fn with_reserve(limit: usize, reserve: usize) -> Self;
    pub fn try_spend_reserved(&mut self, cost: usize) -> bool;  // reserve first, then general
    pub fn release_reserve(&mut self);                          // unspent reserve → general pool
}
```

Rules:

1. `reserve = floor(limit * min_safe_context_fraction)`, default fraction `0.40`.
2. Level 0 admissions call `try_spend_reserved`: reserve first, then the general pool.
3. `release_reserve()` runs once Level 0 is complete; **unspent reserve returns to the general
   pool** (FR-442). A project with no task, no warnings and no pins therefore delivers exactly what
   it delivers today — the reserve is a cap on the lower levels, not a floor Level 0 must spend.
4. Level 1 and Level 2 admissions call the existing `try_spend`, which now sees only the general
   pool.
5. `try_spend` keeps measure-before-emit, so `estimated_tokens <= budget` remains a property of the
   loop, not a statistic (FR-445, I16). The existing 5,000-memory property test is extended to every
   level.
6. Below `min_context_budget_tokens` (600) the briefing is still produced, truncated in Level 0's
   admission order. It is never rejected for size.

### Selection reasons

Every admitted item carries reasons from a closed enum; every omission carries one (FR-461):

```text
reasons:   scope_match | canonical_answer | verified | pinned | drift_warning |
           conflict_warning | pattern_signal_match | checkpoint_assumption | task_binding
omissions: budget_exhausted | scope_mismatch | superseded | not_canonical |
           level_2_only | pin_budget | cap_reached
```

Returned only when `explain: true`; it costs no budget otherwise (FR-463). Shape:

```json
{
  "selection": {
    "budget": 3000, "reserve": 1200, "reserve_used": 512, "reserve_released": 688,
    "included": [
      { "level": "minimum_safe", "kind": "task", "id": "…",
        "reasons": ["task_binding"], "cost": 96 },
      { "level": "minimum_safe", "kind": "warning", "id": "conflict:infrastructure.production_database",
        "reasons": ["conflict_warning"], "cost": 41 },
      { "level": "relevant", "kind": "memory", "id": "…",
        "reasons": ["scope_match", "canonical_answer", "verified"],
        "rank": { "scope_bucket": 0, "relevance": 8.41, "verification_rank": 0, "importance": "normal" },
        "cost": 24 }
    ],
    "omitted": [
      { "kind": "memory", "id": "…", "reason": "budget_exhausted" },
      { "kind": "memory", "id": "…", "reason": "not_canonical" },
      { "kind": "pattern", "id": "…", "reason": "cap_reached" }
    ]
  }
}
```

Warnings are Level 0 **content**, not diagnostics: they appear in the briefing whether or not
`explain` is set (FR-464).

`cairn context --explain` renders the same data as a table, which is the answer to "why did Cairn
tell the agent this?" (FR-462).

## Part 3 — Pinned invariants

| Rule | Value / behaviour |
|---|---|
| Who may pin | A user, always (`cairn memory pin`). An agent, within budget (`cairn_remember action=pin`) |
| Budget | `pin_budget_project` 12; `pin_budget_per_scope` 4 |
| Over budget | Refused with `pin_budget_exhausted`, listing current pins. Nothing auto-unpinned (FR-454) |
| Scope | A pin never widens scope. A pinned `branch:feature/x` memory is in force only on that branch (FR-453) |
| In context | `pins_in_context_max` 4, applicable pins only, ordered by scope precedence then importance |
| On supersession | The predecessor's pin is cleared in the same transaction. The successor is pinned only explicitly (FR-456) |
| On drift | The pin is kept and the drift warning travels with it — a constraint that no longer holds is exactly what must be said (FR-456) |
| On `local_only` | Stays local. The `pinned` flag may sync for a shareable memory; no additional content leaves (FR-457) |
| Unpin | `cairn memory pin --off`, `cairn_remember action=pin pinned=false` |

Rendered in Level 0:

```text
CRITICAL CONSTRAINTS
  • published Skill refs are immutable — never move an existing skill-release branch
  • never mutate CC Switch's private database directly
  ⚠ • the API listens on 8080  (drifted: config/app.yml now says 9000)
```

## Error codes

| Code | Meaning |
|---|---|
| `pin_budget_exhausted` | The project or scope pin budget is full; current pins are listed |
| `checkpoint_not_found` | No checkpoint exists for the session |
| `checkpoint_unresolvable` | The assumed task or worktree no longer exists; partial continuity returned |
| `no_boundary_record` | A checkpoint was requested with no handoff to anchor to; one is derived first |

`checkpoint_unresolvable` is `ok: true` with a note — partial continuity is a result, not a failure
(FR-435).
