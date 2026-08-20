# Implementation Plan: Cairn Project Intelligence

**Feature directory**: `specs/003-project-intelligence` | **Git branch**:
`claude/cairn-feature-003-spec-qidkpr` | **Date**: 2026-08-14 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/003-project-intelligence/spec.md`.

| Provenance | Commit | What it carried |
|---|---|---|
| Inspected baseline | `0b79b31` (`main`) | v0.1.0-alpha.4 — the implementation this plan was written against |
| Initial specification | this artifact set | 158 FR / 28 SC |
| Clarification pass | this artifact set | 11 resolutions, +FR-308, FR-369, FR-500, +SC-326–SC-328 |
| Planning reconciliation | this artifact set | no requirement added; every requirement assigned an owning surface |
| Design reconciliation pass | this artifact set | 158 → **163 FR**, 28 → **31 SC**; D76–D83; four HIGH and four MEDIUM findings resolved |

Three of the brief's structural assumptions did not survive inspection of the baseline, and the
design turns on the corrections. They are recorded as baseline findings B1–B7 in
[research.md](./research.md) and summarised in the next section.

## Summary

Cairn already records knowledge. It does not yet maintain it. Feature 003 inserts one boundary —
between what a session **proposes** and what the project **holds** — and everything else follows
from it.

A session's contribution stays what it is today: a memory row, attributed to one session, never
rewritten. What is new is that a memory may name the **subject** it concerns, that decisions
relating memories are durable append-only records, and that the project's **canonical answer** for a
subject is *derived* from those two things rather than stored. That single choice — no canonical row
— is what makes "no silent last-write-wins" structural instead of aspirational: there is no row to
overwrite, and after any merge from any device the answer is simply recomputed.

On top of that boundary the feature adds four capabilities that share it:

- **Evidence and verification.** Bounded, redacted, attributable facts, checked by deterministic
  verifiers that read files and Git and run nothing. A memory carries a verification state separate from
  its lifecycle state, and an **authority** separate from both — what established it. An agent may attest
  a fact Cairn cannot read, and that attestation is useful, labelled everywhere, carried across sync, and
  refused by the two consumers where it would matter: criterion verification and cross-project promotion.
- **Drift.** A changed evidence fingerprint marks a claim `needs_recheck`; a verifier then finds it
  `verified`, `drifted` or `inconclusive`. No memory is ever rewritten because a file moved.
- **Compression-safe continuity.** A structured checkpoint, anchored to the handoff Cairn already
  derives, carrying the assumptions it was taken under — including a bounded fingerprint per relevant
  path, so a session resuming after compaction is told when the branch, the commit, the task state or a
  file has moved beneath it, whoever moved it and whether or not Cairn was watching.
- **Conservative cross-project reuse.** Reusable patterns: a separate, project-independent, local,
  never-synchronized record, promoted only through a deterministic fail-closed gate, with trust that
  advances on distinct non-origin projects and never on repetition or on Cairn's own suggestion.

Underneath all four, the context assembler gains a **reserved floor** with a guarantee it can actually
keep. Feature 001's briefing is one flat priority list spending one budget top-down; adding warnings and
criteria states to it would put them exactly where they get dropped. Level 0 draws from a reserve the
lower levels cannot take, and it is split in two: a **guaranteed tier** whose every field is O(1) in the
size of the project and the task — goal, status, derived progress counts, readiness, the most actionable
blocker, the next action, warning counts, repository state — and a **bounded detail tier** admitted as
budget allows, with omissions counted by kind and a retrieval path. Unbounded prose was never something a
finite budget could promise; bounded state is, and it is what an agent needs to continue.

Secondarily, tasks get stable criterion identity, blockers, and derived readiness. This is not
decoration: `update_task` writes the whole acceptance-criteria array today, so two sessions editing
different criteria lose one another's work. Per-criterion rows remove that by construction, and
`tasks.acceptance_criteria` is retained as a synchronized projection so the CLI, the briefing, the server
and the web UI keep working unchanged. Task *identity* across machines is content-addressed — a digest
over the converged criteria and blockers — because a per-store counter cannot mean the same thing on two
machines, and the counter therefore never leaves the store.

**No new crate, no new service, no new datastore, no new transactional model, no seventh tool, and
no model or vector dependency anywhere in the correctness path.**

## Technical Context

**Language/Version**: Rust (workspace, pinned via `rust-toolchain.toml`); TypeScript for the web UI,
unchanged

**Primary Dependencies**: existing only — Tokio, SQLx, serde, `uuid`, `chrono`, `regex`, `sha2`,
`clap`, `tracing`. **No dependency is added.** The one thing Feature 003 might have reached for — a
similarity library — is rejected by D46

**Storage**: the existing local SQLite database, extended by one additive migration
(`0005_project_intelligence.sql`); the existing PostgreSQL shared schema, extended by one additive
migration (`0002_project_intelligence.sql`). No existing migration is modified

**Testing**: `cargo test --workspace` across the five tiers Feature 002 established (D40), plus a new
deterministic corpus in `tests/knowledge/` and new end-to-end suites under `tests/tests/`. No
release gate reads a model's judgement (D73)

**Target Platform**: macOS, Linux and Windows developer machines, matching the current release

**Project Type**: multi-binary Rust workspace plus a separate web application — unchanged

**Performance Goals**: Feature 001's bounds hold unchanged with Feature 003 enabled on a project
carrying 5,000 memories, 10,000 evidence facts and 500 subjects — capture ≤10 ms median inside the
250 ms deadline, session open inside the 1,500 ms context deadline, session close inside its budget.
Zero verification work on the session-open path. Every bound in D75 asserted, not assumed

**Constraints**: fully offline; no evidence content, verification run, checkpoint, pattern or
selection diagnostic ever leaves the machine; no timestamp arbitration in any reconciliation path;
no command execution to establish truth; capture stays fail-soft; every derived value rebuildable
from durable records

**Scale/Scope**: one project with thousands of memories and hundreds of subjects; several sessions,
agents, worktrees, machines and team members; ten consecutive compaction cycles

## Constitution Check

*GATE: evaluated before Phase 0 and re-evaluated after design.*

| Principle | Assessment |
|---|---|
| I. Usable MVP First | PASS — eight vertically sliced phases, each ending runnable. Phase B alone (subjects, reinforcement, conflict visibility, `cairn memory subject`) is a shippable improvement over the current release: duplicate accumulation stops and disagreement becomes visible. No architecture-first phase — the pure derivation in Phase A is proved by Phase B, not by existing. |
| II. Simple Architecture | PASS, and this is the principle the design was optimised for. No new crate, no new dependency, no new service, no new datastore, no new transactional model, no seventh tool. The largest simplification is D44: the canonical answer is derived on read, so there is no projection, no invalidation, and no rebuild path. Speculative machinery the brief invited — embeddings, a graph store, an event log, a confidence engine — is rejected with reasons in research §6. |
| III. Local-First Reliability | PASS — everything works offline: reconciliation, verification, drift, context, continuity and pattern suggestion are all local. Nothing Feature 003 adds runs on the session-open or capture path beyond one indexed lookup with a documented cap (D54). Verification is background or on demand, and `unverified`/`needs_recheck` is a valid answer rather than a stall (FR-473). Cairn failure degrades Cairn's output, never the agent's operation. |
| IV. Project-Scoped Memory | PASS — no scope is added. Subjects are keyed *by* project, scope and scope key, so they narrow rather than widen. Reusable patterns are deliberately a different record rather than a wider scope (D61), which is what keeps Feature 002's FR-189/FR-190 invariant intact: nothing is keyed to the agent, and retrieval still ranks by project/branch/task/session only. Every new field carries provenance to the session that wrote it. |
| V. Privacy by Default | PASS — the boundary is drawn structurally. Evidence facts, verification runs, checkpoints, patterns, applications, task changes and selection diagnostics have **no outbox entity type and no server table**, which is the same mechanism that already makes "raw observations never sync" a schema property. What a shared memory may say about evidence is an enumerated five-field object carrying verifier *kinds* and one authority enum, no content. Promotion — the highest risk in the feature — is a deterministic fail-closed gate with a seeded adversarial corpus. |
| VI. Deterministic Data Boundaries | PASS, and strengthened. Reconciliation reads no clock and no identifier order (D49), asserted by a clock-swap invariance test. Relations are idempotent on their primary key. Every derived value has a documented rebuild exercised by test. Git remains the only source of repository state. The same inputs still produce the same briefing, now including the new levels. |
| VII. Testable Behavior | PASS — every success criterion has a named test at a named tier, including the negative ones: no false merge on the paired corpus, no silent winner on any conflict case, no lost write under 32-way concurrency, no clock-order dependence, no criterion verified on attested evidence alone, no pattern validated on repetition, no promotion that leaks a seeded secret, and no verification work on the session-open path. |

**Re-check after Phase 1 design**: PASS. The design added zero crates, zero dependencies and zero
components. Two judgment calls are recorded in Complexity Tracking; neither introduces
infrastructure.

**Re-check after the design reconciliation pass (2026-08-14)**: PASS. Eight findings resolved (R11–R18),
four of them HIGH. The resolutions added no crate, dependency, service, datastore, tool or table. Two
reduced the design: the task counter was removed from the sync payload and the server schema, and
automatic reconciliation lost one of its two rules. Principle V is strengthened — an attestation can no
longer arrive at a peer wearing a deterministic check's badge. Principle VI is strengthened — task state
identity is content-addressed rather than counter-based, so it cannot disagree across devices.

**Re-check after cross-artifact reconciliation (2026-08-14)**: PASS. Reconciliation closed ten
findings — two CRITICAL/HIGH structural, four MEDIUM, four LOW consistency (see
[Reconciliation](#reconciliation) below). The resolutions removed a materialized subject table,
removed a proposed `pattern_signals` table in favour of a bounded column plus a digest, replaced a
"canonical" flag with the derivation, tightened the criterion-verification rule, named the degradation
path for an older server, and moved one derived backfill into an explicitly documented approximation.
Every structural resolution made the design smaller, and none added a component, dependency or table.

## Project Structure

### Documentation (this feature)

```text
specs/003-project-intelligence/
├── plan.md                          # This file
├── spec.md                          # Feature specification (163 FR, 31 SC)
├── research.md                      # Baseline findings B1–B7; decisions D43–D75
├── data-model.md                    # Entities, fields, invariants, state transitions, rebuilds
├── migration.md                     # Additive migration design and its no-loss proof
├── compatibility.md                 # Feature 001 / 002 / store / server / agent compatibility matrix
├── quickstart.md                    # Acceptance walkthrough, one section per user story
├── traceability.md                  # FR/SC → owning surface → test coverage map
├── checklists/requirements.md       # Specification quality gate
├── contracts/
│   ├── knowledge.md                 # Subject identity, relations, the derivation, conflict, temporal
│   ├── evidence-verification.md     # Evidence facts, verifier catalog, the state machine, drift
│   ├── continuity-context.md        # Checkpoints, staleness, the three levels, pins, explainability
│   ├── patterns.md                  # Reusable patterns, the promotion gate, independence, counterexamples
│   ├── task-model.md                # Criteria, blockers, revision, change log, readiness
│   ├── mcp-tools.md                 # The six tools' additive extension
│   ├── privacy-sync.md              # Record catalog by boundary, outbox/server delta, degradation
│   ├── records-and-rebuild.md       # Durable record catalog, ordering, idempotency, concurrency, rebuild
│   └── evaluation.md                # The deterministic corpus, the harness, the gates
└── tasks.md                         # Generated by /speckit-tasks — NOT created here
```

### Source Code (repository root)

```text
crates/
├── cairn-core/          # + domain.rs: VerificationState, VerificationOrigin, Importance,
│                        #   RelationKind, RelationBasis, EvidenceKind, VerifierKind,
│                        #   VerifyResult, CriterionState, CriterionVerification,
│                        #   BlockerState, CheckpointState, DivergenceKind, PatternTrust,
│                        #   PatternOutcome, PatternDiscovery, ContextLevel, SelectionReason
│                        # + knowledge.rs   NEW — topic/value normalization, content_norm,
│                        #                  the subject derivation, conflict classification,
│                        #                  scope overlap. Pure, no I/O
│                        # + verify.rs      NEW — verifier specs, the total state machine,
│                        #                  fingerprint comparison. Pure
│                        # + continuity.rs  NEW — assumption comparison, divergence
│                        #                  classification. Pure
│                        # + budget.rs      reserve support (with_reserve, spend_reserved)
│                        # + context.rs     three levels, Level 0 admission order, reasons
│                        # + config.rs      the D75 bounds
│                        # + wire.rs        new requests, extended payloads, new error codes
├── cairn-git/           # + ref/commit resolution and ancestry for the git verifiers and
│                        #   branch-merge detection
├── cairn-store/         # + migrations/0005_project_intelligence.sql
│                        # + knowledge.rs   NEW — relations, subject queries, rebuilds
│                        # + evidence.rs    NEW — evidence facts, links, verification runs
│                        # + patterns.rs    NEW — patterns, applications, trust derivation
│                        # + criteria.rs    NEW — criteria, blockers, change log, revision
│                        # + continuity.rs  NEW — checkpoints
│                        # + repo.rs        memory columns, task revision, pin transitions
│                        # + search.rs      verification/conflict/topic/as_of filters
│                        # + outbox.rs      three new entity types and payload fields
├── cairnd/              # + verify.rs      NEW — verifier execution (the only worktree access)
│                        # + drift.rs       NEW — locator marking on the capture path
│                        # + continuity.rs  NEW — checkpoint write and restore
│                        # + briefing.rs    subject queries, warnings, patterns, levels
│                        # + capture.rs     drift marking hook, bounded
│                        # + main.rs        verification pass on the existing maintenance tick
│                        # + handlers.rs    new requests
├── cairn/               # + memory subject/verify/pin/reconcile, evidence, pattern,
│                        #   context --explain, session checkpoint, task criterion/blocker
│                        #   commands; MCP action dispatch; renderers
└── cairn-server/        # + migrations/0002_project_intelligence.sql
                         # + sync.rs: allowlist extension, memory_relation/task_criterion/
                         #   task_blocker upserts, sync_changes arrays, rejection classes
                         # + api.rs: verification and conflict state on the memory read-back

skills/cairn/            # + the new obligations in the usage contract and Skill (FR-498),
                         #   within the existing size bound

tests/
├── knowledge/           # NEW — the deterministic corpus (JSON fixtures)
└── tests/               # + us1_reconciliation, us2_temporal, us3_conflict, us4_evidence,
                         #   us5_drift, us6_continuity, us7_offline_merge, us8_patterns,
                         #   us9_counterexamples, us10_min_safe_context, us11_task_criteria,
                         #   clock_swap_invariance, relation_order_invariance,
                         #   rebuild_equivalence, bounds, perf_intelligence,
                         #   privacy_promotion, sync_degradation, migration_alpha4,
                         #   mcp_backward_compatibility
                         # extended: privacy_payloads, scope_audit, ci_hermeticity
```

**Structure Decision**: no new crate. The dependency-direction argument that forced
`cairn-integrate` in Feature 002 does not apply here. The reconciliation derivation, the
verification state machine and the staleness comparison are **pure functions over domain types**, so
they belong in `cairn-core` beside `context.rs` — which is exactly the existing pattern where
`cairn-core/context.rs` is pure and `cairnd/briefing.rs` feeds it from the database. Verifier
*execution* needs worktree and Git access, which only the daemon has, so it lives in `cairnd`.
Persistence stays behind daemon requests in `cairn-store`, preserving Feature 001's single-writer
rule. Splitting any of this into a new crate would give one boundary two homes and buy no
testability: the corpus is JSON fixtures against pure functions, testable with `cargo test -p
cairn-core`.

## Architectural decisions

Full reasoning in [research.md](./research.md); this is the index.

| # | Decision | Where |
|---|---|---|
| A | "Replay" means deterministic rebuild, not event replay — Cairn has no event log | B1, D43, [contracts/records-and-rebuild.md](./contracts/records-and-rebuild.md) |
| B | The canonical answer is derived on read; **there is no subject table** | D44, [contracts/knowledge.md](./contracts/knowledge.md) |
| C | Subject identity is two optional normalized columns; no ontology, no registry | D45, [contracts/knowledge.md](./contracts/knowledge.md) |
| D | Automatic reconciliation covers exactly two decidable cases | D46 |
| E | Decisions are append-only idempotent relations; `superseded_by_id` becomes a view of one | D47 |
| F | Conflict needs same-scope overlap; differing precedence is a scope exception | D48 |
| G | No timestamp or identifier order ever arbitrates, asserted by a clock-swap test | D49 |
| H | Four temporal instants; explicitly not bitemporal | D50 |
| I | Evidence facts are a new local table; Feature 001's observation evidence is untouched | D51, [contracts/evidence-verification.md](./contracts/evidence-verification.md) |
| J | Verifiers split by collector; Cairn executes nothing | D52 |
| K | The verification state machine is total and documented | D53 |
| L | Drift marks on an indexed lookup, verifies on the existing maintenance tick | D54 |
| M | Checkpoints anchor to the handoff Cairn already derives | D55, [contracts/continuity-context.md](./contracts/continuity-context.md) |
| N | Four divergence classes, all from local state | D56 |
| O | Continuity mode is derived from Feature 002 capabilities; no new event, no new capability | D57, [compatibility.md](./compatibility.md) |
| P | `Budget` gains a reserve; Level 0 has a documented admission order | D58 |
| Q | Pins are bounded, scope-respecting, cleared on supersession | D59 |
| R | Selection reasons are a closed set; `explain` costs no budget | D60 |
| S | Patterns are separate, project-independent, local, never synced | D61, [contracts/patterns.md](./contracts/patterns.md) |
| T | The promotion gate is ten deterministic checks, fail-closed, order-fixed | D62 |
| U | Trust advances on distinct non-origin projects, not on repetition or on Cairn's suggestion | D63 |
| V | A counterexample is an application, never a deletion | D64 |
| W | Three new outbox entity types; everything else is structurally local | D66, [contracts/privacy-sync.md](./contracts/privacy-sync.md) |
| X | Remote supersession lands as an imported decision, never a row overwrite | D67 |
| Y | Criteria become rows; `acceptance_criteria` stays as a synchronized projection | D68, [contracts/task-model.md](./contracts/task-model.md) |
| Z | A criterion is `verified` only on Cairn-collected evidence | D69 |
| AA | Six tools, extended by action and parameter, no nested discriminators | D70, [contracts/mcp-tools.md](./contracts/mcp-tools.md) |
| AB | No model or vector dependency in the correctness path; assistance proposes | D71 |
| AC | No new crate; five existing crates absorb the feature | D72 |
| AD | A deterministic corpus, five tiers, model judgement outside every gate | D73, [contracts/evaluation.md](./contracts/evaluation.md) |
| AE | One additive local migration, one additive server migration | D74, [migration.md](./migration.md) |
| AF | Every bound has a documented, configurable, test-asserted default | D75 |
| AG | Verification carries an **authority**; `verification_origin` is retired | D76, [evidence-verification](./contracts/evidence-verification.md) §Authority |
| AH | Equal value keys never merge; `Corroborated` is a derived subject state | D77, [knowledge](./contracts/knowledge.md) |
| AI | Symmetric relation kinds normalize their endpoints | D78, [knowledge](./contracts/knowledge.md) §Symmetric |
| AJ | Checkpoints record a bounded fingerprint per relevant path | D79, [continuity-context](./contracts/continuity-context.md) |
| AK | The task counter is local; cross-device identity is a derived digest | D80, [task-model](./contracts/task-model.md) |
| AL | `blocked` — a fifth outbox state for recoverable capability refusal | D81, [privacy-sync](./contracts/privacy-sync.md) |
| AM | The temporal claim is narrowed; `stale_at` added going forward | D82, [knowledge](./contracts/knowledge.md) §Temporal |
| AN | Level 0 has a guaranteed O(1) tier and a bounded detail tier | D83, [continuity-context](./contracts/continuity-context.md) |

### The shape of the derivation

The whole knowledge model is one pure function, which is why it is testable against a JSON corpus
and why it survives any merge:

```rust
// cairn-core/src/knowledge.rs — no I/O, no clock, no randomness.
pub fn derive_subject(
    members:   &[MemoryFacts],      // active, topic-keyed, one (scope, scope_key, topic_key)
    relations: &[Relation],         // every relation touching those members
) -> SubjectView;

pub struct SubjectView {
    pub reconciliation: Reconciliation,   // Settled | Reinforced | Conflicted | Historical
    pub answers:        Vec<Uuid>,        // 1 when settled/reinforced, ≥2 when conflicted, 0 when historical
    pub narrowed_by:    Vec<Uuid>,        // scope exceptions applicable in a narrower context
    pub decisions:      Vec<RelationRef>, // what produced this
}
```

`derive_subject` reads `state`, `value_key`, `verification`, evidence counts and relations. It does
not read `created_at`, `updated_at` or the UUID's embedded timestamp (D49). Identifier order sorts
`answers` for stable output and nothing else.

## Phasing

Each phase ends runnable. Checkpoints are demoable states; the feature is the whole table.

| Phase | Delivers | Runnable proof |
|---|---|---|
| **A. Domain** | New enums; `knowledge.rs`, `verify.rs`, `continuity.rs` as pure functions; `budget.rs` reserve; the D75 bounds; the corpus loader | `cargo test -p cairn-core` — the paired reconciliation corpus, the total state machine, and the budget-reserve property test all pass with no database |
| **B. Knowledge** | The additive local migration; relations; subject queries; reinforcement and duplication; conflict surfacing; `superseded_by_id` kept consistent; `cairn memory subject`, `reconcile`, extended `search` filters; temporal `as_of` | Three sessions recording equivalent knowledge produce one canonical answer; two incompatible proposals produce a visible conflict with no winner; `cairn memory subject <key>` explains both. `us1_reconciliation`, `us2_temporal`, `us3_conflict`, `rebuild_equivalence` pass |
| **C. Evidence & verification** | Evidence facts and links; verification runs; the verifier catalog; `cairn evidence`, `cairn verify`; the background pass on the existing maintenance tick | A memory backed by a file digest and a Git ref reports `verified` with what was checked and when; nothing verifies at session open. `us4_evidence`, `perf_intelligence` pass |
| **D. Drift** | Locator indexing; capture-path marking within its cap; the recheck transition; drift warnings | Changing a configuration file moves a `verified` memory to `needs_recheck` and then to `drifted`, with the memory byte-identical throughout. `us5_drift` passes |
| **E. Context & continuity** | Three levels; the Level 0 reserve and admission order; pins; selection reasons and `--explain`; checkpoints; staleness detection; the derived continuity mode | Ten compaction cycles preserve every continuity field; a second session moving the head and the task produces a divergence report instead of a stale instruction; a 5,000-memory project at the minimum budget still leads with the pinned constraint and the warnings. `us6_continuity`, `us10_min_safe_context` pass |
| **F. Tasks** | Criteria rows with the retained projection; blockers; the revision counter and change log; derived progress and readiness; `cairn task` extensions | Two sessions update different criteria and both survive; a session bound at revision 5 is told what changed at 6; every criterion verified reports `ready` and changes no status. `us11_task_criteria` passes |
| **G. Sync & multi-device** | Three outbox entity types; the extended memory payload; relation import and re-derivation; the server migration and allowlist; `sync_changes` arrays; degradation reporting | Two offline stores with incompatible proposals merge to a visible conflict on both, identically under reversed clocks; a supersession decided on one lands on the other; an older server degrades and says so. `us7_offline_merge`, `clock_swap_invariance`, `migration_alpha4` pass |
| **H. Patterns & evidence of the whole** | Patterns; the promotion gate; applications, independence accounting, counterexamples; signal-matched suggestion; the usage-contract and Skill update; the full corpus and the privacy suites | A verified procedure promotes, is suggested in another project labelled unverified there, is contested by a counterexample, and refuses every seeded privacy violation. The quickstart runs end to end. `us8_patterns`, `us9_counterexamples`, `privacy_promotion` pass |

Dependencies: B needs A. C needs B (evidence links to memories). D needs C. E needs B for warnings
and is otherwise independent of C/D, but its warnings are only demonstrable after D. F is independent
of C–E except that criterion verification needs C. G needs B and F (relations and criteria are what
sync). H needs C and E (promotion requires verification; suggestion requires Level 1). The
performance and privacy suites in H exercise everything.

## Risks and mitigations

| Risk | Severity | Mitigation |
|---|---|---|
| Agents never attach topic keys, so reconciliation almost never fires and the feature's headline capability is invisible | **HIGH** | The honest one. Mitigated on three surfaces: the usage contract, the Skill and the tool descriptions all ask for a topic key on durable project facts (FR-498, FR-318); `cairn_remember` accepts a key on `create` so it costs no extra call; and exact-normalized-content duplication (D46) catches the commonest accidental repeat without any key at all. If adoption is still poor, the fallback is a *reported* metric — `cairn status` showing what share of project-scoped memories carry a subject — not a similarity heuristic, which D46 rejects. |
| Level 0's reserve visibly shrinks what today's briefings show, and reads as a regression | **HIGH** | Resolved structurally by making the reserve a cap on the lower levels rather than a floor Level 0 must spend (D58, FR-442): unspent reserve returns to the general pool, so a project with no task, no warnings and no pins is byte-identical to today. `us10_min_safe_context` asserts both halves — the empty case unchanged, the loaded case reserving. |
| The subject derivation on the session-open path costs more than the context deadline allows on a large project | **MEDIUM** | One indexed query over active, topic-keyed memories in the applicable scopes, bounded by `subject_warning_scan_max` (256) with the highest-precedence scopes first and the remainder reported as degraded. `perf_intelligence` measures it at 5,000 memories and 500 subjects against Feature 001's baseline. If it regresses, the fallback is to compute warnings only for the bound task and current branch, which is where they matter. |
| Drift marking on the capture path pushes a hook past its 250 ms deadline | **MEDIUM** | `evidence_lookups_per_event_max` (8) against an index on `(project_id, source_locator)`, and exceeding it defers to the background pass rather than continuing (D54, FR-374). Capture keeps its always-exit-0 fail-soft rule unchanged, so a slow lookup drops work rather than delaying the agent. `perf_intelligence` measures capture latency per adapter with 10,000 evidence facts present. |
| Conflict warnings become noise, and developers learn to ignore them | **MEDIUM** | Two structural exclusions, not a severity dial: conflict requires the same scope *and* scope key (D48), so the project/task-fixture case and the main/feature-branch case cannot produce one; and `warnings_in_context_max` (5) bounds what reaches an agent, highest precedence first. The corpus includes both false-positive shapes as negative cases (SC-302's paired corpus). |
| Cross-project patterns leak something | **MEDIUM** | Three layers. Patterns are structurally unable to sync — no outbox entity type, no server table (D61). The promotion gate is ten deterministic fail-closed checks with the order fixed so the reported reason is stable (D62). And `privacy_promotion` runs a seeded adversarial corpus of secrets, absolute paths, project names, remotes and shared identifiers, asserting 100% refusal, no echoed value and no partial pattern (SC-315). |
| A pattern becomes "validated" through Cairn's own suggestion loop | **MEDIUM** | Resolved arithmetically (D63): applications are unique on `(pattern, project, signal_digest)` so one incident counts once, trust advances only on distinct non-origin projects, and a `cairn_suggested` application needs deterministic evidence collected in the applying project to count at all. `us9_counterexamples` asserts the ten-sessions-one-project case yields a count of 1. |
| An agent attests its way to a `ready` task and readiness becomes self-certification | **MEDIUM** | A criterion reaches `verified` only on Cairn-collected evidence (D69, FR-484), and Cairn already captures test and command outcomes with exit codes through Feature 001's hooks, so the honest path is open. SC-328 asserts the negative. |
| The retained `acceptance_criteria` projection drifts from the criteria rows | **MEDIUM** | Written in the same transaction as every criterion change, and asserted by a rebuild test that recomputes the array from the rows and compares (D68, SC-324). This is the one denormalization the feature keeps, and it is kept because removing it breaks the server, the web UI and the briefing at once. |
| A mixed-version deployment — newer daemon, older server — silently stops syncing something | **MEDIUM** | The daemon records the rejection class, stops sending that class, keeps delivering everything the server accepts, and names the degradation in `cairn sync status` (D67, FR-415). SC-326 measures it against a server accepting no Feature 003 field. |
| `superseded_at` backfilled from `updated_at` is wrong for a memory superseded and later touched | **LOW** | It is an approximation and is recorded as one (D74). It affects only historical `as_of` queries for memories superseded *before* this feature existed, where no better source exists; nothing derived from it changes current knowledge. Documented in [migration.md](./migration.md) rather than silently applied. |
| The reconciliation derivation acquires a clock read during maintenance | **LOW** | `clock_swap_invariance` runs the whole offline-merge corpus with reversed clocks and asserts a byte-identical result (D49, SC-304). A clock read that changes an outcome fails that test. |
| An agent attests a fact and it becomes indistinguishable from a check Cairn ran | **resolved** | Was a real gap on the wire, not a hypothetical: `collector` lived on the evidence fact, which never syncs. Closed by `verification_authority` travelling with the state everywhere, and by the two strict consumers refusing attestation (D76, R11, SC-329). |
| A coarse value key silently merges two different claims | **resolved** | Closed by removing the rule: only identical normalized content merges, and equal-value/differing-content derives `Corroborated` with both retained. The residual cost is deduplication requiring one explicit call, which is prompted in the response (D77, R12, SC-301). |
| Two offline machines disagree about what "revision 6" means | **resolved** | Closed by taking the counter off the wire and deriving a content-addressed `task_state_digest` from the converged records. No CRDT (D80, R13, SC-330). |
| Feature 003 sync data stranded against an old server | **resolved** | Closed by the `blocked` outbox state plus a capability probe on an endpoint that already exists, whose *absence* of the new fields is the answer for an old server (D81, R14, SC-331). |
| 163 requirements is a large surface for one feature | **LOW** | Recorded in Complexity Tracking. The feature combines four capabilities the brief deliberately merged; the requirement count is proportionate to that (Feature 002 carried 145 for one). Fifteen requirements were tightened rather than duplicated during reconciliation, which is the discipline that keeps the count honest. The eight-phase slicing is what keeps it deliverable, and every phase ends demonstrable. |

## Reconciliation

Cross-artifact reconciliation ran before task generation and closed six findings. Each resolution
made the design smaller.

| # | Severity | Finding | Resolution |
|---|---|---|---|
| R1 | CRITICAL | The first data model materialized a `knowledge_subjects` table with a `canonical` flag — the exact silent-last-write-wins path FR-303/FR-336 forbid, and an unrebuildable projection under FR-517 | Removed the table. The canonical answer is derived on read (D44). No row to overwrite, no projection to rebuild, and FR-302/FR-517 hold by construction |
| R2 | HIGH | Criterion verification accepted any evidence, so an agent could attest its way to `completion_readiness = ready`, making FR-483 decorative | Criterion verification restricted to `collector = cairn` evidence (D69, FR-484). SC-328 asserts the negative. The path stays open because Cairn captures test and command outcomes itself |
| R3 | HIGH | The plan asserted additive server compatibility but named no behaviour for a server that rejects a new field, leaving FR-415 unsatisfiable and the outbox liable to retry forever | Named the degradation: record the rejection class, stop sending it, keep delivering the rest, report in sync status (D67, FR-415, SC-326) |
| R4 | MEDIUM | A proposed `pattern_signals` table added a join for a bounded list, and gave signal matching two representations that could disagree | Folded into a bounded column plus a normalized `signal_digest` on the pattern (D61). One representation, used for both matching and duplicate detection |
| R5 | MEDIUM | `memories.state` and the `supersedes` relation could disagree after a merge, since the relation synced and the state was copied | The state and `superseded_by_id` are **re-derived from relations** on import rather than copied (D67). The Feature 001 columns become a view of the relation, which is also what makes FR-324 true |
| R6 | MEDIUM | The migration silently backfilled `superseded_at` from `updated_at` as though it were known | Kept the backfill — no better source exists — but recorded it as the feature's single derived approximation, scoped to `state = 'superseded'` rows, with its consequence stated (D74, [migration.md](./migration.md)) |
| R7 | MEDIUM | `data-model.md` presented the new `memories` constraints as DDL `CHECK`s while `migration.md` correctly noted SQLite cannot add one without rebuilding the table — the two artifacts contradicted each other about a user's data being rewritten | Reconciled on `migration.md`'s side: the `memories` predicates are enforced at the repository boundary and asserted by test; new tables keep their DDL `CHECK`s. Recorded as a deliberate deviation ([compatibility.md](./compatibility.md) §Open notes 1) |
| R8 | LOW | FR-500 enumerated six bounds while research D75 documented sixteen, so ten bounds had no requirement obliging a default | FR-500 restated as a minimum set covering all sixteen classes |
| R9 | LOW | The evaluation harness claimed a "byte-identical" briefing against the pre-feature output, which cannot hold once the response carries new top-level objects | Narrowed to the fields the claim is actually about: the `briefing` object, `estimated_tokens`, `truncated` and `omitted_sections` |
| R10 | LOW | Four planned test binaries appeared in the evaluation and traceability tables but not in the plan's source tree | Added to the tree, with the three extended existing suites named separately |

### Design reconciliation pass (2026-08-14)

A second pass re-verified eight independently raised concerns against both the artifacts and the
implementation. **All eight were valid**; four were HIGH. Each resolution was checked for whether the
existing architecture already solved the problem before anything was added.

| # | Severity | Finding | Verified against | Resolution |
|---|---|---|---|---|
| R11 | **HIGH** | Agent-attested evidence could reach `verified` and sync as indistinguishable from a deterministic Cairn check. `verification_origin ∈ {local, remote}` distinguished the *machine*, not the *kind of check*; `collector` lived on the evidence fact, which never syncs; and `basis` could not close the gap because `test_outcome`/`command_outcome` are reachable both ways | `contracts/evidence-verification.md` line 99, `contracts/privacy-sync.md` payload shape, `data-model.md` §2.1 | `verification_authority ∈ {cairn, attested, remote_cairn, remote_attested}`, derived, reported everywhere, and on the wire as one enum key. Criterion verification and promotion accept `cairn` only. No new axis — the existing `verification_origin` column was widened (D76) |
| R12 | **HIGH** | Equal `value_key` triggered automatic `reinforces`, and `derive_subject` then collapsed the value partition to one representative — so `auth.strategy=jwt` "HS256 with a shared secret" and "RS256 with rotating public keys" merged into one canonical answer with a false reinforcement | `contracts/knowledge.md` automatic-reconciliation table + `derive_subject` step 4 | Automatic merging requires identical normalized content. Equal value + differing content derives **`Corroborated`**: no relation, every statement retained, the matched member reported to the writer. `reinforces` demoted to explicit-only. Automatic behaviour got *smaller* (D77) |
| R13 | **HIGH** | `tasks.revision` was described as monotone and used for CAS, but it synced, so two offline machines each advancing 5→6 produced two different "revision 6" states — and because `task_changes` is local, a divergence report omitted criterion changes that had arrived from another machine | `contracts/task-model.md`, `data-model.md` §2.2/2.3, `contracts/privacy-sync.md` | Counter renamed `local_revision`, **removed from the payload and the server schema**. Cross-device identity is a derived `task_state_digest` over sorted converged records. Divergence diffs `sessions.task_snapshot_at_bind` against current records, so remote changes appear. No CRDT, and the sync surface shrank (D80) |
| R14 | **HIGH** | Capability-refused work was stranded. `outbox::claim` takes only `pending` and stale `in_flight`; `cairnd/src/sync.rs` calls `mark_failed` on a `rejected` status — so a relation an old server refused stayed `failed` forever, including after an upgrade | `crates/cairn-store/src/outbox.rs::claim`, `crates/cairnd/src/sync.rs` line 646, `0003_outbox_claim.sql` | Fifth outbox state `blocked`, distinct from `failed`; two nullable columns; a capability probe on the existing public `/api/version`, whose *absence* of the new fields is itself the answer for an old server. Blocked rows return to `pending` on a capability change and deliver exactly once by their original idempotency key (D81) |
| R15 | MEDIUM | FR-443 required criterion text in Level 0 and SC-309 claimed it fits the minimum budget — forty criteria at ~20 tokens is 800 against a 600-token minimum. The budget was never at risk; the *claim* was false | `spec.md` FR-443/SC-309, `contracts/continuity-context.md` Level 0 order | Level 0 split: **Tier 0a** guaranteed and O(1) (goal, progress counts, readiness, top blocker, next action, warning counts, repository state); **Tier 0b** bounded detail with criterion text in action order, omissions counted by kind with a retrieval path (D83, FR-448) |
| R16 | MEDIUM | Path divergence depended on a `file_changed` observation from another session, so an edit by a human, a formatter, `git apply` or an IDE was invisible while the commit stayed put | `contracts/continuity-context.md` divergence table | Bounded per-path fingerprints recorded at checkpoint and recomputed on restore — `digest`, `size` above the payload cap, `unknown` when excluded. `not_fingerprintable` is reported as itself, never as unchanged. ≤32 paths, no scan, no execution (D79) |
| R17 | MEDIUM | `conflicts_with` was documented as symmetric but keyed `(from, to, kind)`, so two offline machines detecting one conflict produced two durable rows facing opposite ways | `data-model.md` §3.1 | Endpoint normalization (`min`/`max` on the identifier) for symmetric kinds only; directional kinds untouched. The primary key then absorbs the second machine's record (D78) |
| R18 | MEDIUM | The `as_of` predicate claimed "what was effective at T" but `mark_stale_scopes` records no instant, so a memory stale since T2 was still reported as effective for every later T | `crates/cairn-store/src/repo.rs::mark_stale_scopes`, `contracts/knowledge.md` temporal section | Claim narrowed to proposal effectiveness and explicit supersession intervals; nullable `stale_at` added and set going forward, NULL meaning **unknown**; a transition with no authoritative instant reports `applicability: unknown` rather than a bounded fact. No backfill — that would be a second approximation (D82) |

**Requirement accounting for this pass**: 5 new FRs (FR-327, FR-370, FR-418, FR-448, FR-493) and 3 new
SCs (SC-329, SC-330, SC-331). Fifteen existing requirements were **tightened rather than duplicated** —
FR-305, FR-316, FR-321, FR-341, FR-342, FR-355, FR-368, FR-396, FR-415, FR-432, FR-443, FR-484, FR-488,
FR-489, FR-490, FR-499, FR-502 — and eight SCs restated. Nothing was added that an existing requirement
could carry.

**Complexity added by this pass**: no crate, no dependency, no service, no datastore, no MCP tool, no new
table. One outbox state, three nullable columns, one renamed column, one replaced column, one new derived
value, and one additive field on an endpoint that already exists. Two changes made the design *smaller*:
the task counter left the wire and the server schema, and automatic reconciliation lost a rule.

Findings R1–R6 came from reconciling the design against the requirements; R7–R10 from a consistency pass
across the artifact set; R11–R18 from this independent design reconciliation. All three passes verified
that every requirement and success criterion is cited, that every cross-feature citation resolves in the
Feature 001 or 002 spec, and that every internal link resolves.

Remaining MEDIUM/LOW notes are non-blocking and recorded in [compatibility.md](./compatibility.md)
§Open notes.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| 163 functional requirements for one feature, against Constitution "requirement counts kept proportionate to the work" | The brief deliberately combines four capabilities — canonical reconciliation, evidence/verification/drift, compression-safe continuity, and conservative cross-project reuse — plus a secondary task capability. Each carries its own privacy, concurrency, migration and degradation obligations, and the constitution requires those stated as requirements rather than left to implementation. Feature 002 carried 145 for one capability | Splitting into four features was considered and rejected on the brief's own terms and on the constitution's: the four share one boundary (proposal → reconciliation → canonical answer), one context budget and one privacy model, so specifying them separately would either duplicate that boundary four times or leave three features depending on an unspecified one. The eight-phase slicing delivers the same incrementality a split would, with one coherent design |
| One denormalization retained: `tasks.acceptance_criteria` as a projection of `task_criteria` | Three live readers plus the shared server and the web UI consume the JSON array today. Retaining it as a synchronized projection is what makes criterion identity an *additive* change rather than a simultaneous break across `cairn-core/context.rs`, the `cairn task` CLI, `cairn-server/sync.rs` and `web/app/(app)/projects/[id]/tasks` | Replacing the column with a join breaks all five readers at once for no capability gain. Leaving criteria as strings keeps the lost-update defect B3 identified, which is the reason the task capability is in scope at all. The drift risk is answered by writing both in one transaction and asserting the rebuild (SC-324) |
