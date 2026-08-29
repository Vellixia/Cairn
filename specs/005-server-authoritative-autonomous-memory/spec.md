# Feature Specification: Server-Authoritative Autonomous Memory

**Feature Directory**: `specs/005-server-authoritative-autonomous-memory`

**Git Branch**: `feature-005-spec`

The two differ, which is expected: this repository's `create-new-feature.sh` creates no git
branch, so the directory name and the working branch are independent. Both are recorded rather
than one being assumed from the other.

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
queued events produced exactly one canonical event each, and that no knowledge was created
locally that competes
with the server's.

**Acceptance Scenarios**:

1. **Given** an unreachable server, **When** the agent performs tool calls and lifecycle
   transitions, **Then** the agent's own work completes normally and no Cairn operation
   blocks it beyond its stated deadline.
2. **Given** an unreachable server, **When** safe events are produced, **Then** they are
   spooled locally and no durable knowledge is created locally that the server has not
   accepted.
3. **Given** spooled events and a restored server, **When** delivery retries, **Then** however
   many times delivery is retried, at most one canonical event and one consolidation input
   exist. A retry answered `duplicate` is a success, not a failure.
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
- **Semantic extraction is unavailable or fails.** Safe events continue to be captured,
  transmitted and persisted; the backlog grows and is reported, and no event is dropped for
  want of an extractor. Ingestion is never throttled to protect consolidation.
- **The server restarts mid-consolidation.** Durable progress state means the interrupted work
  is reclaimed and completed, and no already-consolidated event is consolidated a second time.
- **A model proposes a key that does not survive validation.** The candidate is refused and the
  refusal is recorded. Cairn does not repair the key into a plausible one, because repairing it
  would change which existing knowledge the candidate collides with.
- **A model proposes a source event reference that does not exist, or belongs to another
  project.** The candidate is refused rather than persisted with an unverifiable citation.
- **A vendor supplies an absolute path to a file inside the repository.** This is the ordinary
  case, not an error. The adapter relativizes it against the repository root locally and a valid
  `repo_file` crosses; the root itself never leaves the machine.
- **A vendor supplies a path outside the repository.** The event carries an out-of-repository
  disposition and no `repo_file`. It is neither transmitted as an absolute path nor discarded.
- **A vendor supplies no file identity in any form.** The event records identity as unavailable
  from the vendor.
- **A malformed or hostile client sends an absolute, drive-prefixed or traversal-bearing
  `repo_file`.** The server refuses it by its own validation, independently of what the client
  should have done. Local relativization is how the field is produced correctly; server-side
  refusal is how the rule is enforced.
- **A Feature 004 client contacts a server that has cut over.** Its knowledge synchronization is
  refused with `upgrade_required`; its local data is untouched; its agent keeps working.
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
  holding machine-local integration and operational state, and holding a bounded
  non-authoritative cache. Raw material read during capture is held in memory for the duration
  of that work and is not a storage role (FR-763).
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
- **FR-727a**: A semantic signal MUST carry enough structure for the decision or instruction to
  be learnable. A signal that records only that something happened destroys the information at
  the machine boundary, where no later stage can recover it, and would leave Feature 005 unable
  to learn the decisions and constraints it exists to learn.
- **FR-727b**: That structure MUST NOT be free text derived from user or assistant messages. It
  MUST be a closed classification plus tokens drawn from a vocabulary the machine can justify
  from non-prose evidence already present in the session's own events — file and module tokens,
  command verbs, test identifiers, and keys already established in that project's knowledge.
- **FR-727c**: A token that cannot be justified against that vocabulary MUST be refused, by the
  producing machine and independently by the server. This is structural rather than procedural:
  a sentence's words and a credential are both absent from a derived vocabulary, so neither can
  be encoded in a token even deliberately.
- **FR-727d**: Semantic signal structure MUST be derivable without a model. A model MAY propose
  the classification or the tokens, but the vocabulary check governs the outcome either way, so
  a model is never the gate and is never required.
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

The local machine performs vendor parsing, semantic event normalization, redaction, the
deterministic secret, path and privacy checks, and bounded safe-event construction. It does
**not** perform semantic knowledge extraction. Extraction is server-side work over events the
privacy boundary has already approved, and is specified under Consolidation.

- **FR-749**: Rich vendor material MAY be read transiently on the machine that produced it,
  for the sole purpose of parsing it, normalizing it into canonical events, redacting it and
  evaluating the deterministic privacy checks. Only material the privacy boundary approves may
  be transmitted. The machine MUST NOT retain the raw material beyond that work, and MUST NOT
  perform semantic knowledge extraction over it.
- **FR-749a**: This local work is larger than today's — more vendor fields, more event kinds,
  more content to redact — while the existing capture deadlines are preserved. The specification
  therefore requires the richer local pipeline to fit within those deadlines. If measurement
  shows the work cannot fit, the deadline budget MUST be restated explicitly rather than allowed
  to erode by degrees, because the agent's critical path is what Principle III protects first.
- **FR-749b**: A capture-class event that exceeds its deadline MUST NOT become an agent-facing
  failure. The hook MUST still exit successfully, MUST NOT block the agent, and MUST NOT surface
  an error to the vendor. The event may be dropped.
- **FR-749c**: A deadline-driven drop is nonetheless a capture failure in Cairn's own account of
  itself, and MUST NOT be silent. Cairn MUST record a local disposition distinguishing it from
  other outcomes — a `capture_deadline_exceeded` disposition or equivalent — and MUST surface it
  in capture health and counters. Fail-soft describes what the agent experiences, not what Cairn
  is permitted to know: a drop the agent never notices is exactly the drop Principle X forbids
  Cairn from reporting as health.
- **FR-749d**: The record of a deadline drop MUST carry no rejected or raw content. It carries
  the disposition, the event kind where already determined, the agent and the session — nothing
  from the payload that was being processed when the deadline expired.
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
- **FR-758**: Semantic extraction and privacy enforcement MUST remain separate acts, wherever
  each runs. A model MUST NOT be the sole or final gate on whether material may be transmitted
  or persisted.
- **FR-759**: Output of semantic extraction MUST pass the same deterministic checks that govern
  any other candidate content, and MUST be refused on the same terms. Extraction running on the
  server does not exempt its output from validation; it is subject to the same single
  implementation of each rejection class that FR-760 requires.
- **FR-760**: Cairn MUST maintain exactly one implementation of each privacy rejection class,
  so that two entry points cannot drift apart.
- **FR-761**: Existing user controls MUST continue to hold: path and content exclusion,
  local-only knowledge, and deletion of any observation, memory or session.
- **FR-762**: Promotion of knowledge from one domain to a wider one MUST remain an explicit
  act passing the existing fail-closed gate. Consolidation MUST NOT promote across domains as
  a side effect.
- **FR-763**: Raw vendor material held transiently on the local machine MUST be discarded once
  parsing, redaction and the privacy checks complete or fail, and MUST NOT outlive that work.
  It MUST NOT be written to durable local storage, and MUST NOT be retained pending any later
  extraction step.
- **FR-763a**: Semantic knowledge extraction MUST run on the server, over `SafeCanonicalEvent`
  records that the privacy boundary has already approved and that the server has already
  persisted. No new local transient extraction process is introduced. This is what removes the
  conflict between rich extraction and the local capture path: the per-event capture process is
  short-lived and runs under a millisecond-scale deadline, and FR-730 forbids the raw payload
  reaching the daemon — so extraction is moved to the one place that legitimately holds
  material rich enough to extract from, namely the approved events themselves.
- **FR-763b**: Extraction MUST operate only on data the server was already permitted to hold.
  It MUST NOT be granted a side channel to raw vendor payloads, transcripts, raw tool output,
  secrets or absolute local paths, none of which ever reach the server. If a safe event does not
  carry enough information to extract a claim, no claim is extracted; the missing material MUST
  NOT be fetched from the machine that produced it.
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
  the drift FR-760 forbids for rejection classes.
- **FR-777a1**: This obligation is general, not satisfied by renaming one field. The refused
  set includes `summary`, `command`, `details`, `exit_code`, `outcome` and `content_norm_digest`
  — every one of which is the natural name for something a command, test or tool-failure event
  must carry. The safe-event schema MUST therefore define its own distinct, explicitly named
  and individually constrained field for each such value, and the mapping from canonical
  meaning to field name MUST be stated once in the contract rather than settled per
  implementation. `repo_file` is the first instance of this rule, not an exception to it.
- **FR-777b**: File identity on a safe event MUST be carried by a field named `repo_file`,
  holding repository-relative file identity only. The refused name `path` MUST NOT be reused,
  and `repo_file` MUST NOT be permitted to carry anything a repository-relative path could not
  express.
- **FR-777c**: A `repo_file` value MUST be relative, slash-normalized and non-empty. It MUST
  NOT contain a `..` traversal segment, MUST NOT begin with `/`, and MUST NOT carry a
  drive-letter or UNC prefix. Its maximum length MUST be stated as a number in the contract and
  enforced. A value failing any of these MUST cause the event to be refused.
- **FR-777d**: The server MUST validate `repo_file` independently of the client, applying the
  same rules. A client that constructs the field correctly is not the mechanism by which the
  rule holds; the server's own check is.
- **FR-777e**: Most vendors report an absolute path. Relativizing it against the repository
  root is local normalization work and MUST happen on the machine, before the event is
  constructed. The repository root is machine configuration and MUST NOT be transmitted; only
  the relative result crosses.
- **FR-777f**: A file that lies outside the repository — a path that cannot be expressed
  relative to the root without traversal — MUST be recorded as out-of-repository, carrying no
  `repo_file`. Cairn MUST NOT transmit it, MUST NOT truncate it into something that looks
  repository-relative, and MUST NOT discard the event merely because its file lies outside.
- **FR-777g**: Where a vendor genuinely supplies no file identity in any form, the event MUST
  record identity as unavailable from the vendor. Cairn MUST NOT synthesize a `repo_file`, MUST
  NOT substitute the working directory or a tool name for one, and MUST NOT downgrade the event
  to a generic command in order to avoid the absence. FR-720 treats a path the adapter could
  have extracted and did not as a capture defect; FR-777f and this requirement cover the two
  cases that are genuinely not defects.
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

- **FR-793**: Cairn MUST produce durable knowledge from accepted safe events without requiring
  an agent or human to invoke a memory-creation tool.
- **FR-793a**: Consolidation MUST run as in-process background work inside the existing server
  process. It MUST NOT be a new service, a broker, a distributed worker platform, or work driven
  by a client. Client-driven consolidation is specifically excluded: it would return the
  decision about what becomes durable knowledge to the edge, which is the arrangement this
  feature exists to end.
- **FR-793a1**: Because consolidation shares a process and a connection pool with request
  serving, its resource share MUST be bounded and stated — its concurrency, its batch size and
  its share of the connection pool. FR-814's prohibition on back-pressure is not achievable by
  intention alone; it is achievable because consolidation cannot consume the capacity ingestion
  needs. Principle II independently requires deferred work to be bounded.
- **FR-793b**: Consolidation progress MUST be durable in PostgreSQL. The server MUST record
  which accepted events have been consolidated and which remain outstanding, such that a server
  restart at any point loses no completed consolidation work. Re-deriving over events whose
  claim was abandoned mid-pass is permitted and expected; what MUST NOT happen is a second
  durable effect from it, which FR-797 guarantees.
- **FR-793c**: The consolidation backlog MUST be a queryable, bounded-report resource: its
  depth, its oldest outstanding event and its failure count MUST be reportable.
- **FR-793d**: A consolidation pass MUST claim its work so that concurrent passes within the
  process cannot consolidate the same events twice, and a claim abandoned by a restart MUST
  become reclaimable rather than stranding the work.
- **FR-794**: Consolidation MUST produce *candidates*. A candidate is not knowledge until it
  has passed Cairn's governance rules.
- **FR-795**: Candidates MUST be expressed using the existing durable vocabulary — fact,
  decision, convention, failure, procedure. Feature 005 introduces no sixth kind.
- **FR-796**: Each candidate MUST carry a deterministic identity derived from its subject and
  claim — its normalized topic key and value key — and not from the wording of its content.
- **FR-796d**: Normalized key identity is the identity consolidation uses for duplicate,
  conflict and reinforcement decisions on the server. The existing exact content-digest rule is
  not available there: `content_norm_digest` is a refused field name at the synchronization
  boundary and SC-731 forbids weakening that refusal, so the server cannot hold the value the
  existing rule compares. The specification therefore requires key identity to be sufficient
  for server-side reconciliation on its own, and does NOT require the content digest to be
  reproduced centrally. Where the digest rule continues to operate on records that already have
  one, it remains as it is; FR-823 and FR-824 preserve the relation kinds and the key concepts,
  not the storage location of a local implementation detail.
- **FR-796a**: A model-proposed topic key or value key is an input, never the identity itself.
  Cairn MUST normalize and validate every proposed key through its own deterministic function
  before that key is used for duplicate, conflict or reinforcement decisions. Syntactic variants
  of one key — differing by case, by separator, or by surrounding whitespace, such as
  `Storage Authority`, `storage_authority` and `storage-authority` — MUST resolve to a single
  canonical representation.
- **FR-796b**: A proposed key that does not survive normalization and validation MUST cause the
  candidate to be refused, with the refusal recorded under the same fixed vocabulary as any
  other refusal. Cairn MUST NOT silently repair a malformed key into a plausible one, because a
  repaired key silently changes which existing knowledge the candidate collides with.
- **FR-796c**: Identity normalization MUST NOT require embeddings or any similarity model. It is
  a deterministic syntactic function, and two keys are the same key or they are different keys.
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
- **FR-798b**: A corroboration endpoint MUST carry a deterministic identity that is stable
  across re-execution, so that re-deriving it after an abandoned claim yields the same endpoint
  rather than a second one. Without this, a mid-pass restart would add a corroboration record
  and a relation on every retry, which FR-797 and SC-703 forbid.
- **FR-798c**: That identity MUST NOT be derived from the set of source events. An earlier
  formulation of FR-798b required exactly that, and it is withdrawn here because it is
  self-defeating: the event set is *not* stable across re-execution — a reclaim after a lease
  expires sweeps in events that arrived meanwhile, and an event that exhausts its attempts
  leaves the batch — so an identity including it changes on retry and produces the duplicate the
  requirement existed to prevent. Identity is derived from the project, the session and the
  normalized keys. Source events remain recorded, additively, as evidence for the endpoint;
  they are provenance, not identity.
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
- **FR-804a**: Candidate refusals MUST be recorded under a fixed, enumerated vocabulary of
  reasons, distinct from the event-rejection vocabulary FR-741 defines and covering at minimum:
  key normalization failure, privacy refusal, unverifiable source reference, domain or scope
  unresolvable, and conflict-with-existing. As with event rejections, a refusal record MUST NOT
  carry the content that caused it.
- **FR-805**: A model MAY propose candidates. It MUST NOT decide whether a candidate becomes
  durable knowledge, what domain it belongs to, or whether it supersedes anything.
- **FR-805a**: A model performing extraction MUST receive only `SafeCanonicalEvent` records
  that Cairn was already permitted to persist centrally. It MUST NOT receive raw vendor
  payloads, transcripts, raw tool output, secrets, absolute local paths, or any other material
  the privacy boundary refused.
- **FR-805a1**: Server-held is not the same as readable-by-anyone, and extraction MUST be scoped
  the way every other read path is. An extraction request MUST be confined to the events of one
  project and one account context, and MUST NOT be handed a cross-project or cross-account
  corpus. Nothing else in Cairn reads without a membership guard, and extraction MUST NOT be the
  exception.
- **FR-805b**: A model MAY propose candidate content, the knowledge kind, a topic key, a value
  key, and references to the source events it drew from. It MUST NOT determine durability,
  authorization, domain ownership, scope, privacy acceptance, reconciliation outcomes,
  verification, or supersession. Each remains a Cairn decision made by deterministic rules, and
  each MUST be reachable without consulting a model at all.
- **FR-805c**: Source event references a model proposes MUST be verified to exist, to belong to
  the same project and session context being consolidated, and to have been accepted by the
  server. A candidate citing an event that fails any of those checks MUST be refused rather than
  persisted with an unverifiable citation.
- **FR-805d**: Where extraction is performed by any party other than the operator of the Cairn
  server, that party is a second recipient at a second boundary. Its identity and its use MUST
  be disclosed to affected users as plainly as the Cairn server connection itself, at a
  comparable point and in comparable terms. Cairn MUST NOT forward a safe event to a
  third-party extractor that has not been disclosed. The fact that the material was permitted
  to reach the user's own server MUST NOT be treated as permission to forward it onward.
- **FR-805e**: A third-party extractor's retention and training behaviour MUST NOT be assumed.
  Cairn MUST NOT enable a hosted extractor unless the deployment has established, from the
  provider's current documentation, what that provider does with submitted content. Where
  compliance cannot be established, the hosted extractor MUST NOT be enabled and the condition
  MUST be reported, rather than being enabled on the assumption that a default configuration is
  acceptable.
- **FR-805f**: Extraction MUST be replaceable without changing anything else in the pipeline.
  No requirement in this feature may depend on a particular extractor, a particular provider, or
  a hosted extractor existing at all; an operator MUST be able to run Cairn with a
  self-hosted or deterministic extractor and lose no guarantee other than extraction quality.
- **FR-806**: Consolidation MUST be able to run without embeddings, using structured
  extraction, topic and value keys, exact matching, and full-text search. Server-side full-text
  search presently covers project memory only; personal and team knowledge have no server-side
  text index. Extending indexing to the domains consolidation must match against is therefore
  in scope, and MUST NOT be assumed to already exist.
- **FR-807**: Consolidation MUST record its runs: which events a pass claimed, how many
  candidates were produced, how many were accepted, and how many were refused and why. A pass
  is described by the set of events it claimed, not by a contiguous range, because an
  interruption leaves the consolidated set non-contiguous.
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
- **FR-810a**: Consolidation runs without an authenticated caller, so the owner it assigns MUST
  come from the account binding the server recorded when it accepted the events (FR-779), never
  from event body content and never from model output. This is how a caller-less process
  satisfies the rule that identity is established rather than asserted.
- **FR-811**: Verification MUST continue to be derived from verification runs and attached
  evidence. Consolidation MUST NOT assert that knowledge is verified.
- **FR-811a**: Raw evidence facts and verification runs remain local-only (FR-707). Server-held
  knowledge MAY carry a derived verification *summary* — the verification state, its authority,
  and when it was last established — because a state a user cannot see centrally is a state the
  web control plane cannot report.
- **FR-811b**: A verification summary MUST be derived from verification runs that actually
  occurred, and MUST NOT be settable by a client or a model asserting a state directly. The
  server MUST NOT accept `verified` as a claim; it MUST accept only an attested report of a run,
  bound to the account and project it came from, and derive the stored state itself under the
  same rules that derive it locally today.
- **FR-811c**: A verification summary MUST NOT carry raw evidence: no observed values, no source
  locators, no digests of file content, no command output, no local paths. It carries state,
  authority, a timestamp and counts.
- **FR-811d**: Where a summary cannot be established — because the run was local-only, or its
  report was refused — the server MUST hold the knowledge as unverified rather than inheriting a
  state it cannot justify. Unverified is a truthful answer; an unsubstantiated `verified` is the
  overclaim Principle X forbids.
- **FR-812**: A candidate derived from activity Cairn could not attribute to a project MUST
  NOT be attributed to one by guesswork.
- **FR-813**: Consolidation MUST be observable while running: its throughput and its failures
  MUST be reportable alongside the backlog FR-793c requires.
- **FR-814**: Consolidation MUST NOT be a precondition for capture. Events MUST continue to be
  accepted, and safe events MUST continue to be persisted, while consolidation is unavailable,
  failing or arbitrarily backlogged. A consolidation backlog MUST NOT apply back-pressure to
  ingestion, and MUST NOT be reported to a client as an ingestion failure.
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

### Supported agents and delivery points

This section fixes the population that SC-701, SC-706, SC-708 and SC-712 measure against, so
that it cannot be narrowed by an implementation. It is derived from each vendor's current
official documentation, checked on 2026-08-30, and from the adapters on `main`.

| Capability | Claude Code | Codex CLI | OpenCode |
|---|---|---|---|
| Capture events | documented, stable | documented, stable | documented (v1 bus events + plugin hooks) |
| Session-open delivery | documented, stable | documented, stable | no stable documented injection point |
| Prompt-time retrieval | documented, stable | documented, stable | exists in v2 beta; Cairn declines to depend on it |
| Post-compaction opportunity | via session-open `compact` trigger | via session-open `compact` trigger | pre-compaction only |
| Receipt acknowledgement | none | none | none |
| MCP | yes | yes | yes |

- **FR-838a**: Feature 005 commits to automatic capture for **Claude Code**, **Codex CLI** and
  **OpenCode**. It commits to automatic context delivery for **Claude Code** and **Codex CLI**
  only.
- **FR-838b**: OpenCode is a supported capture agent whose automatic delivery Cairn does NOT
  commit to in this feature. This is a Cairn decision, not a vendor absence, and MUST be
  reported as such. OpenCode 2 does expose prompt and context delivery hooks; they are beta,
  and the vendor states its plugin APIs may change. OpenCode v1's injection points exist in the
  published type definitions but are absent from the documentation and carry experimental
  names. Cairn declines to make an automatic-delivery guarantee that rests on an unstable
  vendor surface. The capability MUST therefore be reported as **declined by Cairn**, with the
  reason recorded, and MUST NOT be reported as unsupported by the vendor, which would be untrue.
  OpenCode remains required for automatic capture and for manual MCP use. If the surface
  stabilizes, delivery may be added without a specification change, because FR-828 states the
  requirement by capability rather than by vendor.
- **FR-838c**: Both committed agents expose a prompt-time hook that fires before the model
  processes a user prompt and accepts returned context. Prompt-time retrieval is therefore an
  available delivery point for both, and MUST be treated as a capability the capture matrix
  declares per agent, not as an assumption about all agents.
- **FR-838d**: Post-compaction delivery MUST NOT be implemented by returning context from a
  post-compaction event. At least one committed vendor documents that its post-compaction event
  cannot carry returned context at all. The supported mechanism for both committed agents is
  delivery at session open, distinguished by the trigger that opened the session: both document
  a session-start source value of `compact`. Post-compaction delivery is therefore an
  established capability for Claude Code and Codex CLI, not an open vendor question.
- **FR-838e**: No receipt-acknowledgement mechanism was established for any committed agent
  from the official documentation reviewed on 2026-08-30. That is an absence of evidence, not a
  vendor statement that none exists, and MUST be recorded as such: receipt status is
  `unavailable / no evidence`, never `vendor does not support it`. Cairn MUST NOT claim
  confirmed receipt for any agent unless a named vendor mechanism establishes it, at which point
  the matrix entry changes on that evidence.
- **FR-838f**: Agents reachable only through generic MCP remain supported for manual use and are
  NOT part of the automatic capture or delivery population. Their capability MUST be reported as
  absent rather than healthy.

### Automatic retrieval and bounded context

- **FR-827**: Relevant knowledge MUST reach an agent's session without the agent or the user
  invoking a Cairn tool, at one or more of the delivery points that agent's declared capability
  matrix records as available.
- **FR-828**: Retrieval MUST be driven by the session's own context — the project, the branch,
  the bound task where one exists, and the activity in the session. Where an agent exposes a
  prompt-time delivery point, retrieval MAY additionally be driven by the prompt about to be
  processed. Cairn MUST NOT require a prompt-time hook from an agent whose vendor does not
  provide one.
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
- **FR-838**: Automatic retrieval MUST deliver at session open for every agent whose matrix
  records session-open delivery, and MAY additionally deliver at a prompt-time point where the
  matrix records one. Post-compaction restoration reaches an agent through a session opened by
  compaction, not through the post-compaction event itself, which at least one committed vendor
  documents as unable to carry returned context. Feature 005 MUST NOT describe post-compaction
  delivery as an existing behaviour being continued.

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
- **FR-867a**: Migration MUST normalize the topic and value keys of existing records using the
  same deterministic function FR-796a applies to new candidates. Without this, a normalized
  candidate stops colliding with a legacy record carrying an un-normalized key, and duplicate,
  conflict and reinforcement detection silently degrade against exactly the knowledge users
  already have. Normalizing a key is not reassigning a record's domain, scope or authorship, and
  where two existing records normalize to one key the collision MUST be surfaced through the
  ordinary conflict machinery rather than resolved by discarding one.
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
- **FR-875**: Server instance binding MUST be preserved across migration and across cutover,
  including for a client refused under FR-876b: a refused client MUST still be able to establish
  that it is bound to the same server instance, so an upgrade prompt cannot be induced by
  pointing a client at a different server.
- **FR-876**: A server MUST have an explicit authority mode, and its transition to
  server-authoritative mode MUST be an observable cutover rather than an emergent state.
- **FR-876a**: Before cutover, a Feature 004 client MUST be able to migrate normally.
- **FR-876b**: After cutover, a pre-005 client MUST NOT perform knowledge synchronization that
  assumes local authority. The server MUST refuse such a write with an explicit, distinguishable
  result meaning `upgrade_required` — not a generic error and not a silent success.
- **FR-876b1**: The server cannot determine how an already-shipped client reacts to a result
  code that did not exist when that client was built; a pre-005 client will route an unknown
  refusal into its existing blocked-or-failed branch. What the specification requires is
  therefore what the server controls: the refusal MUST be permanent rather than
  capability-shaped, so that a client treating it as deferrable makes no progress and loses no
  data, and the condition MUST be visible to the operator. An upgraded client MUST recognise
  `upgrade_required` and stop retrying, surfacing the upgrade to the user.
- **FR-876c**: An `upgrade_required` refusal MUST leave the legacy client's local data intact.
  Cairn MUST NOT delete, demote, truncate or rewrite the local store of a client it has just
  refused. The client is out of date, not wrong.
- **FR-876d**: After that client upgrades, it MUST be able to run the Feature 005 migration,
  and its local replicas MUST be demoted only after canonical possession has been established
  for the records concerned, exactly as FR-865 and FR-872 require for any other store.
- **FR-876e**: Two different mechanisms must not be confused here. The *dual-authority
  convergence system* — offline multi-writer merge over knowledge the server now owns — may be
  retired at cutover, because a returning pre-005 client is refused rather than merged. The
  *migration path* — draining a store's remaining queued writes and establishing canonical
  possession — MUST survive cutover, because FR-876d requires clients to migrate after it. The
  migration path belongs to the upgraded client and the migration tooling, not to the legacy
  convergence system, and retiring the latter MUST NOT remove the former. This supersedes the
  earlier condition that convergence removal wait until every store bound to the server had
  migrated, which made retirement contingent on a device that might never return.
- **FR-877**: An installation that has not migrated MUST continue to function under the
  Feature 004 semantics for as long as the server it is bound to has not cut over. Once that
  server has cut over, the client's knowledge synchronization is refused per FR-876b while its
  local data remains readable to it; ordinary local agent operation MUST NOT break.
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
- **AuthorityMode**: Whether a server has cut over to server-authoritative mode, and whether a
  given local store has migrated, is mid-migration, or still operates under Feature 004
  semantics. The server's mode is what makes `upgrade_required` answerable.
- **ConsolidationBacklog**: The durable record of which accepted events remain to be
  consolidated, including claim state, so that progress survives a restart and outstanding work
  is reportable rather than inferred.
- **ExtractionInput**: The bounded set of already-approved safe events handed to extraction. It
  is the complete description of what any extractor, model or otherwise, is permitted to see.
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
  pre-registered rubric, for every agent FR-838a commits to automatic capture — Claude Code,
  Codex CLI and OpenCode. Accuracy is a reviewed judgement recorded as such; the automated
  portion asserts existence, provenance resolution and rubric completion, and fails if any is
  missing.
- **SC-701a**: Over a pre-registered scenario set of at least twenty sessions in which a
  decision or a standing instruction is expressed and then acted on, at least **fourteen**
  produce a durable `decision` or `convention` record whose subject and object tokens match the
  scenario's declared expectation. This criterion exists because SC-701 can be satisfied by a
  structural record alone — a test-failure `failure` memory — which would leave the feature's
  actual purpose untested. A run in which every produced record is structural **fails**
  SC-701a even if it passes SC-701.
- **SC-701b**: Across the same scenario set, zero durable records contain any word from the
  originating prompt or assistant turn that is not independently present in that session's
  derived vocabulary. This is the falsifiable form of the claim that reasoning does not cross
  the boundary.
- **SC-702**: 100% of durable records produced by consolidation resolve to the session and the
  events they were derived from; zero have unresolvable provenance.
- **SC-703**: Re-running consolidation over an unchanged set of accepted events produces zero
  additional records, zero additional relations and zero reinforcement changes, in 100% of
  trials.
- **SC-704**: Zero candidates containing material the deterministic privacy checks refuse are
  persisted, across an adversarial corpus exercising every refusal class.
- **SC-705**: Zero refusal records contain any portion of the content they refused.
- **SC-706**: For every vendor signal on the per-agent capture matrix declared under FR-838a,
  Cairn either captures it or reports it as unsupported, declined or unimplemented; zero are
  silently dropped. The matrix is fixed before implementation and is the population under test,
  so the criterion cannot be satisfied by narrowing the list.
- **SC-707**: For a file-changing tool on every supported agent, the changed file's identity is
  captured or is explicitly recorded as unavailable from the vendor; zero are recorded as a
  generic command.
- **SC-708**: A second session in a related area receives relevant prior knowledge with no tool
  call, for every agent FR-838a commits to automatic delivery — Claude Code and Codex CLI — at
  each delivery point that agent's matrix records as available. OpenCode is excluded because
  Cairn **declines** to depend on its beta delivery surface (FR-838b), reported as
  `declined_by_cairn` and never as a vendor limitation: OpenCode 2 does expose the hooks. An
  OpenCode *capture* failure still fails SC-701 and SC-706.
- **SC-709**: 100% of delivered briefings are within their stated budget; zero exceed it.
- **SC-710**: In 100% of briefings where project knowledge alone would fill the reserve,
  personal and team guidance occupy none of it.
- **SC-711**: For 100% of delivered briefings, Cairn states, per included item, which selection
  rule admitted it and what budget remained at that point — sufficient for a reviewer to
  reproduce the same selection by hand from the recorded inputs. An explanation that does not
  permit that reproduction does not satisfy this criterion.
- **SC-712**: Delivery status reports acknowledgement as confirmed for zero agents, because no
  acknowledgement mechanism has been established from reviewed vendor documentation (FR-838e);
  all report `unavailable / no evidence`. Zero report acknowledgement as unsupported-by-vendor,
  which the evidence does not license. Should a named vendor mechanism be established, the
  criterion becomes: confirmed only where the matrix records acknowledgement as supported.
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
- **SC-737**: The feature's dependency manifest gains zero datastore, broker or
  worker-platform dependencies, asserted by comparing the manifest against its pre-feature
  baseline and failing on any added entry in those categories. An extraction model provider, if
  one is configured, is declared separately as an extraction dependency and is governed by
  FR-805a and FR-805a1 rather than counted here — it stores nothing and coordinates nothing.
- **SC-738**: Reusable patterns survive deletion of the local store in 100% of trials, closing
  the gap Feature 004 deferred.
- **SC-739**: Restarting the server at each of at least twenty pre-registered points during
  consolidation, including mid-pass, yields the same durable knowledge, the same relations and
  the same reinforcement counts as an uninterrupted run over the same events, and leaves zero
  events permanently unconsolidated. Re-derivation after an abandoned claim is expected; a
  second durable effect from it is a failure.
- **SC-740**: With consolidation stopped and a backlog of at least ten thousand outstanding
  events, event ingestion accepts and persists safe events at a median latency within 20% of its
  latency at an empty backlog, over at least ten trials, and zero ingestion requests are refused
  with a backlog-derived reason.
- **SC-741**: Against an adversarial corpus that attempts to carry raw payloads, transcripts,
  raw tool output, secrets and absolute paths through capture, zero instances reach the
  extraction stage, asserted by inspecting exactly what extraction was handed. The corpus must
  attempt the ingress rather than assume the schema prevents it, and must include material that
  is well-formed for the safe-event schema but carries a secret inside an approved text field.
- **SC-742**: With the extractor replaced by a stub emitting adversarial output — claiming
  durability, a foreign domain, a wider scope, verified status, supersession of existing
  records, and another account's ownership — zero of those claims take effect. Every resulting
  durability, authorization, domain, scope, privacy, verification and supersession outcome is
  identical to the outcome produced with the stub emitting nothing.
- **SC-743**: 100% of `repo_file` values that are absolute, drive-prefixed, UNC-prefixed,
  traversal-bearing, empty, or longer than the contract's stated maximum are refused by the
  server, asserted against an adversarial corpus covering every listed form on both POSIX and
  Windows shapes. The contract states that maximum as a number; an unstated bound fails this
  criterion.
- **SC-744**: For every supported agent, a file-changing tool produces either a valid
  `repo_file`, an out-of-repository marker, or an explicit unavailable-from-vendor marker —
  and never a synthesized value, a working-directory substitute, or a degradation to a generic
  command. The population is every supported agent's file-changing tools, so an agent reporting
  absolute paths is tested rather than excluded.
- **SC-745**: Syntactic variants of one key resolve to a single canonical identity across a
  pre-registered corpus of at least fifty variant groups; zero variant pairs produce two
  distinct records. Separately, 100% of keys that fail validation refuse their candidate and
  zero are repaired into a different valid key.
- **SC-746**: After cutover, 100% of pre-005 knowledge-synchronization writes receive an
  explicit `upgrade_required` result distinguishable from a generic error and from a
  capability-shaped deferral; zero are silently accepted; and the condition is visible to the
  operator. An upgraded client receiving it stops retrying and surfaces the upgrade, asserted on
  the upgraded client rather than on the legacy binary.
- **SC-747**: After an `upgrade_required` refusal, zero durable knowledge records in the legacy
  client's local store are deleted, demoted, truncated or rewritten by Cairn, asserted by
  comparing record-level content before and after rather than by comparing file bytes. The
  client's ordinary local capture and recall continue to function.
- **SC-748**: The consolidation backlog's depth, oldest outstanding event and failure count are
  retrievable at any time, including while a pass is running and immediately after a restart.
- **SC-749**: Extraction requests are confined to one project and one account context in 100% of
  trials; zero requests carry events from a project or account the request was not scoped to.
- **SC-750**: After migration, 100% of pre-existing records carry normalized keys, and a
  candidate matching a legacy record's normalized key collides with it rather than creating a
  second record, over a corpus seeded with un-normalized legacy keys.
- **SC-751**: Every safe-event field carrying a value whose natural name is refused by the
  synchronization boundary uses a distinct declared name; zero refused names appear in the
  safe-event schema.
- **SC-752**: Inducing capture-deadline expiry produces zero agent-facing errors and zero
  blocked agent operations, while 100% of the resulting drops appear in capture health with a
  deadline-specific disposition. A drop that is invisible to health fails this criterion just as
  an agent-facing error does.
- **SC-753**: Zero deadline-drop records contain any portion of the payload being processed when
  the deadline expired.

---

## Clarifications

### Session 2026-08-29

Most ambiguities in the initial direction were resolvable from current `main`, the Feature
001–004 contracts, and the constitution, and were resolved as Assumptions below. Three
questions could not be, and were carried openly rather than defaulted. They are closed in the
session below.

### Session 2026-08-30

The three open questions are now decided, together with three further decisions the first
pass had left implicit. Each is recorded with what was rejected and why, so a later reader can
see that the alternative was considered rather than missed.

- Q: Where does consolidation execute? → A: **In-process background work inside the existing
  server process.** PostgreSQL holds durable backlog and progress state, so a restart loses no
  completed work; an abandoned claim may be reclaimed and re-executed, and re-execution
  produces no duplicate durable effect. Rejected: a new service, a broker or a distributed
  worker platform,
  all of which Principle II forbids without a requirement that exists today; and client-driven
  consolidation, which was rejected on stronger grounds than cost — it would return the
  decision about what becomes durable knowledge to the edge, which is the arrangement this
  feature exists to end. (FR-793a–FR-793d, FR-814, SC-739, SC-740)

- Q: What performs semantic extraction, and where does it run? → A: **On the server, over
  already-approved `SafeCanonicalEvent` records.** The local machine performs vendor parsing,
  normalization, redaction, the deterministic privacy checks and bounded safe-event
  construction — and nothing more. This dissolves the problem rather than solving it: the first
  pass looked for a local process able to hold rich material long enough to extract from it and
  found that none was eligible, then specified a new local transient boundary to fill the gap —
  a requirement since deleted, its identifier now reused for the server-side rule. Moving extraction behind the privacy boundary removes the need for that
  boundary altogether. Rejected: inventing a local transient extraction process, which added a
  new place for private material to live in exchange for nothing. (FR-749, FR-763, FR-763a,
  FR-763b)

- Q: May a model perform extraction, and what may it see? → A: **Yes, restricted to
  `SafeCanonicalEvent` records Cairn was already permitted to persist centrally, scoped to one
  project and one account context.** An earlier answer justified a hosted model on the grounds
  that the material had already legitimately left the machine, so no new egress occurred. That
  reasoning is withdrawn: it is the derivation-as-loophole argument Constitution v1.2.1
  Principle V explicitly refuses. A third-party extractor is a **second recipient at a second
  boundary**, and being permitted to reach the user's own server does not permit forwarding
  anywhere else. A third-party extractor therefore remains possible, but only with the naming,
  disclosure and scoping duties FR-805d imposes. The model may propose content, kind, topic key,
  value key and source event references; it may not decide durability, authorization, domain
  ownership, scope, privacy acceptance, reconciliation, verification or supersession.
  (FR-805a–FR-805e, SC-741, SC-742, SC-752, SC-753)

- Q: Under what field name may a repository-relative path cross the safe-event boundary? → A:
  **`repo_file`**, carrying repository-relative file identity only — relative, slash-normalized,
  non-empty, bounded, no `..` traversal, no leading `/`, no drive or UNC prefix, and validated
  independently by the server. The refused name `path` is not reused. Where a vendor supplies no
  file identity in any form, the event records it as unavailable; a file outside the repository
  is recorded as out-of-repository; and an absolute vendor path — the common case — is
  relativized locally against the repository root, which is never itself transmitted.
  (FR-777b–FR-777g, SC-743, SC-744, SC-751)

- Q: How is a model-proposed key turned into canonical identity? → A: **Cairn normalizes and
  validates it deterministically before it is used for anything.** A proposed key is an input,
  never the identity. `Storage Authority`, `storage_authority` and `storage-authority` resolve
  to one canonical representation. A key that fails validation refuses the candidate rather
  than being silently repaired, because a repaired key changes which existing knowledge the
  candidate collides with. Rejected: trusting model output as identity, and introducing
  embeddings to reconcile variants — the problem is syntactic, and a syntactic function solves
  it. (FR-796a–FR-796c, SC-745)

- Q: What happens to a Feature 004 client after the server cuts over to server-authoritative
  mode? → A: **Its knowledge synchronization is refused with an explicit `upgrade_required`
  result, and its local data is left intact.** It upgrades, migrates, establishes canonical
  possession, and only then are its local replicas demoted. Rejected: preserving the
  dual-authority convergence system indefinitely against the possibility that a dormant device
  returns — that made retirement of the machinery contingent on a device that might never come
  back, and it is the reason the first pass could not state when convergence could be removed.
  (FR-876–FR-877, SC-746, SC-747)

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
- Moving extraction to the server does not weaken the privacy boundary, because the boundary
  is unchanged and still runs locally: the server only ever sees what that boundary already
  approved. Extraction gains richer material than a millisecond-budget hook could produce, and
  gains it without any new class of data leaving the machine.
- Extraction runs on the server; whether the extractor is a model the server operator runs
  themselves or a third-party service is a deployment choice, not an architectural one. A
  third-party extractor is permitted but carries the naming, disclosure and scoping duties
  Principle V sets out, and the baseline assumes no particular extractor.
- **No hosted extraction provider is selected by this specification, and none is assumed
  compliant.** Provider retention and training behaviour is configuration-, account- and
  model-dependent; a default hosted API configuration cannot be assumed to satisfy Constitution
  v1.2.1's requirement that a third-party extractor store nothing and coordinate nothing.
  Before any hosted extractor is selected during planning, Phase 0 MUST establish from the
  provider's current official documentation: the provider, the model, the endpoint, customer
  content retention, whether submitted content is used for training or model improvement,
  eligibility for a zero-retention or no-training mode, prompt or application-state caching,
  project and account isolation, the disclosure the provider requires be made to end users, and
  the behaviour when a compliant mode is unavailable. If compliance cannot be established for
  the actual deployment, the plan MUST evaluate a compliant self-hosted or deterministic
  alternative, or report the blocker. It MUST NOT record compliance it did not verify.
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
  existing Rust server and daemon are extended; consolidation is in-process background work.
- **Client-driven consolidation** — rejected on architecture rather than cost. Deciding what
  becomes durable knowledge at the edge is the arrangement this feature exists to end.
- **A local transient extraction process** — considered and removed. Moving extraction behind
  the privacy boundary makes it unnecessary, and it would have created a new place for private
  material to live in exchange for nothing.
- **Indefinite dual-authority convergence for dormant legacy devices** — a returning pre-005
  client is refused with `upgrade_required`, not merged.
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
