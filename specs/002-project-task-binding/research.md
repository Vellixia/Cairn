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

- **Decision**: Store a unique UUID idempotency key on each task revision. A retry with
  the same key and same task/canonical request returns the existing revision; reuse for a
  different task or canonical body returns `TASK_REVISION_CONFLICT`. Revision 1 has no
  parent. Later revisions default to the immediately previous revision; an explicitly
  selected parent must belong to the same task and have a lower revision number.
- **Rationale**: Persisted keys survive lost responses and daemon restarts while the
  parent rule preserves a clear default history without prohibiting deliberate
  same-task branching.
- **Alternatives considered**: Content fingerprint as idempotency key (identical content
  may be an intentional new revision); event ID only (caller cannot safely retry);
  process memory cache (not restart-safe).

## R7. Goal-contract canonicalization and fingerprint

- **Decision**: Define `GoalContractV1` as a typed, fixed-field-order structure:
  `schema_version=1`, `goal`, `included_scope`, `excluded_scope`,
  `acceptance_criteria`, `constraints`. Normalize CRLF and CR to LF, then trim
  surrounding Unicode whitespace on each scalar/list item. Preserve all internal
  whitespace and exact list order. Empty lists are valid; goal and supplied list entries
  must be non-empty after normalization. Serialize compact UTF-8 JSON from the typed
  struct and store lowercase BLAKE3 hex of those exact bytes.
- **Rationale**: Fixed typed fields avoid map-order ambiguity; including the schema
  version in hashed bytes makes future formats explicit.
- **Alternatives considered**: JSON object maps (key ordering risk); sorting lists
  (changes user meaning); aggressive whitespace collapsing (changes content); hash of
  user input bytes (line-ending/platform instability).

## R8. Migration strategy

- **Decision**: Add one SQLx migration `0002_project_task_binding.sql`. It adds
  `sessions.binding_mode NOT NULL DEFAULT 'local_unbound'`, creates new projection
  and aggregate-head tables, and adds nullable aggregate columns/indexes to `events`.
  It never updates an existing event or creates project/task/binding rows. SQLx's
  `_sqlx_migrations` version/checksum gate makes subsequent opens no-ops; SQLite
  transactional DDL rolls the migration back on failure.
- **Rationale**: A default column explicitly classifies every existing session without
  fabricating history. Nullable aggregate columns preserve legacy rows exactly.
- **Alternatives considered**: Rebuild/backfill the events table (rewrites history);
  create placeholder projects/tasks (invalid lifecycle semantics); parallel Feature 002
  database (breaks atomicity and daemon ownership).

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

- **Decision**: Keep globally unique `events.idempotency_key`. Derive event keys from
  event type plus operation UUID or immutable entity tuple. For task creation, derive
  separate `task.created:<operation>` and
  `task.revision_created:<operation>` keys and append both in one transaction.
  Check the first key before allocating sequences; a retry returns the committed
  projection without incrementing aggregate heads.
- **Rationale**: Existing append semantics remain compatible and multi-event retries
  cannot produce partial or duplicate histories.
- **Alternatives considered**: Reuse one key for multiple events (violates unique
  constraint); random keys generated inside every retry (duplicates events); separate
  idempotency table for every command (unnecessary).

## R12. Replay compatibility

- **Decision**: Replay new projections in ascending `events.seq`. New event payloads
  contain enough complete state to reconstruct projects, associations, tasks, revisions,
  and bindings. Legacy Feature 001 rows with null aggregate columns derive scope in
  memory from their existing most-specific real foreign key (session, then worktree,
  then repository); the database is not backfilled. Strict verification reports unknown
  Feature 002 event versions, while normal listing preserves unknown rows.
- **Rationale**: This keeps one ordered ledger and preserves old event bytes while
  allowing exact projection equality.
- **Alternatives considered**: Ignore global order and replay each aggregate separately
  (loses cross-aggregate causality); rewrite legacy rows (forbidden); silently accept
  unknown event payload versions in verification (false confidence).

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

- **Decision**: Extend `SessionService::start` with a tagged requested scope. Omitted
  scope from old v1 clients decodes as `local_unbound`; new clients send it
  explicitly. New bound creation validates and commits session, `session.started`,
  `session.bound`, and binding projection together, then follows the unchanged
  watcher-ready/reconcile boundary. A live collision returns existing only for identical
  stored/requested scope; otherwise `SESSION_SCOPE_CONFLICT`.
- **Rationale**: One path preserves uniqueness, stale takeover, leases, recovery,
  watcher readiness, snapshots, and token handling.
- **Alternatives considered**: Start unbound then call bind in a second transaction
  (exposes an unintended unbound window); duplicate bound-start service (behavior drift);
  silently convert a colliding session (violates explicit binding).

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

- **Decision**: Add `v1.project.*`, `v1.task.*`, and `v1.session.bind` methods;
  extend `v1.session.start/get/list` DTOs additively with tagged scope. Keep
  JSON-lines framing, `cairn.cli.v1`, checked-in schemars output, golden examples, and
  closed errors. Existing clients that omit start scope remain local-unbound.
- **Rationale**: The changes are additive and preserve the established local transport.
- **Alternatives considered**: Introduce v2 immediately (unnecessary without removals or
  reinterpretation); free-form JSON payloads (lose schema tripwires); direct CLI storage
  writes (split invariants).

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

- **Decision**: Build/fetch first, then run the success demonstration and focused
  migration/replay tests inside Linux OS-level network isolation while proving external
  network denial and local filesystem/IPC success. Run platform-specific IPC,
  migration, restart, and persistence suites on Windows, macOS, and Linux. Do not demand
  duplicate platform evidence for pure canonicalization code with no platform branch.
- **Rationale**: This proves offline behavior rather than merely passing
  `cargo --offline`, while keeping cross-platform scope tied to actual differences.
- **Alternatives considered**: Configured-but-unrun matrix (not evidence); dependency
  offline flag alone (network still available); require every unit test on every OS
  regardless of platform behavior (cost without additional assurance).
