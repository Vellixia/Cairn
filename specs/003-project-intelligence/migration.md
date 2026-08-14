# Migration Design: Cairn Project Intelligence

**Feature**: `003-project-intelligence`
**From**: v0.1.0-alpha.4 — local schema version **4**, server schema version **1**
**To**: local schema version **5**, server schema version **2**

Additive only. No existing migration is edited (FR-513). No user deletes their store, and no user
action is required (FR-514). Nothing is fabricated to fill a new column (FR-515).

## What an existing store actually contains

The migration is designed against the real thing, not a clean one. A v0.1.0-alpha.4 store in use
carries:

| Table | Notable existing state |
|---|---|
| `projects` | linked and unlinked, with and without `server_project_id` |
| `tasks` | `acceptance_criteria` as a JSON array of strings, possibly empty, possibly with duplicates |
| `sessions` | active, completed, interrupted; some with `handoff_pending = 1`; some tombstoned |
| `observations` | thousands; some tombstoned |
| `memories` | `active`, `stale` and `superseded`; `superseded_by_id` chains; `local_only` rows |
| `memory_evidence` | rows whose observation has been deleted |
| `handoffs` | all three triggers, including `recovered` |
| `outbox` | `pending`, `in_flight`, `delivered`, `failed` rows |
| `sync_meta` | a `pull_cursor` mid-stream |
| Feature 002 tables | `agent_integrations`, `installed_resources`, `resource_bindings`, `capability_evidence`, `migration_states`, `recovery_artifacts` |

Every one of these is left byte-identical except for the new columns' defaults and the two backfills
below.

## Local migration `0005_project_intelligence.sql`

Applied by the existing `migrate::run`, inside its own transaction, with its `schema_migrations` row —
so an interruption rolls it back entirely and the next start retries (`migrate.rs`, unchanged).

### Step 1 — columns on `memories`

```sql
ALTER TABLE memories ADD COLUMN topic_key             TEXT;
ALTER TABLE memories ADD COLUMN value_key             TEXT;
ALTER TABLE memories ADD COLUMN content_norm_digest   TEXT;
ALTER TABLE memories ADD COLUMN importance            TEXT NOT NULL DEFAULT 'normal';
ALTER TABLE memories ADD COLUMN verification           TEXT NOT NULL DEFAULT 'unverified';
ALTER TABLE memories ADD COLUMN verification_authority TEXT;
ALTER TABLE memories ADD COLUMN last_verified_at       TEXT;
ALTER TABLE memories ADD COLUMN effective_from         TEXT;
ALTER TABLE memories ADD COLUMN superseded_at          TEXT;
ALTER TABLE memories ADD COLUMN stale_at               TEXT;
ALTER TABLE memories ADD COLUMN pinned                INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN pinned_at             TEXT;
ALTER TABLE memories ADD COLUMN pinned_by_session     TEXT;
ALTER TABLE memories ADD COLUMN pin_reason            TEXT;
ALTER TABLE memories ADD COLUMN reinforcement_count   INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN distinct_origin_count INTEGER NOT NULL DEFAULT 1;
```

SQLite `ADD COLUMN` is O(1) — it writes the table header, not the rows. A store with 100,000 memories
migrates in milliseconds and the existing rows are not rewritten (FR-513's "existing rows MUST NOT be
rewritten" holds literally, not just semantically).

`CHECK` constraints cannot be added to an existing SQLite table without rebuilding it, which would
rewrite every row. They are therefore enforced in code for `memories`, with the same predicate a
`CHECK` would express, and asserted by test. New tables carry their `CHECK` constraints in DDL as
usual. This is recorded as a deliberate deviation from the Feature 001 convention, for the sake of not
rewriting a user's table.

### Step 2 — two backfills, both bounded and explicit

```sql
-- (a) Known, exact: a memory was effective from when it was created.
UPDATE memories SET effective_from = created_at WHERE effective_from IS NULL;

-- (b) Approximated, and the only one: for a memory already superseded before this
--     feature existed, `updated_at` is the best available record of when that happened,
--     because `supersede_memory` sets `state`, `superseded_by_id` and `updated_at`
--     together and nothing else touches a superseded row.
UPDATE memories SET superseded_at = updated_at
 WHERE state = 'superseded' AND superseded_at IS NULL;
```

**(b) is the feature's single derived approximation** (D74, R6). It is wrong only for a memory that
was superseded and then touched again by something else — in practice a delete tombstone, which clears
content anyway. Its consequence is bounded: `superseded_at` is read only by `as_of` historical
queries, so an imprecise value can misplace a *pre-existing* superseded memory in a historical
window. It cannot affect current knowledge, reconciliation, verification or context.

Recorded rather than silent, and asserted by
`migration_alpha4::rebuild_matches_migration_except_superseded_at`.

Deliberately **not** backfilled:

| Column | Left at | Why |
|---|---|---|
| `topic_key`, `value_key` | NULL | Inferring a subject from prose is exactly what FR-317 and D46 forbid |
| `content_norm_digest` | NULL | Computed lazily on next write or by `doctor --rebuild-derived`; a NULL simply means "no exact-duplicate detection for this row yet" |
| `verification` | `unverified` | The honest state. No evidence exists, so nothing is verified (FR-514) |
| `verification_authority` | NULL | Meaningless unless `verified`, and nothing is |
| `stale_at` | NULL | **Deliberately not inferred.** A memory already `stale` has no authoritative instant, and `updated_at` is a worse source here than for supersession — several paths touch it. NULL means *unknown*, which is exactly what a historical answer will say (FR-341, D82). Set going forward by the maintenance tick |
| `reinforcement_count` | 0 | No relations exist yet |
| `distinct_origin_count` | 1 | Exactly one origin session — which is true |
| `pinned` | 0 | No one has pinned anything |

### Step 3 — supersession relations from existing links

```sql
INSERT OR IGNORE INTO memory_relations
    (from_memory_id, to_memory_id, kind, project_id,
     decided_by_session, decided_at, basis, rationale)
SELECT s.id, m.id, 'supersedes', m.project_id,
       s.origin_session_id, m.updated_at, 'explicit_user',
       'migrated from Feature 001 superseded_by_id'
  FROM memories m
  JOIN memories s ON s.id = m.superseded_by_id
 WHERE m.superseded_by_id IS NOT NULL;
```

This is what makes existing supersessions visible to `derive_subject` and to sync. `basis` is
`explicit_user` because a Feature 001 supersession was always an explicit act — `supersede_memory` has
no automatic caller. `rationale` names the migration so the provenance is honest about where it came
from.

`decided_at` reuses `updated_at`, with the same caveat as backfill (b). It is recorded but never read
by the derivation (D49), so its imprecision has no effect on any outcome.

### Step 4 — new tables and indexes

`memory_relations`, `evidence_facts`, `memory_evidence_facts`, `verification_runs`,
`continuity_checkpoints`, `reusable_patterns`, `pattern_applications`, `task_criteria`,
`task_blockers`, `task_changes`, `criterion_evidence` — created with their `CHECK` constraints and
indexes as specified in [data-model.md](./data-model.md).

Plus the six new `memories` indexes, the task and session columns, and the recoverable-refusal columns:

```sql
ALTER TABLE tasks     ADD COLUMN local_revision        INTEGER NOT NULL DEFAULT 1;
ALTER TABLE sessions  ADD COLUMN task_snapshot_at_bind TEXT;

ALTER TABLE outbox    ADD COLUMN blocked_reason        TEXT;
ALTER TABLE outbox    ADD COLUMN blocked_at_capability TEXT;
ALTER TABLE sync_meta ADD COLUMN server_capability     TEXT;
```

`sessions.task_snapshot_at_bind` stays NULL for existing sessions. A session that bound before this
feature existed genuinely does not know the state it bound at, and synthesizing one would produce a false
divergence report. NULL means "unknown", and divergence is not reported for it.

The three refusal columns are nullable and default to NULL, so every existing queued, in-flight,
delivered and failed outbox row is untouched and stays exactly as claimable as it was. **No existing row
becomes `blocked` by migration** — `blocked` is only ever reached by an actual capability refusal
(D81).

### Step 5 — criteria from the existing JSON arrays

For each non-deleted task, one `task_criteria` row per array element, in position order:

```text
ordinal        1-based position
label          AC-<ordinal>
text           the string, unchanged
state          pending
verification   unverified
local_revision 1
created_at     the task's created_at        (not "now" — the criterion is as old as the task)
```

Empty arrays produce no rows. Duplicate strings produce distinct rows with distinct ids, because they
were distinct entries and merging them would lose one.

`tasks.acceptance_criteria` is **not modified** — it is already exactly the projection
`rebuild_criteria_projection` would compute, which is asserted immediately after migration.

FTS is untouched. `memory_fts` and its three triggers are not recreated and no reindex occurs
(research B7).

## Server migration `0002_project_intelligence.sql`

Additive, run by the server's own migration path.

```sql
ALTER TABLE memories ADD COLUMN IF NOT EXISTS topic_key             TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS value_key             TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS importance            TEXT NOT NULL DEFAULT 'normal';
ALTER TABLE memories ADD COLUMN IF NOT EXISTS verification           TEXT NOT NULL DEFAULT 'unverified';
ALTER TABLE memories ADD COLUMN IF NOT EXISTS verification_authority TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS stale_at               TIMESTAMPTZ;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS last_verified_at      TIMESTAMPTZ;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS verification_basis    JSONB NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS evidence_fact_count   INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS effective_from        TIMESTAMPTZ;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS superseded_at         TIMESTAMPTZ;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS pinned                BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS reinforcement_count   INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS distinct_origin_count INTEGER NOT NULL DEFAULT 1;

CREATE TABLE IF NOT EXISTS memory_relations (…);   -- no basis_evidence_id, no rationale
CREATE TABLE IF NOT EXISTS task_criteria    (…);
CREATE TABLE IF NOT EXISTS task_blockers    (…);
```

No backfill on the server. The daemon re-sends what it holds; the server's rows converge as memories
and relations arrive, and `effective_from`/`superseded_at` land with the next upsert of each memory.
Backfilling server rows from `updated_at` would apply a *second*, less-informed approximation on top
of the local one.

**Not created**, and the absence is the privacy boundary: `evidence_facts`,
`memory_evidence_facts`, `verification_runs`, `continuity_checkpoints`, `reusable_patterns`,
`pattern_applications`, `task_changes`, `criterion_evidence`.

## Mixed-version behaviour

| Combination | Behaviour |
|---|---|
| New daemon, new server | Everything syncs |
| New daemon, **old server** | The server rejects unknown entity types and fields. The daemon classifies the rejection, stops sending that class, keeps delivering memories/tasks/sessions/handoffs, and reports it in `cairn sync status` and `cairn doctor` (FR-415, SC-326) |
| Old daemon, new server | The daemon sends no new fields; new columns keep their defaults. The daemon ignores the new read-back arrays because it does not read them |
| New daemon, **old local store** | Migration 5 runs at open |
| Old daemon, **new local store** | Refused by the existing schema-version guard: *"database schema version 5 is newer than this build supports (4); upgrade Cairn"* (FR-516) |
| Two devices, one on each version | The older sends no relations; the newer's relations are rejected by the old server and reported. Memories still merge, and conflicts are still detected on the newer device |

## Rollback

There is no down-migration, matching Feature 001 and 002. Rolling back means reinstalling the older
binary, which then refuses the store by version guard — the safe failure, not a silent
misinterpretation.

For an operator who needs it, the documented recovery is a file copy of `cairn.sqlite3` taken before
upgrading. `cairn doctor` names the schema version so the situation is diagnosable, and the release
notes state the one-line backup command. That is the same posture the project already takes pre-1.0.

## Proof

`tests/tests/migration_alpha4.rs` builds a store at the **real** alpha.4 schema — by running
migrations 1–4 only, not by hand-writing DDL — populates it, migrates, and asserts:

| # | Assertion |
|---|---|
| 1 | Row counts unchanged for `projects`, `tasks`, `sessions`, `observations`, `memories`, `memory_evidence`, `handoffs`, `outbox`, `sync_meta` and every Feature 002 table |
| 2 | Every pre-existing column value byte-identical, except `effective_from` and `superseded_at` which were NULL and are now set per the documented rules |
| 3 | Every new column at its documented default; `verification = 'unverified'` for all; `topic_key`/`value_key` NULL for all |
| 4 | One `supersedes` relation per pre-existing `superseded_by_id`, and no others |
| 5 | `rebuild_supersession` produces exactly the pre-existing `state`/`superseded_by_id` values |
| 6 | `task_criteria` count equals the total element count of all non-deleted tasks' arrays; labels and ordinals in position order |
| 7 | `rebuild_criteria_projection` equals every `tasks.acceptance_criteria` byte for byte |
| 8 | Pending and in-flight outbox rows are untouched and still deliverable; `pull_cursor` unchanged |
| 9 | `local_only` memories still produce no outbox row |
| 10 | FTS still returns every memory it returned before, with the same ranking |
| 11 | Every Feature 001 and Feature 002 end-to-end suite passes against the migrated store |
| 12 | An interrupted migration (injected failure mid-script) leaves `schema_migrations` at 4 and the store fully usable by the old build |
| 13 | Running the migration twice is a no-op |
| 13a | Every pre-existing outbox row keeps its state; none becomes `blocked`; pending rows remain claimable |
| 13b | `stale` memories have `stale_at IS NULL`, and a historical query reports their applicability as unknown rather than bounded |
| 14 | Every rebuild procedure equals what the migration wrote, except `superseded_at` (named explicitly) |

Assertion 10 is worth its place: FTS is an external-content table with triggers, and the most likely
way a careless additive migration breaks a user's store is by disturbing it. This test proves it was
not touched.

Assertion 12 proves the safe-failure story: a user who loses power mid-upgrade has a working
alpha.4 store, not a half-migrated one.
