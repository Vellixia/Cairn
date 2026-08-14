# Feature Specification: Cairn Project Intelligence

**Feature Branch**: `003-project-intelligence`

**Created**: 2026-08-14

**Status**: Draft

**Baseline**: `main` @ `0b79b314f616df27409370b8dce54193c092a1fc` (v0.1.0-alpha.4)

**Input**: Cairn maintains consistent, compact, evidence-backed project knowledge and work
continuity across agents, sessions, compression boundaries, worktrees, devices, and projects.

## Overview

Feature 001 gave Cairn memory. Feature 002 gave every supported agent access to it. Both
succeeded, and both left the same gap: Cairn records what a session *said*, and treats every
statement as equally true, equally current, and equally worth repeating.

That is fine for ten memories. At a thousand it becomes persistent confusion. Three sessions
each record "we use PostgreSQL"; all three come back. One session records CockroachDB after a
migration; now two contradictory facts are both `active` and neither is marked. A memory
recorded when the API listened on 8080 stays confident about 8080 forever. A session that
compacts loses its goal to a provider's summariser. A second session changes the task while the
first is mid-compaction, and the first resumes editing a file that has moved on. A hard-won fix
in one project is invisible in the next.

Feature 003 closes that gap.

> **Feature 003** — Cairn maintains what should be believed, what is still true, what changed,
> what another agent already accomplished, what can safely transfer to another project, and the
> minimum information required to continue correctly.

### The core principle

> **Cairn never converts one agent's statement directly into shared project truth.**

```text
agent activity
    ↓
observation                     (Feature 001, unchanged)
    ↓
knowledge proposal / candidate  (a memory, attributed to its session)
    ↓
reconciliation                  (deterministic: reinforce, duplicate, supersede, conflict)
    ↓
canonical project knowledge     (a derived subject head, rebuildable from proposals)
    ↓
evidence                        (bounded, attributable, privacy-safe)
    ↓
verification                    (deterministic checks only)
    ↓
trusted / bounded context       (minimum safe context first, then ranked relevance)
```

Cross-project reuse is a second, stricter ladder:

```text
project knowledge
    ↓
promotion candidate             (agent- or user-proposed, never automatic)
    ↓
privacy sanitization            (deterministic refusal, not best effort)
    ↓
applicability definition        (signals + conditions, explicitly bounded)
    ↓
reusable pattern
    ↓
independent validation / counterexamples
    ↓
higher trust
```

Two ownership rules follow from the principle and govern the whole feature:

- **Sessions own observations. Projects own canonical project knowledge.** A session
  contributes proposals; it never writes the project's answer directly.
- **Tasks are project-owned work state.** Sessions contribute task deltas; no session holds an
  independent copy of task truth.

### What this reuses rather than rebuilds

Feature 003 is an extension of the existing architecture, not a parallel one. It reuses:

- the `memory` record with its types, scopes, states, provenance, `local_only` flag,
  supersession link and observation evidence references (Feature 001);
- FTS5/BM25 lexical retrieval with scope-first ranking — no embeddings, no vector store, no
  graph database;
- the deterministic token-budgeted context assembler and its measure-before-emit loop;
- the derived handoff and its sealed session-close boundary;
- `BEGIN IMMEDIATE` plus bounded retry as the local concurrency mechanism;
- the transactional outbox, its idempotency keys, and the server's `sync_state` claim as the
  synchronization mechanism;
- Feature 002's seven canonical lifecycle events, its capability model with its
  availability/confidence distinction, and its honest-degradation rule;
- exactly six MCP tools.

It adds no new datastore, no new service, no new transactional model, and no seventh tool.

## Clarifications

### Session 2026-08-14

Every material ambiguity in the initial draft was resolvable from the current implementation, the
Feature 001 and Feature 002 contracts, and the constitution. No question required a product
decision with materially different outcomes, so none was escalated. The resolutions and the
alternatives rejected are recorded here.

- Q: How does Cairn decide that two differently-worded free-form memories concern one subject?
  → A: **It does not.** Automatic reconciliation is limited to equal value keys on one subject, or
  identical content after normalization. Rejected: model- or embedding-assisted equivalence, which
  FR-511 forbids in the correctness path and which would make reconciliation non-deterministic.
- Q: Does evidence content synchronize to the shared server?
  → A: **No.** A shared memory carries only its verification state, the instant of verification, the
  count of supporting facts and the kinds of verifier involved. Rejected: extending the server
  allowlist to observed values and locators, which would widen Feature 001's privacy contract.
- Q: Do reusable patterns synchronize, and is the source project visible?
  → A: **Patterns are local to the machine and never synchronize; the origin is an opaque local
  reference.** Rejected: team-shared patterns, deferred to a future feature.
- Q: What triggers a memory's `conflicted` **verification** state, as distinct from a subject's
  `conflicted` **reconciliation** state? → A: Evidence attached to that one memory both supports and
  contradicts it, or two verifier runs of the same kind disagree at the same repository state.
  Subject-level conflict is a property of the claim set, not of a verification run; the two are
  reported separately.
- Q: Are `drifted` memories returned by default retrieval? → A: **Yes.** A drifted memory stays
  lifecycle-`active` and is returned, always accompanied by its drift warning. Rejected: hiding it,
  which would make an agent silently re-derive knowledge Cairn holds.
- Q: Does reserving budget for Level 0 reduce what today's briefings show?
  → A: **Only when Level 0 content actually exists.** The reserve is a cap on what Level 1 and
  Level 2 may not take, not a floor Level 0 must spend. With no task, no warnings and no pins,
  Level 0 costs almost nothing and Level 1 receives the whole budget. The default context budget is
  unchanged.
- Q: Can an agent's attested evidence make a task criterion `verified`?
  → A: **No.** A criterion reaches `verified` only on Cairn-collected evidence — a captured test or
  command observation with its exit code, a file digest, a Git ref, or a configuration value Cairn
  read. Attested evidence can support a memory, labelled as attested, but never a criterion's
  verification. This is what keeps "the agent said it passes" out of completion readiness.
- Q: What can `importance` do? → A: **Rank within a bucket, nothing more.** It never changes scope
  precedence and never admits an item into Level 0. Rejected: importance as a soft scope override,
  which would reintroduce ambient memory.
- Q: What happens when the shared server predates Feature 003?
  → A: The daemon degrades to Feature 001 synchronization semantics for the affected entities,
  keeps working, and reports the degradation in sync status. Rejected: failing the sync.
- Q: Which term names a subject's derived current value? → A: **canonical answer**, used
  consistently; a subject may have one, several when conflicted, or none when every member is
  historical.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Canonical project knowledge (Priority: P1) 🎯 First usable slice

Several sessions, over weeks, record the same thing about a project. A developer opens a new
session and receives that knowledge once, as the project's current answer, with the evidence
that it has been seen repeatedly — not three near-identical lines competing for budget.

**Why this priority**: Without it, every later capability inherits a knowledge base whose
duplicates crowd out the things that matter. This is the slice that makes memory stop degrading
as it grows.

**Independent Test**: Record equivalent knowledge from three sessions on one project, then read
the briefing and the search results. One canonical item, reinforcement recorded, all three
provenance chains resolvable.

**Acceptance Scenarios**:

1. **(A)** **Given** three sessions that each recorded the same subject with an equivalent value —
   either the same topic key and value, or identical normalized content — **When** the project
   briefing is assembled, **Then** exactly one active canonical item for that subject appears,
   its reinforcement count is 3, its distinct-origin count is 3, and each contributing memory
   remains individually retrievable with its own session provenance.
2. **(B)** **Given** a project-scoped memory "production database is PostgreSQL" and a
   task-scoped memory "this integration fixture uses SQLite", **When** reconciliation runs,
   **Then** no conflict is declared, the task-scoped memory is the applicable answer inside that
   task, and the project answer is presented as the broader one it narrows.
3. **Given** a free-form memory with no topic key, **When** reconciliation runs, **Then** it is
   never merged with, superseded by, or reinforced against another memory on the basis of
   similarity, and it continues to be searchable and briefable exactly as in Feature 001.
4. **Given** a project with no topic keys anywhere, **When** every Feature 001 and Feature 002
   behaviour is exercised, **Then** nothing changes in what the developer sees except the
   additional fields being reported as `unverified` and `null`.

---

### User Story 2 - Knowledge evolves safely (Priority: P1)

A decision is replaced. The new answer becomes current, the old answer stays readable as
history, and a session that ran six weeks ago can still be understood against what was true
then.

**Why this priority**: Supersession already exists but carries no time semantics, so
"what did the agent believe in July" is unanswerable. Debugging an old session needs it.

**Independent Test**: Supersede a memory, then query current knowledge and knowledge as of a
past instant. Different answers, both correct, neither rewritten.

**Acceptance Scenarios**:

1. **(D)** **Given** memory X superseded by memory Y at time T, **When** current project
   knowledge is requested, **Then** Y is returned and X is not; **When** knowledge as of a time
   before T is requested, **Then** X is returned as what was then current, and X's content,
   provenance and evidence are byte-identical to what they were before the supersession.
2. **Given** a superseded memory, **When** anything in the system later changes — new evidence,
   a verification run, a sync from another device — **Then** the superseded memory's own content
   is never rewritten.
3. **Given** a session that ran while X was current, **When** that session's handoff and
   observations are read back, **Then** the knowledge they are interpreted against is the
   knowledge that was effective during the session, not today's.

---

### User Story 3 - Conflicts are visible (Priority: P1)

Two agents, or two developers, record incompatible things about the same subject. Cairn does not
pick one. It says both were proposed, says they disagree, and says who proposed each.

**Why this priority**: A silent winner is worse than a visible conflict — the losing knowledge
disappears with no trace, and the agent proceeds confidently on a coin flip.

**Independent Test**: Have two sessions propose incompatible values for one topic in one scope.
Both remain active, the subject reports `conflicted`, and the briefing carries a conflict
warning.

**Acceptance Scenarios**:

1. **(C)** **Given** two active memories with the same topic key, the same scope and scope key,
   and incompatible values, **When** reconciliation runs, **Then** the subject's reconciliation
   state is `conflicted`, both memories remain `active`, neither is marked superseded, no
   canonical single answer is emitted, and the conflict appears in the briefing's warnings.
2. **(E)** **Given** two concurrent sessions on one machine — one Claude Code, one Codex —
   proposing incompatible values for one topic at the same moment, **When** both writes complete,
   **Then** both memories exist with distinct identities and intact provenance, no write is lost,
   and the outcome does not depend on which committed first.
3. **Given** a conflicted subject, **When** an agent or developer resolves it by explicitly
   superseding one side, or by narrowing one side's scope, or by attaching verification that
   distinguishes them, **Then** the resolution is recorded with its author, its basis and its
   timestamp, and the subject leaves `conflicted`.
4. **Given** a conflicted subject, **When** no one resolves it, **Then** Cairn never resolves it
   on its own, and never lets recency, ordering or identifier order decide a winner.

---

### User Story 4 - Evidence-backed knowledge (Priority: P2)

A fact about the project can carry the safe, bounded thing that supports it — a file digest, a
Git ref, a configuration value, a recorded test outcome — and can be reported as verified
against it.

**Why this priority**: Verification is what separates "an agent said so" from "this is
established". It is P2 rather than P1 because canonical reconciliation is useful without it.

**Independent Test**: Attach deterministic evidence to a memory, run verification, read the
memory's verification state and basis.

**Acceptance Scenarios**:

1. **(G)** **Given** a memory asserting a configuration value and an evidence fact recording
   that value with its digest and its in-repository locator, **When** verification runs, **Then**
   the memory's verification state becomes `verified`, and the record names what was checked,
   against which evidence, when, and at which branch and commit.
2. **Given** a memory whose only support is an agent's assertion that it is important, **When**
   verification runs, **Then** the memory stays `unverified`, and the agent's assertion is
   recorded as a proposal rather than as verification.
3. **Given** evidence collected by an agent rather than by Cairn — a test exit code, a runtime
   response — **When** it is attached, **Then** it is recorded as agent-attested, is usable to
   reach `verified`, and is distinguishable from evidence Cairn read itself.
4. **Given** a configuration file containing a credential, **When** evidence is recorded from it,
   **Then** the stored evidence carries the subject, the observed value after redaction, a digest
   and a locator, and never the raw secret-bearing text.

---

### User Story 5 - Drift detection (Priority: P2)

The configuration changes. Cairn does not quietly rewrite what it remembers, and it does not go
on asserting the old value as verified. It says the support moved and the claim needs
rechecking, then says whether it still holds.

**Why this priority**: Silent mutation on one changed file is the fastest way to corrupt a
knowledge base. Drift has to be a state, not an edit.

**Independent Test**: Verify a memory, change the evidence, observe `needs_recheck`, re-verify,
observe `drifted`, confirm the memory content is unchanged throughout.

**Acceptance Scenarios**:

1. **(H)** **Given** a `verified` memory, **When** its supporting evidence's fingerprint changes,
   **Then** the memory's verification becomes `needs_recheck`, its content, type, scope and
   provenance are unchanged, and no new memory is created.
2. **(I)** **Given** a memory in `needs_recheck` whose re-verification finds a different value,
   **Then** the memory's verification becomes `drifted`, the memory itself stays intact and
   lifecycle-`active`, the drift is reported in the briefing's warnings, and a superseding memory
   is created only when someone explicitly creates it.
3. **Given** a memory in `needs_recheck` whose re-verification finds the same value, **Then** it
   returns to `verified` with a new `last_verified_at`.
4. **Given** a memory whose evidence can no longer be read at all — the file is gone, the ref is
   unresolvable — **Then** re-verification is `inconclusive`, the memory stays in
   `needs_recheck`, and Cairn does not report it as either verified or drifted.

---

### User Story 6 - Compression-safe continuity (Priority: P1)

A long session compacts, repeatedly. Afterwards the agent still knows the goal, the acceptance
criteria, what is done, what is left, what is blocking it, which approaches were already rejected,
and what to do next — because that came from Cairn, not from a summariser.

**Why this priority**: This is the capability developers feel immediately, and the one Cairn is
uniquely positioned to provide. It is also where a wrong answer is most dangerous, which is why
staleness detection is part of the same story.

**Independent Test**: Drive ten compaction cycles against a real repository and assert that a
fixed set of continuity fields survives every one. Then have a second session change the task and
the branch head mid-flight and assert the divergence is reported.

**Acceptance Scenarios**:

1. **(J)** **Given** a session with a bound task, recorded progress, an open blocker, decisions,
   a pinned constraint and known failures, **When** it crosses ten compaction boundaries,
   **Then** after each one the delivered context still carries the goal, the acceptance criteria
   with their states, the derived progress, the open blocker, the active decisions, the critical
   constraints, the rejected approaches and a next action — and none of them is a paraphrase of a
   conversation.
2. **(K)** **Given** a checkpoint recorded at commit `abc123` with task revision 7, **When**
   another session advances the branch head to `def456`, advances the task to revision 8 and
   modifies a file the checkpoint named, and the first session then resumes, **Then** the
   checkpoint is reported `diverged`, the context names the commit change, the revision change
   and the file with the session that changed it, and the recorded next action is presented as
   possibly stale rather than as the instruction to follow.
3. **(S)** **Given** an agent whose adapter has no post-compaction signal, **When** continuity is
   reported, **Then** Cairn reports continuity as agent-initiated rather than automatic, names
   the tool call that retrieves it, and never claims guaranteed rehydration.
4. **Given** an agent with no pre-compaction signal either, **When** continuity is reported,
   **Then** Cairn reports compression-safe continuity as unavailable automatically for that
   agent, and the checkpoint written at the session's other boundaries is still available on
   demand.

---

### User Story 7 - Multi-agent, multi-device consistency (Priority: P1)

Two machines, offline, both change the project's answer. They reconnect. Nothing is lost and
nothing is decided by whose clock was later.

**Why this priority**: Cairn is local-first and explicitly supports several agents, worktrees and
machines. A merge that silently drops one side would make shared memory untrustworthy.

**Independent Test**: Two independent local stores linked to one shared project, disjoint offline
writes on the same subject, sync in both directions, assert both survive and the subject is
conflicted. Then repeat with the write order and the clocks reversed and assert the same outcome.

**Acceptance Scenarios**:

1. **(F)** **Given** machine A recording "production database is PostgreSQL" and machine B
   recording "production database is CockroachDB", both offline, **When** both sync, **Then** both
   proposals exist on both machines with their original provenance, the subject is `conflicted` on
   both, and no timestamp comparison contributed to the outcome.
2. **Given** the same scenario with the two machines' system clocks reversed relative to each
   other, **When** both sync, **Then** the merged state is identical to the previous scenario.
3. **Given** a supersession decided on machine A, **When** machine B syncs, **Then** B applies the
   same supersession from the recorded decision rather than by overwriting the memory row, and B's
   canonical answer matches A's.
4. **Given** two worktrees of one repository with concurrent sessions, **When** both propose
   knowledge, **Then** both are attributed to their own session and worktree and neither is lost.
5. **Given** a memory verified on machine A, **When** machine B reads it, **Then** B reports it as
   verified elsewhere rather than as verified here, and B's own completion readiness does not
   count it.

---

### User Story 8 - Reusable cross-project knowledge (Priority: P3)

A problem solved in one project helps in the next — as a clearly labelled prior pattern with
stated applicability, not as a fact that has silently become universal.

**Why this priority**: Real value, but it depends on everything above being trustworthy first,
and it carries the highest privacy risk in the feature.

**Independent Test**: Promote a verified procedure from project A to a reusable pattern, then work
in project B on a matching symptom and confirm the suggestion arrives labelled unverified for B.

**Acceptance Scenarios**:

1. **(L)** **Given** a verified, evidence-backed procedure in project A promoted to a reusable
   pattern, **When** project B records a matching signal, **Then** the pattern is offered in
   project B's context as a prior pattern, is explicitly marked as not verified in project B, and
   states its applicability conditions and its caveats.
2. **(R)** **Given** a candidate memory containing an absolute path, the project's name, its
   repository remote, a shared project identifier or a redaction-surviving secret shape, **When**
   promotion is attempted, **Then** promotion is refused, the refusal names the offending class
   without echoing the value, and no partial pattern is created.
3. **Given** a `local_only` memory, **When** promotion is attempted, **Then** promotion is
   refused.
4. **Given** a promoted pattern, **When** the origin project or the source memory is deleted,
   **Then** the pattern survives, its origin reference resolves to "origin deleted", and no
   project-identifying information appears anywhere in it.
5. **Given** an unverified or conflicted source memory, **When** promotion is attempted, **Then**
   promotion is refused with the reason named.

---

### User Story 9 - Counterexamples prevent poisoning (Priority: P3)

A pattern that looks right and is wrong gets recorded as wrong. Repetition of the same incident,
or of Cairn's own suggestion, does not make a pattern trusted.

**Why this priority**: Without it, cross-project reuse becomes a confident-nonsense amplifier.
It is inseparable from User Story 8 and is specified alongside it.

**Independent Test**: Record a not-applicable outcome with a different root cause and assert the
pattern is contested rather than reinforced, then repeat one project's incident ten times and
assert independence accounting does not move.

**Acceptance Scenarios**:

1. **(M)** **Given** a pattern suggested in project B where the true cause turns out to be
   different, **When** the outcome is recorded as not applicable with the alternative cause,
   **Then** the pattern's success accounting does not increase, the counterexample is stored, the
   pattern is not deleted, and later suggestions carry both the pattern and the known alternative
   cause with what to check first.
2. **(N)** **Given** ten sessions in project A that all describe the same incident, **When**
   independence is accounted, **Then** the pattern's distinct-project validation count is 1, not
   10, and its trust does not advance on repetition alone.
3. **Given** a pattern applied in project B only because Cairn suggested it, **When** the outcome
   is recorded as resolved without deterministic evidence in project B, **Then** the application
   is recorded as Cairn-suggested and does not advance the pattern's trust.
4. **Given** a pattern with both successes and counterexamples, **When** it is offered, **Then**
   it is offered as contested with both sides stated.

---

### User Story 10 - Minimum safe context (Priority: P1)

A project with thousands of memories still hands the agent a briefing that leads with the things
it cannot work without.

**Why this priority**: Feature 003 adds warnings, criteria states and continuity to a budget that
was already fully spent. Without a reserved floor, the new signal is the first thing dropped.

**Independent Test**: Build a project with thousands of memories, assemble the briefing at a tight
budget, and assert the reserved content is present and the budget is not exceeded.

**Acceptance Scenarios**:

1. **(O)** **Given** a project with 5,000 memories, a relevant pinned constraint, an active drift
   warning and a conflicted subject, **When** the briefing is assembled at any budget from the
   documented minimum upwards, **Then** the current task with its criteria states, the next
   action, the pinned constraint, the drift warning and the conflict warning are all present, the
   estimated tokens never exceed the budget, and the omissions are reported.
2. **Given** a budget below the documented minimum, **When** the briefing is assembled, **Then**
   it is truncated rather than rejected, the reserved content is admitted in its documented
   internal order, and what was dropped is named.
3. **Given** an unlimited number of low-priority memories, **When** the briefing is assembled,
   **Then** they can never displace reserved content.
4. **Given** any briefing, **When** its selection is inspected, **Then** every included item names
   why it was selected and every excluded section names why it was omitted.

---

### User Story 11 - Evidence-aware tasks (Priority: P2, secondary)

A task's acceptance criteria stay stable across sessions. Each criterion can be updated on its
own, can carry evidence, can be blocked, and the task can report how ready it is — without Cairn
becoming a project tracker.

**Why this priority**: Secondary by design. It exists because continuity and verification need a
task model with stable parts, not because Cairn is becoming a planning tool.

**Independent Test**: Two sessions update different criteria on one task; both changes survive.
An older session detects that the task advanced and receives the changes.

**Acceptance Scenarios**:

1. **(P)** **Given** a task with four criteria and two sessions that each update a different
   criterion, **When** both updates complete, **Then** both are present, no criterion was reset by
   the other session's write, and the task's revision reflects both changes.
2. **(Q)** **Given** a session that bound the task at revision 5 and a task now at revision 6,
   **When** that session's context is refreshed, **Then** it is told the task advanced from 5 to
   6 and which criteria, blockers, goal or status changed, and Cairn does not present the session
   as having always worked against revision 6.
3. **Given** every criterion satisfied and verified and no open blocker, **When** readiness is
   derived, **Then** it reports ready, and the task's status is not changed to done by Cairn.
4. **Given** an agent asserting that a criterion is satisfied with no evidence, **When** progress
   is derived, **Then** the criterion is reported satisfied-but-unverified and is counted
   separately from verified.
5. **Given** an agent that reports "80% complete", **Then** no such field exists to store it, and
   progress is reported only as derived counts.

---

### Edge Cases

- **A topic key that means two different things.** Two projects, or two parts of one project, use
  the same dotted key for unrelated subjects. Subjects are scoped by project, scope and scope key,
  so cross-project collision cannot occur; within a project it is a genuine modelling mistake and
  is visible as an implausible conflict rather than a silent merge.
- **A malformed or hostile topic key.** Over-long, deeply nested, non-ASCII, or shaped like a
  path. Normalization and validation reject it and the memory is stored free-form rather than
  rejected outright.
- **Reinforcement of a superseded memory.** A late session reinforces the value that has since
  been replaced. The reinforcement is recorded against the memory it names; it does not resurrect
  it, and it does not decrement the successor.
- **A subject whose every member is superseded.** The subject has no canonical answer; it reports
  as historical and contributes nothing to the briefing.
- **Evidence for a path that is excluded by privacy configuration.** No evidence fact is created,
  and the memory stays unverified with the reason recorded as excluded rather than as missing.
- **A verifier whose target is outside the worktree.** Refused. Cairn reads inside the project
  worktree and inside Git; nothing else.
- **An evidence locator that becomes ambiguous after a rename.** Re-verification is inconclusive
  and the memory returns no stronger than `needs_recheck`.
- **A checkpoint whose task no longer exists.** Reported `unresolvable`; the continuity fields that
  do not depend on the task are still delivered.
- **A checkpoint from a deleted session.** Deletion tombstones the checkpoint with the session, as
  Feature 001 does for handoffs; nothing dangles.
- **Compaction with no bound task.** Continuity still carries repository state, decisions,
  failures, pinned constraints and next action.
- **Ten thousand evidence facts and a 250 ms capture deadline.** Drift marking is an indexed
  lookup with a documented per-event cap; exceeding the cap defers rather than blocks.
- **Verification that never finishes.** The memory stays `needs_recheck` indefinitely, which is a
  valid reported state and never an error.
- **A pinned memory that drifts.** It stays pinned and stays in reserved context, carrying its
  drift warning — a constraint that no longer holds is precisely what must be surfaced.
- **The pin budget exhausted.** Pinning is refused with the current pins named, and nothing is
  silently unpinned.
- **A pattern whose signals match everything.** Signal sets below a documented minimum specificity
  are refused at promotion; a pattern that matches indiscriminately is worse than none.
- **A pattern promoted from a project that is later unlinked.** Unchanged: the pattern never held
  project identity.
- **A shared project whose server has no Feature 003 columns.** The daemon degrades to Feature 001
  sync semantics for the affected entities and reports it; it does not fail the sync.
- **A newer daemon reading an older store, or the reverse.** The existing schema-version guard
  applies unchanged: an older build refuses a newer schema rather than writing against it.
- **A store with existing memories, handoffs, tasks and outbox rows.** Migration is additive and
  defaults are explicit; nothing is fabricated to satisfy a new column.

## Requirements *(mandatory)*

### Proposal boundary and canonical knowledge

- **FR-301**: A session's knowledge contribution MUST be recorded as an attributed proposal. Cairn
  MUST NOT allow any agent statement to become the project's canonical answer without passing
  through reconciliation.
- **FR-302**: The **canonical answer** for a subject MUST be a **deterministic derivation** over that
  subject's proposals and the recorded reconciliation decisions. Given the same proposals and
  decisions, the derivation MUST produce the same result, and it MUST be possible to discard and
  rebuild every derived canonical value from those durable inputs alone. A subject MUST be able to
  have one canonical answer, several when it is conflicted, or none when every member is historical.
- **FR-303**: The derivation MUST NOT read wall-clock creation or update timestamps, or identifier
  ordering, to choose between competing proposals. Those MAY be used only as a final total-order
  tiebreak for stable output ordering, never to declare one proposal correct.
- **FR-304**: Every reconciliation decision — reinforcement, duplication, supersession, conflict,
  scope narrowing, conflict resolution — MUST be stored as a durable, append-only record carrying
  the memories it relates, the relation kind, the deciding session, the time, and the **basis** on
  which it was decided: a deterministic rule, attached evidence, or an explicit agent or user
  instruction.
- **FR-305**: A reconciliation decision record MUST be idempotent: recording the same decision
  twice MUST leave the same state and MUST NOT double-count reinforcement.
- **FR-306**: Cairn MUST NOT delete, rewrite or truncate the content of a proposal as a
  consequence of reconciliation. The only fields reconciliation may change on an existing memory
  are its lifecycle state, its supersession link, and cached derivations of the durable records
  above.
- **FR-307**: Cairn MUST report, for any subject, its members, its canonical answer or answers, its
  reconciliation state, and the decisions that produced it.
- **FR-308**: A memory MAY carry an **importance** hint. It MUST affect only ordering among
  candidates already selected within one ranking bucket. It MUST NOT change scope precedence,
  MUST NOT admit an item into reserved context, and MUST NOT affect reconciliation, verification or
  promotion.

### Subject identity

- **FR-311**: A memory MAY carry an optional **topic key** identifying the subject it concerns, and
  an optional **value key** identifying the comparable value it asserts. A value key MUST NOT be
  accepted without a topic key.
- **FR-312**: Topic keys MUST be normalized deterministically before storage — lower-cased, a
  bounded set of characters, a bounded number of dot-separated segments, a bounded total length.
  A key that cannot be normalized MUST NOT reject the memory; the memory MUST be stored without a
  topic key and the reason MUST be reported.
- **FR-313**: Cairn MUST NOT require a topic key. Free-form memories MUST remain fully valid,
  searchable, briefable and syncable, and MUST behave exactly as they do in Feature 001.
- **FR-314**: Cairn MUST NOT define, ship or enforce a vocabulary, taxonomy or registry of allowed
  topic keys. Keys are minted by whoever records the memory.
- **FR-315**: A **subject** MUST be identified by project, memory scope, scope key and topic key
  together. Two memories with the same topic key in different scopes or scope keys MUST NOT be
  members of the same subject.
- **FR-316**: Automatic reconciliation MUST be limited to cases decidable without inference:
  same subject with equal value keys, or same subject and scope with identical normalized content.
  Everything else MUST remain a candidate requiring evidence or an explicit decision.
- **FR-317**: Cairn MUST NOT infer that two differently-worded free-form memories concern the same
  subject, and MUST NOT merge, supersede or reinforce them on the basis of similarity.
- **FR-318**: Agents MUST be able to propose a topic key and value key when recording knowledge,
  and to attach one to an existing memory, through the existing tool surface.

### Reinforcement, duplication and supersession

- **FR-321**: When a proposal matches an existing subject member with an equal value key, Cairn
  MUST record a reinforcement rather than a second competing active answer, and MUST keep the new
  proposal individually retrievable with its own provenance.
- **FR-322**: Reinforcement accounting MUST distinguish the number of reinforcements from the
  number of **distinct origin sessions**, and MUST NOT present a repetition count as independent
  confirmation.
- **FR-323**: Supersession MUST preserve the superseded memory, its provenance, its evidence
  references and its content unchanged, and MUST record the link in both directions.
- **FR-324**: Supersession MUST remain expressible exactly as in Feature 001, and Feature 001's
  supersession link MUST continue to reflect the current supersession relation.
- **FR-325**: Cairn MUST NOT supersede a memory automatically. Supersession requires an explicit
  instruction or a verification result that establishes the replacement.
- **FR-326**: Duplicate detection MUST be deterministic and content-exact after normalization. No
  fuzzy, semantic, embedding-based or model-assisted equivalence may affect stored state.

### Conflict

- **FR-331**: Cairn MUST distinguish **semantic conflict** — two applicable proposals that
  disagree — from **concurrent write conflict** — two writers changing the same mutable state at
  the same time — and MUST handle them by different mechanisms.
- **FR-332**: Semantic conflict MUST be declared only where scopes overlap: the same scope and
  scope key. Memories at different scopes, or at the same scope with different scope keys, MUST
  NOT be declared in conflict with one another.
- **FR-333**: Where a narrower-scoped memory disagrees with a broader-scoped one on the same
  topic, Cairn MUST treat this as a **scope exception**, MUST apply the narrower memory in its own
  context by existing scope precedence, and MUST present the broader answer as the one it narrows.
- **FR-334**: A conflicted subject MUST keep every member `active` and attributed, MUST emit no
  single canonical answer, MUST return every competing answer, and MUST surface the conflict in the
  briefing's reserved warnings.
- **FR-335**: Conflict resolution MUST be explicit and MUST record its basis: an explicit
  supersession, an explicit scope narrowing, or a verification result that distinguishes the
  members.
- **FR-336**: Concurrent write conflict on canonical knowledge MUST be structurally impossible:
  proposals are distinct records, and decisions are append-only and idempotent. There MUST be no
  path by which one writer's proposal or decision replaces another's.
- **FR-337**: Concurrent write conflict on mutable work state MUST be handled by explicit revision
  comparison, not by timestamp. Where a caller supplies the revision it read and that revision has
  advanced, the write MUST be refused with the current state named.

### Temporal truth

- **FR-341**: A memory MUST carry the instant from which it is effective, and, once superseded, the
  instant from which it no longer is.
- **FR-342**: Cairn MUST be able to answer both "what is the best-supported current project
  knowledge" and "what was effective at a given instant", and MUST distinguish the two in its
  output.
- **FR-343**: Answering a historical question MUST NOT modify any record.
- **FR-344**: A memory MUST carry the instant it was last verified, distinct from the instants it
  was created, updated or made effective.
- **FR-345**: Cairn MUST NOT implement general bitemporal storage, retroactive correction of
  effective intervals, or branching histories.

### Evidence

- **FR-351**: Cairn MUST support **evidence facts** as bounded, attributable records of an observed
  state of the world, covering at least: a captured observation, a file's existence or digest, a
  Git ref or commit, a configuration value, a recorded test outcome, a recorded command outcome, an
  explicitly submitted runtime or API response, and a schema version.
- **FR-352**: An evidence fact MUST carry: its kind, the subject it describes, the observed value
  after redaction and bounding, a digest, a source locator, a fingerprint used for change
  detection, when it was collected, which session collected it, and the branch and commit it was
  collected at.
- **FR-353**: An evidence fact's source locator MUST be repository-relative or a Git reference. It
  MUST NOT contain an absolute machine path.
- **FR-354**: An evidence fact MUST pass through Feature 001's exclusion, redaction and
  payload-bounding pipeline before storage. Cairn MUST NOT store raw secret-bearing configuration
  in order to support a memory; it MUST store the safe fact, its digest and its locator.
- **FR-355**: Cairn MUST distinguish evidence it collected itself from evidence an agent attested.
  Agent-attested evidence MUST be usable, MUST be labelled, and MUST NOT be re-executed by Cairn.
- **FR-356**: Feature 001's observation-reference provenance MUST continue to work unchanged, and a
  memory MUST remain valid with zero evidence of any kind.
- **FR-357**: An evidence fact MUST be able to become stale, and MUST be able to be rechecked where
  a deterministic verifier exists for its kind.
- **FR-358**: Deleting an evidence fact, or the observation behind it, MUST leave the reference
  resolvable and reported as deleted, exactly as Feature 001 does for observation evidence.
- **FR-359**: Evidence facts MUST link to memories with an explicit role, at least **supports** and
  **contradicts**.

### Verification

- **FR-361**: Verification MUST be deterministic. A model's opinion MUST NOT constitute
  verification.
- **FR-362**: A memory MUST carry a verification state distinct from its lifecycle state, with at
  least: `unverified`, `verified`, `needs_recheck`, `drifted`, `conflicted`. Lifecycle state MUST
  remain exactly Feature 001's `active`, `stale`, `superseded`. The two MUST NOT be collapsed into a
  single value anywhere in storage or in output.
- **FR-369**: A memory's `conflicted` verification state MUST mean that evidence attached to that
  memory both supports and contradicts it, or that two verifier runs of the same kind disagree at
  the same repository state. It MUST NOT be used to express a subject-level disagreement between
  two memories, which is a subject reconciliation state (FR-334), and the two MUST be reported
  separately.
- **FR-363**: A verification run MUST record what was checked, which verifier was used, against
  which evidence, when, at which branch and commit, and its result: verified, drifted or
  inconclusive.
- **FR-364**: Verification runs MUST be append-only. A later run MUST NOT overwrite an earlier
  run's record; only the memory's cached current verification state may change.
- **FR-365**: Cairn's own verifiers MUST be limited to reading inside the project worktree, subject
  to privacy exclusions, and to Git plumbing. Cairn MUST NOT execute arbitrary commands, run test
  suites, or reach the network to verify a memory.
- **FR-366**: Where a verifier's target cannot be read, the result MUST be `inconclusive` and the
  memory MUST NOT become either `verified` or `drifted`.
- **FR-367**: An agent MUST be able to propose a verification and to submit deterministic evidence
  for it, and Cairn MUST record the distinction between the proposal, the evidence and the result.
- **FR-368**: A verification state imported from another machine MUST be labelled as established
  elsewhere, MUST NOT be presented as verified here, and MUST NOT contribute to locally derived
  readiness.

### Drift

- **FR-371**: When an evidence fact's fingerprint changes, Cairn MUST set the verification state of
  every memory that fact supports to `needs_recheck`. It MUST NOT change the memory's content,
  type, scope, provenance or lifecycle state, and MUST NOT create a memory.
- **FR-372**: Cairn MUST NOT rewrite canonical project knowledge because a single evidence source
  changed. A new or superseding proposal MUST be created explicitly.
- **FR-373**: A drifted memory MUST remain historically intact, MUST remain lifecycle-`active`, and
  MUST continue to be returned by default retrieval and by context assembly. Its drift MUST be
  surfaced wherever it is delivered. Cairn MUST NOT hide a drifted memory, and MUST NOT count it as
  verified for any derived readiness.
- **FR-374**: Drift detection MUST be triggered by cheap, bounded, indexed matching against
  recorded change signals — a changed file path, a branch change, a commit change — with a
  documented per-event cap. Exceeding the cap MUST defer work, never block the agent.
- **FR-375**: The transitions between verification states MUST be a documented, total state machine,
  and every transition MUST name its trigger.

### Scope and branch lifecycle

- **FR-381**: Scope MUST be evaluated before conflict, reconciliation, verification relevance,
  context selection and promotion. Memory scoping and ranking MUST continue to use only project,
  branch, task and session (Feature 002's cross-agent invariant is unchanged: agent identity is
  never a scope).
- **FR-382**: Branch-scoped knowledge MUST NOT become project knowledge automatically when a branch
  merges. A merge MAY produce a promotion candidate, which MUST be verified against the current
  target branch and applied only on an explicit decision.
- **FR-383**: Branch deletion MUST preserve the branch's knowledge as history. Cairn's existing
  behaviour — marking memory whose scope key no longer resolves as `stale` rather than deleting it
  — MUST be preserved.
- **FR-384**: A rebase or a commit change MUST NOT invalidate branch knowledge by itself; it MUST
  mark commit-pinned evidence for rechecking.
- **FR-385**: Where a branch-scoped and a project-scoped memory disagree, both MUST remain valid
  and the applicable one MUST be selected by scope, without declaring conflict (FR-333).

### Reusable cross-project patterns

- **FR-391**: Cairn MUST represent transferable knowledge as a **reusable pattern**, a record
  distinct from a memory. A project memory MUST NOT become a global memory, and no memory scope
  crossing projects may be introduced.
- **FR-392**: A reusable pattern MUST be able to carry: the problem, its signals, its applicability
  conditions, the root cause, the known approach, its constraints and caveats, its supporting
  applications, its counterexamples and its trust state.
- **FR-393**: A reusable pattern MUST NOT carry project identity: no project name, repository
  remote, shared project identifier, absolute path, user identity or any other
  project-identifying token. Its origin MUST be recorded as an opaque local reference.
- **FR-394**: Promotion MUST be staged: candidate, sanitized, validated, and — where
  counterexamples exist — contested. Trust MUST advance only by the recorded gates.
- **FR-395**: Promotion MUST be explicit in this feature. An agent or a developer proposes it;
  Cairn MUST NOT promote autonomously. Cairn MAY compute and report promotion suggestions, and
  those suggestions MUST NOT themselves change trust.
- **FR-396**: Cairn MUST refuse promotion, naming the reason, when the source is not lifecycle
  `active`, is not verified, has no evidence, is `local_only`, belongs to a conflicted subject, is
  of a memory type whose content is inherently project-specific, fails a deterministic privacy
  check, duplicates an existing pattern, or defines signals below a documented minimum specificity.
- **FR-397**: A privacy refusal MUST name the offending class and MUST NOT echo the offending
  value. A refusal MUST leave no partial pattern.
- **FR-398**: A pattern MUST be offered to another project only where the current project's own
  recorded signals match its signals, and MUST be labelled as unverified in the receiving project.
- **FR-399**: Deleting the origin project or the source memory MUST NOT delete the pattern; the
  origin reference MUST resolve to "origin deleted".

### Independence accounting and counterexamples

- **FR-401**: A pattern application MUST record its project, its session, its outcome — resolved,
  not applicable, or failed — and whether the application was independent or followed a Cairn
  suggestion.
- **FR-402**: Trust MUST be advanced only by **distinct projects other than the origin**.
  Repetition within one project, one session or one incident MUST NOT advance trust.
- **FR-403**: An application that followed a Cairn suggestion MUST NOT advance trust unless it
  carries deterministic evidence collected in the applying project.
- **FR-404**: A not-applicable outcome MUST be recorded as a counterexample with its alternative
  cause where one is known, MUST NOT increase any success count, and MUST NOT delete the pattern.
- **FR-405**: A pattern with both successes and counterexamples MUST be offered as contested, with
  the alternative cause and what to check first stated alongside it.
- **FR-406**: Cairn MUST NOT present a reinforcement or application count as a number of
  independent verifications anywhere in its output.

### Cross-device reconciliation

- **FR-411**: Merging state from another device MUST NOT resolve a difference by timestamp
  comparison. A merge outcome MUST be identical under any relative ordering of the two devices'
  clocks.
- **FR-412**: Merging MUST preserve every proposal from every device with its original provenance,
  and MUST NOT overwrite a local memory row with a remote one.
- **FR-413**: Reconciliation decisions MUST synchronize, so that a supersession or a resolution
  decided on one device is applied on another from the decision itself rather than by row
  replacement.
- **FR-414**: Synchronization MUST reuse the existing transactional outbox, its idempotency keys
  and the server's applied-once claim. Feature 003 MUST NOT introduce a second delivery mechanism,
  a second idempotency scheme or a second transactional model.
- **FR-415**: Synchronization of Feature 003 state MUST be additive and backward compatible in both
  directions: an older peer that does not send or understand the new fields MUST continue to work.
  Where the shared server does not accept a Feature 003 field or entity, the daemon MUST degrade to
  Feature 001 synchronization semantics for the affected entities, MUST continue to deliver
  everything the server does accept, and MUST report the degradation in its synchronization status
  rather than failing the batch or retrying indefinitely.
- **FR-416**: Cairn MUST detect same-subject incompatibility introduced by a merge and expose it as
  a conflict, on every device that holds both proposals.
- **FR-417**: The design MUST hold for several sessions, several agents, several worktrees, several
  machines and several team members, including offline-then-sync.

### Continuity across compression

- **FR-421**: Cairn MUST maintain a durable, structured **continuity checkpoint** derived from
  recorded state. It MUST NOT be a summary of conversation, and MUST NOT depend on any provider's
  compression quality.
- **FR-422**: A checkpoint MUST carry at least: the current task, its goal, its acceptance criteria
  with their states, completed work, remaining work, open blockers, active decisions, critical
  constraints in force, known failures and rejected approaches, relevant test outcomes, changed
  repository state, and the next action.
- **FR-423**: A checkpoint MUST be anchored to the derived boundary record Cairn already produces
  at that boundary rather than re-deriving the same fields by a second mechanism.
- **FR-424**: A checkpoint MUST record the assumptions it was taken under: branch, commit, task
  identity, task revision, and the repository paths it considered relevant.
- **FR-425**: A checkpoint MUST be written at the pre-compaction boundary where the agent provides
  one, and at session close. It MUST be obtainable on demand through the agent tool surface and
  the command line.
- **FR-426**: Where an agent provides no post-compaction signal, Cairn MUST report continuity as
  agent-initiated rather than automatic and MUST name how to retrieve it. Where an agent provides
  no pre-compaction signal either, Cairn MUST report automatic compression-safe continuity as
  unavailable for that agent. Cairn MUST NOT claim a rehydration guarantee an adapter cannot
  provide.
- **FR-427**: Continuity delivery MUST reuse Feature 002's normalized lifecycle and capability
  model. Feature 003 MUST NOT add a canonical lifecycle event, and MUST NOT report a capability as
  present that Feature 002's rules would report as expected, conditional or absent.
- **FR-428**: After any number of compaction cycles, the fields in FR-422 that exist in recorded
  state MUST still be delivered.

### Checkpoint staleness

- **FR-431**: On restoration, Cairn MUST compare a checkpoint's recorded assumptions against
  current state and MUST classify the checkpoint as current, diverged or unresolvable.
- **FR-432**: Divergence MUST be detected for at least: a different branch, a different commit, an
  advanced task revision, and a relevant path modified by another session after the checkpoint was
  taken.
- **FR-433**: A diverged checkpoint's report MUST name the specific differences, including the
  recorded and current commit, the task revision transition, and which paths changed and which
  session changed them.
- **FR-434**: Where a checkpoint is diverged, Cairn MUST present the recorded next action as
  possibly stale rather than as the action to take, and MUST NOT instruct the agent to resume from
  an obsolete assumption.
- **FR-435**: An unresolvable checkpoint MUST still deliver the continuity fields that do not
  depend on the missing state.

### Minimum safe context and layering

- **FR-441**: Context MUST be organized in three levels: **Level 0** minimum safe continuity,
  **Level 1** relevant current knowledge, **Level 2** history and evidence on demand.
- **FR-442**: Level 0 MUST have a reserved share of the context budget that Level 1 and Level 2
  content cannot consume. The reserve is a cap on what the lower levels may not take, not an amount
  Level 0 must spend: where no Level 0 content exists, the whole budget MUST remain available to
  Level 1. Level 0 MAY spend beyond its reserve when the budget allows. The default context budget
  MUST NOT change.
- **FR-443**: Level 0 MUST contain at least: the current task and goal, acceptance criteria with
  their states, open blockers, the next action with its staleness assessment, critical drift and
  conflict warnings, pinned constraints in force, and essential repository state.
- **FR-444**: Level 2 content MUST NOT appear in the automatic briefing and MUST be reachable only
  by explicit request.
- **FR-445**: The assembler MUST retain Feature 001's guarantee that estimated tokens never exceed
  the budget, and MUST retain measure-before-emit as the mechanism. A briefing MUST be truncated to
  fit, never rejected for size.
- **FR-446**: Level 0's internal admission order MUST be documented and deterministic, so that
  behaviour under a budget too small for all of Level 0 is specified rather than incidental.
- **FR-447**: Omissions MUST continue to be reported, and Feature 001's high-priority-section
  guarantee MUST continue to hold.

### Protected invariants

- **FR-451**: Cairn MUST support pinning a memory as a **critical constraint**, so that it is not
  displaced by lexical or recency ranking.
- **FR-452**: A pin MUST record who pinned it, when and why, within a bounded reason.
- **FR-453**: A pin MUST NOT override scope. A pinned memory enters reserved context only where its
  scope applies.
- **FR-454**: The number of pins MUST be bounded per project and per scope. Exceeding the bound MUST
  refuse the pin, name the current pins, and MUST NOT silently unpin anything.
- **FR-455**: A user MUST be able to pin and unpin from the command line. An agent MUST be able to
  pin within the bound.
- **FR-456**: A pinned memory that becomes superseded MUST lose its pin; its successor MUST be
  pinned only by an explicit act. A pinned memory that drifts MUST keep its pin and carry its drift
  warning.
- **FR-457**: A `local_only` pinned memory MUST remain local. A pin on a shareable memory MAY
  synchronize; the pin MUST NOT cause any additional content to leave the machine.

### Explainability

- **FR-461**: Every item Cairn selects for context MUST have a recorded, internally consistent
  reason for its selection, drawn from a documented closed set — scope match, canonical answer,
  verification state, pin, drift warning, conflict warning, pattern signal match, checkpoint
  assumption, task binding.
- **FR-462**: Cairn MUST be able to report, for a given assembly, why each item was selected, why
  each candidate was omitted, and each item's scope, verification state and ranking basis.
- **FR-463**: Diagnostic selection detail MUST NOT be injected into the agent's normal context by
  default, and MUST NOT consume budget when not requested.
- **FR-464**: Warnings that change what the agent should do — drift, conflict, checkpoint
  divergence, task divergence — MUST be delivered as Level 0 content and are not diagnostics.

### Bounded work, performance and failure isolation

- **FR-471**: Session start MUST NOT verify memories, scan the repository, run tests, or wait on
  any Feature 003 background work. Feature 001's context deadline and its bounded fallback are
  unchanged.
- **FR-472**: Verification MUST be cached, and MUST run only in a bounded background pass or on
  explicit demand. Documented caps MUST bound the work per pass: a maximum number of evidence
  facts examined, a maximum number of verifier runs, and a maximum wall-clock share.
- **FR-473**: Where verification has not completed, `unverified` or `needs_recheck` MUST be
  reported. Cairn MUST NOT block an agent to establish truth.
- **FR-474**: Reconciliation MUST be bounded per write: a documented maximum number of subject
  members examined, with deferral rather than unbounded work.
- **FR-475**: Capture-class handling MUST keep Feature 001's 250 ms deadline and its always-exit-0,
  fail-soft rule. Feature 003 MUST NOT add work to a capture hook that can exceed it.
- **FR-476**: Any Feature 003 failure — storage busy, a verifier fault, a corrupt derived value —
  MUST degrade Cairn's own output and MUST NOT abort, delay or fail the coding agent.
- **FR-477**: Cairn MUST remain fully useful offline. Reconciliation, verification, drift, context,
  continuity and pattern suggestion MUST all work with no network and no server.
- **FR-478**: A derived value that cannot be trusted MUST be rebuilt or reported as unavailable,
  never guessed. Where a derived value and its durable inputs disagree, the durable inputs win.

### Tasks (secondary capability)

- **FR-481**: A task's acceptance criteria MUST have stable identities, so a session can update one
  criterion without rewriting the list.
- **FR-482**: A criterion MUST carry a work state — at least pending, satisfied, blocked, waived —
  distinct from a verification state — at least unverified, verified, failed.
- **FR-483**: Cairn MUST NOT equate an agent's assertion that a criterion is satisfied with
  verification of it.
- **FR-484**: A criterion MUST be able to carry evidence, using the same evidence facts as memory. A
  criterion MUST reach `verified` only on **Cairn-collected** evidence — a captured test or command
  observation with its recorded outcome, a file digest, a Git reference, or a configuration value
  Cairn read. Agent-attested evidence MAY be attached and MUST be labelled, and MUST NOT by itself
  make a criterion `verified`.
- **FR-485**: A task MUST support blockers as append-only records with an explicit cleared
  transition, each attributed to the session that opened or cleared it.
- **FR-486**: Task progress MUST be derived and reported as counts by state. Cairn MUST NOT accept
  or store an agent-authored completion percentage.
- **FR-487**: Cairn MUST derive a completion readiness value and MUST NOT change a task's status on
  the basis of it. Completing a task remains an explicit act.
- **FR-488**: A task MUST carry a monotone revision that advances on any change to the task, its
  criteria or its blockers, and every such change MUST be recorded in an append-only change log
  naming its author, the prior and the new value.
- **FR-489**: A session MUST record the task revision it bound at. Where the task has advanced,
  context refresh MUST state the transition and what changed, and MUST NOT present the session as
  having worked against the current revision all along.
- **FR-490**: Concurrent updates to different criteria or different blockers MUST both apply. A
  caller that supplies the revision it read MUST be refused when that revision has advanced.
- **FR-491**: Feature 003 MUST NOT introduce sprints, story points, epics, boards, assignees,
  estimates, dependencies between tasks, or reporting unrelated to agent continuity.
- **FR-492**: Feature 001's task fields and their existing meanings MUST continue to work
  unchanged for every existing reader, including the shared server and the web interface.

### Agent surface

- **FR-495**: The MCP tool surface MUST remain exactly the six Feature 001 tools. Feature 003 MUST
  NOT add an MCP tool.
- **FR-496**: Feature 003 capabilities MUST be reached by backward-compatible extension of the six
  tools' actions, parameters and results. An existing call with existing arguments MUST behave as
  it does today.
- **FR-497**: A caller that omits every Feature 003 parameter MUST receive Feature 001 behaviour
  plus the new read-only fields.
- **FR-498**: The Cairn usage contract MUST be extended to teach the new obligations — propose a
  topic key for durable project facts, attach evidence rather than assert importance, record a
  conflict rather than overwrite, record a pattern outcome including a negative one — and MUST
  remain within its documented size bound.
- **FR-499**: Every new capability MUST be reachable from the command line with the existing stable
  JSON envelope, covering at least: inspecting a subject and its reconciliation state; recording and
  resolving a conflict; attaching evidence; running and reading verification; pinning and unpinning;
  reading and writing a continuity checkpoint; explaining a context assembly; and listing,
  promoting, and recording outcomes for reusable patterns.
- **FR-500**: Every bound this feature relies on MUST have a documented default, MUST be asserted by
  test rather than assumed, and MUST be adjustable through the existing configuration file. The set
  MUST include at least: the reserved Level 0 share; the minimum budget below which Level 0 itself
  truncates; the pin budget per project and per scope; the number of pins, warnings and reusable
  patterns admitted to context; the maximum subject members examined per write; the maximum
  topic-keyed memories scanned for warnings; the maximum evidence lookups per captured event; the
  maximum evidence facts and verifier runs per background pass and its wall-clock share; the stored
  bounds on an evidence value and locator; and the minimum signal specificity for a reusable pattern.

### Privacy

- **FR-501**: Raw observations MUST remain local. Feature 003 MUST NOT create a path by which
  observation content reaches the server, and the server MUST continue to have no observations
  table and to reject observation-bearing payloads.
- **FR-502**: Evidence fact content — observed values, subjects, locators and digests — MUST NOT
  leave the machine in this feature. What a shared memory may carry about evidence is limited to
  the verification state, the instant of verification, the count of supporting facts, and the kinds
  of verifier involved.
- **FR-503**: Verification runs, drift records, continuity checkpoints, selection diagnostics,
  reusable patterns and pattern applications MUST be local machine state and MUST NOT be
  transmitted.
- **FR-504**: `local_only` MUST continue to mean never transmitted, for memories and for everything
  Feature 003 derives from them.
- **FR-505**: Feature 001's deleted-origin provenance semantics MUST be preserved for every new
  reference type: a deleted origin resolves to a reported deletion, never to a dangling reference
  and never to restored content.
- **FR-506**: The server's field allowlist MUST be extended only by explicitly enumerated,
  reviewed fields, and MUST continue to be enforced on the wire rather than assumed.
- **FR-507**: Cross-project promotion MUST be treated as the feature's highest privacy risk: the
  sanitization gate MUST be deterministic, MUST fail closed, and MUST be tested against a seeded
  corpus of secrets, paths and project identifiers.
- **FR-508**: Team-shared reusable patterns are out of scope for this feature. Patterns MUST NOT
  synchronize.

### Determinism, migration and compatibility

- **FR-511**: Cairn MUST NOT require an external language-model service, an embedding service, a
  vector database or a graph database for any Feature 003 correctness behaviour. Lexical retrieval
  remains the retrieval mechanism.
- **FR-512**: Model assistance MAY propose — a topic key, an equivalence, a promotion, a summary —
  and every proposal MUST pass a deterministic gate before it affects stored state. A proposal MUST
  be recorded as a proposal.
- **FR-513**: Every schema change MUST be additive. Existing migrations MUST NOT be modified, and
  existing rows MUST NOT be rewritten to satisfy a new column.
- **FR-514**: Migration from an existing store MUST be lossless and MUST require no user action.
  Existing memories MUST default to unverified with no topic key, no value key and no pins;
  existing tasks' criteria MUST become stably identified criteria in their existing order with
  pending, unverified states; existing handoffs, sessions, observations, evidence references and
  outbox rows MUST be untouched.
- **FR-515**: Cairn MUST NOT fabricate a topic key, a value key, an evidence fact or a verification
  result in order to populate a new field.
- **FR-516**: The existing schema-version guard MUST continue to prevent an older build from
  writing against a newer schema.
- **FR-517**: A derived or cached Feature 003 value MUST be rebuildable from durable records by a
  documented deterministic procedure, and that procedure MUST be exercised by test.
- **FR-518**: Corruption of a derived value MUST fail closed: report unavailable and rebuild, never
  serve a value known to be inconsistent with its inputs.
- **FR-519**: Every Feature 001 and Feature 002 behaviour that a developer depends on MUST continue
  to hold, and the existing test suites MUST pass unchanged.

### Key Entities

- **Knowledge proposal**: A memory, as in Feature 001, extended with an optional subject identity,
  an importance hint, a verification state, temporal fields, reinforcement accounting and a pin.
  Attributed to exactly one origin session. Never rewritten by reconciliation.
- **Knowledge subject**: The identity of a thing the project has knowledge about — project, scope,
  scope key and topic key. Groups proposals into a claim set. Holds no content of its own.
- **Canonical answer**: The derived current answer for a subject — one, or several when the subject
  is conflicted, or none when every member is historical. Rebuildable from proposals and decisions,
  and never written directly by a session.
- **Reconciliation decision**: An append-only record relating two proposals — reinforces,
  duplicates, supersedes, conflicts with, narrows, or is not applicable to — with its deciding
  session, time and basis.
- **Evidence fact**: A bounded, redacted, attributable record of an observed state of the world,
  with a kind, a subject, an observed value, a digest, a locator, a fingerprint and its collection
  context. Local.
- **Verification run**: An append-only record of a deterministic check of a proposal against
  evidence, with its verifier, its repository state and its result. Local.
- **Continuity checkpoint**: A structured, derived snapshot of work state at a boundary, anchored to
  the boundary record Cairn already produces, plus the assumptions it was taken under. Local.
- **Reusable pattern**: Project-independent, sanitized, applicability-bounded transferable
  knowledge, with a trust state derived from independent applications and counterexamples. Local,
  never synchronized.
- **Pattern application**: A record that a pattern was applied in a project, with its outcome, its
  independence and any alternative cause discovered. Local.
- **Task criterion**: A stably identified acceptance criterion belonging to a task, with a work
  state, a verification state and optional evidence.
- **Task blocker**: An append-only record of something preventing progress, with an explicit
  cleared transition and attribution on both ends.
- **Task change**: An append-only entry in a task's revision history, naming its author, the kind
  of change, and the prior and new value.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-301**: On a curated deterministic corpus of equivalent-knowledge cases, reconciliation
  produces exactly one canonical answer per subject in 100% of cases and produces zero false
  merges across a paired corpus of deliberately distinct cases.
- **SC-302**: For every real-conflict case in the corpus, both proposals remain active, the subject
  reports conflicted, and no case produces a single canonical answer. Zero silent winners.
- **SC-303**: Under 32 concurrent proposals against one subject from separate processes, the number
  of persisted proposals equals 32 and the number of lost writes is 0.
- **SC-304**: Reversing the relative clock order of two offline devices produces a byte-identical
  merged canonical state and an identical conflict set.
- **SC-305**: For every supersession in the corpus, the superseded proposal's content, provenance
  and evidence references are byte-identical before and after, and a historical query at an instant
  before the supersession returns the historical answer.
- **SC-306**: The verification state machine is exercised exhaustively: every documented transition
  is reachable by a test and every undocumented transition is unreachable.
- **SC-307**: For every drift case, the observed sequence is verified → needs_recheck → drifted, no
  memory content changes at any step, and no memory is created without an explicit act.
- **SC-308**: Across budgets from the documented minimum to four times the default, estimated
  tokens never exceed the budget, in 100% of assemblies, including with 5,000 memories present.
- **SC-309**: At the documented minimum budget with 5,000 memories present, the relevant pinned
  constraint, the active drift warning, the conflict warning, the current task with criteria states
  and the next action are all present in 100% of assemblies.
- **SC-310**: Across ten consecutive compaction cycles, every continuity field that exists in
  recorded state is present after every cycle, in 100% of cycles.
- **SC-311**: Every divergence class — branch, commit, task revision, concurrently modified
  relevant path — is detected in 100% of seeded cases, and a diverged checkpoint never presents its
  recorded next action as the action to take.
- **SC-312**: A pattern promoted from one project and offered in another is labelled unverified in
  the receiving project in 100% of suggestions.
- **SC-313**: Recording a not-applicable outcome increases no success count in 100% of cases, and
  the pattern is retained in 100% of cases.
- **SC-314**: Ten same-project applications of one pattern yield a distinct-project validation
  count of 1.
- **SC-315**: Against a seeded corpus of secrets, absolute paths, project names, repository remotes
  and shared identifiers, promotion refuses 100% of violating candidates, no refusal echoes the
  offending value, and no partial pattern is created.
- **SC-316**: No Feature 003 payload accepted by the server contains an evidence value, an evidence
  locator, a verification run, a checkpoint, a pattern, a selection diagnostic or any observation
  field. Enforced on the wire and asserted by test.
- **SC-317**: Two sessions updating different criteria on one task both persist their change in
  100% of cases, and no criterion is reset by another session's write.
- **SC-318**: A session bound at an earlier revision is told the transition and the specific changes
  in 100% of cases where the task advanced.
- **SC-319**: Session-open latency and capture-hook latency are within the Feature 001 budgets on
  the same measurement, with Feature 003 enabled and a project carrying 5,000 memories, 10,000
  evidence facts and 500 subjects.
- **SC-320**: Every documented bound in FR-500 is asserted by a test, a bounded verification pass
  never exceeds its caps, and no verification work occurs on the session-open path.
- **SC-321**: The dependency manifests contain no language-model client, embedding library, vector
  database or graph database, and no retrieval path reaches the network. Asserted by test.
- **SC-322**: Migration of a store created by v0.1.0-alpha.4 containing memories, superseded
  memories, evidence references, sessions, handoffs, tasks, pending outbox rows and shared server
  data completes with zero rows lost, zero rows rewritten, and every new field at its documented
  default.
- **SC-323**: The Feature 001 and Feature 002 test suites pass unchanged, the MCP surface is still
  exactly six tools, and the outbox still has no observation entity type.
- **SC-324**: Every derived value is rebuilt from durable records in a test, and the rebuilt value
  equals the incrementally maintained value in 100% of cases.
- **SC-325**: No release gate depends on a language-model judgement. Any semantic-similarity
  evaluation runs outside the release gates with its trust boundary documented.
- **SC-326**: Against a server that accepts no Feature 003 field, synchronization still delivers
  100% of the Feature 001 payload, loses nothing from the outbox, and reports the degradation by
  name in synchronization status.
- **SC-327**: No memory scope, partition, ownership domain or retrieval filter is added beyond
  project, branch, task and session, and no importance, pin or verification value can change scope
  precedence. Asserted by the existing scope audit extended to the new fields.
- **SC-328**: A task criterion never reaches `verified` on agent-attested evidence alone, in 100% of
  seeded cases.

## Assumptions

- **Agents will attach topic keys where it matters, because the contract asks them to.** Cairn
  cannot deterministically decide that two differently-worded sentences concern one subject, and it
  will not pretend to. Feature 003's reconciliation is strongest on memories that carry a subject
  identity, and the usage contract, the Skill and the tool descriptions all ask for one on durable
  project facts. Free-form memories remain correct, they simply do not reconcile.
- **Deterministic verification covers the cases that matter most.** File existence, digests, Git
  refs, configuration values read from the repository, recorded test outcomes and recorded command
  outcomes cover the great majority of what a project memory asserts. Claims that cannot be checked
  deterministically stay `unverified`, which is an honest state and not a defect.
- **Cross-project reuse is single-user in this feature.** Patterns stay on the machine. That removes
  the hardest privacy question — what may leave one team's project and enter another's — from this
  feature's scope, and it still delivers the story a developer actually has: the same person's next
  project.
- **Evidence content staying local is a feature, not a limitation.** A teammate learns that a memory
  was verified, when, and against what kind of check. If they need the value they look where it was
  verified. This is a strict extension of the rule Feature 001 already enforces for observations.
- **Task work state legitimately has a current value.** The invariant Feature 003 protects is that
  no canonical project knowledge is silently overwritten and no criterion's change is lost. A
  criterion's state is work state; its history is preserved in an append-only change log, and a
  caller that reads before writing is protected by revision comparison.
- **The existing 15-minute maintenance tick is the right home for bounded background work.** It
  already reaps idle sessions, sweeps owed handoffs and marks stale scopes. Verification and drift
  passes join it rather than introducing a scheduler.
- **Reconciliation cost is bounded because subjects are small.** A subject in a real project has a
  handful of members, not thousands. The design caps the work per write and defers rather than
  scanning, so the assumption failing degrades throughput rather than correctness.
- **The shared server may be older than the daemon.** Alpha deployments will run mixed versions.
  Feature 003's synchronization is additive in both directions and reports degradation rather than
  failing.

## Out of Scope

Feature 003 is not, and MUST NOT become:

- **Repository code intelligence.** No Tree-sitter or other source parsing, no LSP integration, no
  symbol graph, no dependency graph, no blast-radius analysis, no semantic source-code retrieval.
- **Embeddings or vector search.** No embedding service, no vector database, no similarity index.
  Lexical retrieval remains the mechanism.
- **A graph database.** Relations are rows with a documented derivation, not a graph engine.
- **Autonomous whole-repository reasoning.** No continuous scanner, no background language-model
  analysis of the project.
- **Autonomous command or test execution.** Cairn reads files inside the worktree and Git. It does
  not run builds, tests or arbitrary commands to establish truth.
- **Autonomous mutation of knowledge.** No memory is rewritten, superseded or resolved because a
  single evidence source changed.
- **Autonomous task completion.** Readiness is derived; completion is an explicit act.
- **Autonomous cross-project promotion.** Promotion is proposed and gated. Suggestions do not
  change trust.
- **A project-management suite.** No sprints, story points, epics, boards, assignees, estimates,
  inter-task dependencies, burndown or velocity.
- **Team-shared reusable patterns.** Patterns are local to the machine in this feature.
- **A replacement for Git, for continuous integration, or for a task tracker.** Cairn reads Git,
  records what CI told a session, and tracks only the work state an agent needs to continue.
- **A seventh MCP tool.** The surface stays at six.

Each of these may be a separate future feature. None of them is this one.
