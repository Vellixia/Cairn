# V1 architecture

## Boundary

```text
cairn setup
  -> hidden hook/MCP adapters
  -> cairnd capture, typed spools, receipts, bounded cache
  -> cairn-server canonical behavior
  -> web human interaction
```

PostgreSQL is canonical for project, personal, and team knowledge; evidence and
relations; accepted safe events; sessions and handoffs; governance; verification and
supersession; graph traversal; retrieval ranking and decay; bounded analytics; and
idempotency receipts.

SQLite is an edge database only. It stores server/account/project binding, distinct
capture and command spools, receipts, bounded returned context, hook/session
correlation, Cairn-owned integration metadata, and migration-manifest metadata. It
does not own searchable knowledge or a replica of canonical server state.

## Delivery invariant

Each queued payload is immutable and has stable identity. Daemon claims it, sends it,
and acknowledges it only after server acceptance is recorded. Retrying after a lost
acknowledgement sends the same identity; server returns the existing receipt and does
not repeat canonical effect. At-least-once transport therefore produces exactly one
canonical effect.

Capture and command records remain distinct because their ordering, capacity, and
rejection rules differ. One delivery loop handles both without a generic command
router or entity outbox.

Privacy-policy changes may suppress or replace an operation only after prior
identity's receipt state is resolved.

## Retrieval and offline behavior

Applicability order is current session, then branch, then project. Verification,
authority, conflict, supersession, and pinning outrank recency. Decay affects ranking
only and appears in retrieval explanation.

Offline capture continues up to explicit spool limits. Saturation rejects capture
rather than silently dropping it. Eligible cached context is finite-age and labelled
with age and identity; search reports server unavailability. Authentication denial
invalidates matching cache immediately. Network failure does not imply knowledge of
revocation.

## Authorization

Every surviving operation has one server application implementation and an explicit
project, personal, team, account, or administrator authorization boundary. Hooks,
MCP, and web are thin adapters. Credentials determine actor identity; request bodies
cannot name account ownership.

## Removed domains

Task domain, task scope, task identifiers on sessions, task retrieval, task evidence,
task analytics, task sync, local canonical knowledge, full-state pull sync, namespace
authority, cutover runtime, browser-facing CLI administration, and registration/join
stubs are not V1 runtime concepts.

Legacy Task records and dependent scoped data are exported as `removed_feature` and
retained offline. Migration never widens their scope or promotes them into canonical
knowledge.

## Migration

Migration is setup/import work, not a continuous runtime subsystem. Legacy SQLite is
backed up with WAL state and left unchanged; a fresh edge database starts immediately.
Safe pending operations retain identity. Server logical import is bounded, resumable,
idempotent, and records a disposition per source record. Applied PostgreSQL migrations
remain immutable history.
