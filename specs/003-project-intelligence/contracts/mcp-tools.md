# Contract: MCP Tool Surface — Feature 003 Extension

**Feature**: `003-project-intelligence`

**Still exactly six tools.** Feature 001 fixed the surface at six (FR-040); Feature 002 reaffirmed it
(FR-128); Feature 003 adds none (FR-495). The existing tests
`exposes_exactly_six_tools` and `the_surface_is_still_exactly_six_tools` remain the gate.

```text
cairn_context   cairn_search   cairn_remember   cairn_session   cairn_task   cairn_handoff
```

**Not added, and each one was considered**: `cairn_verify`, `cairn_evidence`, `cairn_pattern`,
`cairn_subject`, `cairn_checkpoint`. Every one of them is an action on a tool that already exists.

## Compatibility rule

A call carrying only Feature 001 arguments behaves exactly as it does today, plus new **read-only**
fields in the response (FR-496, FR-497). No existing parameter changes meaning, no existing action
changes behaviour, no existing response field is removed or retyped.

Asserted by `mcp_backward_compatibility`: a recorded corpus of Feature 001/002 tool calls replayed
against the Feature 003 server, comparing every pre-existing response field for equality.

---

## `cairn_context`

### Added input

| Field | Type | Default | Notes |
|---|---|---|---|
| `explain` | bool | `false` | Return the `selection` object. Costs no budget when false (FR-463) |
| `depth` | enum | `standard` | `minimum` = Level 0 only; `standard` = Levels 0 and 1. Level 2 is never automatic |
| `include_patterns` | bool | `true` | Signal-matched reusable patterns in Level 1, capped at 2 |
| `reason` | enum | `refresh` | **Extended** with `post_compaction`. Unknown values still fall back to `refresh` |

`reason=post_compaction` is how an agent whose adapter has no post-compaction event restores
continuity (D57, FR-426). It restores the latest checkpoint, runs staleness detection, and increments
`restore_count`.

### Added output

```json
{
  "ok": true,
  "briefing": { "…": "Feature 001 fields, unchanged" },
  "estimated_tokens": 2840, "budget": 3000, "truncated": true,
  "omitted_sections": ["project_memory"], "degraded": false,

  "minimum_safe": {
    "task": { "id": "…", "goal": "…", "status": "in_progress" },
    "criteria": [{ "id": "…", "label": "AC-1", "text": "…",
                   "state": "satisfied", "verification": "verified" }],
    "progress": { "verified": 3, "satisfied_unverified": 1, "blocked": 1,
                  "pending": 2, "waived": 0, "total": 7 },
    "completion_readiness": "not_ready",
    "open_blockers": ["staging credentials expired"],
    "next_action": "finish the retry backoff in config.rs",
    "previous_next_action": null,
    "pinned_constraints": [{ "id": "…", "content": "…", "verification": "verified" }],
    "repository": { "…": "Feature 001 RepositoryState" }
  },

  "continuity": {
    "mode": "automatic",
    "checkpoint": { "id": "…", "created_at": "…", "restore_count": 3,
                    "state": "diverged",
                    "divergences": [
                      { "kind": "commit",  "recorded": "abc123", "current": "def456" },
                      { "kind": "task",    "recorded": 7, "current": 8,
                        "changes": ["criterion added — AC-4", "blocker opened"] },
                      { "kind": "files",   "paths": ["src/config.rs"],
                        "changed_by_session": "0192f4…", "changed_by_agent": "claude-code" }
                    ] }
  },

  "warnings": [
    { "kind": "checkpoint_divergence", "detail": "recorded at abc123, current def456" },
    { "kind": "conflict", "topic_key": "infrastructure.production_database",
      "answers": [{ "memory_id": "…", "value_key": "postgresql", "agent": "claude-code" },
                  { "memory_id": "…", "value_key": "cockroachdb", "agent": "codex" }] },
    { "kind": "drift", "memory_id": "…", "topic_key": "service.api_port",
      "remembered": "8080", "evidence_locator": "config/app.yml" }
  ],

  "patterns": [
    { "id": "…", "title": "…", "trust": "contested",
      "verified_in_this_project": false,
      "applicability": ["…"], "constraints": ["…"],
      "alternative_causes": ["VPN route collision, not pool exhaustion"] }
  ],

  "selection": { "…": "only when explain=true" }
}
```

`warnings` is Level 0 content and appears whether or not `explain` is set (FR-464). `patterns` always
carries `verified_in_this_project: false` (SC-312).

---

## `cairn_search`

### Added input

| Field | Type | Notes |
|---|---|---|
| `verification` | enum | `unverified \| verified \| needs_recheck \| drifted \| conflicted` |
| `conflicted` | bool | Restrict to memories in a conflicted subject |
| `topic_key` | string | Exact or prefix match (`infrastructure.` matches the subtree) |
| `as_of` | timestamp | Temporal query: what was effective then (FR-342) |
| `include_patterns` | bool | Reusable patterns in a **separate** `patterns` array |
| `pinned` | bool | Restrict to pinned memories |

`state` is unchanged and still defaults to `active`. A `drifted` memory is lifecycle-`active` and is
therefore returned by default, with its verification state visible (FR-373).

### Added output, per result

```json
{
  "id": "…", "type": "fact", "scope": "project", "scope_key": "…",
  "content": "…", "state": "active", "created_at": "…",
  "provenance": { "…": "Feature 001, unchanged" },
  "rank": { "scope_bucket": 2, "relevance": 8.41, "age_days": 12 },

  "topic_key": "infrastructure.production_database",
  "value_key": "postgresql",
  "importance": "normal",
  "pinned": false,
  "verification": { "state": "verified", "origin": "local",
                    "last_verified_at": "…", "fact_count": 2,
                    "basis": ["configuration", "git_ref"] },
  "temporal": { "effective_from": "…", "superseded_at": null },
  "reinforcement": { "count": 3, "distinct_origins": 3 },
  "subject": { "reconciliation": "reinforced", "is_canonical_answer": true,
               "competing_answers": [] }
}
```

`verification.basis` carries **verifier kinds only** — never a subject, value, locator or digest
(FR-502). `reinforcement` is never labelled as verifications (FR-406).

Patterns, when requested, are a separate array — never mixed into `results`, so a caller cannot
mistake a cross-project pattern for project knowledge.

---

## `cairn_remember`

### Actions

| Action | Status | Purpose |
|---|---|---|
| `create` | existing, extended | Record knowledge; now accepts `topic_key`, `value_key`, `importance` |
| `supersede` | existing, extended | Replace; records a `supersedes` relation with an explicit basis |
| `forget` | existing | Unchanged |
| `reinforce` | **new** | Record that this session found an existing memory still true |
| `attach_evidence` | **new** | Attach an evidence fact, with a role |
| `verify` | **new** | Run the deterministic verifier for a memory's evidence now |
| `pin` | **new** | Pin or unpin (`pinned: bool`) |
| `reconcile` | **new** | Record a reconciliation decision: `narrows`, `not_applicable_to`, or resolve a conflict |
| `promote` | **new** | Propose a reusable pattern from a memory |
| `record_outcome` | **new** | Record a pattern application outcome, including a counterexample |

One discriminator per tool. No action takes a sub-operation (D70).

### Added input by action

**`create` / `supersede`**

| Field | Notes |
|---|---|
| `topic_key` | Normalized; a key that fails normalization yields `ok: true` with note `invalid_topic_key` and the memory is stored free-form (FR-312) |
| `value_key` | Requires `topic_key` |
| `importance` | `low \| normal \| high`. Ordering hint only (FR-308) |

`create` response gains `reconciliation`:

```json
{ "ok": true, "memory": { "…": "as today" },
  "reconciliation": { "outcome": "reinforced",
                      "reinforced_memory_id": "…",
                      "subject": "infrastructure.production_database",
                      "relation_recorded": "reinforces",
                      "conflict_detected": false },
  "notes": [] }
```

`outcome` ∈ `created | reinforced | duplicate | conflict_detected | deferred`. `deferred` carries the
note `reconciliation_deferred` when `reconcile_members_max` was exceeded — the memory is stored either
way.

**`reinforce`** — `memory_id`. Records `reinforces` from the caller's session context. Idempotent per
`(from, to, kind)`.

**`attach_evidence`** — `memory_id`, `kind`, `subject`, `observed_value`, `source_locator`, optional
`observation_id`, `role` (default `supports`), optional `collector` (`agent` when the agent is
attesting). Refuses `absolute_locator`, `evidence_excluded`, `evidence_outside_worktree`,
`evidence_too_large`.

**`verify`** — `memory_id`. Runs the applicable verifiers within the standard caps and returns the
run. `verification_inconclusive` is `ok: true`.

**`pin`** — `memory_id`, `pinned` (bool), `reason` (bounded). Refuses `pin_budget_exhausted` with the
current pins listed.

**`reconcile`** — `from_memory_id`, `to_memory_id`, `relation` ∈ `narrows | not_applicable_to |
supersedes`, `basis` ∈ `explicit_agent | evidence`, optional `basis_evidence_id`, optional
`rationale`. `supersedes` here is the conflict-resolution path; the `supersede` action remains the
create-and-replace path.

**`promote`** — `memory_id`, `signals[]`, `applicability[]`, `root_cause`, `approach`,
`constraints[]`, optional `dry_run`. Returns the gate outcome; a refusal names the class and echoes no
value (FR-397).

**`record_outcome`** — `pattern_id`, `outcome`, optional `alternative_cause`, optional `evidence_id`,
optional `signals[]` (this incident's signal set). `discovery` is set by the daemon: `cairn_suggested`
when this session received the pattern in its context, `independent` otherwise — the agent cannot
choose it (D63).

That last rule matters: letting an agent declare its own discovery mode would hand the anti-poisoning
control to the party it constrains.

---

## `cairn_session`

| Action | Status | Purpose |
|---|---|---|
| `current`, `start`, `bind_task`, `end` | existing | Unchanged |
| `checkpoint` | **new** | Write a continuity checkpoint now |

`checkpoint` derives the boundary record if none exists (`no_boundary_record` is handled, not
raised), writes the checkpoint, and returns its id and assumption set. This is the on-demand path
FR-425 requires and the one an `unavailable_automatic` agent uses.

`current` response gains:

```json
{ "task_revision_at_bind": 5,
  "task_divergence": { "from": 5, "to": 6,
                       "changes": ["criterion added — AC-4", "blocker opened"] },
  "latest_checkpoint": { "id": "…", "created_at": "…", "state": "diverged" } }
```

---

## `cairn_task`

| Action | Status | Purpose |
|---|---|---|
| `list`, `get`, `create`, `update` | existing | Unchanged; `get`/`list` gain read-only fields |
| `add_criterion` | **new** | `task_id`, `text` |
| `update_criterion` | **new** | `criterion_id`, optional `text`, `state`, `verification`, `evidence_observation_id`, `expected_revision` |
| `blocker` | **new** | `task_id` + `description` to open; `blocker_id` + `clear: true` to clear |
| `readiness` | **new** | `task_id`; derived counts and readiness |

`create` and `update` still accept `acceptance_criteria` as an array of strings and still work
exactly as today; `update` diffs by text, preserving ids for unchanged entries (see
[task-model.md](./task-model.md)).

`get` response gains `revision`, `criteria[]` (with `id`, `label`, `text`, `state`, `verification`,
`revision`, `evidence_count`), `blockers[]`, `progress`, `completion_readiness`.

---

## `cairn_handoff`

| Field | Notes |
|---|---|
| `include_checkpoint` | bool, default `false`. Adds the anchored checkpoint and its staleness assessment to `latest` |

Actions and triggers are unchanged. `stop` is still absent from `trigger` — a turn checkpoint is not a
handoff boundary (Feature 001 D16), and the existing test asserting it remains.

---

## Tool descriptions

Descriptions are what an agent actually reads, so each names its Feature 003 obligation in one clause,
within the existing size discipline:

| Tool | Added clause |
|---|---|
| `cairn_context` | "…plus the minimum safe continuity, drift and conflict warnings, and whether your checkpoint diverged." |
| `cairn_search` | "Filter by verification state or subject; ask for reusable patterns explicitly." |
| `cairn_remember` | "Give durable project facts a `topic_key` so Cairn can reconcile them. Attach evidence rather than asserting importance. Record a conflict rather than overwriting." |
| `cairn_session` | "…or write a continuity checkpoint before you compact." |
| `cairn_task` | "Update one criterion at a time and pass the `expected_revision` you read." |
| `cairn_handoff` | "…optionally with its continuity checkpoint." |

The usage contract and the Skill carry the same obligations in their own renderings, generated from
the one canonical source Feature 002 established, and the existing size-bound assertion still applies
(FR-498, Feature 002 FR-123/FR-125).

## Error codes

Every code added by Feature 003 is listed in its own contract:
[knowledge.md](./knowledge.md), [evidence-verification.md](./evidence-verification.md),
[continuity-context.md](./continuity-context.md), [patterns.md](./patterns.md),
[task-model.md](./task-model.md).

All are added to the single stable set in `cairn-core/src/wire.rs::codes` with the existing
exit-code mapping. There is still no `budget_exceeded`: a briefing is truncated to fit, never
rejected for size (FR-445).
