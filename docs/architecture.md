# Cairn V1 Architecture

`PRD.md` defines product behavior. This document names current architecture
foundations and target-V1 mechanisms required for acceptance; target sections do
not claim unimplemented behavior is shipped.

## Components and ownership

```text
Agent hooks/MCP/CLI ──> cairnd + SQLite edge state ──> Axum API + PostgreSQL
                                  │                         │
                                  └── bounded cache          └── canonical truth
                                                            └── Next.js control plane
```

- `cairn-core` owns domain types, deterministic privacy/redaction, context
  budgeting, and handoff synthesis.
- `cairn-git` owns Git CLI access and local repository/worktree state derivation.
- `cairn-integrate` owns agent adapters, lifecycle normalization, integration
  configuration, installation, capability evidence, and integration contracts.
- `cairn-store` owns SQLite edge persistence, FTS lexical search, pending-event
  spool/outbox, cache metadata, and local operational state.
- `cairnd` owns local capture, delivery, recovery, cache use, and sync.
- `cairn` owns CLI, hook runtime, and six-tool MCP protocol.
- `cairn-server` owns authenticated API, authorization, canonical acceptance,
  PostgreSQL persistence, and server-side governance.
- `web/` is a Next.js control plane. It consumes authorized server APIs; it is
  never an authority.

No component adds a second canonical store. No graph service, vector database,
raw archive, broker, workflow engine, or new process is part of V1.

## Write, sync, and recovery path

1. Agent-facing capture is bounded, redacted, and fail-soft. It records safe
   local work or returns without blocking the agent.
2. For linked projects, local mutation becomes a stable pending event inside one
   transaction. Local success means queued intent, never canonical acceptance.
3. Drainer sends event under authenticated server/account/project context.
   Server authorizes current caller, assigns/validates canonical outcome, and
   deduplicates retries by stable identity.
4. Client applies accepted response/pull as cache replacement. It never merges a
   local assertion into server truth. Interrupted claims become retryable; replay
   converges to one canonical effect.
5. Recovery reports pending/failed/blocked state, resumes safe work, and never
   re-executes accepted events as side effects.

## Read and cache path

Fresh server reads make authorization decisions at server. Client cache may serve
only an already-authorized bounded context for same server, account, and project.
Each cache record includes those identities, retrieval time, and staleness state.
Changing any identity invalidates rows before read. Cache output is labelled; a
miss reports unavailable durable context rather than elevating local copies.

Retrieval orders applicable project scope before broader domains. Lexical search
is baseline; optional vector/relation signals are additive, bounded, and returned
as score explanation fields. No retrieval score alters verification or authority.

## Data and safety invariants

- Canonical records retain stable identity, scope/domain, ownership, provenance,
  evidence references, relations, tombstones, and supersession history.
- Supersession and conflicts are explicit relations/states. Stale and superseded
  records are historical by default, not erased truth.
- Raw observations, transcripts, raw tool output, secrets, absolute paths, and
  vendor payloads do not cross boundary. Deterministic gate is shared by ingress
  paths and fails closed.
- Caller identity comes from authentication, not request payload. Server checks
  membership and domain/ownership authorization for every operation.
- Export/import uses versioned manifests and is resumable/idempotent. It reports
  accepted, rejected, and retained records, including pending and local-only
  state, rather than silently dropping them.

## Target V1: bounded advanced actions

Existing foundations are six MCP tools, authorized HTTP domains, typed memory
relations, and canonical records. V1 acceptance requires graph, replay,
governance, and decay/analytics/search extensions to use typed actions within
that surface; it does not add MCP tools. When implemented, graph will read
existing typed relations through capped PostgreSQL traversal; replay will read
only accepted safe events; decay will be deterministic ranking input; analytics
will aggregate existing records; governance transitions will preserve separate
proposer, ratifier, retirer, ownership, conflict, and supersession checks. None
may create authority, new durable infrastructure, or unbounded UI surface.

## Verification boundary

Acceptance tests exercise capture → acceptance → cross-device context, restart
and outage queue behavior, cache identity invalidation, authorization/privacy,
bounded retrieval explanations, graph/replay safety, governance transitions, and
interrupted import recovery. Production-code reduction is measured independently
of moved code, tests, fixtures, generated output, and dependencies.
