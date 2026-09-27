# Roadmap

> Product: **Cairn**
> Updated: **2026-09-27**
> Published baseline: **[v0.1.0-alpha.8](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.8)** — tag `4cbb2c6`, published 2026-09-25.
> Workspace: [Cargo.toml](../../Cargo.toml) declares **0.1.0-alpha.7**. This checkout is not the alpha.8 release source.

## Status

- [x] Done — shipped in the named release; does not imply every deployment was verified.
- [~] In Progress — implementation or validation has recorded evidence.
- [ ] Planned — proposed work, not an approved release commitment.
- [!] Blocked — cannot finish until the named blocker is resolved.

**Release plan:** `v0.1.0-alpha.9` → `v0.1.0-alpha.10` → `v0.1.0-beta.1` → `v0.1.0-beta.2` → `v0.1.0-rc.1` → `v0.1.0`. These are concrete draft target versions, not published releases or promises of completion. Dates follow exit gates; a failed gate holds its release. Changed artifacts receive a new version; never replace alpha.8 assets or move its tag. Requirements live in [PRD](prd.md); each release must attach its own evidence report using the [published testing guidance](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/testing.md).

### Version and scope contract

| Target version | User outcome | Required scope | Exit gate |
| --- | --- | --- | --- |
| **v0.1.0-alpha.9** | Secure clean deployment, reliable setup, safe recall/recovery | N0–N5, N8–N12; primary-path N13; existing corrective recovery patch | All safety, migration, first-use, installed support and artifact gates below; five-scenario usefulness smoke |
| **v0.1.0-alpha.10** | Complete inspection and everyday administration | N6, remaining N13, N14, N15 | Complete history traversal, actionable failures, authorized maintenance reflected in later recall |
| **v0.1.0-beta.1** | Demonstrably useful memory across varied later work | N7, quality fixes driven by paired evaluation, topic guidance | Frozen 30-pair evaluation meets specified draft benefit/harm targets |
| **v0.1.0-beta.2** | Known operating envelope and predictable data lifecycle | Capacity tests, supported recovery, retention/deletion | Declared load and restore targets pass; deletion preserves evidence/receipt invariants |
| **v0.1.0-rc.1** | Installable release candidate with frozen supported contract | Versioned API/MCP/import compatibility, full support matrix, upgrade rehearsal | All supported combinations pass; 14-day pilot with no unresolved release-blocking defect |
| **v0.1.0** | First stable 0.1 release for solo/small self-hosted teams | Same validated RC scope; final manuals and artifact verification | RC promotion gates and published-artifact smoke; maintenance/compatibility policy published |

**Planning defaults:** solo developers and teams of 1–10, one self-hosted deployment, Claude Code/Codex primary native journeys. Same-origin web `/` and API `/api` is default; split-origin is opt-in with exact-origin tests. Preserve Rust/Axum/PostgreSQL canonical server, SQLite delivery edge, existing Next.js web, five MCP tools and one visible setup command. This is the proposed scope for review, not a new implementation or release approval.

**Architecture boundary through v0.1.0:** finite-age identity-scoped cache stays in memory; daemon-restart cache persistence and model-assisted extraction remain outside planned scope. Remote matching uses authorized project UUID plus normalized Git remote; ambiguous authorized matches fail with actionable guidance. Alpha.9 must disclose current data growth/deletion/backup behavior; broader retention automation lands beta.2.

**Release ownership:** maintainer owns candidate/version/support declarations; implementation owner links each N-item to PRs and checks; release verifier records source SHA, binaries, images, digests, environment and `PASS`/`FAIL`/`NOT RUN`. Required `NOT RUN` blocks publication. N8/N9 and applicable security, migration and delivery gates repeat every release; passing an earlier version never substitutes for candidate evidence.

### Planning decisions reviewed with Astra

- Preserve full documented release history, including fixes, removals, migration notes, and limits; historical capabilities do not imply current support.
- Move deployment security, delivery bounds, and upgrade/transfer proof into Current gates for the claims being shipped.
- Complete first-use feedback alongside routing/remote fixes; move broader browsing and operator convenience into Next.
- Publish existing numeric limits now; distinguish safety caps from measured performance promises.
- Run a small usefulness comparison early; make future topic assistance, scale, retention, and integration expansion depend on measured need.

Review used alpha.8 tagged source and the corrective worktree on 2026-09-27. Source findings below are not runtime reproductions. New proposals are mapped to existing PRD outcomes; they do not silently amend approved scope. Detailed verification-plan changes should accompany implementation.

---

## Past

Inventory of documented alpha-line changes, fixes, removals, migration behavior, and historical limits. Checked items mean shipped implementation recorded for that version; they do not certify every release gate. Historical CLI/task/local-authority features are **not current interfaces**; alpha.8 removals are explicit below.

### v0.1.0-alpha.1 — Local-first memory foundation

- [x] Local daemon (`cairnd`) and SQLite capture structured file, command, test, and error observations; exclude full conversations and raw tool output.
- [x] Project, branch, task, and session memory with session/observation provenance.
- [x] Git-derived repository, branch, commit, and working-tree identity across clones.
- [x] Bounded deterministic context briefings degrade from the bottom rather than truncate.
- [x] Compaction, session-end, and recovery handoffs record completed/remaining work, decisions, failures, tests, and next action.
- [x] Claude Code hooks and MCP registration through historical `cairn connect claude-code`; daemon starts automatically.
- [x] SQLite FTS5 lexical memory search without embeddings or external index service.
- [x] Opt-in server sync through transactional outbox and server-side idempotency keys; exactly one canonical effect on retry.
- [x] Web browsing of projects, sessions, handoffs, tasks, memory, and sync state.
- [x] Structural privacy boundary excludes observation entities from wire payloads; unlinked projects send no outbound request.
- [x] Common secret patterns redacted before persistence.
- [x] macOS arm64/x86_64 and Linux arm64/x86_64 archives include CLI, daemon, server, license, and install guidance.
- [x] `SHA256SUMS`, SPDX SBOM, multi-architecture GHCR server/web images, and build provenance.
- [x] Example Compose stack for server, web, and PostgreSQL.

**Limits and release notes:**

- Windows unsupported at this release; added in alpha.3.
- Alpha APIs, schemas, and wire protocol may change before 1.0.0; sharing requires a self-hosted server.
- Earlier v0.x implementation retired; this release begins a new, incompatible release line.

Evidence: [tagged changelog — 0.1.0-alpha.1](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md), [release](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.1).

### v0.1.0-alpha.2 — Operator, browser, and update hardening

- [x] Environment-configured operator email/password, applied after migrations and before listener binds; setting only one is a startup error.
- [x] Browser API-token creation, review, and revocation; hashed storage and one-time plaintext display.
- [x] `cairn update` discovers eligible releases, checks archive digest, refuses mismatches, and replaces CLI/daemon together.
- [x] `cairn update --check`; prereleases offered only to existing prerelease users.
- [x] `/api/version` reports running version, latest eligible release, and update state with cached server-side lookup.
- [x] Version/update display in sidebar and public sign-in page.
- [x] `cairn auth status` reports saved credential/server without printing the token.
- [x] `cairn daemon logs` reads persisted `cairnd.log` instead of discarding daemon stderr.
- [x] shadcn/ui shell with collapsible sidebar, breadcrumb, route titles, styled 404, loading, empty, and error states.
- [x] Confirmations on irreversible browser actions.
- [x] `cairn status` reports linked server and missing credential.
- [x] Session cookies use `Secure` for HTTPS web origin; logout uses matching attributes without breaking HTTP deployments.
- [x] Idle sessions close after two hours, including orphaned SessionEnd cases that previously caused ambiguity.
- [x] Memory recording propagates ambiguous-session errors rather than creating a throwaway origin; explicit `--session`/MCP session selection.
- [x] Daemon startup preserves an already-served socket; CLI retries bounded handovers.
- [x] Account-menu crash fixed by placing theme label inside required menu group.
- [x] Token copying falls back when async clipboard is unavailable, then selects token if copying fails.
- [x] Mobile sidebar closes after navigation.
- [x] Memory filters have distinct labels; page subtitles wrap; search is debounced, clearable, and reports matched count.

**Limits and release notes:**

- Environment values acted as operator password on restart in this release; alpha.8 intends bootstrap-only semantics. Verify actual candidate behavior before upgrading.

Evidence: [tagged changelog — 0.1.0-alpha.2](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md), [release](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.2).

### v0.1.0-alpha.3 — Windows and privacy fixes

- [x] Windows named-pipe CLI/daemon transport replaces Unix socket on that platform.
- [x] Windows x86_64 release archive and Windows CI test suite.
- [x] Windows updater handles `.exe` and renames running binaries before replacement.
- [x] GitLab token redaction covers `glpat-`, `gloas-`, `glrt-`, `glcbt-`, and `gldt-` families before any write.
- [x] `cairn link` reports existing binding correctly without needing network or credentials.
- [x] CLI stops leaking inherited standard handles into daemon, allowing captured output pipes to reach EOF.

**Limits and release notes:**

- Token file mode is `0600` on Unix; Windows privacy depends on user-profile directory permissions.
- PostgreSQL-backed suites run on Linux CI; Windows/macOS lanes establish their local/transport behavior.

Evidence: [tagged changelog — 0.1.0-alpha.3](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md), [release](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.3).

### v0.1.0-alpha.4 — Agent integration platform

- [x] Native Claude Code, Codex, and OpenCode integration through one canonical seven-event lifecycle.
- [x] Generic MCP onboarding without bespoke client adapter.
- [x] Content-addressed Cairn Skill and rendered agent-contract revisions independent of package version.
- [x] Historical `cairn agents`, `connect`, `doctor`, `repair`, `disconnect`, and `integration` surfaces.
- [x] Integration commands preview changes and identify untouched resources before mutation.
- [x] Integration ownership/migration, including adoption of existing Claude Code setup.
- [x] CC Switch classified as integration manager; manager-owned edits delegated, removals report `manager_action_required`.
- [x] Observed capability levels `FULL`, `MCP_PLUS`, `MCP_ONLY`; full capability earned through real session and withdrawn when vendor version invalidates evidence.
- [x] Cross-agent project/task continuity keeps decisions, failures, procedures, and handoffs independent of agent vendor.
- [x] Source-preserving JSON/JSONC, TOML, and Markdown configuration mutation.
- [x] Hosted Playwright end-to-end CI alongside Linux, macOS, and Windows suites.
- [x] Session close acknowledged once termination is durable; handoff synthesis follows to respect vendor deadline.
- [x] Windows integration suites covered on same terms as other desktop platforms.

**Limits and release notes:**

- Release entry left live vendor-agent evidence, cross-agent onboarding walkthrough, and CC Switch Skill distribution evidence incomplete. Later releases must supply their own proof.

Evidence: [tagged changelog — 0.1.0-alpha.4](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md), [release](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.4).

### v0.1.0-alpha.5 — Knowledge, evidence, and continuity

- [x] `topic_key`/`value_key` canonical subjects; current answers derived from recorded proposals/decisions without overwriting original memories.
- [x] Only normalized identical content auto-reconciles; same-value/different-content claims report corroborating member and require explicit reconciliation.
- [x] Applicable conflicting claims remain visible without clock, identifier, or arrival-order winner.
- [x] Evidence supports files, configuration keys, Git references, and command outcomes; Cairn rechecks evidence.
- [x] System-run checks and agent attestations remain distinct in storage and rendering.
- [x] Evidence changes produce `drifted` state without rewriting/deleting claim.
- [x] Minimum-safe context reserves work state and critical warnings before history.
- [x] Compression-safe checkpoints validate branch, commit, task state, and relevant paths; divergence suppresses obsolete next actions.
- [x] Evidence-aware task criteria, stable identifiers, criteria state/evidence, and derived readiness that never sets task status or treats agent attestation as verification.
- [x] Clock-independent multi-device convergence of proposals, decisions, criteria, and blockers, with clock-reversed corpus twins.
- [x] Mixed-version unsupported work remains blocked without retry/failure until server upgrade, then delivers once.
- [x] Sanitized cross-project patterns retain evidence and no project identity; receiving project sees unverified status.
- [x] Pattern applications count independent projects, refuse circular confirmation, and preserve counterexamples without deleting history.
- [x] Historical `cairn pattern`, `cairn evidence`, `cairn verify`, `cairn memory subject/reconcile/pin`, `cairn task criterion/blocker`, and `cairn doctor --rebuild-derived`.
- [x] `cairn status` reports subject adoption, conflict, needs-recheck, drift, and sync degradation.
- [x] Six-tool MCP surface preserved through actions and additive read-only fields; replay against original-call corpus.
- [x] `GET /api/version` reports schema/capabilities; absent fields honestly mean older peer.
- [x] Agent contract adds specific claim keys, attached evidence, reinforcement of corroborating claim, and pattern outcome including failures.
- [x] OpenCode reports `agent_initiated` continuity rather than unsupported automatic guarantee.
- [x] Additive migration 5 adds intelligence fields/tables without rewriting existing values or inventing subject keys.
- [x] Continuity separates pre-compaction capture from post-compaction delivery; automatic mode additionally requires observed delivery on installation.
- [x] Post-compaction restoration occurs on next session open, using Cairn records rather than vendor string, and is delivered without manual context request.
- [x] Live compactions exercised across three agents; Claude Code/Codex automatic, OpenCode agent-initiated, generic MCP unavailable-automatic.
- [x] Codex hook trust read from authoritative `config.toml` rather than `hooks.json`, fixing permanently conservative reporting.

**Limits and release notes:**

- Legacy subject/value/digest fields remain NULL; prior `superseded_at` approximates `updated_at`, old `stale_at` remains unknown, and old task snapshots remain NULL.
- OpenCode automatic continuity unavailable (#49); conditional pre-compaction probe not actionable (#50). Alpha.8 has a different capability contract; revalidate before carrying issues forward.
- Topic-key consistency/adoption remains a product limitation; historical evaluation did not show cross-agent convergence.
- Historical local large-file size fingerprint misses same-length edits; lexical pattern signals may miss differently worded matches.
- Attested evidence remains `needs_recheck` until another attestation; verifier-kind ordering is a set, and relation basis may differ across machines.
- Historical web lacked local-only patterns/evidence/checkpoints. Alpha.8 moves canonical behavior to server; this historical gap is not a current assertion.
- Some SQLite memory constraints enforced at repository boundary; three sync tests serialized for shared PostgreSQL capacity; optional DB suites could silently skip.
- Historical performance fixture uses one tenth of stated population; full-scale behavior and multi-platform measurements require separate evidence.

Evidence: [tagged changelog — 0.1.0-alpha.5](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md), [release](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.5), [historical follow-ups](https://github.com/Vellixia/Cairn/blob/0af760163ef6c43e3bfee516f9760a88d5b6c8a6/docs/feature-003-followups.md), [topic-key evaluation](../../evals/topic-key-effectiveness/RESULTS.md).

### v0.1.0-alpha.6 — Withdrawn attempt

**Limits and release notes:**

- Tag exists, but container job failed before artifact upload; no published release, archives, or images.
- Cause repaired in alpha.7, carrying same feature content. Tag retained rather than moved/reused; updater reads releases, so does not offer withdrawn tag.

Evidence: [tagged changelog — 0.1.0-alpha.6](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md).

### v0.1.0-alpha.7 — Autonomous and collaborative memory

- [x] Privacy-screened agent hooks spool safe canonical events durably and deliver when server reachable.
- [x] Fire-and-forget hook capture honors deadline and exits successfully rather than delaying agent.
- [x] Server consolidation leases closed sessions and applies eight deterministic rules: test-confirmed fix, persistent failure, established command, repeated procedure, suite identity, decision near change, recorded decision, standing instruction.
- [x] Automatic budgeted project-knowledge delivery on session open, without explicit tool call.
- [x] Semantic material crosses as structured tokens justified by earlier session events; server independently rederives justification, with no prose transfer.
- [x] `cairn status`/`cairn doctor` expose spool depth, oldest undelivered entry, disposition counts, and ordered blocked-reason vocabulary.
- [x] Personal knowledge follows account/devices; team knowledge is member-proposed and administrator-ratified.
- [x] Knowledge domain remains separate from project scope; no `MemoryScope::Global` or rewritten reconciliation semantics.
- [x] Personal/team records exclude project identity, evidence references, observation IDs, and verification.
- [x] Applicability is AND across kinds, OR within kind; no facts means universal.
- [x] Shared `validate_global_content` applies nine rejection classes at every creation path; eight-check promotion gate reuses it.
- [x] Salted machine-local origin digest never crosses wire.
- [x] Team lifecycle proposed/ratified/retired records acting account.
- [x] Server schema `0003`–`0005` and local schema `0007`–`0012`.
- [x] Daemon renders server-selected durable truth and reports transmission rather than becoming authority; unreachable server has explicit status.
- [x] `cairn sync now` reports account, movement, withheld work, and reasons per lane instead of generic `applied 0`.
- [x] Team revision counter preserves commit order instead of transaction timestamp or allocation order.
- [x] Five sync/link authorization holes closed: no self-registration/self-join, membership-scoped discovery, project-scoped tombstones/upserts, operator-created accounts.
- [x] Hook ordinals allocated in accept order before independent handler tasks, eliminating order race.
- [x] Fresh bounded peer-identity probe makes daemon-replacement status survive prior process exit.
- [x] Queued work authored as one account never submits as another.
- [x] Retirement actor survives stale pull pages; lifecycle merge guarded by server ordering, and unrecorded transitions fail.
- [x] Deleted projects keep contributing privacy identities; command and ingest use same gatherer.
- [x] `capture_deadline_exceeded` is journaled by hook and collected by daemon; deadline drops distinguished from content declines.
- [x] Windows live-handle replacement/cleanup faults fixed in fixtures and upgrade paths.
- [x] Store-open errors identify file and exceeded budget rather than generic pool error.
- [x] Server Docker context copies embedded `skills/`; regression asserts every compile-time external path is available.
- [x] Per-capability mixed-version negotiation queues only unsupported work and keeps supported namespaces flowing; drains after server upgrade.
- [x] Alpha.5 local stores remain readable; migrations apply on daemon startup and server migrations on server startup.

**Limits and release notes:**

- Upgrade server before clients recommended. Capture works unlinked, but server-consolidated durable knowledge needs linked server.
- Preexisting knowledge has no invented autonomous provenance.
- Published mechanism evidence is not broad varied-work usefulness evidence.

Evidence: [tagged changelog — 0.1.0-alpha.7](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md), [release](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.7).

### v0.1.0-alpha.8 — V1 reduction and migration

- [x] Human CLI reduced to `cairn setup`; hook/MCP remain hidden machine adapters.
- [x] Exactly five MCP tools: context, search, remember, session, handoff.
- [x] Delivery-only edge retains bounded capture, separate typed event/command spools, retry receipts, session correlation, integration ownership, and finite-age context cache.
- [x] PostgreSQL exclusively owns knowledge, evidence/relations, sessions/handoffs, governance, retrieval, graph, replay, bounded analytics, verification, supersession, and idempotency.
- [x] Web consolidated to Overview, Memory, Sessions, Governance, Settings, under authorized project selector; project/personal memory share scoped view.
- [x] Setup binds existing credential/project membership, starts daemon, verifies connectivity, and owns only installed integration bytes.
- [x] Setup rerun refreshes only matching owned bytes and preserves user edits; no background force repair.
- [x] Tasks, task scope/retrieval/sync/APIs, task identifiers, and `cairn_task` removed.
- [x] Local canonical knowledge/search/ranking, entity synchronization, namespace/cutover runtime, and account/admin daemon proxies removed.
- [x] Obsolete CLI commands and superseded standalone web routes removed; no restored local authority implied by historical milestones.
- [x] Legacy SQLite store remains unchanged; source and WAL preserved with verified backup, then fresh thin edge created.
- [x] Unambiguously safe pending operations retain original identities.
- [x] Versioned idempotent import bundle and conservation report produced.
- [x] Task, ambiguous, unsupported, and local-only records retained offline as `removed_feature`; scope never widened.
- [x] PostgreSQL removed-task rows/dependencies archived before dropping live task schema; conflicting retries rejected.
- [x] Setup credential changes reload into running daemon.
- [x] Event sessions established atomically.
- [x] Authorization denial immediately invalidates matching stale context.
- [x] Server applies forward PostgreSQL migrations at startup.
- [x] Offline bounded capture and finite-age identity-labelled cache; full spool reports rejection and unavailable search stays explicit.
- [x] Settings include logical import/export excluding credentials, role-gated administration, and timestamped/stale health reporting.
- [x] macOS arm64/x86_64, Linux arm64/x86_64, Windows x86_64 archives; amd64/arm64 containers, checksums/provenance.

**Limits and release notes:**

- Known deployment routing, browser remote provisioning, history pagination, UI feedback, and operational proof gaps are tracked below, not marked fixed.
- Upgrade guidance requires PostgreSQL backup first, server upgrade before local agents, then matching `cairn`/`cairnd` from one archive. Explicit setup rerun repairs only owned integration resources; these operator steps require observed candidate proof.
- Source/publication proves shipped implementation; candidate deployment, upgrade/restore, packaged setup, and varied usefulness need their own observed evidence.

Evidence: [tagged changelog — 0.1.0-alpha.8](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md), [release](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.8).

---

## Current

### v0.1.0-alpha.9 — Secure setup and reliable recall

**Goal:** one pinned candidate supports browser → authorized project → setup → accepted event → inspectable memory → later recall, with honest recovery and release evidence.

**Implementation lead:** reported uncommitted recovery patch on `codex/session-recovery-tests`, based on `4cbb2c6`. These statuses describe that patch, not this documentation branch or a committed release candidate:

- [~] Recover hook failures through current MCP tools with exact caller key/directory; never imply registration succeeded when it did not.
- [~] Render actual server budget/section envelopes, reduced/cached/empty replies, and outages; reject malformed populated sections.
- [~] Isolate simultaneous sessions/projects, make start retries idempotent, resume interrupted sessions, and keep explicitly completed sessions closed.
- [~] Reject unsupported generic session starts without creating local rows; retain native agent identity in recovery.
- [~] Report transmission only after valid rendering/output, and keep selected/generated/transmitted/confirmed states distinct.
- [~] Refresh installed skill/contract guidance to current commands/scopes while preserving user edits.
- [~] Require database prerequisites in mandatory test lanes; optional missing infrastructure reports `NOT RUN`.

**Candidate entry (N0):** choose alpha.8-derived source, commit the recovery patch separately, record reviewed SHA, then build matching CLI/daemon/server/web. This documentation branch declares alpha.7 and cannot serve as alpha.9 implementation source. Do not merge unrelated dirty work blindly.

**Reported local evidence, 2026-09-27:** recovery patch had some successful local checks; final PostgreSQL/setup rerun was incomplete after test-container/storage failure. Patch and raw logs are not included in this PR. Treat this as an investigation lead, not a pinned candidate failure or acceptance result. Alpha.9 must produce a fresh evidence report; ordinary alpha.7 workspace tests cannot close this gate.

### Fix / Improve

**Evidence labels:** **Source gap** = inspected implementation differs from intended outcome. **Validation gap/risk** = proof missing or failure needs reproduction. **Proposal** = usability/product improvement, not a confirmed defect.

**Priority:** P0 = security, identity, data integrity, deadline or release-proof blocker; P1 = required first-use/product flow; P2 = conditional improvement. Prioritize P0; prerequisite P1 deployment work may proceed. No release ships with open P0.

**All items below are alpha.9 requirements.** N13 minimum readiness is included here; its richer diagnostics continue in alpha.10. Work may run in parallel after its dependencies pass. Full N6/N14/N15 usability scope belongs to alpha.10; N7 broad evaluation belongs to beta.1.

- [ ] **Required PostgreSQL gate — no current candidate verdict.** Run strict suites and final setup/two-caller journey on the committed alpha.9 candidate with disposable PostgreSQL. Missing infrastructure is `NOT RUN`; an observed candidate failure is `FAIL`. Earlier recovery-patch passes and local infrastructure failures establish neither current success nor a global release blocker.
- [ ] **N0 — P0: Establish canonical candidate source.** Use alpha.8-derived corrective source, commit/review recovery changes, and record candidate SHA while preserving existing worktree changes. **Proof:** recorded clean candidate commit/artifact identities and baseline gates; missing infrastructure is `NOT RUN`. **PRD-09; validation gap.**
- [ ] **N1 — P1: Make documented browser deployment work.** Example stack exposes web/API separately while runtime API origin defaults empty. Ship same-origin reverse-proxy deployment: web at `/`, API at `/api`; keep database/API private by default and align Compose/environment/operator steps. Split-origin remains opt-in, separately tested. **Depends on N0. Proof:** pinned images, clean DB, browser login/reload/authenticated read, and unrelated-origin denial without manual origin repair. **PRD-01, PRD-09; source gap.**
- [ ] **N2 — P1: Browser creates setup-ready projects.** Project form omits remote; API stores supplied remote unchanged while setup compares normalized remote. Validate/normalize at producer boundary. **Depends on N1. Proof:** browser remote → membership → token → setup → accepted event → memory → later recall; invalid/missing/duplicate/SCP remotes, mismatch, and nonmember cases have explicit outcomes. **PRD-01; source gap.**
- [ ] **N4 — P0: Validate secure deployment and credential lifecycle now.** Private/loopback API default, HTTPS cookies, exact-origin CORS including PATCH, revocation/disabled-account isolation, and bounded password hashing belong in corrective gates. Bootstrap-only behavior is partly implemented; stale Compose/CLI comments still say password is reapplied on restart. **Depends on N1 topology. Proof:** web password rotation survives restart, environment only bootstraps/recovers missing admin, permitted split-origin requests pass while other origins fail, and failed-login load does not stall unrelated requests. **PRD-05, PRD-09; mixed source/documentation and validation gaps.**
- [ ] **N11 — P1: Make first-use actions explain their outcome.** Password/project/member mutations lack visible failure/pending/success feedback; privacy fetch failure appears as perpetual loading. **Depends on N1/N2 interfaces. Proof:** denied/invalid/offline/duplicate submissions show actionable accessible feedback, values remain recoverable, repeated clicks do not duplicate operations, and successful changes are visible. Extend to related existing forms where inspection finds the same issue. **PRD-01, PRD-07; source gap.**
- [ ] **N10 — P0: Enforce one delivery deadline and test lane fairness.** Drain can wait half the configured deadline, then retrieval waits a full deadline. Event drain errors also skip command drain; shared drain lock spans serial network work. **Depends on N0. Proof:** stalled drain/retrieval/cold start respect one total budget with honest fallback; blocked event endpoint/slow command does not indefinitely prevent another lane/identity progressing; cancellation/restart leaves claims reclaimable. Fix demonstrated coupling without raising hook deadline. **PRD-04, PRD-06; deadline source gap, fairness/cancellation validation risk.**
- [ ] **N12 — P0: Publish actual limits and validate boundaries.** Inventory existing configurable defaults, hard safety caps, retention, overflow behavior, and measured performance separately. **Depends on N0. Proof:** candidate values below agree with source and operator docs; zero/near/full/overflow, expiry, contention, and recovery cases have runnable checks. Use alpha.9 deadline/cap checks below and beta.2 workload targets; do not invent measured guarantees from constants. **PRD-03, PRD-06; documentation/validation gap.**
- [ ] **N5 — P0: Prove upgrade, transfer, and recovery before promising them.** Include pinned alpha.7 WAL/pending data, backup, import, restore, interrupted retry, accept-before-ack, and every-row conservation. Logical export currently reads tables separately without one consistent snapshot and buffers the full bundle. **Depends on N3 artifact identity; design/reproduction can start on N0. Proof:** export during concurrent writes round-trips into clean destination with valid references/conservation; representative bundle sizes complete within declared memory/time limits; credentials and physical disaster recovery are documented separately from logical transfer. Unsupported/task data stays offline, without scope widening. **PRD-04, PRD-08; snapshot source gap and recovery/capacity validation risks.**
- [ ] **N3 — P0: Align all release identities.** Candidate source, archives/images, Compose fallback, env template, package versions, and docs must agree. **Depends on N0; final installation proof also N1/N2. Proof:** deliberate mismatch fails release check; installed CLI/daemon/server identify the tested candidate. **PRD-09; validation gap.**
- [ ] **N8 — P0: Verify installed integrations and advertised platforms.** Setup/repair must install current hooks, MCP, contract, and skill with trust/capability guidance; preserve user edits, unrelated config, and manager ownership. **Depends on N0/N3; primary adapter journey also N1/N2. Proof:** actual packaged fresh setup/rerun/conflict/restart/compaction tests for primary supported agents; adapter mapping and platform transport checks for advertised targets. Generic manual tools and OpenCode declined delivery remain explicit; Intel macOS requires real execution evidence or an honest unverified support label. **PRD-02, PRD-06, PRD-09; validation gap.**
- [ ] **N9 — P0: Tie release gates and documentation to exact candidate.** Tag release verification currently runs Rust gates while web/browser checks live in separate CI jobs; packaged smoke mostly checks version/help. Published alpha.8 README names nonexistent `npm run check:api-contract` instead of `npm run api-contract:check`; this PR already corrects version-specific advice, but release source must do the same. **Depends on N3 and applicable N4/N5/N8 gates. Proof:** Rust, strict PostgreSQL, web contract/type/build, browser/setup, image build, and installed-artifact checks gate same candidate before publication; retain results/digests and repeat smoke after publication. Version-correct setup/operations/upgrade/removal guides and embedded skill are checked by executing advice, not just searching strings. **PRD-02, PRD-09; workflow/documentation source gaps and artifact validation gap.**
- [ ] **N13 minimum — P0: Readiness and first failure diagnosis.** Separate process liveness from authenticated operator readiness. Server readiness requires reachable migrated database and required worker availability; an unavailable required worker fails readiness. Show worker last progress and consolidation backlog separately; pending eligible work with no progress for two configured worker intervals is a stalled-stage diagnostic. Individual offline edge, refused command or queue backlog does not make healthy server globally unready. Show affected event/command lane and safe recovery action. **Depends on N1/N4/N10. Proof:** stop DB/required worker and readiness fails; stall one edge lane and only its diagnostic changes, without exposing private content. New endpoint is optional; truthful supported deployment readiness is mandatory. **PRD-06, PRD-07, PRD-09.**
- [ ] **Early usefulness checkpoint — five paired scenarios.** Freeze remembered decision, repeated failed approach, useful procedure, stale/conflicting guidance, and benign work needing no memory. **Exit:** expected supported claim recalled in all three positive scenarios; stale/conflicting case shows uncertainty, never unsupported certainty; benign case invents no memory; zero privacy/attribution defects. Record with/without outcomes and overhead. This is a smoke gate, not broad effectiveness proof. **PRD-03, PRD-05, PRD-06.**

### Alpha.9 execution order and exit

1. **Candidate foundation:** N0 + N3; reviewed alpha.8-derived source and matching versions.
2. **Primary journey:** N1 → N2; N4 + N11 protect login, credentials and mutations.
3. **Reliability:** N10 + N12 + minimum N13; single total deadline, isolated delivery, honest status.
4. **Recovery:** N5; snapshot export, alpha.7 WAL/import conservation, restore and retry.
5. **Ship proof:** N8 + N9 and five-scenario checkpoint; all applicable gates run on exact candidate.

- [ ] Three independent fresh-deployment runs complete browser login → setup-ready project → membership/token → packaged setup → accepted memory → later recall without undocumented edits.
- [ ] Draft usability target: repository setup finishes within 5 minutes once deployment/access exist; clean deployment to inspectable accepted memory within 20 minutes on documented supported host, excluding download time. Record operator steps and elapsed time; failing target requires workflow fix or explicit plan revision.
- [ ] Total capture/context path honors configured 250ms/1,500ms default budgets within documented scheduler tolerance; stalled network cannot consume a fresh full budget after earlier wait. Report p95/p99 and each timeout disposition.
- [ ] Zero cross-account/project leakage, duplicate canonical effects, false delivery confirmation or unaccounted source-record loss in declared adversarial corpus.
- [ ] Snapshot export and recovery have explicit size limits and measured peak memory; alpha.9 documents tested ceiling instead of silently claiming arbitrary bundle support.
- [ ] Required DB/browser/installed support gates all `PASS`; no known critical/high security or data-integrity defect. No alpha.10 feature may displace an alpha.9 safety gate.

### Existing source defaults to preserve and check

Values below come from alpha.8 source, not benchmark results or promises that every exposed configuration knob is wired correctly.

| Boundary | Source value | Candidate check |
| --- | --- | --- |
| Context/capture | 3,000-token default context; 250ms capture and 1,500ms context deadlines | Identify effective configuration and measure total observed path, including cold start/fallback. |
| Outage cache | 200 sessions, 64KiB per response, 300s TTL | Expiry, identity isolation, eviction, and auth-denial invalidation. |
| Event spool | 50,000 events / 256MiB undelivered payload bounds | Verify precise overflow/disposition policy and protected lifecycle events; do not equate payload cap with total disk cap. |
| Delivery claims/network | 60s claim lease; 20s HTTP request timeout | Startup reclaim, cancellation, duplicate acceptance, slow endpoints, bounded shared work. |
| Retrieval traces | 90-day retention, sweep batches of 500 | Expired trace deletion and continued evidence/receipt correctness. |

Sources: [configuration](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-core/src/config.rs), [cache/deadline](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairnd/src/deliver.rs), [spool](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-store/src/spool.rs), [delivery lanes](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairnd/src/sync.rs), [trace retention](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/retrieve.rs).

### Evidence for added corrective work

| Finding | Pinned evidence | What is established |
| --- | --- | --- |
| Routing and remote provisioning | [Compose](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/deploy/docker-compose.yml), [browser settings](<https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/web/app/(app)/settings/page.tsx>), [API](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/api.rs) | Default topology/form/normalization gaps; no fresh deployment proof from this review. |
| Password bootstrap versus old advice | [auth](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/auth.rs), [CLI help source](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/main.rs), Compose above | Bootstrap-only implementation exists; help/comments disagree. Restart/load tests remain necessary. |
| Export consistency and size | [logical transfer](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/transfer.rs), browser settings above | Sequential pool reads lack shared snapshot; server/browser materialize full bundle. Actual inconsistent-export incident not reproduced. |
| Release gates and command advice | [release workflow](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/.github/workflows/release.yml), [CI](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/.github/workflows/ci.yml), [README](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/README.md), [web scripts](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/web/package.json) | Tag publication lacks tied web/browser gate; documentation script spelling wrong; version-only smoke insufficient for journey claims. |

---

## Next

### v0.1.0-alpha.10 — Complete history and everyday operations

**Depends on alpha.9. Goal:** members understand what Cairn knows, what happened, and how authorized correction changes future recall.

- [ ] **N6 — P1: Complete memory/session browsing.** Add stable cursor pagination to current APIs/UI; order ties by stable ID. Test 250 memories and 250 sessions, including identical timestamps, inserts and deletes between pages. Every fixture row appears once in stable snapshot traversal, or API explicitly reports snapshot expiry and UI offers restart; no silent skip/duplicate in a claimed same view. Newly inserted rows appear after refresh, and deletion behavior is documented. No silent 25/100-row ceiling. Detail/replay explains truncation and failure.
- [ ] **N13 — P1: Actionable diagnostics.** Reuse existing health/receipts: pending, accepted, retrying, refused, stale cache, unsupported capability, last report time, affected lane. Test expired/revoked token, DB outage, saturation, blocked receipt and daemon restart; each state names safe next action. Readiness from alpha.9 stays mandatory.
- [ ] **N14 — P1: Account/member selection.** Authorized operator chooses existing account through searchable label/identity instead of copying raw UUID. Show intended project and role before grant/removal. Test wrong-project, duplicate, nonmember and disabled-account cases, keyboard-only operation and 390px viewport.
- [ ] **N15 — P1: Knowledge correction from existing detail/governance.** Inspect claim/evidence; reinforce, recheck, supersede or retire through supported action. Explain permissions and pending/error/success. Later retrieval reflects correction; original history remains inspectable. Never grant verified authority from an unsupported click/attestation.
- [ ] Apply accessible loading/empty/error/success/pending feedback across Memory, Sessions, Governance and Settings; keep entered values on retryable failures and prevent repeated submission.

**Exit:** three operators complete membership change, history lookup, failed-delivery diagnosis and knowledge correction without raw-UUID copying or undocumented API calls. All four tasks succeed for each operator; authorization negatives pass. Repeat alpha.9 release/security/recovery/support gates on alpha.10 candidate.

Sources: [session API](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/api.rs), [Memory](<https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/web/app/(app)/projects/%5Bid%5D/memory/page.tsx>), [Sessions/replay](<https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/web/app/(app)/projects/%5Bid%5D/sessions/page.tsx>), [Settings](<https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/web/app/(app)/settings/page.tsx>).

---

## Future

### v0.1.0-beta.1 — Prove useful recall and improve knowledge quality

**Priority P1; depends on alpha.10. Goal:** later work improves across varied projects, with bounded harm. These are draft acceptance targets, not observed results.

- [ ] **N7 — P1: Freeze 30 paired scenarios** across at least three repositories and Claude Code/Codex, with five pairs per class: fixes, decisions/procedures, repeated failures, stale/conflicting guidance, privacy/refusal boundaries, and benign no-memory work. Each scenario runs with/without Cairn from equivalent repository state; counterbalance order and record agent/model/rule versions. Blind-score supported relevance where practical.
- [ ] Prelabel relevant claims, tasks, harm and scoring method before running. **Targets:** useful recall ≥80% of eligible labelled claims; ≥90% of delivered claims relevant/supported; zero privacy leaks, wrong-actor attribution or high-impact harmful advice; ≥20% median reduction in repeated investigation steps on repeat-work scenarios; task-completion rate no lower than control. For repeat-work reduction, score only prelabelled pairs with control investigation count >0; report absolute counts for zero-control cases. Report Cairn-delivered tokens per pair and total agent input tokens where measurable, with median/p95; existing per-request context budget stays enforced. Report counts/denominators and uncertainty; 30 pairs do not establish universal performance.
- [ ] Re-run topic-key consistency across sessions/agents. If fragmentation misses labelled same-subject claims, surface existing applicable topic keys/corroborating members and revise installed guidance. Never automatically merge different semantic claims. **Exit:** at least 20% fewer missed same-subject groupings versus frozen candidate baseline with zero false grouping in scored corpus; if baseline has no misses, no topic feature needed.
- [ ] Fix measured retrieval/extraction weaknesses with current deterministic rules, evidence and query paths; add explicit remembering guidance for decisions hooks cannot observe. Publish no-memory and refusal outcomes alongside successes.
- [ ] Missed benefit targets require implementation fixes or a narrower supported use case, followed by fresh evaluation. Privacy leaks, wrong-actor attribution and high-impact harmful behavior block release until fixed or affected capability is disabled and remaining scope revalidated; documentation changes alone cannot close these gates. No cosmetic feature substitutes for failed usefulness evidence.

### v0.1.0-beta.2 — Supported capacity, retention and recovery

**Priority P1; depends on beta.1. Goal:** small teams operate Cairn within explicit resource and data-lifecycle limits.

- [ ] Benchmark **draft envelope:** 10 provisioned accounts, 10 concurrent active sessions, 20 projects, 100,000 canonical memories and 1,000,000 accepted events. Reference host: 4 vCPU/8GiB RAM, SSD, server/PostgreSQL/web together; native edge tested separately on supported platforms; LAN RTT ≤50ms. Record versions, dataset distribution and retained receipts/traces, not scaled-only fixtures.
- [ ] **Draft targets:** ordinary authenticated list/detail/mutation p95 ≤500ms excluding export/import; usable main page p95 ≤2s; existing capture/context deadlines remain unchanged. Sustained 10 events/second for 30 minutes; once arrivals stop, processing backlog drains within 10 minutes with no duplicate effect or unexplained loss. Failures block capacity claim; optimize measured SQL/index/batching first.
- [ ] Logical export/import of up to 256MiB serialized bundle completes each direction within 10 minutes with ≤1GiB additional process memory on reference host. Rehearse concurrent-write snapshot and interrupted import. Stream/chunk only if current buffering cannot satisfy cap; enforce limit and show safe refusal before resource exhaustion.
- [ ] Rehearse physical PostgreSQL backup/restore independently: **draft RPO ≤24h, RTO ≤60min** within envelope, daily backup schedule and operator-run recovery. Logical transfer excludes credentials and is not physical disaster recovery.
- [ ] Publish per-class retention: keep canonical knowledge/evidence until authorized retirement/deletion, 90-day traces as existing default; introduce explicit event/handoff/receipt cleanup only after dependency policy is defined. Dry-run counts and deletion tests preserve surviving evidence, unresolved retry identity, actor boundaries and backup disclosure. Never silently sweep accepted history merely because it is old.
- [ ] Surface declared limits, actual queue/DB growth and actionable saturation/recovery states through existing diagnostics. Document supported ceiling and failures before increasing caps.

### v0.1.0-rc.1 — Freeze contracts and rehearse release

**Priority P0; depends on beta.2. Goal:** exact supported installation and upgrade can be repeated by someone other than author.

- [ ] Freeze supported API/MCP actions, error envelope, schema/import format and installed contract/skill revisions. Add current/previous supported client-server compatibility checks and documented server-first upgrade order; do not freeze hidden vendor internals.
- [ ] Publish evidence matrix for macOS arm64/x86_64, Linux arm64/x86_64, Windows x86_64 edge; amd64/arm64 server/web images; primary native agents, OpenCode capture/declined automatic delivery and generic manual MCP. Every advertised combination has actual installed proof or is explicitly outside supported matrix.
- [ ] Test fresh install, setup rerun/user edits, daemon restart, session/compaction recovery, credential revoke, upgrade, import/restore and owned-resource removal using candidate archives/images. Chromium desktop/mobile is minimum browser gate; extra browser claims require their own execution.
- [ ] Pilot for **14 consecutive days across three deployments**, at least one solo and one small team; record failures and named owner. Zero unresolved critical/high security or data-integrity defects and no unresolved primary-flow defect at exit. A release-blocking correction starts a new RC version and repeats affected gates/pilot; never replace rc.1 artifacts.
- [ ] Finalize maintainer support/triage procedure, versioned upgrade guide, backup/removal runbooks and known limits. RC may contain bug fixes; new product features return to prerelease planning.

### v0.1.0 — First stable 0.1 release

- [ ] Promote validated RC scope with only version/metadata changes; rebuild and rerun applicable exact-artifact gates before publication and smoke after publication.
- [ ] Publish 0.1 support/compatibility policy: additive API changes within 0.1.x; deprecate before planned breaking changes in 0.2.0; document any unavoidable security exception and migration. Schema upgrades remain forward-only; downgrade uses validated backup restore, not assumed reverse migration.
- [ ] Publish complete onboarding, evidence/support matrix, operating envelope, usefulness results, retained limitations and recovery procedures with artifact digests/source identity.
- [ ] Use **v0.1.1** onward for compatible fixes; larger feature or contract change belongs to **v0.2.0** planning. `v0.1.0` is not a claim of enterprise readiness or product `v1.0.0`.

---

## Backlog

### Product and release decisions

- [ ] Review draft primary audience (solo/1–10-person self-hosted team), default same-origin topology and explicit version sequence before approving implementation scope.
- [ ] Assign implementation/release owners to alpha.9 N-items and attach PR/evidence links; optional split-origin support requires separate tests.
- [ ] Ratify beta.1 scoring rubric and beta.2 reference envelope/targets before running measurements; revise targets transparently rather than after observing scores.
- [ ] Implement immutable alpha.9 successor path; do not replace alpha.8 assets. Emergency extra prereleases receive new numbers, and roadmap is renumbered before publication.
- [ ] Declare upgrade/support matrix and any unverified target honestly; do not silently turn absent infrastructure into acceptance.
- [ ] Define retention/deletion expectations before implementing broader cleanup.

### Historical follow-ups to revalidate

Historical records are evidence of earlier ceilings, not instructions to restore removed systems.

| Earlier item | Current disposition / next proof |
| --- | --- |
| Stale binaries in subset-package E2E runs | Harness still checks existence, not provenance. N8/N9 require explicit build/source identity; fail a deliberately stale-binary run before adding heuristic freshness checks. |
| CLI-spawned daemon contention instrumentation missing | Reproduce on current edge in isolated Linux environment; trace only configuration/status, no payloads. Add diagnostics only if same gap remains. |
| Prompt abandoned-claim recovery lacked direct startup proof | N10 uses current typed spool/claim lifecycle; do not reintroduce obsolete entity outbox. |
| `--max-connections` introduced for test pressure | Document supported operator meaning and resource envelope, or revisit only if actual confusion/capacity problem remains. |
| Local memory CHECK/timestamp/task snapshot caveats | Keep in historical migration/import conservation notes; no local canonical schema rebuild or invented historical timestamps. |
| Large-file size-only fingerprints | Revalidate current server evidence path first; if still applicable, expose weaker verification and test same-length edits. |
| Lexical pattern matching and topic fragmentation | Re-evaluate current rules/workflows before changes; no guessed merging or unsupported pattern guarantees. |
| Historical missing web patterns/evidence/checkpoints | Alpha.8 changed authority and UI; measure current maintenance flow in N15 rather than carrying obsolete absence claim. |
| Historical silent DB skip / scaled perf fixture | Strict DB prerequisite work is in progress; N7/N12 require current full-scale and platform-specific proof. |
| OpenCode #49/#50 and manager evidence gaps | Revalidate against alpha.8 capability contract and tested vendor versions; no inherited automatic-delivery promise. |

Sources: [foundation follow-ups](https://github.com/Vellixia/Cairn/blob/0af760163ef6c43e3bfee516f9760a88d5b6c8a6/docs/feature-001-followups.md), [intelligence follow-ups](https://github.com/Vellixia/Cairn/blob/0af760163ef6c43e3bfee516f9760a88d5b6c8a6/docs/feature-003-followups.md), [tagged binary harness](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/tests/src/lib.rs).

**Product boundaries:** no task feature/scope, `cairn_task`, local canonical search, entity pull sync, or separate offline authority without a new product contract and migration. No embeddings/vector DB, separate graph DB, broad analytics, or new CLI surface without measured need. No self-registration/self-join or hosted SaaS commitment. Historical references do not automatically become current backlog.

---

## Release Checklist

Applies to corrective candidate and later releases. No gate is checked by this planning pass. Attach versioned candidate evidence report to release PR, recording `PASS`, `FAIL`, or `NOT RUN` with reason, source commit, installed binary paths/versions, artifact digests, platform/DB and commands. This checklist defines required gates; it does not contain execution results. Required pre-publication gates pass before publishing; exact published-artifact smoke follows publication.

- [ ] **Core flow works:** fresh browser → normalized project/remote → membership/token → packaged setup → accepted event → inspectable memory → later recall, plus correct session recovery.
- [ ] **Tests pass:** regression, format/Clippy/workspace, strict PostgreSQL/API, web contract/type/build, and browser checks tied to candidate. **Candidate requirement:** strict server gate and final journey rerun; historical local infrastructure trouble is not a current candidate verdict.
- [ ] **No known critical candidate bugs:** resolve critical failures and missing required reruns; reproduction outcomes documented separately from speculative risk.
- [ ] **Security checked:** privacy corpus, authorization role matrix, disabled/revoked/cross-account/project isolation, origins/TLS/cookies, bootstrap/password restart behavior, and hashing responsiveness.
- [ ] **Delivery bounded and durable:** total elapsed deadline, queue fairness, saturation/cancellation/reclaim, outage/expiry/cache isolation, accept-before-ack retry, and one canonical effect.
- [ ] **Migration/transfer checked:** alpha.7 WAL backup/restore, interrupted/idempotent import, concurrent snapshot export, bundle bounds, credential recovery, conservation, no scope widening.
- [ ] **Deployment checked:** supported topology, actual liveness/readiness semantics, matching archive/image/environment versions, safe defaults, and executed operator steps.
- [ ] **Support claims checked:** actual packaged setup/repair/ownership/trust/compaction and platform transports for advertised agent/OS claims; unverified combinations named.
- [ ] **UX checked:** first-use errors/pending/success, destructive-action confirmation, recovery guidance, and desktop/mobile keyboard basics.
- [ ] **Usefulness checkpoint recorded:** pre-agreed rubric/thresholds, with/without outcomes, harm/uncertainty, and claim adjustment. Broad N7 claims require broader evidence.
- [ ] **Documentation updated:** PRD, roadmap, changelog, version-correct setup/operations/upgrade/removal guides, installed skill/contract, limits, support matrix, and known gaps agree.
- [ ] **Publication verified:** exact published archives/images pass smoke; implementation, CI, publication, and observed deployment remain separate evidence.

---

## Releases

| Version | Status | Main Change |
| --- | --- | --- |
| v0.1.0-alpha.1 | Done — published | Local memory, daemon/capture, scoped recall/handoffs, early server/web, release foundation. |
| v0.1.0-alpha.2 | Done — published | Operator bootstrap, tokens, updater/version/logs, browser rebuild, daemon/provenance fixes. |
| v0.1.0-alpha.3 | Done — published | Windows transport/archives/CI/update, GitLab privacy and inherited-handle fixes. |
| v0.1.0-alpha.4 | Done — published | Native/generic/manager integration, ownership, repair, capability reporting, browser CI. |
| v0.1.0-alpha.5 | Done — published | Reconciliation, evidence/drift, continuity, historical criteria/patterns, convergence. |
| v0.1.0-alpha.6 | Withdrawn — not published | Container build failed before artifacts; content/fix carried into alpha.7. |
| v0.1.0-alpha.7 | Done — published | Autonomous capture/consolidation, personal/team memory, authorization/order/privacy fixes. |
| v0.1.0-alpha.8 | Done — published baseline | Thin edge/canonical server, one setup command/five tools, consolidated web, removed tasks/local authority, import/conservation. |
| v0.1.0-alpha.9 | Current target — recovery patch reported in progress; release gates open | Secure first-use, bounded delivery, upgrade/export recovery, primary readiness, installed/artifact proof. |
| v0.1.0-alpha.10 | Planned — after alpha.9 | Complete history, diagnostics, account/member selection and knowledge correction. |
| v0.1.0-beta.1 | Planned — after alpha.10 | Thirty-pair useful-recall evidence and measured topic/knowledge-quality fixes. |
| v0.1.0-beta.2 | Planned — after beta.1 | Declared capacity, performance, retention and physical/logical recovery. |
| v0.1.0-rc.1 | Planned — after beta.2 | Contract/support freeze, exact installed matrix and 14-day pilot. |
| v0.1.0 | Planned — after validated RC | First stable 0.1 release, compatibility policy and operating/support evidence. |

When work ships, record version/commit, artifact identity, validation result, and remaining limitation. Update statuses when scope/dependencies/evidence change. Source existence alone does not complete a proposed outcome.
