# Feature 001 Compatibility Matrix

The frozen Feature 001 implementation baseline is
`4a06c4125715bb4b78b54e49c81eccd82100a7b7`. Feature 002 extends that behavior
additively.

| Feature 001 surface | Feature 002 change | Compatibility requirement | Verification |
|---|---|---|---|
| Repository UUIDv7 identity | Referenced by project association | Path/remote never becomes identity; moves still resolve | Move-repository association test |
| Worktree identity | Inherits repository project membership | Existing IDs and uniqueness unchanged | Multi-worktree binding test |
| `repositories` table | Referenced by association FK | Rows/columns/values unchanged | Pre/post fixture hashes and counts |
| `worktrees` table | Used to validate session repository | Rows/columns/values unchanged | Migration fixture audit |
| `snapshots` table | No semantic change | Every row and fingerprint preserved | Byte/count audit |
| `sessions` table | Add `binding_mode` defaulting to `local_unbound` | IDs, lifecycle states, leases, timestamps, resume-token hashes unchanged | Real Feature 001 fixture migration |
| Session lifecycle | Orthogonal binding dimension | Active/recovering/stopped/interrupted behavior unchanged | Lifecycle × binding matrix tests |
| Session live uniqueness | No change | Existing repository/agent uniqueness remains authoritative | Existing conflict suites |
| Existing unbound start request | Binding scope omitted | Decodes as explicit `local_unbound` only while bootstrap eligibility holds; otherwise `PROJECT_SCOPE_REQUIRED` | Old request goldens for both eligibility outcomes |
| Extended start request | Optional typed project binding | Reuses one existing start path and readiness boundary | Bound-start integration suite |
| Session start response | Add typed scope | Existing fields/semantics retained; consumers can distinguish modes | Additive schema tripwire |
| Watcher readiness | No change | Success still follows install, ack, reconciliation | All US3 suites |
| Git reconciliation | No change | Remains authoritative over advisory notifications | Existing dropped-event tests |
| Crash recovery | Preserve binding projection | Recovery never changes revision or project | Bound restart/recovery tests |
| Resume tokens | No change to generation/storage | Only hashes persisted; raw token remains response-only | Privacy and migration audits |
| `events.seq` | Reused as global total order | Existing sequence values and row bytes unchanged | Fixture ledger comparison |
| Feature 001 event payloads | No rewrite | Old catalog/replay remains accepted exactly | Combined replay test |
| New event aggregate columns | Nullable for legacy rows | No fake worktree/aggregate IDs backfilled | Migration SQL assertions |
| Event/operation idempotency | Preserve event keys; add immutable global raw-key registry | Existing keys remain valid; same raw key/method/request returns original result; any other reuse is `IDEMPOTENCY_CONFLICT` | Independent-connection retry/conflict suites |
| Projection transactions | Extended with registry and aggregate heads | Registry + events + heads + projections commit or roll back together; cancellation never leaks a reservation | Failure/cancellation/lock-exhaustion suites |
| Local JSON-lines IPC | Add methods and DTO fields | Framing/version/auth behavior unchanged | Old and new IPC goldens |
| CLI JSON envelope | Add commands and scope fields | `cairn.cli.v1` envelope preserved | CLI goldens |
| CLI human output | Show names plus IDs | Existing commands stay compatible | Snapshot/golden tests |
| Secure local IPC | No change | Permissions/peer checks retained | Feature 001 security suites |
| Offline operation | Add project/task behavior | No network dependency introduced | OS-level network-isolated run |
| Privacy | Goal contract is local content | No full contract in logs/errors; old exclusions remain | Sentinel audit |
| SQLite startup migration | Add version 2 | Transactional/version-gated; checksum mismatch or future schema version fails `MIGRATION_FAILED` with no healthy daemon | Actual-runner negative migration tests |
| Formatting, lint, workspace tests | No weakening | Existing Feature 001 targets remain green | Standard quality gate |

## Explicitly unchanged

Feature 002 does not alter:

- snapshot computation or BLAKE3 rules;
- ignored-file handling;
- repository or worktree registration semantics;
- filesystem watcher installation, acknowledgement, event coalescing, or recovery;
- lease acquisition/refresh/release semantics;
- Feature 001 lifecycle transitions;
- committed-event crash durability;
- daemon authentication or local IPC transport;
- existing CLI command behavior.

## Additive compatibility rules

1. Existing databases migrate without fabricated projects, tasks, revisions, associations, bindings, operation-idempotency records, or Feature 002 events.
2. Every migrated session is explicitly `local_unbound` and remains historically valid.
3. Existing event payload bytes and every pre-existing column value remain unchanged; additive aggregate columns read null on legacy rows.
4. Existing clients omitting scope retain bootstrap behavior only when no active repository association or no selectable active task revision exists; otherwise they receive `PROJECT_SCOPE_REQUIRED`.
5. New clients receive typed scope and never infer project awareness from absence.
6. `session.started` always replays unbound; only `session.bound` binds. Bound start commits both in order or exposes neither.
7. A bound session retains its immutable revision through restart and later revisions.
8. Feature 001 replay is a prefix-compatible part of full Feature 002 replay.

## Required regression gate

Before Feature 002 can converge, the implementation commit must pass:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test -p cairn-daemon --test us2_agent_sim
cargo test -p cairn-daemon --test us3_tracking
cargo test -p cairn-daemon --test us3_events
CAIRN_CRASH_ITERS=100 CAIRN_CRASH_EXPECTED_ITERS=100 cargo test -p cairn-daemon --test us4_crash_restart -- --nocapture
cargo test -p cairn-daemon --test perf -- --ignored
```

Feature 002 modifies session start, event writes, recovery, and storage, so the frozen Feature 002 implementation SHA must execute the 100-kill acceptance with exactly 100 completed kills and zero committed event/state loss, plus the ignored SC-007 performance suite. Evidence records exact SHA, command, OS/architecture/toolchain, counters or measurements/fixture size/limits, and pass/fail. A configured workflow, workspace run that leaves perf ignored, or result from another SHA is not evidence.
