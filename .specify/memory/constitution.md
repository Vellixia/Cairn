<!--
Sync Impact Report
==================
Version change: 1.0.0 → 1.1.0
Rationale: MINOR amendment materially expands Principle II with a narrowly
defined local-bootstrap sequencing exception, explicit classification and
binding rules, and a future migration gate. The project/task invariant is
preserved for project-aware and task-aware sessions.

Modified principles:
- II. Exact Execution Scope → clarified project-aware, task-aware, and
  `local_unbound` bootstrap session invariants.

Added sections:
- Governance amendment record for version 1.1.0.

Removed sections: none.

Templates requiring updates:
- ✅ .specify/templates/plan-template.md — existing Constitution Check gate
  requires each feature to declare its applicable session class; no template
  wording change needed.
- ✅ .specify/templates/spec-template.md — existing scope and assumptions
  sections can declare bootstrap classification; no change needed.
- ✅ .specify/templates/tasks-template.md — existing migration and contract
  task structure covers future binding work; no change needed.
- ✅ .specify/templates/checklist-template.md — generic, no conflict.
- ✅ Spec Kit command definitions — reviewed; they defer to the constitution
  and require no wording change.

Feature artifacts aligned:
- ✅ specs/001-local-session-foundation/spec.md
- ✅ specs/001-local-session-foundation/plan.md
- ✅ specs/001-local-session-foundation/data-model.md
- ✅ specs/001-local-session-foundation/contracts/ipc-contract.md
- ✅ specs/001-local-session-foundation/tasks.md
- ✅ specs/001-local-session-foundation/evidence/quickstart-run.md

Follow-up TODOs: none. No deferred placeholders.
-->

# Cairn Constitution

## Core Principles

### I. Observable Reality Is Authoritative

Conversation summaries, agent claims, and generated documentation are never
authoritative by themselves. Repository state, Git metadata, command results,
test results, configuration, and other directly observed evidence take
precedence whenever they conflict with narrative sources. Every AI
interpretation stored or surfaced by Cairn MUST remain traceable to the
evidence that produced it.

Rationale: Cairn exists to record what actually happened during agent
execution; an untraceable claim is indistinguishable from a hallucination.

### II. Exact Execution Scope

A project-aware execution session MUST reference exactly one project. A
task-aware execution session MUST reference exactly one immutable task
revision. Every execution session MUST remain scoped to exactly one local
user, repository, worktree, and agent instance. Each recorded session state
MUST reference an explicit branch state (including detached HEAD), commit,
and working-tree snapshot. Project, branch, task, and session knowledge MUST
remain separately scoped.

A local bootstrap session MAY temporarily be unbound from a project and task
revision only when project and task-revision capabilities are not yet
available. It MUST be explicitly represented as `local_unbound`, remain
scoped to repository, worktree, agent instance, and local execution, and MUST
NOT be synchronized, promoted to project truth, or used as authoritative
project memory while unbound. When a bootstrap-only feature contract exposes
no project-aware synchronization surface and no mixed bound/unbound modes,
that contract MAY explicitly classify every session it defines as
`local_unbound`; a record-level discriminator is REQUIRED before mixed modes
or project-aware synchronization are introduced.

Binding a local bootstrap session later MUST be explicit, reference exactly
one project and one immutable task revision, append a binding event, preserve
the original execution history, and MUST NOT retroactively rewrite prior
events. The feature that introduces projects and task revisions MUST define
migration and binding behavior before project-aware synchronization is
allowed. Information from one scope MUST NOT silently become truth in
another; any cross-scope promotion MUST be an explicit, recorded operation.

Rationale: Knowledge that leaks across scopes silently corrupts every
consumer of that scope; explicit promotion keeps provenance intact. The
`local_unbound` rule is a sequencing clarification for a local foundation,
not removal of the long-term project/task invariant and not permission to
invent placeholder project or task records without lifecycle semantics.

### III. Append-Only Execution History

Prompts may be edited and conversations may branch, but completed actions
MUST NOT be rewritten. File changes, commands, test results, task revisions,
context epochs, repository snapshots, and verification events MUST be
represented through an append-only event history. Derived state MAY be
rebuilt at any time; historical evidence MUST be retained.

Rationale: Replayable, immutable history is the foundation for audit,
drift detection, and deterministic reconstruction of derived state.

### IV. Evidence Before Confidence

AI confidence is not verification. Important claims MUST carry evidence,
applicability, scope, freshness, and invalidation dependencies. Completion
states MUST distinguish implemented, partially verified, verified, blocked,
and failed. An agent saying "done" is never sufficient evidence of
completion.

Rationale: Treating assertion as verification is the primary failure mode
Cairn is built to prevent.

### V. Automatic Operation with Exceptional Human Intervention

Routine repository understanding, memory extraction, classification,
deduplication, revalidation, checkpointing, context compilation,
contradiction handling, and drift correction MUST be automated. Human input
is reserved for destructive operations, business-policy ambiguity,
security-sensitive decisions, and major irreversible architecture changes.

Rationale: A system that demands routine human babysitting will be bypassed;
a system that acts destructively without humans will be distrusted.

### VI. Goal Stability and Controlled Adaptation

The task goal and acceptance criteria are stable within a task revision.
Plans and hypotheses MAY change when evidence changes. Cairn MUST
distinguish valid replanning from goal, scope, instruction, architecture,
context, assumption, and completion drift, and MUST classify which kind
occurred when divergence is detected.

Rationale: Adaptation is healthy only when the fixed point (the goal) is
known; without it, drift and progress are indistinguishable.

### VII. Local Repository Truth

The native daemon is responsible for local repository and execution truth:
Git state, uncommitted changes, file fingerprints, agent lifecycle, and
offline events. The central server is responsible for shared durable state.
The server MUST NOT present uncommitted local state as known unless a daemon
observed and reported it.

Rationale: Only the machine hosting the working tree can observe it;
pretending otherwise fabricates evidence and violates Principle I.

### VIII. Deterministic Analysis Before AI Interpretation

Deterministic mechanisms MUST be used for Git state, hashes, dependency
versions, branch state, event ordering, exit codes, API schema changes, and
file relationships. AI is used only where semantic interpretation is
required. All AI-produced structured data MUST be schema validated and
treated as untrusted until supported by evidence.

Rationale: Deterministic answers are cheaper, reproducible, and correct;
spending model inference on them adds cost and error for no benefit.

### IX. Minimal Reliable Infrastructure

Cairn begins as a Rust modular monolith with a native Rust daemon,
PostgreSQL as the central source of truth, SQLite for daemon-local durable
state, and filesystem artifact storage. New infrastructure — embeddings,
pgvector, Valkey, Garage, graph databases, message brokers, microservices —
MUST NOT be added without a measured, documented need. Speculative
infrastructure is prohibited.

Rationale: Every additional moving part is an operational liability;
capability must be earned by evidence of need, not anticipated.

### X. Privacy and Secret Containment

Cairn MUST NOT persist credentials, private keys, environment-variable
values, or unnecessary source content. Collection and model submission MUST
respect ignore rules, secret detection, redaction, project boundaries, and
least-data principles.

Rationale: Cairn observes repositories and agent sessions at high fidelity;
without strict containment it becomes the highest-value leak target in the
toolchain.

## Additional Technical Constraints

- Rust with Tokio, Axum, Tower, Serde, SQLx, and tracing for the daemon,
  CLI, MCP server, central server, workers, and core engines.
- PostgreSQL is the authoritative central datastore.
- SQLite is used for daemon-local sessions, snapshots, offline events, and
  synchronization state.
- Next.js App Router with strict TypeScript, shadcn/ui, Tailwind, and
  TanStack Query for the web dashboard.
- MCP over stdio is the primary agent interface.
- HTTPS JSON is used for daemon-server durable synchronization.
- Server-Sent Events are used initially for server-to-client updates.
- Git CLI is preferred initially for repository-state inspection.
- No workspace hierarchy: direct users, projects, and project membership.
- Only the Admin role exists initially.
- Tests MUST cover event replay, snapshot determinism, offline
  synchronization, task revisioning, context restoration, and scope
  isolation.
- Logs MUST be structured and MUST NOT contain sensitive repository content
  or secrets.

## Development Workflow and Quality Gates

- Features MUST be delivered as vertical slices with explicit user stories
  and independently testable acceptance criteria.
- Every feature that changes stored state REQUIRES migrations and rollback
  or compatibility consideration.
- Protocol changes REQUIRE contract tests between daemon, server, MCP, and
  TypeScript clients.
- Repository fixtures and deterministic agent simulations MUST be used for
  lifecycle testing.
- The smallest correct architecture is preferred; speculative infrastructure
  is prohibited (see Principle IX).
- A feature is not complete unless its acceptance criteria have observable
  evidence (see Principle IV).
- Constitution conflicts MUST be reported rather than silently bypassed.

## Governance

This constitution supersedes generated plans, generated documentation, and
implementation convenience. When a plan, spec, task list, or implementation
choice conflicts with this document, the conflict MUST be surfaced and
resolved — never silently bypassed.

Amendment procedure: an amendment REQUIRES a documented reason, an impact
analysis across dependent templates and artifacts, consideration of
migration consequences for stored state and protocols, and a version update
recorded in this file.

Versioning policy: semantic versioning of this document. MAJOR for
backward-incompatible governance changes or principle removals or
redefinitions; MINOR for new principles or materially expanded guidance;
PATCH for clarifications and non-semantic refinements.

Compliance review: every plan produced via the Constitution Check gate MUST
verify conformance with these principles before design begins and again
after design completes. Complexity that violates Principle IX (Minimal
Reliable Infrastructure) MUST be justified in the plan's Complexity Tracking
table with measured evidence; unjustified violations block the plan.

Amendment record:

- **2026-07-19 — v1.1.0**: Clarified Principle II with the explicit
  `local_unbound` bootstrap-session exception, append-only future binding,
  and a migration gate. This resolves feature sequencing without removing
  the project/task-revision invariant.

**Version**: 1.1.0 | **Ratified**: 2026-07-16 | **Last Amended**: 2026-07-19
