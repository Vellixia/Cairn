# Cairn delivery history and feature roadmap

**Updated:** 2026-09-26. **Latest published version:** [v0.1.0-alpha.8](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.8), released 2026-09-25. **This checkout:** `main` at `3741719`, still alpha.7; alpha.8 tag is `4cbb2c6` on `codex/cairn-v1-evidence-consolidation`. The [PRD](PRD.md) describes the published alpha.8 product.

**How to read this checklist:** `[x]` under history means shipped or recorded by the cited tag, not that this alpha.7 checkout contains it or that every older requirement still applies. `[ ]` means work proposed or evidence still needed. Source, release, and operational validation are separate states. No dates or owners are committed for future items.

## What to do next

**Recommendation:** make the published alpha.8 installable and usable end to end before adding features. First product outcome: a new administrator can deploy the example stack, create a setup-ready project in the browser, issue a token, run `cairn setup`, and see the first accepted event and memory without direct API calls or undocumented configuration. Current tagged sources do not support that journey as written.

**Critical path:** align development source with alpha.8 → repair example deployment/browser API routing → allow project creation with repository remote → run first-use test → harden public boundary and rehearse upgrade → complete memory browsing → measure knowledge quality. Work packages and proof below. This order is a recommendation, not a claimed release schedule.

| Order | Work package | Why now | Exit proof |
| --- | --- | --- | --- |
| 0 | Establish alpha.8 development baseline | `main` still builds alpha.7; otherwise fixes may target obsolete code. | Clean, explicit branch/commit strategy; alpha.8 build and docs agree. |
| 1 | Fresh-install and first-use journey | Broken example web API routing and missing project remote block product use. | Browser → project/token → `cairn setup` → accepted event → visible memory, using documented steps. |
| 2 | Safe operations and upgrade | Credentials, network exposure, and migration risk precede wider adoption. | TLS/private API example, restart/password checks, failure and restore rehearsal. |
| 3 | Complete everyday UI | Memory limited to first page; degraded delivery is hard to diagnose. | All matching memory accessible; recovery states and actions visible. |
| 4 | Prove memory quality at scale | Existing repeated fixture cannot establish general usefulness. | Varied corpus, independent scoring, published results, targeted fixes only. |

**First ticket to open:** `A1 — Make the example alpha.8 stack usable from a fresh browser`. Reproduce with tagged images and a new database; record exact request URL and failure. Set one documented browser-to-API topology, fix Compose/web configuration, and add a browser test that logs in and reads a project without manually changing origins. Keep this ticket separate from project creation so failures have one owner and one regression test. Then take `A2` below.

## Version and evidence snapshot

| Question | Current answer | Evidence |
| --- | --- | --- |
| Latest public release? | `v0.1.0-alpha.8`, published 2026-09-25 | [GitHub release](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.8) |
| What does this checkout build? | `0.1.0-alpha.7` on `main` at `3741719` | [local Cargo.toml](../Cargo.toml); checkout commit |
| Where is alpha.8 source? | Signed tag `v0.1.0-alpha.8` at `4cbb2c6`; local/remote `codex/cairn-v1-evidence-consolidation` branch | Git refs and [tagged source](https://github.com/Vellixia/Cairn/tree/v0.1.0-alpha.8) |
| Is alpha.8 the old 001–005 feature set plus one version? | No. It removes tasks, one MCP tool, broad CLI, local canonical knowledge, and many web routes; it keeps and relocates the remaining behavior. | [Release notes](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.8), [architecture](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/architecture.md) |
| Did this document run alpha.8 locally? | No. Status below is tag/source/release evidence, with concrete unverified checks listed separately. | This documentation pass |

### Current status at a glance

| Area | State on published alpha.8 | Remaining proof or gap |
| --- | --- | --- |
| Release/source | Published tag and artifacts; local `main` is behind | Choose canonical development branch and align checkout/documentation. |
| Agent setup | One-command implementation shipped | Settings-created projects lack the remote setup needs; full web-to-agent onboarding cannot complete as written. |
| Capture and delivery | Safe event path, spools, retries, receipts, and server consolidation shipped | Rehearse failure/restart cases with published artifacts and varied agent work. |
| Knowledge and governance | Server-owned project/personal/team memory, evidence, verification, and ratification shipped | Measure usefulness across diverse work; retain privacy/authorization checks. |
| Browser | Consolidated destinations shipped | First-page memory limit; default Compose browser API routing; split-origin PATCH failure. |
| Migration | Backup, conservation, import, and task archive code shipped | Record representative alpha.7 → alpha.8 migration and restore outcome. |
| Release verification | CI gates defined; alpha.8 release published | This document pass did not rerun alpha.8 test or live deployment gates. |

## Past releases: delivered work

### Foundation and integration

- [x] **alpha.1, 2026-08-08:** native local CLI/daemon, Git repository identity, SQLite structured capture, scoped memory, lexical search, session handoffs, bounded briefings, Claude Code integration, opt-in server, initial web UI, privacy boundary. [Historical changelog](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md).
- [x] **alpha.2, 2026-08-09:** operator bootstrap, browser token management, CLI updater and version endpoint, daemon logs, revised web shell, reliability/privacy fixes. [Changelog](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md).
- [x] **alpha.3, 2026-08-12:** Windows x86_64 local transport and packaging support. [Changelog](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md).
- [x] **alpha.4, 2026-08-13 / Feature 002:** Claude Code, Codex CLI, OpenCode, generic MCP and CC Switch support; adapter capability reporting, safe configuration ownership, diagnosis/repair. This was the historical integration interface; alpha.8 folds its human setup into `cairn setup`. [Historical 002 tasks](../specs/002-agent-integration-platform/tasks.md).

### Knowledge and autonomy

- [x] **alpha.5, 2026-08-21 / Feature 003:** canonical subject knowledge, deterministic reconciliation, conflicts, evidence and verification, drift, bounded continuity, multi-device convergence, reusable patterns. Task features from this era are historical and removed in alpha.8. [Historical 003 tasks](../specs/003-project-intelligence/tasks.md).
- [x] **alpha.6:** tag existed, but release was withdrawn before artifacts were published after the container build failed. Do not count as a published version. [Changelog](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md).
- [x] **alpha.7, 2026-09-13 / Features 004–005:** administered accounts and explicit membership; personal and team knowledge; privacy gate and team ratification; safe-event capture; deterministic server consolidation; automatic retrieval; delivery traces and health; migration. [Release](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.7).
- [x] **Feature 005 review evidence:** a dated independent external AI review recorded PASS for the supplied claim forms. Thirty trials repeated one scenario per agent; this is bounded evidence, not a general accuracy guarantee. [Acceptance record](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/tests/feature005/acceptance-results.md).
- [ ] **Historical record cleanup:** Feature 004's archived [200 task boxes](../specs/004-collaborative-global-memory/tasks.md) are unchecked despite alpha.7 shipment. Reconcile only if maintaining that archive; do not infer alpha.8 functionality from its task count. Alpha.8 tagged tree removed `specs/` entirely.

### V1 consolidation

- [x] **alpha.8, published 2026-09-25:** one visible `cairn setup`, hidden hook/MCP adapters, five MCP tools, thin durable edge, canonical server, consolidated web, and legacy import/conservation path. [Release](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.8), [tagged changelog](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md).
- [x] **Removed intentionally:** tasks and task scope across CLI/MCP/daemon/server/database/web/retrieval; local canonical search and knowledge; entity pull feeds; namespace/cutover runtime; old CLI administration and standalone web routes. The migration retains unsupported and task records offline as `removed_feature`. [Architecture](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/architecture.md).

## Current alpha.8 capability checklist

This section answers “what can the published product do now?” Each check cites tagged product evidence. Gaps below are separate.

### Setup, integrations, and local edge

- [x] Setup requires a Git repository, configured remote, valid server token, and existing project membership; it verifies binding and starts daemon. [README](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/README.md).
- [x] Setup installs Cairn-owned hooks, MCP, instructions, and skill resources; rerun repairs matching owned bytes and reports user-edit conflicts without overwriting them. [Integration ownership](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/integrations.md).
- [x] Native adapters exist for Claude Code, Codex CLI, and OpenCode; generic MCP gets manual tools. Automatic delivery must follow actual supported agent capability. [Adapter source](https://github.com/Vellixia/Cairn/tree/v0.1.0-alpha.8/crates/cairn-integrate/src/agents).
- [x] Five MCP tools: context, search, remember, session, handoff. The task tool is gone. [README](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/README.md).
- [x] Daemon owns separate bounded capture and command spools, stable retry identity, receipts, finite-age context cache, session correlation, and integration metadata. [Architecture](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/architecture.md).
- [x] Saturation rejects new capture visibly; server acceptance precedes acknowledgment; retries yield one canonical effect. [README](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/README.md).

### Server knowledge, retrieval, and governance

- [x] PostgreSQL is authoritative for accepted events, sessions, handoffs, project/personal/team knowledge, evidence, relations, verification, governance, retrieval, graph/replay/analytics views, and deduplication. [Architecture](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/architecture.md).
- [x] Server consolidates safe accepted events into attributable durable knowledge through deterministic rules. [alpha.7/8 changelog](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md).
- [x] Search/retrieval is bounded and explainable; project context has priority, and generated/transmitted/received states remain distinct. [Architecture](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/architecture.md).
- [x] Members may operate within authorized projects; personal knowledge is owner-scoped; team proposals need admin ratification; admin surfaces are role-gated. [README](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/README.md).
- [x] Logical export/import, evidence, relations, verification, supersession, and idempotency remain server behavior after V1 consolidation. [Server API source](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/api.rs).

### Web, privacy, migration, release

- [x] Web has project selector, Overview, Memory, Sessions, Governance, Settings, and login; project memory and session detail are subordinate pages. [Web routes](https://github.com/Vellixia/Cairn/tree/v0.1.0-alpha.8/web/app).
- [x] Web exposes memory search/detail/actions, accepted-event replay, governance, credential/project administration, health, and logical import/export by role. [README](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/README.md).
- [x] Edge/server safe-event boundaries exclude raw prompts, transcripts, diffs, command output, credentials, and unbounded payloads from network transfer. [Privacy section](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/README.md).
- [x] Upgrade backs up legacy SQLite including WAL state, creates fresh edge database, preserves only safe pending identities, writes import bundle and conservation report; PostgreSQL archives task rows before live schema removal. [Release](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.8).
- [x] Published binaries target macOS arm64/x86_64, Linux arm64/x86_64, Windows x86_64; server/web images target Linux amd64/arm64. [Release](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.8).
- [x] CI defines Rust, PostgreSQL, web contract/build, and live Playwright gates. Presence of a gate is not evidence this documentation run executed it. [CI workflow](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/.github/workflows/ci.yml).

## Current gaps and decisions needing action

These items were checked against the alpha.8 tag, not copied blindly from the alpha.7 audit.

| Priority | State | Evidence | Completion check |
| --- | --- | --- | --- |
| P0 | Published alpha.8 and `main` alpha.7 disagree | This checkout `Cargo.toml` says alpha.7; GitHub release and tag say alpha.8. | Decide canonical branch, integrate published V1 source without overwriting this dirty worktree, and make source/docs/version provenance agree. |
| P0 | Example environment still pins alpha.4 | [Tagged `deploy/.env.example`](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/deploy/.env.example) says alpha.4 while tagged Compose fallback and packages say alpha.8. | Fix template; fresh documented deployment pulls matching alpha.8 server/web images. |
| P1 | Web project creation cannot make a setup-ready project | Tagged Settings sends only project name; server stores absent `repository_remote` as null; daemon setup requires exact remote match. [Settings](<https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/web/app/(app)/settings/page.tsx>), [server API](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/api.rs). | Web can register a normalized remote under existing authorization; account → project → membership → token → setup succeeds end to end. Until then, API provisioning with `repository_remote` is required. |
| P1 | Example Compose cannot route default browser API calls | Tagged Compose exposes web and API on separate ports, `CAIRN_API_ORIGIN` defaults empty, and Next has no `/api` rewrite. [Compose](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/deploy/docker-compose.yml), [web config](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/web/next.config.mjs). | Ship a working same-origin proxy example or explicit split-origin configuration; verify fresh browser login against example stack. |
| P1 | Environment password-rotation guidance is wrong | Tagged template says restart reapplies admin password, but server returns when any active admin already exists. [Template](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/deploy/.env.example), [auth](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/auth.rs). | Document environment values as bootstrap/recovery inputs; rotate existing password in web. Restart test proves changed password persists. |
| P1 | Split-origin admin edits blocked by CORS | Tagged server allows GET/POST/DELETE/OPTIONS but admin edit route and web client use PATCH. [Server main](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/main.rs) and [web client](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/web/lib/api.ts). | Browser preflight and PATCH succeed for exact allowed origin; unrelated origin remains refused. |
| P1 | Deployment transport boundary needs safe default | Tagged Compose publishes API port on all host interfaces; default web origin is HTTP. [Compose](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/deploy/docker-compose.yml). | Host-proxy example binds API to loopback or keeps it private; remote plaintext path fails; HTTPS cookie attributes match deployment. Configuration risk, not a proven auth bypass. |
| P1 | Authentication load can block async serving | Tagged login calls synchronous Argon2 verification in async handler. [API](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/api.rs). | Bound password work outside async executor; failed-login load does not stall unrelated health/retrieval requests. Availability risk pending measurement. |
| P2 | Memory browsing stops at first page | Tagged Memory page requests 25 and shows truncation notice; project `MemoryPage` has no continuation token. [UI](<https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/web/app/(app)/projects/%5Bid%5D/memory/page.tsx>). | User can visit every matching record with stable paging; test ties and writes between pages. |
| P2 | Broad extraction usefulness unmeasured | [Acceptance record](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/tests/feature005/acceptance-results.md) repeats one fixture scenario per agent. | Frozen varied-workflow corpus reports supported-claim precision, useful recall, false positives, and refusals by rule/agent. |
| P2 | Real upgrade/restore evidence should be retained | Tag includes migration code and tests; this documentation pass did not run a production-like alpha.7 → alpha.8 restore. | Publish conservation report from a representative upgrade, restart, retry, and PostgreSQL restore rehearsal. |

## Ordered execution backlog

All boxes below are open. They are proposed work, not shipped alpha.8 behavior. `A` blocks first-use, `B` protects operation and upgrade, `C` completes routine use, `D` validates value. Do not assign a later release number until its gate passes. The [PRD](PRD.md) defines product behavior; this section defines next actions.

### 0 — establish one source baseline

- [ ] **0.1 Choose canonical alpha.8 development branch.** Decide how tagged `4cbb2c6` reaches `main` or where successor work lands. Preserve this dirty alpha.7 worktree and its documentation changes; do not treat a Cargo version bump as an upgrade. **Done when:** branch/commit and merge path are recorded; `Cargo.toml`, release docs, and build output refer to the same source.
- [ ] **0.2 Run baseline gates before edits.** On that alpha.8 source run Rust checks, PostgreSQL-backed tests, web contract/build, and live browser smoke test; mark missing infrastructure as *not run*, not pass. **Done when:** results, commands, and environment are retained for comparison after fixes.

### A — make first use actually work

**Dependency:** baseline 0.1. This is the next delivery milestone; close A before new product features.

- [ ] **A1 Example stack/browser API routing — first ticket.** Choose and document one supported topology: same-origin reverse proxy or explicit split-origin API. Fix Compose/web example so browser login and authenticated project read reach server without manual URL repair. Set matching cookie, origin, and TLS behavior for chosen topology. **Proof:** clean database + published images + example environment; browser test logs in, reloads, reads project, and rejects an unrelated origin.
- [ ] **A2 Setup-ready project creation.** Add validated repository remote to authorized web project form and request; reuse server's existing remote normalization/authorization contract. Show missing/duplicate/invalid remote errors clearly. **Proof:** admin creates project in browser, grants membership, creates token, and `cairn setup` binds a repository with that remote; mismatched remote and nonmember remain denied. No direct API provisioning step.
- [ ] **A3 Versioned deployment defaults.** Replace alpha.4 example image tag with intended alpha.8 tag and add a small release check for Cargo, web package, Compose fallback, environment template, and published artifact tags. **Proof:** fresh copy of example deploys matching server/web images; mismatch fails the release check. Decide whether to backport this documentation/deployment repair to published alpha.8 artifacts or ship a corrective successor.
- [ ] **A4 First-memory journey.** With A1–A3, run a fresh user through token, setup, owned integration install, first safe capture, server receipt, consolidation, retrieval, and web inspection. Record observed agent capability; do not promise automatic delivery on unsupported adapters. **Proof:** one reproducible end-to-end test plus human steps in README; no raw prompt or transcript crosses boundary. Fix only failures it exposes.

**A gate:** clean install completes above journey using instructions and released artifacts. Browser and CLI tests cover it. Any workaround must be documented and tracked, not counted as completion.

### B — secure and recover operation

**Dependency:** A1 topology choice for B1/B2; A gate for final upgrade verdict. Security defects found during A take priority immediately.

- [ ] **B1 Network and credential boundary.** For host-proxy deployment, keep API private/loopback by default; document container-proxy option and HTTPS termination. Correct environment admin password text: it bootstraps a missing admin, while existing password rotation occurs in web. **Proof:** unauthenticated remote host cannot reach private API port; HTTPS session has expected cookie attributes; web-rotated password survives restart, changed environment password does not silently replace it.
- [ ] **B2 Split-origin admin operation, if supported.** Add PATCH to exact-origin CORS policy and browser-test admin edit with allowed and denied origins. If A1 deliberately removes split-origin as supported topology, document that decision and remove split-origin instructions instead; do not leave a half-working mode. **Proof:** supported admin edit succeeds without opening CORS to arbitrary origins.
- [ ] **B3 Keep login load from stalling service.** Move/bound Argon2 verification outside async request executor using existing runtime facilities. **Proof:** under repeated failed logins, unrelated health and retrieval latency stays within an agreed threshold; limit and measurement recorded. No custom worker system unless measurement demands one.
- [ ] **B4 Upgrade and recovery rehearsal.** Back up representative alpha.7 SQLite with WAL and PostgreSQL, run alpha.8 migration/import, reconcile every row with conservation report, then restore and retry pending operations. Exercise crash after server accept/before edge ack, spool saturation, credential revocation, account/server switch. **Proof:** same operation has one canonical effect after retry; archived tasks remain offline; restore and rejected/pending counts are accounted for. Record exact source and artifact versions.

**B gate:** supported deployment has no known plaintext/exposed-API default; password and authorization behavior match docs; representative upgrade and restore are demonstrated, not inferred from migration code.

### C — finish routine UI and diagnosis

**Dependency:** A gate. Work can proceed alongside B after first-use contract is stable.

- [ ] **C1 Browse every memory.** Add stable server continuation for project and personal memory and a clear “Load more” path. Preserve filters/rank; handle equal sort keys and new writes between pages. **Proof:** a user can reach all >25 matching records without duplicates or skips in a seeded browser test.
- [ ] **C2 Explain failed/degraded journeys.** Show stale context, queued versus accepted capture, blocked delivery, rejected import, and unsupported adapter capability with a next action in the relevant Overview, Memory, Sessions, or Settings view. **Proof:** inject each state and verify label, cause, and action; never display generated context as confirmed agent receipt.
- [ ] **C3 Daily-workflow browser pass.** Test desktop and mobile account/membership/token/setup, first retrieval, knowledge curation, outage and recovery. Keep generated API contract and UI tests in sync. **Proof:** issue list names observed failures and closes each with a browser regression; do not treat a visual tour alone as pass.

**C gate:** authorized users can inspect full knowledge and diagnose common stalls without database access.

### D — measure usefulness before expanding features

**Dependency:** A/B for reproducible capture and delivery. Evaluation design can start earlier.

- [ ] **D1 Freeze varied evaluation corpus.** Include fixes, repeated failures, decisions, procedures, stale evidence, conflicting claims, and benign work that should yield no memory across supported agents. Keep corpus, rule, and reviewer revisions pinned. **Proof:** scenario list and independent labels are reviewable; not thirty repeats of one scenario.
- [ ] **D2 Score what matters.** Publish supported-claim precision, useful recall, false positive/refusal rate, provenance defects, and agent/rule breakdown. Distinguish knowledge creation from later context delivery. **Proof:** another reviewer can rerun scoring and trace each result to evidence.
- [ ] **D3 Fix measured weaknesses only.** Prioritize highest-impact false claims or missing useful memories; rerun privacy, authorization, idempotency, and D2 corpus after each rule change. Measure large-corpus retrieval/consolidation latency and query plans before adding indexes/caches. **Proof:** before/after results improve without a gate regression.

**D gate:** claims about Cairn's usefulness and reliability rest on varied, reproducible evidence rather than one repeated fixture or source-code presence.

## Keep out of scope unless product evidence changes

- [ ] Reintroducing Task, `cairn_task`, or task scope requires a new user problem and migration design. Alpha.8 explicitly removed them.
- [ ] Reintroducing local canonical knowledge, local search authority, or entity pull sync requires a new authority contract. Do not use an offline cache as a substitute.
- [ ] Embeddings, vector database, separate graph database, broad analytics, or new CLI subcommands require measured need and a new PRD decision.

## Maintenance rule

When work ships, update this checklist with release/commit evidence and an observed validation result. Keep “source exists,” “CI passed,” “published,” and “works in a deployment” separate. The prior 001–005 task ledgers are historical archives; do not bulk-check them to make current progress look complete.
