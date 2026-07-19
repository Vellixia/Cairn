# Quickstart & Validation Guide: Local Session Foundation

**Feature**: 001-local-session-foundation | **Date**: 2026-07-16
Contracts: [ipc-contract.md](contracts/ipc-contract.md), [cli-json-contract.md](contracts/cli-json-contract.md) · Data model: [data-model.md](data-model.md)

## Prerequisites

- Rust stable toolchain (`rustup show`), `git` ≥ 2.40 on PATH
- Windows, Linux, or macOS; ordinary user privileges
- Build: `cargo build --workspace` (binaries: `cairn`, `cairnd`)
- Tests: `cargo test --workspace --all-targets` (fixtures auto-built by `fixtures/repositories`)

## Scenario 1 — Register + inspect exact state (US1)

```console
$ mkdir demo && cd demo && git init -b main && echo hi > a.txt && git add a.txt && git commit -m one
$ cairn init
Registered repository demo (id 019… ) at D:/tmp/demo
$ cairn init                        # idempotent
Already registered (id 019…)       # exit 0, created=false in --json
$ echo change >> a.txt && echo new > b.txt && git add b.txt
$ cairn status --json | jq '.data | {branch, head_commit, staged, unstaged, untracked, ignored_summary.total_count}'
```

**Expected**: branch `main`; `staged` lists `b.txt` (added); `unstaged` lists `a.txt`
(modified); `untracked` empty; re-running `cairn init` created no second registration
(FR-003); `cairn init` in a non-Git dir exits 3 with `NOT_A_REPOSITORY` and writes
nothing (FR-004). Verify identity file exists: `.git/cairn/repository-id`.

## Scenario 2 — Session bound to exact start snapshot (US2)

```console
$ export CAIRN_AGENT_INSTANCE=$(uuidgen)
$ cairn session start --agent demo-agent --json | jq '.data.outcome'   # "created"
$ cairn session start --agent demo-agent --json | jq '.data.outcome'   # "existing" (idempotent, FR-034)
$ cairn session show --json | jq '.data.session | {state, start_snapshot: .start_snapshot.snapshot_fp, current: .current_snapshot.snapshot_fp}'
$ cairn session stop
```

**Expected**: start twice → same `session_id`, outcome `existing`, no new token.
Unchanged repo ⇒ `start_snapshot.snapshot_fp == current_snapshot.snapshot_fp`
(determinism SC-002). After stop: state `stopped`, `session.stopped` event in
`v1.events.list`.

A successful first start also proves the watcher-readiness barrier: the OS watcher is
installed, its event path is ready, and post-install Git reconciliation has completed
before the command returns. An immediate edit after return must therefore be observed.
`WATCHER_START_FAILED` with schema-constrained
`data={kind:"watcher_start_failure",stage:"install|reconcile"}` is the only valid result
when that readiness sequence fails; the created session must be `interrupted`, not
active, the CLI must exit 1, and no raw internal details, paths, or tokens may leak.

## Scenario 3 — Live tracking during a session (US3)

```console
$ cairn session start --agent demo-agent
$ echo more >> a.txt                       # wait ≤5 s after quiescence (SC-003)
$ cairn session show --json | jq '.data.session.current_snapshot.snapshot_fp'   # changed
$ git checkout -b feature                   # branch switch
$ cairn session show --json | jq '.data.session.current_snapshot.branch'        # "feature"
```

**Expected**: fingerprint changes after edit; `branch.changed` +
`repository.state_changed` events appended; rapid bursts (e.g., `for i in $(seq 100); do echo $i >> a.txt; done`)
coalesce to a final snapshot matching `git status` reality (FR-023). A `touch` with no
content change produces **no** new snapshot (FR-022, hint reconciliation).

Automated acceptance additionally pauses watcher installation with an explicit barrier
and performs create/modify/rename operations in the installation window. Its deletion
case starts with a committed or otherwise initially tracked file in the authoritative
snapshot, deletes that file while installation is paused, releases the barrier, and
verifies the returned/current snapshot, expected `repository.state_changed` event, and
absence of a duplicate logical change event. It also drops a notification deliberately
and proves explicit reconciliation converges without duplication. Timing sleeps are not
the primary correctness mechanism for these checks.

## Scenario 4 — Restart durability + recovery (US4)

```console
$ cairn session start --agent demo-agent --json > start.json
$ export CAIRN_RESUME_TOKEN=$(jq -r '.data.resume_token' start.json)
$ kill -9 $(pgrep cairnd)                  # Windows: Stop-Process -Name cairnd -Force
$ cairn daemon status                      # auto-respawns daemon
$ cairn session show --json | jq '.data.session.state'          # "recovering"
$ CAIRN_RESUME_TOKEN=wrong cairn session reattach --session $SID   # LEASE_MISMATCH, exit 1
$ cairn session show --json | jq '.data.session.state'          # still "recovering" (reject-only)
$ cairn session reattach --session $SID    # valid token from env → "active", session.recovered
# OR let the grace deadline (recovering_since + grace) expire → "interrupted"
```

**Expected**: zero committed events lost across the kill (SC-005 — compare
`v1.events.list` before/after); session never silently disappears; wrong/absent token ⇒
`LEASE_MISMATCH` rejection + `session.reattach_rejected` audit event, session stays
recovering; killing the daemon again does NOT extend the grace deadline
(`recovering_since` preserved); tokens never appear in human output or logs.

## Scenario 5 — Privacy audit (SC-006)

```console
$ echo "SECRET_TOKEN=abc123" > .env && echo ".env" >> .gitignore
$ cairn session start --agent demo-agent && cairn status
$ sqlite3 "$CAIRN_DB" ".dump" | grep -c "abc123"     # → 0
$ cairn status --json | jq '.data.ignored_summary'   # .env counted, content absent
```

**Expected**: zero secret bytes anywhere in the DB dump; ignored files appear only as
summary metadata (FR-026–028, FR-035).

## Scenario 6 — Offline + failure handling (spot checks)

- Disable external network access with a Linux OS-level network namespace or container
  after dependencies and required binaries/tests have been fetched and built. Preserve
  local IPC and filesystem access, then run the relevant CLI, daemon, registration,
  inspection, session, and quickstart behavior. Record the isolation mechanism and prove
  the scenario fails if it attempts external network access. `cargo --offline` alone is
  not acceptance evidence (FR-024, SC-001).
- `git checkout --detach` → `cairn status` shows `detached: true`, `branch: null`.
- Repo with no remote → `default_remote: null`, no errors.
- `git rebase` mid-flight → `in_progress: "rebase"`; snapshots reflect post-rebase state.
- Corrupt the DB (`truncate` it) → commands exit 6 with `STATE_CORRUPTED`, no fabricated
  output (FR-033).

## Success criteria traceability

| Check | Criteria |
|---|---|
| Scenario 1 | SC-004, FR-001…FR-007, FR-035 |
| Scenario 2 | SC-002, FR-013…FR-017, FR-034 |
| Scenario 3 | SC-003, FR-021…FR-023 |
| Scenario 4 | SC-005, FR-018…FR-020 |
| Scenario 5 | SC-006, FR-026…FR-028 |
| Scenario 6 | SC-001, FR-024, FR-032, FR-033 |
| `cargo test -p cairn-daemon --test perf -- --ignored` on the frozen implementation commit | SC-007 |
| Golden JSON parse loop | SC-008 |

## Required completion evidence

Feature 001 evidence must include:

- One frozen Feature 001 implementation commit tested on both Windows and macOS. The
  macOS run starts from a clean checkout; Windows may use a completed CI job that checks
  out that exact SHA. For each OS record version/architecture, `rustc --version`,
  `cargo --version`, implementation SHA, exact commands, Scenarios 1–6 results, required
  event counts, and outcomes from `cargo test --workspace --all-targets`,
  `cargo fmt --check`, and
  `cargo clippy --workspace --all-targets -- -D warnings`. Record the possibly newer
  evidence-document commit separately. Stale Windows output and configured-only matrices
  are not evidence.
- A dedicated CI SC-005 acceptance execution on that same implementation SHA, configured
  to and completing exactly 100 forced daemon kills, with workflow/job reference,
  committed-event loss `0`, invalid session outcomes `0`, and final result. Smaller runs
  and local dirty-tree runs do not satisfy SC-005.
- A completed Linux OS-level network-isolated CI execution on that same implementation
  SHA. Record workflow/job, Ubuntu version/architecture, dependency fetch/build before
  isolation, exact namespace or no-network-container mechanism, proof external networking
  is unavailable, proof local filesystem and IPC work, and all commands/scenario results.
  Namespace unavailability must fail explicitly or use a genuinely isolated fallback;
  `cargo --offline` and configured-only jobs are not evidence.
- Explicit `cargo test -p cairn-daemon --test perf -- --ignored` execution on that same
  implementation SHA. Record OS/architecture, 10,000 tracked files, measured inspect and
  snapshot durations, the under-2-second SC-007 limits, and pass/fail. A workspace run
  that reports this test ignored is not evidence.
- Passing T061 typed watcher-contract/golden coverage, T065 authoritative
  installation-window deletion coverage, `us2_agent_sim`, every `us3_tracking` test, and
  `us3_events` after remediation.
- Final evidence that all 76 authoritative tasks are complete (76/76) and Feature 002 is
  untouched.
