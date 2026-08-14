# Contract: Durable Records, Concurrency and Rebuild

**Feature**: `003-project-intelligence`

This is the contract the brief asked for as an "event catalog", adjusted to what Cairn actually is.

**Cairn has no event log and no event-sourced projection** (research B1). Its durable model is
direct-state tables plus one append-only delivery queue. So the obligation is not event replay — it is
that **every derived value is rebuildable from durable records by a documented deterministic
procedure** (D43, FR-302, FR-517).

Introducing an event log to satisfy the letter of "replay" would create a second transactional model,
which the brief forbids and Constitution II rejects.

## Record catalog

Nine durable record types own the feature's correctness. For each: who may write it, what makes it
idempotent, and how it behaves under concurrency.

| Record | Owner (writer) | Identity / idempotency | Append-only | Ordering matters? |
|---|---|---|---|---|
| `memories` (row) | daemon, per request | `id` (UUIDv7) | rows yes | no — distinct proposals are independent |
| `memory_relations` | daemon, per request | PK `(from, to, kind)` | yes | no — the derivation is order-independent |
| `evidence_facts` | daemon (collector), agent (attested) | `id` | yes | no |
| `memory_evidence_facts` | daemon | PK `(memory, evidence, role)` | yes | no |
| `verification_runs` | daemon only | `id` | yes | yes — latest by `checked_at` sets the cached state |
| `continuity_checkpoints` | daemon only | `id` | yes | yes — latest by `created_at` is restored |
| `task_criteria` | daemon, per request | `id`; `revision` for CAS | state mutable, every change logged | per criterion only |
| `task_blockers` | daemon, per request | `id`; one transition | yes | no |
| `task_changes` | daemon, in the same transaction | `id`; `(task_id, revision)` unique | yes | yes — the revision sequence |
| `pattern_applications` | daemon, per request | unique `(pattern, project, signal_digest)` | yes | no |

**Aggregate ownership.** Three aggregates, each with one writer and one transaction boundary:

| Aggregate | Root | Contains | Transaction rule |
|---|---|---|---|
| Knowledge | `(project, scope, scope_key, topic_key)` | memories, relations, evidence links, verification runs | A proposal and its automatic relation commit together |
| Task work state | `tasks.id` | criteria, blockers, changes, revision, the retained projection | Any change writes the criterion/blocker, the change row, the revision bump and the projection in one transaction |
| Continuity | `sessions.id` | checkpoints | A checkpoint and its anchoring handoff commit together |

The daemon remains the single writer to the local store (Feature 001). Nothing here changes that.

## Ordering

Cairn has no global order and does not need one. Where order matters it is **local and explicit**, not
implied by arrival:

| Needs order | Order used | Why it is safe |
|---|---|---|
| Which verification run set the cached state | `checked_at DESC`, then `id DESC` | Runs are local-only; there is one writer and one clock |
| Which checkpoint is restored | `created_at DESC` | Same |
| The task revision sequence | `tasks.revision`, monotone under `BEGIN IMMEDIATE` | One writer, one counter |
| Stable output ordering of a conflicted answer set | `id ASC` | Presentation only, never arbitration (D49) |

**Nothing that syncs depends on order.** Relations are order-independent by construction: applying a
set of relations in any sequence yields the same `SubjectView`, because `derive_subject` consumes them
as a set. Asserted by `relation_order_invariance`, which shuffles the relation set and compares.

That property is what makes cross-device merge simple: there is no ordering authority to elect.

## Idempotency

Three layers, all existing mechanisms:

1. **Record identity.** `memory_relations` on `(from, to, kind)`, `memory_evidence_facts` on
   `(memory, evidence, role)`, `pattern_applications` on `(pattern, project, signal_digest)`,
   `task_changes` on `(task_id, revision)`. Recording twice changes nothing (FR-305, I2).
2. **Delivery.** The existing outbox idempotency key —
   `digest(entity_type:entity_id:operation:digest(payload))` — with the server's
   `sync_state` claim under `ON CONFLICT DO NOTHING`. Unchanged (FR-414).
3. **Import.** `INSERT OR IGNORE` on the primary key for relations, upsert by id for criteria and
   blockers. Re-importing a full history converges to the same state.

## Concurrency

The local mechanism is unchanged: `BEGIN IMMEDIATE` taking the write lock as the transaction opens,
with bounded retry (10 attempts, 2 ms → 400 ms ceiling) before any work is done, so a retry replays
nothing and can duplicate nothing (`cairn-store/src/tx.rs`). Feature 003 adds no second mechanism
(FR-414).

| Situation | Outcome |
|---|---|
| Two sessions propose incompatible knowledge concurrently | Two memory rows, distinct ids. Both survive. The subject derives `Conflicted`. No lost write, no winner (FR-336, SC-303) |
| Two sessions record the same relation concurrently | One row. The second collides on the PK and is ignored |
| Two sessions reinforce the same memory concurrently | Two `reinforces` rows from different sources; `distinct_origin_count` = 2. Correct by construction |
| Two sessions update different criteria concurrently | Both apply, different rows. `tasks.revision` advances twice; two change rows (SC-317) |
| Two sessions update the same criterion, both supplying `expected_revision` | One applies; the other is refused `revision_conflict` with the current state named (FR-337) |
| Two sessions update the same criterion, neither supplying a revision | Both apply in lock order; both are recorded in `task_changes` with `blind_write = true`. Nothing is lost, and the overwrite is visible (see [task-model.md](./task-model.md)) |
| Two sessions pin concurrently at the budget edge | The lock serializes them; the second is refused `pin_budget_exhausted` |
| The background verification pass and an on-demand verify overlap | Concurrency 1 on the pass; the on-demand run proceeds and both append runs. The cached state reflects the latest `checked_at` |
| Two drainers claim outbox rows | Unchanged: `UPDATE … RETURNING` gives disjoint sets, 60-second stale-claim expiry |
| A merge arrives while a local write is in flight | Serialized by the write lock. Both persist; the derivation runs after |

**32-way concurrency test** (SC-303): 32 processes propose against one subject; the assertion is 32
persisted proposals, zero lost writes, and a derivation whose outcome does not depend on commit order.

## Rebuild procedures

Every derived value has one. Each is a pure function of durable records, and each is exercised by
`rebuild_equivalence`, which discards the stored value, recomputes it, and asserts equality
(FR-517, I20, SC-324).

| Procedure | Rebuilds | From | Invoked |
|---|---|---|---|
| `derive_subject` | canonical answers, reconciliation state | active topic-keyed memories + relations | every read (nothing stored) |
| `rebuild_supersession(project)` | `memories.state = 'superseded'`, `superseded_by_id`, `superseded_at` | `supersedes` relations | after import; on demand; in the test |
| `rebuild_reinforcement(memory)` | `reinforcement_count`, `distinct_origin_count` | `reinforces` + `duplicates` relations, and the origin sessions of the memories they come from | after import; on demand |
| `rebuild_verification(memory)` | `verification`, `last_verified_at` | latest `verification_runs` + current evidence fingerprints | after import; after a fingerprint change; on demand |
| `rebuild_criteria_projection(task)` | `tasks.acceptance_criteria` | `task_criteria.text` in ordinal order | same transaction as any criterion change; on demand |
| `rebuild_pattern_trust(pattern)` | `reusable_patterns.trust` | the gate outcome + `pattern_applications` | after any application; on demand |
| `derive_progress(task)` | progress counts, `completion_readiness` | criteria + blockers | every read (nothing stored) |
| `classify_checkpoint(checkpoint)` | `checkpoint_state`, divergences | assumptions vs Git, task revision, observations | every restore (nothing stored) |

Exposed as `cairn doctor --rebuild-derived [--project <id>]`, which recomputes every derived value,
reports any that differed, and writes the corrected value. A difference is a bug report, not a normal
outcome — the command prints how many differed and exits non-zero if any did.

## Fail-closed behaviour

| Condition | Behaviour |
|---|---|
| A derived value disagrees with its durable inputs | The durable inputs win. The derived value is rebuilt and the discrepancy is logged (FR-478, FR-518) |
| A derived value cannot be computed (storage busy, corrupt row) | Report **unavailable**, never a guess. Context marks `degraded: true` — Feature 001's existing flag |
| `memory_relations` names a memory that does not exist | The relation is ignored by the derivation and reported by `doctor`. It is never deleted automatically — it may be a memory that has not synced yet |
| A relation set implies mutual supersession (A supersedes B and B supersedes A) | The derivation refuses to resolve, reports `Conflicted`, and `doctor` names the cycle. Writing the second such relation is refused with `relation_conflict` |
| A `verification_run` references a deleted evidence fact | The run stays as history; the cached state rebuilds to `needs_recheck` |
| A checkpoint's handoff is deleted | The checkpoint is tombstoned with it (deletion cascade) |
| The store is unreadable | Existing Feature 001 behaviour: the damaged store is reported as damaged, not as "the daemon did not start" |
| A migration is interrupted | The existing per-migration transaction rolls it back; `schema_migrations` is unchanged, and the next start retries |

## Migration and replay-equivalence

The full procedure is in [migration.md](../migration.md). Two properties belong here:

1. **The forward step is additive and transactional.** One local migration, one server migration,
   each applied inside the existing per-migration transaction with its `schema_migrations` row.
   Existing migrations are untouched (FR-513).
2. **Post-migration state equals rebuild.** After migrating an alpha.4 store, running every rebuild
   procedure produces the same values the migration wrote — for every value except
   `superseded_at`, whose backfill from `updated_at` is the feature's single documented approximation
   (D74). That exception is asserted explicitly rather than excluded silently:
   `migration_alpha4::rebuild_matches_migration_except_superseded_at`.

The existing schema-version guard is unchanged: a build refuses a database whose version exceeds what
it supports, rather than writing against a schema it does not understand (FR-516).

## What is deliberately absent

| Not built | Why |
|---|---|
| An event log | B1; a second transactional model the brief forbids |
| A global sequence number or vector clock | Nothing that syncs depends on order (`relation_order_invariance`) |
| A materialized subject projection | D44; the derivation on read is cheaper than the invalidation it would need |
| Conflict resolution by version vector | FR-334 — a conflict is *reported*, not resolved. A vector clock would tell us which write was concurrent, which we already know; it would not tell us which answer is true, which is the actual question |
| Snapshotting or compaction of the record set | The record volumes are bounded by project size; the derivation is a small indexed query |
