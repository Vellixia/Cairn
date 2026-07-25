# Research: Project and Task Binding Foundation

**Feature**: 002-project-task-binding
**Date**: 2026-07-20
**Status**: Complete — no unresolved `NEEDS CLARIFICATION` items

## R1. Module and crate ownership

- **Decision**: Add one focused `cairn-project` crate for project and task mutation
  policy. Put pure identities, statuses, binding-mode types, and goal-contract
  canonicalization in `cairn-domain`; SQL in `cairn-storage-local`; event payloads
  and replay in `cairn-events`; session binding/start policy in the existing
  `cairn-session`; transport and rendering remain in daemon/CLI adapters.
- **Rationale**: Project/task policy is large enough to deserve a focused boundary but
  does not justify a framework or service process. Keeping binding in `cairn-session`
  prevents a circular dependency and preserves one session lifecycle authority.
- **Alternatives considered**: Put policy in daemon handlers (rejected: transport would
  own invariants); expand `cairn-session` to own all project/task commands (rejected:
  mixed-purpose crate); introduce repository/service framework traits everywhere
  (rejected: unnecessary abstraction for one SQLite backend).

## R2. Identity and timestamps

- **Decision**: Add UUIDv7 newtypes for `ProjectId`,
  `ProjectRepositoryAssociationId`, `TaskId`, and `TaskRevisionId`. Continue
  RFC 3339 UTC timestamp strings and existing Feature 001 repository/worktree/session
  IDs. Use UUID-valued idempotency keys; CLI generates UUIDv7 when the caller does not
  supply one, while machine IPC requires the key for create/update/revise operations.
- **Rationale**: This matches Feature 001 identity ordering and avoids path/name
  identity. UUID idempotency keys are bounded, non-secret, and schema-validatable.
- **Alternatives considered**: Integer public IDs (leak ordering and hinder future
  synchronization); names as identifiers (duplicates are valid); caller-supplied free
  text keys (unbounded and easier to misuse as secret material).

## R3. Project and repository association representation

- **Decision**: Persist projects separately from repository associations. An association
  has its own UUIDv7 ID and a unique `repository_id`; it is always active in Feature
  002 and has no removal/transfer state. Worktree membership is derived by joining
  `worktrees.repository_id` to the association. Project archive does not deactivate or
  detach the association.
- **Rationale**: Repository identity is already stable and path-independent. One unique
  repository row enforces exclusivity and automatically covers every current/future
  worktree without fabricated association rows.
- **Alternatives considered**: Store project ID on each worktree (duplicates state and
  breaks inheritance); use path/remote URL keys (mutable metadata); nullable
  `projects.repository_id` (cannot represent multiple repositories).

## R4. Project status and duplicate names

- **Decision**: `ProjectStatus` is a closed `active|archived` enum. Updates are
  append-only events plus mutable project projection rows. Archived projects are
  readable but reject association, task/revision, bind, and bound-start mutations;
  explicit update to `active` restores mutation ability. Names are trimmed,
  non-empty display metadata and are not unique.
- **Rationale**: This directly represents the clarified lifecycle without deletion or
  hidden state changes.
- **Alternatives considered**: Soft-delete status (out of scope); unique normalized
  names (contradicts duplicate-name requirement); archive cascading to sessions or
  associations (would rewrite established scope).

## R5. Transactional task-revision allocation

- **Decision**: Use a SQLite `BEGIN IMMEDIATE` transaction, not only a Tokio mutex.
  After idempotency lookup, atomically increment
  `tasks.latest_revision_number = latest_revision_number + 1 RETURNING ...`, validate
  the parent, insert the revision under
  `UNIQUE(task_id, revision_number)`, append the event, and commit. The unique index
  and SQLite write lock are correctness backstops across separate connections/processes;
  an aggregate mutex only reduces contention.
- **Rationale**: Concurrent requests cannot both observe and publish the same revision
  number, even if process-local coordination is bypassed or the daemon restarts.
- **Alternatives considered**: `SELECT MAX()+1` under a deferred transaction (race);
  process-local lock only (fails across processes/restarts); global revision numbers
  (violates task-scoped numbering).

## R6. Revision idempotency and parents

- **Decision**: Persist one global immutable operation record for every keyed mutation. Appending operations locate their immutable result event so exact retries reproduce the first post-state and original `created/updated` flag even after live projections change. A distinct-key identical association/binding no-op locates that immutable projection and records `created:false`. Same key/method/fingerprint returns that exact result; another method/request returns `IDEMPOTENCY_CONFLICT`. Revision parents remain same-task/earlier and sequential.
- **Rationale**: Immutable locators make “original result” literal across restarts and later mutable updates while one database authority closes cross-method reuse.
- **Alternatives considered**: Locate mutable project/Task rows (returns later state); store response blobs (duplicative); per-revision keys (cross-method gap); process memory (not restart-safe).

## R7. Goal-contract canonicalization and fingerprint

- **Decision**: Define `GoalContractV1` as a typed, fixed-field-order structure: `schema_version=1`, `goal`, `included_scope`, `excluded_scope`, `acceptance_criteria`, and `constraints`. Normalize CRLF and CR to LF, then trim surrounding Unicode whitespace on each scalar/list item. Preserve all internal whitespace and exact list order. Empty lists are valid; goal and supplied list entries must be non-empty after normalization. Serialize compact UTF-8 JSON from the typed struct and store lowercase BLAKE3 hex of those exact bytes. Validation uses the closed bounded violation union `missing_required_field | malformed_structure | empty_goal | empty_list_entry | unsupported_version`, with only field/list-index/version metadata and never contract content.
- **Rationale**: Fixed typed fields avoid map-order ambiguity; including the schema version in hashed bytes makes future formats explicit; bounded violations support safe machine handling.
- **Alternatives considered**: JSON object maps (key ordering risk); sorting lists (changes user meaning); aggressive whitespace collapsing (changes content); hash of user input bytes (line-ending/platform instability); free-form validation messages (privacy and compatibility risk).

## R8. Migration strategy

- **Decision**: Add one SQLx migration `0002_project_task_binding.sql`. It adds `sessions.binding_mode NOT NULL DEFAULT 'local_unbound'`, projection/aggregate-head tables, the immutable global `operation_idempotency` registry, and nullable aggregate columns/indexes on `events`. It never updates an existing event or fabricates project/task/binding/operation rows. SQLx version/checksum gating makes subsequent opens no-ops; checksum mismatch or a database version newer than the application fails closed as `MIGRATION_FAILED`; transactional DDL rolls back on failure.
- **Rationale**: A default column explicitly classifies every existing session without fabricating history. Nullable aggregate columns preserve legacy rows, and the global registry provides one cross-method retry authority.
- **Alternatives considered**: Rebuild/backfill events (rewrites history); placeholder projects/tasks (invalid lifecycle semantics); per-method idempotency tables (cross-method gap); parallel Feature 002 database (breaks atomicity).

## R9. Real Feature 001 migration fixture

- **Decision**: Commit a real SQLite database produced by the frozen converged Feature
  001 implementation SHA `4a06c4125715bb4b78b54e49c81eccd82100a7b7`, plus a JSON
  manifest recording schema version, SHA-256 file hash, table counts, ordered event
  hashes, representative active/recovering/stopped/interrupted sessions, leases,
  snapshots, and token hashes (never raw tokens). Tests copy the fixture before opening.
- **Rationale**: Replaying DDL into an empty database does not prove upgrade compatibility
  with real persisted rows and SQLx metadata.
- **Alternatives considered**: Synthetic schema-only fixture (insufficient evidence);
  mutate the committed fixture in place (non-repeatable); include raw resume tokens
  (privacy violation).

## R10. Explicit aggregate event scope

- **Decision**: Extend `events` with nullable `aggregate_type`,
  `aggregate_id`, and positive `aggregate_seq`. All events appended after migration
  set them. Supported scopes are repository, worktree, session, project, and task.
  `events.seq` remains global order. `event_aggregate_heads` atomically increments
  per scope/ID; a partial unique index enforces aggregate sequence uniqueness.
- **Rationale**: Project/task events need real scope without fake worktrees, while the
  global existing sequence preserves Feature 001 replay and pagination.
- **Alternatives considered**: Separate event table (splits history); force global
  worktree key (fabricated scope); use global sequence alone (no explicit
  per-aggregate ordering/serialization evidence).

## R11. Event idempotency and multi-event operations

- **Decision**: The global `operation_idempotency` registry owns the raw caller key. Event idempotency keys are deterministically derived from that operation key plus event position/type; task creation therefore appends distinct `task.created` and `task.revision_created` keys. Under `BEGIN IMMEDIATE`, lookup or reserve the registry record before sequence allocation, then append every event and update every projection in the same transaction. The first committed concurrent caller wins; waiters reread the committed registry record and either return its result or raise `IDEMPOTENCY_CONFLICT`.
- **Rationale**: One authoritative registry closes cross-method conflicts while preserving the ledger's unique event keys and atomic multi-event retries.
- **Alternatives considered**: Reuse one raw key for multiple events (violates event uniqueness); event-only lookup (cannot distinguish another method/request); random retry keys (duplicates); process-local locks (do not serialize independent connections).

## R12. Replay compatibility

- **Decision**: Establish a typed replay dispatcher before US1. Add the project handler with US1, the task/revision handler with US2, the session-binding handler with US3, and bound-start mixed-event handling with US4. Replay remains ascending `events.seq`. `task.revision_created` carries the complete immutable revision and complete resulting Task post-state, including `latest_revision_number` and `updated_at`. `session.started` always initializes `local_unbound`; `session.bound` is the sole event establishing `project_bound`. Legacy Feature 001 rows with null aggregate columns derive scope in memory from their existing most-specific real foreign key; the database is not backfilled. The later replay phase owns mixed-ledger integration, corruption handling, unknown-version checks, and exact field-for-field equality.
- **Rationale**: Story work cannot create unreplayable events, one ordered ledger preserves cross-aggregate causality, and complete post-state permits exact projection equality without rewriting legacy bytes.
- **Alternatives considered**: Defer all handlers until late integration (temporary unreplayable features); replay aggregates independently (loses causality); rewrite legacy rows (forbidden); infer bound scope from `session.started` (ambiguous).

## R13. Session binding policy

- **Decision**: Store binding mode on the session row and immutable details in a
  one-row-per-session `session_bindings` projection. Binding validates project status,
  repository association, worktree ownership, and task-revision project ownership in one
  immediate transaction, appends `session.bound`, inserts the projection, and changes
  only `binding_mode`. Identical triples are success; any different triple is rejected.
- **Rationale**: The mode is explicit for every session, binding facts remain
  replayable, and Feature 001 lifecycle state is untouched.
- **Alternatives considered**: Put nullable project/revision columns directly on
  `sessions` without a binding event projection (poor replay/provenance); create a new
  session on bind (loses identity); mutate the original `session.started` event
  (forbidden).

## R14. Bound session start

- **Decision**: Extend `SessionService::start` with a tagged requested scope. A new `local_unbound` start is allowed only when the repository has no active project association or its associated active project has no selectable active task revision; otherwise return `PROJECT_SCOPE_REQUIRED` without creating a session. Historical migrated unbound sessions remain valid. A bound start validates scope and appends `session.started` followed by `session.bound`, plus both projections, in one transaction; neither event is externally visible before commit. The unchanged watcher-ready/reconcile boundary follows. A live collision returns existing only for identical stored/requested scope; otherwise `SESSION_SCOPE_CONFLICT`.
- **Rationale**: One path preserves Feature 001 uniqueness, leases, recovery, watcher readiness, snapshots, and token handling while honoring the constitutional bootstrap restriction.
- **Alternatives considered**: Always allow new unbound sessions (violates project/task invariant); start unbound then bind in a later transaction (externally visible invalid window); duplicate bound-start service (behavior drift); encode binding in `session.started` (breaks sole binding-event semantics).

## R15. Error-name alignment

- **Decision**: Keep the spec's canonical wire codes
  `REPOSITORY_PROJECT_CONFLICT` and `SESSION_BINDING_CONFLICT`. The requested
  implementation concepts `RepositoryAlreadyAssociated` and
  `SessionAlreadyBound` are typed Rust domain variants mapped to those wire codes.
  Do not add duplicate wire spellings for the same invariant. Add typed bounded
  `ErrorData` discriminants so clients receive the existing/requested IDs without raw
  internal details.
- **Rationale**: One invariant must have one stable machine code; the spec is
  authoritative, while domain names can express the cause naturally.
- **Alternatives considered**: Emit both uppercase spellings nondeterministically
  (unstable); replace spec codes only in the plan (cross-artifact conflict); return
  untyped messages (not machine-safe).

## R16. CLI name resolution and bounded ambiguity

- **Decision**: IPC and JSON machine mode use IDs only. Human CLI may accept
  `--name`/`--title`; it resolves by calling bounded list IPC, using exact
  case-sensitive match after surrounding-whitespace normalization. Zero matches maps to
  not-found, one proceeds by ID, and multiple return `AMBIGUOUS_NAME` with at most 20
  candidate IDs plus `truncated`. Human list/show output always prints IDs beside
  names/titles.
- **Rationale**: The daemon contract remains ID-authoritative and duplicate names never
  resolve silently.
- **Alternatives considered**: Daemon name selectors (machine interface would cease to
  be ID-only); first-match selection (unsafe); unbounded candidate arrays (resource and
  privacy risk).

## R17. Contract evolution

- **Decision**: Add `v1.project.*`, `v1.task.*`, and `v1.session.bind` methods; extend `v1.session.start/get/list` DTOs additively with tagged scope. Keep JSON-lines framing, `cairn.cli.v1`, checked-in schemars output, golden examples, and closed errors. Existing clients that omit start scope receive `local_unbound` only when bootstrap eligibility holds; otherwise they receive `PROJECT_SCOPE_REQUIRED`.
- **Rationale**: The transport remains additive without silently bypassing the constitutional scope gate.
- **Alternatives considered**: Introduce v2 immediately (unnecessary without removals); preserve unconditional omitted-scope behavior (invalid once selectable scope exists); free-form JSON payloads (lose schema tripwires); direct CLI storage writes (split invariants).

## R18. Privacy and observability

- **Decision**: Persist canonical goal contracts only in revision projections and typed
  events needed for replay. Structured logs may include entity IDs, schema version,
  list counts, fingerprint, operation outcome, and stable violation codes—never goal
  text/list values, complete contracts, ignored-file contents, environment values, raw
  tokens, or raw migration errors/paths. Error data exposes bounded enums and IDs only.
- **Rationale**: Goal contracts are legitimate local project content, but diagnostic
  duplication creates unnecessary leakage.
- **Alternatives considered**: Log canonical JSON at debug level (still leakage);
  redact only known secret patterns (goal text can itself be sensitive); omit goal
  content from events (would prevent replay).

## R19. Offline and platform evidence

- **Decision**: Build/fetch first, then run the success demonstration and focused migration/replay tests inside Linux OS-level network isolation while proving external network denial and local filesystem/IPC success. Run platform-specific IPC, migration, restart, and persistence suites on Windows, macOS, and Linux. Because Feature 002 modifies session start, event writes, recovery, and storage, the frozen implementation SHA must also execute exactly 100 forced process kills with zero committed-event/state loss using `CAIRN_CRASH_ITERS=100 CAIRN_CRASH_EXPECTED_ITERS=100 cargo test -p cairn-daemon --test us4_crash_restart -- --nocapture`, and explicitly execute `cargo test -p cairn-daemon --test perf -- --ignored`. Record exact commands, SHA, OS/architecture/toolchain, counters or measurements/fixture size/limits, and pass/fail. Configured workflows are not evidence.
- **Rationale**: This proves offline and inherited Feature 001 durability/performance behavior on the actual candidate rather than merely configuring jobs or using `cargo --offline`.
- **Alternatives considered**: Configured-but-unrun matrix (not evidence); dependency offline flag alone (network still available); reuse evidence from another SHA (not compatible); rely on workspace tests where perf is ignored (does not execute SC-007).


## R20. SQLite contention, cancellation, and rollback

- **Decision**: Configure every Feature 002 write connection with a 5,000 ms SQLite busy timeout and perform zero application-level transaction retries, so maximum lock wait is 5,000 ms. Exhaustion returns bounded `STORAGE_BUSY`. Cancellation drops and rolls back the transaction before the connection returns to the pool. Tests may inject a shorter deterministic timeout through a test-only configuration that is unavailable in production.
- **Rationale**: One bounded database wait prevents hidden retry multiplication, starvation, and ambiguous cancellation while retaining SQLite's cross-connection serialization.
- **Alternatives considered**: Unbounded busy wait (hang risk); layered retries (unbounded elapsed time); process-local mutex (does not coordinate processes); commit after cancellation (partial operation).

## R21. Evidence and declaration ordering

- **Decision**: Freeze one implementation commit, execute every required platform and inherited Feature 001 acceptance job at that exact SHA, commit preliminary evidence, then run analyze, verify, verify-tasks, converge, and any appended remediation before producing the final-gate report and a separate final evidence/declaration commit. A committed document never claims its own commit SHA; record that SHA in workflow metadata, an annotated tag, or a later external manifest.
- **Rationale**: Preliminary executions and final convergence are different evidence states, and Git objects cannot truthfully contain their own eventual identity.
- **Alternatives considered**: Treat configured jobs as evidence; declare before convergence; require a document to contain its own SHA; keep 130 as an artificial denominator.
