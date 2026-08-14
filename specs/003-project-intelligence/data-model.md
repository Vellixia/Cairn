# Data Model: Cairn Project Intelligence

**Feature**: `003-project-intelligence` | **Date**: 2026-08-14
**Baseline**: `main` @ `0b79b31` — local schema version 4, server schema version 1

Every change here is **additive**. No existing column changes type or meaning, no existing migration
is edited, and no existing row is rewritten except to fill a new column with its documented default
(see [migration.md](./migration.md)).

Lowercase text enums with `CHECK` constraints, UUIDv7 identifiers, RFC 3339 timestamps in `TEXT`
locally and `TIMESTAMPTZ` on the server — the conventions Feature 001 established, unchanged.

---

## 1. What is derived and what is durable

This distinction governs the whole model. Feature 003 has **no event log** (research B1); the
obligation is that everything derived is rebuildable from durable records (D43).

**Durable** — the source of truth. Append-only unless noted.

| Record | Table | Append-only | Syncs |
|---|---|---|---|
| Knowledge proposal | `memories` (existing, extended) | rows yes; four cached columns and the Feature 001 state view are maintained | yes |
| Reconciliation decision | `memory_relations` | yes | yes |
| Evidence fact | `evidence_facts` | yes (soft-deleted) | **no** |
| Evidence link | `memory_evidence_facts` | yes | **no** |
| Verification run | `verification_runs` | yes | **no** |
| Continuity checkpoint | `continuity_checkpoints` | yes (restore counters mutate) | **no** |
| Reusable pattern | `reusable_patterns` | trust and counts derived; text mutable by explicit edit | **no** |
| Pattern application | `pattern_applications` | yes | **no** |
| Task criterion | `task_criteria` | state/text mutable, every change logged | yes |
| Task blocker | `task_blockers` | yes, with one `cleared` transition | yes |
| Task change | `task_changes` | yes | **no** |
| Criterion evidence link | `criterion_evidence` | yes | **no** |

**Derived** — computed, never authoritative, always rebuildable.

| Derived value | Computed from | Rebuild |
|---|---|---|
| `SubjectView` (canonical answer, reconciliation state) | active topic-keyed `memories` + `memory_relations` | on every read; nothing stored |
| `memories.state` = `superseded`, `memories.superseded_by_id` | the `supersedes` relation | `rebuild_supersession(project)` |
| `memories.reinforcement_count` | count of inbound `reinforces` + `duplicates` relations | `rebuild_reinforcement(memory)` |
| `memories.distinct_origin_count` | distinct `origin_session_id` over the memory and its reinforcing memories | `rebuild_reinforcement(memory)` |
| `memories.verification`, `last_verified_at` | latest `verification_runs` row + evidence fingerprint comparison | `rebuild_verification(memory)` |
| `memories.verification_authority` | the runs that established the state, and the `collector` of the evidence each consulted | `derive_authority(memory)` |
| `tasks.acceptance_criteria` | `task_criteria` text in ordinal order | `rebuild_criteria_projection(task)` |
| `task_state_digest` | title, goal, status + sorted criteria and blockers | `derive_task_state_digest(task)` — nothing stored |
| Task progress, `completion_readiness` | `task_criteria` + `task_blockers` | on every read; nothing stored |
| `reusable_patterns.trust` | the gate outcome + `pattern_applications` | `rebuild_pattern_trust(pattern)` |
| `checkpoint_state`, divergences | checkpoint assumptions vs current Git, the derived task state digest, and recomputed path fingerprints | on every restore; nothing stored |

Rebuild procedures and their equality test are specified in
[contracts/records-and-rebuild.md](./contracts/records-and-rebuild.md).

---

## 2. Extended entities

### 2.1 `memories` — additive columns

Existing columns unchanged: `id`, `project_id`, `type`, `scope`, `scope_key`, `content`, `state`,
`superseded_by_id`, `origin_session_id`, `local_only`, `created_at`, `updated_at`, `deleted_at`.

| Column | Type | Default | Syncs | Meaning |
|---|---|---|---|---|
| `topic_key` | TEXT NULL | NULL | yes | Normalized subject identity (D45). NULL = free-form |
| `value_key` | TEXT NULL | NULL | yes | Normalized comparable value. Only valid with `topic_key` |
| `content_norm_digest` | TEXT NULL | NULL | no | SHA-256 of normalized content, for exact duplicate detection (D46) |
| `importance` | TEXT | `'normal'` | yes | `low \| normal \| high` — within-bucket ordering only (FR-308) |
| `verification` | TEXT | `'unverified'` | yes | `unverified \| verified \| needs_recheck \| drifted \| conflicted` |
| `verification_authority` | TEXT NULL | NULL | **yes**, as `cairn`/`attested` | `cairn \| attested \| remote_cairn \| remote_attested`. Derived from the runs that established the state; NULL when not `verified` (FR-370) |
| `last_verified_at` | TEXT NULL | NULL | yes | Instant of the run that produced the current state |
| `effective_from` | TEXT | `created_at` | yes | Start of the interval this memory was current |
| `superseded_at` | TEXT NULL | NULL | yes | End of that interval; set with the `supersedes` relation |
| `stale_at` | TEXT NULL | NULL | yes | Set by the maintenance tick when the scope key stops resolving. NULL means **unknown**, never "not stale" (FR-341, D82) |
| `pinned` | INTEGER | `0` | yes | Protected invariant (FR-451) |
| `pinned_at` | TEXT NULL | NULL | yes | |
| `pinned_by_session` | TEXT NULL | NULL | yes | |
| `pin_reason` | TEXT NULL | NULL | yes | Bounded, redacted |
| `reinforcement_count` | INTEGER | `0` | yes | Derived; count of reinforcing decisions |
| `distinct_origin_count` | INTEGER | `1` | yes | Derived; distinct origin sessions. **Never presented as a verification count** (FR-406) |

**Constraints** — enforced **in code** at the repository boundary, not in DDL:

- `value_key IS NULL OR topic_key IS NOT NULL`
- `importance IN ('low','normal','high')`
- `verification IN ('unverified','verified','needs_recheck','drifted','conflicted')`
- `verification_authority IS NULL OR verification_authority IN ('cairn','attested','remote_cairn','remote_attested')`
- `verification <> 'verified'` implies `verification_authority IS NULL`
- `pinned IN (0,1)`; `pinned = 0` implies `pinned_at`, `pinned_by_session`, `pin_reason` all NULL
- `superseded_at IS NULL OR state = 'superseded'`

Why not DDL: SQLite cannot add a `CHECK` to an existing table without rebuilding it, which would
rewrite every row in a user's `memories` table and violate FR-513's literal reading. The predicates
above are enforced at the same boundary that would have raised the constraint error, and each is
asserted by test. **New** tables carry their `CHECK` constraints in DDL as usual, per the Feature 001
convention. Recorded as a deliberate deviation in
[compatibility.md](./compatibility.md) §Open notes 1 and in [migration.md](./migration.md) §Step 1.

**Indexes**

- `memories_topic ON (project_id, topic_key, state) WHERE topic_key IS NOT NULL` — the subject query
- `memories_subject ON (project_id, scope, scope_key, topic_key) WHERE topic_key IS NOT NULL`
- `memories_verification ON (project_id, verification) WHERE verification <> 'unverified'`
- `memories_pinned ON (project_id, scope, scope_key) WHERE pinned = 1`
- `memories_temporal ON (project_id, effective_from, superseded_at)`
- `memories_content_norm ON (project_id, content_norm_digest) WHERE content_norm_digest IS NOT NULL`

**FTS is untouched.** `memory_fts` remains an external-content table over `content` with its three
triggers (research B7). `topic_key` is matched by exact or prefix SQL filter, not by FTS.

### 2.2 `tasks` — additive column

| Column | Type | Default | Syncs | Meaning |
|---|---|---|---|---|
| `local_revision` | INTEGER | `1` | **no** | Monotone counter for **this store only**; advances on any local change to the task, its criteria or its blockers. Never transmitted, never a shared identity (FR-488, D80) |

`acceptance_criteria` is **retained** as the ordinal-ordered projection of `task_criteria.text`
(D68). It is rewritten in the same transaction as any criterion change.

### 2.3 `sessions` — additive column

| Column | Type | Default | Syncs | Meaning |
|---|---|---|---|---|
| `task_snapshot_at_bind` | TEXT NULL | NULL | no | Bounded JSON snapshot of the task state this session bound at, in the same shape as `continuity_checkpoints.criteria_snapshot`. The bind-time state digest is derived from it, so one column carries both (FR-489, D80) |

Set by `bind_task` and by `start_session` when a task is supplied. Local: it is a fact about this
machine's session, and Feature 001 already keeps session-local fields off the server.

---

## 3. New entities

### 3.1 `memory_relations` — reconciliation decisions

The whole reconciliation record, and the reason no canonical row exists (D47).

| Column | Type | Notes |
|---|---|---|
| `from_memory_id` | TEXT | PK part. The subject of the relation |
| `to_memory_id` | TEXT | PK part. The object |
| `kind` | TEXT | PK part. `reinforces \| duplicates \| supersedes \| conflicts_with \| narrows \| not_applicable_to` |
| `project_id` | TEXT | Denormalized for scoping and indexing |
| `decided_by_session` | TEXT | Mandatory provenance (FR-304) |
| `decided_at` | TEXT | Recorded, **never read by the derivation** (D49) |
| `basis` | TEXT | `deterministic_rule \| evidence \| explicit_agent \| explicit_user` |
| `basis_evidence_id` | TEXT NULL | Required when `basis = 'evidence'`; local-only reference |
| `rationale` | TEXT NULL | Bounded (≤512 bytes), redacted |
| `deleted_at` | TEXT NULL | Tombstone only; a decision is never edited |

**Primary key** `(from_memory_id, to_memory_id, kind)` — this is what makes recording the same
decision twice a no-op, on one machine and across a merge (FR-305, FR-336).

**Symmetric kinds normalize their endpoints before the write.** `conflicts_with` is the one kind whose
meaning has no direction, so `from = min(id)` and `to = max(id)` lexicographically. Without that, two
machines detecting one conflict while offline produce `A→B` and `B→A` — two durable rows for one fact,
both syncing, and the same conflict reported twice. Every other kind is directional and is **not**
normalized (D78, [knowledge.md](./contracts/knowledge.md) §Symmetric relation normalization).

**Semantics**

| Kind | Direction | Meaning | Automatic? |
|---|---|---|---|
| `reinforces` | new → existing | Same subject, equal value key | yes (D46) |
| `duplicates` | new → existing | Same subject, identical normalized content | yes (D46) |
| `supersedes` | new → old | The new memory replaces the old | **never** (FR-325) |
| `conflicts_with` | either → either | Two applicable answers disagree. Symmetric: recorded once, read both ways | detected automatically, **resolved never** |
| `narrows` | narrow → broad | Documented scope exception (FR-333) | no; absence does not create a conflict |
| `not_applicable_to` | memory → memory | This knowledge does not apply in the other's context | no |

Recording `supersedes` also, in the same transaction: sets the old memory's `state = 'superseded'`,
`superseded_by_id`, `superseded_at`; clears the old memory's pin (D59, FR-456).

**Indexes**: `(project_id, kind)`, `(to_memory_id, kind)`, `(project_id, basis_evidence_id)`.

**Syncs**: yes, as outbox entity `memory_relation`. `basis_evidence_id` is **stripped from the
payload** — the evidence it names is local (FR-502). The receiving peer sees `basis = 'evidence'`
with no identifier, which is honest: the decision was evidence-backed elsewhere.

### 3.2 `evidence_facts` — bounded observations of the world

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT | PK, UUIDv7 |
| `project_id` | TEXT | |
| `kind` | TEXT | `observation \| file \| git_ref \| configuration \| test_outcome \| command_outcome \| runtime_state \| schema_version` |
| `collector` | TEXT | `cairn \| agent` (D52). Determines what it may establish |
| `subject` | TEXT | Bounded label, ≤128 bytes, redacted. "database backend" |
| `observed_value` | TEXT | ≤`evidence_value_max_bytes` (256) **after redaction** (FR-354) |
| `value_digest` | TEXT | SHA-256 of the normalized observed value |
| `source_locator` | TEXT | ≤256 bytes. Repository-relative path, or a Git ref. **Never absolute** (FR-353) |
| `fingerprint` | TEXT | What change detection compares (D53 table) |
| `observation_id` | TEXT NULL | Set when `kind = 'observation'`, or when a test/command outcome came from a captured observation. The bridge to Feature 001 provenance |
| `repo_branch` | TEXT | |
| `repo_commit` | TEXT NULL | |
| `collected_at` | TEXT | |
| `collected_by_session` | TEXT | Mandatory provenance |
| `local_only` | INTEGER | Always `1` in this feature; the column exists so a future feature has a place to widen deliberately |
| `deleted_at` | TEXT NULL | Tombstone: identity and timestamps survive, value and locator cleared (Feature 001 deletion semantics) |

**Constraints**

- `kind IN (…)`, `collector IN ('cairn','agent')`
- `length(observed_value) <= 256`, `length(source_locator) <= 256`
- `source_locator` must not begin with `/`, `\`, or match `^[A-Za-z]:[\\/]` — enforced in code and
  asserted by test (FR-353)
- `collector = 'cairn'` requires either `observation_id IS NOT NULL` or a `kind` Cairn can read
  itself (`file`, `git_ref`, `git_commit`, `configuration`, `schema_version`)

**Indexes**: `(project_id, source_locator)` — the drift-marking lookup, capped by
`evidence_lookups_per_event_max`; `(project_id, kind)`; `(project_id, fingerprint)`.

**Syncs**: **no.** There is no outbox entity type and no server table (FR-502, FR-503).

### 3.3 `memory_evidence_facts` — evidence links with a role

| Column | Type | Notes |
|---|---|---|
| `memory_id` | TEXT | PK part |
| `evidence_id` | TEXT | PK part |
| `role` | TEXT | PK part. `supports \| contradicts` (FR-359) |
| `attached_at` | TEXT | |
| `attached_by_session` | TEXT | |

Survives deletion of the evidence fact so the reference reports "evidence deleted" rather than
disappearing, exactly as `memory_evidence` does for observations (FR-358, FR-505).

Feature 001's `memory_evidence(memory_id, observation_id, content_digest)` is **not modified**
(D51).

### 3.4 `verification_runs` — append-only deterministic checks

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT | PK |
| `memory_id` | TEXT NULL | One of `memory_id` / `criterion_id` is set |
| `criterion_id` | TEXT NULL | |
| `project_id` | TEXT | |
| `verifier` | TEXT | `file_exists \| file_digest \| git_ref \| git_commit \| configuration \| schema_version \| test_outcome \| command_outcome \| runtime_state` |
| `evidence_id` | TEXT NULL | The fact checked against |
| `expected_digest` | TEXT NULL | What the memory asserted |
| `observed_digest` | TEXT NULL | What the verifier found |
| `result` | TEXT | `verified \| drifted \| inconclusive` |
| `detail` | TEXT NULL | Bounded (≤256), redacted. Why inconclusive, or what differed |
| `repo_branch` | TEXT | The state it was checked at (FR-363) |
| `repo_commit` | TEXT NULL | |
| `checked_at` | TEXT | |
| `triggered_by` | TEXT | `background_pass \| on_demand \| attach` |

Append-only (FR-364). A later run never rewrites an earlier one; only the memory's or criterion's
cached state changes.

**Indexes**: `(memory_id, checked_at DESC)`, `(criterion_id, checked_at DESC)`,
`(project_id, result)`.

**Syncs**: **no.**

### 3.5 `continuity_checkpoints` — structured work state at a boundary

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT | PK |
| `session_id` | TEXT | |
| `project_id` | TEXT | |
| `handoff_id` | TEXT | The derived boundary record this anchors to (D55, FR-423) |
| `trigger` | TEXT | `context_compacting \| session_closed \| explicit` |
| `assumed_branch` | TEXT | Assumption set (FR-424) |
| `assumed_commit` | TEXT NULL | |
| `assumed_task_id` | TEXT NULL | |
| `assumed_task_state_digest` | TEXT NULL | The derived cross-device task state identity at that instant (D80) |
| `relevant_paths` | TEXT | JSON array, bounded to 32 repository-relative paths |
| `path_fingerprints` | TEXT | JSON array, one per relevant path: `{path, class, value}` with `class` ∈ `digest \| size \| unknown` (D79) |
| `criteria_snapshot` | TEXT | JSON: `[{criterion_id, state, verification}]` at that instant |
| `open_blockers` | TEXT | JSON array of blocker ids and bounded descriptions |
| `pinned_constraints` | TEXT | JSON array of memory ids in force at that instant |
| `next_action` | TEXT | Derived, bounded |
| `created_at` | TEXT | |
| `restored_at` | TEXT NULL | Last restoration |
| `restore_count` | INTEGER | Advances per restoration; the ten-compaction test reads it |
| `deleted_at` | TEXT NULL | Tombstoned with its session (FR-505) |

Everything except the assumption set, the snapshots and the counters is reachable through
`handoff_id`; the checkpoint deliberately does not copy it (D55).

**Indexes**: `(session_id, created_at DESC)`, `(project_id, created_at DESC)`.

**Syncs**: **no.**

### 3.6 `reusable_patterns` — project-independent transferable knowledge

**No `project_id` column.** That absence is the design (D61, FR-393).

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT | PK |
| `title` | TEXT | ≤128, sanitized |
| `problem` | TEXT | ≤1024, sanitized |
| `signals` | TEXT | JSON array of normalized tokens, 2–16 entries, each ≤128 (`pattern_signals_min`) |
| `signal_digest` | TEXT | SHA-256 over the sorted normalized signal set. Used for matching *and* duplicate detection — one representation (R4) |
| `applicability` | TEXT | JSON array of bounded condition strings |
| `root_cause` | TEXT | ≤1024, sanitized |
| `root_cause_digest` | TEXT | For duplicate detection with `signal_digest` |
| `approach` | TEXT | ≤2048, sanitized |
| `constraints` | TEXT | JSON array of bounded caveat strings |
| `trust` | TEXT | `candidate \| sanitized \| validated \| contested` — derived (D63) |
| `origin_ref` | TEXT | **Opaque local reference.** A digest of the source project id salted per machine; never a name, path or remote |
| `origin_deleted` | INTEGER | Set when the origin project or source memory is deleted (FR-399) |
| `source_memory_id` | TEXT NULL | Local only; cleared and `origin_deleted` set on deletion |
| `sanitization_report` | TEXT | JSON: which check classes ran and passed. Names classes, never values |
| `created_at`, `updated_at` | TEXT | |
| `deleted_at` | TEXT NULL | |

**Constraints**: `trust IN (…)`; `json_array_length(signals) BETWEEN 2 AND 16`.

**Indexes**: `signal_digest`, `(trust)`, unique `(signal_digest, root_cause_digest)
WHERE deleted_at IS NULL` — which is how `duplicate_pattern` refusal is enforced structurally.

**Syncs**: **no.** No outbox entity type, no server table (FR-508).

### 3.7 `pattern_applications` — outcomes, independence and counterexamples

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT | PK |
| `pattern_id` | TEXT | |
| `project_id` | TEXT | The applying project. Local, never leaves |
| `session_id` | TEXT | |
| `signal_digest` | TEXT | The signal set of *this* incident |
| `outcome` | TEXT | `resolved \| not_applicable \| failed` |
| `discovery` | TEXT | `independent \| cairn_suggested` (D63) |
| `alternative_cause` | TEXT NULL | Bounded. Present on `not_applicable` where known (FR-404) |
| `evidence_id` | TEXT NULL | Deterministic evidence collected in *this* project |
| `is_origin` | INTEGER | True when `project_id` is the pattern's origin. Excluded from trust |
| `applied_at` | TEXT | |

**Unique** `(pattern_id, project_id, signal_digest)` — one incident counts once however many
sessions touch it. This is the anti-poisoning mechanism (SC-314).

**Syncs**: **no.**

### 3.8 `task_criteria` — stably identified acceptance criteria

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT | PK, UUIDv7. **Stable across every update** (FR-481) |
| `task_id` | TEXT | |
| `ordinal` | INTEGER | Position; determines `label` and the retained projection order |
| `label` | TEXT | `AC-<n>` derived from `ordinal` at creation and **not renumbered** when another criterion is added |
| `text` | TEXT | Bounded, redacted |
| `state` | TEXT | `pending \| satisfied \| blocked \| waived` (FR-482) |
| `verification` | TEXT | `unverified \| verified \| failed` — distinct from `state` |
| `revision` | INTEGER | Per-criterion, for revision comparison (FR-490) |
| `created_at`, `updated_at` | TEXT | |
| `deleted_at` | TEXT NULL | |

**Constraints**: `state IN (…)`, `verification IN (…)`, unique `(task_id, ordinal)
WHERE deleted_at IS NULL`.

`verification = 'verified'` requires at least one `criterion_evidence` row whose evidence fact has
`collector = 'cairn'` — enforced in code, asserted by SC-328 (D69).

**Syncs**: yes, as outbox entity `task_criterion`.

### 3.9 `task_blockers` — append-only, with one cleared transition

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT | PK |
| `task_id` | TEXT | |
| `description` | TEXT | Bounded, redacted |
| `state` | TEXT | `open \| cleared` |
| `opened_by_session` | TEXT | |
| `opened_at` | TEXT | |
| `cleared_by_session` | TEXT NULL | |
| `cleared_at` | TEXT NULL | |
| `deleted_at` | TEXT NULL | |

Only transition: `open → cleared`. Reopening creates a new blocker (FR-485).

**Syncs**: yes, as outbox entity `task_blocker`.

### 3.10 `task_changes` — the append-only revision history

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT | PK |
| `task_id` | TEXT | |
| `local_revision` | INTEGER | The `tasks.local_revision` this change produced (local sequence only) |
| `kind` | TEXT | `goal_changed \| title_changed \| status_changed \| criterion_added \| criterion_text \| criterion_state \| criterion_verification \| criterion_removed \| blocker_opened \| blocker_cleared` |
| `subject_id` | TEXT NULL | Criterion or blocker id |
| `session_id` | TEXT | Author |
| `prior_value` | TEXT NULL | Bounded, redacted |
| `new_value` | TEXT NULL | Bounded, redacted |
| `blind_write` | INTEGER | True when the caller supplied no read revision (FR-490 note) |
| `changed_at` | TEXT | |

This is what makes FR-488's "no assertion is lost even when a later one replaces it" true, and what
the divergence report reads to say *what* changed between revisions 5 and 6.

**Indexes**: `(task_id, revision)`.

**Syncs**: **no.** It is diagnostic and provenance-rich; the peer receives the criteria and blockers
themselves.

### 3.11 `criterion_evidence`

| Column | Type | Notes |
|---|---|---|
| `criterion_id` | TEXT | PK part |
| `evidence_id` | TEXT | PK part |
| `attached_at` | TEXT | |
| `attached_by_session` | TEXT | |

**Syncs**: **no** (it names local evidence).

---

## 4. State transitions

### 4.1 Memory lifecycle — unchanged from Feature 001

```text
active ──── scope key no longer resolves (maintenance tick) ────▶ stale
active ──── a `supersedes` relation names it ───────────────────▶ superseded
stale  ──── a `supersedes` relation names it ───────────────────▶ superseded
```

`active`, `stale`, `superseded` and their triggers are exactly as they are today (FR-362). Feature
003 adds no lifecycle state and no lifecycle transition.

### 4.2 Memory verification — new, total, orthogonal to lifecycle

Full trigger table in [contracts/evidence-verification.md](./contracts/evidence-verification.md).

```text
unverified ──run:verified──▶ verified ──fingerprint changed──▶ needs_recheck
unverified ──run:inconclusive──▶ unverified
needs_recheck ──run:verified (same value)──▶ verified
needs_recheck ──run:different value──▶ drifted
needs_recheck ──run:inconclusive──▶ needs_recheck
drifted ──fingerprint changed──▶ needs_recheck
any ──supporting + contradicting evidence, or two verifiers disagree──▶ conflicted
conflicted ──fingerprint changed──▶ needs_recheck
```

Supersession does **not** change verification state: a superseded memory keeps its last verification,
which is what lets a historical query say what was verified then (D50, D53).

### 4.3 Subject reconciliation — derived, never stored

```text
Historical    every member superseded or stale        → no canonical answer
Settled       exactly one active member               → that member
Reinforced    several active, one value_key,
              all sharing one content_norm_digest     → one answer + duplication accounting
Corroborated  several active, one value_key,
              differing content                       → every distinct statement; the VALUE is
                                                        agreed, the statements are several
Conflicted    several active, differing value_key,
              same scope and scope key                → every competing answer, no winner
```

`Corroborated` is the state that closed the coarse-value-key false-merge path (D77). It is not a
conflict — the members agree — and it is not a merge — they say different things. Leaving it needs no
decision at all; collapsing it needs an explicit `reinforce` or `duplicates` from the party that can read
both statements.

Leaving `Conflicted` requires a recorded decision: `supersedes`, `narrows`, or a verification result
that distinguishes the members (FR-335). Cairn never leaves it on its own (FR-334).

### 4.4 Criterion

```text
state:        pending ⇄ satisfied,  pending ⇄ blocked,  any → waived
verification: unverified ⇄ verified,  unverified ⇄ failed
```

Independent axes. `state = 'satisfied'` with `verification = 'unverified'` is a normal, reported
combination — the "satisfied but unverified" bucket in derived progress (FR-483, US11 #4).

### 4.5 Blocker

```text
open → cleared        (terminal; reopening creates a new blocker)
```

### 4.6 Pattern trust

```text
candidate ──gate passes──▶ sanitized
sanitized ──≥1 distinct non-origin project resolved, independent or with local evidence──▶ validated
sanitized|validated ──any not_applicable or failed application──▶ contested
```

`contested` is not a demotion below the evidence — it reports both sides (FR-405). A `contested`
pattern that later gains a qualifying success stays `contested` while any counterexample stands.

### 4.7 Checkpoint

```text
current      every assumption matches
diverged     ≥1 divergence in {branch, commit, task, files}
unresolvable the task or worktree the checkpoint names no longer exists
```

---

## 5. Invariants

Each is asserted by a named test; the map is in [traceability.md](./traceability.md).

| # | Invariant | Enforced by |
|---|---|---|
| I1 | No canonical answer is stored, so none can be overwritten | No such column exists (D44) |
| I2 | Recording a decision twice changes nothing | `memory_relations` primary key |
| I3 | The subject derivation reads no clock and no identifier order for arbitration | `clock_swap_invariance`, code review of `derive_subject`'s inputs |
| I4 | A conflict is never resolved without a recorded decision | `derive_subject` has no branch that picks a winner |
| I5 | Reconciliation never rewrites proposal content | No `UPDATE memories SET content` anywhere outside the delete tombstone |
| I6 | Drift changes only verification state | `drift.rs` writes exactly `verification`, `last_verified_at` |
| I7 | `memories.state`/`superseded_by_id` always agree with the `supersedes` relations | Written in one transaction; `rebuild_supersession` equality test |
| I8 | Evidence content, verification runs, checkpoints, patterns, applications and task changes cannot reach the server | No outbox entity type, no server table (structural) |
| I9 | A stored evidence value is ≤256 bytes after redaction and carries no absolute path | Column constraint + locator check + `privacy_payloads` |
| I10 | A criterion is `verified` only on a local `cairn`-authority verification | Code check + SC-328 |
| I10a | An attested verification is never rendered or transmitted as a deterministic one | Authority on every surface and on the wire + SC-329 |
| I11 | `tasks.acceptance_criteria` equals the ordinal projection of `task_criteria` | Same transaction + `rebuild_criteria_projection` equality test |
| I12 | `tasks.local_revision` advances on every local task, criterion or blocker change | Same transaction + `task_changes` row count equality |
| I12a | Two stores holding the same converged task compute the same `task_state_digest` | `derive_task_state_digest` over sorted records + SC-330 |
| I12b | The local counter never appears in any payload or server column | Payload construction test + server schema |
| I13 | One pattern incident in one project counts once | `pattern_applications` unique key |
| I13a | One symmetric decision is one durable row, whichever machine wrote it | Endpoint normalization + PK |
| I14 | A pinned memory loses its pin when superseded | Same transaction as the `supersedes` relation |
| I15 | Pins never exceed their budget and are never silently cleared | Refusal path + `pin_budget_exhausted` |
| I16 | `estimated_tokens <= budget`, always | `Budget::try_spend` measure-before-emit, unchanged |
| I17 | Level 0 content cannot be displaced by Level 1 or Level 2 | Reserve accounting in `Budget` |
| I18 | Unspent reserve returns to the general pool | Reserve released after Level 0 completes |
| I19 | No verification work runs on the session-open path | `perf_intelligence` + absence of a call site |
| I19a | Tier 0a guaranteed work state is present at any budget ≥ the documented minimum | `us10_min_safe_context` + O(1) field set |
| I19b | Capability-refused outbox work is never `failed` and never stranded | `blocked` state + capability probe + SC-331 |
| I20 | Every derived value equals its rebuild | `rebuild_equivalence` |

---

## 6. Entity relationships

```text
Project ──1:N──▶ Memory ──1:N──▶ MemoryEvidence (observation refs, Feature 001, unchanged)
                   │
                   ├──1:N──▶ MemoryEvidenceFact ──N:1──▶ EvidenceFact ──0:1──▶ Observation
                   ├──1:N──▶ VerificationRun
                   ├──N:N──▶ Memory  via MemoryRelation (kind, basis, decided_by)
                   └──0:1──▶ ReusablePattern.source_memory_id   (local; cleared on delete)

(Project, scope, scope_key, topic_key) ──derived──▶ SubjectView      [no table]

Project ──1:N──▶ Task ──1:N──▶ TaskCriterion ──1:N──▶ CriterionEvidence ──N:1──▶ EvidenceFact
                   ├──1:N──▶ TaskBlocker
                   └──1:N──▶ TaskChange

Session ──1:N──▶ Handoff ──0:1──▶ ContinuityCheckpoint
Session ──0:1──▶ Task            (task_snapshot_at_bind records the state bound at; its digest is derived)

ReusablePattern ──1:N──▶ PatternApplication ──N:1──▶ Project   (local only)
```

Cardinality notes: a memory has at most one `supersedes` relation *out* (one successor) but may have
several *in* over time only through distinct predecessors; a `conflicts_with` relation is stored once
and read in both directions; a checkpoint always has a handoff, and a handoff has at most one
checkpoint.

---

## 7. Domain types added to `cairn-core`

Declared with the existing `text_enum!` macro so each gets `as_str`, `FromStr`, `ALL` and a
round-trip test, matching every existing enum.

```text
VerificationState      unverified | verified | needs_recheck | drifted | conflicted
VerificationAuthority  cairn | attested | remote_cairn | remote_attested
Importance             low | normal | high
RelationKind           reinforces | duplicates | supersedes | conflicts_with | narrows |
                       not_applicable_to
RelationBasis          deterministic_rule | evidence | explicit_agent | explicit_user
EvidenceKind           observation | file | git_ref | configuration | test_outcome |
                       command_outcome | runtime_state | schema_version
EvidenceCollector      cairn | agent
EvidenceRole           supports | contradicts
VerifierKind           file_exists | file_digest | git_ref | git_commit | configuration |
                       schema_version | test_outcome | command_outcome | runtime_state
VerifyResult           verified | drifted | inconclusive
VerifyTrigger          background_pass | on_demand | attach
Reconciliation         historical | settled | reinforced | corroborated | conflicted
CriterionState         pending | satisfied | blocked | waived
CriterionVerification  unverified | verified | failed
BlockerState           open | cleared
CheckpointTrigger      context_compacting | session_closed | explicit
CheckpointState        current | diverged | unresolvable
FingerprintClass       digest | size | unknown
PathOutcome            unchanged | changed | removed | added | not_fingerprintable
DivergenceKind         branch | commit | task | files
PatternTrust           candidate | sanitized | validated | contested
PatternOutcome         resolved | not_applicable | failed
PatternDiscovery       independent | cairn_suggested
ContextLevel           minimum_safe | relevant | on_demand
SelectionReason        scope_match | canonical_answer | verified | pinned | drift_warning |
                       conflict_warning | pattern_signal_match | checkpoint_assumption |
                       task_binding
OmissionReason         budget_exhausted | scope_mismatch | superseded | not_canonical |
                       level_2_only | pin_budget | cap_reached
TaskChangeKind         goal_changed | title_changed | status_changed | criterion_added |
                       criterion_text | criterion_state | criterion_verification |
                       criterion_removed | blocker_opened | blocker_cleared
CompletionReadiness    not_ready | ready_unverified | ready
ContinuityMode         automatic | agent_initiated | unavailable_automatic
```

`OutboxEntityType` gains exactly three variants: `MemoryRelation`, `TaskCriterion`, `TaskBlocker`
(D66). It still has no observation variant, and the existing
`outbox_cannot_carry_observations` test is extended to assert that no Feature 003 local-only entity
has one either.

---

## 7a. `outbox` — additive columns for recoverable capability refusal

Feature 003 adds one state and two columns to Feature 001's `outbox`. Nothing else about the queue
changes: the idempotency key, the claim mechanism, the stale-claim timeout and the drainers are
untouched (D81, FR-418).

| Column | Type | Default | Meaning |
|---|---|---|---|
| `blocked_reason` | TEXT NULL | NULL | The capability class that refused it: `unknown_entity_type \| unknown_field \| schema_older` |
| `blocked_at_capability` | TEXT NULL | NULL | The server capability fingerprint observed when it was blocked, so a change is detectable |

`state` gains `blocked`. The `CHECK` on `outbox.state` **can** be extended without rewriting rows only
by recreating the table, so — as with `memories` (§2.1) — the predicate is enforced in code and asserted
by test, and the existing `CHECK` continues to permit the four original values. A `blocked` row is
stored with the state string and is excluded from `claim` by an explicit predicate rather than by the
constraint.

`outbox_claimable` already indexes `(project_id, state, created_at)`, so excluding `blocked` costs
nothing and finding blocked rows to release is a single indexed scan.

| State | Claimable | Terminal |
|---|---|---|
| `pending` | yes | no |
| `in_flight` | on stale claim | no |
| `delivered` | no | yes |
| `failed` | no | yes — the **content** was refused |
| `blocked` | **no, until the server capability changes** | **no** — retained and deliverable later |

`sync_meta` gains `server_capability` (TEXT NULL) — the last capability fingerprint observed from
`/api/version`, and the value a change is compared against.

## 8. Server schema additions

`cairn-server/migrations/0002_project_intelligence.sql`. Additive only.

**`memories`** gains `topic_key`, `value_key`, `importance`, `verification`, `last_verified_at`,
`verification_basis JSONB` (verifier kind names only), `evidence_fact_count INTEGER`,
`effective_from`, `superseded_at`, `pinned BOOLEAN`, `reinforcement_count`,
`distinct_origin_count`.

**`tasks`** gains **nothing**. The local counter is not transmitted and the state digest is derived on
each side, so the server's `tasks` table is untouched (D80).

**New tables**: `memory_relations` (without `basis_evidence_id`), `task_criteria`, `task_blockers`.

**Not added, deliberately** — and their absence is the privacy boundary (I8):

`evidence_facts`, `memory_evidence_facts`, `verification_runs`, `continuity_checkpoints`,
`reusable_patterns`, `pattern_applications`, `task_changes`, `criterion_evidence`, and any
observations table.

The wire allowlist in `cairn-server/src/sync.rs` is extended with new forbidden field names —
`observed_value`, `source_locator`, `value_digest`, `fingerprint`, `relevant_paths`,
`sanitization_report`, `origin_ref`, `alternative_cause`, `pin_reason`, `rationale`,
`basis_evidence_id`, `detail`, `prior_value`, `new_value` — so a payload carrying evidence or
diagnostic content is refused on the wire rather than trusted not to exist (FR-506).
Full delta in [contracts/privacy-sync.md](./contracts/privacy-sync.md).
