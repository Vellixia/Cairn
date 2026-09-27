# Product Requirements Document

> **Product:** Cairn
> **Version:** v0.1.0-alpha.9 target; staged through v0.1.0
> **Status:** Draft
> **Updated:** 2026-09-27

**Evidence baseline:** published [v0.1.0-alpha.8](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.8), tag `4cbb2c6`. This checkout's [Cargo.toml](../../Cargo.toml) declares `0.1.0-alpha.7`; building it does not produce the published V1 interface. Corrective work on `codex/session-recovery-tests` has partial validation, not release acceptance.

This PRD is a proposed product contract reviewed with Astra. It covers intended outcomes, architecture, user stories, requirements, and acceptance. “Published” means source/artifact evidence; “Target” means behavior still requiring candidate proof. Every unchecked item below remains an acceptance obligation, not a claim the feature is absent. [Roadmap](roadmap.md) assigns explicit versions and [release gates](roadmap.md#release-checklist); candidate evidence reports record observed checks and gaps.

`PRD-01`–`PRD-09` retain existing traceability. `US-xxx` and `FR-xxx` below belong to **this PRD revision**, not historical specifications with similarly named IDs. Historical test citations require explicit mapping before they count as proof.

## 1. Overview

**Problem**

Coding agents lose decisions, failed approaches, conventions, and useful procedures between sessions, machines, and compactions. Developers repeat explanations and investigations. Saved material can also be stale, contradictory, irrelevant, or impossible to verify; merely adding more context can make work worse.

**Solution**

Cairn captures supported, screened structured events and explicit durable claims; a canonical server validates and consolidates supported knowledge, preserves provenance and governance, then returns bounded, explainable context to later authorized sessions. A native edge handles durable delivery so capture can outlive the agent. Humans inspect and correct shared state in the web application.

Cairn uses versioned deterministic extraction rules; it cannot reconstruct arbitrary reasoning or guarantee a useful memory from every session. Explicit remembering remains available when a durable decision is not observable through supported events. Neither path bypasses privacy, evidence, or authorization.

**Goal**

- Make later work measurably better: useful decisions/procedures recalled, fewer repeated failed approaches, bounded context overhead.
- Complete one understandable deployment → access → setup → capture → inspect → later recall journey.
- Keep memory attributable, correctable, and honest about conflict, drift, absence, delivery, and outages.
- Preserve one canonical effect under retry and account for data through restart, upgrade, transfer, and restore.
- Keep agent interruption, local resource use, and operational complexity bounded.

**Non-Goal**

- Full transcripts, arbitrary command output, autonomous reconstruction of every decision, or guaranteed knowledge from every session.
- Task domain/scope or `cairn_task`; these were removed in alpha.8.
- Local canonical memory/search, full-state entity pull sync, or separate offline authority.
- Self-registration, self-join, hosted SaaS, or enterprise compliance promises.
- Required embedding/model service, vector database, separate graph database, broker, or microservice split without measured need.
- Automatic semantic merging or a graph-based truth engine. Existing bounded relational graph/replay capabilities are supported interfaces, not excluded merely because they use relations.

**Architecture and technology**

Recommended draft choice: preserve published thin edge and canonical server; improve adoption and reliability within that boundary. Solo developers can administer the same stack. Solo use does not imply an offline authority redesign.

```text
Git repository + supported coding agent
  → owned hooks / five manual MCP tools
  → native cairnd: screening, SQLite spools, receipts, correlation
  → authenticated server: PostgreSQL canonical state and application rules
  → bounded retrieval to later agent session
  → web inspection, governance, and administration
```

| Component | Existing technology | Responsibility and tradeoff |
| --- | --- | --- |
| Native CLI/adapters | Rust, Clap, Git CLI, source-preserving config editors | One human `cairn setup` path; machine hook/MCP adapters. Must preserve user-owned configuration. |
| Edge daemon | Rust/Tokio, SQLite/SQLx, Unix socket or Windows named pipe, HTTP client | Binding, event/command spools, receipts, session correlation, ownership/migration metadata. Queue survives agent exit; queued work is not server-accepted knowledge. |
| Outage context | Edge-owned finite-age cache | Conditional continuity during outage. Alpha.8 runtime cache is in memory; no daemon-restart persistence guarantee. |
| Canonical server | Rust/Tokio, Axum, SQLx, PostgreSQL | Accepted events, sessions/handoffs, knowledge/evidence/relations, consolidation, governance, retrieval, receipts, logical transfer. Fresh canonical recall/search needs reachable authorized server. |
| Human interface | Next.js, React, TypeScript, TanStack Query, Tailwind, existing UI components | Project selector plus Overview, Memory, Sessions, Governance, Settings. Authorization always enforced server-side. |
| Verification/deployment | Cargo checks, existing SQLite/PostgreSQL harnesses, Playwright, native archives, Compose/GHCR | Reuse current infrastructure; tie source, installed artifacts, and observed journeys together. |

Server owns application meaning; hooks, MCP, and web must not implement competing authority rules. Keep event and command lanes separate because their admission, ordering, and refusal rules differ. A single server application is sufficient; graph traversal uses existing relational data.

Deterministic extraction avoids an additional model dependency, cost, and data boundary while making rule behavior reproducible. It does not claim to understand every event or outperform model-assisted extraction. Any future model assistance needs measured benefit and the same validation/governance boundaries.

Draft planning decision: default browser topology is one origin with web at `/` and API at `/api`; split-origin is opt-in only after exact-origin verification. Canonical server authority trades increased setup/network dependence for one shared authorization, evidence, and correction model.

Evidence: tagged [architecture](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/architecture.md), [Rust dependencies](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/Cargo.toml), [web dependencies](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/web/package.json), [runtime cache](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairnd/src/deliver.rs).

---

## 2. Users

### Developer — solo or member of a small self-hosted team

- Need: relevant prior decisions, failures, and procedures at the right session boundary.
- Problem: context resets; manual recording/setup can cost more than the memory helps.
- Goal: resume useful work without repeating investigations or managing memory every task.
- Solo developer may also be operator. Draft primary audience is solo developers and teams of 1–10 on one self-hosted deployment.

### Project member / knowledge reviewer

- Need: inspect scope, origin, evidence, verification, conflict, and retrieval explanation.
- Problem: misleading/stale claims spread when nobody can explain or correct them.
- Goal: use supported governance to repair knowledge and observe its effect on later recall.
- This is a user responsibility, not a new server role; actual roles remain `admin` and `member`, with project membership enforced separately.

### Administrator / deployment operator

- Need: safe deployment, account/membership/token control, limits, diagnostics, and recoverable upgrades.
- Problem: routing, bootstrap, restore, or artifact mismatch can prevent adoption or jeopardize state.
- Goal: operate the supported stack and know what is queued, accepted, refused, stale, or unverified.

### Integration maintainer / generic MCP user

- Need: accurate adapter capabilities, stable caller identity, current installed advice, and bounded failure behavior.
- Problem: vendor lifecycle changes or unsupported generic actions can produce false continuity guarantees.
- Goal: use observed native capabilities or explicit manual tools without guessed identity or unsupported automatic delivery.

### User stories

| ID | Actor and desired outcome | Essential boundary | Trace |
| --- | --- | --- | --- |
| US-001 | Operator deploys and creates authorized setup-ready access. | Wrong origin/nonmember denied; normalized repository remote required. | PRD-01, PRD-05, PRD-09 |
| US-002 | Developer connects repository without damaging existing configuration. | Rerun idempotent; user edits reported and preserved. | PRD-01, PRD-02 |
| US-003 | Agent captures safe work without interrupting developer. | Deadline/refusal/saturation explicit; unsafe raw material not sent. | PRD-03, PRD-06 |
| US-004 | Developer resumes later work with relevant supported context. | No sibling-session selection or invented knowledge; absence/outage distinguished. | PRD-04, PRD-06 |
| US-005 | Member understands why a memory exists and appeared. | Existing evidence and honest verification/delivery labels. | PRD-05, PRD-07 |
| US-006 | Authorized reviewer corrects stale/conflicting knowledge. | History and uncertainty retained; later retrieval reflects recorded action. | PRD-05, PRD-07 |
| US-007 | Developer recovers offline/failed delivery without duplicates. | Stable operation identity, finite eligible cache, one canonical effect. | PRD-04, PRD-06 |
| US-008 | Operator manages credentials/access with visible outcome. | Revoked/disabled/nonmember denied; UI never grants permission. | PRD-05, PRD-07 |
| US-009 | Operator upgrades or transfers data with accountable recovery. | Original/WAL preserved; every record disposition accounted; scope not widened. | PRD-08, PRD-09 |
| US-010 | Developer trusts installed support and recovery advice. | Binary/adapter/skill/source identities agree; unsupported behavior explicit. | PRD-02, PRD-09 |

---

## 3. Core Flow

1. Operator deploys pinned server/web/database artifacts using supported topology and creates administrator access.
2. Operator creates account/project with validated normalized remote and grants required project membership.
3. Developer signs in, completes any required password change, creates a revocable API token, and passes protected credentials to `cairn setup` in matching Git repository.
4. Setup verifies actor and authorized repository match, installs only owned integration resources, starts daemon, and reports activation/conflicts.
5. Supported agent opens its own session. Hooks screen bounded events; edge records admitted work/disposition and delivers asynchronously.
6. Server authenticates and binds identity, accepts immutable operations idempotently, then consolidates only supported claims from justified events.
7. Authorized user inspects resulting session/memory and its evidence/state in web; corrects or explicitly records knowledge when needed.
8. Later supported session receives bounded applicable context with explanation, or explicit reduced/cached/unavailable/no-memory state.
9. Developer uses relevant guidance; paired evaluation measures whether later work improved.

### Failure Flow

1. A hook, request, deployment, credential, remote match, owned-file check, or server dependency fails.
2. System classifies failure at its real boundary; capture/command operations keep identity and disposition. A failed hook does not imply a session was registered.
3. User sees actionable reason and last known evidence: pending/refused/blocked/stale/unavailable, including queue age or cache age where available. No unsafe payload echoed.
4. User recovers through supported action: repair topology/access/remote, renew credential, resolve owned-file conflict and rerun setup, or retry MCP recovery with exact native caller identity.
5. Retried durable operation uses the same identity; server acceptance and output confirmation are recorded only when observed.
6. If safe recovery is unavailable, agent continues without fresh canonical memory. Do not delete another session, guess newest sibling, extend hook deadline, or promote edge copies into authority.

| Failure | Required response and recovery |
| --- | --- |
| No matching remote / ambiguous authorized match | Explain mismatch/ambiguity; operator fixes registered normalized remote or makes authorized selection explicit. No auto-join. |
| Hook timed out before registration | Current tools receive exact caller key/directory; native start may be retried idempotently. Generic unsupported start creates no local row. |
| Server offline | Continue bounded capture; label eligible cached context; search states unavailable. Reconnect drains eligible operations. |
| Token revoked / account disabled / forbidden project | Deny operation; invalidate matching cache after observed authentication/authorization denial. Network silence cannot reveal unseen remote revocation. |
| Spool saturation | Count eligible capture shedding or reject new operation explicitly; protect already admitted lifecycle boundaries. Never call capture lossless. |
| User modified owned bytes | Preserve file and report conflict; explicit resolution then setup rerun. |
| No supported durable fact | Produce no invented knowledge; normal no-memory outcome is not an outage. |
| Import/restore interrupted | Retain original, retry same import/operation identity, display every accepted/rejected/retained/pending/unchanged disposition. |

---

## 4. Requirements

### Authorized repository setup — PRD-01

**Description:** one safe access and setup journey. **Published:** setup requires an existing matching remote/membership/token. **Target gaps:** browser omits remote; normalization and fresh deployment journey need correction/proof. **Roadmap:** N0–N3, N11.

- [ ] FR-001 — Operator can create setup-ready project, account access, membership, and token through supported human interface.
- [ ] FR-002 — System validates/normalizes remote at provisioning boundary and compares authorized candidates without treating clone path as shared identity.
- [ ] FR-003 — Setup requires valid credentials and membership, refuses no/ambiguous matches, and cannot register accounts or grant access.
- [ ] FR-004 — Fresh supported deployment completes browser → setup → accepted event → inspectable memory → later recall without undocumented manual repair.
- [ ] FR-005 — Setup reports credential, repository, activation, ownership, and connectivity failures without secret disclosure.

**Rules**

- Authenticated actor and project membership determine access; discovery lists only authorized projects.
- Project UUID is canonical identity; normalized remote is a matching hint, not permission.
- No self-registration/self-join or inferred membership.

**Acceptance**

- [ ] Given clean DB/home and authorized remote, when operator provisions access and developer runs setup, then correct project binds and later accepted memory is inspectable/recallable.
- [ ] Given invalid/duplicate/SCP remote or multiple matches, when provisioning/setup runs, then validation or explicit ambiguity is actionable.
- [ ] Given wrong account/token/nonmember, when setup runs, then no access is granted and no foreign project state changes.

### Owned integrations and capability honesty — PRD-02

**Description:** install current hooks/MCP/skill safely. **Published:** byte-owned setup and adapters exist. **Target gaps:** installed artifact, vendor, and platform journey evidence. **Roadmap:** N8, N9.

- [ ] FR-006 — Setup installs detected supported integration resources while preserving unrelated user/manager-owned bytes.
- [ ] FR-007 — Rerun repairs/updates only matching Cairn-owned resources and reports user-edit/conflicting-owner state.
- [ ] FR-008 — Capability/trust/version reporting distinguishes configured, observed, unsupported, and declined behavior.
- [ ] FR-009 — Native recovery preserves actual agent identity; generic clients receive only supported manual actions and unsupported lifecycle starts create no rows.
- [ ] FR-010 — Installed contract/skill/advice match executable version; removal guidance names only owned resources.

**Rules**

- Five MCP tools: context, search, remember, session, handoff; tool existence does not promise every lifecycle/action for generic clients.
- Claude Code/Codex native delivery depends on observed installed capability; OpenCode automatic delivery is declined under published beta-surface contract.
- No force overwrite, background repair, invented lifecycle, or automatic guarantee from configuration alone.

**Acceptance**

- [ ] Given fresh setup then rerun, when integration is inspected/executed, then resources are current and rerun is idempotent.
- [ ] Given user-edited or manager-owned config, when setup runs, then conflicting bytes remain intact.
- [ ] Given unsupported agent/version/lifecycle, when invoked, then honest capability/error and supported manual alternative appear.

### Safe capture and supported extraction — PRD-03

**Description:** capture only permissible bounded events and justified knowledge. **Published:** shared safe-event screening and deterministic rules. **Target gaps:** varied corpus and edge failures. **Roadmap:** N10, N12, N7.

- [ ] FR-011 — Edge and server enforce closed event shapes, byte/count/path/token bounds, secret screening, and contract version.
- [ ] FR-012 — Raw prompts, transcripts, diffs, command output, credentials, vendor payloads, and unbounded material are not transferred as safe events.
- [ ] FR-013 — Refusal/drop reasons are counted and actionable without echoing refused content.
- [ ] FR-014 — Versioned extraction creates only claims supported by accepted events; explicit remember also validates scope/evidence/privacy.
- [ ] FR-015 — Capture honors effective deadline and documented overflow policy; benign work without supported durable fact produces no invented knowledge.

**Rules**

- Safe structured command/test/failure fields remain screened and bounded; “no raw output” does not mean all structured command tokens are absent.
- Events are never truncated into a different valid path/content shape.
- Saturation may shed eligible capture rows with disposition accounting; protected admitted boundaries cannot be shed. Rejected admission does not silently consume ordinal or partially commit shedding.
- Counts and candidates are mechanism evidence, not usefulness.

**Acceptance**

- [ ] Given secret/path/oversize/malformed input, when edge/server processes it, then refusal class is recorded and unsafe material is absent from transfer/log/error.
- [ ] Given full spool or deadline exhaustion, when capture occurs, then documented admission/shedding/rejection is visible and agent continues within effective budget.
- [ ] Given no safe durable evidence, when consolidation runs, then no fabricated fact, reason, or observation ID appears.

### Durable delivery and operation identity — PRD-04

**Description:** retry and restart preserve meaning. **Published:** typed spools, receipts, server idempotency. **Target gaps:** exact-candidate failure/cancellation/fairness proof. **Roadmap:** N5, N10.

- [ ] FR-016 — Immutable event/command identity and payload survive retry, restart, and accept-before-ack interruption.
- [ ] FR-017 — Server records idempotency and canonical change transactionally; same operation has one effect, while conflicting payload reuse is refused.
- [ ] FR-018 — Work queued for one account/project/server is never submitted as another or silently reattributed.
- [ ] FR-019 — Queue state distinguishes pending/in-flight/accepted/refused/blocked/retryable failure and supplies supported recovery reasons.
- [ ] FR-020 — Cancellation/restart leaves claims recoverable; one lane/identity failure cannot indefinitely starve unrelated eligible work.

**Rules**

- At-least-once transport with idempotent canonical effect, not exactly-once transport.
- Local admission, server receipt, knowledge creation, and actual context delivery are separate.
- Changing privacy policy or credentials cannot rewrite unresolved operation identity and pretend original acceptance never happened.

**Acceptance**

- [ ] Given server accepted but edge missed acknowledgment, when restarted/retried, then prior receipt and one canonical effect remain.
- [ ] Given account/server switch or changed payload under reused identity, when drained, then incorrect submission/effect is refused without deleting unresolved original work.
- [ ] Given stalled lane or canceled claim, when unrelated work/recovery runs, then measured bounded progress and reclaim preserve identity.

### Knowledge, evidence, and governance — PRD-05

**Description:** authorized claims remain attributable and correctable. **Published:** project/personal/team knowledge, evidence, relations, verification, conflict/drift, and governance. **Target gaps:** full role/workflow proof. **Roadmap:** N4, N15, N7.

- [ ] FR-021 — Server enforces project membership, personal ownership, team lifecycle, and admin permission for every read/mutation family.
- [ ] FR-022 — Knowledge retains kind, scope/applicability, origin, valid topic/value keys where supplied, and supported evidence references.
- [ ] FR-023 — System checks and client attestations remain distinct; verification never claims an observation it did not make.
- [ ] FR-024 — Conflicting/drifted/superseded claims and uncertainty stay visible; clocks/recency do not silently decide truth.
- [ ] FR-025 — Authorized users can inspect, reinforce, recheck, supersede, retire, and resolve through supported actions; subsequent retrieval respects recorded outcome.
- [ ] FR-026 — Personal/team/cross-project reuse obeys domain content screening and applicability; proposals/ratification and privacy never collapse into project scope.
- [ ] FR-027 — Reusable pattern success in another project is historical evidence, not verification here; applied/failed/not-applicable outcomes do not treat suggestion as independent confirmation.

**Rules**

- `admin`/`member` role and project membership are distinct checks; hiding browser control is not authorization.
- Applicability selects relevant already-authorized knowledge; it never grants access.
- Project claims keep provenance. Cross-project personal/team content obeys current sanitization policy; personal ownership privacy is distinct from sharing/sanitization.
- Automatic reconciliation is limited to supported identical normalization; same words/value alone do not justify arbitrary semantic merging.
- Explicit remembering cannot forge evidence IDs or upgrade attestation to system verification.

**Acceptance**

- [ ] Given member/nonmember/disabled/cross-account/nonadmin actors, when reads/mutations occur, then only explicitly permitted operations succeed.
- [ ] Given stale/conflicting claim and supported corrective action, when authorized reviewer acts, then history/state and later retrieval show the recorded correction.
- [ ] Given personal/team/pattern knowledge, when reused, then ownership/applicability/sanitization and trust labels remain correct; unrelated project identity does not leak.

### Bounded, explainable recall and continuity — PRD-06

**Description:** relevant context within one budget, or honest absence. **Published:** server ranking/explanations and finite-age outage cache. **Target gaps:** total deadline, delivery/cache boundary and useful-recall proof. **Roadmap:** N10, N12, N13, N7.

- [ ] FR-028 — Context applies current session, then branch, then project relevance, subject to authorization and supported personal/team applicability.
- [ ] FR-029 — Selection respects effective context budget and reserved truth/warnings; verification/authority/conflict/supersession/pinning outrank recency and ranking decay is explained.
- [ ] FR-030 — Capture/retrieval/drain/render/output share applicable end-to-end deadline; no extra internal timeout silently expands advertised wait.
- [ ] FR-031 — Cached context is finite-age, identity-bound, labelled, and invalidated on observed matching access denial; unavailable search never pretends to search local canonical memory.
- [ ] FR-032 — Generated/selected/transmitted/confirmed states require their own evidence; rendered fallback does not confirm selected memory reached or was understood by model.
- [ ] FR-033 — Reads create no sessions; ambiguous/foreign/missing identity never selects newest sibling; explicit completion cannot be reopened by late capture.
- [ ] FR-034 — Explain why knowledge was selected/excluded and distinguish reduced/cache/no-memory/outage/unsupported outcomes.

**Rules**

- Alpha.8 cache is runtime memory; restart may lose eligible cache without losing durable server knowledge.
- Network failure cannot prove whether remote access was revoked.
- Generic server-session lifecycle remains unsupported until explicit contract change; native recovery retains exact key/directory/agent identity.
- More context is not intrinsically better; repeated prompt-time material must remain bounded and useful.

**Acceptance**

- [ ] Given two identities/worktrees/projects, when context/search/read/recovery runs, then attribution and cache isolation remain correct without session creation.
- [ ] Given stale cache, observed denial, malformed server sections, or unavailable server, when delivery runs, then deadline and honest state hold without fabricated confirmation.
- [ ] Given conflicting/stale/irrelevant/empty corpus, when retrieval runs, then selected/excluded reasons and safe absence are observable.

### Web inspection and actionable UX — PRD-07

**Description:** humans can inspect and recover through everyday interface. **Published:** consolidated screens. **Target gaps:** pagination, silent mutation/replay errors, readiness and maintenance workflows. **Roadmap:** N6, N11, N13–N15.

- [ ] FR-035 — Authorized users browse/filter/search/detail all matching supported memory and session history through stable continuation.
- [ ] FR-036 — Lists use deterministic tie ordering and disclose intentional caps/truncation; concurrent writes do not duplicate/skip within defined paging semantics.
- [ ] FR-037 — Queries and mutations show loading/empty/error/success/disabled/pending states, preserve recoverable input, and prevent accidental duplicate submission.
- [ ] FR-038 — Queued/accepted/blocked/refused/stale/offline/unsupported states show last evidence timestamp and supported next action.
- [ ] FR-039 — Desktop/mobile keyboard, labels, focus, error announcements, destructive confirmations, and role-appropriate controls work.

**Rules**

- Memory first 25 and sessions latest 100 are published source limitations, not intended complete browsing.
- Intentional replay/graph bounds are disclosed; capped view is not “all history.”
- Liveness is not DB/pipeline readiness; stale health is not current healthy state.

**Acceptance**

- [ ] Given >25 memories/>100 sessions with tied sort keys/writes, when browsing continues, then expected records remain reachable under documented paging contract.
- [ ] Given denied/offline/invalid settings mutation or replay error, when user acts, then actionable feedback appears and existing state/input is preserved.
- [ ] Given mobile/keyboard-only or nonadmin user, when navigating/acting, then supported flow is accessible and server still enforces permission.

### Upgrade, logical transfer, and restore — PRD-08

**Description:** changes never silently lose or widen knowledge. **Published:** legacy backup/import/conservation and server archives. **Target gaps:** live snapshot consistency, size bounds, real upgrade/restore. **Roadmap:** N5, N12.

- [ ] FR-040 — Setup preserves legacy SQLite unchanged and verifies backup including WAL before thin-edge replacement.
- [ ] FR-041 — Only unambiguously safe pending operations retain identity; task/local-only/ambiguous/unsupported records remain offline with explicit disposition.
- [ ] FR-042 — Logical import is versioned, authorized, resumable/idempotent, and accounts for every accepted/rejected/retained/pending/unchanged source record.
- [ ] FR-043 — Live logical export uses consistent state and declared size/time bounds; transfer excludes credentials and is distinct from physical disaster recovery.
- [ ] FR-044 — Supported server-first upgrade, interrupted retry, restore, and credential recovery are rehearsed against pinned legacy/candidate artifacts.

**Rules**

- Scope never widens to rescue removed records; no manufactured historical provenance/timestamps.
- Applied migrations remain immutable history; schema changes use forward migrations.
- Logical export currently materializes whole tables without shared snapshot; intended consistency/capacity requires correction/proof.

**Acceptance**

- [ ] Given alpha.7 WAL/pending/removed records, when candidate setup/import runs, then original remains recoverable and conservation accounts for all records.
- [ ] Given interruption/duplicate import/concurrent export writes, when retried/restored, then references/identity remain valid without duplicate canonical effects.
- [ ] Given malformed/oversized/unauthorized bundle, when submitted, then supported refusal is safe and existing destination is preserved.

### Reproducible deployment and release evidence — PRD-09

**Description:** support claims name inputs and outcomes. **Published:** native/container artifacts and Rust/web/browser jobs exist. **Target gaps:** tied exact-candidate gates, version drift, packaged journeys. **Roadmap:** N0, N3, N8, N9.

- [ ] FR-045 — Candidate source, package/CLI/daemon/server versions, images, Compose/environment defaults, embedded resources, and docs agree.
- [ ] FR-046 — Required Rust/PostgreSQL/web/browser/image/artifact gates test same candidate; missing required infrastructure fails rather than silently passes.
- [ ] FR-047 — Advertised adapter/OS support has installed setup/transport/capability evidence; unsupported/unverified combinations are named.
- [ ] FR-048 — Publish checksums/provenance and smoke exact published artifacts; retain independent implementation/CI/publication/deployment evidence.
- [ ] FR-049 — Version-correct setup/operations/upgrade/removal guidance executes successfully; liveness/readiness/diagnostics accurately describe observed service state.

**Rules**

- A moving checkout commit is not release evidence; earlier branch pass cannot close later failed/incomplete gate.
- No guessed successor version or release date. Draft is not Approved/Released.
- Counts/citation coverage and source existence do not prove behavior.

**Acceptance**

- [ ] Given stale/mismatched binary/image/script/docs, when gates run, then mismatch fails explicitly.
- [ ] Given fresh supported stack and actual packaged adapters, when documented journey runs, then source/artifact/observed behavior agree.
- [ ] Given absent required DB/browser/target evidence, when release is evaluated, then `FAIL`/`NOT RUN` and limitation prevent unsupported acceptance.

---

## 5. UI / UX

### Screens

- **Project selector** — authorized project choice; distinguish personal/team domain without changing permissions.
- **Overview** — bounded project state, accepted activity, recency, delivery/retrieval state and age.
- **Memory** — scoped search/filter/detail, evidence/verification/relations/retrieval explanation, and permitted correction.
- **Sessions** — stable history, session identity/status, handoffs, bounded accepted-event replay with clear truncation.
- **Governance** — proposals, conflicts, ratification, retirement, supersession, and maintenance through existing supported actions.
- **Settings** — account/password/tokens, projects/remotes/members, privacy policy, logical transfer, admin users, and server health.
- **Login / access recovery** — sign-in, required password change, and clear denied/expired/disabled state.

### States

- [ ] Loading and pending — bounded feedback, accessible progress, duplicate-submit protection.
- [ ] Empty — no knowledge/work, no matches, or no authorized project explained separately.
- [ ] Error — safe reason, preserved input, and permitted recovery; query errors are not infinite loading.
- [ ] Success — accepted result/change visible; local queue acceptance never presented as canonical application.
- [ ] Disabled — permission, prerequisites, or pending action explained.
- [ ] Offline / cached / stale — last observed time and cache age visible.
- [ ] Refused / blocked / unsupported — policy/capability reason and supported next action.
- [ ] Conflict / drift / needs-recheck — provenance and uncertainty remain visible.
- [ ] Truncated / continuation — capped graph/replay/history never implies complete results.

**Interaction rules:** use existing UI components and native controls; keyboard access, visible focus, explicit labels, announcements, one-time token display/copy, and confirmation for irreversible actions. Explain outcomes to users; internal source/version details appear only when needed to diagnose or choose compatible instructions.

---

## 6. Data

Tables below list the fields needed to understand the product contract, not every storage column. Names/nullability come from alpha.8 migrations/types; **Required** describes stored/wire field, not permission for clients to set it. Targets such as mandatory setup-ready remote are stricter than legacy nullable storage. Full schema remains in [server migrations](https://github.com/Vellixia/Cairn/tree/v0.1.0-alpha.8/crates/cairn-server/migrations), [wire events](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-core/src/event.rs), and [web types](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/contracts/web-api-v1.ts).

### Account and API token

| Entity.field | Type | Required |
| --- | --- | --- |
| `users.id` | UUID | Yes |
| `users.email`, `display_name`, `password_hash` | text | Yes; password hash is server-private |
| `users.role` | admin / member | Yes |
| `users.status` | active / disabled | Yes |
| `users.must_change_password` | boolean | Yes |
| `users.created_at` | datetime | Yes |
| `api_tokens.id`, `user_id` | UUID | Yes |
| `api_tokens.name`, `token_hash` | text | Yes; hash is server-private |
| `api_tokens.created_at` | datetime | Yes |
| `api_tokens.last_used_at`, `revoked_at`, `expires_at` | datetime | No |

**Relations:** Account → API tokens and web sessions; Account ↔ Project through membership.

**Constraints:** unique email/token hash; credential determines actor; disabled/expired/revoked authentication denied. Plaintext tokens/temporary passwords returned once, never available through later reads. Browser sessions use hashed token and expiry; API output never includes password/token hashes.

### Project and membership

| Entity.field | Type | Required |
| --- | --- | --- |
| `projects.id` | UUID | Yes |
| `projects.name` | text | Yes |
| `projects.repository_remote` | text | Nullable legacy field; required target for setup-ready project |
| `projects.created_at`, `updated_at` | datetime | Yes |
| `projects.deleted_at` | datetime | No |
| `project_members.project_id`, `user_id` | UUID | Yes |
| `project_members.added_by_user_id` | UUID | Nullable for older/bootstrap records |
| `project_members.created_at` | datetime | Yes |

**Relations:** Project → sessions/memories; membership → existing Project and Account.

**Constraints:** membership composite key `(project_id, user_id)`; remote indexed but currently not a uniqueness guarantee. New normalization/duplicate-ambiguity policy is a target. Creator receives membership atomically with creation; explicit grant names someone other than caller. Clone path/remote never grants authority.

### Session and handoff

| Entity.field | Type | Required |
| --- | --- | --- |
| `sessions.id`, `project_id` | UUID | Yes |
| `sessions.user_id` | UUID | Nullable legacy schema; active ingest binds authenticated actor |
| `sessions.agent`, `branch` | text | Yes |
| `sessions.commit_sha`, `end_reason` | text | No |
| `sessions.status` | active / completed / interrupted | Yes |
| `sessions.started_at` | datetime | Yes |
| `sessions.ended_at` | datetime | No |
| `handoffs.id`, `project_id`, `session_id` | UUID | Yes |
| `handoffs.trigger` | pre_compact / session_end / recovered | Yes |
| Handoff derived progress/tests/next-step fields | bounded typed fields | Per supported handoff contract |
| `handoffs.created_at` | datetime | Yes |

**Relations:** session → one project; handoff → session/project; safe events and knowledge provenance reference session.

**Constraints:** vendor caller key/worktree path remain local correlation metadata; synced session UUID crosses wire. No task association in live V1. Reads do not create sessions; terminal/interrupted recovery semantics preserve identity and history. Handoff fields remain derived supported material, not raw transcripts.

### Accepted safe event

| Wire field | Type | Required |
| --- | --- | --- |
| `event_id` | UUID derived from session/sequence | Yes |
| `contract_version` | integer | Yes |
| `session_id` | synced UUID | Yes |
| `session_seq` | unsigned integer | Yes |
| `kind`, `agent` | closed vocabulary | Yes |
| `occurred_at` | datetime | Yes |
| `vendor_event` | screened bounded token | No |
| `content` | closed typed union | Required for kinds carrying content |

**Relations:** accepted event → authenticated session; stored project/account derived from that session/credential.

**Constraints:** unique event ID and session sequence; consistent derived identity/content on retry; kind/content must agree. No account/project/caller-key field on individual safe-event wire envelope. Batch session binding may name project UUID but membership must be verified. Safe repository-relative fields may exist; raw/absolute paths and arbitrary output are not authorized payloads.

### Project knowledge, evidence, and relations

| Entity.field | Type | Required |
| --- | --- | --- |
| `memories.id`, `project_id`, `origin_session_id` | UUID | Yes in canonical stored record |
| `memories.type` | fact / decision / convention / failure / procedure | Yes |
| `memories.scope` | project / branch / session | Yes |
| `memories.scope_key`, `content` | text | Yes |
| `memories.state` | active / stale / superseded | Yes; other derived views may expose conflict/drift |
| `memories.topic_key`, `value_key` | bounded normalized text | No |
| `memories.created_at`, `updated_at` | datetime | Yes |
| `memories.superseded_by_id`, `deleted_at` | UUID / datetime | No |
| `memory_relations.from_memory_id`, `to_memory_id` | UUID | Yes |
| `memory_relations.kind` | reinforces / duplicates / supersedes / conflicts_with / narrows / not_applicable_to | Yes |

**Relations:** knowledge → authorized Project and origin Session; evidence/source-event records support knowledge; relation → compatible endpoints.

**Constraints:** scope matches applicable key; evidence references exist and remain within allowed boundary; relations cannot authorize a foreign project. Verification, counts, attribution, and canonical state are server-derived, not trusted client fields. Topic/value identity must state supported claim; legacy NULL keys are not guessed. Supersession and conflict preserve history.

### Personal/team knowledge and reusable patterns

| Entity.field | Type | Required |
| --- | --- | --- |
| `personal_knowledge.id`, `owner_user_id` | UUID | Yes |
| `personal_knowledge.knowledge_type`, `content` | supported kind / screened text | Yes |
| `personal_knowledge.topic_key`, `value_key` | bounded text | No; value requires topic |
| Personal applicability entries | language/tool kind + value | Optional collection |
| `team_knowledge.id`, `knowledge_type`, `content` | UUID / kind / screened text | Yes |
| `team_knowledge.state` | proposed / authoritative / retired | Yes |
| Team lifecycle actor/time fields | UUID / datetime | As required by state |
| `shared_patterns.pattern_id`, `owner_user_id` | UUID | Yes |
| Pattern title/problem/root-cause/approach | screened text | Per supported promotion contract |
| Pattern constraints/applicability | bounded collections | Per supported promotion contract |
| `shared_patterns.trust`, `content_key` | sanitized / content identity | Yes |

**Relations:** personal/pattern → authenticated owner; team → recorded proposal/ratification/retirement actors.

**Constraints:** these domains do not gain a project ID merely because created from project work. Content screening and applicability are separate from ownership authorization. Team state changes obey admin governance; pattern identity unique within owner/content key. Reuse does not manufacture local verification.

### Delivery, retrieval, and transfer records

| Record / field | Type | Required |
| --- | --- | --- |
| Edge event/command identity, immutable payload, queue state | UUID / typed payload / state | Yes |
| Edge account/project/server binding and receipt | identity / accepted result | Binding required; receipt only after observation |
| `applied_commands.account_id`, `command_id` | UUID | Yes; composite key |
| `applied_commands.result_id`, `applied_at` | UUID / datetime | Yes |
| `retrieval_traces.trace_id`, session/actor/selection metadata | UUID / scoped metadata | Required by retrieval contract |
| Trace delivery/acknowledgment state and failure | closed states / reason | Per actual observed stage |
| Logical bundle `format`, `version`, `bundle_id`, `exported_at`, `records` | string / integer / UUID / datetime / collection | Yes |
| Logical import identity/report and record disposition | UUID / structured result | Yes |
| Legacy removed-feature archive payload/counts/version | bounded transfer records | Required when retaining removed data |

**Constraints:** no fake receipt/confirmation, no unresolved identity rewrite; at-least-once retry uses original identity. Logical import/export never includes authentication secrets. Snapshot consistency and export size safety remain targets. Edge queue metadata is not a local canonical knowledge replica.

---

## 7. API

Published routes below come from [server operation registry](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/api.rs), [command handlers](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/commands.rs), and [generated web contract](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/contracts/web-api-v1.ts). Examples use fictional values; output projections are labelled. Runtime schema and generated types remain authority for full wire shape. Future pagination/validation changes need synchronized API/client tests, not invented undocumented routes.

### Sign in and change password

`POST /api/auth/login`

**Input**

```json
{"email":"developer@example.test","password":"example-password"}
```

**Output**

```json
{"id":"11111111-1111-4111-8111-111111111111"}
```

Successful response sets HttpOnly browser session cookie. `GET /api/auth/me` returns actor/role/status; logout clears cookie.

`POST /api/auth/password` accepts **only** current published input `{"new_password":"replacement-password"}` and returns `{"changed":true}`. Current authenticated account is actor; a must-change account may reach password-change route before normal settled routes. This PRD does not invent a required `current_password` field absent from source.

**Errors:** malformed/invalid input `400`; bad/absent authentication `401`; settled/role restriction per route `403`. Cookie/origin/restart behavior requires candidate verification.

### Create token and project

`POST /api/tokens`

**Input**

```json
{"name":"development-machine","expires_at":null}
```

**Output**

```json
{"id":"22222222-2222-4222-8222-222222222222","name":"development-machine","token":"example-token-not-a-credential"}
```

Token is returned once. `GET /api/tokens` lists metadata; `DELETE /api/tokens/{id}` revokes owned token.

`POST /api/projects`

**Input**

```json
{"name":"Example repository","repository_remote":"github.com/example/repository"}
```

**Output**

```json
{"id":"33333333-3333-4333-8333-333333333333","name":"Example repository"}
```

Published handler creates project and creator membership in one transaction for a settled authenticated user; it does not currently normalize supplied remote. Above value is already in setup comparison form. Target FR-002 makes supported raw remote forms validated/normalized before setup.

**Errors:** authentication/settled access checks `401`/`403`; target remote/name validation must return explicit invalid/ambiguity outcome. Do not claim duplicate remote is already database-unique.

### Grant or remove membership

`POST /api/projects/{id}/members`

**Input**

```json
{"user_id":"44444444-4444-4444-8444-444444444444"}
```

**Output** — success `201`

```json
{"project_id":"33333333-3333-4333-8333-333333333333","user_id":"44444444-4444-4444-8444-444444444444","added_by_user_id":"11111111-1111-4111-8111-111111111111"}
```

Existing member or admin can grant another account; self-grant is refused. `DELETE /api/projects/{id}/members` takes same `user_id` body and returns removal result. Account creation is separate admin `POST /api/admin/users` with email/display name and one-time temporary password response.

**Errors:** `400` invalid input, `401` unauthenticated, `403` unauthorized/self-grant, `404` missing account/project, `409 already_member` or `schema_too_old` where applicable. Route permissions are not universally “admin only.”

### Safe event ingress and durable commands

`POST /api/events/batch`

**Input:** typed `contract_version`, session binding collection, and `SafeCanonicalEvent` collection. Native adapter constructs event IDs from session/sequence; clients must not improvise IDs or add raw prompt/output fields. Up to 256 events / 1MiB batch body, with event/content limits in section 10.

**Output** — per-event result projection:

```json
{"results":[{"event_id":"55555555-5555-4555-8555-555555555555","status":"accepted"}]}
```

Statuses include accepted/duplicate/rejected and supported reason where relevant. IDs here are shape illustration, not a hand-crafted valid event identity.

`POST /api/commands`

**Input**

```json
{
  "command_id":"66666666-6666-4666-8666-666666666666",
  "kind":"remember",
  "project_id":"33333333-3333-4333-8333-333333333333",
  "session_id":"77777777-7777-4777-8777-777777777777",
  "payload":{"type":"decision","scope":"project","content":"Use PostgreSQL as canonical authority."}
}
```

**Output** — remember result:

```json
{"id":"88888888-8888-4888-8888-888888888888","applied":"accepted"}
```

Envelope requires command ID/kind and object payload; optional target/project/session fields depend on action. Direct memory routes and queued commands use shared server handlers; command result shapes vary by action. Actor/verification authority cannot be set in payload.

**Errors:** invalid/unsafe contract or body `400`; authentication `401`; foreign membership/session `403`; conflicts/deferred unsupported kind have route-specific status/code. Adapter must preserve retryable versus terminal reason, including supported `409 unsupported_kind`, rather than treat every non-2xx as permanent deletion.

### Retrieve bounded context

`POST /api/retrieve`

**Input**

```json
{"session_id":"77777777-7777-4777-8777-777777777777","trigger":"session_open","budget_tokens":1200,"open_trigger":"startup"}
```

**Output** — field projection, not full response:

```json
{"trace_id":"99999999-9999-4999-8999-999999999999","trigger":"session_open","served_from_cache":false,"budget":{"tokens":1200,"spent":0,"reserved_for_level0":480},"sections":{}}
```

Full response includes delivery point, restoration/degradation state, budget, and per-item reference/content/selection/rank/cost/remaining budget. Server derives account/project from session and clamps caller budget; server never reports edge cache usage. See [retrieval types and implementation](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/retrieve.rs).

`POST /api/retrieval-traces/{trace_id}/transmission` reports supported transmitted/failed outcome; it is not proof model understood or used memory.

**Errors:** bad trigger/budget/input `400`, absent authentication `401`, wrong session/project authorization `403`, missing record `404`, operational failure with safe error/degraded handling. No local canonical-search fallback.

### Inspect, govern, transfer, and diagnose

| Action | Published method/path | Input / result |
| --- | --- | --- |
| Memory search/detail | `GET /api/projects/{id}/memories`; `GET /api/memories/{id}` | Supported query filters/limit; memory page/detail with explanation/evidence. Stable complete paging is target. |
| Session/handoff/replay | `GET /api/projects/{id}/sessions`; `GET /api/sessions/{id}/handoff`; `GET /api/projects/{id}/replay` | Session list, handoff, bounded read-only event metadata. Sessions currently capped at 100. |
| Memory mutations | `POST /api/memories/{id}/supersede`, `/reinforce`, `/pin`, `/forget` | Action-specific intent/result; membership/actor/evidence checks. |
| Relations | `POST /api/projects/{id}/memory-relations` | Compatible endpoints/kind; no foreign project authority. |
| Personal/team/patterns | `GET/POST /api/personal/knowledge`, `/api/team/knowledge`, `/api/patterns` | Domain-specific creation/list; sanitization, ownership, applicability enforced. |
| Team governance | `POST /api/team/{id}/ratify`; `/retire` | Admin state transition, recorded actor, conflict on invalid lifecycle. |
| Logical export | `GET /api/admin/logical-export` | Admin bundle: `format="cairn-logical"`, version 1, ID/time, records; excludes credentials. |
| Logical import | `POST /api/admin/logical-import` | `{"import_id":"…","bundle":{…}}`; versioned report and record dispositions. |
| Privacy policy | `GET /api/privacy-policy` | Immutable running-build bounds/refusal policy; not editable form. |
| Liveness / readiness data | `GET /api/health`; `GET /api/system/health` | Public current liveness `{"ok":true}`; admin pipeline health data. Liveness does not certify DB/consolidation readiness. |
| Version | `GET /api/version` | Current/eligible latest/update state; not declaration installed adapters are working. |

### Error contract

Application errors use [error envelope](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/error.rs):

```json
{"error":{"code":"invalid_request","message":"Safe actionable explanation"}}
```

Additional supported detail fields may accompany code/message. Common application constructors: `400 invalid_request`, `401 unauthorized`, `403 forbidden`, `404 not_found`, `500 internal`; handlers add specific conflicts such as `409 already_member`. Framework body/JSON failures must also be checked for target-consistent safe handling; not every route returns every listed status.

**Target gap:** current SQLx conversion can put raw database error text in `500` message. Public errors must become safe, with bounded diagnostics retained privately and no refused content/credentials echoed. Do not claim current error paths already satisfy that acceptance.

---

## 8. Security & Reliability

All are candidate obligations; source implementation is not completed security review.

- [ ] **Authentication:** HttpOnly cookie/bearer validation, expiry/revocation, disabled accounts, required password change, one-time secret display, hashed credential storage.
- [ ] **Authorization:** project/personal/team/admin role matrix on every read/mutation; request body cannot choose actor/owner or stronger verification.
- [ ] **Server-side validation:** event shape/version/bounds, remote normalization, payload/schema validation, compatible relations, valid evidence, logical-bundle admission.
- [ ] **Sensitive data protected:** screened safe transfer, restricted token files, appropriate Windows storage permissions, private DB/API exposure and HTTPS; logs/errors exclude secrets and raw agent material.
- [ ] **Bounded expensive work:** measure failed-login/hash load; apply smallest concurrency/admission/rate control needed to preserve unrelated requests. Rate-limiting design is not assumed already implemented.
- [ ] **Critical operations transactional:** event identity/ordinal admission, canonical command effect/receipt, project/member creation, lifecycle and import transitions; snapshot-consistent export target.
- [ ] **Duplicate request handling:** stable idempotent retry and conflicting-identity refusal; no unresolved rewrite/rebinding.
- [ ] **Safe logging/errors:** structured reason and stage/age/correlation where permitted; bounded private diagnostics and safe public errors.
- [ ] **Deadlines/recovery:** one total elapsed budget, cancellation-safe claims, startup reclaim, measurable lane progress, explicit capture loss/rejection.
- [ ] **Credential lifecycle:** web password rotation survives restart; environment only bootstrap/recovers missing admin; align stale help/comments and test exact candidate.
- [ ] **Outage cache:** finite age/size/identity, honest unknown revocation during network outage, immediate invalidation upon observed denial.
- [ ] **Restore/conservation:** verified originals/WAL, source/destination identities, interrupted/concurrent transfer checks, every record disposition, no scope widening.

Deployment environment/host/database access is outside application role boundary: an operator controlling these can obtain administrator access. Protect those surfaces. This is not a promise an existing administrator password is overwritten on restart.

---

## 9. Edge Cases

- [ ] Empty/no-memory/no-matching-project versus inaccessible data versus server outage.
- [ ] Invalid/oversized/malformed UTF-8/JSON/unknown event kind, contract, action, section, cursor, or remote.
- [ ] Duplicate/reordered event or command, conflicting reused identity, lost acknowledgment, repeated native start/import/setup.
- [ ] Concurrent sessions/worktrees/accounts, tied pagination keys, live writes during export, governance conflict, credential/server switch.
- [ ] Network outage, accepted-but-stalled reply, cold daemon startup, stale/expired cache, observed revocation, inaccessible server.
- [ ] PostgreSQL/daemon/disk/storage failure, capacity exhaustion, lock/hash contention, process kill, Windows live handles/pipe owner.
- [ ] Partial project/setup/migration/import/export/output operation; input and original data preserved appropriately.
- [ ] Explicitly completed session gets late event; interrupted session recovers; read never creates row.
- [ ] User-edited/manager-owned config, untrusted hooks, vendor update beyond evidence, unsupported generic/OpenCode delivery.
- [ ] Benign session yields no supported knowledge; obsolete/conflicting guidance reduces rather than fabricates confidence.
- [ ] Cross-project/personal/team privacy, invalid evidence/reference, attestation recheck, removed task record, legacy NULL/unknown provenance.
- [ ] Trace/data deletion, archival, or retention does not break surviving evidence or unresolved retry identity.

---

## 10. Non-Functional

### Performance and resource bounds

Source defaults are **not measured guarantees**. Candidate must establish effective runtime wiring and total latency under defined environment/load.

| Boundary | Existing alpha.8 value | Acceptance / open target |
| --- | --- | --- |
| Capture deadline | 250ms default | Hook end-to-end elapsed/disposition check; agent proceeds on failure. |
| Context deadline | 1,500ms default | One total drain/retrieve/render/output budget. Current composed waits can exceed intended internal budget. |
| Context budget | 3,000 tokens default | Actual selected/rendered budget and reserved truth/warning behavior checked. |
| Retrieval soft target | 250ms session-open; 100ms prompt-time in server source | Soft degradation targets, not universal API SLA. Verify under declared environment. |
| Safe event/body | 16KiB event, 8KiB content; 256 events/1MiB batch | Reject oversized payload at correct pre-buffer/handler boundary; do not truncate. |
| Outage cache | 200 sessions, 64KiB entry, 300s TTL | Memory/age/eviction/identity; no persistence guarantee. |
| Event spool | 50,000 events / 256MiB undelivered payload | Precise shedding/rejection/row-state accounting; total disk/other retained data ceiling separately measured. |
| Claims/network | 60s claim lease; 20s delivery request timeout | Startup/cancel/retry proof; per-request timeout does not waive context deadline. |
| Traces | 90-day retention, 500-row sweep batch | Retention works without long locks or broken surviving evidence. |
| General API / page load | Not benchmarked | Beta.2 draft target: ordinary authenticated list/detail/mutation p95 ≤500ms; usable primary page p95 ≤2s on declared reference host/LAN, excluding export/import. |

Sources: [configuration](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-core/src/config.rs), [event limits](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-core/src/event.rs), [cache/delivery](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairnd/src/deliver.rs), [spool](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-store/src/spool.rs), [retrieval](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/src/retrieve.rs).

### Scale

- Draft beta.2 envelope: 10 accounts, 10 concurrent active sessions, 20 projects, 100,000 canonical memories and 1,000,000 accepted events. Reference host: 4 vCPU/8GiB RAM with SSD, server/PostgreSQL/web together, LAN RTT ≤50ms; native edge measured separately. Targets are proposed acceptance criteria, not observed supported capacity.
- Measure retained events/knowledge/receipts, queue age and saturation, consolidation backlog, retrieval latency, export/import peak memory/time, and operational DB/disk cost.
- Representative full-size fixtures must replace scaled-only evidence for scale claims. Export currently materializes complete bundle; supported ceiling/streaming need is unmeasured.
- Optimize measured SQL/index/batching bottlenecks before introducing infrastructure. Source cap bounds queued payload, not lifetime DB/filesystem growth.
- Beta.2 draft targets: sustain 10 events/second for 30 minutes; drain processing backlog within 10 minutes after arrivals stop. Up to 256MiB logical bundle imports/exports each within 10 minutes with ≤1GiB additional process memory. Physical backup policy targets RPO ≤24h and RTO ≤60min within envelope; rehearse daily PostgreSQL backup/restore independently. Logical export is not disaster recovery.

### Compatibility

- [ ] Desktop and mobile responsive web with keyboard/accessibility basics.
- [ ] Advertised native targets: macOS arm64/x86_64, Linux arm64/x86_64, Windows x86_64; each support claim names packaged test evidence.
- [ ] Unix sockets and Windows named pipes; server/web amd64/arm64 containers.
- [ ] Claude Code/Codex native capture and observed automatic context; OpenCode capture with honest declined automatic delivery; generic supported manual MCP actions.
- [ ] Published-versus-legacy CLI/skill/schema compatibility explicitly separated. Server-first migration tested.
- [ ] Browser coverage: current Playwright desktop/mobile Chromium evidence; additional browsers remain unverified until tested.
- [ ] Self-hosted same-origin or explicitly verified exact split-origin deployment.

---

## 11. Testing

Reuse existing pure/component/journey/hostile tiers and web harness. Add runnable checks for acceptance gaps; no second test platform. Each requirement verdict is `PASS`, `FAIL`, or `NOT RUN` with exact evidence identity.

- [ ] **Unit tests:** screening, bounds, rule extraction, budgeting/rendering, keys/relations, error shaping and idempotency semantics.
- [ ] **Integration tests:** real SQLite/PostgreSQL/session/spool/auth/application handlers, no invented database success.
- [ ] **Main flow E2E:** clean supported deployment + real browser + installed native setup/hooks/MCP + accepted memory + later recall.
- [ ] **Failure flow:** stalled reply, kill/restart, offline/cache/expiry, queue saturation/cancel/reclaim, user-edited config, DB/disk outage, concurrent export/restore.
- [ ] **Permission tests:** member/nonmember/admin/nonadmin/disabled/cross-account/cross-project/personal/team and every mutation family.
- [ ] **Regression tests:** two-caller identity, nested context render, transmission accounting, unsupported starts, exact installed advice, strict DB prerequisites, pagination ties, mutation errors.
- [ ] **Artifact/platform tests:** packaged versions plus real setup/transport/capability; image build/clean DB/browser smoke before and after publication.
- [ ] **Usefulness checks:** frozen paired varied tasks, blind/independent scoring where practical, no-memory controls, per-scenario outcomes.
- [ ] **Performance checks:** effective deadlines/limits and declared workload, release binaries, host/load identity, full-size fixture when claiming scale.

### Traceability and known evidence gaps

| Legacy outcome | PRD requirements | User stories | Roadmap |
| --- | --- | --- | --- |
| PRD-01 setup | FR-001–005 | US-001, US-002 | N0–N3, N11 |
| PRD-02 ownership | FR-006–010 | US-002, US-010 | N8, N9 |
| PRD-03 capture | FR-011–015 | US-003 | N10, N12, N7 |
| PRD-04 durability | FR-016–020 | US-007 | N5, N10 |
| PRD-05 governance | FR-021–027 | US-005, US-006, US-008 | N4, N15, N7 |
| PRD-06 recall | FR-028–034 | US-003, US-004, US-007 | N10, N12, N13, N7 |
| PRD-07 web | FR-035–039 | US-005, US-006, US-008 | N6, N11, N13–N15 |
| PRD-08 upgrade | FR-040–044 | US-009 | N5, N12 |
| PRD-09 release | FR-045–049 | US-001, US-009, US-010 | N0, N3, N8, N9 |

Attach detailed environments/commands/results to each candidate evidence report using [release gates](roadmap.md#release-checklist). Reported uncommitted recovery-patch results are historical local evidence; exact alpha.9 candidate PostgreSQL/setup gates need fresh execution. Browser deployment, upgrade/restore, packaged support, and varied usefulness are not accepted by this PRD rewrite. Historical repeated fixture and [topic-key sample](../../evals/topic-key-effectiveness/RESULTS.md) are narrow historical evidence, not current complete product proof.

When implementing a group, map each FR and Given/When/Then acceptance to actual runnable check or explicit gap. New PRD-local IDs do not retroactively repurpose historical FR/SC tests.

---

## 12. Success Criteria

- [ ] Main supported user flow and failure/recovery flows work on exact installed candidate.
- [ ] Required accepted data persists/retrieves correctly after restart/retry; unresolved queue loss and cache loss are reported honestly.
- [ ] Permissions/privacy/evidence authority work for positive and adversarial role/domain cases.
- [ ] No known unresolved critical candidate defect; required incomplete gates remain visible.
- [ ] Effective deadline/resource limits hold; agreed API/page/workload targets met with evidence.
- [ ] Upgrade/transfer/restore conservation accounts for every source record without widening scope.
- [ ] Beta.1 frozen evaluation meets draft targets: ≥80% useful recall of eligible labelled claims, ≥90% delivered claims relevant/supported, ≥20% median reduction in repeated investigation actions, task-completion rate no lower than control, and zero scored privacy/attribution/high-impact harmful-advice defects.

| Product metric | Measurement | Target / decision |
| --- | --- | --- |
| First-use activation | Time and completion rate from clean deploy to inspectable accepted event, then first useful later recall | Alpha.9: three fresh-deployment runs succeed; setup ≤5 minutes after prerequisites and clean deployment to inspectable accepted memory ≤20 minutes excluding downloads. |
| Useful recall | Relevant evidence-backed decisions/procedures used in later task, per scenario | Beta.1: ≥80% eligible labelled claims usefully recalled; ≥90% delivered claims relevant/supported in frozen 30-pair corpus. |
| Avoided repeated work | Repeated investigation/failed approaches and task success with/without Cairn | Beta.1: ≥20% median reduction in repeated investigation actions; task-completion rate no lower than control. |
| Harm / distraction | Stale/conflicting harmful guidance, false positives, false refusals, context overhead | Zero scored privacy leaks, wrong-actor attribution or high-impact harmful advice; stale/conflict and benign cases cannot fabricate certainty/evidence. Misses block beta verdict until fixed or scope explicitly narrowed. |
| Adoption/consistency | Supported fact recording and topic/value consistency across sessions/agents | Current candidate measurement; historical sample is not baseline guarantee. |
| Reliability | Deadline misses, queue age/saturation/loss reasons, duplicate effects, recovery/restore outcome | Alpha.9 deadline/identity/conservation invariants; beta.2 envelope and performance/restore targets in section 10. |
| Support/evidence coverage | Advertised claims with exact-candidate installed/artifact evidence | Every shipped claim has proof or named limitation. |

Freeze 30 paired scenarios across at least three repositories and both primary adapters, with five pairs per class: fixes, decisions/procedures, repeated failures, stale/conflicting guidance, privacy/refusal boundaries, and benign no-memory work. Prelabel expected claims and scoring, counterbalance execution order, record model/agent/rule versions, and publish counts/denominators/uncertainty. Score repeat-work reduction only for prelabelled pairs with control count >0; report absolute counts for zero-control pairs. Context overhead is Cairn-delivered tokens per pair plus total agent input tokens where measurable, reported at median/p95; existing per-request context budget remains enforced. Alpha.9 only requires five-scenario smoke; broad usefulness verdict belongs to beta.1. Targets are draft values for review and must be fixed before scoring, not tuned to observed results.

Stored-record count is not primary success metric. If paired work does not improve or guidance is harmful, fix measured weakness or narrow claim before expanding features. Do not claim broad usefulness from repeated one-fixture trials.

---

## 13. Dependencies & Risks

| Item | Risk | Mitigation |
| --- | --- | --- |
| Canonical server/PostgreSQL | Setup/network/operations cost delays useful recall; outage prevents fresh search | Supported simple deployment, bounded edge queue/cache, honest degraded state, measured time-to-value. |
| Agent hooks/vendor versions/trust | Missing lifecycle/capture or unsupported delivery | Observed version/capability matrix, owned resources, fixtures and real installed journeys; manual supported recovery. |
| SQLite/disk/claims | Saturation, corruption, contention, process loss before delivery | Explicit admission/loss accounting, immutable IDs, recoverable leases, deadlines, restart/storage fault tests. |
| Deployment origins/TLS/credentials | Broken login, unsafe exposure, stale restart advice | One supported topology, exact-origin tests, private defaults, bootstrap/rotation proof and corrected guidance. |
| Safe deterministic extraction | Sparse/fragmented knowledge, false supported-looking inference | Closed/versioned rules, evidence review, explicit remembering, benign no-memory controls, paired usefulness evaluation. |
| Topic names / lexical matching | Same claim fails to meet; cross-project suggestions miss | Revalidate current consistency; suggest existing supported topics if needed, never guessed semantic merge. |
| Session identity/output envelope | Wrong sibling attribution or false transmission/confirmation | Exact native key/directory, read-only selection, strict parser/output accounting, two-caller/stalled transport checks. |
| Logical transfer/migrations | Inconsistent snapshot, high memory use, interrupted import, scope widening | Snapshot and size contracts, pinned WAL/conservation/retry/restore tests; physical backup separately rehearsed. |
| Authorization/private domains | Cross-project/account leaks or body-forged ownership | Shared server logic, role/domain matrix, sanitization, evidence authority, revocation/cache tests. |
| Password hashing/log/errors | Expensive work blocks unrelated traffic; raw DB errors expose internal detail | Measure bounded work/admission; safe public errors and bounded private diagnostics. |
| Native/web/artifact versions | Tests drive stale binary; published image/skill/docs differ | Exact source/artifact identities, tied gates, version checks, actual packaged setup and post-publication smoke. |
| Retention/data growth | Unlimited retained data or deletion breaks provenance/retry | Define per-class policy, measure disk/DB growth, preserve unresolved receipt/evidence semantics. |

---

## 14. Open Questions

**Concrete draft decisions for review:** immutable `v0.1.0-alpha.9` successor; solo/1–10-person self-hosted audience; same-origin default; preserve current authority/technology; in-memory cache through 0.1.0. [Roadmap version contract](roadmap.md#version-and-scope-contract) assigns alpha.9 safety/first-use, alpha.10 operations, beta.1 quality, beta.2 capacity/retention, rc.1 compatibility/pilot, then stable 0.1.0. Planning versions do not imply release acceptance or implementation approval.

Remaining choices are bounded decisions to settle before their named release gate.

| Question | Needed before | Current draft recommendation |
| --- | --- | --- |
| Does review accept primary audience and architecture? | Alpha.9 scope approval | Solo/teams of 1–10; current canonical server. Full offline authority and enterprise expansion excluded through 0.1.0. |
| Will split-origin be advertised as optional supported topology? | Alpha.9 N1/N4 | Same-origin default is selected; advertise split-origin only with its complete browser/origin evidence. |
| Which fork/mirror/duplicate remote spellings require explicit selection? | Alpha.9 FR-002 tests | Project UUID authority and ambiguous-match refusal are selected; enumerate normalization/fork fixtures and actionable selection behavior. |
| Does review accept beta.1 rubric/targets? | Freeze beta.1 corpus | Section 12 fixes numerical draft benefit/harm targets, zero-denominator rules and delivered/total-input token overhead metrics. |
| Does reference host/envelope represent intended operators? | Beta.2 benchmark freeze | Section 10 declares users/data/load/API/page/transfer and physical recovery targets; record hot/cold results. |
| Who owns each alpha.9 implementation and release gate? | Candidate work begins | Roadmap fixes successor/version/scope; assign named owners and exact source SHA. No alpha.8 asset replacement. |
| What adapter/OS/browser combinations have current installed proof? | Advertised support | Primary native journey plus focused supported-platform checks; label unverified combinations. |
| What retention/deletion rules apply beyond 90-day traces? | Broader cleanup | Per data class, preserve evidence and unresolved retry identity. |
| Does evidence justify a later cache-persistence proposal? | Future scope beyond planned 0.1.0 | Cache stays in-memory through 0.1.0; any later change requires measured benefit and storage/privacy design. |
| Is model-assisted extraction or expanded automatic delivery justified? | Future scope | Only measured benefit and stable supported data boundary; no automatic authority or unsafe vendor surface. |

- [ ] Ratify declared audience/topology/version scope and assign implementation/release owners.
- [ ] Ratify numerical draft evaluation/operating targets and freeze evidence method before scoring.
- [ ] Approve candidate scope, support matrix, and release identity.
- [ ] Resolve future retention/cache/integration choices only when their scope is pursued.

---

## 15. Release Checklist

This is a candidate checklist, not publication certification. [Roadmap release checklist](roadmap.md#release-checklist) defines required gates. Attach versioned candidate evidence report with execution results to release PR.

- [ ] Requirements complete: draft assumptions/decisions resolved for claimed scope; FR/acceptance/story mapping current.
- [ ] Acceptance criteria pass: primary and failure journeys, benign absence, domain/permission and action outcomes observed.
- [ ] Tests pass: Rust/strict PostgreSQL/web/browser/hostile and relevant installed-platform checks on same candidate; required `NOT RUN` gates unresolved.
- [ ] Security checked: credentials/privacy/authorization/origins/cache/error/work bounds.
- [ ] Migration checked: originals/WAL/forward schema, snapshot transfer, limits/conservation/retry, restore/credential recovery.
- [ ] Deployment checked: supported clean topology, matching source/archives/images/config/docs, actual packaged setup.
- [ ] Monitoring ready: liveness versus readiness explicit, last-evidence age, queue/pipeline state and safe diagnostics/recovery usable.
- [ ] Documentation updated: README, PRD, roadmap, changelog, version-correct guides, embedded contract/skill, support/limits/known gaps.
- [ ] Usefulness evidence recorded: thresholds and paired outcomes, uncertainty, measured weaknesses/claim limitations.
- [ ] Publication verified: checksums/provenance retained and exact published artifacts smoke-tested afterward.

No runtime fixes, deployment, complete security audit, or release gates are claimed from editing this document.
