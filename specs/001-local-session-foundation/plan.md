# Implementation Plan: Local Session Foundation

**Branch**: `001-local-session-foundation` | **Date**: 2026-07-16 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/001-local-session-foundation/spec.md`

## Summary

Build Cairn's local foundation: a native daemon and CLI that register Git repositories
under stable Git-private identities, inspect exact repository state via the Git CLI,
produce deterministic BLAKE3 snapshot fingerprints, manage agent sessions (active /
recovering / stopped / interrupted) keyed by per-agent-instance UUIDs, and persist an
append-only event history in daemon-local SQLite — fully offline, privacy-safe, with
filesystem events treated only as invalidation hints reconciled against Git truth.

## Technical Context

**Language/Version**: Rust (stable, workspace edition 2021; MSRV pinned in workspace `Cargo.toml`)

**Primary Dependencies**: Tokio (async runtime), Clap (CLI), Serde + schemars (contracts/JSON Schema), SQLx (SQLite driver, compile-checked queries), BLAKE3 (fingerprints), uuid v7 (identifiers), notify (filesystem events), ignore (.gitignore/.cairnignore), tracing + tracing-subscriber (structured logs), UDS on Linux/macOS and named pipes on Windows for CLI↔daemon IPC

**Storage**: SQLite (WAL mode) via SQLx — one per-user local database; filesystem for Git-private identity markers

**Testing**: `cargo test --workspace --all-targets`; repository fixtures under `fixtures/repositories`; contract tests validating JSON Schemas; crash/restart and replay determinism suites. Quality gates also require `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings`.

**Target Platform**: Windows, Linux, macOS developer machines (daemon + CLI, ordinary user privileges)

**Project Type**: Multi-crate Rust workspace — two binaries (daemon, CLI) + library crates

**Performance Goals**: Inspection and snapshot each < 2 s on 10,000 tracked files (SC-007); current-snapshot refresh within 5 s of filesystem quiescence (SC-003)

**Constraints**: Fully offline (FR-024); append-only immutable events (FR-019/FR-020); no secret values or ignored-file contents persisted (FR-026–028); deterministic fingerprints (FR-009/FR-010); snapshot consistency under concurrent change (FR-012)

**Scale/Scope**: Single developer machine; multiple registered repositories; multiple concurrent sessions per repository (one per agent instance); event history growth unbounded by design (append-only, metadata-sized rows)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Gate | Status |
|---|---|---|
| I. Observable reality authoritative | Git reconciliation produces authoritative state; fs notifications advisory only (arch rules 7–8) | PASS |
| II. Exact execution scope | The bootstrap-only v1 contract explicitly classifies every session as `local_unbound`; sessions remain scoped to user, repository, worktree, agent instance, branch/commit through start/current snapshots, expose no project-aware synchronization, and cannot become project truth | PASS |
| III. Append-only history | Events immutable, append-only, idempotent; SQLite triggers forbid UPDATE/DELETE; projections rebuildable | PASS |
| IV. Evidence before confidence | Session states explicit (active/recovering/stopped/interrupted); snapshots are evidence records | PASS |
| V. Automatic operation | Auto snapshot refresh, coalescing, recovery, staleness handling — no routine human input | PASS |
| VI. Goal stability | Task revisioning and later project/task binding are out of scope; the future capability must define migration and append-only binding before synchronization | N/A |
| VII. Local repository truth | Daemon owns all local truth; no central server in scope (arch rule 13) | PASS |
| VIII. Deterministic before AI | Zero AI; Git CLI + hashes only | PASS |
| IX. Minimal infrastructure | SQLite + filesystem only; no pgvector/Valkey/brokers/embeddings (arch rule 13) | PASS |
| X. Privacy & secret containment | ignore + .cairnignore respected; content hashes only, never contents; nothing uploaded | PASS |
| Tech constraints | Rust/Tokio/Serde/SQLx/tracing per constitution. Axum/Tower not used: no HTTP surface exists in this feature (IPC is UDS/named pipes); smallest-correct-architecture rule applies | PASS (deviation noted, justified) |
| Workflow gates | Migrations included; IPC + CLI contracts defined with contract tests; fixtures + deterministic agent simulation planned | PASS |

**Post-Phase-1 re-check (2026-07-16)**: design artifacts introduce no new infrastructure,
no server, no AI; events remain append-only with transactional projections. PASS.

**Pre-final convergence re-check (2026-07-19)**: Runtime and exact-SHA evidence PASS.
`us2_agent_sim`, `us3_tracking`, `us3_events`, the full workspace suite, formatting,
Clippy, exactly-100-kill acceptance, Linux network isolation, Windows/macOS scenarios,
and explicit SC-007 performance execution all pass against frozen implementation SHA
`4a06c4125715bb4b78b54e49c81eccd82100a7b7`. Constitution v1.1.0 classifies this
bootstrap-only contract as `local_unbound`; project/task binding and migration remain
out of scope. Feature completion remains at 75/76 until the fresh analysis, convergence,
and T076 declaration gate succeed.

## Project Structure

### Documentation (this feature)

```text
specs/001-local-session-foundation/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output (entities + SQLite schema)
├── quickstart.md        # Phase 1 output (validation guide)
├── contracts/
│   ├── ipc-contract.md          # daemon IPC methods, envelopes, errors
│   ├── cli-json-contract.md     # CLI machine-readable output contract
│   └── migrations/
│       └── 0001_init.sql        # initial SQLite migration (design artifact)
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT created by /speckit-plan)
```

### Source Code (repository root)

```text
apps/
├── daemon/                  # cairnd binary: tokio runtime, IPC server, fs watcher,
│   └── src/                 # coalescing, session liveness sweeper, orchestration
└── cli/                     # cairn binary: clap commands, IPC client,
    └── src/                 # human + --json rendering, daemon auto-spawn

crates/
├── cairn-domain/            # pure types + logic: identities, snapshot model,
│   └── src/                 # fingerprint composition, session state machine (no IO)
├── cairn-protocol/          # serde DTOs, JSON Schemas (schemars), method names,
│   └── src/                 # error codes, protocol + CLI schema versions
├── cairn-git/               # Git CLI adapter: porcelain v2 parsing, rev-parse,
│   └── src/                 # worktree list, identity marker files, reconciliation
├── cairn-events/            # event types, idempotent append API, replay,
│   └── src/                 # projection traits
├── cairn-session/           # session lifecycle service: start/idempotent-return,
│   └── src/                 # heartbeat, lease verify, recovery, staleness
└── cairn-storage-local/     # SQLx SQLite: pool, migrations, DAOs, single-txn
    └── src/                 # event-append + projection update

fixtures/
└── repositories/            # scripted Git fixture builders (detached HEAD, rebase,
                             # worktrees, no-remote, dirty trees, huge-ignored)
```

**Structure Decision**: Rust workspace monorepo per constitution and user input. Domain
logic isolated in `cairn-domain` (arch rule 1: no CLI/transport dependencies). Apps
depend on service crates; service crates depend on domain; `cairn-protocol` is the only
shared contract surface between daemon and CLI.

### Module Ownership Map

| Module | Owns | Must not contain |
|---|---|---|
| `cairn-domain` | Entity types, fingerprint canonicalization, session/snapshot invariants, state transitions | IO, SQL, transport, Git invocation |
| `cairn-protocol` | IPC method names + DTOs, CLI JSON envelope, error codes, JSON Schemas, version constants | Business logic, persistence |
| `cairn-git` | All `git` subprocess execution, porcelain v2 parsing, identity marker read/write, consistency retry loop | SQL, session logic |
| `cairn-events` | Event enum + payload schemas, idempotency-key rules, append/replay traits | Direct SQLite access (uses storage traits) |
| `cairn-session` | Lifecycle policy: idempotent start, lease/heartbeat, recovering-state machine, grace period | Transport, SQL specifics |
| `cairn-storage-local` | Migrations, WAL setup, DAOs, transactional append+projection, corruption detection | Domain policy decisions |
| `apps/daemon` | IPC server, notify watcher + coalescer, liveness sweeper, wiring | Business rules (delegates to crates) |
| `apps/cli` | Argument parsing, IPC client, human/JSON rendering, exit codes, daemon auto-spawn | Business rules, direct DB access |
| `fixtures/repositories` | Deterministic Git repo builders for tests | Production code |

Dependency direction (arch rule 1): `apps/* → cairn-{session,events,git,storage-local,protocol} → cairn-domain`.
`cairn-domain` depends on no internal crate. `cairn-protocol` is transport-free (serde types only).

## Watcher Readiness Protocol

Session start is a coordinated readiness protocol, not a fire-and-forget watcher command.
The daemon MUST execute this sequence:

```text
session creation
  → watcher installation request
  → operating-system watcher installed
  → watcher event-processing path ready
  → watcher-ready acknowledgement
  → authoritative post-install Git reconciliation
  → session-start response
```

The acknowledgement crosses from the per-worktree watcher task back to the session-start
orchestrator and confirms both OS installation and event-path readiness. The post-install
reconciliation compares Git reality with the initial session snapshot and updates the
current snapshot transactionally when the installation window contained changes. It runs
even when no filesystem notification was received; notifications remain advisory hints.

If watcher installation or reconciliation fails, `v1.session.start` returns
`WATCHER_START_FAILED` with a typed, schema-discriminated payload equivalent to
`data = {kind: watcher_start_failure, stage: install|reconcile}`. The schema constrains
that payload when this error code is selected; arbitrary JSON is not accepted. A session
created before the failure transitions to `interrupted` and emits `session.interrupted`
with reason `watcher_start_failed` and the failed stage through the existing serialized
append plus projection transaction. No success envelope or falsely healthy active session
is allowed. IPC and CLI goldens cover both stages, both CLI mappings exit 1, and the
schema-breaking-change tripwire detects incompatible changes. Failure payloads and logs
contain no raw internal error details, paths, repository content, environment values, or
token material.

Daemon recovery applies the same readiness barrier when reinstalling watches for
recovering sessions and performs a post-install reconciliation before the watcher is
counted healthy. Reconciliation and event idempotency must prevent duplicate
`repository.state_changed` events when a notification and readiness reconciliation observe
the same snapshot transition.

## Testing Strategy

| Layer | What is proven | Where |
|---|---|---|
| Unit (domain) | Fingerprint determinism (SC-002) and sensitivity (FR-010); session state machine legality (no active→active, recovering rules) | `cairn-domain` tests |
| Git integration | Porcelain parsing across detached HEAD, rebase-in-progress, no-remote, worktrees, dirty trees, deletions (FR-032) | `cairn-git` tests + `fixtures/repositories` |
| Storage | Migration up from empty; append-only enforcement (UPDATE/DELETE on events rejected); idempotent append (same idempotency key → one row); single-txn append+projection atomicity under injected failure | `cairn-storage-local` tests |
| Event replay | Projections rebuilt from event log equal live projections (constitution: event replay, snapshot determinism) | `cairn-events` tests |
| Contract | Every IPC response and CLI `--json` output validates against schemas in `cairn-protocol`; `WATCHER_START_FAILED` has a discriminated `watcher_start_failure` payload with `install|reconcile` stage, IPC and CLI goldens for both stages, exit-code-1 assertions, non-leakage checks, and a breaking-change tripwire | `cairn-protocol` tests + CLI integration |
| Session lifecycle | Idempotent start (healthy), stale takeover, reattach within grace, grace expiry → interrupted; deterministic agent simulation script (constitution: deterministic agent simulations) | `cairn-session` + daemon integration |
| Watcher readiness and races | Explicit barriers/acknowledgements prove OS watcher installation, event-path readiness, post-install reconciliation, immediate post-return edit, create/modify/rename during a deliberately paused install, and deletion of a file present in the initial authoritative snapshot during that window; the returned/current snapshot, expected change event, and logical-event deduplication are asserted. Coverage also includes coalesced bursts, dropped notification recovery, installation failure, and restart reinstallation; timing sleeps are not the primary correctness mechanism | `apps/daemon/tests/support`, `us2_agent_sim`, `us3_tracking`, `us3_events`, recovery integration tests |
| Crash/restart | Configurable fast local loop plus an acceptance execution with exactly 100 forced daemon kills: zero committed event loss, sessions → recovering (SC-005); corrupted DB detected and reported, never fabricated (FR-033) | daemon integration tests + dedicated CI acceptance job |
| Concurrency | Repo mutated during snapshot → bounded retry, consistent snapshot or explicit failure (FR-012) | `cairn-git` + daemon tests |
| Privacy | Snapshot of fixture with ignored secret files: DB audit finds no secret bytes, no ignored contents (SC-006) | integration audit test |
| Performance | Explicitly run `cargo test -p cairn-daemon --test perf -- --ignored` on the frozen implementation commit; for the 10k-tracked-file fixture, inspect < 2 s and snapshot < 2 s (SC-007), and record SHA, OS/architecture, fixture size, measured durations, limits, and result. A workspace run showing the test ignored is insufficient; quiescence→snapshot remains ≤ 5 s (SC-003) | `apps/daemon/tests/perf.rs` |
| Offline | On the same frozen implementation SHA used for cross-platform evidence, build dependencies and required binaries/tests before isolation, then run CLI, daemon, repository inspection, sessions, and quickstart behavior inside a Linux network namespace or no-network container while preserving and proving local IPC/filesystem access; prove external networking is unavailable, fail rather than silently skip if namespaces are unavailable, and record the completed workflow/job plus Ubuntu and mechanism details | dedicated Linux CI validation |
| Cross-platform | Scenarios 1–6 execute on Windows and from a clean checkout on macOS against the same frozen implementation SHA; each records OS/version/architecture, Rust and Cargo versions, exact commands, scenario results, event counts, workspace tests, formatting, and Clippy. The implementation commit and possibly newer evidence commit are recorded separately; a configured matrix or stale Windows run is insufficient | completed CI/local runs + `evidence/quickstart-run.md` |

## CI and Evidence Acceptance

- The standard cross-platform quality command is `cargo test --workspace --all-targets`.
  Retain `cargo fmt --check` and
  `cargo clippy --workspace --all-targets -- -D warnings`. Nextest is not required by
  this feature's acceptance gate.
- A dedicated SC-005 acceptance job sets the configurable crash harness to exactly 100
  iterations on the frozen implementation SHA and records the workflow/job, configured
  and completed counts of 100, zero committed-event loss, zero invalid session outcomes,
  and the final result. Smaller runs and dirty-working-tree local runs are diagnostic only.
- A dedicated Linux offline job fetches dependencies and builds before isolation, then
  runs the required behavior on that same SHA under an OS-level no-external-network
  mechanism, proves external networking is unavailable and local filesystem/IPC work,
  and fails explicitly or uses a no-network container fallback when namespaces are
  unavailable. `cargo --offline` alone is not acceptable evidence.
- A configured matrix is not cross-platform evidence until Windows and clean-checkout
  macOS runs complete against the same exact implementation SHA and their real output,
  environment metadata, and separate evidence-document commit are recorded.

## Feature 001 Completion Gate

Do not begin the next feature until all of the following are true:

1. All 76 authoritative tasks are complete (76/76), and Feature 002 remains untouched.
2. Watcher readiness acknowledgement and post-install Git reconciliation are implemented.
3. T061's typed `WATCHER_START_FAILED` schema, IPC/CLI goldens for `install` and
   `reconcile`, exit-code-1 mappings, compatibility tripwire, and non-leakage tests pass.
4. T065's barrier-controlled deletion of an initially tracked file during watcher
   installation passes with the authoritative snapshot/change event and no duplicate.
5. `us2_agent_sim`, every `us3_tracking` test, `us3_events`, and
   `cargo test --workspace --all-targets` pass.
6. `cargo fmt --check` and
   `cargo clippy --workspace --all-targets -- -D warnings` pass.
7. `cargo test -p cairn-daemon --test perf -- --ignored` explicitly passes on the frozen
   implementation commit, with OS/architecture, 10,000-file fixture size, measured
   inspect/snapshot durations, SC-007 limits, and result recorded.
8. Windows and clean-checkout macOS Scenarios 1–6 pass on the same frozen implementation
   SHA with the required environment, command, event-count, and quality evidence.
9. Linux OS-level network-isolated validation passes on that same SHA with the completed
   workflow/job and network, filesystem, and local-IPC proofs.
10. The dedicated exactly-100-kill CI job passes on that same SHA with 100 configured and
    completed kills, zero committed-event loss, and zero invalid session outcomes.
11. Evidence records real completed executions and distinguishes the implementation
    commit tested from the evidence-document commit.

## Complexity Tracking

No constitution violations. One negative deviation recorded: Axum/Tower omitted because
this feature exposes no HTTP surface; adding them would violate Principle IX
(speculative infrastructure). They enter when the central-server feature does.
