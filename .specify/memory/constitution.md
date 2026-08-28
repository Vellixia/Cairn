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

### IV. Explicitly Domained Memory

All durable knowledge carries an explicit domain — project, personal, or team —
and no knowledge is domain-less. Within the project domain, knowledge additionally
carries explicit scope (project, branch, task, or session) and explicit provenance
back to the session and observations that produced it. Memory is never *ambient*:
nothing is recalled that cannot name the domain it belongs to and state why it
applies here. Retrieval respects domain separation and scope precedence, and any
recalled item can be traced to where it came from.

A domain is not a scope. Personal and team knowledge are project-independent by
construction — they cannot name a project, and no memory scope may be introduced
that crosses projects. Knowledge that applies beyond one project is a distinct
knowledge type with its own storage, never a wider scope on project memory.

### V. Privacy by Default

Cairn stores structured facts, not transcripts. Full conversations and raw tool
output are not persisted by default; captured payloads are bounded and
summarized. Common secret patterns are redacted before storage. Users can
exclude paths and content, keep memory local-only, and delete any observation,
memory, or session. Data leaves the machine only when the user has chosen to
share it.

Raw observations, evidence, verification runs, local paths, tool output and
machine configuration never leave the machine that produced them. Moving knowledge
from one domain to a wider one is always an explicit act, never a side effect, and
must pass a deterministic privacy gate that fails closed. Structural prevention is
preferred to procedural rules: a record that has no column for a secret cannot
carry one.

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

A test that cannot fail protects nothing. Assertions that depend on parsing source
text, or on state a test never actually establishes, must be written so that a
missing target fails the test rather than passing it vacuously.

### VIII. Project Truth Is Not Displaceable

Broader knowledge never outranks narrower knowledge. Personal and team guidance
may occupy only context space that project knowledge has left unused; they can
never enter a reserved allocation, never displace project state, and never be
presented as interchangeable with it. Domains stay visibly separate in every
result rather than being merged into one ranked list. Cross-project guidance is
never the basis of a verification claim: a deterministic check performed against
one project does not transfer its authority to a project-independent assertion.

## Product Constraints

- Cairn integrates with coding agents through MCP and lifecycle hooks. Claude
  Code is the first-class integration; other MCP-compatible agents must remain
  able to use Cairn manually.
- The agent-facing tool surface stays compact. Tools are product verbs, not
  database operations.
- Context delivered to an agent is bounded and budgeted. Depth is reached by
  explicit search, not by inflating the automatic briefing.
- Local storage is embedded and file-based. Shared storage is a single
  relational database behind one server. One server is one team; Cairn has no
  organizations, tenants, or nested groups.
- The agent-facing tool surface does not grow to accommodate new knowledge
  domains. New capability is reached by extending the actions of existing tools.
- Knowledge that agents can create and knowledge that becomes shared policy are
  different acts. An agent may propose; only a human administrator may make
  team-wide guidance authoritative.

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

## Amendment History

### 1.1.0 — 2026-08-21

Adopted with Feature 004 (Collaborative Global Memory), which required knowledge
that applies across projects.

- **Principle IV** was retitled from "Project-Scoped Memory" to "Explicitly
  Domained Memory". Its sentence "Memory is never global or ambient" is replaced.
  The prohibition that mattered was on *ambient* memory — knowledge recalled
  without being able to say what it belongs to or why it applies. That prohibition
  is retained and strengthened. The word "global" was doing a second job it should
  not have been doing: forbidding cross-project knowledge outright. Principle IV
  now requires every record to name a domain, and separately forbids widening a
  memory *scope* across projects — which is the constraint that actually protects
  project truth.
- This amendment supersedes the wording of Feature 003's FR-391 ("A project memory
  MUST NOT become a global memory, and no memory scope crossing projects may be
  introduced") in one respect only: its second clause is retained verbatim as
  binding, while its first clause is understood as forbidding *silent* promotion,
  not explicit gated promotion into a separate knowledge type. Feature 003's own
  `reusable_patterns` — a table deliberately carrying no project identifier — was
  already the precedent for project-independent knowledge under v1.0.0.
- **Principle V** gained the explicit-promotion and fail-closed-gate requirement,
  and the preference for structural over procedural prevention.
- **Principle VII** gained the rule against vacuously passing tests, after one was
  found in the repository.
- **Principle VIII (new)** — "Project Truth Is Not Displaceable" — states the
  non-displacement guarantee that makes broader domains safe to add at all.
- Product Constraints gained: one server is one team; the tool surface does not
  grow for new domains; proposing is not ratifying.

**Version**: 1.1.0 | **Ratified**: 2026-08-07 | **Last Amended**: 2026-08-21
