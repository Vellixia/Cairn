# T122 — Feature 002 preliminary acceptance summary

This consolidates the completed, exact-SHA execution evidence for Feature 002. Every
result below comes from a job that **ran to a `success` conclusion against the single
frozen implementation commit**, verified by an in-job SHA check. Configured-only,
skipped, cancelled, dirty-checkout, different-SHA, or merge-SHA results are rejected
and none are cited here.

This is the **preliminary** evidence summary (T122). It is not the final convergence
declaration; the independent final-gate audit (T124–T130) runs in a separate session.

## Frozen implementation

| Field | Value |
|---|---|
| Frozen commit SHA | `95dc67e9dd3e39be3b4a82bcc015ac32875a75da` |
| Frozen tree SHA | `8a3353eee55fc6805bea70e1b7e6f823dd7ab022` |
| Branch | `feature/002-project-task-binding` |
| Freeze metadata | [`implementation-freeze.json`](implementation-freeze.json) (generation 4) |
| Pre-freeze gate | [`pre-freeze.md`](pre-freeze.md) run 5 — 12/12 commands exit 0, on the stable toolchain |
| Tracked files at frozen tree | 614 |

Three earlier freezes were invalidated by exact-SHA CI and superseded (never amended,
never rebased, retained in history): `9021ba9` (clippy-on-stable + harness file mode),
`964eb3a` (isolation container glibc + root-owned files), `b194614` (non-existent
`rust:1-noble` image). Evidence from those SHAs is void. Details in
`implementation-freeze.json`.

## Workflow run

| Field | Value |
|---|---|
| Workflow run | https://github.com/Vellixia/Cairn/actions/runs/30144710029 |
| Trigger | `workflow_dispatch` with `implementation_sha=95dc67e9dd3e39be3b4a82bcc015ac32875a75da` |
| Final conclusion | **success** — 13/13 jobs |
| Attempt | 2 (attempt 1 rerun for a single transient Windows `git rebase` failure; see `windows.md`) |

## Platform / toolchain matrix

| Platform | OS | Arch | Rust / Cargo | SQLite | Job conclusion |
|---|---|---|---|---|---|
| macOS | macOS 26.4 | arm64 | 1.97.1 | 3.50.6 | success |
| Linux | Ubuntu 24.04.4 | x86_64 | 1.97.1 | 3.45.1 | success |
| Windows | Windows Server 2025 (10.0.26100) | AMD64 | 1.97.1 | — | success (attempt 2) |
| Linux (perf) | Ubuntu 24.04 | x86_64 | 1.97.1 | 3.46.0 | success |

The CI toolchain (`dtolnay/rust-toolchain@stable`, observed 1.97.1) differs from the
local gate's stable (1.92.0); both evaluate the same lint set, which is why the local
gate now runs through `rustup run stable`.

## Every job (all 13 `success`)

| Job | Task | Job URL suffix |
|---|---|---|
| macOS exact-SHA Feature 002 acceptance | T117 | `/job/89644192922` |
| Windows exact-SHA Feature 002 acceptance | T118 | `/job/89644737954` (attempt 2) |
| Linux exact-SHA Feature 002 acceptance | T119 | `/job/89644192955` |
| Linux network-isolated Feature 002 acceptance | T120 | `/job/89644192973` |
| SC-010 Feature 002 performance acceptance | T121 | `/job/89644192956` |
| SC-005 exactly 100 forced daemon kills | T135 | `/job/89644192937` |
| SC-007 explicit performance acceptance | T136 | `/job/89644192965` |
| named-pipe ACL negative test (analysis I1) | (Windows security) | `/job/89644192976` |
| Linux network-isolated Feature 001 validation | (F001 isolation) | `/job/89644192950` |
| build + test (ubuntu-latest) | (workspace) | `/job/89644192938` |
| build + test (macos-latest) | (workspace) | `/job/89644192944` |
| build + test (windows-latest) | (workspace) | `/job/89644192958` |
| Windows Feature 001 Scenarios 1-6 | (F001 regression) | `/job/89644192947` |

Full URL prefix: `https://github.com/Vellixia/Cairn/actions/runs/30144710029`.

## Consolidated results

| Acceptance dimension | Result | Source |
|---|---|---|
| Quality (fmt / clippy / workspace tests) | pass on macOS, Linux, Windows | `macos.md`, `linux.md`, `windows.md` |
| Migration from populated Feature 001 fixture | pass (all three platforms) | per-platform quality/sqlite steps |
| Quickstart — exact counts | `project.created 1`, `project.repository_associated 1`, `task.created 1`, `task.revision_created 2`, `session.started 2`, `session.bound 2`; projections 1/1/2/2 | all platforms; `quickstart_counts=…` |
| Replay equality (mixed ledger) | pass (`feature002_replay` + `feature002_replay_invalid`) | all platforms |
| Atomicity / failure injection | pass (`feature002_atomicity`) | all platforms |
| Exactly 100 idempotent retries | `association 100 / revision 100 / binding 100`, 1 registry record each, 0 sequence gaps, 3 conflicts | `feature002_retries=…`, all platforms |
| Restart / recovery (bound sessions) | pass (`feature002_binding_restart` + `feature002_bound_recovery`) | all platforms |
| Privacy (daemon + CLI sentinels) | pass (`feature002_privacy` + `feature002_privacy_cli`) | all platforms |
| Genuine offline isolation | `docker --network none`; external denial + local fs/git/ipc/migration/quickstart/replay/privacy all proven | `linux-isolated.md` |
| Feature 002 performance (SC-010) | 10/10 operations p95 ≤ 1.828 ms vs 2000 ms | `performance.md` |
| Feature 001 exactly-100 forced kills | 100/100, committed_event_loss=0, invalid_session_outcomes=0 | `feature001-100-kill.md` |
| Feature 001 performance (SC-007) | inspect 48 ms, snapshot 106 ms vs 2000 ms | `feature001-sc007.md` |
| Named-pipe security (Windows) | second local user cannot connect | `windows.md` |
| Schema / error inventory | checked-in schemas report zero drift; all stable errors have typed goldens (workspace tests include `cairn-protocol` schema + golden gates) | build+test + exact-SHA quality gates |
| Feature 001 regression | Feature 001 focused scenarios and the workspace sweep pass on all platforms | build+test + Windows Feature 001 job |

## Success-criterion coverage (exact-SHA)

| SC | Evidence | Result |
|---|---|---|
| SC-001 (Quickstart) | exact counts, all platforms | pass |
| SC-002 (100 retries) | `feature002_retries` | pass |
| SC-003 (migration) | migration acceptance, all platforms | pass |
| SC-005 (replay equality) | `feature002_replay` | pass |
| SC-006 (no partial state) | atomicity + invalid replay | pass |
| SC-007 (F001 perf) | inspect 48 / snapshot 106 ms | pass |
| SC-008 (offline) | genuine `--network none` isolation | pass |
| SC-009 (typed contracts) | schema/golden gates in workspace tests | pass |
| SC-010 (F002 perf) | 10 ops under 2 s | pass |
| SC-011 (privacy) | daemon + CLI sentinel audits | pass |
| SC-012 (cross-platform parity) | identical behavior on macOS/Windows/Linux | pass |
| Feature 001 SC-005 (100 kills) | 100/100/0/0 | pass |

## Task accounting at this point

- T001–T123 addressed (T122 = this summary; T123 = the evidence commit that records it).
- T131–T136 complete.
- T124–T130 remain unchecked (independent final-gate audit, convergence, and
  declaration — a separate session).

## Rejections (none applied)

No evidence here is from another SHA, a dirty checkout, a skipped/cancelled job, a
configured-only job, or a pull-request merge SHA. Every job verified
`git rev-parse HEAD == 95dc67e9dd3e39be3b4a82bcc015ac32875a75da`.
