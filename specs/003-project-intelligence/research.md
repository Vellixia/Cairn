# Research: Cairn Project Intelligence

**Feature**: `003-project-intelligence` | **Date**: 2026-08-14
**Baseline**: `main` @ `0b79b314f616df27409370b8dce54193c092a1fc` (v0.1.0-alpha.4)

Feature 001 recorded decisions D1–D17; Feature 002 recorded D18–D42. This feature continues at
**D43**.

Every decision below was taken against the code on that commit, not against a remembered design.
Section 0 records what the baseline actually is, because three of the brief's assumptions did not
survive contact with it.

---

## 0. Baseline findings

These were established by reading the implementation. They constrain everything that follows.

### B1 — There is no event-sourced projection anywhere in Cairn

The brief asks for "deterministic replay semantics if Cairn's current architecture uses
event-sourced projections for that domain". It does not.

`crates/cairn-store/migrations/0001_init.sql` defines direct-state tables. There is no event log,
no aggregate, no projection rebuild, and no ordering guarantee beyond SQLite's own. The single
append-only structure is the **transactional outbox** (`outbox`), which is a delivery queue, not a
source of truth: `outbox::enqueue` writes a row in the same transaction as the change it
describes, and `crates/cairn-server/src/sync.rs` applies it at most once against
`sync_state.idempotency_key`.

**Consequence**: Feature 003 must not introduce event sourcing (Constitution II — no speculative
infrastructure, and the brief's own "do not invent a second competing transactional model"). What
the brief's replay requirement *means* here is: every derived value must be **rebuildable from
durable records by a documented deterministic procedure**. That is D43, and it is a strictly
weaker and more appropriate obligation than event replay.

### B2 — Memory rows are already append-mostly, which is why cross-device merge does not lose writes

`repo::create_memory` always inserts a new UUIDv7. `repo::supersede_memory` inserts the
replacement and then updates only the original's `state` and `superseded_by_id`. The only other
in-place writes to a memory are `mark_stale_scopes` (state → `stale`) and the delete tombstone.

On the sync path, `cairnd/src/sync.rs::import_memory` does `INSERT OR IGNORE` and returns early if
the row already exists. The server's `upsert_memory` is `ON CONFLICT (id) DO UPDATE`, but the
conflict can only be the *same* memory redelivered, never two different proposals.

**Consequence**: two devices proposing incompatible knowledge already produce two surviving rows.
Feature 003 does not need to fix a lost-update bug in memory — it needs to *detect* the
disagreement those two rows represent, and to make state changes (supersession) travel as
decisions rather than as row overwrites, which `import_memory`'s early return currently prevents.

### B3 — Tasks are the real last-write-wins surface

`repo::update_task` writes `title`, `goal`, `acceptance_criteria` and `status` in one statement,
substituting current values for absent arguments. `acceptance_criteria` is a JSON array of plain
strings with no identity. The server's `upsert_task` replaces the whole array on conflict.
`domain.rs` states it outright: *"No revision history exists (FR-039)."*

**Consequence**: two sessions editing different acceptance criteria today lose one another's work.
The task side quest is not decoration — it is where the brief's "no lost writes" requirement has
actual teeth. It is still secondary in scope, but it is not optional.

### B4 — Context assembly is one flat priority list with no reserve

`cairn-core/src/context.rs` walks a fixed `SECTION_ORDER` of eight sections and spends a single
`Budget` top-down; `HIGH_PRIORITY_SECTIONS` is only used to *report* whether a degradation was
material, not to protect anything. `Budget::try_spend` measures before emitting, which is what
makes "never exceeds the budget" a property of the loop.

**Consequence**: adding warnings, criteria states and continuity to this list would put them below
`known_failures` and above nothing — they would be dropped exactly when they matter most. A
reserve is required, and it must preserve the measure-before-emit invariant.

### B5 — Feature 002 already models honest degradation, and already has an evidence precedent

`cairn-integrate/src/capability.rs` carries 14 capabilities across availability
(guaranteed/conditional/absent) × confidence (verified/expected), and
`migrations/0004_integrations.sql` already has a `capability_evidence` table. `LifecyclePostCompaction`
is a real capability that OpenCode provides only experimentally and generic MCP not at all.

**Consequence**: Feature 003's continuity honesty requirement (FR-426) needs no new capability and
no new canonical event. It is a **derived read** over capabilities Feature 002 already models.

### B6 — Privacy is enforced structurally, not by convention

There is no observation entity type in `OutboxEntityType` (asserted by
`outbox_cannot_carry_observations`), no observations table on the server, and
`sync.rs::reject_forbidden_fields` refuses eight observation-bearing field names and five
local-only session fields on the wire. `local_only` memories never produce an outbox row at all.

**Consequence**: Feature 003 must extend the allowlist by enumerated fields, and anything it
declines to share should be declined *structurally* — by having no outbox entity type and no
server table — rather than by a rule someone must remember.

### B7 — Retrieval is FTS5 over `memories.content`, external-content, with three triggers

`0002_memory_fts.sql` builds `memory_fts` as an external-content table over `memories` with
insert/delete/update triggers. Adding an indexed column would mean recreating the virtual table and
reindexing.

**Consequence**: keep FTS on `content`. Match subject identity with an ordinary indexed SQL filter.
No reindex, no migration risk, no change to `search.rs`'s ranking.

---

## 1. Knowledge model

### D43 — "Replay" means deterministic rebuild, not event replay

**Decision**: Feature 003 introduces no event log. Every value Cairn derives — a subject's
canonical answer, a reinforcement count, a memory's current verification state, a task's derived
progress — MUST be computable from **durable records** by a pure function, and that function MUST be
exercised by a test that discards the derived value and recomputes it.

Durable records are: `memories`, `memory_relations`, `evidence_facts`, `verification_runs`,
`task_criteria`, `task_blockers`, `task_changes`, `pattern_applications`, and Feature 001's
existing tables.

**Rationale**: satisfies the brief's replay intent (nothing derived that cannot be rebuilt) with the
architecture that exists (B1). Event sourcing would be a second transactional model, which the
brief forbids and Constitution II rejects.

**Alternatives considered**: a real event log with projections — rejected, it would duplicate the
outbox's job, require an ordering authority Cairn does not have across devices, and rewrite
Feature 001's storage. A "rebuild is not required, trust the increment" position — rejected,
FR-517 exists because an incrementally maintained counter that drifts is indistinguishable from a
correct one until someone audits it.

### D44 — The canonical answer is derived on read. There is no subject table

**Decision**: a subject is not a row. `SubjectView` is computed from an indexed query over active,
topic-keyed memories in the applicable scopes plus their relations, grouped in memory by
`(scope, scope_key, topic_key)`.

Only four denormalized values live on the memory row, each with a documented rebuild:
`reinforcement_count`, `distinct_origin_count`, `verification`, `last_verified_at`.

**Rationale**: this is the single most consequential simplification in the feature. With no cached
subject there is no projection to rebuild after a merge, no consistency window between a proposal
landing and the subject reflecting it, and no repair path to get wrong. FR-302 and FR-517 become
true by construction. Cost is one indexed query: with 5,000 memories and 500 subjects the
applicable topic-keyed set is small, and `subject_warning_scan_max` (D75) bounds it explicitly.

**Alternatives considered**: a materialized `knowledge_subjects` table with incremental maintenance
— rejected, it buys nothing measurable and adds the exact class of unrebuildable projection the
brief warns against. A materialized table treated as a pure cache — rejected as the worst of both:
still needs the derivation, and adds an invalidation bug surface.

### D45 — Subject identity is two optional, normalized columns on the memory row

**Decision**: `memories.topic_key` and `memories.value_key`, both nullable.

- `topic_key`: lower-cased; segments of `[a-z0-9_]` joined by `.`; at most 6 segments; at most 128
  characters total. Normalization is total — anything unrepresentable yields `None`.
- `value_key`: lower-cased, whitespace-collapsed, trimmed, at most 64 characters. Accepted only
  alongside a `topic_key`.
- A memory whose proposed key fails normalization is **stored without a key** and the reason is
  reported. It is never rejected (FR-312).

There is no registry, no allowlist, and no shipped vocabulary (FR-314).

**Rationale**: the smallest structure that makes reconciliation deterministic. Putting the keys on
the memory row rather than in a side table keeps `search.rs`, the outbox payload, the server upsert
and the web UI reading one row, and keeps FTS untouched (B7).

**Alternatives considered**: a shipped ontology — rejected explicitly by the brief and by
Constitution II. A separate `memory_subjects` join table — rejected, one memory asserts one thing
about one subject; a join table implies a many-to-many that has no use case. Requiring a key on
every memory — rejected by FR-313; it would break every existing memory and every manual-mode
recording.

### D46 — Automatic reconciliation covers exactly two decidable cases

**Decision**: Cairn reconciles automatically only when the answer needs no inference:

| Condition | Automatic outcome |
|---|---|
| same subject, equal `value_key` | `reinforces` relation |
| same subject **and** identical `content_norm` digest | `duplicates` relation |
| same subject, different `value_key` | **conflict candidate** — surfaced, never resolved |
| no `topic_key` on either side | nothing; free-form memories never auto-reconcile |

`content_norm` is: NFC, lower-cased, whitespace-collapsed, trailing punctuation stripped, then
SHA-256. Nothing fuzzier.

**Rationale**: this is where the brief's "Cairn must remain conservative" and FR-511's no-model rule
meet. Cairn cannot decide that "we use Postgres" and "the production DB is PostgreSQL" are the same
claim without inference, and it will not pretend otherwise. The honest consequence — stated in the
spec's Assumptions — is that reconciliation is strongest on memories carrying a subject, and the
usage contract asks agents for one.

**Alternatives considered**: token-overlap or trigram similarity with a threshold — rejected, a
threshold is a tuned constant masquerading as a rule, and a false merge destroys knowledge silently.
Model-assisted equivalence gating stored state — rejected by FR-511/FR-512; it may *propose*.

### D47 — Reconciliation decisions are append-only relation rows; `superseded_by_id` becomes a view of one

**Decision**: `memory_relations(from_memory_id, to_memory_id, kind, decided_by_session, decided_at,
basis, rationale)`, primary key `(from_memory_id, to_memory_id, kind)` — which makes recording the
same decision twice a no-op (FR-305).

`kind` ∈ `reinforces | duplicates | supersedes | conflicts_with | narrows | not_applicable_to`.
`basis` ∈ `deterministic_rule | evidence | explicit_agent | explicit_user`.

Feature 001's `memories.superseded_by_id` and `state = 'superseded'` are **retained and kept
consistent** with the `supersedes` relation, in the same transaction. Every existing reader — the
CLI, the web UI, `search.rs`, the server — keeps working unchanged (FR-324).

**Rationale**: an append-only, idempotent, primary-key-deduplicated edge is the whole mechanism.
It survives concurrent writers because two writers recording the same decision collide on the
primary key and the second is ignored; it survives merge because importing an edge twice is a
no-op; and it carries the provenance FR-304 demands.

**Alternatives considered**: mutating a `canonical` flag on memories — rejected, that is precisely
the last-write-wins path the brief forbids. Replacing `superseded_by_id` — rejected, it is a
Feature 001 contract with live readers.

### D48 — Conflict requires scope overlap; differing precedence is a scope exception

**Decision**:

- **Semantic conflict** is declared only for two active memories with the *same* `(scope,
  scope_key, topic_key)` and different `value_key`.
- Two memories on one topic at scopes of *different* precedence rank are a **scope exception**, not
  a conflict. The narrower applies in its own context by Feature 001's existing precedence
  (`MemoryScope::bucket`), and the broader is reported as the answer it narrows. A `narrows`
  relation may be recorded explicitly; its absence does not create a conflict.
- Two memories at the same scope with *different* scope keys — `branch:main` and
  `branch:feature/graphql` — never interact at all. They are never simultaneously applicable.

**Rationale**: scenario B (project PostgreSQL / task SQLite fixture) and the main-REST /
feature-GraphQL case are the two ways naive conflict detection produces noise that trains people to
ignore it. Both are excluded structurally, by the scope key, rather than by a heuristic.

**Alternatives considered**: conflict across scopes with a severity level — rejected, a "low
severity conflict" that is actually correct behaviour is a false positive with extra steps.

### D49 — No timestamp, and no identifier order, ever arbitrates

**Decision**: the reconciliation derivation reads `state`, `scope`, `scope_key`, `topic_key`,
`value_key`, relations, `verification` and evidence counts. It does **not** read `created_at`,
`updated_at`, `effective_from` or the UUID's embedded time to choose between competing proposals.
Identifier order is used only as the final total-order tiebreak for **stable output ordering** of an
already-conflicted set.

Enforced by test: `clock_swap_invariance` runs the offline-merge corpus with the two stores' clocks
reversed and asserts a byte-identical canonical result and conflict set (SC-304).

**Rationale**: this is the brief's hardest constraint and the easiest to violate accidentally,
because UUIDv7 *is* a timestamp and `ORDER BY created_at DESC` is already all over `search.rs`.
Making it a named, tested invariant is the only way it survives maintenance.

### D50 — Four instants, and no bitemporal store

**Decision**: `created_at` (existing), `effective_from` (defaults to `created_at`), `superseded_at`
(set when the `supersedes` relation is recorded), `last_verified_at`.

Two queries, both indexed:
- *current*: `state = 'active'` and the subject derivation.
- *as of T*: `effective_from <= T AND (superseded_at IS NULL OR superseded_at > T)`.

No valid-time table, no retroactive interval correction, no branching history (FR-345).

**Rationale**: answers both of the brief's questions — "what was believed when this session ran"
and "what is the best-supported current knowledge" — with two predicates over columns that already
have to exist. A bitemporal store would be the largest piece of speculative infrastructure in the
feature for the smallest incremental answer.

---

## 2. Evidence and verification

### D51 — Evidence facts are a new local table; Feature 001's observation evidence is untouched

**Decision**: `evidence_facts` holds bounded, redacted, attributable records of an observed state,
linked to memories through `memory_evidence_facts(memory_id, evidence_id, role)` with `role` ∈
`supports | contradicts`.

`memory_evidence` — Feature 001's `(memory_id, observation_id, content_digest)` — is **not
modified**. An evidence fact of kind `observation` carries the observation id, which is how the two
systems bridge without merging.

Kinds: `observation | file | git_ref | configuration | test_outcome | command_outcome |
runtime_state | schema_version`.

Every fact carries `subject`, `observed_value` (redacted, ≤256 bytes), `value_digest`,
`source_locator` (repository-relative or a Git ref, ≤256 bytes, never absolute — FR-353),
`fingerprint`, `collected_at`, `collected_by_session`, `collector` ∈ `cairn | agent`, `repo_branch`,
`repo_commit`.

**Rationale**: generalizes provenance rather than forking it. The brief's bad/good example is
enforced by construction: there is nowhere to put a raw connection string, because the value column
is bounded at 256 bytes *after* redaction and the locator is a path, not content.

**Alternatives considered**: overloading `memory_evidence` with a nullable kind — rejected, it would
change a Feature 001 table with a live server counterpart and a documented privacy meaning.

### D52 — The verifier catalog splits by who collected the evidence, and Cairn runs no commands

**Decision**: two classes.

**Cairn-collected** — Cairn reads it itself, inside the worktree (subject to
`excluded_paths`/`excluded_commands`) or through `cairn-git`:

| Verifier | Reads | Fingerprint |
|---|---|---|
| `file_exists` | path presence | presence + size |
| `file_digest` | file bytes | content digest |
| `git_ref` | `cairn-git` ref resolution | resolved object id |
| `git_commit` | commit presence / ancestry | commit id |
| `configuration` | a key in a repository configuration file | value digest |
| `schema_version` | a declared schema/version field | value digest |
| `test_outcome` | a **captured** `test_run` observation's recorded outcome and exit code | outcome + commit |
| `command_outcome` | a **captured** `command_run` observation's recorded exit code | exit code + commit |

**Agent-attested** — the agent submits a deterministic value Cairn cannot read:
`runtime_state`, and any `test_outcome`/`command_outcome` with no captured observation behind it.
Recorded with `collector = agent`, usable to reach `verified` on a *memory*, always labelled, and
**never** sufficient for a task criterion (D69).

Cairn executes nothing. No build, no test, no shell, no network (FR-365).

**Rationale**: this resolves the brief's apparent tension — "known test result at a commit" is
listed as valid verification, yet "autonomous arbitrary command execution" is a non-goal. The
resolution is that Cairn already captures test and command outcomes with exit codes through
Feature 001's hooks. Those are Cairn-collected deterministic facts obtained without running
anything. Only what Cairn genuinely cannot see is attested, and attestation is labelled and
capped in what it can establish.

**Alternatives considered**: letting Cairn run a project's test command — rejected outright, it
turns a memory tool into an executor. Refusing attested evidence entirely — rejected, it would
make runtime facts permanently unverifiable and push agents to lie in prose instead.

### D53 — The verification state machine is total and documented

**Decision**: five states, and every transition names its trigger. Full table in
[contracts/evidence-verification.md](./contracts/evidence-verification.md).

```text
                 attach evidence + run
   unverified ─────────────────────────▶ verified
        │                                  │  ▲
        │ run → inconclusive               │  │ run → verified (same value)
        ▼                                  ▼  │
   unverified                        needs_recheck ◀── evidence fingerprint changed
                                           │        (from verified, drifted, or conflicted)
                                           │ run → different value
                                           ▼
                                        drifted
   any ──── supporting and contradicting evidence, or two verifiers disagree ───▶ conflicted
```

`drifted` and `conflicted` both re-enter `needs_recheck` when their evidence changes again; neither
is terminal. Supersession does not change verification state — the superseded memory keeps its last
verification, which is what makes historical interpretation possible (D50).

**Rationale**: FR-375 requires a total machine so that "how did this memory get here" is always
answerable, and SC-306 requires every documented transition to be reachable and every undocumented
one unreachable. Writing it as a table makes both testable.

### D54 — Drift is marked by indexed locator matching, and verified by a bounded background pass

**Decision**: two separate mechanisms, deliberately.

**Marking** (cheap, on the capture path): when a `file_changed` observation is stored, Cairn looks up
evidence facts whose `source_locator` equals that path, capped at `evidence_lookups_per_event_max`
(8) by an index on `(project_id, source_locator)`. When a session's branch or commit changes,
commit-pinned facts for that branch are marked. Marking sets `verification = needs_recheck` on
supported memories and nothing else. Exceeding the cap defers to the background pass; it never
extends the 250 ms capture deadline (FR-374, FR-475).

**Verifying** (bounded, off the request path): a pass on the **existing 15-minute maintenance tick**
— the one already reaping idle sessions, sweeping owed handoffs and marking stale scopes — takes at
most `verify_pass_evidence_max` (200) facts, runs at most `verify_pass_runs_max` (50) verifiers, and
yields after `verify_pass_wall_ms` (2000). Concurrency 1. Plus an explicit on-demand path
(`cairn verify`, `cairn_remember action=verify`).

**Rationale**: separating "something moved" from "recheck it" is what keeps FR-471 (nothing verifies
at session start) and FR-374 (bounded per event) both true. Reusing the maintenance tick avoids a
scheduler, exactly as Feature 002 did for the pending-handoff sweep (D22).

**Alternatives considered**: filesystem watching — rejected, a new long-running mechanism with
platform-specific behaviour for a problem the capture stream already reports. Verifying at session
open — rejected by FR-471; it is the one place a latency regression is guaranteed to be noticed.

---

## 3. Continuity and context

### D55 — A continuity checkpoint anchors to the handoff Cairn already derives

**Decision**: `continuity_checkpoints` references a `handoff_id`. At `context_compacting` Cairn
already writes a `pre_compact` handoff carrying goal, progress, completed and remaining work,
changed files, decisions, failures, tests and next step. The checkpoint adds only what the handoff
does not have:

- the **assumption set**: `branch`, `commit_sha`, `task_id`, `task_revision`, `relevant_paths`
  (bounded);
- the **criteria snapshot** and open blockers at that instant;
- the **pinned constraints in force**;
- `restored_at`, `restore_count`, `checkpoint_state`.

No second synthesis engine (FR-423).

**Rationale**: the brief's checkpoint field list is, field for field, almost exactly `Handoff` plus
staleness inputs. Building a parallel synthesizer would double the derivation logic and guarantee
the two drift. Anchoring means one derivation, tested once, and `cairn handoff show` and continuity
can never disagree.

**Alternatives considered**: a standalone checkpoint deriving its own fields — rejected as above.
Extending `handoffs` in place — rejected, `handoffs` syncs and a checkpoint must not (FR-503).

### D56 — Four divergence classes, all detectable from local state

**Decision**: on restore, compare and classify `current | diverged | unresolvable`:

| Class | Comparison | Source |
|---|---|---|
| `branch` | checkpoint `branch` vs current | `cairn-git` |
| `commit` | checkpoint `commit_sha` vs current head | `cairn-git` |
| `task` | checkpoint `assumed_task_state_digest` vs the current derived digest | store; the change list is diffed from the bound snapshot. **Revised by D80** — the original wording compared a local counter and read the local change log, both of which miss remote changes |
| `files` | a `relevant_paths` entry with a `file_changed` observation from **another** session after `created_at` | store, indexed by session and time |

`unresolvable` when the task or the worktree the checkpoint names no longer exists; the
task-independent fields are still delivered (FR-435).

A diverged checkpoint's next action is emitted as `previous_next_action` with its recorded commit,
never as `next_action` (FR-434).

**Rationale**: all four are answerable from data Cairn already has, with no new capture. The
files class is the one that catches the genuinely dangerous case — two agents in one worktree — and
it works because observations already carry `session_id`, `path` and `occurred_at`.

### D57 — Continuity mode is derived from Feature 002 capabilities. No new event, no new capability

**Decision**: a derived, reported value:

| `LifecyclePreCompaction` | `LifecyclePostCompaction` | `continuity_mode` |
|---|---|---|
| present | present | `automatic` |
| present | absent or conditional | `agent_initiated` — names `cairn_context(reason=post_compaction)` |
| absent | any | `unavailable_automatic` — checkpoint still written at session close and on demand |

Under the delivery path as built, that is Claude Code `agent_initiated`, Codex
`agent_initiated`, OpenCode `agent_initiated` (its compaction hook is experimental), generic MCP
`unavailable_automatic` — outputs of the rule, not a hand-maintained table.

No agent reaches `automatic` today, and the row above is reachable only if Cairn gains a
post-compaction delivery path. `automatic` requires *context re-delivery* after compaction;
`ContextCompacted` is capture class, so `cairn hook` sends it one-way and returns without
emitting anything back to the session (`delivers_context` in `crates/cairn/src/hook.rs` is
`SessionOpened` alone). This paragraph previously said "currently verified" and named two agents
`automatic`; neither had been driven live, and Claude Code's claim was disproved by the first real
compaction (T148). The rule is kept as written rather than narrowed, because the missing piece is
the delivery path, not the derivation.

**Rationale**: B5. Feature 002 built exactly this machinery and its honesty rules (FR-241, FR-242)
already forbid claiming a capability that is only expected. Adding a capability would duplicate it;
adding a canonical event would violate FR-427 and Feature 002's FR-113.

### D58 — `Budget` gains a reserve; Level 0 has a documented admission order

**Decision**: `Budget::with_reserve(limit, reserve)`. Level 0 admissions draw from the reserve
first, then the general pool. Level 1 and Level 2 admissions may only draw from the general pool.
The reserve is a **cap on the lower levels, not a floor Level 0 must spend** — unspent reserve
returns to the general pool once Level 0 is complete, so a project with no task, no warnings and no
pins delivers exactly what it delivers today (FR-442).

`min_safe_context_fraction` default `0.40`. `try_spend`'s measure-before-emit contract is unchanged,
so `estimated_tokens <= budget` remains a property of the loop (FR-445).

Level 0 admission order, applied when even the reserve binds:

1. task identity, goal, status
2. next action, or `previous_next_action` with its divergence statement
3. open blockers
4. critical warnings — checkpoint divergence, task divergence, conflict, drift (in that order)
5. pinned constraints in force
6. repository state
7. acceptance criteria with states, then derived progress

**Rationale**: B4. The order is the answer to "what would I most regret losing", and it is
deterministic so behaviour under an absurdly small budget is specified rather than incidental
(FR-446).

**Alternatives considered**: raising the default budget to make room — rejected, it silently
increases every agent's prompt cost to solve a prioritization problem. Two separate assemblers —
rejected, two budget loops means two places for the never-exceed invariant to break.

### D59 — Pins are bounded, scope-respecting, and lost on supersession

**Decision**: `pinned`, `pinned_at`, `pinned_by_session`, `pin_reason` on the memory row. Budget
`pin_budget_project` 12, `pin_budget_per_scope` 4, `pins_in_context_max` 4. Exceeding refuses with
`pin_budget_exhausted` and lists the current pins; nothing is auto-unpinned (FR-454). A pin never
widens scope (FR-453). Recording a `supersedes` relation clears the predecessor's pin in the same
transaction; the successor is pinned only explicitly (FR-456). A drifted memory keeps its pin and
carries its warning.

**Rationale**: an unbounded "critical" flag is a tragedy of the commons — every agent marks its own
work critical and the mechanism becomes noise. A hard, refusable budget is the only bound that
holds. Auto-clearing on supersession prevents the worst failure: a pinned constraint that has been
replaced still occupying reserved space.

### D60 — Selection reasons are a closed set, and `explain` is off by default

**Decision**: every admitted item carries reasons from a closed enum — `scope_match`,
`canonical_answer`, `verified`, `pinned`, `drift_warning`, `conflict_warning`,
`pattern_signal_match`, `checkpoint_assumption`, `task_binding`. Omissions carry
`budget_exhausted`, `scope_mismatch`, `superseded`, `not_canonical`, `level_2_only`,
`pin_budget`, `cap_reached`.

Returned only when the caller passes `explain: true`; it costs no budget otherwise (FR-463).
Warnings themselves are Level 0 *content*, not diagnostics (FR-464).

**Rationale**: answers "why did Cairn tell the agent this?" without spending the agent's tokens on
the answer. A closed set makes the reasons a testable contract rather than free-text logging.

---

## 4. Cross-project reusable patterns

### D61 — Patterns are a separate, project-independent, local, never-synced record

**Decision**: `reusable_patterns` has **no `project_id`**. It carries `title`, `problem`, `signals`
(bounded normalized tokens + a `signal_digest`), `applicability`, `root_cause`, `approach`,
`constraints`, `trust`, `origin_ref` (an opaque local reference), `origin_deleted`,
`sanitization_report`. It has no outbox entity type and no server table (FR-508, B6).

**Rationale**: three things at once. It keeps a project memory from ever becoming a global memory
(FR-391) because the two are different records with different tables. It removes the feature's
largest privacy question — what may leave one team's project and enter another's — from this
feature's scope, as the brief's "bias conservative" instruction directs. And it still delivers the
story a developer actually has: their own next project.

**Alternatives considered**: a `global` memory scope — rejected explicitly by the brief and by
Feature 002's FR-190 (scoping uses only project/branch/task/session). Syncing patterns to the
server — deferred; it needs a sharing-consent model this feature does not have.

### D62 — The promotion gate is deterministic and fails closed

**Decision**: promotion is refused, with the class named and the value never echoed, when any of the
following holds. All checks are deterministic; the order is fixed so the reported reason is stable.

1. source not lifecycle `active` → `source_not_active`
2. source `verification != verified` → `source_unverified`
3. source has zero evidence facts → `no_evidence`
4. source `local_only` → `local_only_memory`
5. source's subject is `conflicted` → `source_conflicted`
6. source `type` ∈ `fact` and the memory is subject-bound to project configuration →
   `not_transferable`
7. content matches the redaction pattern set after redaction → `possible_secret`
8. content contains an absolute path, the project name, the normalized repository remote, the
   `server_project_id`, the `git_common_dir`, or a user email → `project_identifying`
9. `signals` after normalization has fewer than `pattern_signals_min` (2) entries →
   `insufficient_specificity`
10. an existing pattern has an equal `signal_digest` and an equal normalized `root_cause` digest →
    `duplicate_pattern`

A refusal writes nothing (FR-397).

**Rationale**: FR-507 names promotion the feature's highest privacy risk, so the gate must be a
list of deterministic predicates with a seeded adversarial corpus (SC-315), not a judgement.
Check 6 is the one that stops "production DB is PostgreSQL" — a true, verified, evidence-backed
project fact — from becoming a universal claim.

### D63 — Trust advances on distinct non-origin projects, and Cairn-suggested successes do not count without local evidence

**Decision**: `pattern_applications(pattern_id, project_id, session_id, outcome, discovery,
alternative_cause, evidence_id, signal_digest, applied_at)`, unique on
`(pattern_id, project_id, signal_digest)` so one incident counts once however many sessions touch
it.

- `outcome` ∈ `resolved | not_applicable | failed`
- `discovery` ∈ `independent | cairn_suggested`

Trust ladder:

| Trust | Condition |
|---|---|
| `candidate` | proposed, gate not yet passed |
| `sanitized` | gate passed |
| `validated` | ≥1 **distinct non-origin** project with `outcome = resolved`, and either `discovery = independent` or an `evidence_id` collected in that project |
| `contested` | any `not_applicable` or `failed` application exists |

`contested` coexists with evidence of success and is reported as such (FR-405). No count is ever
presented as a number of independent verifications (FR-406).

**Rationale**: this is the brief's anti-poisoning requirement made arithmetic. Ten sessions on one
project A incident collapse to one row by the unique key, so the distinct-project count is 1
(SC-314). And a pattern that only ever "worked" because Cairn suggested it and an agent agreed
cannot reach `validated` without deterministic local evidence — which is the actual feedback loop
that would otherwise manufacture confidence.

### D64 — A counterexample is an application, not a deletion

**Decision**: `outcome = not_applicable` with `alternative_cause` recorded. It increments no
success count, never deletes the pattern, and makes the pattern `contested`. Subsequent suggestions
carry the pattern, the known alternative cause, and a "check this first" line derived from the
alternative cause's own signals.

**Rationale**: the Docker-pool / VPN-route-collision case in the brief is exactly why deleting is
wrong — the original pattern is still right in its own applicability. Negative knowledge is
knowledge.

### D65 — Patterns are suggested only on recorded signal match, and always labelled unverified here

**Decision**: a pattern enters Level 1 context only when the current project's own recorded
signals — error signatures from `error` observations and from `failure`-type memories, matched
lexically against the pattern's normalized signals — overlap by at least
`pattern_signals_min` tokens. At most `patterns_in_context_max` (2). Always rendered with
"unverified in this project" and its applicability conditions (FR-398, SC-312).

**Rationale**: an unsolicited pattern in every briefing is budget spent on noise. Gating on the
project's own recorded failure signals means a pattern arrives when the symptom is present, which
is the only time it helps.

---

## 5. Synchronization, privacy and compatibility

### D66 — Three new outbox entity types; everything else is structurally local

**Decision**: `OutboxEntityType` gains exactly `MemoryRelation`, `TaskCriterion`, `TaskBlocker`.
The memory payload gains enumerated fields. Nothing else in Feature 003 has an outbox entity type,
which is what makes "evidence, verification runs, checkpoints, patterns, applications, task changes
and selection diagnostics never leave the machine" a property of the schema rather than a promise
(B6, FR-502, FR-503).

Memory payload additions: `topic_key`, `value_key`, `importance`, `effective_from`,
`superseded_at`, `pinned`, `reinforcement_count`, `distinct_origin_count`, and a `verification`
object of exactly `{state, last_verified_at, fact_count, basis: [verifier_kind]}`.

`basis` carries **verifier kinds only** — `["git_ref","test_outcome"]` — never a subject, a value, a
locator or a digest.

**Rationale**: a strict extension of the rule Feature 001 already enforces: identifiers and a
count, never the rows behind them. A teammate learns that something was verified, when, and against
what kind of check. If they need the value they look where it was verified.

### D67 — `import_memory` learns to apply relations without overwriting rows

**Decision**: `sync_changes` gains optional `relations`, `criteria` and `blockers` arrays and keeps
`memories`. `import_memory` keeps its `INSERT OR IGNORE` (B2) and stops being the only importer:
relations are imported separately with `INSERT OR IGNORE` on their primary key, and the local
`state`/`superseded_by_id` view is then **re-derived from relations** rather than copied from the
remote row.

Verification state imported from a peer is reported as verified elsewhere and never counts toward local
readiness (FR-368, SC-305 companion). **Revised by D76**: the import records the peer's *authority* —
`remote_cairn` or `remote_attested` — rather than a single `remote` flag, because a peer's attestation and
a peer's deterministic check are not the same claim.

Where the server rejects a new field or entity type, the daemon records the rejection class, stops
sending that class, keeps delivering everything else, and surfaces it in `cairn sync status`
(FR-415, SC-326).

**Rationale**: B2 identified the actual gap — a supersession decided remotely never lands locally,
because `import_memory` returns early on an existing row. Importing the *decision* fixes it without
introducing row overwriting, which is the thing the brief forbids.

### D68 — Tasks: criteria become rows, the JSON array stays as a synchronized projection

**Decision**: `task_criteria(id, task_id, ordinal, label, text, state, verification, revision, ...)`
and `task_blockers(id, task_id, description, state, opened_by_session, opened_at, cleared_at,
cleared_by_session)`.

`tasks.acceptance_criteria` is **retained** and rewritten in the same transaction as any criterion
change, as the ordinal-ordered list of criterion texts. Every existing reader — `context.rs`'s
`admit_task`, the `cairn task` CLI, the server's `upsert_task`, the web UI's task page — continues
to work with no change (FR-492).

`tasks.revision` is a monotone counter advanced by any change to the task, its criteria or its
blockers, in that same transaction. `task_changes` is a local append-only log naming author, kind,
prior and new value. `sessions.task_revision_at_bind` records what a session bound at.

> **Revised by D80.** The counter turned out to be sound only *within* one store: it synced, so two
> offline machines each advancing 5→6 produced two different "revision 6" states, and a divergence report
> built from the local change log omitted criterion changes that arrived from elsewhere. D80 makes the
> counter local and unsent, adds a derived `task_state_digest` for cross-device identity, and derives the
> change list by diffing a bound snapshot against the converged records. The per-criterion row design
> below is unchanged, and is what made that fix cheap.

**Rationale**: B3 is the lost-update surface; per-criterion rows remove it by construction, because
two sessions editing different criteria touch different rows. Keeping the JSON array as a projection
is what makes the change additive rather than a breaking migration across three readers and a
server.

**Alternatives considered**: replacing `acceptance_criteria` with a join — rejected, it breaks the
server, the web UI and the briefing simultaneously for no gain. A separate `TaskRevision` entity
holding a full goal contract snapshot — rejected as heavier than the requirement; the change log
plus a counter answers every question FR-488/FR-489 pose.

### D69 — A criterion reaches `verified` only on Cairn-collected evidence

**Decision**: criterion verification accepts only `collector = cairn` evidence facts. Attested
evidence may be attached and is labelled, but leaves the criterion `unverified`.

**Rationale**: completion readiness is the one derived value with an incentive attached. If an agent
can attest its way to `verified`, readiness becomes self-certification and FR-483 is decorative.
Cairn *does* capture test and command outcomes itself (D52), so the honest path is open.

### D70 — Six tools, extended by action and parameter. No nested discriminators

**Decision**: still exactly six tools (asserted by the existing
`the_surface_is_still_exactly_six_tools` test). New actions:

| Tool | Existing actions | Added |
|---|---|---|
| `cairn_remember` | create, supersede, forget | `reinforce`, `attach_evidence`, `verify`, `pin`, `reconcile`, `promote`, `record_outcome` |
| `cairn_task` | list, get, create, update | `add_criterion`, `update_criterion`, `blocker`, `readiness` |
| `cairn_session` | current, start, bind_task, end | `checkpoint` |
| `cairn_context` | — | parameters `explain`, `depth`, `include_patterns`; `reason` gains `post_compaction` |
| `cairn_search` | — | parameters `verification`, `conflicted`, `topic_key`, `as_of`, `include_patterns` |
| `cairn_handoff` | latest, generate, annotate | parameter `include_checkpoint` |

`pin` carries `pinned: bool`; `reconcile` carries a `relation` naming which decision is being
recorded. No action takes a sub-operation — one discriminator per tool, as Feature 001 established.

A call with only Feature 001 arguments behaves exactly as today, plus read-only fields
(FR-496, FR-497).

**Rationale**: the constitution requires a compact surface and Feature 002's FR-128 fixes it at
six. The action discriminator is the established mechanism for growth; nesting a second one would
make the schema unlearnable for the agents that read it.

### D71 — No model or vector dependency in the correctness path; assistance is a recorded proposal

**Decision**: no language-model client, embedding library, vector store or graph database enters any
manifest (asserted by test, SC-321). Model assistance is confined to *proposing* — a topic key, an
equivalence, a promotion, a pattern's applicability text — and every proposal is recorded with
`basis = explicit_agent` and passes a deterministic gate before affecting stored state
(FR-512).

**Rationale**: Feature 001's FR-025 already holds this line and Constitution II forbids the
machinery. The distinction that makes the feature work is that intelligence proposes and Cairn
decides.

### D72 — No new crate. Five existing crates absorb the feature

**Decision**:

| Crate | Gains | Why here |
|---|---|---|
| `cairn-core` | domain enums, `knowledge.rs` (the pure reconciliation derivation), `verify.rs` (verifier specs and the state machine, no I/O), `continuity.rs` (staleness comparison, pure), `context.rs` extension, `budget.rs` reserve, wire types | pure data and pure functions; `cairn-server` depends on it and must keep compiling |
| `cairn-store` | one additive migration, `knowledge.rs`, `evidence.rs`, `patterns.rs`, `criteria.rs` repositories, `search.rs` filters | the single-writer boundary, unchanged |
| `cairn-git` | ref/commit/ancestry reads the `git_ref`, `git_commit` and branch-merge verifiers need | it is already the only Git adapter |
| `cairnd` | verifier execution (the only place with worktree access and the maintenance tick), drift marking on the capture path, checkpoint write and restore, subject queries feeding the briefing | the daemon owns I/O and scheduling |
| `cairn` | new CLI commands, MCP action dispatch, renderers | the CLI and MCP surface |
| `cairn-server` | one additive migration, allowlist extension, `sync_changes` arrays | the shared-data boundary |

**Rationale**: Feature 002 needed a new crate because vendor parsing could not live in an I/O-free
core and the fixture corpus had to be testable without a daemon. Neither applies here: the
reconciliation derivation is pure and belongs in `cairn-core` beside `context.rs`, and verifier
execution needs the daemon's worktree access. Adding a crate would split one boundary across two
homes for no testability gain.

### D73 — A deterministic corpus, five tiers, and model-judged evaluation outside every gate

**Decision**: `tests/knowledge/` holds the corpus as JSON fixtures — paired positive/negative
reconciliation cases, conflict cases, offline-merge cases, verification and drift transition cases,
budget cases, compaction cases, staleness cases, pattern cases, and an adversarial privacy corpus.
Tiers, gates and the metric list are in [contracts/evaluation.md](./contracts/evaluation.md).

No release gate reads a model's judgement (SC-325). If a semantic-equivalence evaluation is ever
wanted, it lives in a non-gating harness with its trust boundary stated, and it may inform the
corpus but never the pass/fail.

**Rationale**: the brief is explicit that "feels smarter" is not a criterion. A paired
positive/negative corpus is what makes "zero false merges" measurable rather than aspirational.

### D74 — One additive local migration, one additive server migration, no rewrites

**Decision**: `0005_project_intelligence.sql` (local) and `0002_project_intelligence.sql` (server).
Existing migrations are untouched (FR-513). Defaults on migration: `verification = 'unverified'`,
`verification_authority` NULL (D76), `topic_key`/`value_key` NULL, `importance = 'normal'`,
`pinned = 0`, `effective_from = created_at`, `superseded_at` set from existing superseded rows'
`updated_at` **only where `state = 'superseded'`** — the one derived backfill, and it is the best
available approximation, recorded as such. `reinforcement_count = 0`,
`distinct_origin_count = 1`. Existing `acceptance_criteria` entries become `task_criteria` rows in
position order with `state = 'pending'`, `verification = 'unverified'`; `tasks.local_revision = 1`.

Nothing is fabricated (FR-515). Full procedure and its proof in [migration.md](./migration.md).

**Rationale**: the schema-version guard (`migrate.rs`) already prevents an older build writing a
newer schema, so the only obligation is that the forward step is lossless and that every new column
has an honest default. `superseded_at` is the single place where a default is inferred rather than
known, and it is documented rather than silent.

### D75 — Every bound has a documented, configurable default

**Decision**: added to `CairnConfig`, all asserted by test (FR-500, SC-320).

| Setting | Default | Bounds |
|---|---|---|
| `min_safe_context_fraction` | `0.40` | Level 0 reserve share |
| `min_context_budget_tokens` | `600` | below this, Level 0 truncates in D58's order |
| `pin_budget_project` | `12` | pins per project |
| `pin_budget_per_scope` | `4` | pins per scope+key |
| `pins_in_context_max` | `4` | pins admitted to Level 0 |
| `warnings_in_context_max` | `5` | divergence + conflict + drift admitted |
| `patterns_in_context_max` | `2` | patterns admitted to Level 1 |
| `reconcile_members_max` | `64` | subject members examined per write |
| `subject_warning_scan_max` | `256` | applicable topic-keyed memories scanned for warnings |
| `evidence_lookups_per_event_max` | `8` | evidence lookups per captured observation |
| `verify_pass_evidence_max` | `200` | facts examined per background pass |
| `verify_pass_runs_max` | `50` | verifier runs per background pass |
| `verify_pass_wall_ms` | `2000` | wall-clock share per background pass |
| `evidence_value_max_bytes` | `256` | stored observed value, after redaction |
| `evidence_locator_max_bytes` | `256` | stored source locator |
| `pattern_signals_min` | `2` | minimum specificity for promotion and for matching |

**Rationale**: the brief requires bounded behaviour everywhere; an unnamed bound is an unasserted
one. Putting them in the existing config file means an operator can move one without a rebuild, and
a test can assert the default did not drift.

---

## 7. Reconciliation-pass decisions (D76–D83)

Eight decisions taken during the design reconciliation pass. Four resolve HIGH findings, four MEDIUM.
Each was verified against the artifacts *and* the implementation before being accepted; the disposition
is in [plan.md](./plan.md) §Reconciliation.

### D76 — Verification carries an **authority**, and `verification_origin` is retired

**Decision**: replace `verification_origin ∈ {local, remote}` with
`verification_authority ∈ {cairn, attested, remote_cairn, remote_attested}`. Derived from the runs that
established the state and the `collector` of the evidence each consulted. Present on every surface that
shows a state, and on the wire as `cairn`/`attested` inside the existing verification object (now five
keys). Strict consumers — criterion verification (FR-484) and promotion (FR-396) — accept `cairn` only.

**Rationale**: the gap was real and it was on the wire. An agent attests a `test_outcome`, the memory
becomes `verified`, and it synced as `{state: verified, basis: ["test_outcome"]}`. A peer stored
`verification_origin = 'remote'` and rendered it exactly like a peer that had actually run the tests.
`basis` could not close it, because `test_outcome` and `command_outcome` are each reachable both ways —
from a captured observation, or from an agent's submission. `collector` lived on the evidence fact, which
never syncs, so the label did not survive the boundary.

**Alternatives considered**: `attested` as a fifth *verification state* — rejected, it conflates *what
was established* with *how*, and it loses the ability to say "attested and now needs recheck", since an
attested claim's fingerprint can change like any other. A third orthogonal enum — rejected as
unnecessary: `verification_origin` already existed to answer "where did this come from", so widening it
adds no column and no axis. Renaming was free because Feature 003 is unimplemented.

### D77 — Equal value keys no longer merge; `Corroborated` is a derived subject state

**Decision**: automatic merging requires equal `content_norm_digest`. Same subject + equal `value_key` +
differing content derives `Corroborated` — several answers that agree on the value — records **no**
relation, retains every statement, and reports the matched member to the writer. `reinforces` becomes
**explicit-only**.

**Rationale**: `auth.strategy=jwt` is an honest value key for both "HS256 with a shared secret" and
"RS256 with rotating public keys". The old rule wrote a `reinforces` relation and `derive_subject` then
collapsed the partition to a single representative — suppressing one of two materially different claims
from the canonical answer and reporting corroboration that never happened. Telling them apart requires
reading the content, which is inference (FR-317, FR-511).

So Cairn decides what it can — they agree on the value — and hands the rest to the party that can read
both. This is the proposal boundary applied one level deeper, and it makes automatic behaviour *smaller*:
one automatic rule instead of two, one relation kind demoted to explicit.

**Alternatives considered**: telling agents to write more specific keys (the brief's option A) — rejected
as unenforceable; the failure was silent, and correctness would rest on agent discipline. A token-
containment rule — "the content adds no token beyond the keys" — rejected as a heuristic dressed as a
rule: "PostgreSQL 16" versus "postgresql" would fail it, and it would fire constantly. Requiring an
explicit merge for *identical* content too — rejected, nothing is lost by collapsing byte-identical
claims, and it would cost the deduplication the feature exists for.

**Accepted cost**: three differently-worded equivalents no longer collapse without one explicit call.
The response names the member matched and the exact call, the usage contract asks for it, and if it never
comes the briefing still spends budget once and reports `+N further statements`. The failure mode moved
from *wrong* to *slightly less compact*, which is the right direction.

### D78 — Symmetric relation kinds normalize their endpoints

**Decision**: for `conflicts_with` only, `from = min(id)` and `to = max(id)` lexicographically before the
write. Every other kind is directional and is not normalized.

**Rationale**: the primary key is `(from, to, kind)`, so two machines detecting one conflict offline
produced `A→B` and `B→A` — two durable rows for one fact, both syncing, and `cairn memory subject`
reporting the same conflict twice. Normalization makes the primary key absorb the second machine's record
exactly as it absorbs a local duplicate.

**Alternatives considered**: a canonical-form check at read time — rejected, it leaves two rows in the
store and two rows on the wire. Making all kinds symmetric — rejected, `supersedes` direction *is* its
content.

### D79 — Checkpoints record a bounded fingerprint per relevant path

**Decision**: store one fingerprint per `relevant_path` at checkpoint time — `digest` (content SHA-256)
by default, `size` (existence + length) above the payload cap, `unknown` when excluded or unreadable —
and recompute on restoration. Per-path outcome is `unchanged | changed | removed | added |
not_fingerprintable`.

**Rationale**: the old rule detected a path change by looking for a `file_changed` observation from
another session, which misses every change Cairn did not see — a human editor, a formatter, `git apply`,
an IDE refactor — all of which leave the commit unmoved. Those are precisely the cases where a stale
"continue editing config.rs" is most dangerous.

Bounded: ≤32 paths (the existing cap), ≤`payload_cap_bytes` per path, no globbing, no directory walk, no
repository scan, no execution — and it runs on *restoration*, not on every session open, so FR-471 is
untouched.

**Alternatives considered**: `mtime` — rejected, it changes on checkout and `touch` without the content
changing, and a spurious divergence warning trains people to ignore warnings. A repository-wide scan —
rejected by FR-471 and by the non-goals. Watching the filesystem — rejected, a new long-running
platform-specific mechanism.

`not_fingerprintable` is deliberately not folded into `unchanged`: "I could not look" and "nothing moved"
are different answers.

### D80 — The task counter is local; cross-device identity is a derived digest

**Decision**: three changes together.

1. `tasks.revision` → `tasks.local_revision`: a monotone counter for **one store**, removed from the sync
   payload and from the server schema.
2. `task_state_digest`: derived, content-addressed over the converged records —
   `title_digest, goal_digest, status` ++ criteria sorted by `(ordinal, id)` ++ blockers sorted by `id`.
   Nothing stores it.
3. `sessions.task_revision_at_bind` → `sessions.task_snapshot_at_bind`: a bounded local JSON snapshot.
   The divergence change list is derived by **diffing it against the current synchronized records**, not
   by reading `task_changes`.

**Rationale**: the seven questions the brief poses each exposed the same error — treating a per-store
counter as a shared identity. Two offline machines each advancing 5→6 produce two different "revision 6"
states; the counter was in the payload and in the server schema, so an `expected_revision` could cross a
store boundary and be honoured against different content. And because `task_changes` is local, a
log-based divergence report described only this machine's edits and silently omitted a criterion the other
machine had changed — even though the criterion row itself had arrived.

The criteria and blockers already converged correctly by stable id; what was missing was an *identity for
the resulting state*, and a digest over converged records supplies it with no merge algebra. Removing the
counter from the wire is a **deletion** from the sync surface, not an addition.

**Alternatives considered**: a CRDT or a vector clock — rejected and unnecessary: disjoint criterion
changes never collide, so there is nothing to merge; the open question was identity, not resolution.
Syncing `task_changes` — rejected, it carries `prior_value`/`new_value` free text about local context and
the privacy default is conservative; and it is not needed, because diffing converged records is strictly
better information. A server-assigned revision — rejected, it would make task edits require the network,
breaking local-first.

### D81 — `blocked`: a fifth outbox state for capability refusal

**Decision**: classify a rejection as **content** (permanent → `failed`, unchanged) or **capability**
(`unknown_entity_type | unknown_field | schema_older` → `blocked`). `blocked` rows keep their payload and
idempotency key, carry `blocked_reason` and `blocked_at_capability`, and are excluded from `claim`. The
server's existing public `/api/version` gains `schema_version` and a `capabilities` array; the sync worker
probes it at most once per drain cycle, caches it in `sync_meta.server_capability`, and returns blocked
rows to `pending` when the capability changes.

**Rationale**: verified in the code — `outbox::claim` takes only `pending` and stale `in_flight` rows, and
`cairnd/src/sync.rs` calls `mark_failed` on a `rejected` status with the comment "`rejected` is permanent".
So a relation refused by an old server was `failed` forever, including after the server was upgraded. The
first design stopped at "stop sending that class", which is correct and insufficient.

The probe works against exactly the servers it must detect: an old server returns neither field, and that
**absence is the answer**. Delivery stays exactly-once because the original idempotency key is reused and
the server's `sync_state` claim is unchanged.

**Alternatives considered**: leaving rows `pending` and relying on the class filter — rejected, they would
be claimed and re-sent on every cycle, which is the futile retry FR-415 forbids. A new `blocked` table —
rejected, two nullable columns and one state on the existing queue is smaller. Requiring an operator
command — rejected by FR-418's "no manual repair".

### D82 — Narrow the temporal claim, and add `stale_at` going forward

**Decision**: the historical answer reconstructs **proposal effectiveness and explicit supersession
intervals**. Add nullable `stale_at`, set by the maintenance tick from now on; NULL means **unknown**,
never "not stale". Where a transition has no authoritative instant, the answer reports
`applicability: unknown` rather than an unbounded interval. Deleted memories are absent and reported as
deleted; their content is not reconstructable, which is what deletion means.

**Rationale**: `mark_stale_scopes` set only `state` and `updated_at`, so a memory that went stale at T2
still satisfied `superseded_at IS NULL` and was reported as effective for every T after T2. The claim was
stronger than the stored evidence. Adding a nullable column costs nothing and makes the answer precise
going forward; backfilling it from `updated_at` would be a **second** approximation on top of the one
already documented, and several paths touch `updated_at` on a stale row, so it would be a worse one.

**Alternatives considered**: a full bitemporal model — rejected by FR-345. Backfilling `stale_at` —
rejected as above. Narrowing only, with no new column — considered, and rejected because the column is one
nullable field that makes every future answer exact for free.

### D83 — Level 0 has a guaranteed O(1) tier and a bounded detail tier

**Decision**: split Level 0. **Tier 0a** is guaranteed and every field is O(1) in project and task size —
task, goal, status, derived progress counts, readiness, open-blocker count plus the single most actionable
blocker, next action with staleness, warning kinds with counts, repository state. **Tier 0b** is bounded
detail — warning detail, pins, criterion text in **action order** (`blocked → satisfied-unverified →
pending → verified → waived`), further blockers — admitted while budget allows, with omissions counted by
kind and a retrieval path.

**Rationale**: FR-443 said Level 0 "MUST contain acceptance criteria with their states" and SC-309 claimed
they are present at the documented minimum budget. Forty criteria at ~20 tokens is 800 tokens against a
600-token minimum: the requirement was unsatisfiable. The budget was never at risk — the admission order
already put criteria last and `try_spend` measures before emitting — but the *claim* was false.

Splitting the tier makes the guarantee true and keeps it meaningful: what an agent needs to continue is
the state, not the prose, and the state has a bounded worst case. Action order is the part that earns its
place — a blocked criterion is what stops progress, so it arrives first.

**Alternatives considered**: dropping the criteria guarantee entirely — rejected, criterion *states* are
continuity. Summarizing criterion text — rejected, that is generation, and Cairn derives rather than
writes. Raising the minimum budget until forty criteria fit — rejected, unbounded input cannot be fixed by
a larger constant.

---

## 6. Rejected scope

Recorded so it is not rediscovered.

| Rejected | Why |
|---|---|
| Event sourcing with projections | B1; a second transactional model the brief forbids |
| A materialized subject table | D44; buys nothing, adds an unrebuildable projection |
| Embeddings or a vector store for equivalence | FR-511, Constitution II, and a tuned threshold is not a rule |
| A graph database for relations | relations are rows with a documented derivation |
| Running the project's tests to verify | FR-365; turns a memory tool into an executor |
| Filesystem watching for drift | a new long-running platform-specific mechanism; the capture stream already reports changes |
| A `global` memory scope | Feature 002 FR-190; patterns are a different record, not a wider scope |
| Syncing patterns or evidence content | FR-502, FR-508; needs a consent model this feature does not have |
| Replacing `acceptance_criteria` with a join | D68; breaks three readers and the server for no gain |
| A `TaskRevision` snapshot entity | heavier than FR-488/FR-489 require |
| A seventh MCP tool | Feature 002 FR-128 |
| A new crate | D72; no testability or dependency-direction argument for one |
| Raising the default context budget | D58; silently increases every agent's cost to solve a prioritization problem |
| Autonomous promotion | FR-395; the brief asks for explicit promotion in the first version |
| Automatic merging on equal value keys | D77; a value key states a value, not a whole proposition |
| `attested` as a verification state | D76; conflates what was established with how, and loses recheck |
| A CRDT or vector clock for task state | D80; records already converge by identity — only the identity of the result was missing |
| Syncing the task change log | D80; free text about local context, and diffing converged records is better information |
| Backfilling `stale_at` from `updated_at` | D82; a second approximation, and a worse one than the first |
| `mtime` as a path fingerprint | D79; changes without the content changing, and false warnings train people to ignore warnings |
| Summarizing criterion text to fit a budget | D83; that is generation, and Cairn derives rather than writes |
| Code intelligence, symbol graphs, source RAG | spec Out of Scope; separate future work |
