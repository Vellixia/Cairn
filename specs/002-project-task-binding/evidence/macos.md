# T117 — macOS exact-SHA Feature 002 evidence

This document records a **completed, successful** macOS job executed against the
frozen implementation commit. A configured job is not evidence; the job below ran
to a `success` conclusion with every step green.

## Frozen implementation commit

| Field | Value |
|---|---|
| Frozen commit SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` |
| Freeze metadata | [`evidence/implementation-freeze.json`](implementation-freeze.json) (generation 4) |
| Pre-freeze gate | [`evidence/pre-freeze.md`](pre-freeze.md) run 5 |

## Workflow references

| Field | Value |
|---|---|
| Workflow run | https://github.com/Vellixia/Cairn/actions/runs/30144710029 (attempt 1) |
| Job | `macOS exact-SHA Feature 002 acceptance` |
| Job URL | https://github.com/Vellixia/Cairn/actions/runs/30144710029/job/89644192922 |
| Job conclusion | **success** |
| Checked-out SHA (verified by the job's `Verify exact implementation SHA` step) | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` |
| Exact-SHA verification | the job fails if `git rev-parse HEAD` ≠ the requested SHA and if the checkout is dirty; the step passed |

## Environment (recorded by the job)

| Field | Value |
|---|---|
| OS | macOS 26.4 |
| Architecture | arm64 |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)` |
| Cargo | `cargo 1.97.1 (c980f4866 2026-06-30)` |
| Git | `git version 2.55.0` |
| SQLite | 3.50.6 |

## Steps and results (all `success`)

| Step | Commands | Result |
|---|---|---|
| Verify exact implementation SHA | `git rev-parse HEAD` == requested SHA; checkout clean | pass |
| Quality gate | `cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace --all-targets` | pass |
| Unix-socket IPC, path, restart, SQLite | `cargo test -p cairn-daemon --test feature002_ipc` · `cargo test -p cairn-cli --test feature002_ipc_only` · `cargo test -p cairn-daemon --test us1_register_inspect` (registration + path inspection) · `cargo test -p cairn-daemon --test feature002_binding_restart --test feature002_bound_recovery` (restart + recovery) · `cargo test -p cairn-daemon --test feature002_migration_acceptance` + `cargo test -p cairn-storage-local --tests` (migration + SQLite) | pass |
| Quickstart and Feature 002 acceptance | `feature002_quickstart` · `feature002_replay` + `feature002_replay_invalid` (replay equality) · `feature002_privacy` · `feature002_retry_acceptance` · `feature002_atomicity` · `feature002_binding_races` · `feature002_privacy_cli` | pass |
| Upload macOS logs and counters | artifact `macos-feature002-evidence-95dc67e9dd3e39be3b4a82bcc015ac32875a75da` | uploaded |

## Recorded counters

```text
quickstart_counts={"project_events":2,"task_events":3,"session_started":2,"session_bound":2,"projects":1,"tasks":1,"revisions":2,"bindings":2}
feature002_retries={"association":100,"revision":100,"binding":100,"registry_records":3,"sequence_gaps":0,"conflicts":3}
```

Quickstart event/projection counts are exact (not "at least"): 1 `project.created`,
1 `project.repository_associated`, 1 `task.created`, 2 `task.revision_created`,
2 `session.started`, 2 `session.bound`; 1 project, 1 task, 2 revisions, 2 bindings.

## Artifact

`macos-feature002-evidence-95dc67e9dd3e39be3b4a82bcc015ac32875a75da` — contains
`environment.txt`, `fmt.log`, `clippy.log`, `workspace-tests.log`, `ipc.log`,
`paths.log`, `restart.log`, `sqlite.log`, `quickstart.log`, `replay.log`,
`acceptance.log`.
