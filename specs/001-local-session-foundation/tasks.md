---

description: "Task list for Local Session Foundation implementation"
---

# Tasks: Local Session Foundation

**Input**: Design documents from `/specs/001-local-session-foundation/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: INCLUDED — constitution mandates event replay, snapshot determinism, scope isolation, contract tests, fixtures, and deterministic agent simulations.

**Organization**: Tasks grouped by user story (US1 register+inspect P1, US2 sessions+snapshots P2, US3 live tracking P3, US4 durability+recovery P4).

**Revision note (2026-07-16)**: renumbered after /speckit-analyze remediation — added T024 (events.list handler, was gap G1), T050/T053 (CLI heartbeat/reattach + tests, U3), and folded I1/A1/A2/I2/I3/I4/U1/U2 decisions into task text.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: US1–US4 per spec.md
- Exact file paths in every description

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Workspace skeleton compiling end to end

- [x] T001 Create Rust workspace: root `Cargo.toml` (members: apps/daemon, apps/cli, crates/cairn-domain, crates/cairn-protocol, crates/cairn-git, crates/cairn-events, crates/cairn-session, crates/cairn-storage-local, fixtures/repositories; workspace deps: tokio, clap, serde, serde_json, schemars, sqlx, blake3, uuid v7, notify, ignore, tracing, tracing-subscriber, anyhow/thiserror), plus empty lib/main stubs so `cargo build --workspace` passes
- [x] T002 [P] Add `rustfmt.toml`, workspace lints (`[workspace.lints]` clippy warnings-as-errors), `.gitignore` for target/, and `cargo nextest` config in `.config/nextest.toml`
- [x] T003 [P] Implement Git fixture builder crate in `fixtures/repositories/src/lib.rs`: programmatic construction of scripted repos (init, commit, branch, stage, dirty, detach, rebase-in-progress, linked worktree, huge-ignored-tree, ignored-secrets, **bare repository**, **identity-marker-deleted**, **directory-copied-with-git-data**) used by all integration tests

**Checkpoint**: `cargo build --workspace && cargo nextest run --workspace` green (empty tests OK)

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Domain types, protocol, storage, Git parsing, IPC plumbing every story needs

**⚠️ CRITICAL**: No user story work before this phase completes

- [x] T004 [P] Define identity + time types in `crates/cairn-domain/src/ids.rs` and `crates/cairn-domain/src/time.rs`: RepoUuid, WorktreeUuid, SessionId, SnapshotId, EventId, AgentInstanceId (UUID newtypes, UUIDv7 constructors), RFC 3339 UTC timestamp type
- [x] T005 [P] Define snapshot model + fingerprint canonicalization in `crates/cairn-domain/src/snapshot.rs`: component entry structs, sorted canonical serialization, BLAKE3 component fingerprints, versioned final fingerprint (research R2); pure functions, no IO
- [x] T006 [P] Define session state machine in `crates/cairn-domain/src/session.rs`: SessionState enum (Active/Recovering/Stopped/Interrupted), typed transition function permitting only legal transitions from data-model.md (incl. recovering→stopped authenticated stop; NO transition on failed reattach), liveness reason codes enum (heartbeat_expired, process_dead, reattach_timeout, process_unknown), interruption reason enum (stale_takeover, grace_expired)
- [x] T007 [P] Unit tests in `crates/cairn-domain/tests/fingerprint.rs`: determinism (same entries any insertion order → same fp, repeated 100×) and sensitivity (each mutation class — stage, edit, add untracked, delete — changes exactly the right component fp and the final fp)
- [x] T008 [P] Unit tests in `crates/cairn-domain/tests/session_state.rs`: exhaustive legal/illegal transition matrix incl. recovering→stopped legal, recovering unchanged on rejected reattach, terminal states immovable
- [x] T009 Define protocol DTOs in `crates/cairn-protocol/src/lib.rs` (+ `methods.rs`, `errors.rs`, `dto.rs`): request/response envelopes, all v1 method param/result types from contracts/ipc-contract.md (incl. heartbeat/reattach params with `agent_instance_id`, Session fields `lease_expires_at`/`recovering_since`, register `identity_outcome`, events.list `worktree_id` filter, `agent_pid` param naming), closed error-code enum (incl. NOT_A_WORKTREE, LEASE_EXPIRED), CLI JSON envelope (`cairn.cli.v1`), schemars derive on every type, `SCHEMA_VERSION` constants
- [x] T010 [P] Contract schema tests in `crates/cairn-protocol/tests/schemas.rs`: export JSON Schemas to `crates/cairn-protocol/schemas/`, golden-diff them (breaking-change tripwire), validate sample payloads for every method
- [x] T011 Implement storage bootstrap in `crates/cairn-storage-local/src/lib.rs` (+ `db.rs`): platform data-dir resolution, SQLite pool with WAL/foreign_keys/synchronous=FULL/busy_timeout pragmas, `sqlx::migrate!` wiring, corruption detection mapping to StateCorrupted error
- [x] T012 Copy migration DDL from `specs/001-local-session-foundation/contracts/migrations/0001_init.sql` to `crates/cairn-storage-local/migrations/0001_init.sql` (tables incl. sessions.resume_token_hash/lease_expires_at/recovering_since and events.worktree_id, partial unique session index, append-only + immutability triggers, meta seed)
- [x] T013 Implement DAOs in `crates/cairn-storage-local/src/repos.rs`, `worktrees.rs`, `snapshots.rs`, `sessions.rs`: typed CRUD honoring immutability (snapshots insert-or-get by (worktree_id, snapshot_fp); sessions update only via legal transitions; recovering_since written once, never overwritten)
- [x] T014 Implement serialized transactional event append in `crates/cairn-storage-local/src/events.rs`: per-worktree single-writer processor (one daemon-owned task/actor per worktree serializes all appends + projection changes — analysis I4); `append_with_projection(event, projection_fn)` in one SQLite transaction with seq assignment inside that transaction; duplicate idempotency_key returns the previously accepted event's result WITHOUT re-running projection_fn; seq-ordered reads for replay
- [x] T015 [P] Storage tests in `crates/cairn-storage-local/tests/storage.rs`: migration from empty DB; UPDATE/DELETE on events and snapshots rejected by triggers; duplicate idempotency_key → single row + prior result returned + projection not re-executed; injected projection failure rolls back event append (atomicity); concurrent appends to one worktree serialize in seq order with consistent projections; partial unique index blocks second live session per (repo, agent_instance)
- [x] T016 Define event types in `crates/cairn-events/src/lib.rs`: event enum for the 11-type catalog (data-model.md — incl. session.reattach_rejected audit event that never contains token values, and identity.marker_restored), serde payload structs with worktree_id linkage, per-type idempotency-key derivation rules (research R7), append/replay traits over storage
- [x] T017 [P] Replay tests in `crates/cairn-events/tests/replay.rs`: apply event sequences → rebuild sessions/repositories projections → assert equality with live projections (constitution: event replay)
- [x] T018 Implement Git subprocess runner in `crates/cairn-git/src/runner.rs`: async command execution (tokio), `-z`/NUL-safe output capture, git-missing → GitUnavailable, timeout guard
- [x] T019 Implement repo discovery + porcelain v2 parser in `crates/cairn-git/src/discover.rs` and `crates/cairn-git/src/status.rs`: `rev-parse --show-toplevel --git-common-dir --absolute-git-dir` (incl. bare-repo detection → NotAWorktree), `git status --porcelain=v2 --branch --untracked-files=all -z` parsing (branch headers, detached HEAD, staged/unstaged/untracked, rename/copy — **no `--ignored` on the default path, analysis I2**; git-side ignored enumeration only behind an explicit diagnostic flag), `git worktree list --porcelain`, `git remote` default-remote detection, rebase/merge in-progress detection
- [x] T020 [P] Parser tests in `crates/cairn-git/tests/status_parse.rs` using `fixtures/repositories`: detached HEAD, no-remote, dirty tree, deletions, renames, linked worktree, rebase-in-progress, bare repository rejection (FR-032 matrix)
- [x] T021 Implement IPC server scaffold in `apps/daemon/src/ipc.rs` + `apps/daemon/src/main.rs`: UDS in user-owned 0700 directory on Unix; Windows named pipe `\\.\pipe\cairn-<user>-daemon` created with explicit SECURITY_ATTRIBUTES/DACL granting access only to current user SID + required system principals (never default pipe permissions — analysis I1); JSON-lines framing, request router, per-user singleton via bind race, tracing JSON-file + console init (no token/content fields), graceful shutdown; document that resume tokens transit only this authenticated local IPC
- [x] T022 Implement IPC client + daemon auto-spawn in `apps/cli/src/ipc.rs`: connect, request/response correlation, spawn detached `cairnd` on missing socket/pipe with ~3 s retry backoff → DAEMON_UNAVAILABLE exit 5 (research R11)
- [x] T023 Implement CLI skeleton in `apps/cli/src/main.rs` + `apps/cli/src/output.rs`: clap command tree (init, status, session start/show/heartbeat/reattach/stop, daemon status), global `--json`, envelope rendering per contracts/cli-json-contract.md, exit-code mapping (0–6), stderr-only diagnostics, secure resume-token input plumbing (`--resume-token-stdin` → `CAIRN_RESUME_TOKEN` env → `--resume-token-file`; never ordinary argv; never printed in human output or logs)
- [x] T024 Implement `v1.events.list` handler in `apps/daemon/src/handlers/events.rs`: stable seq ordering, `after_seq` cursor pagination with bounded page size (≤1000), composable repository/worktree/session filters, machine-readable errors (analysis G1)

**Checkpoint**: daemon starts, CLI connects, `cairn daemon status` returns stub, events queryable — user stories can begin

---

## Phase 3: User Story 1 — Register repository and inspect exact state (Priority: P1) 🎯 MVP

**Goal**: `cairn init` + `cairn status` deliver trustworthy exact-state answers, offline, idempotent

**Independent Test**: quickstart.md Scenario 1 (+ Scenario 6 spot checks) on a fresh fixture repo

### Implementation for User Story 1

- [x] T025 [P] [US1] Implement identity markers in `crates/cairn-git/src/identity.rs`: read-or-create `<git-common-dir>/cairn/repository-id` and `<absolute-git-dir>/cairn/worktree-id` (UUIDv7, atomic write); duplicate-path/copy detection returning re-identification info (research R4); marker-loss recovery (analysis U1): search DB by normalized canonical path + Git common-dir metadata — exactly one compatible match → restore marker + `identity.marker_restored` event; no unique match → new identity with explicit warning/`identity_outcome=new_after_marker_loss`; never infer identity from remote URL alone
- [x] T026 [P] [US1] Implement ignored-summary builder in `crates/cairn-git/src/ignored.rs`: ignore-crate walker as the **authoritative** source (analysis I2) stacking gitignore + `.cairnignore`; IgnoredSummary (total, by_source rule provenance, collapsed roots, ≤20 samples, truncated flag) and cursor-paginated enumeration (FR-035); must keep default inspection within the 2 s target
- [x] T027 [US1] Implement registration service in `apps/daemon/src/handlers/repository.rs` (register): discovery → bare repo → NOT_A_WORKTREE with zero writes (analysis U2); identity read-or-create/restore → repositories/worktrees upsert-by-uuid with `identity_outcome` in result → `repository.registered`/`worktree.registered` events via T014 (created=false path emits nothing); non-Git → NOT_A_REPOSITORY with zero writes; copy detection → new identity + copied_from + IDENTITY_CONFLICT data (FR-001…FR-005)
- [x] T028 [US1] Implement inspection handler in `apps/daemon/src/handlers/repository.rs` (inspect + ignored_files): fresh reconciliation → RepoStateInspection DTO (root, branch/detached, head, remote, staged/unstaged/untracked from Git; ignored_summary from walker; worktree info, in_progress); `v1.repository.ignored_files` pagination (FR-006, FR-007, FR-035)
- [x] T029 [US1] Implement real `v1.daemon.status` in `apps/daemon/src/handlers/daemon.rs`: version, pid, uptime, db_path, db_healthy, watched repo + active session counts
- [x] T030 [US1] Wire CLI commands in `apps/cli/src/commands/init.rs`, `status.rs`, `daemon.rs`: human + `--json` output for init (incl. identity_outcome + marker-loss warning)/status(+`--ignored` paging)/daemon status per contracts/cli-json-contract.md
- [x] T031 [P] [US1] Integration tests in `apps/daemon/tests/us1_register_inspect.rs`: acceptance US1-1…5 — fresh init registers once; re-init created=false, single row; non-Git dir → error, DB row count unchanged; bare repo → NOT_A_WORKTREE, zero rows/markers/events; marker deletion → unique restore (+ marker_restored event) and ambiguous → new identity + explicit status; dirty fixture inspection matches `git status` ground truth; offline run (network-isolated env in CI)
- [x] T032 [P] [US1] Contract golden tests in `apps/cli/tests/us1_cli_contract.rs`: `cairn init`/`status`/`daemon status` `--json` outputs validate against cairn-protocol schemas; exit codes 0/3 asserted incl. bare-repo → 3 (SC-008 seed)

**Checkpoint**: US1 fully functional — MVP shippable

---

## Phase 4: User Story 2 — Sessions bound to deterministic snapshots (Priority: P2)

**Goal**: session start/show/stop anchored to exact fingerprinted state; idempotent starts; lease-based staleness

**Independent Test**: quickstart.md Scenario 2; SC-002 determinism loop

### Implementation for User Story 2

- [x] T033 [US2] Implement fingerprint pipeline in `crates/cairn-git/src/fingerprint.rs`: `git ls-files -s` → staged_fp; changed/untracked file content hashing (BLAKE3, spawn_blocking pool, deleted-file sentinel, .cairnignore filtering) → unstaged_fp/untracked_fp; compose final via cairn-domain; consistency read-verify-retry ×3 → SnapshotContention (FR-008…FR-012, research R2/R3)
- [x] T034 [US2] Implement snapshot service in `apps/daemon/src/handlers/snapshot.rs`: `v1.snapshot.create` — insert-or-get by (worktree_id, snapshot_fp), `snapshot.created` event only on create, created flag in result
- [x] T035 [US2] Implement session lifecycle core in `crates/cairn-session/src/lib.rs`: start (agent_instance resolution, healthy-collision idempotent return when lease unexpired / stale-collision takeover per FR-034 — staleness = lease expiry, `process_dead` may confirm earlier, `process_unknown` NEVER implies death, reason codes recorded); 256-bit resume-token generation + BLAKE3 `resume_token_hash` storage; configurable initial lease (long default) vs heartbeat TTL extension; heartbeat refresh of last_heartbeat_at + lease_expires_at; stop with final snapshot (active AND authenticated recovering stop); adaptive get resolution single/ambiguous (FR-036); list
- [x] T036 [US2] Wire session IPC handlers in `apps/daemon/src/handlers/session.rs`: `v1.session.start/get/list/stop/heartbeat` emitting `session.started`/`session.stopped`/`session.interrupted(stale_takeover + liveness_detail)` events transactionally with projection updates via the per-worktree writer; heartbeat errors LEASE_MISMATCH (reject-only + audit event) / LEASE_EXPIRED
- [x] T037 [US2] Wire CLI session commands in `apps/cli/src/commands/session.rs`: `session start --agent [--agent-instance] [--agent-pid]` (env `CAIRN_AGENT_INSTANCE` fallback, generated id printed, resume token in `--json` only), `session show` (ambiguous → exit 4 + candidate list), `session stop`; human + `--json`
- [x] T038 [P] [US2] Determinism + sensitivity integration tests in `crates/cairn-git/tests/fingerprint_determinism.rs`: unchanged fixture → 100 snapshots, one fingerprint (SC-002); mutation matrix (commit, branch switch, stage, edit, untracked add, delete) each flips fingerprint; concurrent-mutation-during-snapshot → retry or SNAPSHOT_CONTENTION, never torn (FR-012)
- [x] T039 [P] [US2] Session lifecycle integration tests in `apps/daemon/tests/us2_sessions.rs`: acceptance US2-1…8 — start captures start snapshot; double start same instance with unexpired lease → existing, no new token; expired lease + dead pid → takeover with `process_dead`; expired lease + unknown pid → takeover only via lease expiry with `process_unknown` (absence of PID never accelerates staleness); two instances coexist; stop records event; scope isolation across instances
- [x] T040 [P] [US2] Deterministic agent simulation in `apps/daemon/tests/us2_agent_sim.rs`: scripted client driving start→heartbeat→edit→stop over real IPC, asserting event sequence golden (constitution: deterministic agent simulations); reopened suite passed after watcher-readiness remediation on 2026-07-19
- [x] T041 [P] [US2] Contract goldens for session/snapshot methods in `apps/cli/tests/us2_cli_contract.rs`: start/show/stop `--json` schema validation, exit code 4 on ambiguity, token absent from human output

**Checkpoint**: PASS — T040 passed with fresh watcher-readiness evidence on 2026-07-19

---

## Phase 5: User Story 3 — Live repository-state tracking (Priority: P3)

**Goal**: active sessions track fs changes; hints reconciled against Git; coalesced but lossless

**Independent Test**: quickstart.md Scenario 3

### Implementation for User Story 3

- [x] T042 [US3] Implement watcher in `apps/daemon/src/watch/mod.rs` + `watch/filter.rs`: notify recommended-watcher per active-session worktree; filter through ignore stack; include `.git/HEAD`, `.git/index`, refs (branch/commit/rebase signals), exclude other `.git` internals; overflow/error → full-reconcile flag (research R9)
- [x] T043 [US3] Implement coalescer in `apps/daemon/src/watch/debounce.rs`: per-worktree 500 ms quiescence window, 3 s hard deadline under churn, collapse burst → single reconcile request (FR-023)
- [x] T044 [US3] Implement reconciliation loop in `apps/daemon/src/watch/reconcile.rs`: on trigger run fingerprint pipeline; fingerprint unchanged → no-op (touch case, FR-022); changed → snapshot insert-or-get + `repository.state_changed` (+ `branch.changed` when branch/HEAD symbolic ref differs) + sessions.current_snapshot_id update, all through the per-worktree serialized writer (analysis I4); start/stop watching on session lifecycle events
- [x] T045 [P] [US3] Watcher integration tests in `apps/daemon/tests/us3_tracking.rs`: acceptance US3-1…5 — edit reflected ≤5 s after quiescence (SC-003); branch switch → branch.changed + new current snapshot; 100-write burst → final state matches `git status`, coalesced; touch-no-change → zero new snapshots; delete + rebase reflected without corruption; reopened suite plus deterministic race/failure coverage passed on 2026-07-19
- [x] T046 [P] [US3] Event-stream tests in `apps/daemon/tests/us3_events.rs` against the T024 handler: seq ordering, `next_after_seq` pagination, repository/worktree/session filter composition, state_changed payload from/to snapshot linkage; reopened suite passed on 2026-07-19

**Checkpoint**: PASS — T045/T046 and the Phase 8 watcher-race coverage passed on 2026-07-19

---

## Phase 6: User Story 4 — Restart durability and session recovery (Priority: P4)

**Goal**: committed events survive any kill; recovery usable end-to-end via CLI before adapters exist

**Independent Test**: quickstart.md Scenario 4

### Implementation for User Story 4

- [x] T047 [US4] Implement boot recovery in `apps/daemon/src/recovery.rs`: on startup mark all `active` sessions `recovering`, persisting `recovering_since` ONLY if not already set (repeated restarts preserve the original timestamp — analysis A2); rebuild watcher set; detect DB corruption → serve STATE_CORRUPTED and refuse fabricated state (FR-033)
- [x] T048 [US4] Implement grace sweeper in `apps/daemon/src/recovery.rs` (sweeper task): recovering sessions past deadline `recovering_since + recovery_grace_period` (default 15 min, configurable) → `interrupted` + `session.interrupted(grace_expired)` event; lease-expiry staleness sweep for active sessions with reason codes
- [x] T049 [US4] Implement `v1.session.reattach` in `apps/daemon/src/handlers/session.rs`: verify agent_instance_id + resume-token hash; success → recovering→active, clear recovering_since, `session.recovered` event, fresh snapshot captured and set current; wrong/missing token → LEASE_MISMATCH reject-only + `session.reattach_rejected` audit event (no token values), session untouched (analysis I3); past deadline → GRACE_EXPIRED; authenticated recovering→stopped supported via stop handler
- [x] T050 [US4] Wire CLI recovery commands in `apps/cli/src/commands/session.rs`: `session heartbeat` and `session reattach` (session ID + agent-instance ID + resume token via secure input: `--resume-token-stdin` / `CAIRN_RESUME_TOKEN` / `--resume-token-file`; never argv, never printed in human output or logs; reattach `--json` returns new token) exercising the same daemon handlers adapters will use (analysis U3)
- [x] T051 [P] [US4] Implement the configurable crash/restart harness in `apps/daemon/tests/us4_crash_restart.rs`: spawn real daemon, generate events, SIGKILL/TerminateProcess at randomized points, restart, assert zero committed-event loss and all pre-kill active sessions recovering; permit a smaller ordinary local default for fast feedback; repo mutated during downtime → next snapshot reflects reality (US4-4). This checked implementation task does not itself satisfy SC-005 until the separate exactly-100-iteration acceptance task passes.
- [x] T052 [P] [US4] Recovery-path tests in `apps/daemon/tests/us4_recovery.rs`: valid reattach before deadline → active + recovered event + fresh snapshot; wrong token → LEASE_MISMATCH + reattach_rejected audit event + session STILL recovering; repeated daemon restarts do NOT extend the grace deadline (recovering_since preserved — analysis A2); deadline expiry → interrupted(grace_expired); authenticated stop of recovering session → stopped; corrupted DB (truncated file) → STATE_CORRUPTED, CLI exit 6
- [x] T053 [P] [US4] CLI recovery contract tests in `apps/cli/tests/us4_cli_contract.rs`: heartbeat success (lease extended) / expired lease → LEASE_EXPIRED; reattach valid / mismatch rejection / grace expiry `--json` goldens; token never in human output; secure-input resolution order (stdin → env → file) verified

**Checkpoint**: US4 recovery behavior is implemented. Current Feature 001 blockers are typed `WATCHER_START_FAILED` contract/golden coverage; authoritative installation-window deletion coverage; exact-commit Windows and macOS evidence; exact-commit Linux network-isolation evidence; exact-commit 100-kill CI evidence; explicit SC-007 performance execution; and final convergence evidence.

---

## Phase 7: Polish & Cross-Cutting Concerns

- [x] T054 [P] Privacy audit test in `apps/daemon/tests/privacy_audit.rs`: ignored-secrets fixture → full DB dump + log files contain zero secret bytes, zero ignored-file contents, and zero raw resume tokens (SC-006, FR-026…028)
- [x] T055 [P] Log redaction policy in `apps/daemon/src/logging.rs` + test: central field policy (no contents, no diffs, no tokens, no env values), integration grep over emitted JSON logs (constitution: structured logs)
- [x] T056 [P] Performance suite in `apps/daemon/tests/perf.rs`: 10k-tracked-file fixture — inspect < 2 s (walker-based ignored summary included), snapshot < 2 s (SC-007); quiescence→snapshot ≤ 5 s (SC-003); `#[ignore]`-by-default with CI nightly job
- [x] T057 [P] SC-008 machine-output loop in `apps/cli/tests/json_stability.rs`: scripted consumer parses all eight commands × success/failure repeatedly; schema goldens diffed
- [x] T058 [P] Align the cross-platform CI workflow in `.github/workflows/ci.yml`: run `cargo test --workspace --all-targets` on windows-latest, ubuntu-latest, and macos-latest; retain `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, the schema-golden diff gate, and the Windows named-pipe ACL negative test; Nextest is not required by this feature. Workflow aligned and exact commands passed locally on 2026-07-19.
- [x] T059 Run quickstart.md Scenarios 1–6 end-to-end on Windows and macOS against the same frozen Feature 001 implementation commit. For each OS, record OS version/architecture, Rust and Cargo versions, exact implementation SHA, exact commands, scenario-by-scenario results, required event counts, and `cargo test --workspace --all-targets`, `cargo fmt --check`, and `cargo clippy --workspace --all-targets -- -D warnings` outcomes. The macOS run must begin from a clean checkout of that commit; a Windows CI job is acceptable only when it checks out and tests that exact SHA. Record the implementation commit tested separately from the possibly newer evidence-document commit. A configured matrix or watcher-pre-remediation Windows run is not evidence. Completed on frozen implementation SHA `4a06c4125715bb4b78b54e49c81eccd82100a7b7` with a clean detached macOS run and the successful exact-SHA Windows GitHub Actions job recorded in `evidence/quickstart-run.md` (T071/T073, Constitution IV).
- [x] T060 [P] Workspace docs: `README.md` quick usage + crate-level `//!` docs for the six crates and two apps, module ownership table copied from plan.md

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: none
- **Foundational (Phase 2)**: after Setup — BLOCKS all stories
- **US1 (Phase 3)**: after Phase 2
- **US2 (Phase 4)**: after Phase 2; uses US1 registration at runtime (fixtures can self-register) — testable independently
- **US3 (Phase 5)**: after US2 (needs sessions + snapshot pipeline)
- **US4 (Phase 6)**: after US2 (needs sessions); US3 not required
- **Polish (Phase 7)**: after desired stories

### Key task-level dependencies

- T012 → T011 (migrations before DAOs run) → T013 → T014 → T024
- T009 → T010, T021, T022, T023 (protocol before transport/CLI)
- T025/T026 → T027/T028 (identity + ignored before handlers)
- T033 → T034 → T035/T036 (fingerprints before snapshots before sessions)
- T042 → T043 → T044 (watch pipeline in order)
- T047/T048 → T049 → T050 (recovery states before reattach before CLI)

### Parallel opportunities

- Phase 2: T004–T008 (domain) ∥ T009–T010 (protocol) ∥ T018–T020 (git) after T001; T015/T017 ∥ following their impl tasks
- US1: T025 ∥ T026; T031 ∥ T032
- US2: T038 ∥ T039 ∥ T040 ∥ T041 after T037
- US3/US4 test tasks ∥ after their handlers; T051 ∥ T052 ∥ T053 after T050
- Phase 7: T054–T058, T060 all ∥
- After Phase 2: US1 and US2 implementation can proceed in parallel by different contributors; US3 and US4 in parallel after US2

---

## Parallel Example: Phase 2 kickoff

```bash
# After T001 lands, run concurrently:
Task: "T004 domain ids"        # crates/cairn-domain
Task: "T009 protocol DTOs"     # crates/cairn-protocol
Task: "T018 git runner"        # crates/cairn-git
Task: "T003 fixture builder"   # fixtures/repositories
```

---

## Implementation Strategy

### MVP First (US1 only)

1. Phase 1 → Phase 2 → Phase 3
2. **STOP and VALIDATE**: quickstart Scenario 1 + 6 on Windows and Linux
3. MVP = `cairn init` / `cairn status` / `cairn daemon status` with contract-stable JSON

### Incremental Delivery

1. +US2 → sessions with deterministic anchors (core product promise) → validate Scenario 2
2. +US3 → live tracking → validate Scenario 3
3. +US4 → durability hardening + CLI recovery → validate Scenario 4
4. Polish → privacy/perf/CI evidence → feature complete per constitution IV

---

## Notes

- Every state-changing surface ships with its migration (T012) — constitution workflow gate
- Protocol changes require regenerating schema goldens (T010) — breaking-change tripwire
- Commit after each task or logical group; evidence artifacts land in specs/001-local-session-foundation/evidence/

---

## Phase 8: Convergence

**Goal**: close the watcher-readiness race and produce current, observable acceptance evidence for Feature 001 without beginning another feature

- [x] T061 [US2] Complete typed watcher-failure contracts across `crates/cairn-domain`, `crates/cairn-protocol`, `crates/cairn-events`, daemon IPC, and CLI mappings for FR-037/FR-038: represent `WATCHER_START_FAILED` with a typed discriminated payload equivalent to `data: {"kind":"watcher_start_failure","stage":"install|reconcile"}`; constrain the JSON Schema by error code so this payload is not arbitrary JSON; retain the legal `active→interrupted` transition and `session.interrupted(reason=watcher_start_failed, watcher_stage)` replay behavior; add IPC request/response goldens and CLI JSON-envelope goldens for both `stage=install` and `stage=reconcile`; assert both CLI paths exit 1; add schema-breaking-change tripwire coverage; and prove raw internal errors, paths, environment values, tokens, and repository contents never leak. Completed with passing protocol schema/golden, CLI contract, event replay, workspace, formatting, and Clippy validation on 2026-07-19.
- [x] T062 [US2] Implement watcher installation requests with explicit ready acknowledgement in `apps/daemon/src/watch/mod.rs`: acknowledge only after the OS watcher is installed and the event-processing path is ready; expose an awaitable result to session-start orchestration rather than fire-and-forget queuing (FR-037)
- [x] T063 [US2] Change `apps/daemon/src/handlers/session.rs` and watcher reconciliation orchestration to enforce session creation → watcher request → watcher-ready acknowledgement → authoritative post-install Git reconciliation → session-start response; capture changes made during the installation window; on install/reconcile failure return `WATCHER_START_FAILED`, atomically interrupt the created session through append-only evidence, and leave no falsely healthy active session (FR-037, FR-038, Constitution I/III/IV)
- [x] T064 [P] Add deterministic watcher controls in `apps/daemon/src/watch/mod.rs`, enabled and exposed only by `apps/daemon/tests/support/mod.rs`: synchronization barriers/explicit acknowledgements to pause and release installation, inject install/reconciliation failure, drop notifications, and observe installation/reconciliation completion; timing sleeps must not be the primary correctness mechanism (SC-003)
- [x] T065 [P] Expand `apps/daemon/tests/us2_agent_sim.rs` and `apps/daemon/tests/us3_tracking.rs` with deterministic watcher-race coverage: retain the immediate post-return edit and installation-window create/modify/rename cases, and add an authoritative deletion case that places a committed or otherwise initially tracked file in the initial snapshot, pauses watcher installation through the existing synchronization control, deletes that file during the installation window, resumes installation, completes post-install Git reconciliation, and asserts the session-start result/current snapshot reflects the deletion, the expected `repository.state_changed` event exists, and notification plus reconciliation produce no duplicate logical change event. Use barriers/acknowledgements, not correctness sleeps. Completed with the seed-committed deletion barrier test and passing `us2_agent_sim`, `us3_tracking`, `us3_events`, and workspace validation on 2026-07-19 (US2/AC9-11, US3/AC6).
- [x] T066 [P] In `apps/daemon/tests/us3_tracking.rs`, cover coalesced bursts, reconciliation after a deliberately dropped notification, and event idempotency when a notification and reconciliation observe the same change; assert no duplicate `repository.state_changed` event for one snapshot transition (FR-022, FR-023, US3/AC7-8)
- [x] T067 [P] Add watcher-installation failure and daemon-restart recovery coverage in daemon integration tests: failed start returns the stable error and leaves an interrupted session with append-only evidence; restart reinstalls each recovering-session watcher, acknowledges readiness, reconciles Git reality, and creates no duplicate change event (FR-018, FR-037, FR-038, US3/AC9)
- [x] T068 Add a dedicated SC-005 CI acceptance execution using the configurable crash harness with exactly `CAIRN_CRASH_ITERS=100`; fail unless all 100 forced kills complete with zero committed-event loss and every affected session follows the recovery/interruption contract; record the exact command, completed count, and results in feature evidence. The initial local 100-kill pass remains historical; final evidence is the successful dedicated GitHub Actions job against frozen SHA `4a06c4125715bb4b78b54e49c81eccd82100a7b7`, with 100 completed forced kills, zero committed-event loss, and zero invalid session outcomes on 2026-07-19.
- [x] T069 Add Linux OS-level network-isolated CI validation for SC-001/FR-024 against the same frozen Feature 001 implementation commit used by Windows and macOS: fetch dependencies and build required binaries/tests before isolation; inside an OS-level network namespace, or a container launched with networking disabled, execute the relevant CLI, daemon, repository registration/inspection, session, live-change, and quickstart behavior while preserving local IPC/filesystem access. The job must prove external networking is unavailable, filesystem access works, and local IPC works; it must fail explicitly rather than silently skip when network namespaces are unavailable, using a genuinely isolated fallback such as a no-network container. Record the workflow run/job reference, exact implementation SHA, Ubuntu version/architecture, exact isolation mechanism, commands, and scenario results; `cargo --offline` or configured-only workflow text is not evidence. Completed successfully on Ubuntu 24.04.4 x86_64 against frozen SHA `4a06c4125715bb4b78b54e49c81eccd82100a7b7` using Docker `--network none`, with completed external-network, filesystem, local-IPC, and scenario proofs recorded in `evidence/quickstart-run.md` (T074).
- [x] T070 Complete reopened T058 by updating `.github/workflows/ci.yml` to run `cargo test --workspace --all-targets` across Windows/Linux/macOS while retaining `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, schema-golden verification, and the Windows named-pipe ACL negative test (plan: CI acceptance)
- [x] T071 Complete T059 by running Scenarios 1–6 on macOS from a clean checkout of the same frozen Feature 001 implementation commit tested on Windows. Record macOS version/architecture, Rust and Cargo versions, exact implementation SHA, exact commands/output, relevant event counts, and workspace-test/fmt/Clippy outcomes in `evidence/quickstart-run.md`; record the evidence-document commit separately and do not substitute a configured CI matrix or dirty working-tree run for completed evidence. Completed from a clean detached macOS 26.5.2 arm64 checkout of `4a06c4125715bb4b78b54e49c81eccd82100a7b7`; the distinct exact-SHA evidence payload was committed as `808fdc257a9f42b0a3370448f624618bfb95e2bc` before final analysis/convergence (T059/T073, Constitution IV).
- [x] T072 Run and record the pre-final Feature 001 verification after T061–T071: typed watcher contracts/goldens, authoritative installation-window deletion, `us2_agent_sim`, all `us3_tracking`, `us3_events`, `cargo test --workspace --all-targets`, exactly-100-kill SC-005 acceptance, Linux network-isolated validation, same-commit Windows/macOS Scenarios 1–6, `cargo fmt --check`, and `cargo clippy --workspace --all-targets -- -D warnings`; leave final convergence to authoritative T076 after exact-commit evidence and explicit SC-007 execution exist. Completed against frozen SHA `4a06c4125715bb4b78b54e49c81eccd82100a7b7`; all pre-final suites and explicit SC-007 executions passed and are recorded in `evidence/quickstart-run.md` (Constitution IV).

**Convergence dependencies**: T061 → T062 → T063; T064 enables T065–T067; T063 enables T065–T067; T068–T071 may proceed after their required implementation/CI prerequisites; T072 runs last.

**Checkpoint**: Phase 8 is only a pre-final verification checkpoint. Feature 001 is not converged until T073–T076 also complete and T076 records 76/76 with fresh observable evidence.

---

## Phase 9: Convergence

**Goal**: replace dirty-tree, stale, and configured-only claims with completed evidence against one frozen Feature 001 implementation commit, then close the authoritative 76-task gate without beginning Feature 002

- [x] T073 Freeze one exact Feature 001 implementation commit and complete cross-platform E1 against that same SHA: from a clean checkout on macOS, and from a Windows checkout or GitHub Actions job explicitly checking out the exact SHA, run Quickstart Scenarios 1–6. For each OS record OS version/architecture, Rust and Cargo versions, implementation SHA, exact commands, scenario-by-scenario results, required event counts, and `cargo test --workspace --all-targets`, `cargo fmt --check`, and `cargo clippy --workspace --all-targets -- -D warnings` outcomes. Record the possibly newer evidence-document commit separately. Watcher-pre-remediation Windows output, a dirty macOS tree, or a configured matrix is not evidence. Completed with frozen implementation SHA `4a06c4125715bb4b78b54e49c81eccd82100a7b7`; clean detached macOS and exact-SHA Windows evidence is recorded, and the distinct exact-SHA evidence payload was committed as `808fdc257a9f42b0a3370448f624618bfb95e2bc` before final analysis/convergence.
- [x] T074 Complete Linux isolation E2 against the same frozen implementation SHA used by T073: run the Linux network-isolated CI job and record its workflow run/job reference, exact implementation SHA, Ubuntu version/architecture, pre-isolation dependency fetch/build, exact isolation mechanism, proof external networking is unavailable, proof filesystem and local IPC access work, commands, and passing scenario results. If `unshare(1) --net` is unavailable, use a genuinely isolated alternative such as a container launched with networking disabled; fail explicitly rather than silently skip. A configured workflow without a completed successful job is not evidence. Completed by successful GitHub Actions job `88203425632` against `4a06c4125715bb4b78b54e49c81eccd82100a7b7` using Docker `--network none`, with all required proofs recorded; T069 is complete.
- [x] T075 Complete exactly-100-kill E3 in the dedicated CI job against the same frozen implementation SHA used by T073/T074. Record the workflow run/job reference, exact implementation SHA, configured iteration count `100`, completed iteration count `100`, `committed_event_loss=0`, `invalid_session_outcomes=0`, and final job result. A smaller run or a local dirty-working-tree result does not satisfy the final acceptance gate. Completed by successful GitHub Actions job `88203425605` against `4a06c4125715bb4b78b54e49c81eccd82100a7b7`: configured 100, completed 100, committed-event loss 0, invalid session outcomes 0.
- [ ] T076 Execute the final Feature 001 completion gate on the exact frozen implementation commit and declare convergence only when all 76 authoritative tasks are complete (76/76); T061 typed `WATCHER_START_FAILED` schemas, IPC/CLI goldens, exit mappings, tripwire, and non-leakage tests pass; T065 authoritative installation-window deletion passes; `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets`, `us2_agent_sim`, all `us3_tracking`, and `us3_events` pass; `cargo test -p cairn-daemon --test perf -- --ignored` explicitly passes with evidence recording the exact implementation SHA, OS/architecture, 10,000-tracked-file fixture size, measured inspect and snapshot durations, SC-007 limits, and result; Windows Scenarios 1–6 and a clean-checkout macOS Scenarios 1–6 pass on that same SHA; Linux network-isolated validation and the exactly-100-kill CI job pass on that same SHA; evidence records real completed executions and its own evidence commit; and Feature 002 remains untouched (partial).
