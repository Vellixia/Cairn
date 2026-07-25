# T136 — Feature 001 SC-007 performance acceptance (exact-SHA)

A **completed, successful** job executed the authoritative Feature 001 SC-007
performance acceptance (`--ignored` perf suite) against the frozen implementation
commit. An ignored result inside the ordinary workspace sweep does not count; this
is the explicit `-- --ignored` execution.

## Frozen implementation commit

| Field | Value |
|---|---|
| Frozen commit SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` |
| Freeze metadata | [`evidence/implementation-freeze.json`](implementation-freeze.json) (generation 4) |

## Workflow references

| Field | Value |
|---|---|
| Workflow run | https://github.com/Vellixia/Cairn/actions/runs/30144710029 (attempt 1) |
| Job | `SC-007 explicit performance acceptance` |
| Job URL | https://github.com/Vellixia/Cairn/actions/runs/30144710029/job/89644192965 |
| Job conclusion | **success** |
| Checked-out SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` (verified by the job's SHA-verify step) |

## Command and environment

| Field | Value |
|---|---|
| Command | `cargo test -p cairn-daemon --test perf -- --ignored` (with `RUST_TEST_NOCAPTURE=1`) |
| OS / arch | Ubuntu 24.04 / x86_64 |
| Toolchain | rustc/cargo 1.97.1 |
| Profile | debug (the Feature 001 perf test has no profile gate; the authoritative invocation is the plain `--ignored` form) |
| Exit code | 0 |

## Measurements

```text
SC-007 acceptance: tracked_files=10000 inspect_ms=48 snapshot_ms=106 inspect_limit_ms=2000 snapshot_limit_ms=2000
```

| Metric | Value | Limit | Result |
|---|---:|---:|---|
| Tracked files | 10,000 | — | — |
| Inspect | 48 ms | 2000 ms | PASS |
| Snapshot | 106 ms | 2000 ms | PASS |

## Artifact

`sc007-performance-95dc67e9dd3e39be3b4a82bcc015ac32875a75da` (`sc007-perf.log`).
