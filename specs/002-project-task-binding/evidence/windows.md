# T118 — Windows exact-SHA Feature 002 evidence

A **completed, successful** Windows job executed against the frozen implementation
commit, exercising named-pipe transport (there is no Unix-socket reliance on
Windows). The job failed once on a transient `git rebase` error in an unrelated
Feature 001 test and passed on rerun against the same frozen SHA (see *Transient
rerun* below).

## Frozen implementation commit

| Field | Value |
|---|---|
| Frozen commit SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` |
| Freeze metadata | [`evidence/implementation-freeze.json`](implementation-freeze.json) (generation 4) |

## Workflow references

| Field | Value |
|---|---|
| Workflow run | https://github.com/Vellixia/Cairn/actions/runs/30144710029 |
| Job | `Windows exact-SHA Feature 002 acceptance` (attempt 2) |
| Job URL | https://github.com/Vellixia/Cairn/actions/runs/30144710029/job/89644737954 |
| Job conclusion | **success** |
| Checked-out SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` (verified: `requested_sha` == `implementation_sha`) |
| Named-pipe security job | `named-pipe ACL negative test (analysis I1)` — job https://github.com/Vellixia/Cairn/actions/runs/30144710029/job/89644192976 — **success** (a second local user cannot connect to the daemon's named pipe) |

## Environment (recorded by the job)

| Field | Value |
|---|---|
| OS | Microsoft Windows Server 2025 Datacenter 10.0.26100 |
| Architecture | AMD64 |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)` |
| Cargo | `cargo 1.97.1 (c980f4866 2026-06-30)` |
| Git | `git version 2.55.0.windows.2` |

## Steps and results (all `success`)

| Step | Coverage | Result |
|---|---|---|
| Verify exact implementation SHA | `requested_sha` == `git rev-parse HEAD` | pass |
| Quality gate | `cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace --all-targets` (with `$LASTEXITCODE` propagated after each) | pass |
| Named-pipe IPC, path, restart, SQLite | `feature002_ipc` (`every_feature002_method_runs_over_the_real_local_transport` — Feature 002 methods over the real Windows named pipe) · `feature002_ipc_only` · `us1_register_inspect` (path) · `feature002_binding_restart` + `feature002_bound_recovery` (restart/recovery) · `feature002_migration_acceptance` + `cairn-storage-local --tests` (migration + SQLite) | pass |
| Named-pipe security | separate `named-pipe ACL negative test` job proves a second local user cannot connect | pass |
| Quickstart and Feature 002 acceptance | `feature002_quickstart` · `feature002_replay` + `feature002_replay_invalid` · `feature002_privacy` · `feature002_retry_acceptance` · `feature002_atomicity` · `feature002_binding_races` · `feature002_privacy_cli` | pass |
| Upload Windows logs and counters | artifact `windows-feature002-evidence-95dc67e9dd3e39be3b4a82bcc015ac32875a75da` | uploaded |

No Unix-only behavior is relied upon; the transport is the Windows named pipe
`\\.\pipe\...`.

## Recorded counters

```text
quickstart_counts={"project_events":2,"task_events":3,"session_started":2,"session_bound":2,"projects":1,"tasks":1,"revisions":2,"bindings":2}
feature002_retries={"association":100,"revision":100,"binding":100,"registry_records":3,"sequence_gaps":0,"conflicts":3}
```

## Transient rerun (recorded honestly)

Attempt 1 of the Windows exact-SHA job failed in its `Quality gate` step on a single
Feature 001 test, `us3_tracking.rs::delete_and_rebase_are_tracked_without_corruption`,
with `git ["rebase", "main"] failed: exit code: 128` — git could not start the
rebase. This was classified **transient (infrastructure)**, not an implementation or
test defect, on decisive evidence: on the *same run and same frozen SHA*, the
`build + test (windows-latest)` job ran the same `cargo test --workspace` (including
that test) and **passed**. Same commit, same test, two Windows runners → one pass,
one fail. `git rebase` sequencer temp-file races on hosted Windows runners are a
known transient class.

Per the failure policy, the failed job was rerun against the **same** frozen SHA
(`gh run rerun 30144710029 --failed`, attempt 2). The 12 already-successful jobs were
preserved. Attempt 2 of the Windows exact-SHA job reached `success` with zero failed
steps. No source was changed; the frozen SHA is unchanged.

## Artifact

`windows-feature002-evidence-95dc67e9dd3e39be3b4a82bcc015ac32875a75da` (attempt 2).
