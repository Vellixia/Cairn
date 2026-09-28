# Cairn verification plan

Updated 2026-09-27 after independent Astra review. Target: alpha.8 and its corrective successor, not the older alpha.7 workspace interface. This is a coverage plan, not a claim that every check exists or passed. The source fix lives on `codex/session-recovery-tests`, based on tag `4cbb2c6`.

## Evidence contract

A green test count is not a complete product verdict. Each release requirement needs a runnable check or an explicit gap. Report `PASS`, `FAIL`, or `NOT RUN` with reason; record source commit, binary versions/paths, platform, database version, candidate image digests, and actual commands. Never record credentials or raw agent material. Citation coverage alone does not prove behavior.

Required server lanes set `CAIRN_REQUIRE_DATABASE_TESTS=1` and `CAIRN_TEST_DATABASE_URL`; missing/empty database configuration must fail before a server suite silently returns. Optional local runs emit `NOT RUN`; use `--show-output` to display Cargo-captured notices. An unreachable configured database is a failure. Keep skipped infrastructure separate from passed local logic. Platform lanes that intentionally omit PostgreSQL establish only their edge/transport behavior.

## Immediate regression slice

| Boundary | Required behavior | Evidence location / state |
| --- | --- | --- |
| Hook failure → recovery | Current MCP tool instructions include exact caller key and directory; do not claim Cairn registration succeeded; no removed CLI command. | `crates/cairn/src/hook.rs` regression plus real hook/MCP fault test in corrective branch. |
| Server payload → rendering | Render actual nested budget/sections envelope, cached/reduced/empty replies and budget-free outage; reject malformed or unknown populated sections. | Shared `crates/cairn/src/render.rs` regression; real PostgreSQL/setup/MCP journey. |
| Delivery accounting | Written fallback establishes channel delivery only; selected knowledge is transmitted only when rendering and output succeed. | Real hook transmission regression with malformed and valid selected content. |
| Session selection | Zero/one/many active sessions; explicit own key works with stale siblings; unknown or foreign-project identity cannot select neighbor; reads create no sessions. | `crates/cairnd/src/handlers.rs` component regression in corrective branch. |
| Lifecycle recovery | Start retries are idempotent; interruption resumes; explicit completion cannot reopen; unsupported generic start creates no row. | SQLite/daemon component regression plus real server two-caller journey. |
| Transport timeout | Accepted request with stalled response yields bounded fallback and no fabricated receipt; retry retains caller identity. | Deterministic local socket fault test in corrective branch; supported platform transport variants remain release checks. |
| Installed advice | Shipped skill/contract mentions only current CLI and scopes; owned setup refreshes byte-matching resources, preserves user edits. | Skill/contract check in corrective branch; actual setup/ownership tests must run against candidate. |
| Database prerequisites | Required mode refuses missing/empty URL; optional mode visibly declines; configured URL is used without exposing it. | Shared server harness check and Linux CI strict mode in corrective branch. |

Do not pick newest session, delete other agents' sessions, widen authorization, resurrect local canonical memory, or extend hook deadline merely to hide a failure. A hook may time out before cold daemon startup completes; recovery must work later through the same identity. Any stale-session reaper needs a separate liveness contract, late-event recovery, and tests before backport.

## Product coverage matrix

These are required acceptance checks or explicit remaining gaps, not all implemented by this patch. Reuse existing store, daemon, PostgreSQL and browser harnesses; add no second test platform.

| Area | Positive proof | Negative / boundary / recovery proof | Minimum lane |
| --- | --- | --- | --- |
| Deployment and first use | Fresh DB and temporary home: browser login → project with normalized remote → membership/token → setup → accepted event → inspectable memory → later recall. | Invalid/missing/duplicate remote, nonmember, mismatched account, wrong browser origin, reload/cookie persistence; execute documented steps without direct API provisioning. | Candidate Compose/browser + native CLI; currently blocked by documented routing/remote gaps. |
| Owned integrations | Install actual embedded skill, hooks and MCP; rerun is idempotent. | User edits/conflicting owner preserved; missing resource restored; agent trust/unsupported capability visible; installed executable and advice agree. | Temporary home, actual setup entry point. |
| Identity and lifecycle | Two simultaneous caller keys in one repo; keys across project/worktree; manual MCP path where supported. | Missing/unknown/malformed/foreign identity; repeated start; explicit completion; interruption/restart; never silently choose sibling. | Real SQLite component plus one real hook/MCP journey. |
| Safe capture | Every supported adapter fixture maps to correct key and allowed event; rejection names policy class. | Secrets, paths, oversized/malformed Unicode payloads; raw prompt/output absent from spool/server/logs; no-event work yields no memory. | Adapter corpus + real spool + server ingress. |
| Durable delivery | Queue offline, reconnect, receipt visible; event and command lanes retain identity. | Crash after accept before ack, duplicate/reordered retry, saturation, expired pending item, partial reply, server restart; one canonical effect. | Real edge store + PostgreSQL restart journey. |
| Knowledge and evidence | Supported fact/decision/convention/failure/procedure retains evidence and attribution; reinforce/supersede/verify work. | Conflicts, stale evidence, unsupported inference, refusal, duplicate claim, missing evidence, changed rule revision; no fabricated observation IDs. | Frozen corpus + real consolidation integration. |
| Retrieval and cache | Relevant bounded project/branch/session context with explanation; exact delivery states. | Wrong-account/project/session cache, revoked token, authorization denial invalidates cache, expiry, outages, truncated budget, generated versus confirmed delivery. | Component boundary tests + server retrieval + native hook. |
| Governance and authorization | Member/owner/admin allowed operations; personal privacy and team ratification. | Nonmember/cross-account/disabled account/nonadmin reads and every mutation family refused; body cannot choose owner; denied origin. | Real PostgreSQL API role matrix. |
| Browser | Search/filter/detail, memory curation, sessions, governance, settings, account/token/membership actions. | >25 records, equal sort keys and interleaved writes; loading/error/empty/stale/offline states, recovery actions; desktop/mobile keyboard basics. | Generated contract + production build + live browser. |
| Upgrade and restore | Representative alpha.7 WAL/pending data → candidate; conservation report accounts for every input row; restore repeats safely. | Interrupted import, unsupported/task records remain offline, malformed bundle, duplicate retry, account/server switch, rollback/restore; no scope widening. | Pinned legacy fixture + actual binaries + PostgreSQL. |
| Capacity and responsiveness | Measured hook latency, context budget, cache TTL, spool bounds and large-corpus retrieval. | Failed-login load cannot stall health/retrieval; bounded work under contention; query plan/restart/saturation. | Controlled release-build benchmark; thresholds agreed before verdict. |
| Release identity and platforms | Candidate source/archive/image versions agree; packaged setup/transport on advertised targets. | Stale binary, mismatched artifact/env/Compose tag, missing binary, broken installed command, unsupported automatic adapter. | Candidate artifact checks; repeat smoke against exact published digests. |

Use full journey on primary supported topology/adapter. Use focused tests for adapter mapping and platform transport; avoid full Cartesian product. Generic manual server-session lifecycle is currently unsupported; refused starts must create no rows. Native recovery must use its true agent identity. A packaged target is not automatically journey-verified. Track each advertised support claim separately.

The current project creation API stores a supplied remote unchanged, while native setup compares a normalized remote. The new server journey explicitly normalizes its fixture before setup; it does not prove browser/API provisioning handles raw SCP remotes. N2 must verify and fix that producer boundary.

## Catching failures not yet named

Add a bounded, reproducible lifecycle sequence test using existing dependencies: two identities, start/read/checkpoint/end/restart/retry. Record seed and operation sequence on failure. Invariants: no cross-identity attribution, no read-created session, stable retry identity, explicit completion never resumed by a late capture event, unrelated project unchanged. Keep deterministic examples for known regressions; generated sequences supplement them.

Faults belong at actual seams: accepted-but-stalled socket, server response/refusal, store transaction, process kill. A 1ms timeout on a fast computer is not deterministic fault injection. Every test-owned daemon/server must have cleanup ownership; retain bounded logs on timeout. Test an error instruction by executing its current action, not merely matching its text.

## Usefulness checkpoint

Run a small frozen paired evaluation alongside first-use/operations work; keep broader varied-corpus scoring later. Scenarios: remembered decision, repeated failed approach, useful procedure, stale/conflicting guidance, and benign work requiring no memory. Pin agent/model, repository input, rule revision, prompts and scoring rubric; use independent/blind review where practical. Keep sensitive raw inputs out of durable memory.

Compare task success, repeated investigation, harmful guidance, useful recall, and context overhead with/without Cairn. Publish per-scenario outcomes and uncertainty. Agree minimum acceptable benefit, tolerated harm and consequence of failure before scoring; do not invent percentages before measuring baseline. Failure means fix measured weakness or narrow claim, not add unrelated features.

## Execution order

1. Run focused regressions and affected unit/component suites.
2. Run formatting, Clippy and workspace checks with pinned Rust toolchain.
3. Run required PostgreSQL/API lane with strict prerequisite mode; report absent infrastructure as `NOT RUN` locally.
4. Run web contract/type/build and browser journey on candidate deployment.
5. Rehearse retry/crash/restore and platform transport; retain exact-artifact evidence.
6. Publish only after applicable gates pass; repeat published-artifact smoke.

This plan makes coverage and uncertainty inspectable. It cannot establish that no unknown bug exists.

## Observed corrective-branch checks — 2026-09-27

Environment: macOS arm64, Rust 1.97.1, debug candidate binaries in this worktree; PostgreSQL 17 test container. Source: uncommitted patch on `4cbb2c6`. These results do not validate installed or published binaries.

| Check | Latest evidence |
| --- | --- |
| Affected `cairn`, `cairnd`, `cairn-integrate`, `cairn-store`, `cairn-e2e` library/binary tests | PASS: 327 checks. |
| Real hook/MCP socket fault and transmission accounting | PASS: 2 tests. |
| Real PostgreSQL command delivery / ingress | Earlier clean run PASS: 42 tests. Latest rerun FAIL: command delivery 12 passed, ingress 17 passed / 13 DB connection timeouts after test container stopped. Docker then returned storage I/O errors; current server gate remains unresolved. |
| Real setup/two-caller MCP journey | Earlier PASS: 1 test. Later cleanup/MCP response deadlines added; final runtime rerun NOT RUN because PostgreSQL infrastructure failed. |
| Clippy, warnings denied | PASS after clearing only task-owned incremental cache; initial attempt failed on full disk. |
| Formatting and whitespace | PASS. |
| Browser deployment, upgrade/restore, packaged platforms, usefulness evaluation | NOT RUN in this patch. |

Reproduce with Rust 1.97.1 on `PATH`:

```sh
cargo test --locked -p cairn -p cairnd -p cairn-integrate -p cairn-store -p cairn-e2e --lib --bins
cargo test --locked -p cairn --test session_recovery
cargo clippy --locked -p cairn -p cairnd -p cairn-integrate -p cairn-store -p cairn-e2e --all-targets -- -D warnings
cargo fmt --all -- --check
# Set CAIRN_TEST_DATABASE_URL to a disposable test database before this command.
CAIRN_REQUIRE_DATABASE_TESTS=1 cargo test --locked -p cairn-e2e --test session_recovery_server --test feature005_command_delivery --test feature005_ingest
```
