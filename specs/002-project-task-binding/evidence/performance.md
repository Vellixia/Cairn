# T121 — Feature 002 performance (SC-010) exact-SHA evidence

A **completed, successful** job executed the SC-010 acceptance in the **release**
profile against the frozen implementation commit. The debug invocation is rejected
by the test on purpose (`!cfg!(debug_assertions)`), so only the release run is
authoritative.

## Frozen implementation commit

| Field | Value |
|---|---|
| Frozen commit SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` |
| Freeze metadata | [`evidence/implementation-freeze.json`](implementation-freeze.json) (generation 4) |

## Workflow references

| Field | Value |
|---|---|
| Workflow run | https://github.com/Vellixia/Cairn/actions/runs/30144710029 (attempt 1) |
| Job | `SC-010 Feature 002 performance acceptance` |
| Job URL | https://github.com/Vellixia/Cairn/actions/runs/30144710029/job/89644192956 |
| Job conclusion | **success** |
| Checked-out SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` (verified by the job's SHA-verify step) |
| Command | `cargo test --release -p cairn-daemon --test feature002_perf -- --ignored --nocapture` |
| Measurement-set completeness | the job asserts exactly 10 `feature002_perf operation=` lines and the presence of `threshold_ms=2000`; both passed |

## Environment (recorded by the test)

```text
feature002_perf_environment os=linux arch=x86_64 rustc=rustc 1.97.1 (8bab26f4f 2026-07-14) cargo=cargo 1.97.1 (c980f4866 2026-06-30) sqlite=3.46.0 projects=100 tasks=1000 revisions_per_task=5 profile=release warmup=read-path-once
```

| Field | Value |
|---|---|
| OS / arch | linux / x86_64 (Ubuntu 24.04 runner) |
| Toolchain | rustc/cargo 1.97.1 |
| SQLite (linked) | 3.46.0 |
| Fixture | 100 projects, 1,000 tasks, 5 revisions/task (5,000 revisions), 20 associations, 20 bound sessions |
| Profile | release |
| Warm-up | read paths warmed once before sampling |

## Measurements (p95, threshold 2000 ms)

| Operation | Samples | p95 (ms) | Result |
|---|---:|---:|---|
| project_create | 99 | 1.172 | PASS |
| project_list | 100 | 0.557 | PASS |
| project_show | 100 | 0.372 | PASS |
| project_update | 100 | 1.174 | PASS |
| repository_associate | 19 | 1.155 | PASS |
| task_create | 990 | 1.469 | PASS |
| task_list | 100 | 1.211 | PASS |
| task_show | 100 | 0.202 | PASS |
| task_revise | 4000 | 1.428 | PASS |
| session_bind | 19 | 1.828 | PASS |

All ten operations are far inside the two-second bound; the slowest p95 is
`session_bind` at 1.828 ms, over 1,000× under the threshold. The test fails if the
measurement set is empty or incomplete.

## Artifact

`feature002-performance-95dc67e9dd3e39be3b4a82bcc015ac32875a75da` (`feature002-perf.log`).
