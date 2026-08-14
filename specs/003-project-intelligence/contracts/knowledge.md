# Contract: Canonical Knowledge

**Feature**: `003-project-intelligence`

How a session's proposal becomes the project's answer, and how it fails to when it should.

```text
proposal (memory row, attributed, never rewritten)
    │
    ├── topic_key? ──no──▶ free-form: searchable, briefable, never auto-reconciled
    │
   yes
    │
    ▼
subject = (project, scope, scope_key, topic_key)
    │
    ▼
derive_subject(members, relations)  ── pure, no clock, no id arbitration
    │
    ├─▶ Historical    no canonical answer
    ├─▶ Settled       one answer
    ├─▶ Reinforced    one answer + duplication accounting (content identical)
    ├─▶ Corroborated  several answers agreeing on the value, differing in content
    └─▶ Conflicted    every competing answer, no winner, a warning
```

There is **no canonical row**. That is the contract's foundation: nothing to overwrite means no
silent last-write-wins is expressible (FR-336).

## Normalization

Total functions. Anything unrepresentable yields `None`; nothing is rejected (FR-312).

### `topic_key`

```text
1. Unicode NFC, then lower-case
2. split on '.'
3. each segment: keep [a-z0-9_], map '-' and ' ' to '_', collapse repeats, trim '_'
4. drop empty segments
5. reject (→ None) if: 0 segments, > 6 segments, or total length > 128
```

| Input | Normalized |
|---|---|
| `Infrastructure.Production_Database` | `infrastructure.production_database` |
| `infra/prod-db` | `infra_prod_db` (a `/` is not a separator; it becomes `_`) |
| `a.b.c.d.e.f.g` | `None` — 7 segments |
| `"; DROP TABLE memories;--` | `drop_table_memories` |
| `데이터베이스` | `None` — no representable characters |

`/` deliberately does not separate: a topic key is not a path, and accepting path syntax would invite
absolute paths into a shared column.

### `value_key`

```text
1. NFC, lower-case
2. collapse whitespace runs to one space, trim
3. reject (→ None) if empty or > 64 characters
```

Accepted only alongside a `topic_key` (schema constraint).

### `content_norm_digest`

```text
SHA-256( NFC → lower-case → collapse whitespace → strip trailing .,;:!? )
```

Used only for exact duplicate detection (D46). It is not a similarity measure and must never be
compared for partial equality.

## Automatic reconciliation

**Exactly one merging case**, and it is the only one Cairn can decide without inference: content that
is identical after normalization. Everything else is surfaced and never merged.

| Condition | Outcome | Relation recorded | `basis` |
|---|---|---|---|
| same subject, equal `content_norm_digest` | duplicate | `duplicates` new→existing | `deterministic_rule` |
| same subject, equal `value_key`, **differing** content | **corroboration** — derived, both retained | **none** | — |
| same subject, differing `value_key`, **same scope and scope key** | conflict | `conflicts_with` | `deterministic_rule` |
| same topic, **different scope precedence rank** | scope exception | none automatically | — |
| same topic, same scope, **different scope key** | unrelated | none | — |
| either side has no `topic_key` | nothing beyond exact-content duplication | none | — |

### Why equal value keys do not merge

A value key states a **value**. It does not establish that two differently-worded statements are the
same **proposition**, because the content may carry material claims the key does not capture:

```text
topic_key auth.strategy   value_key jwt   "JWT uses HS256 with a shared secret."
topic_key auth.strategy   value_key jwt   "JWT uses RS256 with rotating public keys."
```

Both agents wrote an honest value key. Merging them would suppress one of two materially different
claims from the canonical answer and would report a reinforcement that never happened. Deciding
whether they are one claim requires reading the content — inference, which FR-511 forbids in the
correctness path and FR-317 forbids outright.

So Cairn does what it can decide: it observes that they **agree on the value**, derives
`Corroborated`, keeps both, and tells the writer which member it matched. The party that can read both
statements — the agent — then decides explicitly, and the decision is recorded with
`basis = explicit_agent` like every other agent decision in this design.

This is the proposal boundary applied one level deeper. Cairn detects the candidate; it does not
resolve it.

The existing memory a duplicate points at is the highest-precedence active member of the subject; when
several are equally applicable, the one with the lowest identifier, for stability.

**Never automatic**: `supersedes` (FR-325), `reinforces`, `narrows`, `not_applicable_to`, and any
resolution of a `conflicts_with`.

`reinforces` is now an **explicit-only** relation, recorded by `cairn_remember action=reinforce` and
meaning *a session confirmed this memory is still true*. That is a real, useful act with a real author.
It is no longer something Cairn infers from a matching key.

Bounded: at most `reconcile_members_max` (64) subject members are examined per write. Beyond that the
write completes, the relation is deferred to the maintenance tick, and the response reports
`reconciliation_deferred` (FR-474).

## Scope overlap

Two memories are **simultaneously applicable** — the precondition for conflict — when a single
working context would select both.

| A | B | Overlap | Consequence |
|---|---|---|---|
| `project:P` | `project:P` | yes | conflict possible |
| `branch:main` | `branch:main` | yes | conflict possible |
| `task:T1` | `task:T1` | yes | conflict possible |
| `session:S1` | `session:S1` | yes | conflict possible |
| `project:P` | `task:T1` | **no** | scope exception — task wins in T1, project elsewhere |
| `project:P` | `branch:main` | **no** | scope exception — branch wins on main |
| `branch:main` | `branch:feature/x` | **no** | never simultaneously applicable |
| `task:T1` | `task:T2` | **no** | never simultaneously applicable |

Precedence is Feature 001's `MemoryScope::bucket` unchanged: task 0, branch 1, project 2, session 3.

This is why scenario B (project PostgreSQL, task SQLite fixture) is not a conflict, and why main-REST
against feature/graphql-GraphQL is not either — both fall out of the scope key, not out of a
heuristic (D48).

## `derive_subject`

```rust
pub fn derive_subject(
    members:   &[MemoryFacts],   // active, same (scope, scope_key, topic_key)
    relations: &[Relation],      // every relation touching those members
) -> SubjectView;
```

**Inputs read**: `id`, `state`, `value_key`, `verification`, `evidence_fact_count`, `pinned`,
`importance`, and relations.

**Inputs deliberately not read**: `created_at`, `updated_at`, `effective_from`, `decided_at`, and the
UUID's embedded timestamp — for arbitration. Identifier order sorts `answers` for stable output and
nothing else (D49, FR-303).

**Algorithm**

```text
1. drop members that any `supersedes` or `duplicates` relation points *to*
2. if none remain                              → Historical,   answers = []
3. partition the remainder by value_key
   (members with no value_key form singleton partitions; they never merge)
4. if one partition
     if it has one member                      → Settled,      answers = [it]
     else if every member shares one
          content_norm_digest                  → Reinforced,   answers = [representative]
     else                                      → Corroborated, answers = every distinct
                                                  content, sorted by rank then id
5. if several partitions
     if a `supersedes` relation resolves them  → recurse from 1 with it applied
     else                                      → Conflicted,   answers = one per partition,
                                                  sorted by id, no winner
6. narrowed_by = members of the same topic at a narrower scope, from the caller's
   applicable set
```

Step 4 is where the false-merge path was closed. A partition whose members share a value key but not
their content yields `Corroborated` — several answers that agree on the value — rather than one
representative silently standing for all of them.

The **representative** of a reinforced partition is the member with the most supporting evidence
facts, then the highest verification rank, then the lowest identifier. Verification rank is
`verified(cairn)` > `verified(attested)` > `needs_recheck` > `unverified` > `drifted` > `conflicted`:
a deterministic check outranks an attestation, which is what stops an attested claim becoming the
face of a subject over a checked one. Every tiebreak is a property of the record, never of time.

`SubjectView.decisions` lists the relations that produced the outcome, so `cairn memory subject`
can answer "why" (FR-307).

## Conflict resolution

A conflicted subject leaves `Conflicted` only on a recorded decision (FR-335):

| Resolution | Recorded | Effect |
|---|---|---|
| Explicit supersession | `supersedes` with `basis = explicit_agent \| explicit_user` | the superseded member drops out; the subject re-derives |
| Scope narrowing | `narrows` plus a new memory at the narrower scope | the members are no longer simultaneously applicable |
| Evidence | `supersedes` with `basis = evidence` and `basis_evidence_id` | a verification result established the replacement |

Cairn never records any of these on its own. A conflict may stand indefinitely; standing is a
reported state, not an error (FR-334).

## Temporal queries

| Question | Predicate |
|---|---|
| Best-supported current knowledge | `state = 'active'` + `derive_subject` |
| What was effective at `T` | `effective_from <= T AND (superseded_at IS NULL OR superseded_at > T)` |
| What a session ran against | `T` = that session's `started_at` |

`as_of` returns the historical set with `as_of: T` echoed in the response, so a caller cannot
mistake a historical answer for a current one (FR-342). No record is modified by a historical query
(FR-343).

### What a historical answer does and does not claim

The predicate reconstructs **proposal effectiveness and explicit supersession intervals**. That is
what Cairn stores authoritatively, and it is the whole of the claim (FR-342).

| Lifecycle transition | Authoritative instant? | In a historical answer |
|---|---|---|
| created / effective | yes — `effective_from` | Bounds the interval exactly |
| superseded | yes — `superseded_at`, set with the relation | Bounds the interval exactly |
| became `stale` (scope key stopped resolving) | **going forward, yes** — `stale_at`, set by the maintenance tick. NULL for anything that went stale before this feature | Where known, reported as the instant applicability ended. Where NULL, the memory is returned as *effective* with `applicability: unknown` |
| deleted | yes — `deleted_at`, but the content is cleared by the tombstone | Absent, reported as deleted. Content is not reconstructable, which is what deletion means |

So a historical answer may say: *this proposal was effective at T; whether its scope still resolved at
T is unknown.* That is weaker than "it applied at T", and it is the honest limit of the stored
evidence. Cairn never presents an unbounded interval as fact (D82).

There is no valid-time table, no retroactive interval correction and no branching history (FR-345).

## Warnings surfaced to context

Computed for the **applicable** scopes only (project + current branch + bound task + current
session), bounded by `subject_warning_scan_max` (256) with the highest-precedence scopes examined
first. Beyond the bound, assembly reports `degraded: true` — Feature 001's existing flag — and names
`subject_scan` in the omissions.

| Warning | Condition | Level |
|---|---|---|
| `conflict` | an applicable subject is `Conflicted` | 0 |
| `drift` | an applicable memory has `verification = 'drifted'` | 0 |
| `corroboration` | an applicable subject is `Corroborated` — rendered inline as `+N further statements`, not as a warning row | 1 |
| `remote_verification` | an applicable memory's verification authority is `remote_cairn` or `remote_attested` and it is being relied on | 1 |
| `attested_verification` | an applicable memory is `verified` with authority `attested` | 1 |

`Corroborated` deliberately does **not** produce a Level 0 warning. Several sessions agreeing on a
value is normal and healthy; the honest signal is the inline count and the retrieval path, which costs
a few tokens rather than a warning slot. A warning here would train people to ignore warnings.

At most `warnings_in_context_max` (5) reach an agent, ordered: checkpoint divergence, task
divergence, conflict, drift.

## Error codes

Added to the existing stable set (`cairn-core/src/wire.rs::codes`):

| Code | Meaning |
|---|---|
| `invalid_topic_key` | The proposed key did not normalize; the memory was stored free-form |
| `value_without_topic` | A `value_key` was supplied with no `topic_key` |
| `subject_not_found` | No subject matches the requested key in the requested scope |
| `not_conflicted` | A resolution was requested for a subject that is not conflicted |
| `relation_conflict` | The requested relation contradicts an existing one (e.g. mutual supersession) |
| `reconciliation_deferred` | The write succeeded; the relation exceeded `reconcile_members_max` |
| `corroborating_member` | The write succeeded; it agrees on the value with a named existing member and differs in content. Not a failure — the prompt for an explicit decision |

`invalid_topic_key` and `reconciliation_deferred` are **not failures** — the envelope is `ok: true`
with the code in a `notes` array, because FR-312 requires the memory to be stored regardless.

## Symmetric relation normalization

`conflicts_with` is the one relation kind whose meaning is symmetric: *A and B disagree* is the same
fact as *B and A disagree*. Stored under a primary key of `(from, to, kind)`, two machines that detect
it independently while offline would produce `A→B` and `B→A` — two durable rows for one fact, both
syncing, both appearing in `SubjectView.decisions`, and the same conflict reported twice.

Endpoints of a symmetric kind are therefore normalized before the write:

```text
from = min(id_a, id_b)      lexicographic on the identifier
to   = max(id_a, id_b)
```

The primary key then absorbs the second machine's record exactly as it absorbs a local duplicate, and
the invariant holds under any order of arrival and any relative clock skew (FR-305, SC-324).

Applied to `conflicts_with` **only**. Every other kind is directional and normalizing it would destroy
its meaning:

| Kind | Symmetric? | Why |
|---|---|---|
| `conflicts_with` | **yes** | Disagreement has no direction |
| `supersedes` | no | Which one replaces which is the entire content of the relation |
| `duplicates` | no | Points from the newer proposal at the member it duplicates |
| `reinforces` | no | Points from the confirming session's memory at the one confirmed |
| `narrows` | no | The narrower points at the broader |
| `not_applicable_to` | no | "A does not apply in B's context" does not imply the reverse |
