# Feature Specification: Server-Authoritative Autonomous Memory

**Feature Branch**: `005-server-authoritative-autonomous-memory`

**Created**: 2026-08-29

**Status**: Draft

**Input**: Cairn becomes a server-authoritative autonomous memory system for AI coding
agents. It captures useful work richly and vendor-natively, transforms it through a local
privacy boundary into safe structured events, persists canonical knowledge centrally,
consolidates activity into durable knowledge without being asked, retrieves relevant
knowledge back into agent sessions automatically, measures truthfully whether capture and
delivery actually worked, and exposes the whole lifecycle through the web interface.

**Baseline**: `origin/main` @ `f76a9fec8a786a76dc7ffa1b0b0daf96aae08b15`. Feature 004
(Collaborative Global Memory) is merged and is treated throughout as existing production
architecture. Findings grounding this specification are recorded in [research.md](./research.md).

**Requirement numbering**: this feature uses **FR-701+** and **SC-701+**. Identifiers are
allocated in semantic blocks, so gaps between blocks are deliberate and ids are not monotonic
with document order — the same convention Feature 004 recorded. The house habit
of matching the leading digit to the feature number is deliberately not followed: Features
003 and 004 already both occupy the FR-401–FR-519 band, and a third occupant would compound
an existing collision. FR-701 is the nearest clean start.

---

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Work becomes knowledge without anyone asking (Priority: P1)

A developer and their agent investigate a defect on a real repository. The agent reads the
relevant code, forms a technical conclusion, changes the implementation, and runs the test
suite until it passes. Nobody types `cairn_remember`. Nobody types `cairn_search`. When the
session ends, Cairn has nevertheless learned something durable and correct about this
project: that the defect existed, what was decided about it, and what procedure now holds.
Each piece of knowledge can name the session and the events it came from.

**Why this priority**: This is the feature. Every other story assumes durable knowledge
exists without a human remembering to create it. Today no such path exists at all — every
durable memory on `main` traces to an explicit tool call — so this story is both the
largest change and the one that makes the rest worth building.

**Independent Test**: On a fresh project with zero memories, drive one real coding session
through a supported agent without invoking any Cairn tool, then assert that durable
knowledge exists, that its provenance resolves to that session, and that a reviewer reading
only the resulting knowledge can state what was learned.

**Acceptance Scenarios**:

1. **Given** a fresh project with no durable knowledge and a connected supported agent,
   **When** a session performs an investigation, a code change and a passing test run
   without invoking any Cairn tool, **Then** at least one durable memory exists whose
   provenance names that session and the safe events it was derived from.
2. **Given** that session's activity, **When** consolidation runs, **Then** every durable
   record it produces carries one of the five existing knowledge kinds — fact, decision,
   convention, failure or procedure — and no new kind is introduced.
3. **Given** a candidate produced by semantic extraction, **When** it fails any
   deterministic privacy or content check, **Then** no durable record is created, the
   refusal names the check that stopped it, and the refusal record contains none of the
   rejected text.
4. **Given** the same set of safe events, **When** consolidation is run over them a second
   time, **Then** no additional durable knowledge is created and no reinforcement count
   changes as a result of the repeat.
5. **Given** a session in which the agent did nothing durable-worthy, **When** consolidation
   runs, **Then** it produces no knowledge rather than inventing some.

---

### User Story 2 - A second session starts already knowing (Priority: P2)

A different session — possibly a different agent, possibly on a different machine — begins
work related to what the first session learned. Without anyone searching Cairn first, the
relevant prior knowledge is selected, assembled into a bounded briefing, and delivered onto
that agent's own context surface. Project knowledge comes first and is never crowded out by
broader personal or team guidance.

**Why this priority**: Capture without recall is a write-only database. This story is what
makes the captured knowledge pay for itself, and it depends only on P1 having produced
something.

**Independent Test**: With durable knowledge present from Story 1, open a new session in a
related area through a supported agent, invoke no Cairn tool, and assert that the delivered
context contains the relevant prior knowledge, stays inside the budget, and orders project
knowledge ahead of personal and team guidance.

**Acceptance Scenarios**:

1. **Given** durable project knowledge relevant to the work at hand, **When** a new session
   opens through a supported agent, **Then** a bounded briefing containing that knowledge
   reaches the agent's context surface with no tool call by the user or the agent.
2. **Given** a briefing being assembled, **When** project knowledge alone would fill the
   budget, **Then** personal and team guidance occupy no part of it, and the briefing never
   exceeds its stated budget.
3. **Given** a briefing was assembled, **When** a user inspects the retrieval afterwards,
   **Then** the trigger, the knowledge considered, the knowledge selected, the reason each
   selection was made, and the delivery outcome are all recoverable.
4. **Given** an agent whose vendor offers no way to confirm receipt, **When** context is
   transmitted to it, **Then** Cairn records transmission as attempted and receipt as
   unavailable, and never reports receipt as confirmed.

---

### User Story 3 - Losing a machine does not lose the knowledge (Priority: P3)

A developer's laptop dies, or they delete Cairn's local database to clear a problem. What
they lose is local: queued work that never reached the server, machine-specific integration
configuration, caches and diagnostics. What they do not lose is the project's knowledge,
their own personal knowledge, or the team's guidance. Those were accepted by the server and
are still there.

**Why this priority**: This is the promise the authority change exists to make. It is
ordered after capture and recall because it is only meaningful once there is knowledge worth
not losing.

**Independent Test**: Drive knowledge into the system, confirm the server has accepted it,
destroy and recreate the local store, then assert that project, personal and team knowledge
are all still reachable and that only machine-local state needed rebuilding.

**Acceptance Scenarios**:

1. **Given** durable project, personal and team knowledge the server has accepted,
   **When** the local database is deleted and recreated, **Then** all of that knowledge is
   reachable again without any manual recovery step.
2. **Given** the same deletion, **When** the user inspects what was lost, **Then** Cairn can
   name the categories that did not survive — undelivered queued events, machine-local
   integration state, caches and diagnostics — rather than reporting silent success.
3. **Given** knowledge a user explicitly marked local-only, **When** the local database is
   deleted, **Then** that knowledge is gone, and the interface that offered the local-only
   choice stated that consequence at the time the choice was made.
4. **Given** events still queued locally when the machine is lost, **When** recovery
   completes, **Then** those events are absent and are reported as lost rather than being
   silently treated as delivered.

---

### User Story 4 - The server goes away and the agent keeps working (Priority: P4)

Cairn Server becomes unreachable mid-session. The developer's coding agent does not stall,
slow down, or error. Capture continues; safe events accumulate locally and are retried when
the server returns. Freshly-derived knowledge and cross-device features degrade, and say so.
When the server comes back, replay produces no duplicates and no second universe of truth.

**Why this priority**: Cairn is attached to something more important than itself. This story
protects the agent, and is ordered here because it constrains rather than creates
capability.

**Independent Test**: Interrupt the connection to the server mid-session, continue the
agent's work, restore the connection, and assert that the agent was never blocked, that
queued events drained exactly once, and that no knowledge was created locally that competes
with the server's.

**Acceptance Scenarios**:

1. **Given** an unreachable server, **When** the agent performs tool calls and lifecycle
   transitions, **Then** the agent's own work completes normally and no Cairn operation
   blocks it beyond its stated deadline.
2. **Given** an unreachable server, **When** safe events are produced, **Then** they are
   spooled locally and no durable knowledge is created locally that the server has not
   accepted.
3. **Given** spooled events and a restored server, **When** delivery retries, **Then** every
   event is accepted exactly once however many times it is replayed.
4. **Given** an unreachable server, **When** a session requests context, **Then** Cairn
   either serves a cached briefing explicitly labelled as cached and possibly stale, or
   reports that fresh knowledge is unavailable — never presenting cached content as current.

---

### User Story 5 - A user can see what Cairn learned, and why (Priority: P5)

Through the web interface, a user follows one piece of knowledge end to end: the session
that produced it, the events behind it, the consolidation run that proposed it, the decision
that accepted it, what it reinforces or supersedes or conflicts with, and every later
session it was retrieved into. They can also see, at a glance, whether the whole pipeline is
healthy: what is arriving, what is being learned, and what is failing.

**Why this priority**: Knowledge nobody can audit is knowledge nobody should trust. This
story turns the system from a black box into something a team can govern, and it depends on
the traces the earlier stories produce.

**Independent Test**: After Stories 1 and 2, use only the web interface to reconstruct the
full path from the originating session to the delivered briefing, without reading a log or
querying a database by hand.

**Acceptance Scenarios**:

1. **Given** knowledge produced by consolidation, **When** a user opens its detail view,
   **Then** they can see its content, kind, domain, scope, state, provenance, evidence
   summary, verification state, relations, reinforcement, and where it has been retrieved.
2. **Given** a completed retrieval, **When** a user opens its trace, **Then** they can see
   the trigger, the candidates considered, the selection and its explanation, and the
   delivery outcome.
3. **Given** an active installation, **When** a user opens the dashboard, **Then** they see
   the memory funnel from events received through candidates produced to knowledge accepted,
   together with failures and backlogs at each stage.
4. **Given** knowledge that supersedes or conflicts with other knowledge, **When** the user
   asks to see those connections, **Then** a bounded relation view is shown over Cairn's own
   existing relation kinds, and it is presented as a derived view rather than as the source
   of truth.
5. **Given** privacy rules that keep some material local, **When** any web view renders,
   **Then** it shows what the server legitimately holds and states plainly where local-only
   material has been withheld.

---

### User Story 6 - Status tells the truth, including when it does not know (Priority: P6)

An operator asks whether an integration is working. Cairn distinguishes what it configured
from what actually ran, and what it sent from what was actually received. Where a vendor
offers no confirmation, Cairn says so instead of showing a green check it cannot justify.

**Why this priority**: A false green is worse than a blank. This story is ordered late
because it reports on the pipeline the earlier stories build, but it is not optional: it is
what makes the other five auditable.

**Independent Test**: Configure an integration without exercising it and assert Cairn does
not claim runtime capture; then exercise it and assert the status advances only on observed
evidence.

**Acceptance Scenarios**:

1. **Given** an integration whose configuration is installed and matches, **When** no
   runtime event has ever been received from it, **Then** its status distinguishes
   configuration from runtime capture, and does not report capture as working.
2. **Given** a capability a vendor does not expose, **When** health is reported, **Then** it
   is shown as unavailable from the vendor and is distinguished from a capability Cairn
   declined, one whose adapter is unimplemented, and one that failed at runtime.
3. **Given** context transmitted to an agent that cannot acknowledge receipt, **When**
   delivery status is reported, **Then** receipt is reported as unavailable rather than
   confirmed or failed.
4. **Given** a capture attempt that failed, **When** health is reported, **Then** the failure
   is visible with its stage, rather than being absorbed into an overall healthy status.

---

### User Story 7 - An existing installation migrates without losing anything (Priority: P7)

A developer already running Feature 004 upgrades. Their existing project memories, personal
knowledge, team knowledge and relations are inspected, any eligible pending writes are
drained, the server is confirmed to hold canonical copies, and only then does authority
switch. If any part of that cannot be completed safely, migration stops in a state the user
can see and act on, rather than proceeding.

**Why this priority**: Last in order, but blocking for release: shipping the authority change
without it would destroy real users' knowledge. It is ordered here because its correctness
is defined in terms of everything the earlier stories establish.

**Independent Test**: Start from a populated Feature 004 local store at schema v7, run the
migration, and assert that every pre-existing record is either confirmed present on the
server or explicitly reported as unmigrated, with authority switching only if the former.

**Acceptance Scenarios**:

1. **Given** a populated local store at the Feature 004 schema, **When** migration runs,
   **Then** no project memory, personal knowledge or team knowledge is lost, and none has its
   author, domain or scope reassigned.
2. **Given** pending or blocked outbox rows at migration time, **When** migration runs,
   **Then** eligible rows are drained before authority switches and ineligible rows are
   reported individually rather than discarded.
3. **Given** a migration interrupted at any point, **When** it is run again, **Then** it
   resumes without creating duplicate canonical knowledge.
4. **Given** knowledge the server cannot accept — because it is local-only, or because the
   server refuses it — **When** migration runs, **Then** authority does not switch for that
   category, the reason is reported, and the local copy is retained rather than demoted.
5. **Given** a completed and verified migration, **When** obsolete local replicas are
   demoted, **Then** the demotion happens only after canonical possession has been confirmed
   for the records concerned.

---

### Edge Cases

- **A vendor reports a tool call Cairn cannot interpret.** The event is recorded as received
  and unclassified with its vendor provenance preserved, rather than being coerced into the
  nearest canonical shape or silently dropped.
- **A vendor supplies no file path for a file-changing tool.** The event records that a file
  changed and that the path was unavailable from the vendor. It does not fabricate one, and
  it does not degrade the event to a generic command.
- **Semantic extraction proposes a candidate containing a secret.** The deterministic gate
  refuses it. The refusal is counted and its reason is named; the offending text is not
  stored, logged, or transmitted.
- **Semantic extraction is unavailable or fails.** Safe events continue to be captured and
  persisted; consolidation reports a backlog rather than dropping the events it could not
  process.
- **Two consolidation runs propose near-identical knowledge in different words.** The
  deterministic identity of a candidate — not its wording — decides whether it is new, so
  the second run reinforces rather than creating a near-duplicate.
- **Consolidation proposes knowledge that contradicts existing project knowledge.** A
  conflict is recorded. The contradiction is not auto-resolved, and neither record is
  silently superseded.
- **Consolidation proposes team guidance.** It becomes a proposal. Ratification remains a
  human administrator's act.
- **The spool grows without bound while the server is unreachable.** Spooling is bounded by a
  stated policy; when the bound is reached, what is dropped and why becomes visible rather
  than being lost silently.
- **The same event is delivered twice because a response was lost.** The second delivery is
  recognised and produces no second canonical event and no second consolidation input.
- **A user's credentials change, or a second device appears, mid-flight.** Events already
  spooled under the previous identity are not delivered under the new one.
- **The endpoint answers as a different server instance than before.** The client refuses to
  merge the two, exactly as it does today.
- **A project memory was marked local-only.** It never leaves the machine, is excluded from
  the durability guarantee, and is reported as excluded rather than counted as protected.
- **The clock moves backwards between events.** Ordering and idempotency do not depend on the
  local clock being monotonic.
- **A session's events arrive after the session has been reaped.** They are attributed to the
  original session rather than opening a new one or being discarded.

---

## Requirements *(mandatory)*

### Storage authority and durability

- **FR-701**: The server MUST be the canonical owner of durable knowledge in the project,
  personal and team domains. Where the server's accepted state and a local copy disagree
  about a record the server owns, the server's state is correct by definition.
- **FR-702**: The local store MUST NOT be the authoritative owner of any durable knowledge
  that the server is capable of holding. Its role is confined to spooling unsent events,
  holding machine-local integration and operational state, holding transient material for
  the privacy boundary, and holding a bounded non-authoritative cache.
- **FR-703**: Deleting the local store MUST NOT destroy project, personal or team knowledge
  that the server has already accepted.
- **FR-704**: Following local-store loss, Cairn MUST restore server-accepted knowledge
  without requiring the user to perform a manual reconciliation or repair step.
- **FR-705**: Cairn MUST be able to state, for a given local store, which categories of data
  would not survive its deletion. At minimum these are: events spooled but not yet accepted,
  machine-local integration state, cached knowledge, local-only knowledge, and local-only
  diagnostic records.
- **FR-706**: Knowledge a user has explicitly marked local-only MUST be excluded from
  FR-703's guarantee, MUST be reported as excluded wherever durability is summarised, and the
  interface offering the local-only choice MUST state that consequence at the point of
  choosing.
- **FR-707**: Observations, evidence facts, verification runs, continuity checkpoints,
  pattern applications, task change history and criterion evidence MUST remain local-only.
  Feature 005 does not move raw local records to the server.
- **FR-708**: Reusable patterns MUST gain a durable server-backed representation, so that
  project-independent transferable knowledge is covered by FR-703. Pattern *applications*
  remain local under FR-707.
- **FR-708a**: A reusable pattern crossing to the server MUST pass the same content validation
  that governs personal and team knowledge: it MUST NOT name the project it was derived from,
  and its origin MUST remain a machine-local salted digest that is never transmitted.
- **FR-708b**: `reusable_pattern` is presently on the synchronization boundary's refused
  entity-type list, and a pattern row additionally carries three refused field names. Making
  patterns durable therefore REQUIRES an explicit, named exception rather than an implicit one:
  the refusal MUST be lifted only for a pattern representation redefined to carry none of the
  refused fields, and lifting it MUST be recorded as a deliberate narrowing with the replacement
  representation stated. Removing the entity from the refusal list without redefining what it
  carries is forbidden.
- **FR-709**: Cairn MUST NOT create durable knowledge locally that the server has not
  accepted and that has no path to acceptance. A local record awaiting delivery is a queued
  write, not an alternative truth, and MUST be represented as such.
- **FR-710**: Cached knowledge held locally MUST be distinguishable from server-accepted
  knowledge at every point where it is read or displayed.
- **FR-710a**: The cache MUST have a stated refill and invalidation policy, and Cairn MUST be
  able to report that the cache is empty or stale rather than presenting an empty cache as an
  absence of knowledge. A store whose cache has not refilled after local loss MUST say so.
- **FR-711**: Cairn MUST NOT require a second datastore, message broker, distributed worker
  platform or service decomposition to satisfy any requirement in this feature.
- **FR-712**: Where a record's canonical owner is the server, local mutation of that record
  MUST be expressed as a request the server accepts or refuses, never as a local write the
  server later discovers.
- **FR-712a**: The local merge rules for personal and team knowledge are presently
  insert-once — content is never rewritten by a pull, and team state may only advance. Under
  server authority a local copy MUST be able to accept a server-side correction, including a
  content correction and a state that did not advance monotonically. Repurposing the local
  replicas as a cache therefore requires replacing that merge rule, and FR-701's declaration
  that the server is correct is not self-executing.

### Vendor-native capture

- **FR-717**: Each supported agent MUST be captured through an adapter that reads that
  vendor's own event and payload vocabulary. Cairn MUST NOT require one vendor's payload
  shape to be the vocabulary in which another vendor's events are expressed.
- **FR-718**: An adapter MUST translate vendor-native events into canonical semantic events
  without discarding semantic information that the canonical model is capable of
  representing.
- **FR-719**: Capture MUST NOT be limited to an allowlist of two payload fields. Each adapter
  MUST define, per vendor tool, which vendor fields carry which canonical meaning.
- **FR-720**: Where a vendor supplies a file path under a key other than the one another
  vendor uses, or supplies file identity in another form entirely, the adapter MUST extract
  it. A file-changing tool whose path Cairn failed to extract is a capture defect, not an
  absence.
- **FR-721**: Where a vendor genuinely does not supply information, the canonical event MUST
  record it as unavailable from the vendor. Cairn MUST NOT fabricate, infer or default the
  missing value.
- **FR-722**: Failure MUST continue to be established from what the vendor reported, never
  inferred. An uninterpretable response MUST NOT produce a fabricated failure.
- **FR-723**: Each canonical event MUST retain enough vendor provenance to diagnose a capture
  failure later: at minimum the agent, the vendor's own event name, and the vendor's tool
  name where one applies.
- **FR-724**: Vendor provenance MUST be bounded and MUST NOT be consulted by ranking,
  consolidation, retrieval or context assembly. This MUST be observable: two runs whose inputs
  differ only in vendor provenance MUST produce identical rankings, identical consolidation
  output and identical briefings.
- **FR-725**: Provenance that an adapter captures MUST be persisted. A provenance field that
  is captured and then dropped before storage is a defect.
- **FR-726**: Subagent activity MUST be captured where the vendor exposes it, and MUST be
  attributable to the parent session.
- **FR-727**: Where a vendor exposes a signal that a user gave an instruction, or that a
  decision was reached, the adapter MUST be able to represent it as a canonical event, subject
  to the privacy boundary.
- **FR-728**: Cairn MUST declare, per agent and per canonical event kind, which of the
  following holds: supported, unsupported by the vendor, declined by Cairn, adapter not
  implemented, or failing at runtime.
- **FR-729**: An agent for which no adapter exists MUST remain usable through the explicit
  tool surface, and MUST report its capture capability as absent rather than as healthy.
- **FR-730**: Raw vendor payloads MUST NOT cross the process boundary between the capture
  process and the daemon, and MUST NOT be written to durable local storage. This preserves
  existing behaviour rather than introducing it, and is stated so that richer capture cannot
  erode it.

### The canonical safe semantic event model

- **FR-734**: Cairn MUST define a canonical semantic event model capable of expressing, at
  minimum: session opened, session closed, context compacting, context compacted, agent or
  subagent started, agent or subagent completed, tool started, tool succeeded, tool failed,
  file read, file changed, command executed, test executed, test result, research or
  discovery activity, a user-instruction signal, and a decision signal.
- **FR-735**: The restriction whereby only tool-success and tool-failure events may carry
  content MUST be lifted. An event kind's ability to carry content MUST be a property of that
  kind, not a blanket rule.
- **FR-736**: Content carried by an event MUST be structured and typed. An event MUST NOT
  carry arbitrary vendor JSON.
- **FR-737**: Every canonical event MUST name the session that produced it. An event that
  cannot name its session MUST be declined, as it is today.
- **FR-738**: Every canonical event MUST carry a stable identity that is derived from the
  event itself and is reproducible, so that a redelivery of the same event is recognisable as
  the same event. That identity MUST include a per-session monotonic ordinal, so a genuinely
  repeated act — reading the same file twice, running the same command twice — is a distinct
  event rather than a suppressed duplicate. A counter is not a clock, so this is compatible
  with FR-780.
- **FR-739**: Not every agent must support every event kind. The model MUST make the absence
  explicit through FR-728's vocabulary rather than by omission.
- **FR-740**: Cairn MUST record the disposition of each event across its lifecycle,
  distinguishing at minimum: captured, declined by policy, redaction-failed, spooled,
  transmitted, accepted, rejected by the server, and persisted.
- **FR-741**: A rejected event MUST record why it was rejected in terms of a fixed vocabulary
  of reasons, and MUST NOT record the content that caused the rejection.
- **FR-742**: The canonical event model MUST be versioned, and an event MUST carry the
  version of the contract under which it was produced.
- **FR-743**: Adding a new event kind MUST NOT invalidate previously persisted events.
- **FR-744**: The existing seven-event lifecycle vocabulary MUST remain expressible, so that
  handoff generation, checkpointing and context delivery behaviour established by Features
  001–003 continue to hold.
- **FR-745**: Event content MUST be bounded in size, and the bound MUST be asserted rather
  than assumed.

### The local privacy boundary

- **FR-749**: Rich vendor material MAY be processed transiently on the machine that produced
  it. Only material approved by the privacy boundary may be transmitted.
- **FR-750**: Full conversation transcripts MUST NOT be persisted, locally or centrally. There
  is no configuration under which Cairn stores one; the prohibition is absolute, matching the
  Out of Scope entry that states it.
- **FR-751**: Raw tool output MUST NOT be persisted centrally.
- **FR-752**: Credentials and secrets MUST NOT be persisted anywhere, and MUST be redacted
  before any write, exactly as they are today.
- **FR-753**: Absolute local filesystem paths, machine identifiers and machine configuration
  MUST NOT be persisted centrally.
- **FR-754**: Arbitrary vendor JSON MUST NOT be persisted centrally.
- **FR-755**: The privacy boundary MUST be deterministic and fail closed wherever a
  deterministic check is applicable. Given the same input it MUST produce the same decision.
- **FR-756**: The privacy boundary MUST be evaluable without a database handle, a clock or a
  network call, so that it can be tested exhaustively against an adversarial corpus.
- **FR-757**: A refusal by the privacy boundary MUST identify the check that refused, and the
  refusal record MUST be structurally incapable of carrying the refused content.
- **FR-758**: Where semantic extraction is used to derive structure from event material, it
  MUST be separated from privacy enforcement. A model MUST NOT be the sole or final gate on
  whether material may be transmitted or persisted.
- **FR-759**: Output of semantic extraction MUST pass the same deterministic privacy checks as
  any other candidate content, and MUST be refused on the same terms.
- **FR-760**: Cairn MUST maintain exactly one implementation of each privacy rejection class,
  so that two entry points cannot drift apart.
- **FR-761**: Existing user controls MUST continue to hold: path and content exclusion,
  local-only knowledge, and deletion of any observation, memory or session.
- **FR-762**: Promotion of knowledge from one domain to a wider one MUST remain an explicit
  act passing the existing fail-closed gate. Consolidation MUST NOT promote across domains as
  a side effect.
- **FR-763**: Material held transiently for extraction MUST be discarded once extraction
  completes or fails, and MUST NOT outlive the boundary that created it.
- **FR-763a**: Extraction MUST have an execution home that can actually hold the material it
  extracts from. The per-event capture process is short-lived and runs under a capture deadline
  measured in milliseconds, and FR-730 forbids the raw payload reaching the daemon; extraction
  therefore requires a named transient boundary that is neither of those. That boundary MUST be
  on the machine that produced the material, MUST erase it on completion or failure, MUST NOT
  place it on the agent's critical path, and MUST NOT be reachable by any other subsystem.
- **FR-764**: Where a deterministic check cannot be evaluated because required input is
  missing, the boundary MUST refuse rather than proceed.

### The safe-event ingest boundary

- **FR-765**: Cairn MUST expose a safe-event ingestion contract that is separate from the
  existing entity synchronization boundary.
- **FR-766**: The existing synchronization boundary's refusal of observation, evidence,
  verification and related content MUST remain in force, unweakened, for that boundary.
- **FR-767**: The safe-event contract MUST accept only a strongly typed event schema. It MUST
  reject an event carrying fields the schema does not define.
- **FR-768**: Ingestion MUST require authentication, and MUST authorize the caller against the
  project or account the events concern.
- **FR-769**: Identity used for authorization MUST be derived from the authenticated caller
  and MUST NOT be read from the request body.
- **FR-769a**: An event names its session, and a session identifier is body data. The server
  MUST verify that the session an event names exists, belongs to the project the event is
  attributed to, and belongs to the authenticated account — and MUST refuse the event otherwise.
  Without this, a project member could submit well-formed events naming a colleague's session,
  and consolidation would then produce durable knowledge whose provenance and reuse metrics
  attribute a colleague's authorship to work they never did. This is the same class of defect as
  falsified proposal attribution, moved to a different key.
- **FR-770**: Ingestion MUST be idempotent. Submitting the same event any number of times MUST
  produce at most one canonical event and at most one consolidation input.
- **FR-771**: The result of ingesting a batch MUST report per-event outcomes, distinguishing
  accepted, duplicate and rejected, so a client can retry precisely what needs retrying.
- **FR-772**: An event the server rejects on validation grounds MUST NOT be retried
  indefinitely; the client MUST be able to distinguish a permanent refusal from a transient
  failure.
- **FR-773**: Payloads MUST be bounded: the contract MUST state a maximum batch size and a
  maximum body size, and MUST enforce them.
- **FR-774**: The contract MUST be versioned, and a client and server disagreeing about the
  version MUST fail observably rather than silently discarding meaning.
- **FR-775**: A server that cannot yet store a newer event kind MUST refuse it in a way the
  client can recognise and defer, as the existing capability mechanism already does for
  entities.
- **FR-776**: The server MUST NOT persist arbitrary raw vendor JSON received on this
  boundary, regardless of what a client sends.
- **FR-777**: The server MUST enforce the content restrictions of the privacy boundary
  independently of the client, so that a malfunctioning or hostile client cannot store
  forbidden content by asking.
- **FR-777a**: The safe-event schema MUST NOT declare a field name that the synchronization
  boundary refuses. Two boundaries on one server disagreeing about the same name is precisely
  the drift FR-760 forbids for rejection classes. Where a safe event must carry information that
  a refused name would have described, it MUST use a distinct, explicitly defined field whose
  contents are constrained by the schema — and that definition MUST be recorded as a deliberate
  decision, not introduced by whichever implementation needs it first.
- **FR-778**: Ingestion failures MUST be observable: their rate, their reasons and their
  backlog MUST be reportable.
- **FR-779**: Events MUST be attributable to the project and account under whose credentials
  they were accepted.
- **FR-780**: Accepting an event MUST NOT depend on the local clock of the machine that
  produced it being accurate or monotonic.

### Edge spool and behaviour during outage

- **FR-781**: Agent-facing operations MUST fail soft. Server unavailability MUST NOT block,
  slow or break the coding agent beyond each operation's stated deadline.
- **FR-782**: While the server is unreachable, capture and privacy processing MUST continue.
- **FR-783**: Safe events produced while the server is unreachable MUST be spooled locally and
  retried later.
- **FR-784**: Retry MUST be bounded and MUST back off, and a permanently undeliverable event
  MUST become visible rather than being retried forever.
- **FR-785**: The spool MUST be bounded by a stated policy. When the bound is reached, what is
  dropped MUST be recorded and reportable.
- **FR-786**: Replay after an outage MUST NOT create duplicate canonical events, duplicate
  consolidation inputs, or duplicate durable knowledge.
- **FR-787**: No canonical fork may be created during an outage: Cairn MUST NOT produce
  durable knowledge locally that will later compete with the server's for authority.
- **FR-788**: Server-dependent capabilities MAY degrade during an outage. Degradation MUST be
  reported rather than disguised.
- **FR-789**: A cached briefing served during an outage MUST be labelled as cached and
  possibly stale.
- **FR-790**: Events spooled under one account's credentials MUST NOT be delivered under
  another's.
- **FR-790a**: A cached briefing MUST be bound to the account it was assembled for, and MUST NOT
  be served to a different account. A single store may legitimately hold several identities'
  knowledge, and a credential change or sign-out MUST invalidate or partition the cache. FR-790
  forbids delivering one account's queued writes under another's credentials; this is the same
  rule in the read direction, and without it an outage at session open is enough to hand one
  user's personal knowledge to the next user of that machine.
- **FR-791**: A server answering with a different server instance identity than the one a
  store is bound to MUST be refused, preserving the existing protection.
- **FR-792**: The user MUST be able to see the spool's depth, its oldest entry, and the reason
  delivery is not progressing.

### Automatic consolidation

- **FR-793**: Cairn MUST produce durable knowledge from accepted safe events without
  requiring an agent or human to invoke a memory-creation tool.
- **FR-794**: Consolidation MUST produce *candidates*. A candidate is not knowledge until it
  has passed Cairn's governance rules.
- **FR-795**: Candidates MUST be expressed using the existing durable vocabulary — fact,
  decision, convention, failure, procedure. Feature 005 introduces no sixth kind.
- **FR-796**: Each candidate MUST carry a deterministic identity derived from its subject and
  claim — its topic key and value key — and not from the wording of its content.
- **FR-797**: Consolidation MUST be idempotent with respect to its input: running it again
  over the same accepted events MUST NOT create additional knowledge, additional relations, or
  additional reinforcement.
- **FR-798**: A candidate whose deterministic identity matches existing knowledge MUST
  reinforce or be recognised as duplicate rather than creating a near-duplicate record.
- **FR-798a**: Reinforcement is a relation between two durable records, so a candidate that
  reinforces MUST have a persisted endpoint to reinforce *from*. That endpoint MUST exist, MUST
  be identifiable as consolidation-authored corroboration rather than a second copy of the
  claim, MUST NOT be returned by recall as independent knowledge, and MUST NOT be counted as a
  distinct claim anywhere a user reads a count of what Cairn knows.
- **FR-799**: A candidate that asserts a different value for a subject already held under the
  same topic and an overlapping scope MUST record a conflict, using the existing conflict
  machinery.
- **FR-800**: Consolidation MUST NOT automatically supersede existing knowledge. Supersession
  remains an explicit act.
- **FR-801**: Where consolidation records a reinforcement, it MUST do so only on a
  deterministic identity match — the same basis that already licenses automatic duplicate and
  conflict detection — and MUST record the basis on which it acted. Reinforcement on a model's
  judgement alone is forbidden.
- **FR-801a**: FR-801 deliberately narrows an existing rule. Today reinforcement may never be
  recorded automatically at all: only duplicate and conflict relations may be. Feature 005
  permits automatic reinforcement **solely** on a deterministic identity match, and on no other
  basis. Supersession remains fully manual (FR-800). This change MUST be stated in the relation
  contract rather than left implicit, and a relation recorded on this basis MUST be
  distinguishable from one a human or agent requested.
- **FR-802**: Every relation consolidation records MUST carry a basis drawn from a fixed
  vocabulary, and a relation produced by consolidation MUST be distinguishable from one a
  human or agent asked for.
- **FR-803**: Every durable record produced by consolidation MUST carry provenance back to the
  session and the safe events it was derived from.
- **FR-804**: A candidate MUST pass domain and scope resolution, the deterministic privacy
  checks, duplicate handling, reconciliation and persistence constraints before becoming
  durable. Failing any of these MUST prevent persistence.
- **FR-805**: A model MAY propose candidates. It MUST NOT decide whether a candidate becomes
  durable knowledge, what domain it belongs to, or whether it supersedes anything.
- **FR-806**: Consolidation MUST be able to run without embeddings, using structured
  extraction, topic and value keys, exact matching, and full-text search. Server-side full-text
  search presently covers project memory only; personal and team knowledge have no server-side
  text index. Extending indexing to the domains consolidation must match against is therefore
  in scope, and MUST NOT be assumed to already exist.
- **FR-807**: Consolidation MUST record its runs: what input range was processed, how many
  candidates were produced, how many were accepted, and how many were refused and why.
- **FR-808**: A consolidation run that fails MUST leave its input eligible for reprocessing
  and MUST NOT partially persist a candidate.
- **FR-809**: Consolidation MUST NOT ratify team guidance. It may produce a team proposal;
  only a human administrator may make team guidance authoritative.
- **FR-809a**: A team proposal produced by consolidation MUST carry a proposer attribution that
  is true. Where consolidation runs without an authenticated caller, it MUST NOT attribute the
  proposal to the account whose events it consolidated; it MUST be attributable to the automatic
  process itself, distinguishably from any human proposer, and MUST NOT appear in any account's
  "my pending proposals" view as that account's work. FR-810 forbids cross-user authorship in
  the personal domain; this is its team counterpart.
- **FR-810**: Consolidation MUST NOT create personal knowledge for a user other than the one
  whose events it processed.
- **FR-811**: Verification MUST continue to be derived from verification runs and attached
  evidence. Consolidation MUST NOT assert that knowledge is verified.
- **FR-812**: A candidate derived from activity Cairn could not attribute to a project MUST
  NOT be attributed to one by guesswork.
- **FR-813**: Consolidation MUST be observable while running: its backlog, its throughput and
  its failures MUST be reportable.
- **FR-814**: Consolidation MUST NOT be a precondition for capture. Events MUST continue to be
  accepted while consolidation is unavailable or backlogged.
- **FR-815**: The explicit memory-creation surface MUST remain. `cairn_remember` MUST continue
  to create durable knowledge immediately, without waiting for consolidation heuristics. Here
  "immediately" means without waiting for consolidation — not without waiting for the server.
- **FR-815a**: An explicit memory creation made while the server is unreachable MUST become a
  queued write, not a local durable record competing for authority (FR-709). It MUST be
  acknowledged to the caller as accepted-for-delivery rather than as durable, MUST be visible as
  pending, MUST become durable on acceptance, and MUST surface to the user if it is ultimately
  refused. This is what reconciles FR-815 with FR-787: an explicit creation is never lost and
  never blocks the agent, and it also never creates a second truth.
- **FR-816**: Knowledge created explicitly MUST be distinguishable from knowledge produced by
  consolidation, so a reader can tell what a human asserted from what Cairn inferred.

### Domain preservation and project-truth precedence

- **FR-817**: The project, personal and team domains MUST be preserved with their existing
  semantics.
- **FR-818**: The project domain's scopes — project, branch, task, session — MUST be preserved
  with their existing precedence.
- **FR-819**: Every durable record MUST continue to carry an explicit domain. No record may be
  domain-less.
- **FR-820**: Domains MUST remain visibly separate in every result. Cairn MUST NOT merge
  domains into a single ranked list.
- **FR-821**: Project knowledge MUST NOT be displaceable by personal or team guidance. Broader
  guidance may occupy only budget that project knowledge has left unused.
- **FR-822**: Personal and team knowledge MUST remain project-independent and MUST NOT name a
  project.
- **FR-823**: The existing relation kinds — reinforces, duplicates, supersedes, conflicts
  with, narrows, not applicable to — MUST be preserved.
- **FR-824**: Topic keys and value keys MUST be preserved as the identity of a claim about a
  subject.
- **FR-825**: The team lifecycle — proposal, ratification, retirement — and the rule that only
  a human administrator may ratify or retire MUST be preserved.
- **FR-826**: Cross-project guidance MUST NOT become the basis of a verification claim.

### Automatic retrieval and bounded context

- **FR-827**: Relevant knowledge MUST reach an agent's session without the agent or the user
  invoking a Cairn tool.
- **FR-828**: Retrieval MUST be driven by the session's own context — the project, the branch,
  the bound task where one exists, and the activity in the session.
- **FR-829**: Automatic context MUST be bounded. The existing budget discipline, including the
  reserve for project knowledge and the ceiling on personal and team guidance, MUST continue
  to hold.
- **FR-830**: Automatic context MUST NOT be inflated to compensate for the absence of explicit
  search. Depth remains reached by explicit search.
- **FR-831**: The explicit controls — search, context and remember — MUST remain available as
  overrides, not as the only path to usefulness.
- **FR-832**: Selection MUST be explainable: for any delivered briefing, Cairn MUST be able to
  state why each included item was included.
- **FR-833**: Retrieval MUST respect domain separation and scope precedence.
- **FR-834**: Retrieval MUST NOT return knowledge the caller is not authorized to see.
- **FR-835**: Identical inputs MUST produce an identical briefing, preserving the existing
  determinism guarantee. The degradation level reached under FR-836 counts as one of those
  inputs: identical inputs at the same declared level MUST produce an identical briefing.
  Wall-clock latency is never itself an input to briefing content.
- **FR-836**: Retrieval MUST have a stated deadline, and exceeding it MUST degrade the briefing
  rather than delay the agent. Degradation MUST be to one of a small number of pre-declared
  briefing levels, and the level reached MUST be recorded on the briefing and in its retrieval
  trace. Latency MUST NOT produce an unbounded family of briefings.
- **FR-837**: Where retrieval is served from cache, the briefing MUST say so.
- **FR-838**: Automatic retrieval MUST deliver at session open, which is the only automatic
  delivery point that exists today. Post-compaction restoration currently reaches an agent only
  where that agent reopens a session after compacting, or where a tool call asks for it — the
  post-compaction event itself carries nothing back, for any vendor. Feature 005 MUST either
  preserve that arrangement or state the new delivery point it adds; it MUST NOT describe
  post-compaction delivery as an existing behaviour being continued.

### Retrieval traces and delivery telemetry

- **FR-839**: Cairn MUST persist a record of each retrieval, sufficient to answer what triggered
  it, what knowledge was considered, what was selected, why, and what context was produced. A
  trace MUST record the *identities* of the records selected together with the budget accounting
  and the degradation level — never the rendered briefing text. Rendered text mixes domains and
  carries handoff-derived material, so persisting it centrally would place one account's
  personal knowledge inside a project-scoped record.
- **FR-840**: A retrieval record MUST be linked to the session and agent it served.
- **FR-841**: A retrieval record MUST record how long retrieval took.
- **FR-842**: Cairn MUST record whether transmission of context to the agent was attempted,
  and its outcome.
- **FR-843**: Cairn MUST distinguish these delivery stages: retrieval requested, context
  generated, context transmitted, context acknowledged.
- **FR-844**: Where a vendor provides no means of acknowledging receipt, `context
  acknowledged` MUST be reported as unavailable. Cairn MUST NOT report receipt it cannot
  establish.
- **FR-845**: Cairn MUST record which durable records were delivered into which session, so
  that a record's reuse is measurable.
- **FR-846**: Retrieval records MUST be subject to the same privacy rules as any other
  centrally persisted data, and MUST NOT carry raw context text that the privacy boundary
  would refuse.
- **FR-846a**: A retrieval trace MUST have a stated readership, and it MUST NOT widen access to
  anything it references. A briefing spans project, personal and team domains, so a trace of it
  names records from all three. A trace MUST NOT allow a reader to enumerate another account's
  personal knowledge, directly or by inference from identifiers, regardless of shared project
  membership. Where a trace references a record the reader may not see, the reference MUST be
  withheld rather than rendered as an opaque handle that still discloses its existence.
- **FR-847**: Retrieval records MUST be bounded in volume by a stated retention policy.
- **FR-848**: A failed retrieval MUST be recorded with its failure reason, not omitted.
- **FR-849**: Delivery telemetry MUST distinguish a briefing that was empty because there was
  nothing to say from one that was empty because something failed.
- **FR-850**: Telemetry MUST NOT claim a stronger delivery guarantee than the vendor can
  support.

### Integration health

- **FR-851**: Health MUST distinguish these stages: configured, installed, runtime hook fired,
  event received, event parsed, safe event accepted, server persisted event, context
  generated, context transmitted, and context receipt confirmed where the vendor supports it.
- **FR-852**: Evidence obtained by reading configuration back MUST be distinguishable, in the
  status a user reads, from evidence obtained by observing runtime behaviour. The two MUST NOT
  resolve to the same reported confidence.
- **FR-853**: Evidence obtained by reading configuration back MUST NOT raise reported
  confidence to the same level as evidence obtained by observing runtime behaviour. Today both
  kinds of evidence are recorded with their kind intact and then collapse to a single verified
  confidence; the recorded distinction MUST survive into the confidence a user reads. The
  conjunction of vendor availability and observed confidence already holds and is preserved.
- **FR-854**: Writing context to a channel MUST NOT be reported as the agent having consumed
  it.
- **FR-855**: A capture support matrix MUST distinguish: the vendor does not expose the
  capability, Cairn declines the capability, the adapter is not implemented, the capability
  failed at runtime, and the capability succeeded.
- **FR-856**: Where no evidence is available, Cairn MUST report that no evidence is available
  rather than reporting success or failure.
- **FR-857**: Health MUST be per agent and per capability, and MUST be attributable to the
  machine it was observed on.
- **FR-858**: A capture failure MUST be visible with the stage at which it failed.
- **FR-859**: Health status MUST be derived from recorded evidence, and MUST NOT be asserted
  independently of it.
- **FR-860**: Stale evidence MUST be identifiable as stale, so an integration that worked last
  month is not reported as working now on that basis alone.

### Migration from Feature 004

- **FR-861**: Feature 005 MUST provide a migration for installations running the Feature 004
  local schema.
- **FR-862**: Migration MUST NOT be a configuration flag. Authority MUST NOT switch until
  canonical possession has been verified.
- **FR-863**: Migration MUST inspect existing local knowledge and report what it found before
  changing anything.
- **FR-864**: Migration MUST drain eligible pending outbox rows before authority switches.
- **FR-864a**: Draining MUST go through the same per-author claim the outbox already enforces.
  A row whose recorded author does not match the authenticated account MUST NOT be delivered,
  and a row with no recorded author MUST NOT be treated as deliverable under whichever account
  is signed in. A bulk sweep that bypasses the author filter reopens a misattribution defect
  that has already been introduced once and fixed twice in this repository.
- **FR-865**: Migration MUST verify that the server holds canonical copies of the records
  whose authority is being transferred.
- **FR-866**: Migration MUST NOT lose project memory, personal knowledge or team knowledge.
- **FR-867**: Migration MUST NOT reassign authorship, domain or scope of any existing record.
- **FR-868**: Migration MUST NOT create duplicate canonical knowledge as a result of retry.
- **FR-869**: Migration MUST be resumable. An interruption at any point MUST be recoverable by
  running it again.
- **FR-870**: Where migration cannot safely complete, it MUST stop in a clearly reported
  failure state rather than proceeding.
- **FR-871**: Records the server cannot accept — local-only knowledge, or records the server
  refuses — MUST be retained locally, reported, and excluded from the authority switch rather
  than demoted or discarded.
- **FR-872**: Obsolete local replicas MUST be demoted only after canonical possession is
  confirmed for the records concerned.
- **FR-873**: Blocked outbox rows MUST be reported individually with the reason they are
  blocked.
- **FR-874**: Writer identity and writer sequence MUST be retained as provenance and
  delivery-gap detection. They MUST NOT be repurposed to resolve authority between replicas.
- **FR-875**: Server instance binding MUST be preserved across migration.
- **FR-876**: Synchronization machinery that exists only because authority was duplicated —
  offline multi-writer convergence over knowledge the server now owns — MUST NOT be removed
  while any store bound to the same server instance is still unmigrated. Migration is per store,
  but authority is per server: a user whose second device has not migrated still authors
  locally, and a first device that has discarded convergence has no way to resolve the
  divergence. Either every bound store has migrated, or the server MUST refuse the unmigrated
  store; the specification requires one of those to be chosen and enforced rather than left to
  timing.
- **FR-877**: An installation that has not migrated MUST continue to function under the
  Feature 004 semantics until it does.
- **FR-878**: Migration MUST be demonstrable on a populated store, and its guarantees MUST be
  asserted by tests rather than described.

### Web control plane

- **FR-879**: The web interface MUST present a dashboard showing the memory funnel: active
  agents, sessions, safe events received, capture failures, consolidation runs, candidates
  produced, knowledge accepted, candidates rejected or duplicate, reinforcements, conflicts,
  retrievals and delivery failures.
- **FR-880**: The dashboard MUST distinguish counts that are zero because nothing happened
  from counts that are unavailable.
- **FR-881**: The web interface MUST present recent activity at a semantic level, showing what
  Cairn is receiving and learning.
- **FR-882**: The activity view MUST default to a declared subset of event kinds rather than to
  every event, and MUST let a user widen it to the full stream deliberately. The default subset
  MUST be stated in the contract; "low-value" MUST NOT be left to the implementation to define.
- **FR-883**: The web interface MUST present a memory explorer exposing content, kind, domain,
  scope, state, importance, verification state, provenance, evidence summary, relations,
  reinforcement and retrieval usage, subject to privacy rules.
- **FR-884**: The memory detail view MUST allow a user to determine what the knowledge says,
  where it came from, what evidence supports it, whether it is verified, what it supersedes,
  what conflicts with it, what reinforces it, and where and when it has been retrieved.
- **FR-885**: The memory detail view MUST show whether a record was created explicitly or
  produced by consolidation.
- **FR-886**: The web interface MUST present retrieval traces showing the trigger, the
  candidates, the selection, the selection's explanation, the budget accounting and the delivery
  state. It MUST NOT render the briefing text itself, for the reason given in FR-839.
- **FR-887**: The web interface MUST present integration health per agent, using the
  distinctions FR-851 through FR-856 require.
- **FR-888**: The web interface MUST expose the project, personal and team domains, keeping
  them visibly separate.
- **FR-889**: The web interface MUST provide the team curation lifecycle — reviewing
  proposals, ratifying and retiring — restricted to administrators, closing the gap Feature
  004 deferred.
- **FR-889a**: The team curation surface MUST preserve the concurrency and one-way properties of
  the existing lifecycle, not merely its authorization. Ratification MUST be a conditional
  transition that succeeds only from the proposed state, retirement only from the authoritative
  state, each applied as a single atomic statement. A read-then-write handler that checks state
  and then updates it MUST NOT be introduced: it reopens a double-ratification race that is
  already closed, and it would make "un-retire" expressible.
- **FR-890**: The web interface MUST provide web equivalents for the administration surfaces
  Feature 004 shipped as CLI and server endpoints only. The specification MUST enumerate which
  surfaces those are rather than leaving appropriateness to judgement; a surface deliberately
  left CLI-only MUST be listed as such with its reason.
- **FR-891**: The web interface MUST present system health: ingest, consolidation and
  retrieval failures and backlogs.
- **FR-892**: Every web view MUST enforce the same authorization the API enforces, and MUST
  NOT rely on the client to hide what a user may not see.
- **FR-893**: Where a view would show material that is local-only, it MUST state that the
  material is local rather than rendering an empty section.
- **FR-894**: The server MUST expose the read APIs these views require; a view MUST NOT be
  specified that no endpoint can serve.
- **FR-894a**: Every new project-scoped read API — events, consolidation runs, funnel counts,
  health and retrieval traces — MUST enforce project membership itself, and a non-member MUST
  receive a refusal rather than an empty result. An empty result tells a non-member that the
  project exists and that they are outside it, and it makes the absence of a guard indetectable.
  FR-892's requirement that the web match the API is vacuous if the API is unguarded.
- **FR-895**: List views MUST be bounded and paginated.

### Bounded relation graph

- **FR-901**: Cairn MAY expose a bounded relation graph over its existing relation kinds. The
  graph is optional; FR-902 through FR-905 are binding on it if it is built, and are inert if
  it is not. No other requirement in this feature depends on the graph existing.
- **FR-902**: The relation graph MUST be presented as a derived investigative view, never as
  the source of truth.
- **FR-903**: The relation graph MUST state, as part of its contract, the maximum number of
  nodes and the maximum traversal depth it renders, and MUST enforce them. An unstated bound is
  not a bound.
- **FR-904**: The relation graph MUST NOT be built on a graph database, and MUST NOT introduce
  a new datastore.
- **FR-905**: The relation graph MUST NOT include entities Cairn does not already model as
  related knowledge.

### Key Entities

- **SafeCanonicalEvent**: A typed, privacy-approved statement that something semantically
  meaningful happened in an agent session. Carries its kind, its session, its structured
  content, its vendor provenance, its contract version and its stable identity. It is what
  crosses the machine boundary; the vendor payload it was derived from is not.
- **EventDisposition**: The lifecycle state of one event — captured, declined, spooled,
  transmitted, accepted, rejected, persisted — and, where it ended badly, the fixed reason.
- **CaptureCapability**: What a given agent can be captured for, per event kind, drawn from a
  closed vocabulary that distinguishes vendor absence from Cairn's decision, from an
  unimplemented adapter, from a runtime failure.
- **PrivacyDecision**: The outcome of the deterministic boundary for one piece of material:
  the check that decided, and the class of refusal where it refused. It is structurally
  incapable of carrying the material it refused.
- **ConsolidationRun**: One pass over a range of accepted events: what it read, what it
  proposed, what was accepted, what was refused and why. The unit by which autonomous learning
  is audited.
- **KnowledgeCandidate**: A proposed durable record before governance. Carries its deterministic
  identity — subject and claim — its proposed kind and domain, and its provenance. A candidate
  is never truth merely because a model emitted it.
- **CandidateDecision**: What Cairn did with a candidate — accepted, reinforced an existing
  record, recognised a duplicate, recorded a conflict, or refused — and on what basis.
- **RetrievalTrace**: One retrieval: its trigger, its session, what was considered, what was
  selected, the explanation for the selection, what context was produced, and how long it took.
- **DeliveryOutcome**: What became of a generated briefing — generated, transmitted,
  acknowledged, or acknowledgement unavailable — recorded at the strength the vendor actually
  supports.
- **EdgeSpool**: The local queue of safe events awaiting acceptance, with its depth, its oldest
  entry, its retry state and its bound. It holds unsent work, never an alternative truth.
- **AuthorityMode**: Whether a given local store has migrated to server authority, is
  mid-migration, or still operates under Feature 004 semantics.
- **DurabilityClass**: For a category of local data, whether its loss is recoverable from the
  server, recoverable by re-deriving it, or permanent. The vocabulary behind the claim that
  deleting the local store is safe.

---

## End-to-End Acceptance Scenario

Feature 005 is demonstrated on a real repository, in one continuous run. Principle I
requires the feature to end with something a developer can install and use; Principle VII
requires that demonstration to be defined here rather than improvised later.

**Starting state.** A fresh project with zero durable memories. A supported agent connected
and configured. The server reachable. No cached briefing.

**Phase 1 — Agent A does real work.** Agent A investigates a defect: it reads the relevant
code, forms a technical conclusion, changes the implementation, and runs the test suite until
it passes. Neither the user nor the agent invokes `cairn_remember`, `cairn_search` or
`cairn_context` at any point. Invoking one to make the demonstration pass invalidates it.

*Expected*: the session is captured; semantically useful events are accepted and persisted
canonically; consolidation runs; candidates are produced; durable knowledge results;
provenance resolves back to the session and its events; relations, evidence and verification
are recorded where applicable.

**Phase 2 — Agent B benefits.** A second session, in a related area, begins. It does not
search Cairn first.

*Expected*: relevant prior knowledge is selected; a bounded briefing is generated; it reaches
the integration; the delivery state is recorded at the strength the vendor actually supports.

**Phase 3 — The web tells the story.** Using only the web interface, a reviewer traces Agent
A's session, the events it produced, the consolidation run, the candidate decisions, the
accepted knowledge with its provenance and relations, Agent B's retrieval, and the context
delivery status.

**Phase 4 — The server goes away.** The connection to Cairn Server is interrupted while a
session is active.

*Expected*: the agent remains fully usable; safe events accumulate in the local spool;
restoring the server triggers retry; replay is idempotent; the server remains canonical; no
competing local truth was created.

**Phase 5 — The machine is lost.** Once canonical data is confirmed server-side, the local
store is destroyed and recreated.

*Expected*: durable project, personal and team knowledge survive and are reachable.
Machine-local integration state, caches, diagnostics and any events still spooled at the
moment of loss do not survive, and Cairn names that distinction rather than reporting an
unqualified success.

---

## Success Criteria *(mandatory)*

### Measurable Outcomes

A criterion stated as "in 100% of trials" is measured over a defined population, not a single
run: at least ten independent trials per supported agent that has the capability under test,
executed on a real repository. A criterion stating an absolute zero — zero records lost, zero
secrets persisted, zero duplicates — is measured over the whole corpus exercised by the
feature's tests, and a single counterexample fails it.

- **SC-701**: On a fresh project, a single coding session that invokes no Cairn tool produces at
  least one durable knowledge record whose claim is judged accurate by a reviewer against a
  pre-registered rubric, for every agent on the feature's declared supported-agent list.
  Accuracy is a reviewed judgement and is recorded as such; the automated portion asserts
  existence, provenance resolution and rubric completion, and fails if any is missing.
- **SC-702**: 100% of durable records produced by consolidation resolve to the session and the
  events they were derived from; zero have unresolvable provenance.
- **SC-703**: Re-running consolidation over an unchanged set of accepted events produces zero
  additional records, zero additional relations and zero reinforcement changes, in 100% of
  trials.
- **SC-704**: Zero candidates containing material the deterministic privacy checks refuse are
  persisted, across an adversarial corpus exercising every refusal class.
- **SC-705**: Zero refusal records contain any portion of the content they refused.
- **SC-706**: For every vendor signal on the feature's declared per-agent capture matrix, Cairn
  either captures it or reports it as unsupported, declined or unimplemented; zero are silently
  dropped. The matrix is declared before implementation and is the population under test, so the
  criterion cannot be satisfied by narrowing the list.
- **SC-707**: For a file-changing tool on every supported agent, the changed file's identity is
  captured or is explicitly recorded as unavailable from the vendor; zero are recorded as a
  generic command.
- **SC-708**: A second session in a related area receives relevant prior knowledge with no tool
  call, for every agent on the declared supported-agent list whose capture matrix records
  context delivery as supported. Membership of that list is fixed before testing; an agent whose
  delivery fails is a failure, not a reclassification.
- **SC-709**: 100% of delivered briefings are within their stated budget; zero exceed it.
- **SC-710**: In 100% of briefings where project knowledge alone would fill the reserve,
  personal and team guidance occupy none of it.
- **SC-711**: For 100% of delivered briefings, Cairn states, per included item, which selection
  rule admitted it and what budget remained at that point — sufficient for a reviewer to
  reproduce the same selection by hand from the recorded inputs. An explanation that does not
  permit that reproduction does not satisfy this criterion.
- **SC-712**: Delivery status reports acknowledgement as confirmed only for agents whose
  declared capture matrix records acknowledgement as supported; for every other agent on the
  declared list it reports acknowledgement unavailable. Zero agents report a confirmation their
  matrix entry does not license.
- **SC-713**: After deleting and recreating the local store, 100% of project, personal and team
  knowledge the server had accepted is reachable again, with no manual repair step.
- **SC-714**: After the same deletion, Cairn enumerates every category of data that did not
  survive; zero categories are lost silently.
- **SC-715**: With the server unreachable, agent-facing operations complete within their
  existing deadlines in 100% of trials, and zero agent operations are blocked by Cairn.
- **SC-716**: Replaying a spooled batch any number of times yields exactly one canonical event
  per distinct event, in 100% of trials.
- **SC-717**: During an outage, zero durable knowledge records are created locally that the
  server has not accepted.
- **SC-718**: 100% of briefings served from cache are labelled as cached; zero cached briefings
  are presented as current.
- **SC-719**: Migrating a populated Feature 004 store loses zero project memories, zero personal
  knowledge records and zero team knowledge records.
- **SC-720**: Migration reassigns the author, domain or scope of zero existing records.
- **SC-721**: Running migration repeatedly, including after an interruption at any step, creates
  zero duplicate canonical records.
- **SC-722**: Where migration cannot complete safely, it stops and reports the reason in 100% of
  such cases; zero proceed to switch authority.
- **SC-723**: Every record migration could not transfer is reported individually; zero are
  discarded or demoted without a report.
- **SC-724**: An integration that is configured but has never captured a runtime event is
  reported as not capturing, in 100% of cases.
- **SC-725**: Configuration-derived evidence and runtime-derived evidence resolve to visibly
  different reported confidence in 100% of cases.
- **SC-726**: For every agent and every canonical event kind, the support matrix names exactly
  one of: supported, unsupported by vendor, declined by Cairn, adapter unimplemented, failing at
  runtime. Zero cells are blank or ambiguous.
- **SC-727**: Every fact needed to reconstruct the path from an originating session to a
  delivered briefing is retrievable from the web interface's own APIs, asserted automatically by
  walking that path end to end. The human demonstration of the same path is the feature's
  Principle I acceptance run and is reported separately from this criterion.
- **SC-728**: The dashboard reports every funnel stage from events received to knowledge
  accepted; zero stages are unrepresented.
- **SC-729**: 100% of retrievals produce a persisted trace; zero retrievals, including failed
  ones, are unrecorded.
- **SC-730**: Zero centrally persisted records contain a full conversation transcript, raw tool
  output, a credential, an absolute local path, or arbitrary vendor JSON — asserted by a test
  that inspects what the server actually stored.
- **SC-731**: The existing synchronization boundary continues to refuse 100% of the entity types
  and field names it refuses today; zero refusals are weakened by this feature.
- **SC-732**: Ingest rejects 100% of events carrying fields outside the declared schema.
- **SC-733**: Ingest enforces its stated batch-size and body-size bounds in 100% of trials.
- **SC-734**: Zero durable knowledge records are created by consolidation in the team domain
  with authoritative status; 100% enter as proposals.
- **SC-735**: Zero supersessions are recorded automatically by consolidation.
- **SC-736**: Consolidation candidates that restate existing knowledge in different words
  reinforce rather than creating a new record, across a pre-registered paraphrase corpus of at
  least fifty claim pairs spanning all five knowledge kinds. The corpus is fixed before
  implementation.
- **SC-737**: The feature's dependency manifest gains zero datastore, broker, worker-platform
  or service dependencies, asserted by comparing the manifest against its pre-feature baseline
  and failing on any added entry in those categories.
- **SC-738**: Reusable patterns survive deletion of the local store in 100% of trials, closing
  the gap Feature 004 deferred.

---

## Clarifications

### Session 2026-08-29

Most ambiguities in the initial direction were resolvable from current `main`, the Feature
001–004 contracts, and the constitution, and were resolved as Assumptions below. Three
questions remain that a reasonable reader could answer in materially different ways with
materially different consequences, and they are left open rather than defaulted.

- Q: Where does consolidation execute? → **[NEEDS CLARIFICATION: the server today has no
  scheduler, background worker, timer or async job of any kind (`crates/cairn-server/src/`
  contains no `tokio::spawn` or interval task; it is a single request-serving process).
  Consolidation therefore needs an execution home that does not exist yet. The candidates are:
  (a) an in-process background task inside the existing server, (b) a scheduled pass triggered
  by ordinary request traffic, (c) a pass driven by each client's daemon against server APIs.
  Each has different failure, fairness and privacy consequences, and (c) partially reopens the
  question of where knowledge is decided.]**

- Q: What performs semantic extraction, and where does it run? → **[NEEDS CLARIFICATION: two
  problems, not one. (i) If extraction runs against a local model, no raw material leaves the
  machine. If it calls a hosted model, raw material leaves to a third party — which the privacy
  architecture otherwise forbids and no principle contemplates. (ii) Independently of the
  model's location, *no existing process is eligible to host extraction*: the per-event capture
  process is short-lived and runs under a millisecond-scale deadline, and FR-730 forbids the raw
  payload reaching the daemon. FR-763a states the properties the transient boundary must have;
  what that boundary actually is remains open. This is the highest-consequence open question in
  this specification.]**

- Q: Under what field name may a repository-relative path cross the safe-event boundary? →
  **[NEEDS CLARIFICATION: an earlier framing of this question — that repository-relative paths
  cannot cross the boundary at all today — is factually wrong and is corrected here. Handoffs
  are a synchronized entity and the server already stores `changed_files`, a list of
  repository-relative paths, which appears on neither refusal list
  (`crates/cairn-server/migrations/0001_init.sql:127`, `crates/cairn-server/src/sync.rs:772`).
  What the boundary refuses is the *field name* `path`, together with `path_fingerprints`, at
  any depth. So the real question is narrow and answerable: under what explicitly defined field
  name, with what constraints, may a safe event carry repository-relative file identity — given
  that FR-777a forbids the safe-event schema from reusing a name the other boundary refuses.
  This is a smaller decision than it first appeared, but it is still a decision.]**

---

## Assumptions

- The five durable knowledge kinds — fact, decision, convention, failure, procedure — are
  adequate for consolidation output. Repository research found no signal that a sixth is
  needed, and Principle II disfavours adding one speculatively.
- The existing reconciliation, relation, evidence and verification concepts are correct and
  are reused rather than replaced. Feature 005 changes who decides authority, not what
  knowledge means.
- The existing promotion gate's shape — a pure, fail-closed function whose rejection type
  cannot carry rejected text — is the right model for governing consolidation output, and is
  extended rather than duplicated.
- Structured matching on topic and value keys plus full-text search is sufficient for
  consolidation's duplicate and conflict detection. Embeddings are not required, and are not
  adopted on the possibility that they might help.
- PostgreSQL's own full-text search is the right mechanism for server-side retrieval, and no
  separate datastore is adopted. This is an intention, not a finding about current state: a
  server-side text index exists today for project memory only, and personal and team knowledge
  have none. Extending it to the domains consolidation and retrieval must match against is work
  this feature includes (FR-806). If a future requirement demonstrates full-text search is
  insufficient, PostgreSQL-native options are evaluated before any separate datastore.
- The existing per-namespace outbox, its claim protocol, its per-author claim filter and its
  backoff are a suitable foundation for the edge spool, and are repurposed rather than rebuilt.
- Repurposing the local personal and team replicas as a cache requires replacing their
  insert-once merge rule, which cannot apply a server-side content correction (FR-712a). This is
  a change to existing behaviour, not a reinterpretation of it.
- Writer identity and writer sequence are retained. Current code already uses them only for
  provenance and delivery-gap detection, never to resolve competing replicas, so the authority
  change does not invalidate them.
- Server instance binding is retained unchanged. It closes a same-endpoint-different-server
  attack and is unrelated to which side owns authority.
- The existing capture deadlines and their boundary/capture-class split are correct and are
  preserved. Richer capture must not lengthen the agent's critical path.
- Agents that Cairn does not have an adapter for continue to work through the explicit tool
  surface, and report their capture capability as absent.
- One server remains one team. Feature 005 introduces no organizations, tenants or nested
  groups.
- The web interface remains an authenticated client of the same API, enforcing the same
  authorization, with no privileged back channel.
- Existing installations upgrade in place. A user is not expected to export and re-import
  their knowledge.

---

## Out of Scope

- **A graph database, including Neo4j** — the bounded relation view is built over relations
  Cairn already stores; no requirement in this feature needs graph traversal a relational store
  cannot serve.
- **A separate vector database** — nothing in this feature's acceptance criteria requires one.
- **Mandatory embeddings** — consolidation is specified to work without them. If a later
  requirement demonstrates structured matching and full-text search are insufficient,
  PostgreSQL-native options are evaluated first.
- **A raw transcript archive** — full conversations are not persisted, by default or otherwise,
  and no requirement here creates one.
- **A permanent raw tool-output archive** — tool output is processed transiently and is not
  retained centrally.
- **Perfect full-feature offline Cairn** — this is the deliberate constitutional change.
  Capture, privacy processing and the agent's own work continue offline; freshly-derived
  server-side knowledge does not.
- **A semantic graph of every repository entity** — files, modules, technologies, people and
  causal relationships as first-class graph nodes are not modelled. No acceptance criterion here
  requires it.
- **Support for arbitrary future AI vendors** — adapters are written per vendor deliberately,
  which is the point of the vendor-aware architecture. A generic adapter that guesses at an
  unknown vendor's semantics would reintroduce exactly the flattening this feature removes.
- **A new message broker, distributed worker platform or microservice decomposition** — the
  existing Rust server and daemon are extended.
- **Organizations, multiple teams per server, or nested groups** — unchanged from Feature 004.
- **SSO, OAuth, multi-factor authentication or cross-server federation** — authentication is
  unchanged in kind.
- **A fifth memory scope** — scope is untouched; domain remains orthogonal to it.
- **Richer applicability requiring file *content* inspection** — Feature 004 deferred this to
  Feature 005, and it is deferred again. It is independent of the authority, capture and
  consolidation changes this feature is about, and folding it in would widen an already large
  feature for no acceptance criterion stated here. This is a re-deferral, recorded rather than
  silently dropped.
- **Decay models and confidence engines** — knowledge state remains derived from evidence and
  relations, not from a scoring model.
