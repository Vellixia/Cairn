# Task 3 phase A — edge inventory

Scope inspected: `crates/cairn-store`, `crates/cairnd`; head `9658c55` plus
this phase's Task-cleanup repair. This is an inventory only. No broad edge
deletion started because current daemon handlers, recovery, and migration paths
still compile against the local-authority modules below.

## Retain boundary

| Required edge concern | Current owner/evidence |
| --- | --- |
| typed durable capture/command spools, receipts, retry state | `cairn-store/src/spool.rs`: `spool_event`, `claim_events`, `mark_event_*`, `spool_command`, `claim_commands`, `mark_command_*`, `release_*_claims`; schema `0008_safe_events.sql` |
| server/account/project binding | `cairnd/src/state.rs`: `ServerCredentials::{load,mutate_credentials,account_identity}`; `cairn-store/src/repo.rs`: project link fields; `cairnd/src/sync.rs`: `set_token`, `link` |
| bounded returned-context cache | `cairnd/src/deliver.rs`: `OutageCache::{put,get,clear}`; it is in-memory, account-bound, LRU-capped |
| hook/session correlation | `cairnd/src/arrival.rs`: `Arrivals`, `Ticket`; `cairn-store/src/spool.rs`: `session_event_seq`, `command_seq`; `cairnd/src/capture.rs` |
| integration ownership | `cairn-store/src/integrations.rs`: `upsert_agent`, `bind`, `unbind`, `bound_resources`; schema `0004_integrations.sql` |
| offline Task migration manifest | `cairn-store/src/transfer.rs`: export/cleanup/import; `removed_feature_manifest`; schema `0013`/`0014` |

## Delete candidates — local canonical knowledge, search, ranking, replicas

| Module | Exact surviving implementation |
| --- | --- |
| `cairn-store/src/repo.rs` | local project/session/observation/memory/handoff CRUD: `create_memory`, `create_memory_reconciled`, `memory`, `delete_memory`, `import_memory`, `list_sessions`, `latest_handoff`, `start_session`, `end_session`; local sync/deferred state: `last_sync_success`, `record_sync_success`, `pull_cursor`, `set_pull_cursor`, `defer_pulled_record`, `deferred_records`, `clear_deferred_record`, `note_deferred_attempt` |
| `cairn-store/src/search.rs` | local FTS/ranking/retrieval: `search`, `one`, `build_result`, `recency_contribution`, `memory_for_scope`, `search_personal`, `search_team`, `list_team`, `fts_query` |
| `cairn-store/src/knowledge.rs` | local relation, conflict, supersession, reinforcement construction; called by briefing/sync/handlers |
| `cairn-store/src/evidence.rs` | local evidence collection, verification-run write/read, local evidence views |
| `cairn-store/src/patterns.rs` | local reusable pattern and application store/recall |
| `cairn-store/src/global.rs` | personal/team replicas: `merge_synced_personal`, `merge_synced_team`, `merge_synced_pattern`, `recall_personal`, `recall_team`, writer-gap/adoption functions |
| `cairnd/src/briefing.rs` | local durable assembly: `build`, `global_candidates`, `level1_patterns`, `scope_memory`, `latest_handoff_for_branch`, `level0_warnings`, `level0_pins` |
| `cairnd/src/handlers.rs` | local memory/search/personal/team/pattern CRUD and views; main calls at lines 3085, 3183–3573, 3806–3821 |
| `cairnd/src/patterns.rs`, `promote.rs`, `verify.rs`, `drift.rs`, `continuity.rs`, `handoffs.rs`, `recover.rs` | local canonical promotion, verification, drift, checkpoint/handoff generation and recovery; all require replacement with server operations before deletion |

Schema candidates: `memories`, `memory_evidence`, `memory_evidence_facts`,
`memory_relations`, `evidence_facts`, `verification_runs`, `reusable_patterns`,
`pattern_applications`, FTS tables/triggers (`0002`, `0007`), personal/team
knowledge and applicability/relation tables (`0007`), observations, handoffs,
continuity checkpoints, cached patterns (`0009`).

## Delete candidates — replicas, cursors, namespace/authority, entity outbox

| Module | Exact surviving implementation |
| --- | --- |
| `cairn-store/src/outbox.rs` | entity outbox and retry workers: `enqueue`, `enqueue_global`, `claim`, `claim_excluding`, `claim_namespace*`, `mark_delivered`, `mark_failed`, `mark_retryable`, `mark_blocked`, `release_all_claims`, `rename_namespace`, namespace counters/payload builders |
| `cairn-store/src/cursor.rs` | namespace establishment, pull cursor, visibility, retry/capability state |
| `cairn-store/src/authority.rs` | local authority-mode state and transitions |
| `cairn-store/src/migrate.rs` | legacy authority/cutover runtime: `Phase`, `phase_*`, retained-local and pattern-claim APIs (keep only schema migration runner) |
| `cairnd/src/sync.rs` | namespace worker `run_worker`; entity drains `drain`, `drain_global`; establishment/linking `link`; pull/apply paths around `pull`, `apply_*`, deferred replay; account/member/admin proxy functions `auth_*`, `change_password`, `admin_user_*`, `project_member_*`; legacy/backfill/cutover helpers around `legacy_row_eligibility`, `pattern_eligibility` |
| `cairnd/src/migrate005.rs` | complete legacy migration orchestration: `inspect`, `claim_patterns`, `normalize_keys`, `drain`, `verify_possession`, `switch_authority`, `demote`, `run`, `status`, `retry_retained` |

Schema candidates: `outbox`, `sync_meta`, `sync_deferred`, `sync_cursor`,
`authority_mode`, `migration_state`, `retained_local`, `legacy_pattern_claims`,
writer identity/version columns, `cached_patterns`, and local global-memory
tables. Typed `event_spool`/`command_spool` must not be merged into `outbox`.

## Compiler-safe first seam

Do not drop schema/modules next. First route all durable context/search and
knowledge mutations in `cairnd/src/handlers.rs` and `cairnd/src/briefing.rs`
to one server client boundary, retaining only `deliver::OutageCache` fallback.
Then remove their local callers and tests, which isolates `search`, `repo`,
`knowledge`, `global`, `patterns`, and FTS as a compiler-visible deletion
slice. Separately replace `sync::run_worker` with one typed-spool delivery loop
before deleting `outbox`, `cursor`, `authority`, and `migrate005`; capture and
command spool semantics remain distinct.

## Task 3 phase B — typed worker seam

- `cairnd::sync::run_worker` now releases stale event/command claims at start,
  then drains only `event_spool` followed by `command_spool`.
- Worker retry is bounded exponential backoff (500ms–30s). Typed rows retain
  their own claim, capacity, rejection, ordering, and retry semantics.
- Removed worker namespace targets, pull scheduling, capability probes, global
  outbox discovery, and entity-outbox drain scheduling. Legacy `drain`,
  `drain_global`, pull, and migration paths remain callable until their owning
  handlers are deleted.
- Retrieval now runs one deadline-bounded typed-spool drain before server
  retrieval; it no longer invokes legacy `push_pending`.
- Next dead callers: manual `sync_now` and legacy link/backfill/pull paths still
  own entity-outbox, cursor, namespace, and authority behavior.

Validation: `cargo check -p cairnd --all-targets` passed with Rust 1.97.1;
`cargo clippy -p cairnd --all-targets -- -D warnings` passed with Rust 1.97.1;
focused typed-worker/backoff tests passed under same toolchain.

## Task 3 phase C — removed wire callers

- Deleted removed request test module and Feature 005 runtime tests from
  `cairnd/src/handlers.rs`; deleted sync tests tied to removed entity lanes.
- Deleted `migrate005` and local reusable-pattern runtime modules, their main
  module declarations, wire variants, and daemon dispatch arms.
- `cargo check -p cairnd --all-targets` now compiles with Rust 1.97.1, but
  reports 128 dead-code warnings. Remaining work must trim local handlers,
  sync entity/pull/authority code, integration mutation helpers, and promotion
  runtime before Clippy `-D warnings` can pass.

## Task 3 phase C — spool-only sync

- Replaced `cairnd/src/sync.rs` with typed event/command spool delivery and
  minimal authenticated HTTP client. Removed entity outbox, pull, namespace,
  account/admin/member, link, backfill, authority, and manual-sync runtime.
- Removed orphan `promote` module declaration. Legacy status probes and local
  post-ratification mutation no longer keep deleted sync paths alive.
- Validation: `RUSTC=/Users/andresholivin/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc rustup run 1.97.1 cargo check -p cairnd --all-targets` passes. Clippy cannot start because this environment invokes Homebrew Rust 1.95.0 despite the 1.97.1 toolchain; remaining daemon dead-code warnings also require the next handler/module deletion slice.

## Task 3 phase C — edge dead-code completion

- Deleted unreachable daemon status, local evidence listing, subject, team,
  personal, privacy, sync-status, and legacy integration mutation handlers.
  Kept hook evidence recording, lifecycle capture, typed spools, server client,
  bounded retrieval cache, binding/correlation, integration ownership, and
  removed-feature migration.
- Deleted obsolete credential mutation/cache-clear path and only its now-orphan
  fixtures. Removed pattern promotion/outcome MCP actions left after their wire
  variants were deleted; five MCP tools remain.
- Net deletion: 1,723 lines, 9 added lines across the six affected sources.

Validation with Rust 1.97.1: `cargo clippy -p cairnd --all-targets -- -D warnings`,
`cargo check --workspace --all-targets`, `cargo test --workspace --no-run`,
`cargo test -p cairnd` (84 passed), `cargo test -p cairn mcp::tests` (9 passed),
and `git diff --check` all pass. Workspace check retains two pre-existing
`cairn-server/src/api.rs` dead-code warnings outside this edge slice.

## Task 3 phase D — MCP server-only memory seam

- MCP memory search now calls project, personal, and team server HTTP reads;
  unavailable server paths return `server_unavailable` rather than SQLite FTS.
- MCP create, supersede, pin, reinforce, reconcile, forget, verification, and
  personal mutations now enter the typed immutable command spool unconditionally.
  The prior authority-mode local-write fallback is no longer reachable.
- Graph, replay, and governance already use server HTTP boundaries and remain so.

Validation with Rust 1.97.1: `cargo check -p cairnd --all-targets`,
`cargo clippy -p cairnd --all-targets -- -D warnings`, `cargo test -p cairnd`
(84 passed), `cargo test -p cairn mcp::tests` (9 passed), and `git diff --check`.

### Phase D cleanup

Deleted all compile-disabled legacy local evidence, verification, reconciliation,
personal-memory, local search, authority fallback, and on-demand-session code.
No store module had an empty production caller graph: briefing, drift,
continuity, verification, migration, or store tests still own each candidate.

Validation with Rust 1.97.1: `cargo clippy -p cairnd -p cairn-store --all-targets -- -D warnings`,
`cargo check --workspace --all-targets` (two pre-existing server warnings),
`cargo test --workspace --no-run`, `cargo test -p cairnd` (84 passed),
`cargo test -p cairn mcp::tests` (9 passed), and `git diff --check`.

## Task 3 phase E — server-only context and capture cleanup

- `deliver` now returns the server response verbatim, or only its bounded,
  account-bound outage-cache entry. A cache miss returns an explicit unavailable
  result; it never builds a local briefing.
- Removed daemon local briefing, continuity, drift, recovery, and verification
  runtimes. Local observation capture is no longer written by lifecycle tool
  events; automatic capture stays on the typed `event_spool` path.
- Hook vocabulary now derives only from queued safe events. It no longer reads
  local memories or established personal/team knowledge.
- Explicit checkpoint recovery is queued as one typed operation; it no longer
  creates a local handoff/checkpoint.

Validation: Rust 1.97.1 `cargo check -p cairnd --all-targets`,
`cargo test -p cairnd --no-run`, and `git diff --check` pass. Clippy could not
start: this environment launches Homebrew Rust 1.95.0 even under `rustup run
1.97.1`; workspace requires Rust 1.97.1.

Deferred schema/module deletion: `cairn-store` local `repo`, `search`,
`knowledge`, `global`, `evidence`, `patterns`, `continuity`, `outbox`,
`authority`, and `cursor` remain because session/integration/removed-feature
migration callers still compile against them. No schema migration was added.

### Phase E review fix

- Typed event batches now register their referenced sessions atomically before
  ingest. Registration is idempotent, derives ownership from the bearer token,
  and requires existing project membership; fresh edge sessions can therefore
  deliver their first event without a local-authority server write path.
- Event and command 5xx responses are transient; 4xx responses are terminal
  except declared capability deferral. Cached retrieval entries expire after
  five minutes, carry cache age/account identity, and are removed immediately
  on a live refusal before a later outage can reuse them.

Validation: Rust 1.97.1 `cargo check -p cairnd -p cairn-server --all-targets`
and `git diff --check` pass.

## Task 3 phase F — session/handoff edge deletion

- Session start/end retain SQLite correlation only, then enqueue immutable
  boundary-class safe events. They no longer write entity-outbox session
  snapshots or use daemon-side `SyncPolicy`.
- Handoff generate/annotate are typed recovery commands; latest reads only the
  server handoff endpoint. Deleted local handoff synthesis/storage handler and
  its daemon module.
- Removed compaction continuity lookup, local trait/owner helpers revealed by
  that deletion, and obsolete daemon test support.

Validation with Rust 1.97.1: `cargo check -p cairnd --all-targets`, `cargo
test -p cairnd` (27 passed), `cargo test -p cairn mcp::tests` (9 passed), and
`git diff --check` pass. `cargo clippy -p cairnd -p cairn-store --all-targets
-- -D warnings` cannot start: Cargo selects Homebrew Rust 1.95.0 even with the
1.97.1 `RUSTC` binary; workspace requires Rust 1.97.1.

### Phase F correction — command-spool vocabulary

- Added forward SQLite migration `0015_handoff_commands`: the new typed
  handoff recovery command kinds now pass the database `CHECK` while preserving
  every existing command-spool row and index.
- Added focused proofs for stable handoff command identities and server-only
  latest-handoff failure (`server_unavailable` rather than SQLite fallback).

## Task 3 phase G — server handoff recovery boundary

- `POST /api/commands` now handles only the two Phase F command-spool kinds
  that need server work: `handoff_generate` and `handoff_annotate`.
  Session lifecycle remains safe-event ingestion; no session command protocol
  was added.
- The daemon now transmits a command spool row's `session_id`. Server handlers
  bind that session to the authenticated account and named project, reserve the
  stable `command_id` in the existing receipt table, and return the original
  handoff receipt after a lost acknowledgement.
- Generation persists server-derived boundary state; annotation is bounded and
  privacy-screened before updating only `agent_note` on the latest canonical
  handoff. Focused coverage includes acceptance, duplicate receipt,
  unauthorized project, and malformed payload/trigger rejection.

Validation: Rust 1.97.1 `cargo check -p cairn-server --all-targets` and
`cargo check --workspace --all-targets` pass (the workspace retains two
pre-existing `cairn-server/src/api.rs` dead-code warnings). Focused command
test compiles and runs, but its PostgreSQL assertions are skipped here because
`CAIRN_TEST_DATABASE_URL` is unset. Clippy cannot start: Cargo invokes the
Homebrew Rust 1.95.0 Clippy driver while this workspace requires Rust 1.97.1.

## Task 3 phase G — local-store authority deletion

- Deleted local authority, continuity, evidence, global, knowledge, entity
  outbox, patterns, search, and traits modules. `repo` now keeps only project
  binding and session correlation; `cursor` keeps only server-instance binding.
- Removed legacy migration orchestration and local-canonical test suites.
  Typed spools, integration ownership, transfer artifacts, diagnostics, and
  transactions remain. Server comments now describe server-owned contracts.
- No schema migration was added. Legacy SQLite tables remain only in immutable
  migration history and export inventory until fresh-DB setup cutover.

Validation: Rust 1.97.1 `cargo check --workspace --all-targets`, `cargo test
--workspace --no-run`, `cargo test -p cairn-store spool` (3 passed), `cargo
test -p cairn-store --test v1_migration` (4 passed), and `git diff --check`
pass. `cargo clippy -p cairn-store -p cairnd -p cairn-server --all-targets -- -D warnings`
is blocked because installed Clippy launches Rust 1.95.0 while
the workspace requires Rust 1.97.1.

## Task 3 phase H — entity-route deletion

- Deleted live server entity sync/pull, pattern pull, migration/cutover routes
  and their handlers/contracts; safe-event ingestion and typed commands remain.
- Removed web Sync-status API/page and its generated contract entry. Route
  inventory now derives no deleted endpoint from server source.
- Fresh edge migration rebuilds session correlation without legacy `tasks` FK;
  daemon session and outage/cache tests run against a task-free fresh database.

Validation with Rust 1.97.1: `cargo check --workspace --all-targets`,
`cargo test --workspace --no-run`, `cargo test -p cairn-store spool` (3
passed), `cargo test -p cairnd` (28 passed), and `git diff --check` pass.
Clippy remains unavailable: installed driver uses Rust 1.95.0, while workspace
requires 1.97.1.

## Task 3 phase H — binding follow-up

- Fresh capture, lifecycle, and command rows no longer read legacy
  `sync_cursor` namespace state. They are unbound when written; delivery calls
  typed endpoints directly without `/api/version` preflight. Fresh databases
  also remove `sync_meta` and `sync_cursor`.
- Fresh inventory coverage asserts both legacy sync tables are absent.
