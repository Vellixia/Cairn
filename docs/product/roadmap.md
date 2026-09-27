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

Current and Next describe recommended order. Unreleased versions and dates remain **TBD**. Requirements live in [PRD](prd.md); detailed checks and observed results live in [verification plan](../engineering/test-plan.md).

### Planning decisions reviewed with Astra

- Preserve full documented release history, including fixes, removals, migration notes, and limits; historical capabilities do not imply current support.
- Move deployment security, delivery bounds, and upgrade/transfer proof into Current gates for the claims being shipped.
- Complete first-use feedback alongside routing/remote fixes; move broader browsing and operator convenience into Next.
- Publish existing numeric limits now; distinguish safety caps from measured performance promises.
- Run a small usefulness comparison early; make future topic assistance, scale, retention, and integration expansion depend on measured need.

Review used alpha.8 tagged source and the corrective worktree on 2026-09-27. Source findings below are not runtime reproductions. New proposals are mapped to existing PRD outcomes; they do not silently amend approved scope. Detailed verification-plan changes should accompany implementation.

---

## Past

Complete inventory of documented alpha-line changes, fixes, removals, migration behavior, and historical limits. Checked items mean shipped implementation recorded for that version; they do not certify every release gate. Historical CLI/task/local-authority features are **not current interfaces**; alpha.8 removals are explicit below.

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

Evidence: [tagged changelog — 0.1.0-alpha.5](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md), [release](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.5), [historical follow-ups](../history/alpha7/intelligence-followups.md), [topic-key evaluation](../../evals/topic-key-effectiveness/RESULTS.md).

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

### Version TBD — Corrective stabilization and safe first use

**Goal:** one pinned candidate supports browser → authorized project → setup → accepted event → inspectable memory → later recall, with honest recovery and release evidence.

Recorded work on `codex/session-recovery-tests`, based on `4cbb2c6`:

- [~] Recover hook failures through current MCP tools with exact caller key/directory; never imply registration succeeded when it did not.
- [~] Render actual server budget/section envelopes, reduced/cached/empty replies, and outages; reject malformed populated sections.
- [~] Isolate simultaneous sessions/projects, make start retries idempotent, resume interrupted sessions, and keep explicitly completed sessions closed.
- [~] Reject unsupported generic session starts without creating local rows; retain native agent identity in recovery.
- [~] Report transmission only after valid rendering/output, and keep selected/generated/transmitted/confirmed states distinct.
- [~] Refresh installed skill/contract guidance to current commands/scopes while preserving user edits.
- [~] Require database prerequisites in mandatory test lanes; optional missing infrastructure reports `NOT RUN`.

These are branch implementation/partial-validation states, not completed release features. See [observed checks](../engineering/test-plan.md#observed-corrective-branch-checks--2026-09-27).

### Fix / Improve

**Evidence labels:** **Source gap** = inspected implementation differs from intended outcome. **Validation gap/risk** = proof missing or failure needs reproduction. **Proposal** = usability/product improvement, not a confirmed defect.

Order below is recommended. N0–N15 remain proposals unless marked In Progress/Blocked. A gate applies when the candidate makes its associated deployment, upgrade, or support claim.

- [!] **Required PostgreSQL gate — blocked in latest recorded evidence.** Latest rerun had DB timeouts after the test container stopped, followed by Docker storage I/O errors. Restore infrastructure and rerun strict suites plus final setup/two-caller journey on exact candidate. Earlier passes do not close this gate.
- [ ] **N0 — Establish canonical candidate source.** Choose development/merge path while preserving existing worktree and corrective changes. **Proof:** recorded clean candidate commit/artifact identities and baseline gates; missing infrastructure is `NOT RUN`. **PRD-09; validation gap.**
- [ ] **N1 — Make documented browser deployment work.** Example stack exposes web/API separately while runtime API origin defaults empty. Choose supported topology and align Compose/environment/operator steps. **Depends on N0 and topology decision. Proof:** pinned images, clean DB, browser login/reload/authenticated read, and unrelated-origin denial without manual origin repair. **PRD-01, PRD-09; source gap.**
- [ ] **N2 — Browser creates setup-ready projects.** Project form omits remote; API stores supplied remote unchanged while setup compares normalized remote. Validate/normalize at producer boundary. **Depends on N1. Proof:** browser remote → membership → token → setup → accepted event → memory → later recall; invalid/missing/duplicate/SCP remotes, mismatch, and nonmember cases have explicit outcomes. **PRD-01; source gap.**
- [ ] **N4 — Validate secure deployment and credential lifecycle now.** Private/loopback API default, HTTPS cookies, exact-origin CORS including PATCH, revocation/disabled-account isolation, and bounded password hashing belong in corrective gates. Bootstrap-only behavior is partly implemented; stale Compose/CLI comments still say password is reapplied on restart. **Depends on N1 topology. Proof:** web password rotation survives restart, environment only bootstraps/recovers missing admin, permitted split-origin requests pass while other origins fail, and failed-login load does not stall unrelated requests. **PRD-05, PRD-09; mixed source/documentation and validation gaps.**
- [ ] **N11 — Make first-use actions explain their outcome.** Password/project/member mutations lack visible failure/pending/success feedback; privacy fetch failure appears as perpetual loading. **Depends on N1/N2 interfaces. Proof:** denied/invalid/offline/duplicate submissions show actionable accessible feedback, values remain recoverable, repeated clicks do not duplicate operations, and successful changes are visible. Extend to related existing forms where inspection finds the same issue. **PRD-01, PRD-07; source gap.**
- [ ] **N10 — Enforce one delivery deadline and test lane fairness.** Drain can wait half the configured deadline, then retrieval waits a full deadline. Event drain errors also skip command drain; shared drain lock spans serial network work. **Depends on N0. Proof:** stalled drain/retrieval/cold start respect one total budget with honest fallback; blocked event endpoint/slow command does not indefinitely prevent another lane/identity progressing; cancellation/restart leaves claims reclaimable. Fix demonstrated coupling without raising hook deadline. **PRD-04, PRD-06; deadline source gap, fairness/cancellation validation risk.**
- [ ] **N12 — Publish actual limits and validate boundaries.** Inventory existing configurable defaults, hard safety caps, retention, overflow behavior, and measured performance separately. **Depends on N0. Proof:** candidate values below agree with source and operator docs; zero/near/full/overflow, expiry, contention, and recovery cases have runnable checks. Agree performance targets before measuring; do not invent guarantees from constants. **PRD-03, PRD-06; documentation/validation gap.**
- [ ] **N5 — Prove upgrade, transfer, and recovery before promising them.** Include pinned alpha.7 WAL/pending data, backup, import, restore, interrupted retry, accept-before-ack, and every-row conservation. Logical export currently reads tables separately without one consistent snapshot and buffers the full bundle. **Depends on N3 artifact identity; design/reproduction can start on N0. Proof:** export during concurrent writes round-trips into clean destination with valid references/conservation; representative bundle sizes complete within declared memory/time limits; credentials and physical disaster recovery are documented separately from logical transfer. Unsupported/task data stays offline, without scope widening. **PRD-04, PRD-08; snapshot source gap and recovery/capacity validation risks.**
- [ ] **N3 — Align all release identities.** Candidate source, archives/images, Compose fallback, env template, package versions, and docs must agree. **Depends on N1/N2. Proof:** deliberate mismatch fails release check; installed CLI/daemon/server identify the tested candidate. **PRD-09; validation gap.**
- [ ] **N8 — Verify installed integrations and advertised platforms.** Setup/repair must install current hooks, MCP, contract, and skill with trust/capability guidance; preserve user edits, unrelated config, and manager ownership. **Depends on N0/N3; primary adapter journey also N1/N2. Proof:** actual packaged fresh setup/rerun/conflict/restart/compaction tests for primary supported agents; adapter mapping and platform transport checks for advertised targets. Generic manual tools and OpenCode declined delivery remain explicit; Intel macOS requires real execution evidence or an honest unverified support label. **PRD-02, PRD-06, PRD-09; validation gap.**
- [ ] **N9 — Tie release gates and documentation to exact candidate.** Tag release verification currently runs Rust gates while web/browser checks live in separate CI jobs; packaged smoke mostly checks version/help. README also names nonexistent `npm run check:api-contract` instead of `npm run api-contract:check`. **Depends on N3 and applicable N4/N5/N8 gates. Proof:** Rust, strict PostgreSQL, web contract/type/build, browser/setup, image build, and installed-artifact checks gate same candidate before publication; retain results/digests and repeat smoke after publication. Version-correct setup/operations/upgrade/removal guides and embedded skill are checked by executing advice, not just searching strings. **PRD-02, PRD-09; workflow/documentation source gaps and artifact validation gap.**
- [ ] **Early usefulness checkpoint.** Freeze a small paired set: remembered decision, repeated failed approach, useful procedure, stale/conflicting guidance, and benign work needing no memory. Agree benefit/harm thresholds, compare with/without Cairn, and use results to adjust priorities. Runs alongside Current/Next; larger N7 corpus stays in Future. **PRD-03, PRD-05, PRD-06; validation gap.**

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

### Version TBD — Complete inspection and everyday operations

- [ ] **N6 — Browse complete memory/session history and explain delivery failures.** Memory starts at 25; sessions API returns latest 100 without cursor or truncation notice, ordered only by start time. Replay failure is not rendered. **Depends on N1. Proof:** traverse beyond 25 memories/100 sessions with stable tie ordering and no duplicate/skip under writes; disclose intentional replay bounds; loading/error/empty states are distinct. Queued/accepted/blocked/stale/unsupported delivery states show tested next actions. **PRD-06, PRD-07; source gaps.**
- [ ] **N13 — Define operational readiness and safe diagnostics.** Existing `/api/health` reports process liveness; admin system health already has pipeline data. Choose the readiness contract required by supported deployment and reuse those diagnostics. **Depends on N1/N4/N12. Proof:** stop DB/consolidator or break delivery and show which check fails, last-report age, affected lane, and supported recovery action; output excludes credentials/raw agent material. **PRD-06, PRD-07, PRD-09; proposal/validation gap.**
- [ ] **N14 — Reduce membership/project administration friction.** Current form expects a raw member UUID and provides minimal selection context. **Depends on N2/N11. Proof:** authorized operator selects intended user/project, understands permissions, recovers from invalid input, and completes membership changes using keyboard/mobile without accidental duplicate or wrong-project action. Use existing account/project lists; no new administration subsystem. **PRD-01, PRD-05, PRD-07; usability proposal.**
- [ ] **N15 — Complete existing knowledge maintenance workflows.** Measure present memory/governance/evidence interfaces before adding controls; confirm reviewers can resolve stale/conflicting knowledge rather than only inspect it. **Depends on N6 and primary usefulness checkpoint. Proof:** review claim and evidence → reinforce/recheck/supersede/retire using supported action → observe correct later retrieval; denied roles cannot perform mutation, uncertainty/history remain visible. **PRD-05, PRD-07; workflow validation and usability proposal.**

Sources: [session API](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/api.rs), [Memory](<https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/web/app/(app)/projects/%5Bid%5D/memory/page.tsx>), [Sessions/replay](<https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/web/app/(app)/projects/%5Bid%5D/sessions/page.tsx>), [Settings](<https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/web/app/(app)/settings/page.tsx>).

Next usability work does not postpone Current security, delivery, migration, or published-support gates.

---

## Future

### Broader evidence of useful memory

- [ ] **N7 — Evaluate varied real work.** **Depends on reproducible N1–N5; smaller checkpoint runs earlier. Proof:** frozen corpus/labels cover fixes, decisions, procedures, stale/conflicting claims, refusals, and work needing no memory. Report precision, useful recall, false positives/refusals, provenance defects, task success, repeated investigation, harm, and context overhead versus no-memory baseline. Pin repository/agent/model/rule versions; report per-scenario outcomes and uncertainty. **PRD-03, PRD-05, PRD-06.**
- [ ] Extend evidence to supported agents and low-cost configurations after primary journey is stable; separate structural capture, semantic capture, and delivery capabilities instead of comparing raw record counts.
- [ ] Fix measured weaknesses or narrow product claims when thresholds are missed. The earlier [acceptance record](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/tests/feature005/acceptance-results.md) repeats one fixture per agent and does not prove broad usefulness.

### Knowledge quality and topic assistance

- [ ] Re-run cross-session/cross-agent topic-key/adoption evaluation on current candidate. Historical [12-of-26 sample](../../evals/topic-key-effectiveness/RESULTS.md) showed inconsistent topic namespaces and varied unprompted recording; it is motivation, not current defect proof.
- [ ] If fragmentation persists, surface existing applicable topics/corroborating members and improve guidance so agents reuse them. **Proof:** fewer missed groupings without false grouping, fabricated evidence, or automatic semantic merging.
- [ ] If stale/conflicting guidance remains harmful after N15, improve evidence freshness/review prioritization through existing governance; attested evidence must still require new supported observation.

### Guided recovery and supported scale

- [ ] Use N13/paired-task findings to improve recovery guidance for expired/revoked credentials, stale cache, saturation, blocked receipts, and interrupted import. Add only actions supported by current APIs and ownership rules.
- [ ] Measure retrieval/consolidation/export/import/spool ceilings on representative large datasets and Linux/macOS/Windows edge hardware; publish supported limits.
- [ ] Improve query/index/batching behavior for measured bottlenecks. Streaming export or changed lane scheduling is conditional on demonstrated ceilings; new vector/graph infrastructure is not a default solution.

### Retention and integration evolution

- [ ] Define retention/deletion policy by data class: accepted events, evidence/knowledge, handoffs, receipts, refused operations, and local diagnostics. Existing traces already have 90-day retention. **Proof:** expiration/deletion cannot fabricate confirmation, break retry identity, widen scope, or silently invalidate surviving evidence; document physical backup coverage.
- [ ] Evaluate improved OpenCode delivery only when its supported stable contract can safely carry context. Keep manual MCP and honest declined capability until real-version integration tests prove more.
- [ ] Revalidate vendor upgrades, manager distribution, trust, compaction, and removal behavior using smallest supported adapter/platform checks; avoid new CLI surface unless existing setup/web/manual ownership cannot cover measured need.

These are conditional product proposals. Versions, staffing, deadlines, performance promises, and expanded support remain uncommitted.

---

## Backlog

### Product and release decisions

- [ ] Confirm primary adoption scenario: proposed small self-hosted team, one operator, Claude Code or Codex CLI.
- [ ] Select browser/API topology before N1; document supported alternatives.
- [ ] Agree benefit/harm and measured performance targets before scoring; retain current safety caps while gathering evidence.
- [ ] Choose successor versus alpha.8 artifact replacement, assign version, and finalize candidate scope. Recommendation: publish a successor for changed artifacts with immutable evidence.
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

Sources: [foundation follow-ups](../history/alpha7/mvp-followups.md), [intelligence follow-ups](../history/alpha7/intelligence-followups.md), [tagged binary harness](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/tests/src/lib.rs).

**Product boundaries:** no task feature/scope, `cairn_task`, local canonical search, entity pull sync, or separate offline authority without a new product contract and migration. No embeddings/vector DB, separate graph DB, broad analytics, or new CLI surface without measured need. No self-registration/self-join or hosted SaaS commitment. Historical references do not automatically become current backlog.

---

## Release Checklist

Applies to corrective candidate and later releases. No gate is checked by this planning pass. Record `PASS`, `FAIL`, or `NOT RUN` with reason, source commit, installed binary paths/versions, artifact digests, platform/DB, and commands in [verification plan](../engineering/test-plan.md). Required pre-publication gates pass before publishing; exact published-artifact smoke follows publication.

- [ ] **Core flow works:** fresh browser → normalized project/remote → membership/token → packaged setup → accepted event → inspectable memory → later recall, plus correct session recovery.
- [ ] **Tests pass:** regression, format/Clippy/workspace, strict PostgreSQL/API, web contract/type/build, and browser checks tied to candidate. **Blocked in latest record:** server gate/final journey rerun.
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
| TBD — corrective candidate | Current — partial validation; server gate blocked | Recovery plus proposed secure first-use, bounded delivery, migration/transfer, adapter/artifact/docs gates. |
| TBD — usability/operations milestone | Planned | Complete browsing, readiness/diagnostics, member selection, knowledge-maintenance workflows. |
| TBD — quality/scale milestones | Conditional proposals | Varied usefulness evidence, topic assistance, measured capacity, retention and supported integration evolution. |

When work ships, record version/commit, artifact identity, validation result, and remaining limitation. Update statuses when scope/dependencies/evidence change. Source existence alone does not complete a proposed outcome.
