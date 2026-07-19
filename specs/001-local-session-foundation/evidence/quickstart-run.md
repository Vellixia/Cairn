# Quickstart Evidence Run

> **Completion status (2026-07-19): REMEDIATED LOCALLY / NOT CONVERGED.** Fresh macOS
> watcher, workspace, quickstart, and exactly-100-kill results are recorded below. Linux
> OS-level network-isolated validation has been configured but has not run, and the tested
> implementation is still an uncommitted working tree based on the recorded HEAD SHA.
> The typed watcher-error contract and authoritative installation-window deletion
> requirements also postdate those results. Therefore T059, T061, T065, T069, T071–T076
> remain open. Checked task boxes never replace missing acceptance evidence.

**Feature**: 001-local-session-foundation
**Date**: 2026-07-16
**OS**: Windows 11 Pro 10.0.26200 (real `cairn.exe` / `cairnd.exe` debug binaries, isolated data dir + pipe)
**Second OS**: macOS execution recorded below; Linux isolation remains outstanding.

## macOS remediation run — 2026-07-19

### Environment

```text
OS: macOS 26.5.2 (build 25F84)
Architecture: arm64 (Darwin RELEASE_ARM64_T8103)
rustc: rustc 1.90.0-nightly (abf50ae2e 2025-09-16) (1.90.0.0)
cargo: cargo 1.90.0-nightly (840b83a10 2025-07-30) (1.90.0.0)
HEAD SHA: d3491b0ebc7d8574b7af8cb50ff0b7f8a4e705f0
Working tree: dirty; the watcher-readiness implementation is not represented by that SHA
Quickstart root: /private/tmp/cairn-quickstart-019f79bb/demo
Data/socket: /private/tmp/cairn-quickstart-019f79bb/data and cairn.sock
```

The dirty-tree disclosure is material: this is real local execution evidence, but it is
not a reproducible tested-commit attestation. No commit was created because the user did
not authorize committing the existing large working tree.

### Exact command forms

The real debug binaries were built with `cargo build --workspace`. Every CLI invocation
used the same isolated environment:

```text
CAIRN_DATA_DIR=/private/tmp/cairn-quickstart-019f79bb/data
CAIRN_SOCKET_PATH=/private/tmp/cairn-quickstart-019f79bb/cairn.sock
CAIRN_PIPE_NAME=cairn-quickstart-019f79bb
target/debug/cairnd
target/debug/cairn <quickstart command> [--json]
sqlite3 /private/tmp/cairn-quickstart-019f79bb/data/cairn.db <read-only count/dump query>
```

Scenario commands were `cairn init`, repeated `cairn init --json`, `cairn status --json`,
`cairn session start/show/stop`, `kill -9 <PID resolved from the isolated socket>`, daemon
restart, bad and valid `cairn session reattach`, privacy DB dump scanning, detached-HEAD
status, and an intentionally corrupted temporary DB header followed by `cairn status`.
Resume tokens were kept in mode-0600 temporary files and removed from displayed JSON.

### Scenario results

1. **Register + inspect — PASS.** Fresh init returned repository
   `019f79e4-61c4-76a2-b124-039b512d42f4`; repeat init returned `created=false` and
   `identity_outcome=existing`. Status reported branch `main`, staged added `b.txt`,
   unstaged modified `a.txt`, and no untracked files. Init from the isolated non-Git
   directory returned exit 3 with `NOT_A_REPOSITORY`.
2. **Snapshot-bound session — PASS.** First start returned `created`, session
   `019f79e5-0644-7992-be4b-5ec006464032`, state `active`, with identical start/current
   fingerprint `fa41ca78…f5f09`. Repeated start returned `existing`, the same session ID,
   and a null token. Stop returned `stopped`.
3. **Live tracking — PASS.** An edit immediately after successful start changed the
   current fingerprint from `fa41ca78…f5f09` to `e6da9dab…5a625`. Branch switch reported
   `feature`; event counts at that point were `repository.state_changed=2` and
   `branch.changed=1`.
4. **Forced-kill recovery — PASS for the manual scenario.** Events before SIGKILL: 11;
   events immediately after restart: 11 (zero loss). The session was `recovering`; bad
   token returned exit 1 `LEASE_MISMATCH` and left it recovering; valid reattach returned
   `active`; explicit stop returned `stopped`. Events after recovery/stop: 14, including
   one `session.reattach_rejected`, one `session.recovered`, and two total
   `session.stopped` events across Scenarios 2 and 4.
5. **Privacy audit — PASS.** `.env` was reported only in ignored metadata with
   `total_count=1`; scanning the SQLite dump for `abc123` returned `0` matches. The
   automated privacy suite also passed in the full workspace run.
6. **Offline/failure spot checks — PARTIAL.** Detached HEAD returned `detached=true`,
   `branch=null`; the repository had no remote. After backing up and corrupting only the
   isolated temporary DB header, daemon recovery logged corrupted state and CLI status
   returned exit 6 with `STATE_CORRUPTED` and no fabricated success data. The required
   Linux no-external-network namespace execution has not run; the CI job uses Linux
   `unshare(1) --net` after dependency fetch/build, but configuration is not evidence.

### Fresh automated evidence

```text
cargo test -p cairn-daemon --test us2_agent_sim --test us3_tracking --test us3_events
  us2_agent_sim: 1 passed
  us3_tracking: 10 passed
  us3_events: 2 passed

CAIRN_CRASH_ITERS=100 cargo test -p cairn-daemon --test us4_crash_restart -- --nocapture
  SC-005 acceptance: completed_forced_kills=100 committed_event_loss=0 invalid_session_outcomes=0
  result: 1 passed, 0 failed

cargo test --workspace --all-targets
  result: 96 passed, 0 failed, 1 ignored (explicit nightly perf test)

cargo fmt --check
  clean

cargo clippy --workspace --all-targets -- -D warnings
  clean, 0 warnings
```

The dedicated Linux job fetches dependencies and builds all targets before entering a
network namespace, verifies external HTTPS is unreachable, and then runs CLI/daemon,
repository inspection, session, and live-tracking suites with filesystem and Unix IPC
available. Its completed output is still required before T069/T072 can close.

## Scenario results (verbatim output, trimmed)

### S1 — Register + inspect (US1) ✅

```text
=== S1: cairn init (fresh) ===
Registered repository at C:/Users/andre/AppData/Local/Temp/tmp.TGSYtI646f/demo (id 019f6a99-40cf-73f3-83e7-9cba3ff0a1f0, identity created)

=== S1: cairn init (idempotent) ===
Already registered (id 019f6a99-40cf-73f3-83e7-9cba3ff0a1f0, identity existing)

=== S1: identity marker exists ===
repository-id marker: present

=== S1: dirty state inspection ===
branch:    main
staged:    1
  added b.txt
unstaged:    1
  modified a.txt

=== S1: non-git rejection ===
error [NOT_A_REPOSITORY]: not a git repository: ...
exit=3
```

### S2 — Session bound to exact start snapshot (US2) ✅

```text
outcome: created
outcome: existing | token is null: True     ← idempotent start, no new token (FR-034)
state: active
start==current fp: True                     ← unchanged repo anchors identically (SC-002)
```

### S3 — Live tracking (US3) ⚠️ historical run; current acceptance invalidated

```text
fingerprint changed after edit: yes         ← within 4 s of quiescence (SC-003)
branch now: feature                         ← branch switch tracked
```

### S4 — Kill daemon, recover (US4) ✅

```text
state after restart: recovering
error [LEASE_MISMATCH]: resume token or agent instance mismatch   (exit=1)
state after bad reattach: recovering        ← reject-only, session untouched (I3)
state after good reattach: active
events before kill: 9, after recovery: 11 (no loss: yes)          ← SC-005
```

### S5 — Privacy audit (SC-006) ✅

```text
ignored:   1 files
secret bytes in DB: 0
raw resume token in DB: 0
```

### S6 — Detached HEAD + event history ✅

```text
branch:    (detached)

  1  repository.registered
  2  worktree.registered
  3  snapshot.created
  4  session.started
  5  snapshot.created
  6  repository.state_changed
  7  snapshot.created
  8  repository.state_changed
  9  branch.changed
 10  session.reattach_rejected
 11  session.recovered
 12  snapshot.created
 13  repository.state_changed
 14  session.stopped
```

## Automated evidence (same date, same machine)

The results in this section describe the 2026-07-16 Windows run only. They are superseded
for completion decisions by the 2026-07-19 failing watcher suites and must not be treated
as current feature-level acceptance.

- `cargo test --workspace`: **90 passed, 0 failed, 1 ignored** (perf suite).
- Perf suite (`--ignored`): inspect **206 ms**, snapshot **382 ms** at 10,000
  tracked files — SC-007 bound is 2 s each.
- Crash harness: 8 randomized `TerminateProcess` kills (historical diagnostic only), zero
  committed-event loss, all pre-kill sessions recovering. This does **not** satisfy SC-005;
  acceptance requires exactly 100 completed forced kills.
- `cargo clippy --workspace --all-targets`: 0 warnings. `cargo fmt --check`: clean.

## Traceability

| Criteria | Evidence |
|---|---|
| SC-001 offline local answers | S2/S4 outputs; no network APIs in workspace |
| SC-002 determinism | S2 (`start==current fp: True`) + 100× loops in tests |
| SC-003 ≤5 s tracking | S3 + us3_tracking bound asserts |
| SC-004 idempotent init / zero partial writes | S1 |
| SC-005 zero event loss + recovery | S4 + us4_crash_restart |
| SC-006 zero secrets persisted | S5 + privacy_audit test |
| SC-007 <2 s @10k files | Historical Windows perf suite (206/382 ms); final exact-commit execution still required |
| SC-008 machine-output stability | json_stability test (8 commands × success/failure × 3 rounds) |

## Outstanding evidence required for convergence

### Watcher readiness and US3

- [ ] T061's typed `WATCHER_START_FAILED` schema and contract tests pass, including IPC
  request/response and CLI JSON-envelope goldens for `install` and `reconcile`, both CLI
  exit-code-1 assertions, schema-breaking-change tripwire coverage, and non-leakage of raw
  errors, paths, repository contents, environment values, or tokens.
- [ ] `us2_agent_sim` passes on the frozen implementation commit after watcher readiness
  acknowledgement and post-install reconciliation are implemented.
- [ ] Every `us3_tracking` test passes on the frozen implementation commit, including an
  initially tracked file deleted while watcher installation is barrier-paused; the
  returned/current snapshot and change event reflect the deletion without a duplicate.
  Coverage also includes immediate post-return edit, create/modify/rename during install,
  coalesced bursts, dropped notification reconciliation, installation failure, and daemon
  restart watcher reinstallation.
- [ ] `us3_events` passes on the frozen implementation commit; `us3_tracking`
  demonstrates no duplicate repository-change event when notification and reconciliation
  observe the same change.
- [ ] The complete `cargo test --workspace --all-targets` run passes on the frozen
  implementation commit.

### SC-005 exactly-100-kill acceptance

- [ ] Record the dedicated CI workflow run/job and exact frozen implementation SHA.
- [ ] Record configured iterations `100`, completed forced kills `100`, committed-event
  loss `0`, invalid session outcomes `0`, and final job result.
- [ ] Record that every session active at termination entered recovery and was recovered
  or explicitly interrupted according to SC-005.

An 8- or 20-iteration result and the local dirty-tree 100-kill result above are diagnostic
only and cannot check this final section.

### Linux network-isolated validation

- [ ] Record the completed workflow run/job, same frozen implementation SHA, Ubuntu
  version, and architecture.
- [ ] Name the OS-level network namespace or container isolation mechanism.
- [ ] Record dependency fetch and binary/test build before isolation.
- [ ] Inside isolation, record the CLI, daemon, repository registration/inspection,
  session, live-change, and quickstart commands and results while local IPC/filesystem
  access remain available.
- [ ] Demonstrate external networking is unavailable, filesystem access works, and local
  IPC works. If namespaces are unavailable, fail explicitly or use a container launched
  with networking disabled; do not silently skip. `cargo --offline` alone is not proof.

### Windows and macOS same-commit Scenarios 1–6 (T059/T071/T073)

- [ ] Freeze one exact Feature 001 implementation SHA and record the separate evidence
  commit that documents the results.
- [ ] From a clean checkout of that SHA on macOS, record OS version/architecture, Rust and
  Cargo versions, exact commands, Scenarios 1–6 results, required event counts, workspace
  tests, formatting, and Clippy.
- [ ] On Windows, check out and test that same exact SHA and record the equivalent
  environment, commands, scenario, event-count, and quality results. A completed GitHub
  Actions job is acceptable when it explicitly tests that SHA.

The historical Windows run predates watcher remediation and the macOS run above used a
dirty tree, so neither currently satisfies this section. A configured matrix is not
evidence.

### SC-007 explicit performance execution

- [ ] Run `cargo test -p cairn-daemon --test perf -- --ignored` on the same frozen
  implementation SHA.
- [ ] Record exact implementation SHA, OS/architecture, tracked-file fixture size
  (10,000), measured inspect duration, measured snapshot duration, SC-007 limits
  (each under 2 seconds), and pass/fail result.

A workspace test result that reports the performance test as ignored does not satisfy
SC-007.

### Final convergence

- [ ] All 76 authoritative tasks are complete (76/76).
- [ ] Windows, clean-checkout macOS, Linux network isolation, exactly-100-kill CI, and
  SC-007 evidence all reference the same frozen implementation SHA.
- [ ] Real completed execution evidence and its evidence-document commit are recorded.
- [ ] Feature 002 remains untouched.
