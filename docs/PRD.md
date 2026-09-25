# Cairn product requirements — published alpha.8

**Updated:** 2026-09-26. **Product baseline:** published [v0.1.0-alpha.8](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.8), tagged at `4cbb2c6` on 2026-09-25. **Checkout warning:** this workspace's `main` is still `3741719` with alpha.7 source. This document describes the published alpha.8 product; it does not claim this checkout runs alpha.8. [Roadmap](ROADMAP.md) tracks the mismatch and future work.

Source of truth for the current product: the tagged [README](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/README.md), [architecture](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/architecture.md), [integration ownership](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/integrations.md), [testing](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/testing.md), code, and release notes. The Feature 001–005 specifications in this older checkout describe historical delivery; alpha.8 removed them from its tagged tree. They are useful history, not a current interface contract.

## Problem and product promise

AI coding agents lose context between sessions, compactions, agents, and machines. Decisions get repeated, evidence disappears, stale assumptions look current, and a local queued command can be mistaken for accepted shared knowledge.

Cairn captures bounded evidence from agent work, turns supported events into durable project knowledge, and returns relevant context to later sessions. It must show what was accepted, refused, delivered, cached, or unavailable. The server owns canonical truth; the local daemon exists to capture and deliver through interruption.

## Users, jobs, and outcomes

| User | Job | Successful outcome |
| --- | --- | --- |
| Coding developer | Connect a repository, let agent work, resume later | One setup path; next supported session receives relevant context with provenance and no manual search. |
| Developer changing agent or machine | Continue project work without carrying transcripts | Authorized server knowledge remains available; integration and receipt state explain any gap. |
| Project member | Inspect, correct, and share project knowledge | Memory, evidence, conflicts, and decisions are visible within membership boundaries. |
| Account owner | Keep personal guidance across authorized projects | Personal knowledge is private to that account and screened before sharing. |
| Administrator | Govern access, team guidance, migration, and service health | Roles and membership are enforced server-side; import, backup, and recovery have explicit outcomes. |

## Current end-to-end flow

```text
Web Settings: admin creates account/membership; user creates token
  -> setup-ready project needs registered repository remote (API path today)
  -> developer runs cairn setup in Git repository with matching remote
  -> setup verifies credential + membership, installs owned hooks/MCP, starts cairnd
  -> hooks send bounded, privacy-screened events into distinct edge spools
  -> daemon retries immutable operations; server records one canonical effect
  -> server consolidates accepted events into attributable knowledge
  -> retrieval selects and explains bounded context for a supported agent
  -> web lets authorized people inspect, govern, and repair server-owned state
```

The edge stores binding, spools, receipts, finite-age returned context, session correlation, integration ownership, and migration metadata in SQLite. PostgreSQL owns accepted events, sessions, handoffs, project/personal/team knowledge, evidence, relations, verification, governance, graph/replay/analytics views, retrieval, and idempotency receipts. See the tagged [architecture](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/architecture.md).

## Functional requirements and shipped behavior

These `PRD-` identifiers organize this summary. They are not replacements for historical `FR-` or `SC-` identifiers.

| ID | Requirement | Alpha.8 behavior and acceptance boundary |
| --- | --- | --- |
| PRD-01 | One human setup path | `cairn setup` discovers the Git repository, requires a matching registered remote and existing membership, verifies credentials, starts daemon, and installs supported adapters. It cannot grant access or register a user. Today, web project creation omits the remote, so a setup-ready project must be created with `repository_remote` through the API. Hidden `cairn hook` and `cairn mcp` remain machine entry points. |
| PRD-02 | Safe integration ownership | Setup records exact resources and bytes it owns; rerun repairs matching owned bytes and reports conflicts on user-modified bytes. Clone alone changes no local agent configuration. |
| PRD-03 | Five agent tools | `cairn_context`, `cairn_search`, `cairn_remember`, `cairn_session`, and `cairn_handoff`. Routine capture, consolidation, and handoff need no explicit tool call. There is no `cairn_task`. |
| PRD-04 | Private, bounded capture | Hooks accept only safe structured events through local screening. Raw prompts, transcripts, diffs, command output, credentials, and unbounded payloads do not cross to server. Rejection names policy class without echoing rejected content. |
| PRD-05 | Durable, truthful edge delivery | Capture and command spools have separate bounds and rules. Immutable operation identity survives retry; daemon acknowledges only after server acceptance. Saturation and deadline loss must be visible, never reported as successful capture. |
| PRD-06 | One canonical server effect | Authenticated server validates and deduplicates repeated operations by identity, returns prior receipt, and applies authorization by project, owner, team role, or administrator. Client request bodies cannot choose account ownership. |
| PRD-07 | Attributable autonomous memory | Server consolidates accepted closed-session events under bounded work claims. Durable records retain provenance and one of the five knowledge kinds: fact, decision, convention, failure, procedure. Extraction and refusal stay deterministic. |
| PRD-08 | Governed knowledge | Users can create, reinforce, pin, forget, supersede, relate, verify, and inspect knowledge where authorized. Conflicts and drift remain visible; team proposals require administrator ratification. Personal entries remain owner-only. |
| PRD-09 | Bounded retrieval | Current-session, branch, then project applicability and evidence/verification signals drive ranking. Project truth takes priority over personal/team guidance. Explanation exposes selection, decay, and delivery state. A generated briefing is not proof of agent receipt. |
| PRD-10 | Honest offline mode | Safe capture queues within explicit limits. Eligible returned context cache has finite age and matching identity; search reports server unavailability. Authorization denial invalidates relevant cache immediately; network loss cannot establish whether access was revoked. |
| PRD-11 | Web control plane | Authorized users choose a project and use Overview, Memory, Sessions, Governance, and Settings. Settings includes password, tokens, projects, membership, policy, import/export, users, and health according to role. |
| PRD-12 | Safe upgrade | Setup backs up a legacy SQLite store including WAL state, starts a fresh edge store, preserves safe pending operation identities, and writes an import bundle plus conservation report. Task, local-only, unsupported, and ambiguous rows remain offline as `removed_feature`; server migration archives task data before dropping live schema. Import is resumable and idempotent. |
| PRD-13 | Release evidence | Rust tests and lint, PostgreSQL integration, generated web API contract, production web build, and live browser checks are separate gates. A skipped database test is not a pass. |

## Web experience in alpha.8

| Destination | Delivered job | Current limitation or check |
| --- | --- | --- |
| Project selector | Select only a project the account may access | Setup cannot create membership. |
| Overview | Counts, accepted activity, delivery recency, retrieval effectiveness | Show data age and stale health clearly. |
| Memory | Project/personal views, search, creation, detail, provenance, verification, relations, graph, retrieval explanation | Project list displays first 25 and advises filtering; complete browsing needs continuation. |
| Sessions | Session history, handoffs, accepted-event replay | Receipt and handoff visibility must stay distinct from inferred agent state. |
| Governance | Proposals, conflicts, team ratification/retirement, supersession | Role checks live on server. |
| Settings | Password change, tokens, users, projects, membership, privacy, logical import/export, service health | Project creation accepts only a name in web, leaving no matching remote for setup. Split-origin admin edit uses PATCH but CORS omits PATCH. Environment admin credentials bootstrap a missing admin; changing that environment password does not rotate an existing admin password. |

The tagged tree has nine Next.js page files implementing these destinations, including project memory and session detail plus login. The web API has a generated typed contract and a contract check. See [tagged UI source](https://github.com/Vellixia/Cairn/tree/v0.1.0-alpha.8/web/app) and [API contract](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/contracts/web-api-v1.ts).

## Supported integration and operating boundaries

- Claude Code and Codex CLI: native hook capture and supported automatic context delivery. Their actual capability and trust state must be reported from observed integration health.
- OpenCode: native capture. Cairn declines automatic delivery through its beta context surface; do not describe this as no OpenCode capture.
- Generic MCP client: five manual tools, with no automatic hook guarantee.
- Self-hosted server: PostgreSQL 17 in example Compose stack, Rust/Axum server, Next.js web. The native local agent is not containerized in that stack. The tagged example publishes web and API on separate ports while leaving `CAIRN_API_ORIGIN` empty and supplies no reverse proxy; a fresh browser cannot route its default same-origin API requests through that example as written.
- Supported published binary targets: macOS arm64/x86_64, Linux arm64/x86_64, Windows x86_64. Alpha interfaces and schemas may still change.

## Historical feature map: what survives alpha.8

| Historical delivery | Retained in alpha.8 | Removed or relocated in alpha.8 |
| --- | --- | --- |
| 001 MVP | Git identity, capture, sessions/handoffs, bounded context, privacy controls, server/web foundations | Local canonical memory and search; broad human CLI; optional server authority model |
| 002 agent integration | Claude Code/Codex/OpenCode adapters, hooks, MCP, owned configuration | Separate `connect`, `doctor`, `repair`, `disconnect`, and distribution commands as human UI |
| 003 project intelligence | Evidence, reconciliation, conflicts, verification, drift, patterns, bounded retrieval | Task domain/scope and local canonical knowledge; server now owns surviving intelligence |
| 004 collaborative global memory | Admin accounts and membership, personal/team domains, privacy gate, ratification | Namespace pull/cutover runtime; current server authority replaces that sync model |
| 005 autonomous memory | Safe events, durable spools, server consolidation, automatic retrieval/traces, health, web control | Older local cache/entity sync and separate web routes |
| 006 in effect: alpha.8 V1 consolidation | One setup command, five MCP tools, thin edge, canonical server, consolidated web, explicit legacy import | Tasks throughout CLI, MCP, daemon, server, database, web, analytics, and retrieval |

Alpha.8's change is intentional deletion. Historical checkboxes measure work done at the time; they do not imply a removed surface is still in the product. See [alpha.8 changelog](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md).

## Quality targets and release gates

| Goal | Required evidence before claiming it |
| --- | --- |
| Setup works | Fresh machine/repository with a registered remote and existing membership reaches verified daemon/hook/MCP state. Conflict case preserves user edits. |
| Capture is safe | Adversarial secret/path/payload corpus; explicit disposition for refusal, deadline, and spool saturation; no raw material crosses boundary. |
| Delivery is durable | Kill/restart/retry after server accepts but before local acknowledgment; one canonical effect and stable receipt. |
| Recall helps | Related second session receives bounded relevant knowledge; traces distinguish considered, selected, transmitted, and confirmed states. Measure usefulness on varied tasks, not one repeated fixture. |
| Authorization holds | Nonmember, cross-account, disabled account, and nonadmin attempts cannot read or mutate forbidden data. |
| Upgrade conserves data | Real alpha.7 → alpha.8 restore/import rehearsal; conservation report accounts for every retained, accepted, rejected, pending, and unchanged item. No task or local-only scope widening. |
| UI is complete enough | Desktop/mobile flow covers full memory browsing, governance, administration, offline/degraded states, and recovery actions. |
| Release is reproducible | Tagged source, published archive/image versions, example environment, migrations, API contract, and CI all agree. |

The earlier [Feature 005 accuracy record](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/tests/feature005/acceptance-results.md) includes a dated independent AI PASS on supplied claim forms at an earlier commit. Its thirty trials repeat one fixture scenario per agent; it does not establish broad accuracy across repository work or current-tag behavior.

## Exclusions and decisions

No task feature or task scope in V1. No local searchable copy of canonical server knowledge, independent offline authority for linked projects, account self-registration, self-join, hosted SaaS promise, required embeddings/vector database, or automatic graph-based truth engine. Existing server graph is a bounded view over relations, not a separate graph database. New features must preserve privacy, provenance, and server authorization rather than reopening removed paths.

Open product decisions and ordered delivery live in the [roadmap](ROADMAP.md). This PRD records alpha.8's product contract and measurable intended outcomes; it does not mark proposed improvements as shipped.
