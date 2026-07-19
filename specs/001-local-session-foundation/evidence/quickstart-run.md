# Quickstart Evidence Run

> **Completion status (2026-07-19): PRE-FINAL EVIDENCE COMPLETE / NOT YET
> CONVERGED (75/76).** Completed macOS, Windows, Linux network-isolated,
> exactly-100-kill, and SC-007 executions all test the same frozen Feature 001
> implementation SHA. T059, T069, and T071–T075 are complete. T076 remains open until
> the separate final convergence gate runs and records the evidence payload commit.

**Feature**: 001-local-session-foundation

**Evidence date**: 2026-07-19

**Frozen implementation commit tested**: `4a06c4125715bb4b78b54e49c81eccd82100a7b7`

**Evidence payload commit**: 808fdc257a9f42b0a3370448f624618bfb95e2bc

**Evidence document state**: exact-SHA evidence was committed before final convergence.

The authoritative artifacts are the completed run under
`.specify/workflows/runs/a10779ac/evidence/`: `github-run.json`, `github-run.log`, every
`macos-*.log`, and the four `prefreeze-*.log` files.

## Frozen implementation and GitHub Actions run

GitHub Actions workflow `ci` completed successfully with `headSha` exactly
`4a06c4125715bb4b78b54e49c81eccd82100a7b7`:

- Run: [29690938663](https://github.com/Vellixia/Cairn/actions/runs/29690938663)
- Status/conclusion: `completed` / `success`
- Exact-SHA checkout evidence: the Windows, Linux isolation, SC-005, and SC-007 jobs
  each configured `actions/checkout@v4` with
  `ref: 4a06c4125715bb4b78b54e49c81eccd82100a7b7`; their logs show
  `HEAD is now at 4a06c41 feat(session): complete Feature 001 implementation`.

## macOS clean detached-checkout execution

### Environment and checkout

```text
implementation_sha=4a06c4125715bb4b78b54e49c81eccd82100a7b7
OS: macOS 26.5.2 (build 25F84)
architecture=arm64
rustc 1.90.0-nightly (abf50ae2e 2025-09-16) (1.90.0.0)
cargo 1.90.0-nightly (840b83a10 2025-07-30) (1.90.0.0)
worktree=/tmp/cairn-feature001-a10779ac-4a06c4125715bb4b78b54e49c81eccd82100a7b7
detached_head=true
clean_checkout=true
```

The run created the worktree with `git worktree add --detach`, verified that
`symbolic-ref --quiet HEAD` returned no branch, and verified an empty
`git status --porcelain=v1 --untracked-files=all` before and after execution. Compiler
paths in the logs resolve the same worktree through macOS's `/private/tmp` alias.

### Exact commands

All commands ran from the clean detached worktree:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets

cargo test -p cairn-daemon --test us1_register_inspect -- --nocapture
cargo test -p cairn-daemon --test us2_agent_sim --test us2_sessions -- --nocapture
cargo test -p cairn-daemon --test us3_tracking --test us3_events -- --nocapture
CAIRN_CRASH_ITERS=1 CAIRN_CRASH_EXPECTED_ITERS=1 \
  cargo test -p cairn-daemon --test us4_recovery --test us4_crash_restart -- --nocapture
cargo test -p cairn-daemon --test privacy_audit -- --nocapture
cargo test -p cairn-cli --test us1_cli_contract --test us2_cli_contract \
  --test us4_cli_contract -- --nocapture

RUST_TEST_NOCAPTURE=1 \
  cargo test -p cairn-daemon --test perf -- --ignored
```

### Scenarios 1–6

| Scenario | Captured result |
|---|---|
| 1 — registration and exact inspection | **PASS**: `us1_register_inspect`, 8 passed, 0 failed. Coverage includes fresh/idempotent registration, exact dirty inspection, non-Git and bare-repository rejection, identity restoration, detached HEAD/no remote, ignored pagination, and daemon health. |
| 2 — snapshot-bound deterministic session | **PASS**: `us2_agent_sim` 1/1 and `us2_sessions` 7/7. Marker: `event_total=7 repository_state_changed=1 session_started=1 session_stopped=1`. |
| 3 — live tracking and event counts | **PASS**: `us3_events` 2/2 and all `us3_tracking` tests 10/10. Marker: `repository_state_changed=1 branch_changed=1`. This includes authoritative installation-window reconciliation/deletion, failure stages, dropped notifications, coalescing, and no duplicate logical change event. |
| 4 — forced-kill recovery | **PASS**: smoke crash run 1/1 and recovery suite 7/7. Marker: `configured_iterations=1 completed_forced_kills=1 committed_event_loss=0 invalid_session_outcomes=0`. The authoritative 100-kill result is recorded separately below. |
| 5 — privacy audit | **PASS**: `privacy_audit`, 1 passed, 0 failed; persisted state and logs contained no secrets or tokens. |
| 6 — offline/failure contract realization | **PASS**: CLI contracts 6/6 + 5/5 + 5/5. Both watcher install/reconcile goldens exited 1 as specified; repository, session, heartbeat, reattach, token-redaction, ambiguity, lease-expiry, and failure envelopes passed. |

### Quality and SC-007

- `cargo fmt --check`: **PASS**; its zero-byte log is the captured successful no-diff
  output.
- `cargo clippy --workspace --all-targets -- -D warnings`: **PASS**, 0 warnings.
- `cargo test --workspace --all-targets`: **PASS**, 101 passed, 0 failed, 1 ignored
  (the explicitly invoked performance test).
- Exact-commit SC-007: **PASS** with `tracked_files=10000`, `inspect_ms=110`,
  `snapshot_ms=228`, `inspect_limit_ms=2000`, and `snapshot_limit_ms=2000`.

## Windows exact-SHA GitHub Actions execution

The completed
[Windows Feature 001 Scenarios 1-6 job](https://github.com/Vellixia/Cairn/actions/runs/29690938663/job/88203425635)
checked out the frozen SHA directly and concluded `success`.

### Environment

```text
implementation_sha=4a06c4125715bb4b78b54e49c81eccd82100a7b7
os=Microsoft Windows Server 2025 Datacenter 10.0.26100
architecture=AMD64
rustc 1.97.1 (8bab26f4f 2026-07-14)
cargo 1.97.1 (c980f4866 2026-06-30)
```

### Exact commands

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets

cargo test -p cairn-daemon --test us1_register_inspect -- --nocapture
cargo test -p cairn-daemon --test us2_agent_sim --test us2_sessions -- --nocapture
cargo test -p cairn-daemon --test us3_tracking --test us3_events -- --nocapture
$env:CAIRN_CRASH_ITERS = "1"
$env:CAIRN_CRASH_EXPECTED_ITERS = "1"
cargo test -p cairn-daemon --test us4_recovery --test us4_crash_restart -- --nocapture
cargo test -p cairn-daemon --test privacy_audit -- --nocapture
cargo test -p cairn-cli --test us1_cli_contract --test us2_cli_contract --test us4_cli_contract -- --nocapture
```

### Scenarios 1–6 and quality results

| Scenario | Captured exact-SHA Windows result |
|---|---|
| 1 | **PASS**, 8 passed, 0 failed. |
| 2 | **PASS**, 1 + 7 passed; `event_total=7 repository_state_changed=1 session_started=1 session_stopped=1`. |
| 3 | **PASS**, 2 + 10 passed; `repository_state_changed=1 branch_changed=1`. |
| 4 | **PASS**, 1 + 7 passed; `configured_iterations=1 completed_forced_kills=1 committed_event_loss=0 invalid_session_outcomes=0`. |
| 5 | **PASS**, 1 passed, 0 failed. |
| 6 | **PASS**, 6 + 5 + 5 passed, 0 failed. |

The job's final markers explicitly report `feature001_scenario_1=pass` through
`feature001_scenario_6=pass`. `cargo fmt --check` and Clippy passed; the complete
workspace run passed with 101 passed, 0 failed, and 1 explicitly ignored performance
test.

## Linux network-isolated execution

The completed
[Linux network-isolated Feature 001 validation job](https://github.com/Vellixia/Cairn/actions/runs/29690938663/job/88203425632)
checked out the same frozen SHA and concluded `success`.

```text
implementation_sha=4a06c4125715bb4b78b54e49c81eccd82100a7b7
os=Ubuntu 24.04.4 LTS
architecture=x86_64
rustc 1.97.1 (8bab26f4f 2026-07-14)
cargo 1.97.1 (c980f4866 2026-06-30)
```

Dependencies, binaries, tests, and the isolation image were fetched/built before
networking was disabled:

```sh
cargo fetch --locked
cargo test --workspace --all-targets --no-run --locked
docker pull rust:1-bookworm
```

The captured isolation command was:

```sh
docker info >/dev/null
echo "isolation_mechanism=docker --network none"
docker run --rm --network none \
  --volume "/home/runner/work/Cairn/Cairn:/workspace" \
  --volume "/home/runner/.cargo/registry:/usr/local/cargo/registry" \
  --volume "/home/runner/.cargo/git:/usr/local/cargo/git" \
  --workdir /workspace \
  --env CARGO_NET_OFFLINE=true \
  rust:1-bookworm \
  bash -euxo pipefail -c '
    test -f /workspace/Cargo.toml
    printf "filesystem-ok\n" > /workspace/target/network-isolation-filesystem-proof
    grep -F "filesystem-ok" /workspace/target/network-isolation-filesystem-proof
    echo "local_filesystem=available"
    if curl --silent --show-error --connect-timeout 2 https://example.com >/dev/null; then
      echo "external network unexpectedly reachable" >&2
      exit 1
    fi
    echo "external_network=unreachable"
    cargo test --offline --locked -p cairn-daemon \
      --test us1_register_inspect \
      --test us2_agent_sim \
      --test us3_tracking \
      --test us4_recovery \
      --test privacy_audit -- --nocapture
    echo "local_ipc=available"
    cargo test --offline --locked -p cairn-cli \
      --test us1_cli_contract \
      --test us2_cli_contract \
      --test us4_cli_contract -- --nocapture
    echo "feature001_network_isolated_scenarios=pass"
  '
```

The live container output proves:

- `isolation_mechanism=docker --network none`.
- The filesystem write/read check printed `filesystem-ok` and
  `local_filesystem=available`.
- The external HTTPS probe failed with `curl: (6) Could not resolve host: example.com`,
  followed by `external_network=unreachable`.
- Daemon/repository/session/live-tracking/recovery/privacy suites passed 27 tests; their
  event markers were Scenario 2 `event_total=7 repository_state_changed=1
  session_started=1 session_stopped=1` and Scenario 3
  `repository_state_changed=1 branch_changed=1`.
- Real CLI/daemon local IPC remained available; the job printed `local_ipc=available`,
  and all 16 CLI contract tests passed.
- The terminal marker was `feature001_network_isolated_scenarios=pass`.

This is completed OS-level isolation evidence, not `cargo --offline` or configured-only
workflow text.

## SC-005 exactly 100 forced kills

The completed
[SC-005 exactly 100 forced daemon kills job](https://github.com/Vellixia/Cairn/actions/runs/29690938663/job/88203425605)
checked out the frozen SHA and concluded `success`.

```sh
# GitHub Actions step environment:
CAIRN_CRASH_ITERS=100
CAIRN_CRASH_EXPECTED_ITERS=100

# Shell body:
set -o pipefail
cargo test -p cairn-daemon --test us4_crash_restart -- --nocapture 2>&1 | tee sc005-output.txt
grep -F "configured_iterations=100 completed_forced_kills=100 committed_event_loss=0 invalid_session_outcomes=0" sc005-output.txt
echo "configured_iterations=100"
echo "completed_forced_kills=100"
echo "committed_event_loss=0"
echo "invalid_session_outcomes=0"
```

The harness and the explicit postcondition both reported:

```text
configured_iterations=100
completed_forced_kills=100
committed_event_loss=0
invalid_session_outcomes=0
test result: ok. 1 passed; 0 failed
```

The passing test is
`randomized_kills_lose_no_committed_events_and_sessions_recover`; thus every affected
active session satisfied the harness's recovery/outcome assertions across exactly 100
completed forced kills.

## SC-007 10,000-file performance evidence

| Execution | SHA binding | Environment | Inspect | Snapshot | Limits | Result |
|---|---|---|---:|---:|---:|---|
| Clean detached macOS exact-commit run | `4a06c4125715bb4b78b54e49c81eccd82100a7b7` | macOS 26.5.2 arm64 | 110 ms | 228 ms | 2,000 ms each | **PASS** |
| [GitHub Actions SC-007 job](https://github.com/Vellixia/Cairn/actions/runs/29690938663/job/88203425606) | exact-SHA checkout | Ubuntu 24.04.4 x86_64 | 34 ms | 108 ms | 2,000 ms each | **PASS** |
| Pre-freeze corroboration | implementation tree immediately before freeze | macOS arm64 | 82 ms | 212 ms | 2,000 ms each | **PASS** |

Each row used `tracked_files=10000`. The authoritative clean macOS and GitHub Actions
commands were both `cargo test -p cairn-daemon --test perf -- --ignored` with
`RUST_TEST_NOCAPTURE=1`; both tests passed.

## Pre-final verification and task traceability

The exact-commit macOS/Windows workspace runs also passed the T061 typed watcher schemas,
IPC and CLI goldens, install/reconcile exit mappings, replay, and non-leakage coverage;
the T065 authoritative installation-window test; `us2_agent_sim`; all 10
`us3_tracking` tests; and both `us3_events` tests. Before freezing, the same implementation
tree passed formatting, Clippy, 101 workspace tests with 0 failures and 1 ignored perf
test, and SC-007 at 82/212 ms. These completed results, the cross-platform runs above,
Linux isolation, and the 100-kill job satisfy the pre-final T072 gate.

| Task/criterion | Completed evidence |
|---|---|
| T059 / T071 / T073 | Same frozen SHA on clean detached macOS and exact-SHA Windows; commands, environments, scenario results, event counts, workspace, fmt, and Clippy recorded above. |
| T069 / T074 | Completed successful Docker `--network none` job with exact SHA, pre-build/fetch, external-network failure, local filesystem/IPC proofs, and passing behavior. |
| T072 | Complete pre-final verification across watcher contracts/races, scenario suites, workspace/fmt/Clippy, Linux isolation, 100 kills, and explicit SC-007. |
| T075 / SC-005 | Dedicated exact-SHA job completed exactly 100/100 kills with zero committed-event loss and zero invalid session outcomes. |
| SC-001 | Linux no-network container preserved local filesystem and IPC while external HTTPS was unreachable. |
| SC-002 | Scenario 2 deterministic snapshot/session tests passed on both required operating systems and in Linux isolation. |
| SC-003 | All 10 live-tracking tests passed; Scenario 3 emitted one state-change and one branch-change event. |
| SC-004 | Scenario 1 registration/idempotence and zero-write rejection tests passed. |
| SC-005 | Dedicated 100-kill acceptance result above. |
| SC-006 | Privacy audit passed on macOS, Windows, and isolated Linux. |
| SC-007 | Explicit 10,000-file exact-commit results and 2-second limits above. |
| SC-008 | Workspace `json_stability` coverage passed on macOS and Windows. |

T076 remains deliberately incomplete. The final 76/76 declaration and the evidence
payload commit must be produced by the separate final convergence workflow step, not by
this pre-final evidence update.

## Historical disclosures retained

These earlier results remain useful diagnostics but are superseded for completion by the
frozen exact-commit evidence above:

- **2026-07-19 dirty macOS remediation run**: macOS 26.5.2 arm64, nightly Rust/Cargo
  1.90.0, recorded HEAD `d3491b0ebc7d8574b7af8cb50ff0b7f8a4e705f0`, and an
  uncommitted watcher-readiness implementation. Its isolated root was
  `/private/tmp/cairn-quickstart-019f79bb`; repository/session IDs were
  `019f79e4-61c4-76a2-b124-039b512d42f4` and
  `019f79e5-0644-7992-be4b-5ec006464032`. Scenarios 1–5 passed; Scenario 6 had detached
  HEAD and corruption checks but Linux isolation was only configured. Manual recovery
  retained 11/11 pre-kill events and ended at 14 events, including one rejected reattach,
  one recovery, and two cumulative stops. Its local 100-kill diagnostic reported zero
  loss/invalid outcomes, and its workspace run reported 96 passed, 0 failed, 1 ignored.
  Because the tree was dirty, none of those results was used to close exact-commit tasks.
- **2026-07-16 Windows manual run**: Windows 11 Pro 10.0.26200 with real debug binaries
  and isolated data/pipe. Scenarios covered idempotent registration, exact dirty status,
  snapshot equality, live edit/branch tracking, rejected/valid recovery, zero persisted
  secret/token bytes, detached HEAD, and a 14-event history. Its workspace result was 90
  passed, 0 failed, 1 ignored; the performance diagnostic was 206 ms inspect / 382 ms
  snapshot at 10,000 files; and its crash diagnostic used only 8 kills. It predates the
  watcher remediation and does not replace the exact-SHA Windows job above.
- **Earlier Linux disclosure**: the previously described `unshare(1) --net` workflow was
  configured-only and therefore not evidence. It is superseded by the completed Docker
  `--network none` job above.
