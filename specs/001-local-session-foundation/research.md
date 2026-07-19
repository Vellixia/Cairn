# Research: Local Session Foundation

**Feature**: 001-local-session-foundation | **Date**: 2026-07-16

No `NEEDS CLARIFICATION` items remained in the Technical Context (stack fully specified
by user input + constitution). Research below records decisions, rationale, and rejected
alternatives for each consequential choice.

## R1. Git integration: Git CLI subprocess

- **Decision**: Shell out to the `git` binary. Primary inspection call:
  `git status --porcelain=v2 --branch --untracked-files=all -z`,
  plus `git rev-parse --show-toplevel --git-common-dir --absolute-git-dir`,
  `git worktree list --porcelain`, `git ls-files -s` (index state), and
  `git remote` for default-remote detection. Git is authoritative for tracked,
  staged, unstaged, deleted, renamed, and untracked state. **Ignored enumeration is
  deliberately excluded from the default call**: `IgnoredSummary` comes from the
  ignore-crate walker (R10), which is authoritative for ignored roots, counts, rule
  provenance, samples, and paginated drill-down. `git status --ignored=matching` may be
  invoked only in an explicit diagnostic/verification mode (e.g., a future
  `--verify-ignored` flag), never on the default inspection path — protecting the 2 s
  inspection target (SC-007) on repos with 100k+ ignored paths.
- **Rationale**: Constitution prescribes "Git CLI preferred initially". Porcelain v2 is a
  stable, versioned, machine-readable format that already distinguishes staged vs
  unstaged vs untracked vs ignored per entry, handles rename/copy detection, and reports
  detached HEAD and upstream in the `--branch` headers. `-z` (NUL termination) makes
  paths with spaces/newlines safe. Matches observed-reality principle: same evidence a
  developer sees.
- **Alternatives considered**: `gix` (gitoxide) — faster, in-process, but younger status
  implementation and a large dependency surface; `libgit2`/`git2-rs` — C dependency,
  historically lags Git behaviors (sparse index, new ignore semantics). Both violate
  "measured need first". Revisit only if subprocess overhead breaks SC-007.

## R2. Fingerprint scheme (BLAKE3, deterministic, content-sensitive)

- **Decision**: Three component fingerprints + one final:
  - `staged_fp` = BLAKE3 over canonical lines `mode SP stage SP oid SP path LF` from
    `git ls-files -s`, sorted bytewise by path. Captures full index state; any staging
    change alters it.
  - `unstaged_fp` = BLAKE3 over canonical entries `path LF status LF blake3(content) LF`
    for each path Git reports as modified/deleted in the working tree relative to the
    index (deleted files hash to a fixed sentinel), sorted bytewise.
  - `untracked_fp` = same entry shape over untracked, non-ignored files (Git ignore
    rules ∪ `.cairnignore`), sorted bytewise.
  - `snapshot_fp` = BLAKE3 over `schema-version LF branch-or-DETACHED LF head-oid LF
    staged_fp LF unstaged_fp LF untracked_fp LF`.
- **Rationale**: Sorted canonical serialization ⇒ determinism (FR-009, SC-002).
  Content hashes (not mtime/size) ⇒ sensitivity exactly to relevant changes (FR-010) and
  immunity to `touch`. Hashing reads contents transiently but persists only 32-byte
  digests ⇒ privacy (FR-027). A schema-version prefix lets the algorithm evolve without
  silently colliding with old fingerprints.
- **Alternatives considered**: `git stash create`/`git write-tree` to make Git compute a
  tree OID for the dirty state — elegant but mutates object DB (writes objects), fails
  on unmerged entries, and can't incorporate `.cairnignore`. mtime+size fingerprints —
  fast but false-positive on touch, false-negative on content-preserving-size edits.
  SHA-256 — fine but BLAKE3 is ~10× faster and specified by user input.

## R3. Snapshot consistency under concurrent change (FR-012)

- **Decision**: Optimistic read-verify-retry. Capture `(HEAD oid, index checksum via
  ls-files hash, status output hash)` before and after the hashing pass; if any differ,
  discard and retry with exponential backoff, max 3 attempts; on exhaustion return
  explicit `SNAPSHOT_CONTENTION` error (never a torn snapshot).
- **Rationale**: Cheap in the common case; guarantees an internally consistent snapshot
  or an honest failure. No global lock on the developer's repo.
- **Alternatives considered**: Holding `index.lock` — intrusive, breaks concurrent Git
  usage; copying the worktree — violates metadata-only rule and SC-007.

## R4. Repository/worktree identity markers

- **Decision**: `repository-id` file at `<git-common-dir>/cairn/repository-id`;
  `worktree-id` at `<absolute-git-dir>/cairn/worktree-id` for each linked worktree
  (for the main worktree the two directories coincide — main worktree gets a worktree-id
  under the common dir too). Files contain a single UUIDv7. Registration is
  read-or-create; the local DB maps IDs → current canonical path (mutable). Duplicate
  detection: on daemon contact from path P with repository-id already registered at live
  path Q ≠ P (both paths exist and both resolve to that id), the newer instance is
  assigned a fresh id (marker rewritten) and `copied_from` recorded.
- **Marker loss (analysis U1)**: when initialization finds markers missing, search the
  local DB by normalized canonical path plus Git common-directory metadata. Exactly one
  compatible repository/worktree match ⇒ restore the marker with the existing identity
  and append `identity.marker_restored`. No unique match ⇒ never silently attach
  historical data: create a new identity only after an explicit human-output warning and
  an explicit `identity_outcome` status in machine output. Identity is never inferred
  from remote URL alone.
- **Bare repositories (analysis U2)**: `cairn init` in a bare repository is rejected
  with stable error `NOT_A_WORKTREE` (this feature requires a working tree); no
  markers, rows, or events are created.
- **Rationale**: Clarification Q1 (2026-07-16). `.git` contents never travel with
  clones/pushes ⇒ fresh clone naturally gets fresh identity. Survives directory
  moves/renames. Nothing enters the tracked tree.
- **Alternatives considered**: Canonical path as identity — breaks on move; remote URL +
  root commit — fails for remoteless repos (FR-005) and merges distinct clones.

## R5. IPC transport + framing

- **Decision**: Newline-delimited JSON (one JSON object per line) over a Unix domain
  socket (`$XDG_RUNTIME_DIR/cairn/daemon.sock`, fallback `~/.local/share/cairn/`
  with 0700 dir) on Linux/macOS and named pipe `\\.\pipe\cairn-<username>-daemon` on
  Windows. JSON-RPC-2.0-shaped envelopes (`id`, `method`, `params` / `result` /
  `error{code,message,data}`), methods namespaced `v1.*`. Tokio `UnixListener` /
  `tokio::net::windows::named_pipe` directly.
- **Rationale**: Human-debuggable, schema-validatable with schemars (same schemas power
  contract tests), zero codegen, trivially versioned.
- **Channel security (analysis I1)**: UDS sockets live inside a user-owned directory
  with 0700 permissions. Windows named pipes are created with an explicit security
  descriptor whose DACL grants access only to the current user SID plus required system
  principals — default named-pipe permissions are never relied upon. Resume tokens
  transit only this authenticated local IPC channel; raw tokens are never written to
  disk or logs. CI includes a Windows test proving a different user identity cannot
  connect.
- **Alternatives considered**: gRPC/tonic — codegen + HTTP/2 machinery for a local
  socket, violates minimal-infrastructure; tarpc — Rust-only, opaque wire format hurts
  future non-Rust clients (TypeScript contract tests are a constitution requirement
  later); raw bincode — not inspectable, no schema story.

## R6. Local persistence layout

- **Decision**: Single per-user SQLite database at the platform data dir
  (`%LOCALAPPDATA%\cairn\cairn.db`, `~/.local/share/cairn/cairn.db`,
  `~/Library/Application Support/cairn/cairn.db`) in WAL mode with
  `synchronous=FULL` for event commits, `foreign_keys=ON`, `busy_timeout` set.
  SQLx with embedded migrations (`sqlx::migrate!`). Events table protected by
  `BEFORE UPDATE`/`BEFORE DELETE` triggers that `RAISE(ABORT)`.
- **Rationale**: One DB serves all repositories ⇒ one migration lineage, cross-repo
  daemon status, single connection pool. WAL gives crash durability (SC-005) and
  concurrent readers. Triggers enforce append-only at the storage layer, not just by
  convention (constitution III).
- **Alternatives considered**: DB-per-repository — N migration lineages, complicates
  daemon status and session listing; storing state inside `.git/cairn/` — Git prune/gc
  tooling and repo copies would corrupt or duplicate history.

## R7. Event idempotency + transactional projections

- **Decision**: Every append carries a caller-derived `idempotency_key`
  (deterministic function of event type + entity id + causal input, e.g.
  `session.started:<session_id>`) with `UNIQUE(idempotency_key)`. Event insert and
  projection updates (sessions row, repositories row, `current_snapshot_id`) run inside
  one SQLite transaction (arch rule 5). **Single-writer serialization (analysis I4)**:
  all event appends and projection changes for one worktree flow through one
  daemon-owned worktree event processor (an actor/task per worktree); sequence
  assignment happens inside that same serialized transaction, so projection updates can
  never interleave out of order. A duplicate idempotency key returns the previously
  accepted event's result and does **not** re-run the projection function. Monotonic
  `seq INTEGER PRIMARY KEY AUTOINCREMENT` gives total order for replay; projections are
  rebuildable by replaying events in `seq` order.
- **Rationale**: Idempotence (arch rule 6) makes retries after crash/IPC-timeout safe;
  single transaction + single writer guarantee the projection never diverges from, or
  reorders against, committed history.
- **Alternatives considered**: Separate event store + async projector — adds a queue and
  eventual consistency for zero benefit at local scale.

## R8. Session liveness, lease, and recovery mechanics

- **Decision**:
  - `agent_instance_id`: UUID supplied via `CAIRN_AGENT_INSTANCE` env var (set by
    launcher/adapter/hook) or `--agent-instance` flag; daemon records optional
    `agent_pid` as supporting liveness metadata only — never primary identity.
  - Lease: on start/recovery the daemon generates a 256-bit random resume token,
    returns it once; DB stores only its BLAKE3 hash as `resume_token_hash`. Each session
    carries `lease_expires_at`. Heartbeat = any authenticated request or explicit
    `v1.session.heartbeat`, refreshing `last_heartbeat_at` and extending
    `lease_expires_at` by the heartbeat TTL (default 90 s). Callers that cannot
    heartbeat immediately (e.g., plain CLI usage before adapters exist) receive a
    longer, configurable initial lease (default 15 min).
  - Staleness (analysis A1): a session is stale when its lease has expired and no valid
    reattachment has occurred. A present-and-verifiable dead `agent_pid` MAY confirm
    staleness earlier (`process_dead`); a missing or unverifiable PID is
    `process_unknown` and never implies death. Liveness determinations record reason
    codes: `heartbeat_expired`, `process_dead`, `reattach_timeout`, `process_unknown`.
  - Restart + grace anchor (analysis A2): daemon marks all `active` sessions
    `recovering` at boot, persisting `recovering_since` **only if not already set**;
    the grace deadline is `recovering_since + recovery_grace_period` (default 15 min).
    Subsequent restarts preserve the timestamp — the deadline is never extended. A
    sweeper marks recovering sessions `interrupted` once the deadline passes.
  - Reattachment (analysis I3): `v1.session.reattach` with matching instance id + token
    resumes the session (clears `recovering_since`, emits `session.recovered`, captures
    a fresh snapshot). A wrong or missing token rejects only that request with
    `LEASE_MISMATCH` and appends a `session.reattach_rejected` audit event (no token
    values); the recovering session is never mutated by failed attempts. An
    authenticated owner may explicitly stop a recovering session (recovering→stopped).
- **Rationale**: Token hash storage keeps secrets out of the DB (Principle X). PID is
  advisory only — cross-checked, never trusted alone; lease expiry is the single
  authoritative staleness clock. Persisted `recovering_since` makes the grace deadline
  deterministic and restart-proof. Reject-only mismatch prevents same-user DoS of
  recovery. Defaults are config-overridable.
- **Alternatives considered**: OS-level socket peer credentials as identity — breaks
  when agent and CLI are separate processes; file locks per session — leak on crash.

## R9. Filesystem watching + coalescing

- **Decision**: `notify` (recommended watcher per platform) on the worktree root,
  filtering events through the `ignore` crate's matcher (gitignore + `.cairnignore` +
  `.git/` internals except `HEAD`, `index`, and refs — those specific paths signal
  branch switches/commits/rebases). Events feed a per-worktree debouncer: quiescence
  window 500 ms, hard deadline 3 s under continuous churn — then one Git reconciliation
  pass produces the authoritative snapshot (arch rules 7–8). Watcher overflow/error ⇒
  full re-reconcile (hints are droppable by design).
- **Rationale**: Meets SC-003 (≤ 5 s after quiescence) with margin; hint-only semantics
  make lost/duplicated notify events harmless; watching `.git/HEAD` + refs catches
  branch change even with zero worktree file changes.
- **Alternatives considered**: Polling `git status` on a timer — simple but burns CPU on
  big repos and delays detection; watchman — external dependency (Principle IX).

## R10. Ignored-files inspection output (FR-035)

- **Decision**: Inspection response carries `ignored_summary`: total count, rule-source
  breakdown (gitignore vs cairnignore), collapsed top-level ignored roots (dir + entry
  count), ≤ 20 sample paths, `truncated: bool`. Full enumeration only via
  `v1.repository.ignored_files` with cursor pagination (`cursor`, `limit ≤ 1000`,
  glob filter). CLI mirrors: `cairn status` shows summary; `cairn status --ignored`
  pages through the dedicated method.
- **Rationale**: Clarification Q4 policy verbatim; keeps `status` under SC-007 on repos
  with 100k+ ignored paths.

## R11. CLI ↔ daemon lifecycle

- **Decision**: CLI always talks to the daemon over IPC. If the socket/pipe is absent,
  CLI auto-spawns `cairnd` (detached, per-user singleton via socket-bind race; loser
  exits) and retries with backoff (fail after ~3 s with actionable error).
  `cairn daemon status` reports version, uptime, DB path/health, watched repos, active
  session counts. `cairn init` therefore works offline but not daemon-less — acceptable
  because FR-005/FR-024 exclude only *network*; the daemon is local.
- **Rationale**: Single writer to SQLite (no CLI/daemon write races); one code path for
  registration/eventing; auto-spawn keeps UX one-command.
- **Alternatives considered**: CLI writing DB directly when daemon is down — two writers,
  duplicated invariants, split-brain on events; requiring manual `cairnd &` — hostile UX.

## R12. Identifiers and time

- **Decision**: UUIDv7 for all row identities (repositories, worktrees, snapshots,
  sessions, events) — creation-time ordered, index-friendly. `agent_instance_id`
  accepted as any RFC 4122 UUID (caller-generated). All timestamps stored as RFC 3339
  UTC strings with millisecond precision.
- **Rationale**: User input specifies UUIDv7 where chronological ordering helps; RFC 3339
  UTC satisfies "timezone-unambiguous" spec assumption and sorts lexicographically.
- **Alternatives considered**: Integer rowids as public ids — leak ordering across
  entities and complicate future sync; ULID — equivalent benefit, less standard tooling.

## R13. Structured logging

- **Decision**: `tracing` with JSON formatter to a rotating file in the cairn data dir
  plus human console in foreground mode. Field policy enforced centrally: paths logged
  relative to repo root where possible; never log file contents, diffs, token values, or
  env values. Redaction unit test greps emitted logs in integration runs.
- **Rationale**: Constitution: structured logs, no sensitive repository content.
- **Alternatives considered**: `log` crate — no structured fields, no spans.
