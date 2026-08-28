# Implementation Plan: Cairn Collaborative Global Memory

**Branch**: `004-collaborative-global-memory` | **Date**: 2026-08-21 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/004-collaborative-global-memory/spec.md`

**Base**: `main` @ `96178fc` (v0.1.0-alpha.5, Feature 003 merged)

**Constitution**: v1.1.0 (amended by this feature — see [Governance action](#governance-action))

## Summary

Cairn today holds knowledge for one project at a time, on one machine, for whoever happens
to hold a token. Feature 004 adds two knowledge domains that follow the *person* rather
than the project — **Personal Global Memory**, private to one user and synchronized across
that user's devices, and **Team Global Memory**, proposed by any member and made
authoritative only by an administrator — and puts a real identity model underneath them.

The design turns on one decision. Cairn does **not** gain a `MemoryScope::Global`. Scope
answers "how narrow inside a project"; a new orthogonal **domain** answers "whose knowledge
is this". Personal and team knowledge live in their own tables with no `project_id` column
at all, following the precedent Feature 003 already set with `reusable_patterns` — *"a
pattern that cannot name a project cannot leak one"*
(`crates/cairn-store/migrations/0005_project_intelligence.sql:236-238`). That single choice
is what keeps the existing `memories` table, its four-variant `CHECK`, its exhaustive
`resolve_scope` match and all of Feature 003's reconciliation semantics untouched.

Everything else is reuse. Multi-device concurrency needs no clocks because Feature 003
already refuses them: records are immutable after creation, divergence is expressed as
relations, and the canonical answer is *derived* at read time by a function that cannot see
a timestamp (`crates/cairn-core/src/knowledge.rs:10-17`, `:437-454`). Compatibility with an
older server needs no handshake because the existing one-way capability advertisement plus
the `blocked` outbox state already handles it. Recall stays bounded because global sections
are never fetched during Level 0 reserve computation and are capped inside the leftover
Level 1 remainder — and, after the audit round, because reserve that Level 0 *releases*
unspent stays out of their reach too (D449): the allowance is
`min(floor(total_budget * 0.15), remaining_non_reserve)`, so a project with little critical
state does not thereby hand its briefing to project-independent guidance.

One thing is not reuse, and it is worth naming next to the reuse claims above: the privacy
boundary is enforced in **five** places, not four, and the fifth is on the server (D447).
Every earlier draft validated global content client-side only, which makes the guarantee
conditional on client cooperation — a convention, not a boundary. Server-side ingest screens
against the union of the pushing user's project memberships, which is strictly stronger than
any single project's identity and catches the one case a client structurally cannot: content
naming project X, written while working in project Y.

Three things in the plan are repairs of shipped behavior rather than new capability, and the
spec says so plainly: the wire privacy check is documented as an allowlist but implemented
as a non-recursive top-level denylist while `handoff_payload` transmits absolute local
paths; `tests/tests/scope_audit.rs:376-398` passes vacuously and protects nothing; and
background pull never runs for a namespace with an empty outbox, which would mean team
knowledge never arrives on a machine that only consumes it.

## Technical Context

**Language/Version**: Rust 1.97.1 (pinned, `rust-toolchain.toml`); TypeScript 5.7 / Next 15
/ React 19 for the web surface (read-only in this feature — admin UI is deferred to 005)

**Primary Dependencies**: axum 0.8 (server), sqlx 0.8 with the `sqlite` and `postgres`
features, tokio 1, clap 4, uuid 1 (v7), sha2 0.10, serde 1. **No new dependency is
introduced by this feature.**

**Storage**: embedded SQLite with FTS5 locally (`crates/cairn-store`, migrations 0001-0006,
this feature adds 0007); PostgreSQL behind one server (`crates/cairn-server`, migrations
0001-0002, this feature adds 0003). No second datastore, no broker, no cache tier, no
vector store.

**Testing**: `cargo test` per crate plus the workspace e2e harness in `tests/tests/*.rs`
(30 existing suites), which spawns prebuilt binaries. Note that `cargo build --tests` does
not rebuild the binaries the harness spawns — a full `cargo build` is required first or the
harness silently exercises stale binaries.

**Target Platform**: macOS, Linux and Windows (named-pipe transport already supported);
server on Linux behind Docker

**Project Type**: Rust workspace — CLI + MCP server (`cairn`), daemon (`cairnd`), HTTP
server (`cairn-server`), shared crates, plus a Next.js web app

**Performance Goals**: context assembly stays within the existing budget invariant
(`estimated_tokens <= budget`, default 3000, minimum 600, verified by SC-419); the combined
personal and team allowance is `min(floor(total_budget * 0.15), remaining_non_reserve)`;
unified recall across three domains must not add a second round of full-table scans — each
domain is a bounded FTS5 query with `MAX_LIMIT = 50`

**Constraints**: fully useful offline, including reading cached personal and team knowledge
and queueing promotions; agent-facing operations fail soft; six MCP tools and no seventh;
additive migrations except where SQLite forbids it; an older server must not break project
sync, and an older *client* must not lose project sync against a newer server either
(FR-586); no global content is written anywhere without passing one shared validator, at all
five entry points including server-side ingest

**Scale/Scope**: one server is one team — tens of users, low hundreds of projects, a handful
of devices per user. Not a multi-tenant SaaS; there are no organizations.

## Constitution Check

*GATE: evaluated against constitution **v1.1.0** before Phase 0, and re-evaluated after
Phase 1 design. Both passes are recorded.*

### Governance action

This feature required an amendment, and the amendment was made **before** the design was
accepted rather than retrofitted to excuse it. Constitution v1.0.0 Principle IV stated
*"Memory is never global or ambient"*, and Feature 003 FR-391 stated *"A project memory MUST
NOT become a global memory, and no memory scope crossing projects may be introduced."* As
literally worded, both forbid this feature.

The amendment to **v1.1.0** separates the two things that sentence was conflating. The
prohibition worth keeping is on *ambient* memory — knowledge recalled without being able to
say what it belongs to or why it applies — and Principle IV now states that more strongly,
requiring every record to name a domain. The prohibition on *scope* crossing projects is
retained verbatim as binding. FR-391's second clause therefore still holds and this design
obeys it; its first clause is understood as forbidding *silent* promotion, which this design
also forbids. Principle VIII was added to state the non-displacement guarantee outright, and
Principles V and VII were tightened. Full rationale is in
[`research.md`](./research.md) and the amendment history is in
`.specify/memory/constitution.md`.

### Pre-design gate

| Principle | Verdict | Basis |
|---|---|---|
| I. Usable MVP First | PASS | Six user stories, each a vertical slice through agent → daemon → storage → surface. P1 alone (administered accounts, self-registration retired) is independently installable and useful. |
| II. Simple Architecture | PASS | No new dependency, datastore, broker, cache tier or service. Two new logical entities reusing the existing store, outbox, sync loop and FTS5. |
| III. Local-First Reliability | PASS | Cached personal and team knowledge is readable offline; creation and promotion are local writes that enqueue; server absence degrades sharing only. |
| IV. Explicitly Domained Memory | PASS | Domain is explicit on every record; `MemoryScope` is untouched at four variants; no scope crosses projects. |
| V. Privacy by Default | PASS | Promotion is explicit and passes a fail-closed gate; the global tables have no column for a path, an observation or evidence. Also repairs a shipped leak. |
| VI. Deterministic Data Boundaries | PASS | Project traits derived from the working tree, not guessed; applicability is set membership over a closed vocabulary; replay converges. |
| VII. Testable Behavior | PASS | Every requirement is user-observable; bounded outputs asserted; repairs a test that could not fail. |
| VIII. Project Truth Is Not Displaceable | PASS | Global is not fetched during reserve computation; capped in the Level 1 remainder; domains stay separate arrays. |

### Post-design gate

Re-evaluated after `data-model.md`, the six contracts, `migration.md` and
`compatibility.md`. All eight principles still pass. Four items moved into
[Complexity Tracking](#complexity-tracking) because the design cannot honor them without a
recorded justification: the `outbox` rebuild against 003's additive-only policy, the two
separate tables where one would do, the per-namespace refactor of a shipped sync loop, and
the change to a shipped idempotency-key input.

One principle got *stronger* after design rather than weaker, and it is worth recording:
Principle VIII is enforced structurally, not arithmetically. The reserve computation does
not call the global fetch at all, so there is no budget arithmetic that could admit global
knowledge into Level 0 even if a future maintainer changed a constant.

### Post-analysis gate (third pass)

A `/speckit-analyze` pass over the completed artifacts found twelve inconsistencies. Eleven
were repairs within the design. One was a **false guarantee**, and it is recorded here rather
than quietly fixed, because Principle V is the principle it strained.

The original FR-517 claimed a personal or team record has "no field capable of holding" a
file path or a command. That was not true. Those records carry free-text `content`, and free
text can hold anything. Worse, the check that would have caught it lived only in the
*promotion* gate, while two other paths — direct personal creation and team proposal — could
create global content without ever calling it. The guarantee was both overstated and
bypassable.

The repair (D433) splits the guarantee into what is structurally impossible because no column
exists, and what is prevented by validating free text, and moves the free-text validation
into one shared pure function. Principle V now passes on an accurate claim instead of a
flattering one. FR-550 forbids the old phrasing from returning: documentation must say which
of the two mechanisms each guarantee rests on, and may never again describe a free-text field
as structurally incapable of carrying a path.

That repair said "all four entry points". The fourth-pass gate below records why that count
was also wrong.

Constitution v1.1.0 needs no further amendment. Principle V already demanded that structural
prevention be *preferred* to procedural rules, not that it be achievable everywhere; the
error was in the feature's claim, not in the principle.

### Post-audit gate (fourth pass)

An independent audit of the repaired artifacts found three CRITICAL and eight HIGH findings.
Three of them strained a principle, and all three are recorded here rather than fixed quietly.

**Principle V (privacy by construction), again.** The third-pass repair moved the free-text
validation into one shared function and declared the bypass closed. It was not. The validator's
class list held seven classes, and neither the project-name screen nor the shell-command screen
was among them — both lived only in the *promotion* gate, which is one of the paths, not all of
them. Direct personal creation and team proposal, the two paths an agent reaches for first
because they need no project memory to exist, had no project-name screen at all. The repair
(D446) moves both checks into the validator, brings its class list to nine, gives it the
project identities to screen against, and extends it to applicability *values*, which were
treated as safe because their *kind* comes from a closed vocabulary while their value was an
unchecked open string. FR-579 forbids any second implementation of a class.

**Principle V, a second time: the client was trusted.** All four entry points were client-side.
A modified, old, or buggy client wrote unvalidated content straight into the server store, from
which it reached every other device the user owns. A privacy guarantee that holds only when the
client cooperates is a convention, not a guarantee. The repair (D447) makes server-side
synchronization ingest the **fifth** mandatory entry point, screening against the union of the
pushing user's project memberships — which is strictly stronger than any single project, and
catches the one case a client structurally cannot: content naming project X written while
working in project Y.

**Principle VIII (project truth is not displaceable).** Feature 003 releases an unspent Level 0
reserve into the general pool, and nothing said whether global sections could spend it. If they
could, a project with little critical state would hand a large share of its briefing to
project-independent guidance — Principle VIII's failure mode wearing a budget's clothes rather
than a scope's. The repair (D449, D450) computes the global allowance from the non-reserve pool
only, and pins the previously unnamed fraction at `0.15` so the ceiling is testable.

**Principle VII (a test that cannot fail protects nothing).** The audit's most useful finding
was not any single defect but why the third pass certified a design that still contained them.
That pass verified **citation coverage**: every requirement named a task, every task named a
requirement. Citation coverage cannot see that a validator's class list omits the very check
the requirement depends on. Three artifacts agreed with each other and all three were wrong
together. The fourth pass therefore verifies, for every requirement, a named mechanism, every
call site it must reach, and a test that fails if the mechanism is removed — and four
previously certified items (D456) were re-derived rather than re-asserted for exactly this
reason.

Running that pass properly found seventeen requirements with **no success criterion at all**
(D457), which is why `spec.md` grew from 52 criteria to 69 during the repair rather than
shrinking. Six were sentences this audit round had itself just written — a repair round is
where a requirement is least likely to have a criterion yet. The other eleven had survived
three passes, and the worst of them is `FR-521`: *"do not add `MemoryScope::Global`"* is the
constraint this feature's Summary calls "the one decision" everything turns on, it had a task,
and it had nothing asserting it. A task is not a criterion — a task can be reworded, deferred,
or marked done against a different assertion than the one intended. Eight further requirements
are recorded in D457 as deliberately carrying no dedicated criterion, so that "every
requirement has one" is not claimed where it is not true.

Constitution v1.1.0 still needs no further amendment. Principle V asks that structural
prevention be preferred where it is achievable and that the mechanism be named honestly where
it is not; both failures above were failures to apply it, not gaps in it.


## Project Structure

### Documentation (this feature)

```text
specs/004-collaborative-global-memory/
├── spec.md                      # Phase -1: what and why — 160 FR, 70 SC
├── plan.md                      # This file
├── research.md                  # Phase 0: D401-D458, D-U1-U4, 8 investigations,
│                                #   four repair rounds with forward pointers
├── security-prerequisite.md     # The hardening patch that must land on main FIRST
├── data-model.md                # Phase 1: tables, types, what memories does NOT change,
│                                #   the Layer A / Layer B split, the never-transmitted set
├── migration.md                 # Phase 1: local 0007, server 0003, proof test
├── compatibility.md             # Phase 1: both directions — new client/old server AND
│                                #   old client/new server; capability vs ingest refusal
├── quickstart.md                # Phase 1: runnable walkthrough of all six stories
├── traceability.md              # Phase 1: FR/SC -> mechanism -> call sites -> task -> test
├── contracts/
│   ├── identity-administration.md
│   ├── project-authorization.md
│   ├── global-memory.md
│   ├── recall-composition.md
│   ├── sync-namespaces.md
│   └── promotion-privacy.md     # the nine validator classes, the eight-check gate,
│                                #   five entry points
├── checklists/
│   └── requirements.md
└── tasks.md                     # Phase 2 (/speckit-tasks output)
```

### Source Code (repository root)

```text
crates/
├── cairn-core/src/
│   ├── domain.rs                # + KnowledgeDomain, ServerRole, UserStatus,
│   │                            #   ApplicabilityKind; MemoryScope UNCHANGED
│   ├── context.rs               # Level 0/1/2 model + section priority lives HERE
│   ├── budget.rs                # CHARS_PER_TOKEN, budget arithmetic
│   ├── wire.rs                  # Request::Context gains `depth` (never existed)
│   ├── knowledge.rs             # classify_proposal / derive_subject reused per domain
│   ├── global.rs                # NEW: PersonalKnowledge, TeamKnowledge, TeamState
│   ├── validate.rs              # NEW: validate_global_content — the ONLY implementation
│   │                            #   of the nine rejection classes, called by all five
│   │                            #   entry points including server-side ingest (D446/D447)
│   ├── promotion.rs             # NEW: the pure eight-check gate + PromotionRejection;
│   │                            #   delegates content and project-name screening to
│   │                            #   validate.rs rather than re-implementing it
│   ├── applicability.rs         # NEW: ApplicabilityFact, the match predicate; values
│   │                            #   are validated through validate.rs, not just kinds
│   ├── handoff.rs               # REPAIR: repository-relative paths only
│   └── paths.rs                 # salted origin digest, reused for origin_digest
├── cairn-store/
│   ├── migrations/0007_collaborative_global_memory.sql   # NEW (rebuilds outbox)
│   └── src/
│       ├── global.rs            # NEW: personal/team read+write, per-domain FTS5
│       ├── traits.rs            # NEW: project_traits derivation (local-only)
│       ├── outbox.rs            # + namespace column, writer id in the key
│       ├── cursor.rs            # NEW: sync_cursor, replaces single-cursor sync_meta
│       └── search.rs            # + per-domain queries; scope CASE untouched
├── cairnd/src/
│   ├── sync.rs                  # per-namespace cursors, backoff, drain; pull fix
│   ├── briefing.rs              # + personal_notes / team_guidance sections
│   ├── handlers.rs              # new request variants; resolve_scope UNCHANGED
│   └── promote.rs               # NEW: promotion orchestration
├── cairn/src/
│   ├── mcp.rs                   # extended actions/fields only — still six tools
│   ├── main.rs                  # + cairn user / project member / team / personal / traits
│   └── wire.rs                  # new request+payload types; stale duplicate removed
└── cairn-server/
    ├── migrations/0003_collaborative_global_memory.sql   # NEW
    └── src/
        ├── api.rs               # admin users, membership, password change;
        │                        #   register + join DELETED (prerequisite patch)
        ├── auth.rs              # role, status, must_change_password enforcement
        ├── global.rs            # NEW: personal/team ingest + read-back; ingest is the
        │                        #   FIFTH validator entry point — screens against the
        │                        #   union of the pusher's memberships, refuses permanently
        │                        #   without entering `blocked` (D447)
        ├── sync.rs              # project_id predicates; recursive privacy check
        └── version.rs           # SCHEMA_3_CAPABILITIES, server_instance_id

tests/tests/                     # workspace e2e harness
├── scope_audit.rs               # REPAIR: currently passes vacuously
├── clock_swap_invariance.rs     # EXTEND to the two new domains
├── rebuild_equivalence.rs       # EXTEND to per-domain derivation
├── privacy_promotion.rs         # EXTEND to personal/team promotion
├── mcp_backward_compatibility.rs# EXTEND: still six tools, old fields still work
├── authorization_audit.rs       # NEW: membership is the only grant
├── domain_isolation.rs          # NEW: no cross-domain leakage, no project_id
├── global_non_displacement.rs   # NEW: Level 0 never contains global
├── multi_device_convergence.rs  # NEW: two writers, no clock, converges
├── namespace_sync.rs            # NEW: old server blocks global, project keeps flowing
├── promotion_gate.rs            # NEW: each surviving gate check independently; a check
│                                #   that cannot be evaluated refuses
├── global_content_validation.rs # NEW: all nine classes; an empty identity set PASSES
│                                #   (FR-580) while an unevaluable check still fails
│                                #   closed; applicability values screened as content;
│                                #   all five entry points refuse identically (SC-438)
├── privacy_payloads.rs          # NEW: no rejection echoes content; no origin digest,
│                                #   evidence id, or absolute path on any wire payload
├── admin_lifecycle.rs           # NEW: never-zero-admins under concurrency, temporary
│                                #   password flow, expired token indistinguishable from
│                                #   revoked (SC-452)
├── capability_upgrade_e2e.rs    # NEW: blocked namespace drains after a server upgrade;
│                                #   an ingest refusal does NOT block or throttle
└── migration_alpha5.rs          # NEW: 0007 proof test, interrupted-migration safety
```

**Structure Decision**: the existing eight-crate workspace is kept as-is. New code lands as
new modules inside the crates that already own the concern — domain types in `cairn-core`,
persistence in `cairn-store`, orchestration in `cairnd`, surface in `cairn`, HTTP in
`cairn-server`. No new crate is created, because no new deployable or reusable boundary
appears in this feature. The web app is not modified (admin UI deferred to Feature 005).

## Implementation Phases

Ordered so that every phase ends with something demonstrable, per Principle I. **This
sequence is the single dependency graph for the feature, and
[`tasks.md`](./tasks.md) uses exactly these ten phases with the same numbering** (D441). An
earlier draft of this plan described an eight-phase sequence; it was stale and has been
replaced. Where a phase boundary and a task boundary appear to disagree, `tasks.md` is the
finer-grained view of this same graph, not a different one.

**Prerequisite (separate patch against `main`, not part of this feature's branch)**: the
five defects in [`security-prerequisite.md`](./security-prerequisite.md). This must land and
ship before Phase 1 completes. 004 assumes it as a starting condition, and Phase 1's first
task is to prove it landed.

**Phase 1 — Setup**: confirm the prerequisite patch is present, and confirm the local
verification loop runs a full `cargo build` rather than `cargo build --tests`, which leaves
the binaries the e2e harness spawns stale.

**Phase 2 — Foundational (blocking)**: the core types, both migrations, the writer identity,
project traits, the shared content validator (D433) and the promotion gate. Nothing
story-specific. The three highest-risk negative tests are written here, before the code they
constrain. No user story begins until this phase completes.

**Phase 3 — US1, administered accounts (P1)** 🎯 MVP: roles, status, the temporary-password
lifecycle, administrator password reset (D435), the atomic never-zero-admins guarantee
(D436), `server_instance_id`, break-glass restoration, and the `cairn user` CLI. Ends with an
operator running a server where accounts are administered, nobody self-registers, and no
sequence of legal operations can lock administration out.

**Phase 4 — US2, membership and safe auto-link (P2)**: membership endpoints, the corrected
lookup, safe auto-link, and `project_id` predicates as defense in depth over the prerequisite
patch. Depends on Phase 3 because membership authorization is meaningless without accounts
and roles. Ends with a teammate cloning a repository and linking it only because they were
granted access.

**Phase 5 — US3, personal domain, local only (P3)**: personal storage, applicability,
traits, per-domain FTS5, promotion orchestration, and the MCP action extensions. Entirely
local — no sync. Ends with personal knowledge crossing projects on one machine, offline.

**Phase 6 — US4, personal across devices (P4)**: `sync_cursor`, per-namespace cursors,
per-namespace backoff and claim release, the conditional-pull fix, writer identity in the
idempotency key, the capability re-probe cycle (D437), and personal ingest and read-back.
Depends on Phase 5 — there is nothing to synchronize until the local domain works. Ends with
a second device receiving personal knowledge, and an offline divergence resolving into a
standing conflict rather than a silent winner.

**Phase 7 — US5, team domain (P5)**: team storage and relations, the
proposed/authoritative/retired lifecycle with compare-and-swap, ratification restricted to
administrators and to non-MCP surfaces, and server-instance binding for team knowledge only
(D438). Depends on Phase 6 because team synchronization reuses the namespace machinery
personal sync builds and proves. Team is deliberately second: its lifecycle is the more
complex of the two, and it inherits a transport that already works.

**Phase 8 — US6, unified bounded recall (P6)**: the two new context sections, the structural
exclusion of global from the Level 0 reserve, the Level 1 sub-cap, the search sibling arrays,
and wiring `depth` end to end. Depends on Phases 5, 6 and 7, because non-displacement cannot
be tested honestly until all three domains actually hold records — the earlier test would
have passed against an empty global store.

**Phase 9 — Repairs**: the handoff payload leak, the recursive wire check, the vacuous
`scope_audit` test, the stale duplicate field list, and the corrections to 003's
`privacy-sync.md` and `knowledge.md`. Independent of Phases 3 through 8 and safe to run in
parallel with them.

**Phase 10 — Polish and release evidence**: the compatibility matrix exercised against a real
schema-2 server, the migration proof test, `quickstart.md` executed end to end on two
machines, and release evidence.

One Phase 9 repair deserves naming here because a Phase 8 requirement depends on it.
`cairn_context` advertises `depth: {minimum|standard}` in its MCP schema
(`crates/cairn/src/mcp.rs:129`), described as *"`minimum` is Level 0 only"* — but
`Request::Context` (`crates/cairn-core/src/wire.rs:613-627`) has no `depth` field, and the
dispatch at `mcp.rs:341-366` never reads the argument. An agent asking for a minimal
briefing today silently receives the full one. FR-477 requires `depth: "minimum"` to exclude
the global sections, so 004 is the feature that wires this parameter for the first time
rather than merely extending it. Treat it as new plumbing, not a one-line fix.

## Complexity Tracking

> Recorded per Governance: complexity that strains a principle is justified here with the
> simpler alternative that was rejected.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| `outbox` table is **rebuilt**, deviating from 003's additive-only migration policy (003 FR-513) | The new entity types must be accepted by `outbox.entity_type`, which carries a SQL `CHECK` (`0005_project_intelligence.sql:456-459`). SQLite cannot alter a `CHECK` in place. `0005` already performed an `outbox` rebuild, so the recipe, cost and risk are established rather than novel. | Adding a *second* outbox table for the new types was rejected: it would split the claim/lease mechanism (`outbox.rs:92-163`) and the `(created_at, id)` ordering guarantee across two tables, so ordering between a project write and a personal write would become undefined. Dropping the `CHECK` entirely was rejected because it is the structural gate that keeps local-only entity types off the wire. |
| **Two** tables (`personal_knowledge`, `team_knowledge`) where one table with a `domain` discriminator would do | The two domains have different authorization, different lifecycles (team has `proposed`/`authoritative`/`retired`, personal has none) and different sync namespaces. More importantly, in a single shared table a forgotten `WHERE domain = ?` in any one query is a privacy breach that leaks one user's private notes to the whole server. | A single discriminated table was rejected because it makes the catastrophic mistake *writable*. Separate tables make it a compile-or-query error instead. This is the same reasoning 003 used for keeping `reusable_patterns` separate. |
| Per-namespace backoff and drain state **refactors a shipped sync loop** (`cairnd/src/sync.rs:56-118`) rather than adding alongside it | Today's backoff is process-global across all projects. Against a schema-2 server the new entity types are legitimately refused and go `blocked` — and with a global backoff that refusal would throttle *project* sync for every project on the machine. The requirement "an older server must not break normal project sync" is unsatisfiable without this. | Leaving the global backoff and special-casing the new namespaces was rejected as the same refactor with a worse shape: it would encode the exemption in the retry path, where the next namespace added would silently inherit the bug. |
| Global content is validated **twice** — once on the client, once again at server ingest — duplicating work the design otherwise treats as a single shared function | A privacy guarantee that holds only when the client runs the check is not a guarantee. A modified, stale, or buggy client wrote unvalidated content into the server store, from which it propagated to every device the user owns. The two checks are also not the same check: the client knows the one project it is in, and the server knows every project the pusher belongs to, so the server catches content naming project X written while in project Y (D447, FR-577). | Trusting the client and auditing server-side afterwards was rejected: by the time an audit runs the content has already reached the other devices, which is the harm. Screening against a project the client *declares* was rejected because it requires trusting a client-supplied claim to decide what to check a client-supplied payload against. |
| `writer_id` and `writer_seq` **cross the wire and gain server columns**, enlarging the transmitted surface that Principle V otherwise wants minimal | The prior design listed both as never-transmitted while declaring them `NOT NULL` under `UNIQUE (writer_id, writer_seq)` locally — an invariant no pulled record could satisfy. More fundamentally, FR-492's purpose is that a *peer* can detect a gap in a writer's stream, and a peer that cannot see the sequence cannot detect anything (D448, FR-582). Neither value identifies a person, a machine name, or a path: `writer_id` is an opaque per-store id and `writer_seq` is a counter. | Making the local columns nullable was rejected because it destroys the gap detection they exist for. A separate transmitted provenance record was rejected as a second record that must arrive with the first, for two integers already on the row. Server columns *without* the unique constraint were rejected because the constraint is the invariant; a column without it is just a place to store a violation. |
| The outbox **idempotency key input changes** (writer identity is mixed in), altering a shipped format | `sync_state`'s primary key is the idempotency key alone with no writer dimension (`cairn-server/migrations/0001_init.sql:144-150`), so two devices emitting a byte-identical payload collide as `duplicate` and one device's write is silently discarded. Multi-device personal memory is not correct without fixing this. | Adding a server-side device table was rejected as a Device subsystem the brief explicitly does not want; API tokens remain the per-device credential. Salting with the user id was rejected because two devices of the *same* user are exactly the colliding case. **Migration constraint**: rows already in the outbox keep their existing keys and are drained under the old scheme; only newly enqueued rows use the new input, so no in-flight work is re-keyed or duplicated. |

## Risks

- **The prerequisite patch is a hard dependency.** If Phase 0 does not ship first, Phase 2
  builds a membership model on top of an endpoint that hands membership to anyone.
- **The `outbox` rebuild is the highest-risk migration step.** It must satisfy 003's proof
  standard: rebuild the real prior schema through actual migrations 1-6, assert row and byte
  equality, and assert that an interrupted migration leaves the store on the old version.
- **`clock_swap_invariance.rs` must be extended, not copied.** It is the existing guarantee
  that no ordering depends on a clock; if the new domains are not covered by it, a
  timestamp comparison can be introduced later without any test objecting.
- **The applicability *value* is already free text, and the vocabulary was never the guard.**
  An earlier draft of this plan recorded the risk as "if a later change permits free-text
  values, the guard silently becomes a leak channel". That had the mechanism backwards: the
  closed `language | tool` vocabulary constrains a fact's *kind*, and its value has always
  been an open string, because the set of language and tool names is open by nature. The guard
  is the validator, which now screens every applicability value through the same nine classes
  as free-text content (FR-578, SC-448). The live risk is the mirror image of the one
  originally recorded: someone reads "closed vocabulary", concludes the field is safe, and
  routes a new creation path around the validator. FR-579 and its audit exist for that.
- **The bypass has been closed twice and could be re-opened a third time.** Each new way to
  create global content is a new entry point, and the count has already gone from one to four
  to five. The failure mode is not that someone disables the validator; it is that someone
  adds a path that never calls it, exactly as direct personal creation and team proposal
  originally did. SC-438 exercises all five with the same inputs, and it is the test that must
  be extended — not merely kept passing — whenever a sixth appears.
- **`0.15` is a number a reviewer will want to tune.** It is pinned so the ceiling is testable
  (D450), not because it is optimal. Changing it is a one-constant change; changing the
  *basis* from the non-reserve pool back to the whole budget is a Principle VIII regression
  wearing a one-line diff, and SC-451 is what catches it.
- **Certification by citation is the failure this feature has already committed once.** The
  third-pass sweep verified that every requirement named a task and every task named a
  requirement, and certified a design whose validator omitted two of the checks its own
  success criteria demanded. Three artifacts agreed with each other and all three were wrong
  together. Any future sweep over these artifacts has to reach the mechanism, the call sites,
  and a test that fails when the mechanism is removed.
- **Scope pressure.** Feature 003 reached 1354 spec lines and 148 tasks. Team pattern sync
  and the admin web UI are deferred to Feature 005 specifically to hold this feature down;
  re-admitting either during implementation would undo that.
