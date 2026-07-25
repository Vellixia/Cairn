# T119 — normal Linux exact-SHA Feature 002 evidence

A **completed, successful** Linux job executed against the frozen implementation
commit. Configuration alone is not evidence; the job below reached a `success`
conclusion with every step green.

## Frozen implementation commit

| Field | Value |
|---|---|
| Frozen commit SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` |
| Freeze metadata | [`evidence/implementation-freeze.json`](implementation-freeze.json) (generation 4) |

## Workflow references

| Field | Value |
|---|---|
| Workflow run | https://github.com/Vellixia/Cairn/actions/runs/30144710029 (attempt 1) |
| Job | `Linux exact-SHA Feature 002 acceptance` |
| Job URL | https://github.com/Vellixia/Cairn/actions/runs/30144710029/job/89644192955 |
| Job conclusion | **success** |
| Checked-out SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` (verified by the job's SHA-verify step) |

## Environment (recorded by the job)

| Field | Value |
|---|---|
| OS | Ubuntu 24.04.4 LTS |
| Architecture | x86_64 |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)` |
| Cargo | `cargo 1.97.1 (c980f4866 2026-06-30)` |
| Git | `git version 2.54.0` |
| SQLite | 3.45.1 |

## Steps and results (all `success`)

| Step | Coverage | Result |
|---|---|---|
| Verify exact implementation SHA | HEAD == requested SHA; checkout clean | pass |
| Quality gate | `cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace --all-targets` (Feature 001 + Feature 002 regressions) | pass |
| Unix-socket IPC, path, restart, SQLite | `feature002_ipc` · `feature002_ipc_only` · `us1_register_inspect` (path) · `feature002_binding_restart` + `feature002_bound_recovery` (restart/recovery) · `feature002_migration_acceptance` + `cairn-storage-local --tests` (migration + SQLite) | pass |
| Quickstart and Feature 002 acceptance | `feature002_quickstart` · `feature002_replay` + `feature002_replay_invalid` (replay equality) · `feature002_privacy` · `feature002_retry_acceptance` · `feature002_atomicity` · `feature002_binding_races` · `feature002_privacy_cli` | pass |
| Upload Linux logs and counters | artifact `linux-feature002-evidence-95dc67e9dd3e39be3b4a82bcc015ac32875a75da` | uploaded |

## Recorded counters

```text
quickstart_counts={"project_events":2,"task_events":3,"session_started":2,"session_bound":2,"projects":1,"tasks":1,"revisions":2,"bindings":2}
feature002_retries={"association":100,"revision":100,"binding":100,"registry_records":3,"sequence_gaps":0,"conflicts":3}
```

## Artifact

`linux-feature002-evidence-95dc67e9dd3e39be3b4a82bcc015ac32875a75da`.
