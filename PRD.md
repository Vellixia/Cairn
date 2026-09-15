# Cairn V1 Product Requirements

## Status and promise

This document is Cairn V1's product contract. It describes acceptance behavior,
not a claim that every V1 capability has already shipped in the current alpha.

Cairn gives an AI coding session durable, scoped project knowledge without
requiring the developer or agent to reconstruct prior work. It automatically
captures safe structured work signals, turns accepted signals into evidence-backed
knowledge, and delivers a bounded, relevant briefing on the next session. A
linked project has one canonical owner: its Cairn server. The product remains
self-hosted and fail-soft for the agent when that server is unavailable.

## Primary flow

1. Developer installs Cairn, connects a supported agent, and works in a Git
   repository. Cairn identifies repository, branch, commit, task, and session.
2. Hooks capture bounded, privacy-filtered structured events without blocking
   the agent. Cairn derives handoffs and proposes useful knowledge automatically;
   explicit CLI and MCP actions remain an override, not the only path.
3. Server accepts authorized safe events and derives canonical project, personal,
   or team knowledge with provenance and evidence. A second authorized machine
   retrieves same scoped project truth and handoff.
4. Next session receives bounded context prioritized by task, branch, project
   truth, then eligible personal and team guidance. Search explains why each
   result applies and how it ranked.

## Authority, cache, and offline contracts

- **Authority.** An unlinked project is local-only. Once linked, server is
  canonical owner of durable knowledge, membership, ownership, governance, and
  accepted event history. Client input is an intent; no local write is canonical
  before server acceptance.
- **Edge state.** Client retains only safe pending events, machine-local
  integration state, and bounded non-authoritative context cache. Pending events
  persist across restart, have stable idempotency identity, and drain exactly
  once in canonical effect.
- **Cache.** Every cached context names server, account, project, retrieval time,
  and staleness. Server, account, or project identity change invalidates it.
  A cache hit is visibly cached/stale; a miss never promotes local rows to truth
  or crosses an identity boundary.
- **Outage.** Capture remains fail-soft and queues safe work. Server-dependent
  truth may be unavailable; Cairn reports that condition. It does not block the
  coding agent, invent fresh knowledge, or treat queued work as accepted.
- **Privacy and isolation.** Cairn stores structured, bounded facts rather than
  transcripts or raw tool output. Deterministic privacy gates run before storage
  and egress. Authorization derives caller identity from credentials; account,
  project, domain, and ownership boundaries remain enforced on every read/write.

## V1 product surface

- **Memory and context.** Project knowledge has project, branch, task, or
  session scope plus provenance. Personal and team knowledge are separate
  domains, visibly separate in output, and cannot displace reserved project
  context. Superseded and stale records remain historical, never default truth.
- **Search.** Lexical retrieval is baseline. Optional vector and relation signals
  may improve recall only when each result exposes score components and scope
  applicability. Search does not make a verification or authority decision.
- **Graph.** Graph is capped PostgreSQL expansion over existing typed memory
  relations, not a graph datastore or visualisation product. Related-result
  contributions are bounded and explainable.
- **Replay.** Replay is a read-only timeline of accepted safe events. It never
  exposes raw transcripts, re-executes events, or makes new knowledge canonical.
- **Decay.** Decay is deterministic recency contribution to ranking only. It
  never deletes knowledge or changes truth, ownership, evidence, or verification.
- **Analytics.** Analytics are bounded counts from existing records: capture,
  consolidation, retrieval, delivery, latency, and failures. They are not a
  generic analytics platform or source of authority.
- **Governance.** Project knowledge preserves personal ownership. Team guidance
  uses proposal, ratification, retirement, conflict, and supersession states;
  proposal and ratification remain distinct authorities.

## Interface and architecture constraints

- Cairn exposes exactly six MCP tools: `cairn_context`, `cairn_search`,
  `cairn_remember`, `cairn_session`, `cairn_task`, and `cairn_handoff`.
  Graph, replay, and governance are typed actions within this surface, never
  extra MCP tools.
- CLI default discovery emphasizes `setup`, `connect`, `status`, `context`, and
  `search`; deeper work is grouped under memory, session, task, replay,
  governance, migration, and administration.
- Project UI is Overview, Memory, Sessions/Replay, and Tasks. Governance and
  Administration are distinct global areas. Domain-separated server
  authorization remains separate even when UI panels combine workflows.
- PostgreSQL, current crates, and current processes remain unless evidence proves
  a consolidation reduces production code without weakening isolation.

## Feature admission rule

A capability enters V1 only when it advances the primary flow, uses existing
authoritative records and process boundaries, has explicit domain/scope/privacy/
authorization behavior, remains bounded and explainable, and has a focused
user-observable acceptance test. Otherwise it is out of scope. Cairn does not
add a graph service, vector database, raw archive, workflow engine,
project-management suite, managed SaaS, billing, organizations, or generic
analytics platform.

## V1 acceptance criteria

- Automatic capture creates server knowledge without manual MCP calls, and a
  second authorized machine retrieves same scoped memory and handoff.
- Offline events survive restart and canonical drain is idempotent; cached
  context is labelled and cannot cross server/account/project boundaries.
- Unauthorized registration, joining, cross-project reads, and cross-domain
  writes are refused; privacy, ownership, conflict, evidence, and supersession
  rules remain intact.
- Context remains bounded and favors task/project truth. Hybrid retrieval meets
  or improves frozen baseline with explainable lexical, optional vector, and
  relation contributions.
- Graph expansion is bounded and explainable; replay contains accepted safe
  events only; decay changes ranking only; analytics report existing-record
  counts only; governance preserves ownership, conflict, ratification,
  retirement, and supersession.
- Versioned export/import preserves canonical identities, relations, evidence,
  tombstones, pending events, and local-only records across interruption and
  retry, reporting accepted, rejected, and retained records explicitly.
- CLI snapshots cover compact discovery. Web end-to-end tests cover four project
  workflows plus governance and administration. Production-code reduction is
  measured separately from moved code, tests, fixtures, generated files, and
  dependencies.
