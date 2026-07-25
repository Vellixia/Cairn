# T135 — Feature 001 exactly-100-forced-kill acceptance (exact-SHA)

A **completed, successful** job executed the authoritative Feature 001
exactly-100-forced-kill acceptance against the frozen implementation commit. This is
the Feature 001 SC-005 crash-recovery gate — **not** the Feature 002
100-idempotency-retry test.

## Frozen implementation commit

| Field | Value |
|---|---|
| Frozen commit SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` |
| Freeze metadata | [`evidence/implementation-freeze.json`](implementation-freeze.json) (generation 4) |

## Workflow references

| Field | Value |
|---|---|
| Workflow run | https://github.com/Vellixia/Cairn/actions/runs/30144710029 (attempt 1) |
| Job | `SC-005 exactly 100 forced daemon kills` |
| Job URL | https://github.com/Vellixia/Cairn/actions/runs/30144710029/job/89644192937 |
| Job conclusion | **success** |
| Checked-out SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` (verified by the job's SHA-verify step, and echoed as `implementation_sha=95dc67e9dd3e39be3b4a82bcc015ac32875a75da`) |

## Command and environment

| Field | Value |
|---|---|
| Command | `CAIRN_CRASH_ITERS=100 CAIRN_CRASH_EXPECTED_ITERS=100 cargo test -p cairn-daemon --test us4_crash_restart -- --nocapture` |
| OS / arch | Ubuntu 24.04 / x86_64 |
| Toolchain | rustc/cargo 1.97.1 |
| Exit code | 0 |

## Counters (asserted by the job's `grep -F` gate)

```text
SC-005 acceptance: configured_iterations=100 completed_forced_kills=100 committed_event_loss=0 invalid_session_outcomes=0
```

| Counter | Value |
|---|---:|
| Configured iterations | 100 |
| Completed forced kills | 100 |
| Committed event loss | 0 |
| Invalid session outcomes | 0 |

The job's step greps for the exact line
`configured_iterations=100 completed_forced_kills=100 committed_event_loss=0 invalid_session_outcomes=0`;
a missing line fails the step.

## Artifact

`sc005-exactly-100-kill-95dc67e9dd3e39be3b4a82bcc015ac32875a75da`
(`environment.txt`, `counters.txt`, `sc005-output.txt`).
