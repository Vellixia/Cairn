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
| 2 | 2026-07-24T18:15:25Z–18:19:52Z | Rerun after `feature002_binding_races.rs` gained direct `event_aggregate_heads` assertions (T134 audit finding) | 15/15 commands exit 0 |
| 3 | 2026-07-24T19:05:06Z–19:12:24Z | Rerun on the `stable` toolchain after CI on invalidated frozen SHA `9021ba9` exposed two defects the earlier gates could not see | 12/12 commands exit 0 |
| 4 | 2026-07-24T19:30:36Z–19:34:17Z | Rerun after CI on invalidated frozen SHA `964eb3a` exposed a container/glibc defect in the isolation harness | 12/12 commands exit 0 |
| **5 (authoritative)** | **2026-07-25T04:45:35Z–04:48:32Z** | **Rerun after CI on invalidated frozen SHA `b194614` failed to pull the non-existent `rust:1-noble` image; corrected to `rust:1-trixie`** | **12/12 commands exit 0** |

Run 5 is the authoritative record. All figures below are from run 5.

**Runs 1 and 2 are superseded and must not be cited as evidence.** Both executed
under the repository's default `esp` toolchain (nightly 1.90.0), which is *not* what
CI uses. GitHub Actions pins `dtolnay/rust-toolchain@stable` (1.97.1 at run time),
and under stable `clippy::assertions_on_constants` rejects
`assert!(!cfg!(debug_assertions), ...)` in `feature002_perf.rs`. Runs 1 and 2 were
structurally incapable of catching that. Run 3 executes every command through
`rustup run stable` so the local gate and CI evaluate the same lint set.

Two defects were found through CI on invalidated freeze `9021ba9` and fixed before run 3:

| Defect | Detected by | Fix |
|---|---|---|
| `assert!` on a compile-time constant in `feature002_perf.rs` | CI clippy on stable, all platforms | runtime `if cfg!(debug_assertions) { panic!(...) }`; clippy's suggested `const { assert!(..) }` would instead break the debug `--all-targets` build |
| `scripts/ci/feature002-network-isolated.sh` recorded as mode `100644` | Linux isolated job: `Permission denied` | `git update-index --chmod=+x` to `100755`; local `core.fileMode=false` had hidden it |

A third, CI-only robustness defect was fixed at the same time: four jobs created their
log directory only after the gate it was meant to capture, so an `if: always()` upload
failed and masked the real error. All nine upload jobs now create their directory in
an earlier step.

Exact-SHA CI on freeze `964eb3a` then passed **12 of 13 jobs** and exposed one further
defect, fixed before run 4:

| Defect | Detected by | Fix |
|---|---|---|
| Prebuilt acceptance binaries are built on `ubuntu-latest` (glibc 2.39) but the isolation fallback ran them in `rust:1-bookworm` (Debian 12, glibc 2.36), so every binary failed to load with `GLIBC_2.39 not found` | Linux isolated Feature 002 job | default container image changed to `rust:1-noble` (Ubuntu 24.04, glibc 2.39); the harness now also preflights every prebuilt binary with `--list` inside isolation and fails with a precise `prebuilt_binary_incompatible_with_isolation_environment` error instead of a confusing test failure |
| Container-created files were root-owned, so the cleanup trap failed and changed the harness exit status | same job | `docker run --user "$(id -u):$(id -g)"` plus a cleanup trap that can never alter the exit status |

That run did prove, inside genuine `docker --network none` isolation:
`external_network=unreachable`, `local_filesystem=available`, and
`local_git=available`. It failed only when loading the prebuilt binaries.

Exact-SHA CI on freeze `b194614` then failed one job before starting, because the
official Rust images are Debian-based and there is no `rust:1-noble` tag
(`manifest unknown`). Fixed before run 5:

| Defect | Detected by | Fix |
|---|---|---|
| `docker pull rust:1-noble` — the tag does not exist; Rust images track Debian, not Ubuntu | Linux isolated Feature 002 job, preload step | image set to `rust:1-trixie` (Debian 13, glibc 2.41 ≥ the runner's 2.39, ships git). Tag existence was verified against the Docker Hub registry API before committing. |

## Execution environment

| Field | Value |
|---|---|
| Execution date (run 5) | 2026-07-25 (Asia/Jakarta) / 2026-07-25T04:45:35Z–04:48:32Z |
| Checkout commit at execution | `fb204b07d6c8ee4e1f82fce262feb311086df551` |
| Branch | `feature/002-project-task-binding` |
| Working tree | **Dirty** — Feature 002 implementation and specification changes are uncommitted; the frozen implementation commit (T116) does not exist yet |
| OS | macOS 26.5.2 (Darwin 25.5.0) |
| Architecture | arm64 (aarch64) |
| Rust | `rustc 1.92.0 (ded5c06cf 2025-12-08)` — stable, invoked via `rustup run stable` to match CI |
| Cargo | `cargo 1.92.0 (344c4567c 2025-10-21)` |
| SQLite (linked, reported by the perf test) | 3.46.0 |
| SQLite (host CLI) | 3.51.0 |
| Git | 2.50.1 (Apple Git-155) |
| External network required | none |

## Executed commands

Durations are wall-clock from the recorded UTC start/end of each gate.

Every command below was invoked through `rustup run stable` so the local gate and CI
evaluate the same toolchain.

| # | Exact command | Exit | Duration |
|---|---|---:|---:|
| G1 | `rustup run stable cargo fmt --check` | 0 | 1 s |
| G2 | `rustup run stable cargo clippy --workspace --all-targets -- -D warnings` | 0 | 24 s |
| G3 | `rustup run stable cargo test --workspace --all-targets` | 0 | 94 s |
| G4a | `rustup run stable cargo test -p cairn-events --test feature002_replay --test feature002_replay_invalid -- --nocapture` | 0 | 68 s (G4 total) |
| G4b | `rustup run stable cargo test -p cairn-daemon --test feature002_atomicity --test feature002_migration_acceptance --test feature002_quickstart --test feature002_privacy --test feature002_retry_acceptance --test feature002_binding_races -- --nocapture` | 0 | — |
| G4c | `rustup run stable cargo test -p cairn-daemon --test feature002_projects --test feature002_tasks --test feature002_binding_restart --test feature002_bound_start_invariants --test feature002_bound_recovery --test feature002_ipc` | 0 | — |
| G4d | `rustup run stable cargo test -p cairn-cli --test feature002_privacy_cli --test feature002_ipc_only --test feature002_json_stability --test feature002_ambiguous_names` | 0 | — |
| G4e | `rustup run stable cargo test -p cairn-protocol --tests` | 0 | — |
| G5 | `rustup run stable cargo test -p cairn-daemon --test us2_agent_sim --test us3_tracking --test us3_events` | 0 | 11 s |
| G6 | `CAIRN_CRASH_ITERS=100 CAIRN_CRASH_EXPECTED_ITERS=100 rustup run stable cargo test -p cairn-daemon --test us4_crash_restart -- --nocapture` | 0 | 84 s |
| G7 | `rustup run stable cargo test -p cairn-daemon --test perf -- --ignored --nocapture` | 0 | 13 s |
| G8 | `rustup run stable cargo test --release -p cairn-daemon --test feature002_perf -- --ignored --nocapture` | 0 | 46 s (incl. release build) |

Total gate wall time: 2 minutes 57 seconds. Aggregate result: **12 recorded exit codes,
all zero; 135 `test result: ok` lines; zero failures; zero panics.**

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
from `testing-evidence.md` is the plain `--ignored` form, which is what run 3
executed:

| Profile | Command | tracked_files | inspect_ms | snapshot_ms | Limits | Result |
|---|---|---:|---:|---:|---|---|
| debug (authoritative) | `rustup run stable cargo test -p cairn-daemon --test perf -- --ignored --nocapture` | 10000 | 80 | 188 | 2000 / 2000 | PASS |

The frozen-SHA execution required by T136 has not been performed.

## Feature 002 SC-010 performance (G9)

Run in the **release** profile only; the test asserts `!cfg!(debug_assertions)` and a
debug `-- --ignored` invocation fails by design.

```text
feature002_perf_environment os=macos arch=aarch64 rustc=rustc 1.92.0 (ded5c06cf 2025-12-08) cargo=cargo 1.92.0 (344c4567c 2025-10-21) sqlite=3.46.0 projects=100 tasks=1000 revisions_per_task=5 profile=release warmup=read-path-once
```

Fixture cardinality: 100 projects, 1,000 tasks, 5 revisions per task (5,000
revisions), 20 repository associations, 20 bound sessions.

| Operation | Samples | p95 (ms) | Threshold (ms) | Result |
|---|---:|---:|---:|---|
| project_create | 99 | 1.011 | 2000 | PASS |
| project_list | 100 | 0.435 | 2000 | PASS |
| project_show | 100 | 0.174 | 2000 | PASS |
| project_update | 100 | 0.696 | 2000 | PASS |
| repository_associate | 19 | 0.808 | 2000 | PASS |
| task_create | 990 | 1.485 | 2000 | PASS |
| task_list | 100 | 0.711 | 2000 | PASS |
| task_show | 100 | 0.097 | 2000 | PASS |
| task_revise | 4000 | 0.877 | 2000 | PASS |
| session_bind | 19 | 0.721 | 2000 | PASS |

All ten operations remain far inside the two-second bound; the slowest p95 in run 3
is `task_create` at 1.485 ms, over 1,300x under the threshold. Sample
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
run 5, on the stable toolchain CI uses. The only remaining prerequisite for the T116 implementation freeze is the
freeze commit itself; no unresolved implementation or test task blocks it.

At the close of run 5, **zero** `.rs`, `.toml`, `.sql`, or `.lock` files had been
modified after the gate finished, so this record is not stale with respect to the
tree it certifies.
