# T126 — Feature 002 scope and frozen-tree audit

Independent audit of the **frozen implementation tree**
(`8a3353eee55fc6805bea70e1b7e6f823dd7ab022`, commit
`95dc67e9dd3e39be3b4a82bcc015ac32875a75da`), not merely the evidence commit. Every
claim below is backed by the exact command shown.

## Ranges inspected

| Range | Meaning |
|---|---|
| `be1a660` (origin/main) → `95dc67e` (frozen) | main is an ancestor of the freeze; everything ahead of main is on this branch |
| `fb204b0` (the `002` spec commit) → `95dc67e` | the Feature-002-specific implementation range |
| `95dc67e` (frozen) → `e02defe` (evidence) | the post-freeze evidence delta |

The Feature-002-specific range is `fb204b0..95dc67e`; commits before `fb204b0` are
Feature 001 and earlier and are out of Feature 002's scope charter.

## Evidence commit is evidence/accounting only

`git diff --name-only 95dc67e e02defe` → 10 files, **all** under
`specs/002-project-task-binding/evidence/` or `…/tasks.md`. Zero runtime, test,
migration, fixture, schema, CI, or script paths.

## Frozen-tree crate/app inventory

`git ls-tree 95dc67e crates/ apps/`:

```
apps/cli  apps/daemon
crates/cairn-domain  crates/cairn-events  crates/cairn-git  crates/cairn-project
crates/cairn-protocol  crates/cairn-session  crates/cairn-storage-local
```

All in-scope. `git ls-tree -r -d 95dc67e` finds **no** web-UI, dashboard, frontend,
PostgreSQL, MCP, synchronization, orchestration, or embedding directory. (The only
`web/ui/sync` path matches are `.specify/extensions/verify-tasks/tests/fixtures/…`,
Spec Kit tooling fixtures, not Cairn implementation.)

## Feature 002 changed exactly the in-scope crates

`git diff --name-only fb204b0 95dc67e` → top dirs: `apps/cli`, `apps/daemon`,
`crates/cairn-domain`, `crates/cairn-events`, `crates/cairn-project`,
`crates/cairn-protocol`, `crates/cairn-session`, `crates/cairn-storage-local`.
`cairn-git` was **not** modified by Feature 002.

## Out-of-scope charter — each item confirmed absent

Source scan (`git grep -licE … 95dc67e -- 'apps/**/*.rs' 'crates/**/*.rs'`) for the
out-of-scope list returned only two benign hits, both non-implementations:

| Term | Hit | Verdict |
|---|---|---|
| synchroniz | `feature002_quickstart.rs:13` — the test goal-contract string `"no synchronization"` | test data declaring it out of scope; not an implementation |
| permission | `apps/daemon/src/ipc.rs` — Unix socket file-mode `0o700`/`0o600` (analysis I1 socket security) | IPC socket hardening, **not** an accounts/roles/permissions system |

No hits for: PostgreSQL, user accounts, memberships, roles/permissions system,
cross-device continuity, MCP, context compiler, AI memory, truth claims, embeddings,
drift detection, product completion-verification, server synchronization. No
repository transfer/removal, task transfer/deletion, session unbind/rebind, or
task-revision transition for existing bindings was found. No Feature 003 directory or
path exists.

## CLI is IPC-only (no storage access)

- `git show 95dc67e:apps/cli/Cargo.toml` dependencies: `cairn-protocol`,
  `cairn-daemon`, `cairn-session`. **No `sqlx`, no `cairn-storage-local`.**
- `apps/cli/tests/feature002_ipc_only.rs` — `cli_dependency_and_source_tripwire_forbids_storage_access`
  asserts the CLI manifest contains none of `sqlx`/`cairn-storage-local`/`cairn_storage_local`
  and CLI source imports none of `sqlx::`/`cairn_storage_local`.
- `git grep 'open_pool|SqlitePool|sqlite' 95dc67e -- 'apps/cli/src/**/*.rs'` → **no
  matches.** The CLI writes no SQLite directly.

## Migration 0001 was not rewritten

- `git log fb204b0..95dc67e -- …/migrations/0001_init.sql` → **NONE**. No Feature 002
  commit modified it.
- `git diff --quiet fb204b0 95dc67e -- …/0001_init.sql` → **identical.** Feature 002
  preserved Feature 001's `0001_init.sql` byte-for-byte.
- The last commit to touch `0001_init.sql` is `cd3b168` "feat(session): complete
  Feature 001 implementation" (2026-07-19), a Feature 001 commit.
- Feature 002 **added** `0002_project_task_binding.sql` (the only migration change in
  `fb204b0..95dc67e`).

## Legacy event rows are not rewritten

`0002_project_task_binding.sql` touches `events` only via
`ALTER TABLE events ADD COLUMN aggregate_id TEXT` and
`ADD COLUMN aggregate_seq INTEGER` — additive nullable columns. No
`DROP`/`DELETE`/`UPDATE`/row-rewriting statement against `events`. The migration
acceptance test (SC-003) independently proves the ordered legacy event-row hashes
match the manifest and nothing was fabricated.

## No forbidden artifacts in the frozen tree

`git ls-tree -r 95dc67e` finds **no** `.env`, secret, `-wal`, `-shm`, `.DS_Store`,
`node_modules`, `.log`, or `target/` path. The only `.sqlite3` is the approved
canonical fixture `fixtures/databases/feature-001-v1.sqlite3`.

## Verdict

**Scope audit clean.** The frozen Feature 002 implementation stays within its
charter, preserves Feature 001 (0001 unchanged, legacy events additive-only, CLI
IPC-only), introduces no out-of-scope subsystem, and Feature 003 is untouched.
