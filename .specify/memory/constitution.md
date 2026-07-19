<!--
Sync Impact Report
==================
Version change: (template, unversioned) → 1.0.0
Rationale: Initial ratification of the Cairn constitution. MAJOR set to 1
because this establishes the first complete, binding governance baseline.

Modified principles: n/a (all placeholders replaced on first fill)

Added sections:
- Core Principles (10 principles: I. Observable Reality Is Authoritative,
  II. Exact Execution Scope, III. Append-Only Execution History,
  IV. Evidence Before Confidence, V. Automatic Operation with Exceptional
  Human Intervention, VI. Goal Stability and Controlled Adaptation,
  VII. Local Repository Truth, VIII. Deterministic Analysis Before AI
  Interpretation, IX. Minimal Reliable Infrastructure, X. Privacy and
  Secret Containment)
- Additional Technical Constraints
- Development Workflow and Quality Gates
- Governance

Removed sections: none (template example comments removed after replacement)

Templates requiring updates:
- ✅ .specify/templates/plan-template.md — "Constitution Check" gate and
  "Complexity Tracking" table already align (gates derived from this file;
  complexity justification satisfies Principle IX and Governance).
- ✅ .specify/templates/spec-template.md — prioritized, independently
  testable user stories + measurable success criteria satisfy the
  vertical-slice and observable-evidence gates. No change needed.
- ✅ .specify/templates/tasks-template.md — story-scoped, independently
  testable phases align. Note: constitution requires migration/rollback
  tasks for state-changing features and contract tests for protocol
  changes; /speckit-tasks must include them when applicable.
- ✅ .specify/templates/checklist-template.md — generic, no conflict.

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

Every agent session MUST be associated with exactly one user, project,
repository, task revision, branch, commit, and working-tree snapshot.
Project, branch, task, and session knowledge MUST remain separately scoped.
Information from one scope MUST NOT silently become truth in another; any
cross-scope promotion MUST be an explicit, recorded operation.

Rationale: Knowledge that leaks across scopes silently corrupts every
consumer of that scope; explicit promotion keeps provenance intact.

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

**Version**: 1.0.0 | **Ratified**: 2026-07-16 | **Last Amended**: 2026-07-16
