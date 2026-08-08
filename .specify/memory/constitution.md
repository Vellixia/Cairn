# Cairn Constitution

Cairn is persistent, project-aware memory for AI coding agents. These principles
govern how Cairn is built. They are deliberately few. Anything not constrained
here is a normal engineering decision.

## Core Principles

### I. Usable MVP First

Every feature must end with something a developer can install and use. A feature
is not done when its layers exist; it is done when the end-to-end flow it
promises runs on a real repository. Work is sliced vertically (agent → daemon →
storage → surface), never as an architecture-first horizontal buildout. If a
phase leaves nothing runnable, the phase is wrongly scoped.

### II. Simple Architecture, No Speculative Infrastructure

Prefer the smallest component set that satisfies the current requirement. New
infrastructure (a broker, a cache tier, a second datastore, a new service) must
be justified by a requirement that exists today, not by anticipated scale.
Speculative machinery — embeddings, graphs, decay models, confidence engines,
distributed coordination — is out of scope until real usage demonstrates need.
Extensibility is achieved through clean schema and API boundaries, not through
pre-built abstraction layers.

### III. Local-First Reliability

Cairn must be fully useful offline. Capture, recall, context assembly, handoff,
and search must work with no network and no central server. Remote sync is an
enhancement layered on top of a complete local system; server unavailability
degrades sharing, never local operation. Cairn must never block, slow, or break
the coding agent it is attached to: agent-facing operations fail soft.

### IV. Project-Scoped Memory

All durable knowledge carries explicit scope — project, branch, task, or session
— and explicit provenance back to the session and observations that produced it.
Memory is never global or ambient. Retrieval respects scope precedence, and any
recalled item can be traced to where it came from and why it applies.

### V. Privacy by Default

Cairn stores structured facts, not transcripts. Full conversations and raw tool
output are not persisted by default; captured payloads are bounded and
summarized. Common secret patterns are redacted before storage. Users can
exclude paths and content, keep memory local-only, and delete any observation,
memory, or session. Data leaves the machine only when the user has chosen to
share it.

### VI. Deterministic Data Boundaries

Repository state (project, branch, commit, worktree, working-tree status) is
derived from Git, not guessed. Identity, scope, and sync are keyed by stable
identifiers. Synchronization is idempotent: replaying a sync converges to the
same state. The same inputs must produce the same context briefing.

### VII. Testable Behavior

Requirements are stated as user-observable behavior and verified through it.
Every feature defines how it is demonstrated on a real repository. Automated
tests cover the behavior a user depends on — capture, recall, context, handoff,
sync — in preference to internal structure. Bounded outputs (context size,
payload size) are asserted, not assumed.

## Product Constraints

- Cairn integrates with coding agents through MCP and lifecycle hooks. Claude
  Code is the first-class integration; other MCP-compatible agents must remain
  able to use Cairn manually.
- The agent-facing tool surface stays compact. Tools are product verbs, not
  database operations.
- Context delivered to an agent is bounded and budgeted. Depth is reached by
  explicit search, not by inflating the automatic briefing.
- Local storage is embedded and file-based. Shared storage is a single
  relational database behind one server.

## Development Workflow

- Specification precedes planning; planning precedes tasks; tasks precede
  implementation.
- Requirement counts and task counts are kept proportionate to the work. Ceremony
  that does not change the product is not produced.
- Out-of-scope items are recorded explicitly in the spec rather than implemented
  "while we're here".
- Complexity that violates a principle must be recorded with its justification
  and the simpler alternative that was rejected.

## Governance

This constitution supersedes other process conventions in this repository.
Amendments require an explicit edit to this file with a version bump and a note
of what changed. Plans must include a constitution check before design and after
design; violations must be justified in the plan's Complexity Tracking table or
removed. When a requirement and a principle conflict, the conflict is resolved
in the spec — not silently in the implementation.

**Version**: 1.0.0 | **Ratified**: 2026-08-07 | **Last Amended**: 2026-08-07
