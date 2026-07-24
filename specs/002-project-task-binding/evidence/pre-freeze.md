# Feature 002 pre-freeze local readiness gate (T115)

**This document is readiness evidence, not exact-SHA platform evidence and not
inherited Feature 001 acceptance evidence.** It records that the local quality,
Feature 001 focused, and Feature 002 acceptance gates all pass immediately before
the implementation freeze. The exact-SHA macOS/Windows/Linux/isolated/performance
executions remain T117–T121; the frozen-SHA Feature 001 100-kill and SC-007 runs
remain T135–T136. Nothing here satisfies those tasks.

## Revision history

| Run | UTC window | Trigger | Result |
|---|---|---|---|
| 1 | 2026-07-24T17:51:54Z–17:57:55Z | Initial T115 gate | 15/15 commands exit 0 |
| **2 (authoritative)** | **2026-07-24T18:15:25Z–18:19:52Z** | **Rerun after `feature002_binding_races.rs` gained direct `event_aggregate_heads` assertions (T134 audit finding)** | **15/15 commands exit 0** |

Run 2 is the authoritative record: any Rust change after a gate invalidates that
gate, so the full gate was re-executed rather than patched. All figures below are
from run 2 unless a run-1 comparison is stated.

## Execution environment

| Field | Value |
|---|---|
| Execution date (run 2) | 2026-07-25 (Asia/Jakarta) / 2026-07-24T18:15:25Z–18:19:52Z |
| Checkout commit at execution | `fb204b07d6c8ee4e1f82fce262feb311086df551` |
| Branch | `feature/002-project-task-binding` |
| Working tree | **Dirty** — Feature 002 implementation and specification changes are uncommitted; the frozen implementation commit (T116) does not exist yet |
| OS | macOS 26.5.2 (Darwin 25.5.0) |
| Architecture | arm64 (aarch64) |
| Rust | `rustc 1.90.0-nightly (abf50ae2e 2025-09-16) (1.90.0.0)` |
| Cargo | `cargo 1.90.0-nightly (840b83a10 2025-07-30) (1.90.0.0)` |
| SQLite (linked, reported by the perf test) | 3.46.0 |
| SQLite (host CLI) | 3.51.0 |
| Git | 2.50.1 (Apple Git-155) |
| External network required | none |

## Executed commands

Durations are wall-clock from the recorded UTC start/end of each gate.

| # | Exact command | Exit | Duration |
|---|---|---:|---:|
| G1 | `cargo fmt --check` | 0 | <1 s |
| G2 | `cargo clippy --workspace --all-targets -- -D warnings` | 0 | 2 s |
| G3 | `cargo test --workspace --all-targets` | 0 | 97 s |
| G4a | `cargo test -p cairn-events --test feature002_replay --test feature002_replay_invalid -- --nocapture` | 0 | 31 s (G4 total) |
| G4b | `cargo test -p cairn-daemon --test feature002_atomicity --test feature002_migration_acceptance --test feature002_quickstart --test feature002_privacy --test feature002_retry_acceptance --test feature002_binding_races -- --nocapture` | 0 | — |
| G4c | `cargo test -p cairn-daemon --test feature002_projects --test feature002_tasks --test feature002_binding_restart --test feature002_bound_start_invariants --test feature002_bound_recovery --test feature002_ipc` | 0 | — |
| G4d | `cargo test -p cairn-cli --test feature002_privacy_cli --test feature002_ipc_only --test feature002_json_stability --test feature002_ambiguous_names` | 0 | — |
| G4e | `cargo test -p cairn-protocol --tests` | 0 | — |
| G5a | `cargo test -p cairn-daemon --test us2_agent_sim` | 0 | 9 s (G5 total) |
| G5b | `cargo test -p cairn-daemon --test us3_tracking` | 0 | — |
| G5c | `cargo test -p cairn-daemon --test us3_events` | 0 | — |
| G6 | `CAIRN_CRASH_ITERS=100 CAIRN_CRASH_EXPECTED_ITERS=100 cargo test -p cairn-daemon --test us4_crash_restart -- --nocapture` | 0 | 83 s |
| G7 | `cargo test -p cairn-daemon --test perf -- --ignored --nocapture` | 0 | 11 s |
| G8 | `cargo test --release -p cairn-daemon --test perf -- --ignored --nocapture` | 0 | 13 s |
| G9 | `cargo test --release -p cairn-daemon --test feature002_perf -- --ignored --nocapture` | 0 | 4 s |

Total gate wall time: 4 minutes 27 seconds. Aggregate result: **15 recorded exit codes,
all zero; 136 `test result: ok` lines; zero failures; zero panics.**

G4b additionally covers **T134** (`feature002_binding_races`), whose two barrier
scenarios now assert `event_aggregate_heads` directly.

## Ignored and skipped tests

Two `#[ignore]` tests appear as `1 ignored` inside the G3 workspace run:
`cairn-daemon --test perf` and `cairn-daemon --test feature002_perf`. Both are
explicit acceptance suites that are ignored by design in the workspace sweep and are
executed separately and non-ignored in G7/G8 and G9. **No required test was skipped.**

Every other binary reported `0 ignored`.

## Feature 001 exactly-100 forced kills (G6)

```text
SC-005 acceptance: configured_iterations=100 completed_forced_kills=100 committed_event_loss=0 invalid_session_outcomes=0
```

| Counter | Value |
|---|---:|
| Configured iterations | 100 |
| Completed forced kills | 100 |
| Committed event loss | 0 |
| Invalid session outcomes | 0 |

This is a **pre-freeze readiness** run against a dirty tree. The frozen-SHA execution
required by T135 has not been performed.

## Feature 001 SC-007 performance (G7, G8)

The Feature 001 perf test carries no profile gate, so its authoritative invocation
from `testing-evidence.md` is the plain `--ignored` form. Both profiles were executed
and both pass:

| Profile | Command | tracked_files | inspect_ms | snapshot_ms | Limits | Result |
|---|---|---:|---:|---:|---|---|
| debug (authoritative) | `cargo test -p cairn-daemon --test perf -- --ignored --nocapture` | 10000 | 156 | 204 | 2000 / 2000 | PASS |
| release | `cargo test --release -p cairn-daemon --test perf -- --ignored --nocapture` | 10000 | 78 | 159 | 2000 / 2000 | PASS |

The frozen-SHA execution required by T136 has not been performed.

## Feature 002 SC-010 performance (G9)

Run in the **release** profile only; the test asserts `!cfg!(debug_assertions)` and a
debug `-- --ignored` invocation fails by design.

```text
feature002_perf_environment os=macos arch=aarch64 rustc=rustc 1.90.0-nightly (abf50ae2e 2025-09-16) (1.90.0.0) cargo=cargo 1.90.0-nightly (840b83a10 2025-07-30) (1.90.0.0) sqlite=3.46.0 projects=100 tasks=1000 revisions_per_task=5 profile=release warmup=read-path-once
```

Fixture cardinality: 100 projects, 1,000 tasks, 5 revisions per task (5,000
revisions), 20 repository associations, 20 bound sessions.

| Operation | Samples | p95 (ms) | Threshold (ms) | Result |
|---|---:|---:|---:|---|
| project_create | 99 | 1.005 | 2000 | PASS |
| project_list | 100 | 0.411 | 2000 | PASS |
| project_show | 100 | 0.131 | 2000 | PASS |
| project_update | 100 | 0.866 | 2000 | PASS |
| repository_associate | 19 | 2.345 | 2000 | PASS |
| task_create | 990 | 1.694 | 2000 | PASS |
| task_list | 100 | 0.577 | 2000 | PASS |
| task_show | 100 | 0.096 | 2000 | PASS |
| task_revise | 4000 | 1.186 | 2000 | PASS |
| session_bind | 19 | 1.091 | 2000 | PASS |

All ten operations remain far inside the two-second bound; the slowest p95 in run 2
is `repository_associate` at 2.345 ms, 853x under the threshold. Sample
counts are asserted for completeness; an empty or short measurement set fails the test.

## Feature 002 acceptance counters (G4)

```text
quickstart_counts={"project_events":2,"task_events":3,"session_started":2,"session_bound":2,"projects":1,"tasks":1,"revisions":2,"bindings":2}
feature002_retries={"association":100,"revision":100,"binding":100,"registry_records":3,"sequence_gaps":0,"conflicts":3}
```

## Network isolation accounting

Genuine OS-level network isolation was **not** exercised. This macOS host has no
`unshare` and no container runtime, so `scripts/ci/feature002-network-isolated.sh`
exits 69 with
`feature002_network_isolation_error=no_network_namespace_or_container_runtime`. That
proves the harness fails closed; it is **not** isolation evidence. Genuine isolated
execution remains T120, via the `linux-feature002-network-isolated` CI job.

## Readiness conclusion

Every gate required by T115 executed and passed against the current working tree in
run 2. The only remaining prerequisite for the T116 implementation freeze is the
freeze commit itself; no unresolved implementation or test task blocks it.

At the close of run 2, **zero** `.rs`, `.toml`, `.sql`, or `.lock` files had been
modified after the gate finished, so this record is not stale with respect to the
tree it certifies.
