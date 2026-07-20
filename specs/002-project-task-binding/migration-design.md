# SQLite Migration Design

**Feature**: 002-project-task-binding
**Migration**: `0002_project_task_binding.sql`
**Predecessor**: exact converged Feature 001 `0001_init.sql`

## Feature 001 baseline

Migration 0001 currently creates:

- `repositories`: UUIDv7 row ID, Git-private `repo_uuid`, mutable path/remote metadata;
- `worktrees`: stable UUID, repository FK, path metadata;
- `snapshots`: immutable worktree snapshots, unique fingerprint per worktree;
- `sessions`: repository/worktree/user/agent scope, resume-token hash, lease clocks,
  `active|recovering|stopped|interrupted`, start/current snapshots, timestamps;
- partial unique index `sessions_one_live_per_instance(repository_id,
  agent_instance_id)` for live states;
- `events`: global AUTOINCREMENT `seq`, UUID event ID, globally unique idempotency
  key, optional repository/worktree/session/snapshot FKs, JSON payload, timestamp;
- append-only event triggers and immutable snapshot triggers;
- `meta` with fingerprint schema version.

The runtime opens SQLite in WAL mode, enables foreign keys, uses
`synchronous=FULL`, sets a five-second busy timeout, performs
`PRAGMA quick_check(1)`, then runs embedded SQLx migrations.

## Upgrade contents

The design SQL is
[contracts/migrations/0002_project_task_binding.sql](contracts/migrations/0002_project_task_binding.sql).

Migration 0002 performs only additive DDL:

1. add `sessions.binding_mode NOT NULL DEFAULT 'local_unbound'`;
2. add nullable event aggregate columns and partial indexes;
3. create `event_aggregate_heads`;
4. create project, repository-association, task, task-revision, and session-binding
   projections;
5. create integrity/immutability triggers;
6. insert `meta.local_schema_version=2`.

The triggers backstop active-project mutation rules, permanent task ownership/title,
sequential revision counters, parent validity, repository/project membership,
task-revision/project membership, the one-time binding transition, and immutable/no-delete
rows. Services still perform typed preflight validation so callers receive stable domain
errors rather than raw SQLite messages.

It does not:

- update or delete an event;
- rebuild the events table;
- update Feature 001 IDs, timestamps, states, snapshot references, leases, or token
  hashes;
- create any project, task, revision, association, binding, or Feature 002 event;
- infer scope from paths or remotes.

## Existing-session classification

SQLite applies the column default to every existing session, so a query sees
`binding_mode='local_unbound'` immediately after migration. No binding row or event is
created. All prior session columns remain identical.

Verification compares every pre-migration session column byte-for-byte and then asserts:

- same row count and IDs;
- same state, snapshot FKs, lease and heartbeat timestamps;
- same `resume_token_hash`;
- only the new discriminator reads `local_unbound`;
- `session_bindings` is empty.

## Legacy events

Aggregate columns are nullable only to preserve rows written before migration. The
migration never backfills them. A new insertion trigger requires all three fields for
every post-migration event, including existing Feature 001 event types.

Compatibility replay derives a legacy row's scope only in memory:

1. real `session_id` → session aggregate;
2. otherwise real `worktree_id` → worktree aggregate;
3. otherwise real `repository_id` → repository aggregate;
4. otherwise retain as legacy global/unknown and fail strict verification if it affects
   a known projection.

Derived scope is never persisted.

## Transaction and restart behavior

The migration remains a normal SQLx transactional migration. There is no
`-- no-transaction` directive. SQLite DDL in this migration is transactional.

On successful commit, SQLx records version/checksum in `_sqlx_migrations`. Reopening
the database verifies the checksum and skips version 2. Raw re-execution is not the
supported idempotency mechanism; safe version gating is.

On failure:

- the migration transaction rolls back;
- SQLx does not record version 2 as applied;
- no partial table/index/trigger is treated as healthy;
- storage returns a typed migration failure separated from corruption;
- daemon startup keeps serving its safe error channel and maps requests to
  `MIGRATION_FAILED` with bounded
  `{kind:"migration_failure",target_version:2}` data;
- raw SQLx errors, database paths, environment values, and user content remain in
  internal diagnostic causes only and are not returned.

On the next start after the underlying failure is corrected, SQLx retries the complete
migration from the Feature 001 schema boundary.

## Mutation transaction primitive after upgrade

Feature 001's `serialized_txn` begins a deferred transaction and uses a process-local
worktree mutex. Feature 002 requires a database-backed correctness boundary.

The storage layer adds an immediate mutation primitive:

```text
acquire connection
→ BEGIN IMMEDIATE
→ idempotency lookup
→ validate projection/status/ownership
→ allocate aggregate sequence with UPSERT ... RETURNING
→ append event(s)
→ update projection(s)
→ COMMIT
```

Rollback occurs on any error. Aggregate mutexes keyed as
`<aggregate_type>:<aggregate_id>` may remain as contention optimization, but tests
bypass/share-nothing those locks and use independent pools to prove the SQLite boundary.

## Real Feature 001 database fixture

Implementation creates:

```text
fixtures/databases/
├── feature-001-v1.sqlite3
└── feature-001-v1.manifest.json
```

The database must be created by the frozen Feature 001 implementation commit
`4a06c4125715bb4b78b54e49c81eccd82100a7b7`, not by Feature 002 migrations. It
contains representative repositories, worktrees, snapshots, ordered events, and sessions
in all lifecycle states. The manifest records:

- producer commit SHA;
- SQLite and SQLx migration versions;
- fixture file SHA-256;
- schema/table/index/trigger names;
- row count by table;
- ordered hash of every event row including payload bytes;
- per-session identity, state, snapshot IDs, timestamps, lease values, and token hash;
- explicit statement that no raw resume token or secret fixture content exists.

Tests copy the database to a temporary path, capture the manifest baseline, open it
through the Feature 002 runtime, and compare before/after.

## Migration test cases

1. Empty database migrates through 0001 then 0002.
2. Real Feature 001 fixture migrates with zero historical data loss.
3. Every existing session becomes local-unbound and no binding/project/task row/event is
   fabricated.
4. Existing events retain identical seq, ID, idempotency key, FKs, payload bytes, and
   timestamp; new aggregate columns remain null.
5. A second normal open applies no migration and changes no row/hash.
6. Injected failure during a transaction leaves the database at the 0001 boundary.
7. Restart after injected failure completes 0002 once.
8. Migration error maps to `MIGRATION_FAILED` without raw path/SQL/internal leakage.
9. Foreign-key and quick-check pass after upgrade.
10. New post-migration events without an explicit aggregate are rejected.
11. Feature 001 DAO queries continue to return the same rows after the additive columns.
12. Windows, macOS, and Linux open/migration tests pass wherever SQLite file/locking
    behavior differs.

## Rollback and compatibility policy

Feature 002 does not provide a destructive down migration. Downgrading a database after
version 2 is unsupported because old binaries do not understand the new schema checksum
or binding semantics. Recovery is restore-from-backup/copy, not table deletion.

Forward compatibility is additive: Feature 001 columns and indexes remain; v1 clients
may continue local-unbound session operations through the upgraded daemon. This policy
is recorded in [compatibility-matrix.md](compatibility-matrix.md).
