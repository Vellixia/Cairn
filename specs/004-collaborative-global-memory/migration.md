# Migration Design: Cairn Collaborative Global Memory

**Feature**: `004-collaborative-global-memory`
**From**: local schema **6**, server schema **1** (this codebase's server migration count; the
capability name advertised is `SCHEMA_3_CAPABILITIES` — see [compatibility.md](./compatibility.md))
**To**: local schema **7**, server schema **2**

## 1. The additive-only policy, inherited from 003, and where 004 must deviate

003's rule, stated verbatim in its own migration design: "Additive only. No existing migration is
edited... No user action is required... Nothing is fabricated to fill a new column" — the literal
reading behind 003's own FR-513 ("existing rows MUST NOT be rewritten"). 004 inherits that rule
unchanged and 004's spec restates it directly: **FR-523** ("Migration of an existing local store MUST
preserve every existing row unchanged and MUST assign every new field a documented default").

004 deviates from "no existing migration is edited" in exactly one place, and it is not a new kind of
deviation: `outbox` must be rebuilt to widen its `entity_type` `CHECK` to accept `personal_knowledge`
and `team_knowledge` (FR-528). SQLite cannot alter a `CHECK` constraint in place. This is not a novel
problem — `0005_project_intelligence.sql:441-492` already rebuilt this exact table for this exact
reason (widening `entity_type` to add `memory_relation`, `task_criterion`, `task_blocker`, plus the
`blocked` state), so the precedent, the recipe and the cost of a rebuild against a store with a large
outbox are already established and already proven safe by `migration_alpha4`'s equivalent assertions.

**The rebuild recipe** (`data-model.md` §2.12 has the full DDL):

1. `CREATE TABLE outbox_new (...)` with the widened `CHECK` (ten `entity_type` values instead of eight)
   and the new `namespace` column, plus a `CHECK` tying `project_id IS NULL` to the two new entity types.
2. `INSERT INTO outbox_new SELECT ..., 'project:' || project_id FROM outbox` — every existing row is
   copied byte-for-byte into the new shape; the only computed value is `namespace`, deterministically
   derived from the row's own `project_id`.
3. `DROP TABLE outbox`.
4. `ALTER TABLE outbox_new RENAME TO outbox`.
5. Recreate `outbox_pending (state, created_at)` and `outbox_claimable`, the latter re-keyed to
   `(namespace, state, created_at)` in place of `(project_id, state, created_at)`.
6. **Release claims** — carried over unchanged from every existing row, not re-run here: `claimed_at`
   is copied verbatim in step 2, so an in-flight claim's staleness is judged exactly as it would have
   been before the rebuild. (The separate `release_all_claims` sweep that runs at daemon start,
   `outbox.rs:170-177`, is unaffected by this migration; it is a `UPDATE`, not a migration step.)

No row is dropped, no row's `payload`, `idempotency_key`, `state`, `attempts` or `last_error` changes.

## 2. Local migration `0007_collaborative_global_memory.sql`

Applied by the existing `migrate::run`/`run_to` (`cairn-store/src/migrate.rs:66-114`), inside its own
transaction, with its `schema_migrations` row written last — unchanged mechanism, so an interruption
rolls the whole migration back exactly as it does today (§9).

Statement order, in one script:

1. **New tables and their indexes** — `personal_knowledge`, `personal_knowledge_applicability`,
   `personal_knowledge_relations`, `personal_fts` + its three triggers, `team_knowledge`,
   `team_knowledge_applicability`, `team_knowledge_relations`, `team_fts` + its three triggers,
   `project_traits`, `writer_identity`, `sync_cursor`. All `CREATE TABLE IF NOT EXISTS`, in the order
   listed, because `personal_fts`'s triggers reference `personal_knowledge` and `team_fts`'s reference
   `team_knowledge` — the tables they index must exist first.
2. **`sync_cursor` backfill from `sync_meta`** — see §3.
3. **`outbox` rebuild** — see §1, run after the new tables exist so `outbox`'s eventual writers have
   somewhere to enqueue against, though nothing in the rebuild itself reads a new table.
4. **`writer_identity` seed** — not expressible in portable SQL (it needs a fresh UUID, and unlike a
   UUIDv7 identifier this value carries no ordering claim to get wrong, but it must be generated exactly
   once). Runs as a Rust step in `migrate.rs`'s `finish(7, tx)` hook, inside this same transaction,
   immediately after the script — the identical pattern migration 5 already uses for converting
   `tasks.acceptance_criteria` JSON arrays into `task_criteria` rows
   (`0005_project_intelligence.sql:426-435`, `migrate.rs`'s `finish` matching on `version`). An
   interruption before this step commits still rolls the whole migration back (§9's proof, assertion 5).

The whole migration — script plus the `writer_identity` finish-hook plus the `schema_migrations`
insert — is one SQLite transaction, exactly like every migration before it.

### Transactional boundary

Both migrations reuse their runner's existing all-or-nothing shape unchanged; 004 adds no new
transaction machinery on either side.

| | Local (`migrate.rs:66-114`) | Server (`db.rs:61-89`) |
|---|---|---|
| Boundary | One `sqlx::SqliteConnection` transaction per migration version | One `pool.begin()` transaction per migration version |
| Script execution | Statement-by-statement (`split_statements`), because SQLite executes one statement per call | Whole script in one `tx.execute(*sql)` call, because PostgreSQL accepts a multi-statement string |
| Non-SQL step | `finish(7, tx)` — the `writer_identity` seed — runs inside the same transaction, after the script | None for 004's server migration; every server-side value (`server_instance.id`, the `role` backfill) is expressible as SQL and runs inside the script |
| Commit order | `schema_migrations` row inserted last, same transaction | `schema_migrations` row inserted last, same transaction |
| Interrupted mid-migration | Whole transaction rolls back; store stays at version 6 | Whole transaction rolls back; server stays at version 1 |

Because both runners already commit the version row in the same transaction as the migration's own
work, "the migration applied" and "the version advanced" are one atomic fact on both sides — there is no
window where `outbox` has been rebuilt but `schema_migrations` still reads 6, or where `server_instance`
has a row but `schema_migrations` still reads 1.

## 3. `sync_meta` → `sync_cursor` backfill

```sql
INSERT INTO sync_cursor (namespace, pull_cursor, last_success_at, server_capability)
SELECT 'project:' || project_id, pull_cursor, last_success_at, server_capability
  FROM sync_meta;
```

Every existing row becomes namespace `project:<project_id>`, and `pull_cursor` is carried over
**verbatim** — it is an opaque RFC-3339 string produced by the server's `page_cursor`
(`cairn-server/src/sync.rs:818-830`), and 004 does not reinterpret it. `last_success_at` and
`server_capability` are carried the same way, since `sync_cursor` gained those columns specifically to
hold what `sync_meta` already held (data-model.md §2.11). `backoff_until` has no source column and
starts `NULL` — no store has ever computed a per-namespace backoff before this migration.

**`sync_meta` is retained, not dropped.** Two reasons: first, `sync_meta.last_success_at` and
`.pull_cursor` are the only durable audit trail of what a project synced *before* this feature, and
dropping the table would be an edit-in-place of pre-existing state with no compensating benefit — the
additive-only policy (§1) argues for leaving working state alone unless something requires touching it.
Second, and more concretely, nothing in the code that reads `sync_meta` is being changed by this
migration itself (that is a Rust-level cutover for the daemon's sync loop, tracked separately from the
schema); keeping the table means a partially-updated build during rollout has somewhere consistent to
read from rather than a table that vanished mid-release. `sync_meta` becomes vestigial once every
project-namespace reader moves to `sync_cursor`, and removing a vestigial table is a strictly later,
lower-risk migration — consistent with 003 never removing `memory_evidence` once
`memory_evidence_facts` existed alongside it (`data-model.md` 003 §3.3, "not modified").

## 4. Server migration `0003_collaborative_global_memory.sql`

Run by the server's own migration path, one transaction per the existing mechanism. Statement order:

1. `users` additive columns (`role`, `status`, `must_change_password`, `password_changed_at`).
2. **`users.role` backfill** — see §5. Must run immediately after the column exists and before anything
   else references `role`, because the "never zero admins" guarantee (FR-413, FR-524) has to hold the
   instant the column becomes meaningful, not just at the end of the migration.
3. `server_instance` table creation, then a single `INSERT` generating its one row — see §6.
4. `project_members` additive column:
   ```sql
   ALTER TABLE project_members ADD COLUMN IF NOT EXISTS added_by_user_id UUID REFERENCES users(id);
   ```
   A nullable column with no `DEFAULT` already backfills every pre-existing row to `NULL`, so no
   separate backfill statement is needed. `NULL` is exact, not a placeholder: who added a pre-existing
   membership was never recorded, and fabricating an answer would violate the additive-only policy's
   rule against inventing unrecorded state (§7) — the same rule that leaves `topic_key` and `stale_at`
   `NULL` rather than guessed. `created_at` (`cairn-server/migrations/0001_init.sql:52`) already answers
   "when" for every row, existing and new, so no second timestamp column is added here.
5. `api_tokens.expires_at` — additive, nullable, no backfill statement (every existing token keeps
   working exactly as an unexpiring token). A token whose `expires_at` has passed is refused with the
   same status and body as a revoked token (FR-585, SC-452, data-model.md §3.4) — no schema change beyond
   the column itself is needed for that, since both conditions are read from the one column by the one
   check.
6. `personal_knowledge`, `personal_knowledge_applicability`, `personal_knowledge_relations`,
   `team_knowledge`, `team_knowledge_applicability`, `team_knowledge_relations` — new tables, empty at
   creation, no backfill possible or needed. Full DDL for all six is in `data-model.md` §3.5–3.10; two of
   their columns are called out here on their own because they are the one place this feature's own
   design changed mid-flight (D448, FR-582). `personal_knowledge` and `team_knowledge` each carry

   ```sql
   writer_id  TEXT   NOT NULL,
   writer_seq BIGINT NOT NULL,
   ```

   under the same uniqueness the local store already enforces:

   ```sql
   CREATE UNIQUE INDEX IF NOT EXISTS personal_knowledge_writer_seq
       ON personal_knowledge (writer_id, writer_seq);
   CREATE UNIQUE INDEX IF NOT EXISTS team_knowledge_writer_seq
       ON team_knowledge (writer_id, writer_seq);
   ```

   Both columns are declared `NOT NULL` on a table that is itself brand new in this same statement group
   — nothing has ever been inserted into `personal_knowledge` or `team_knowledge` before step 6 runs, so
   there is no pre-existing row for a `NOT NULL` addition to violate and no default or backfill to specify
   (contrast step 4's `added_by_user_id`, added to a table — `project_members` — that already holds rows,
   which is why that column is nullable instead). `NOT NULL` is correct here precisely because a record
   synced from any writer, including a peer's, must always carry its own writer identity and its position
   in that writer's stream: the column exists so a *different* store can detect a gap in it (FR-582), and
   a nullable column would silently defeat the one thing it exists to provide.

## 5. `users.role` backfill — the algorithm

Stated as an algorithm, then as the SQL that implements it, matching FR-414/FR-524 exactly:

```text
1. If CAIRN_ADMIN_EMAIL is set and a user row has that email, that user becomes admin.
2. Otherwise, the single oldest account by created_at becomes admin.
3. Every other existing account becomes member.
4. If the users table is empty, there is nothing to backfill, and the environment-seeded
   admin account (auth.rs:131-176, main.rs:170-194) will be created — already admin, by
   the same rule applied to the moment it is upserted — on the server's next start.
```

```sql
-- Step 1/2: pick exactly one admin, deterministically.
--
-- The CTE is repeated per statement because a CTE's scope is the single
-- statement that declares it — the earlier version of this section referenced
-- `admin_candidate` from a second `UPDATE` that had no `WITH`, which is not
-- valid PostgreSQL and fails at migration time.
--
-- `priority` is explicit because `UNION ALL` does not promise row order. The
-- earlier version relied on the configured-email arm "coming first", which is
-- unspecified: PostgreSQL is free to return either arm first, so the rule was
-- non-deterministic in exactly the case it exists to decide.
WITH admin_candidate AS (
    SELECT id, 0 AS priority, created_at FROM users
     WHERE email = current_setting('cairn.admin_email', true)
    UNION ALL
    SELECT id, 1 AS priority, created_at FROM users
)
UPDATE users SET role = 'admin'
 WHERE id = (SELECT id FROM admin_candidate ORDER BY priority, created_at ASC LIMIT 1);

-- Step 3: everyone else, explicitly, so the DEFAULT 'member' from the ALTER TABLE
-- is restated here as an assertion rather than relied on silently.
WITH admin_candidate AS (
    SELECT id, 0 AS priority, created_at FROM users
     WHERE email = current_setting('cairn.admin_email', true)
    UNION ALL
    SELECT id, 1 AS priority, created_at FROM users
)
UPDATE users SET role = 'member'
 WHERE id <> (SELECT id FROM admin_candidate ORDER BY priority, created_at ASC LIMIT 1);
```

**`cairn.admin_email` has to be set for any of this to matter.** Nothing set it until Phase 2
wired it: `db::connect` now takes the operator's `--admin-email` and issues
`set_config('cairn.admin_email', $1, true)` inside each migration's transaction — local to the
transaction, so it is visible to the script and gone afterwards rather than leaking an
operator's email into a pooled connection's later sessions. Without that call the first arm
matched nothing on every deployment and the backfill silently fell through to the
oldest-account rule, which is a legitimate outcome that also produces exactly one admin — so
the failure was invisible from its result. `tests/tests/migration_alpha5.rs` seeds a
configuration where the two arms disagree, which is the only way to tell which one ran.

`current_setting('cairn.admin_email', true)` reads the same configured identity `auth::ensure_admin`
already keys off of (`auth.rs:131-176`, upserting by `CAIRN_ADMIN_EMAIL`), passed into the migration run
so the backfill and the runtime seed never disagree about which email is authoritative. The `UNION ALL`
with its explicit `priority` and `ORDER BY` is what makes the rule deterministic and total: if the configured email matches no
row, the first arm returns nothing and the second arm's oldest-account row is the only candidate: **the
server never ends this migration with zero admins** (FR-413, FR-524), because exactly one `UPDATE` sets
exactly one row to `admin` whenever `users` has at least one row, and step 4 covers the only remaining
case (no rows at all — nothing to leave non-admin).

## 5a. Never-zero-admins is enforced atomically at runtime (D436)

§5's backfill only guarantees the invariant holds the instant this migration finishes. The same invariant
must also hold across the lifetime of a running server, against a demote (admin → member) or a disable
(active → disabled) issued through the runtime admin API — and it must hold even when two such requests
race each other, targeting the two remaining admins at the same instant (FR-413, FR-560).

**A read-count-then-update implementation is insufficient and MUST NOT be used.** The naive shape —
`SELECT count(*) FROM users WHERE role = 'admin' AND status = 'active'`, then, if the count is greater
than one, a separate `UPDATE users SET role = 'member' WHERE id = $1` — has a race window between the two
statements: two concurrent requests can each run the `SELECT` and each observe two active admins, and
both then proceed to their own `UPDATE`, leaving zero. Counting and acting are two statements, and
nothing stops two connections from interleaving between them.

The guarantee is instead enforced by **serializing every such operation on one application-wide advisory
lock**, taken before the guard is evaluated and held for the rest of the transaction, and then applying a
guarded `UPDATE` (FR-574). Concurrent admin mutations do not race because they are not concurrent:

```sql
-- Every transaction that could reduce the active-admin count takes this lock first.
-- One fixed key, transaction-scoped, released on commit or rollback.
SELECT pg_advisory_xact_lock(4770040001);

-- Demote: admin -> member. Refused if this account is the last active admin.
WITH active_admins AS (
    SELECT id FROM users
     WHERE role = 'admin' AND status = 'active'
)
UPDATE users
   SET role = 'member'
 WHERE id = $1
   AND role = 'admin'
   AND EXISTS (SELECT 1 FROM active_admins WHERE id <> $1);
-- 0 rows affected => refused (this account is the last active admin).

-- Disable: active -> disabled. Same lock, same guard, only when the target is an
-- admin; disabling a non-admin account is never gated by this check.
SELECT pg_advisory_xact_lock(4770040001);

WITH active_admins AS (
    SELECT id FROM users
     WHERE role = 'admin' AND status = 'active'
)
UPDATE users
   SET status = 'disabled'
 WHERE id = $1
   AND status = 'active'
   AND (role != 'admin'
        OR EXISTS (SELECT 1 FROM active_admins WHERE id <> $1));
-- 0 rows affected => either already disabled, or refused as the last active admin.
```

Why this is atomic where count-then-update is not: the advisory lock admits exactly one such transaction
at a time. The second demote blocks on `pg_advisory_xact_lock` before it evaluates anything, and resumes
only after the first has committed and released it. It then reads `active_admins` fresh, finds only itself,
and its own `EXISTS ... WHERE id <> $1` predicate is false: `0` rows affected, refused. Exactly one of the two
concurrent operations succeeds.

An earlier draft of this document instead used a `FOR UPDATE`-locked CTE and argued that the blocked
transaction would re-evaluate the CTE against the committed state. That is real `EvalPlanQual` behaviour
under `READ COMMITTED`, and it very probably works — but making a safety invariant depend on EPQ
re-evaluation through a CTE and an aggregate is fragile, version-sensitive and hard to prove in a test that
could not also pass for the wrong reason. The advisory lock needs no isolation-level reasoning at all, and
admin demotions are rare enough that serializing them costs nothing. SC-444 asserts the outcome against a
real database under genuine concurrency rather than by argument.

This is a runtime application-level guard, not a migration statement — no schema in this feature changes
to support it (the `role`/`status` columns already exist, §4 step 1). It is documented here because it is
the mechanism that makes the invariant §5's backfill establishes durable afterward, and because it directly
answers what spec.md's replaced SC-433 (the repair path for externally corrupted state, not a legal
runtime operation) deliberately does NOT cover: a runtime demote or disable is a legal API operation with
its own atomicity guarantee here, whereas SC-433 is about recovering from state corrupted outside the API
entirely.

## 6. `server_instance` — single-row generation and why it must be immutable

```sql
INSERT INTO server_instance (id, created_at) VALUES (gen_random_uuid(), now());
```

One `INSERT`, unconditional, because this migration only ever runs once against a schema-1 server (the
forward-only guard in §9 prevents a second run). Immutability is not merely a convention — it is what
`server_instance_id` is *for*: a local store records which instance's team knowledge it holds
(FR-495) and refuses a second instance's team knowledge (FR-496). If the id could be regenerated, a
server redeploy that regenerated it would make every already-linked store's recorded instance id stale,
and the mismatch check (compatibility.md §Server-instance mismatch) would then refuse a server's *own*
subsequent team knowledge — the opposite of the guarantee it exists to provide. There is no `UPDATE`
statement anywhere in this feature that targets `server_instance.id`.

## 7. What is deliberately NOT backfilled, and why

003's precedent for this section is `topic_key` and `stale_at`, both left `NULL` rather than inferred
from prose or from an unreliable timestamp source
(`0005_project_intelligence.sql`'s own comment block, "Deliberately NOT backfilled, and each absence is
a decision"). 004 follows the same discipline:

| Column | Left at | Why |
|---|---|---|
| `personal_knowledge`/`team_knowledge` (every row) | *no rows exist yet* | Both tables are new; there is no pre-existing personal or team knowledge to backfill by construction. |
| `project_traits` (every row) | *no rows exist yet* | Populated by the daemon's own derivation at the next link/refresh (D413), never synthesized by a migration guessing at a working tree it cannot see from inside a database transaction. |
| `project_members.added_by_user_id` (pre-existing rows) | `NULL` | Fabricating a "who added them" for a membership that predates this feature would be inventing provenance no record ever captured — exactly the FR-437/D413 discipline of "derived, never guessed," applied here to history rather than to a manifest file. |
| `writer_identity` | generated once, not backfilled from anything | There is nothing to backfill it *from*: no prior concept of a per-store writer identity existed. |
| `sync_cursor.backoff_until` | `NULL` | No store has ever computed a per-namespace backoff value before this migration exists to consume one. |

## 7a. Local and server migrations are independent of each other's timing

Nothing requires `0007` and `0003` to be applied in either order relative to one another, and a fleet
rolling out this feature will not apply them simultaneously in practice. A `cairnd` running migration
`0007` against a server still on schema 1 behaves exactly as [compatibility.md](./compatibility.md)
describes for "new client, old server": the two new entity types are held `blocked` in their own
namespaces (D427) while the project namespace keeps synchronizing. A server that has run `0003` against
a fleet of daemons still on schema 6 simply serves a capability set those daemons never query for — the
one-way advertisement (D428) means nothing is sent to a daemon that never asks. Neither migration reads
or depends on any state the other one produces.

**No schema changes for the re-probe cycle (D437).** A namespace held `blocked` because the server it
talks to has not run `0003` does not stay blocked forever waiting for a user action: the client re-probes
the server's advertised capabilities — the exact same `SCHEMA_3_CAPABILITIES` name this migration causes
the server to start advertising the moment `0003` commits — on a bounded, backed-off schedule (FR-561).
Nothing about this migration changes to support that: the capability advertisement this migration's
completion enables IS what the client-side probe reads, so a fleet member that upgrades its server after
its daemons already run `0007` needs no new local write, no user command and no daemon restart for those
daemons' held personal/team entries to release for delivery (FR-562, FR-563) — the transition from
blocked back to eligible is entirely a client-side read of a capability set this migration already causes
the server to serve.

## 8. The proof test — matching 003's standard exactly

`tests/tests/migration_alpha4.rs` is 003's precedent; 004's equivalent (`migration_alpha5.rs`, naming the
schema version it starts from) follows the same four obligations:

1. **Build the real prior schema, not a hand-written approximation.** Run `migrate::run_to(pool, 6)` —
   real migrations 1 through 6 — then populate it with representative rows: linked and unlinked
   projects, an outbox with `pending`/`in_flight`/`delivered`/`failed`/`blocked` rows, a populated
   `sync_meta` with a mid-stream `pull_cursor`, and every Feature 002/003 table non-empty.
2. **Migrate, then assert row and byte equality** for every table this migration does not touch:
   `projects`, `tasks`, `sessions`, `observations`, `memories` and every Feature 003 table, unchanged
   column-for-column, row-for-row. For `outbox` specifically: same row count, same `id`, `payload`,
   `idempotency_key`, `state`, `attempts`, `last_error`, `created_at`, `delivered_at`, `claimed_at`,
   `blocked_reason`, `blocked_at_capability` — only `namespace` is new, and it is asserted to equal
   `'project:' || project_id` for every row.
3. **Assert the `sync_cursor` backfill**: one row per pre-existing `sync_meta` row, `namespace` equal to
   `'project:' || project_id`, `pull_cursor` byte-identical to the source row's `pull_cursor`.
4. **Assert that an interrupted migration leaves the store safely on the old version** — the same
   injected-failure-mid-script technique 003's assertion 12 uses: force a failure partway through
   `0007`'s statements (for example after the `outbox` rebuild's `DROP TABLE outbox` but before
   `ALTER TABLE outbox_new RENAME TO outbox` commits) and assert `schema_migrations` still reads `6`,
   the transaction rolled back so `outbox` (the original table) still exists and is fully queryable, and
   the store opens and operates normally under the old build. This is FR-525's literal requirement and
   directly satisfies FR-530's proof obligation for the outbox rebuild specifically.

A fifth assertion, new to 004 and without a direct 003 analog: **running the server's `users.role`
backfill against a seeded corpus of account configurations** (no `CAIRN_ADMIN_EMAIL` set with N
accounts; `CAIRN_ADMIN_EMAIL` set matching an existing account; `CAIRN_ADMIN_EMAIL` set matching no
account; exactly one account) and asserting exactly one `admin` row and zero servers with no admin in
every case (SC-405).

## 9. Rollback, and the local guard

There is no down-migration, matching every migration before it. Rolling back means reinstalling the
older binary, which then refuses the store by the existing version guard,
`crates/cairn-store/src/migrate.rs:85-90`:

```rust
if current > supported {
    return Err(MigrateError::TooNew { found: current, supported });
}
```

producing `"database schema version {found} is newer than this build supports ({supported}); upgrade
Cairn"`. An older `cairnd` built before 004 can never write against a schema-7 local store; it fails
loudly and immediately at open, rather than silently misreading (or worse, successfully writing garbage
into) tables it does not know about. This is the same safe-failure posture 003 established and 004
changes nothing about it — no new guard is added because the existing one already generalizes to any
future schema version by construction.
