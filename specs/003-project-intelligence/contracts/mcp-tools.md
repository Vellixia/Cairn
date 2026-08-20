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
    "guaranteed": {
      "task": { "id": "…", "goal": "…", "status": "in_progress" },
      "progress": { "verified": 3, "satisfied_unverified": 1, "blocked": 1,
                    "pending": 2, "waived": 0, "total": 7 },
      "completion_readiness": "not_ready",
      "open_blocker_count": 4,
      "top_blocker": "staging credentials expired",
      "next_action": "finish the retry backoff in config.rs",
      "previous_next_action": null,
      "warning_counts": { "checkpoint_divergence": 1, "conflict": 1, "drift": 1 },
      "repository": { "…": "Feature 001 RepositoryState" }
    },
    "detail": {
      "criteria": [{ "id": "…", "label": "AC-3", "text": "…",
                     "state": "blocked", "verification": "unverified" }],
      "pinned_constraints": [{ "id": "…", "content": "…",
                               "verification": { "state": "verified", "authority": "cairn" } }],
      "further_blockers": []
    },
    "omitted": { "criteria": 37, "blockers": 3, "warnings": 0,
                 "retrieval": "cairn task get <id>" }
  },

  "continuity": {
    "mode": "automatic",
    "checkpoint": { "id": "…", "created_at": "…", "restore_count": 3,
                    "state": "diverged",
                    "divergences": [
                      { "kind": "commit", "recorded": "abc123", "current": "def456" },
                      { "kind": "task",
                        "recorded_state_digest": "3f9c…", "current_state_digest": "8b21…",
                        "changes": [
                          { "kind": "criterion_added", "label": "AC-4",
                            "origin": "another_machine" },
                          { "kind": "blocker_opened", "origin": "this_machine" }
                        ] },
                      { "kind": "files",
                        "changed": [
                          { "path": "src/config.rs", "fingerprint_class": "digest",
                            "outcome": "changed", "last_touched_by_session": "0192f4…" },
                          { "path": "src/retry.rs", "fingerprint_class": "digest",
                            "outcome": "changed", "last_touched_by_session": null }
                        ],
                        "not_fingerprintable": [
                          { "path": "vendor/large.bin", "reason": "over_payload_cap" }
                        ] }
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

`minimum_safe.guaranteed` is Tier 0a — every field is O(1) in the size of the project and the task, so
it is present at any budget from the documented minimum upwards. `minimum_safe.detail` is Tier 0b,
admitted as budget allows, and `minimum_safe.omitted` reports what did not fit by kind with the call
that retrieves it (FR-443, FR-448).

`warnings` is Level 0 content and appears whether or not `explain` is set (FR-464). `patterns` always
carries `verified_in_this_project: false` (SC-312). `last_touched_by_session` is `null` when no Cairn
session recorded the change — which is exactly the case path fingerprints exist to catch (FR-432).

---

## `cairn_search`

### Added input

| Field | Type | Notes |
|---|---|---|
| `verification` | enum | `unverified \| verified \| needs_recheck \| drifted \| conflicted` |
| `authority` | enum | `cairn \| attested \| remote_cairn \| remote_attested` — filter by what established it |
| `corroborated` | bool | Restrict to memories in a subject whose members agree on a value and differ in content |
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
  "verification": { "state": "verified", "authority": "cairn",
                    "last_verified_at": "…", "fact_count": 2,
                    "basis": ["configuration", "git_ref"] },
  "temporal": { "effective_from": "…", "superseded_at": null },
  "reinforcement": { "count": 3, "distinct_origins": 3 },
  "subject": { "reconciliation": "reinforced", "is_canonical_answer": true,
               "competing_answers": [], "corroborating_answers": [] }
}
```

`verification.basis` carries **verifier kinds only** — never a subject, value, locator or digest
(FR-502). `verification.authority` is one of `cairn`, `attested`, `remote_cairn`, `remote_attested` and
is always present when the state is `verified`, so a caller can never mistake an attestation for a
deterministic check (FR-370). `reinforcement` is never labelled as verifications (FR-406).

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
  "reconciliation": { "outcome": "corroborating",
                      "matched_memory_id": "…",
                      "matched_value_key": "jwt",
                      "subject": "auth.strategy",
                      "relation_recorded": null,
                      "conflict_detected": false,
                      "next_step": "if this is the same claim, call action=reinforce with memory_id" },
  "notes": ["corroborating_member"] }
```

`outcome` ∈ `created | duplicate | corroborating | conflict_detected | deferred`.

- `duplicate` — content identical after normalization; a `duplicates` relation was recorded
  automatically, and `matched_memory_id` names the canonical member.
- **`corroborating`** — the same subject and value key, differing content. **Nothing was merged and no
  relation was recorded.** `matched_memory_id` names the member it agrees with, and `next_step` names the
  call an agent that has independently confirmed the claim makes: `action=reinforce` against the matched
  member, from the memory just written. Reinforcing is confirmation, not merging — it records that this
  session found the matched member still true, and the subject stays `corroborated` with both statements
  retained (`contracts/knowledge.md` §Automatic reconciliation, D46). This is the prompt that keeps
  reinforcement cheap without letting Cairn infer it (FR-327).

  If the agent's judgment is stronger than confirmation — that the two statements are not merely both
  true but are *the same claim*, differently worded, and should collapse into one canonical answer —
  reinforcing does not do that. The collapsing call is an explicit `reconcile` with
  `relation: "duplicates"` and `basis: "explicit_agent"`, naming the newer memory as `from_memory_id` and
  the matched member as `to_memory_id`. After it the subject reads `reinforced` with one answer, and the
  folded-in statement is accounted for as its duplicate — still individually retrievable with its own
  provenance. Both calls were driven live against a real store during T146; see implementation-log
  Checkpoint R.
- `conflict_detected` — same subject, incompatible value key, overlapping scope.
- `deferred` — `reconcile_members_max` was exceeded; the memory is stored either way.

There is deliberately no `reinforced` outcome: reinforcement is an explicit act, never an automatic
one (FR-321).

**`reinforce`** — `memory_id`. Records an explicit `reinforces` relation from the caller's session
context, meaning *this session confirmed that memory is still true*. Idempotent per
`(from, to, kind)`. This is the **only** path that produces a `reinforces` relation; Cairn never infers
one (FR-321).

**`attach_evidence`** — `memory_id`, `kind`, `subject`, `observed_value`, `source_locator`, optional
`observation_id`, `role` (default `supports`), optional `collector` (`agent` when the agent is
attesting). Refuses `absolute_locator`, `evidence_excluded`, `evidence_outside_worktree`,
`evidence_too_large`.

**`verify`** — `memory_id`. Runs the applicable verifiers within the standard caps and returns the run,
including the resulting `authority`. `verification_inconclusive` is `ok: true`.

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
{ "task_state_digest_at_bind": "3f9c…",
  "task_divergence": { "advanced": true,
                       "current_state_digest": "8b21…",
                       "changes": [
                         { "kind": "criterion_state", "label": "AC-2",
                           "from": "pending", "to": "satisfied", "origin": "this_machine" },
                         { "kind": "criterion_added", "label": "AC-4",
                           "origin": "another_machine" },
                         { "kind": "blocker_opened", "origin": "another_machine" }
                       ] },
  "latest_checkpoint": { "id": "…", "created_at": "…", "state": "diverged" } }
```

The change list is diffed from the session's bound snapshot against the current synchronized records, so
a change that arrived from another machine appears with `origin: "another_machine"` rather than being
invisible (D80, FR-489).

---

## `cairn_task`

| Action | Status | Purpose |
|---|---|---|
| `list`, `get`, `create`, `update` | existing | Unchanged; `get`/`list` gain read-only fields |
| `add_criterion` | **new** | `task_id`, `text` |
| `update_criterion` | **new** | `criterion_id`, optional `text`, `state`, `verification`, `evidence_observation_id`, `expected_revision` (the **local** counter read from `get`) |
| `blocker` | **new** | `task_id` + `description` to open; `blocker_id` + `clear: true` to clear |
| `readiness` | **new** | `task_id`; derived counts and readiness |

`create` and `update` still accept `acceptance_criteria` as an array of strings and still work
exactly as today; `update` diffs by text, preserving ids for unchanged entries (see
[task-model.md](./task-model.md)).

`get` response gains `local_revision`, `state_digest`, `criteria[]` (with `id`, `label`, `text`,
`state`, `verification`, `verification_authority`, `revision`, `evidence_count`), `blockers[]`,
`progress`, `completion_readiness`.

`local_revision` is this store's concurrency token — pass it back as `expected_revision`. `state_digest`
is the cross-device state identity; it is the value to compare when asking whether two machines agree
(D80).

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
| `cairn_remember` | "Give durable project facts a `topic_key` and a `value_key` specific enough to state the whole claim. Attach evidence rather than asserting importance. If Cairn reports a corroborating member and it is the same claim, reinforce it. Record a conflict rather than overwriting." |
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
