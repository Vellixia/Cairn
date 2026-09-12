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
be justified by a requirement that exists today, not by anticipated scale. What
this forbids is new infrastructure that holds state or coordinates work — another
place for truth to live, or another thing that must be running for Cairn to be
correct. A stateless computation Cairn calls out to, which stores nothing,
coordinates nothing, and whose absence degrades a feature rather than breaking the
system, is a dependency to be justified and disclosed under Principle V, not a
component of the architecture.
Speculative machinery — embeddings, decay models, confidence engines, distributed
coordination — is out of scope until real usage demonstrates need. A *graph* is
out of scope as a datastore and as a modelling ambition; a bounded view over
relations Cairn already records, rendered on demand and authoritative for
nothing, is a rendering of existing data and is not machinery.

A cache is permitted where a component that is not the owner of data must survive
the owner being unreachable. It must be bounded, must be distinguishable from the
authoritative copy wherever it is read, and must have a stated refill and
invalidation policy. A cache that cannot be told apart from the truth is not a
cache tier; it is a second owner.
Extensibility is achieved through clean schema and API boundaries, not through
pre-built abstraction layers.

Work that must happen without a user waiting for it — consolidating captured
activity into knowledge, draining a queue, reclaiming a stale claim — is ordinary
engineering inside the existing processes, not grounds for a new one. Deferred
work is justified by a requirement it serves and is bounded, observable, and
restartable; a background pass whose backlog and failures cannot be reported is
not permitted.

### III. Fail-Soft Capture, Server-Authoritative Knowledge

Cairn must never block, slow, or break the coding agent it is attached to.
Agent-facing operations fail soft, and this is the principle's first and
non-negotiable clause: a Cairn that is unreachable, degraded, or wrong must still
leave the developer's agent working normally.

The server is the canonical owner of durable knowledge. The local store is edge
state — a spool for work not yet accepted, machine-local integration and
operational state, and a bounded cache. Raw material read during capture lives in
memory for the length of that work and is not a storage role. It is not a second knowledge universe, and it does not hold an
authoritative copy that the server must later be reconciled against.

While the server is unreachable, capture and privacy processing continue and safe
events queue locally for later retry; replay is idempotent and creates no
canonical fork. Server-dependent capability — freshly derived knowledge,
cross-device personal knowledge, team knowledge, consolidation, the web control
plane — may degrade, and degradation is reported rather than disguised. Cached
knowledge served during an outage is labelled as cached. What may never happen is
that Cairn's own unavailability becomes the agent's problem.

Deleting the local store must not destroy knowledge the server has accepted.
Whatever a local store holds that would not survive its deletion must be
nameable — Cairn states which categories are lost rather than reporting an
unqualified success.

### IV. Explicitly Domained Memory

All durable knowledge carries an explicit domain — project, personal, or team —
and no knowledge is domain-less. Within the project domain, knowledge additionally
carries explicit scope (project, branch, task, or session) and explicit provenance
back to the session and to the observations or safe events that produced it. Memory
is never *ambient*:
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
memory, or session.

Sharing is a choice about a boundary, made once and revocably, not a decision
retaken per record. A user who connects Cairn to a server has chosen that safe,
privacy-filtered structure derived from their work crosses to it continuously and
without further prompting — that is what connecting means, and Cairn must say so
plainly at the point of connection. What remains per-record is the reverse: the
user may exclude paths and content, mark knowledge local-only, and delete
anything. Automatic egress of approved structure is compatible with this
principle; automatic egress of anything the gate has not approved is not.

Raw observations, evidence, verification runs, absolute local paths, tool output,
raw vendor payloads and machine configuration never leave the machine that
produced them. Repository-relative file identity is not in that set — it already
travels as part of a handoff — but it crosses only under an explicitly defined
field, never by reusing a name a boundary refuses.

Rich source material may be processed transiently on the machine that produced
it; only structure the privacy boundary has approved may be transmitted, and the
transient material does not outlive the work that created it.

Structured information derived from private material may cross the boundary where
it passes a deterministic gate that fails closed. Derivation is not a loophole: the
gate is evaluable without a database, a clock, or a network, so it can be tested
exhaustively; a refusal names the check that refused and is structurally incapable
of carrying the material it refused; and there is exactly one implementation of
each rejection class, so two entry points cannot drift apart.

Semantic extraction and privacy enforcement are different acts. A model may propose
what captured activity means. A model may never be the gate that decides whether
material is safe to transmit or persist — that decision is deterministic, or it does
not happen.

Extraction may read only what the gate already approved, and reading it is a second
boundary, not a consequence of the first. That a record was permitted to reach the
user's own server does not by itself permit forwarding it to anyone else. Where
extraction is performed by a party other than the server operator, that party is
named, its use is disclosed as plainly as the server connection itself, and it
receives material scoped the way every other reader is scoped — one project, one
account context, never a corpus. "It had already left the machine" is not a reason;
it is the derivation-as-loophole argument this principle exists to refuse.

Moving knowledge from one domain to a wider one is always an explicit act, never a
side effect. Structural prevention is preferred to procedural rules: a record that
has no column for a secret cannot carry one.

### VI. Deterministic Data Boundaries

Repository state (project, branch, commit, worktree, working-tree status) is
derived from Git, not guessed. Identity, scope, and sync are keyed by stable
identifiers. Synchronization is idempotent: replaying a sync converges to the
same state. The same inputs must produce the same context briefing; where a
deadline may reduce a briefing, the reduction is to one of a small set of declared
levels and the level reached is itself an input, so that latency changes which
briefing is produced but never produces an undeclared one.

Idempotency extends to everything the edge may retry and everything Cairn derives
without being asked. Delivering the same captured event any number of times yields
at most one canonical event; consolidating the same accepted events again yields no
additional knowledge, no additional relations, and no additional reinforcement.
Neither ordering nor identity may depend on a local clock being accurate or
monotonic.

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

### IX. Autonomy Under Governance

Cairn learns from what it captures without waiting to be asked, and delivers what
it knows without waiting to be asked. Normal useful memory must not depend on an
agent remembering to call a tool. The explicit surface remains, as an override and
a way to assert something immediately — not as the only path to usefulness.

Autonomy does not transfer authority. A model may propose what was learned; Cairn
decides what becomes durable, under the same deterministic rules — domain and scope
resolution, the privacy gate, duplicate handling, reconciliation, provenance and
verification — that govern anything a human asserts. Knowledge produced
autonomously is distinguishable from knowledge a human asserted. Verification is
derived from evidence, never claimed by the process that proposed the knowledge.
Proposing remains distinct from ratifying: what an automatic process may do, it may
do only up to the point where a human's authority begins.

### X. Report What Is Established, Not What Is Assumed

Cairn's account of itself is evidence-based. Configuration that matches is not
runtime capture; writing to a channel is not the agent having consumed it; a
capability a vendor documents is not a capability observed to work here. These are
reported as the different things they are, and status is derived from recorded
evidence rather than asserted alongside it.

Where a vendor offers no way to establish an outcome, Cairn reports that no evidence
is available. "Unknown" is a truthful answer and is always preferred to a
confirmation Cairn cannot justify. A failure is visible with the stage at which it
failed rather than absorbed into a healthy total.

### XI. Identity Is Established, Never Asserted

The identity an authorization decision depends on comes from the authenticated
caller, never from the payload being authorized. A request may say what it is
about; it may not say who it is from. Ownership, authorship, proposal attribution
and membership are bound from the credential the request arrived under and are
re-verified against server-side state before use.

An identifier inside a payload — a session, a project, a record — is a claim about
what the request concerns, and is verified to exist and to belong to the caller
before it is trusted. Where a process acts without a human caller, it is attributed
to itself and never to a user whose data it happened to read.

This principle records a rule the codebase already enforces and paid for. It is
written down because it was learned the expensive way, and because every new
write path is an opportunity to forget it.

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
  team-wide guidance authoritative. This holds for an automatic process exactly as
  it holds for an agent.
- Each supported agent is captured through an adapter written for that vendor's own
  event and payload vocabulary. No vendor's payload shape is the language in which
  another vendor's events are expressed, and enough vendor provenance is retained to
  diagnose a capture failure later. Where a vendor does not supply something, that is
  recorded as unavailable; it is never inferred, defaulted or fabricated.
- Cairn's own state and behaviour are visible in the web control plane. A capability
  that exists only as a local tool call, with no way for a team to see or govern it,
  is an unfinished capability.

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

### 1.2.1 — 2026-08-30

Adopted while closing Feature 005's remaining architectural questions. One of those
decisions — that semantic extraction runs on the server and may be performed by a
model, including a hosted one — outran Principle V as amended in 1.2.0, and the gap
is closed here rather than in the specification.

- **Principle V** gained the extractor boundary. v1.2.0 established that sharing is a
  choice about a boundary, made once at the point of connection: the user's machine to
  the user's server. A hosted extractor is a *second* recipient at a *second* boundary,
  disclosed at neither. The specification's own reasoning for permitting it — that the
  material had already legitimately left the machine, so no new egress occurs — is
  precisely the derivation-as-loophole move that 1.2.0 tightened Principle V to refuse,
  and it was not sound. The principle now requires that reading approved material is
  itself a boundary, that a third-party extractor be named and disclosed as plainly as
  the server connection, and that it be scoped like every other reader — one project,
  one account context, never a corpus.

- **Principle II** was clarified on what "a new service" forbids. The ban is on new
  infrastructure that holds state or coordinates work: another place for truth to live,
  or another thing that must be running for Cairn to be correct. A stateless computation
  that stores nothing, coordinates nothing, and whose absence degrades a feature rather
  than breaking the system is a dependency to be justified and disclosed under Principle
  V, not a component of the architecture. Without this, the principle read as forbidding
  an extraction model outright while the same document permitted one, and the feature's
  own success criterion asserting "zero service dependencies" was a third reading again.

- Consolidation as in-process background work needed no amendment: v1.2.0's Principle II
  already names consolidation as deferred work belonging inside the existing processes,
  and requires it to be bounded, observable and restartable. Decision 4's `repo_file`
  needed none either: v1.2.0's Principle V already states that repository-relative file
  identity crosses "only under an explicitly defined field, never by reusing a name a
  boundary refuses". Both are recorded here as checked rather than assumed.

### 1.2.0 — 2026-08-29

Adopted with Feature 005 (Server-Authoritative Autonomous Memory), which moves
authority for durable knowledge to the server and makes capture and recall
autonomous. This amendment is deliberately explicit: Feature 005 contradicts
v1.1.0 as written, and that conflict is resolved here rather than in an
implementation note.

- **Principle III** was retitled from "Local-First Reliability" to "Fail-Soft
  Capture, Server-Authoritative Knowledge", and its central requirement is
  reversed. v1.1.0 required that "Cairn must be fully useful offline" and that
  "capture, recall, context assembly, handoff, and search must work with no
  network and no central server", with remote sync "an enhancement layered on top
  of a complete local system". Feature 005 makes the server the canonical owner of
  durable knowledge, so full offline equivalence is no longer promised.

  What the old principle was protecting is retained and moved to the front, where
  it belongs: Cairn must never block, slow or break the coding agent. That clause
  was always the load-bearing one — "fully useful offline" was one way of
  guaranteeing it, and a costly one, because it required a complete second
  knowledge system on every machine whose only purpose was to be reconciled away.
  The new principle keeps the guarantee and drops the mechanism: the agent keeps
  working, capture continues, safe events queue and retry idempotently, and no
  canonical fork is created. Server-dependent capability may degrade, and must say
  so. Cached knowledge must be labelled as cached.

  The principle also gains the durability invariant the authority change exists to
  make: deleting the local store must not destroy knowledge the server has
  accepted, and whatever would not survive must be nameable rather than lost
  silently.

- **Principle V** gained the derivation rule. v1.1.0 stated flatly that "raw
  observations, evidence, verification runs, local paths, tool output and machine
  configuration never leave the machine that produced them". That sentence is
  retained and widened to name raw vendor payloads explicitly. What is added is the
  distinction it did not draw: *derived* structure, approved by a deterministic
  fail-closed gate, may cross the boundary even though the material it was derived
  from may not. Without this, Feature 005's entire premise — that rich capture can
  be transformed locally into safe events — is unconstitutional; with it stated
  loosely, "derived" becomes a loophole that swallows the principle. So the gate's
  properties are made binding: evaluable with no database, clock or network;
  refusals that name the check and cannot carry the refused material; exactly one
  implementation of each rejection class. Transient material must not outlive the
  boundary.

  The principle also now separates semantic extraction from privacy enforcement. A
  model may propose what activity means. A model may never be the gate. This is
  stated as a principle because it is the one place where adding intelligence to
  Cairn could quietly remove a guarantee.

- **Principle II** was changed in three places. Deferred work — consolidation, queue
  draining, stale-claim reclaim — is now explicitly ordinary engineering inside the
  existing processes, bounded, observable and restartable; the server had no
  background execution of any kind, so the principle would otherwise have forbidden
  the work this feature requires. The word "graphs" was removed from the list of
  speculative machinery and replaced with a narrower statement: a graph is still
  forbidden as a datastore and as a modelling ambition, but a bounded on-demand view
  over relations Cairn already records, authoritative for nothing, is a rendering of
  existing data rather than machinery. And "a cache tier" — previously listed as
  infrastructure requiring justification — is now permitted under stated conditions,
  because moving authority to the server makes a local cache the mechanism by which a
  non-owner survives the owner being unreachable. The conditions are the point: a
  cache must be bounded, distinguishable from the authoritative copy wherever it is
  read, and have a stated refill and invalidation policy. A cache that cannot be told
  apart from the truth is a second owner, which is exactly what this feature exists to
  eliminate. The ban on new brokers, datastores, services and the remaining
  speculative machinery stands untouched.

- **Principle VI** gained idempotency for captured events and for consolidation, and
  the rule that neither ordering nor identity may depend on a local clock. v1.1.0
  required idempotent *synchronization*; Feature 005 adds two more things that are
  replayed and derived without being asked, and they need the same guarantee. The
  determinism sentence — "the same inputs must produce the same context briefing" —
  was also qualified rather than left to be quietly broken: Feature 005 lets a
  retrieval deadline reduce a briefing, and wall-clock latency is not an input. The
  principle now requires reduction to one of a small set of declared levels, with the
  level reached counting as an input. Latency may change which briefing is produced;
  it may never produce an undeclared one.

- **Principle IV** had its provenance target widened from "the session and observations
  that produced it" to "the session and to the observations or safe events that
  produced it". Observations remain local-only under this feature, so knowledge
  consolidated on the server grounds out in safe events instead; without this the
  provenance requirement would have been unsatisfiable for exactly the knowledge this
  feature exists to create. Nothing else in Principle IV changed.

- **Principle IX (new)** — "Autonomy Under Governance" — states that Cairn learns and
  recalls without being asked, and that doing so transfers no authority. It is the
  principle that makes "LLM proposes, Cairn governs" binding rather than aspirational,
  and it preserves the proposing/ratifying distinction against an automatic process,
  which v1.1.0's product constraint addressed only for agents.

- **Principle X (new)** — "Report What Is Established, Not What Is Assumed" — requires
  Cairn's account of itself to be evidence-based, and makes "unknown" a truthful and
  preferred answer where a vendor cannot establish an outcome. This was previously
  implicit in Principle VII's rule against vacuous tests; Feature 005 makes Cairn
  report on its own runtime health at scale, which is a large enough new surface for
  honest reporting to need stating directly.

- **Principle V** was changed a second way, on user choice. v1.1.0 said "data leaves
  the machine only when the user has chosen to share it", which a feature that
  transmits derived structure continuously and without prompting cannot satisfy as
  written. Rather than delete the sentence, the amendment states what the choice
  actually is: sharing is a choice about a boundary, made once and revocably at the
  point of connection, not a decision retaken per record — and Cairn must say so
  plainly when the connection is made. The per-record controls that remain are the
  reverse ones: exclude, mark local-only, delete. Automatic egress of gate-approved
  structure is compatible with this; automatic egress of anything unapproved is not.
  Principle V's prohibition list also now says *absolute* local paths, because
  repository-relative paths already travel today as part of a handoff
  (`changed_files`), and a principle that forbade what the product already does would
  be a dead letter rather than a constraint.

- **Principle XI (new)** — "Identity Is Established, Never Asserted" — writes down the
  rule that authorization identity comes from the authenticated caller and never from
  the payload, that an identifier inside a payload is a claim to be verified rather
  than trusted, and that a process acting without a human caller is attributed to
  itself. This was already enforced in the codebase and was never stated. It is stated
  now because Feature 005 adds several new write paths, each of which is a fresh
  opportunity to forget a rule that took Feature 004 six rounds of review to get right.

- **Feature 003's FR-321** — "explicit only; never inferred from a matching value key" —
  is superseded in one respect. Feature 005's FR-801a permits an automatic
  reinforcement relation, but *only* on a deterministic identity match, which is the
  same basis that already licenses automatic duplicate and conflict detection.
  Reinforcement on a model's judgement remains forbidden, and automatic supersession
  remains forbidden entirely. This is recorded here, as the 1.1.0 amendment recorded
  its supersession of FR-391, so that the change is discoverable from the constitution
  rather than only from a requirement.

- **Product Constraints** gained: the proposing/ratifying rule extends to automatic
  processes; capture is vendor-native by adapter with provenance retained and nothing
  fabricated; and a capability with no control-plane surface is unfinished.

- Principles I, VII and VIII are unchanged. In particular Principle VIII
  ("Project Truth Is Not Displaceable") and Principle IV's domain separation apply to
  autonomously produced knowledge exactly as they apply to knowledge a human asserts;
  Feature 005 adds no exception to either.

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

**Version**: 1.2.1 | **Ratified**: 2026-08-07 | **Last Amended**: 2026-08-30
