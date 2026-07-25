# T120 — genuine OS-level network-isolation evidence

A **completed, successful** Linux job proved Feature 002 acceptance under genuine
operating-system-level network isolation, running prebuilt binaries with no Cargo
dependency download inside isolation.

The earlier macOS harness exit 69 was fail-closed *harness validation only* and is
**not** T120 evidence. This document records genuine isolation on a Linux runner.

## Frozen implementation commit

| Field | Value |
|---|---|
| Frozen commit SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` |
| Freeze metadata | [`evidence/implementation-freeze.json`](implementation-freeze.json) (generation 4) |

## Workflow references

| Field | Value |
|---|---|
| Workflow run | https://github.com/Vellixia/Cairn/actions/runs/30144710029 (attempt 1) |
| Job | `Linux network-isolated Feature 002 acceptance` |
| Job URL | https://github.com/Vellixia/Cairn/actions/runs/30144710029/job/89644192973 |
| Job conclusion | **success** |
| Checked-out SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` (verified by the job's SHA-verify step) |

## Isolation mechanism

| Field | Value |
|---|---|
| Selected mechanism | `docker --network none` (fail-closed container fallback; `unshare -n` reported `unavailable_or_not_permitted` on the hosted runner and the harness fell through to the container) |
| Container image | `rust:1-trixie` (Debian 13, glibc 2.41 ≥ the Ubuntu 24.04 builder's glibc 2.39, ships git) |
| Dependencies fetched/built | **before** isolation (`cargo fetch --locked`, `cargo test --no-run`); no Cargo invocation inside isolation |
| Binaries | prebuilt on the runner and copied into the bundle; preflight `--list` proved each loads inside the container (`prebuilt_binaries_loadable=yes`) |

## Proof markers (from `target/feature002-isolation-logs/run.log`, all grep-gated by the job)

```text
selected_isolation=docker_network_none
external_network=unreachable
local_filesystem=available
local_git=available
prebuilt_binaries_loadable=yes
local_ipc=available
feature002_migration_acceptance=pass
feature002_quickstart=pass
feature002_mixed_replay=pass
feature002_privacy=pass
feature002_network_isolated=pass
isolation_mechanism=docker --network none
```

| Required proof | Marker | Result |
|---|---|---|
| External network request fails | TCP to 1.1.1.1:80 and DNS for example.com both fail → `external_network=unreachable` | pass |
| Local filesystem works | `local_filesystem=available` | pass |
| Local Git works | `local_git=available` (init + commit + rev-parse in-container) | pass |
| Local IPC works | `local_ipc=available` (Unix-socket IPC exercised by the acceptance binaries) | pass |
| Migration acceptance | `feature002_migration_acceptance=pass` | pass |
| Quickstart | `feature002_quickstart=pass` | pass |
| Mixed replay | `feature002_mixed_replay=pass` | pass |
| Privacy | `feature002_privacy=pass` | pass |

The job's `Execute Feature 002 without external networking` step greps for
`external_network=unreachable`, `local_filesystem=available`, `local_git=available`,
`local_ipc=available`, and `feature002_network_isolated=pass`; a missing marker fails
the step. The harness exits nonzero if isolation cannot be established, so a skipped
isolation cannot be recorded as a pass.

## Artifact

`feature002-network-isolated-95dc67e9dd3e39be3b4a82bcc015ac32875a75da` (`run.log`).
