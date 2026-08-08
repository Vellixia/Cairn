# Feature 001 — non-blocking follow-ups

Feature 001 converged with 0 CRITICAL and 0 HIGH findings. The items below are
known, deliberately deferred, and none of them is a security, privacy,
data-loss, or corruption risk. They are recorded here so they are not
rediscovered from scratch later.

## 1. CI subset-package stale-binary guard (MEDIUM)

`scripts/network-isolated-tests.sh` used to run `cargo test -p cairn-e2e …`
without building `cairn` and `cairnd`. Cargo does not build another package's
binaries for a test run, so the end-to-end suite drove whatever binaries were
left in the cached target directory — for a while it was silently testing a
daemon several hours out of date and reporting it as a pass. The script now
builds them explicitly.

The class of mistake is not fixed, only this instance. `.github/workflows/ci.yml`
runs `cargo test --workspace`, which does build every binary, so CI is not
affected today. Any future job that narrows to a subset of packages inherits the
same trap.

**Follow-up**: assert binary freshness in the harness — for example, have
`cairn_e2e::binary()` compare the binary's mtime against the newest source file
and fail loudly rather than silently testing something stale.

## 2. Daemon-path contention instrumentation (LOW)

`crates/cairn-store/src/diag.rs` reports SQLite contention — operation, stage,
extended and primary result codes, attempt — when `CAIRN_CONTENTION_LOG` names a
file. It is off by default and records no payload, content, path, or identifier.

It works in-process (verified against forced `SQLITE_BUSY`) and in a daemon
started directly with the variable set. It did **not** produce output from a
daemon spawned by the CLI inside the Linux test container, and the reason was
never established. Convergence evidence for that environment therefore rests on
the passing suites and the mutation result, not on the log.

**Follow-up**: work out why the variable does not reach that daemon, or have the
daemon log its diagnostic configuration at startup so the gap is visible rather
than silent.

## 3. Startup claim-release test coverage (LOW)

`recover::release_abandoned_claims` runs at daemon start and hands back outbox
rows a dead run left `in_flight`. Removing the call leaves every test green:
recovery still happens through the 60-second stale-claim timeout, so correctness
holds and only promptness changes (measured: ~12 s recovery with the call, ~70 s
without).

**Follow-up**: cover the startup path directly, asserting recovery is prompt and
not merely eventual.

## 4. `--max-connections` product surface (LOW)

`cairn-server` gained `--max-connections` (default 10, behaviour unchanged when
omitted) so the end-to-end suite can run many servers against one PostgreSQL
without exhausting its connection limit. It is real product surface added for a
test problem, though it is also defensible for shared-database deployments.

**Follow-up**: decide whether to keep it as an operator knob or make it
test-only.

## 5. Traceability-only FR citations (LOW)

FR-025, FR-045 and FR-063 carry no explicit task-ID citation in `tasks.md`. Each
was verified directly:

- **FR-025** (no embeddings, vector store, or knowledge graph in any retrieval
  path) — no such dependency exists in any manifest or source; retrieval is
  FTS5. True by construction.
- **FR-045** (fully offline) — proven by the network-isolated suite, which runs
  the local product in a container with loopback only.
- **FR-063** (no graph visualisation or analytics dashboards in the web UI) — the
  UI has only login and project routes and no charting dependency.

**Follow-up**: add citations if the analyzer's coverage table should read clean.
Do not invent tasks to satisfy it.

## 6. Performance evidence beyond this machine (LOW)

SC-007 is asserted on a release build on one macOS arm64 machine: median
3.4–3.7 ms against a 10 ms budget, comfortable margin. The benchmark is
load-sensitive — it fails when the host is saturated by other test runs, which
is a property of the measurement, not the product.

No hosted-CI run exists for this branch, and there is no evidence from other
hardware or from Linux.

**Follow-up**: record SC-007 from CI on both `ubuntu-latest` and `macos-latest`,
and treat a saturated host as an invalid measurement rather than a failure.
