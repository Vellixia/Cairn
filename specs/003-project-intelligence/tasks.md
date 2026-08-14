---

description: "Task list for Cairn Project Intelligence (003-project-intelligence)"
---

# Tasks: Cairn Project Intelligence

**Input**: Design documents from `/specs/003-project-intelligence/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [migration.md](./migration.md),
[compatibility.md](./compatibility.md), [contracts/](./contracts/), [quickstart.md](./quickstart.md),
[traceability.md](./traceability.md)

**Generated from**: the reconciled design at branch HEAD `e6e3055` — 163 FR, 31 SC, decisions D43–D83.
Not from the original brief. Where the two differ, the reconciled artifacts win.

**Tests**: Included, and not optional. Constitution VII requires user-observable behaviour to be
verified through it, and a large share of this feature's requirements are **negative** — no automatic
merge on a coarse value key, no attestation wearing a deterministic check's badge, no timestamp
arbitration, no capability-refused work stranded, no criterion self-certified, no evidence value on the
wire. A negative requirement that is not asserted by a test is not implemented at all. High-risk
negatives are written **before** the code they constrain, so the test is seen to fail for the right
reason first.

## Format: `[ID] [P?] [MANUAL?] [Story] Description`

- **[P]**: Safe to run in parallel — different files, no dependency on an incomplete task, no shared
  schema change, no shared module, no shared contract
- **[MANUAL]**: Requires a live, authenticated agent, so it can never be a required **CI check**
  (D73, `contracts/evaluation.md` tier 5). That is not the same as non-blocking: each manual task states
  its own `**Blocking**:` verdict, and two of the three are release-blocking because the constitution
  requires the promised flow to run on a real repository (Constitution I, VII). Only the topic-key
  effectiveness evaluation is informational
- **[Story]**: US1–US11 from [spec.md](./spec.md). Setup, Foundational, migration and the cross-cutting
  evidence phases carry no story label, because they serve every story rather than one

## Path conventions

Rust workspace at the repository root:
`crates/{cairn-core,cairn-git,cairn-store,cairnd,cairn,cairn-server}` — **no new crate** (D72).
Deterministic corpus at `tests/knowledge/`. Workspace end-to-end tests in the `cairn-e2e` package at
`tests/` with test files in `tests/tests/`. Canonical Skill source at `skills/cairn/`. Non-gating
evaluations at `evals/`.

## Phase map, and why the order differs from the plan's letters

The plan's eight delivery phases (plan.md §Phasing) map onto the phases below. One reordering is
deliberate and is the only departure from the plan's lettering:

> **The task slice (plan phase F) runs before context and continuity (plan phase E).** Level 0's
> guaranteed tier *is* task work state — goal, status, derived progress counts, readiness, the most
> actionable blocker (FR-443) — and a continuity checkpoint's guaranteed content is the same plus the
> task state digest it was taken under (FR-422, FR-424, FR-493). Building either before the task slice
> would mean building both twice. The plan already permits it: "F is independent of C–E except that
> criterion verification needs C."
>
> This does **not** promote the task capability. It stays secondary — 13 of 163 requirements, no
> project-management machinery (FR-491), and its own phase ends at exactly the scope
> [contracts/task-model.md](./contracts/task-model.md) defines.

| Plan phase | Phases here | Stories |
|---|---|---|
| **A. Domain** | 1–2 | — (setup, foundational) |
| **B. Knowledge** | 3–4 | US1, US2, US3 |
| **C. Evidence & verification** | 5 | US4 |
| **D. Drift** | 6 | US5 |
| **F. Tasks** | 7 | US11 |
| **E. Context & continuity** | 8–9 | US10, US6 |
| **G. Sync & multi-device** | 10–11 | US7 |
| **H. Patterns** | 12 | US8, US9 |
| — | 13 | Agent surface (cross-cutting) |
| — | 14 | Privacy, migration and compatibility evidence |
| — | 15 | Corpus completion and performance evidence |
| — | 16 | Convergence and release readiness |

---

## Phase 1: Setup — corpus scaffolding, fixtures, and the pre-feature baseline

**Purpose**: everything a later test needs to *compare against*, captured before anything changes.
Nothing is wired to product behaviour yet.

- [ ] T001 Create the deterministic corpus tree `tests/knowledge/{reconciliation/{equivalent,distinct,coarse_value_key,duplicate_content,free_form},conflict/{real,scope_exception,disjoint},supersession,merge/{symmetric_relation,task_divergence,blocked_recovery},verification/authority,drift,budget/oversized_task,continuity,staleness/external_edit,patterns/{promote,refuse,independence,counterexample},privacy,tasks}/` each with a `README.md` stating the rule its cases follow, and a root `tests/knowledge/README.md` recording the paired-corpus rule: every `equivalent/` case has a sibling in `distinct/` differing in exactly the way that matters, so "zero false merges" is measurable rather than aspirational (`contracts/evaluation.md` §The corpus)
- [ ] T002 [P] Add the corpus loader to `crates/cairn-core/src/corpus.rs` behind `#[cfg(test)]` plus a `tests/knowledge.rs` integration target for the crate: serde types for a fixture case (inputs, expected derivation, expected relations, expected refusals), a directory walker, and a failure message that names the fixture file — so a corpus failure points at a file rather than a line number (`contracts/evaluation.md` tier 2)
- [ ] T003 [P] Add the alpha.4 store fixture builder to `tests/src/lib.rs`: a helper that opens a store and applies **only** migrations 1–4 through the real `cairn_store::migrate` path rather than hand-written DDL, then populates it with active/stale/superseded memories, a ≥3-deep `superseded_by_id` chain, `local_only` rows, `memory_evidence` rows whose observation is deleted, tasks with empty and duplicate-string criteria, sessions with `handoff_pending = 1`, all three handoff triggers, pending/in-flight/delivered/failed outbox rows and a mid-stream `pull_cursor` (migration.md §What an existing store actually contains)
- [ ] T004 [P] Capture the pre-feature baseline into `tests/knowledge/baseline/`: the `cairn context --json` briefing object, `estimated_tokens`, `truncated` and `omitted_sections` for a project with no task, no warnings, no pins and no checkpoint, plus a recorded corpus of Feature 001/002 MCP `tools/call` requests and responses — the two things later tasks compare against for no-regression (SC-308 metric 13, SC-323 metric 36)
- [ ] T005 [P] Seed the adversarial privacy corpus at `tests/knowledge/privacy/`: one case per refusal class the promotion gate must produce — provider keys in every shape `redact.rs` knows, PEM blocks, JWTs, bearer credentials, connection strings with credentials, `KEY=value` assignments, absolute POSIX/Windows/UNC paths, the project name in four casings, the repository remote with and without credentials, a `server_project_id`, a `git_common_dir` and an email address (`contracts/evaluation.md` §The adversarial privacy corpus, SC-315)

**Checkpoint**: `cargo test --workspace` still green; the corpus loader compiles and finds zero cases;
the alpha.4 fixture builder produces a store the current build opens at schema 4.

---

## Phase 2: Foundational — domain types, bounds, and the pure functions

**Purpose**: plan phase A. Everything here is pure data or a pure function with no I/O, testable at
`cargo test -p cairn-core` with no database. This is the one phase that is legitimately
architecture-before-behaviour, and it is bounded to what every later slice consumes.

**⚠️ Blocking**: no story phase can begin until this phase is complete.

- [ ] T006 Add the Feature 003 domain enums to `crates/cairn-core/src/domain.rs` using the existing `text_enum!` macro so each gets `as_str`, `FromStr`, `ALL` and a round-trip test: `VerificationState`, `VerificationAuthority`, `Importance`, `RelationKind`, `RelationBasis`, `EvidenceKind`, `EvidenceCollector`, `EvidenceRole`, `VerifierKind`, `VerifyResult`, `VerifyTrigger`, `Reconciliation`, `CriterionState`, `CriterionVerification`, `BlockerState`, `CheckpointTrigger`, `CheckpointState`, `DivergenceKind`, `FingerprintClass`, `PathOutcome`, `PatternTrust`, `PatternOutcome`, `PatternDiscovery`, `ContextLevel`, `SelectionReason`, `OmissionReason`, `TaskChangeKind`, `CompletionReadiness`, `ContinuityMode`; extend `OutboxEntityType` with exactly `MemoryRelation`, `TaskCriterion`, `TaskBlocker` and `OutboxState` with `blocked`; keep lifecycle `MemoryState` untouched (data-model.md §7, FR-362, FR-495)
- [ ] T007 Extend the `outbox_cannot_carry_observations` test in `crates/cairn-core/src/domain.rs` to assert that **no** Feature 003 local-only record has an outbox entity type — `evidence_fact`, `verification_run`, `continuity_checkpoint`, `reusable_pattern`, `pattern_application`, `task_change`, `criterion_evidence` all fail `FromStr` — so adding one later fails a test until it is reviewed (FR-503, `contracts/privacy-sync.md` §The boundary by record)
- [ ] T008 [P] Add the sixteen D75 bounds to `crates/cairn-core/src/config.rs` with their documented defaults and a test asserting every default has not drifted: `min_safe_context_fraction` 0.40, `min_context_budget_tokens` 600, `goal_max_tokens` 60, `pin_budget_project` 12, `pin_budget_per_scope` 4, `pins_in_context_max` 4, `warnings_in_context_max` 5, `patterns_in_context_max` 2, `reconcile_members_max` 64, `subject_warning_scan_max` 256, `evidence_lookups_per_event_max` 8, `verify_pass_evidence_max` 200, `verify_pass_runs_max` 50, `verify_pass_wall_ms` 2000, `evidence_value_max_bytes` 256, `evidence_locator_max_bytes` 256, `pattern_signals_min` 2 (FR-500, SC-320)
- [ ] T009 Implement the normalization functions in the new `crates/cairn-core/src/knowledge.rs`: `normalize_topic_key` (NFC, lower-case, dot-split, `[a-z0-9_]` segments, `-`/space → `_`, ≤6 segments, ≤128 chars, total — unrepresentable input yields `None` and never an error), `normalize_value_key` (≤64, requires a topic key) and `content_norm_digest` (NFC, lower-case, whitespace-collapsed, trailing punctuation stripped, SHA-256), with the `contracts/knowledge.md` §Normalization table as the test table including `infra/prod-db`, the 7-segment reject, the SQL-shaped input and the non-representable input (FR-311, FR-312, FR-326)
- [ ] T010 Implement `scope_overlap` in `crates/cairn-core/src/knowledge.rs` returning simultaneous-applicability for the eight rows of the `contracts/knowledge.md` §Scope overlap table, reusing Feature 001's `MemoryScope::bucket` unchanged, so a project/task pair and a `branch:main`/`branch:feature/x` pair are both non-overlapping by construction (FR-332, FR-333, FR-381, FR-385)
- [ ] T011 Implement symmetric endpoint normalization in `crates/cairn-core/src/knowledge.rs`: `normalize_relation_endpoints(kind, a, b)` swaps to `(min, max)` lexicographically for `conflicts_with` **only** and is the identity for `supersedes`, `duplicates`, `reinforces`, `narrows` and `not_applicable_to`, with a test per kind asserting directional kinds are untouched (FR-305, D78)
- [ ] T012 Implement `derive_subject` in `crates/cairn-core/src/knowledge.rs` per the `contracts/knowledge.md` algorithm: drop members any `supersedes`/`duplicates` points at, partition by `value_key`, return `Historical`/`Settled`/`Reinforced`/`Corroborated`/`Conflicted`, compute `narrowed_by`, and rank a reinforced partition's representative by evidence count then verification rank (`verified(cairn)` > `verified(attested)` > `needs_recheck` > `unverified` > `drifted` > `conflicted`) then lowest id — reading no `created_at`, `updated_at`, `effective_from`, `decided_at` or UUID timestamp for arbitration (FR-302, FR-303, FR-327, FR-334, depends on T009, T010, T011)
- [ ] T013 [P] Implement `crates/cairn-core/src/verify.rs`: the total verification state machine as an explicit transition table matching `contracts/evidence-verification.md`, `derive_authority` (`cairn` when ≥1 `verified` run consulted `collector = cairn` evidence, else `attested`, `None` when not verified, strongest basis winning), and fingerprint comparison per verifier kind (FR-362, FR-363, FR-369, FR-370, FR-375)
- [ ] T014 [P] Implement `crates/cairn-core/src/continuity.rs`: `classify_checkpoint` comparing an assumption set against current state to produce `current`/`diverged`/`unresolvable` with per-class divergences, and `compare_path_fingerprint` producing `unchanged`/`changed`/`removed`/`added`/`not_fingerprintable` for the `digest`, `size` and `unknown` classes — `not_fingerprintable` never collapsing into `unchanged` (FR-431, FR-432, FR-435)
- [ ] T015 [P] Implement `derive_task_state_digest` in `crates/cairn-core/src/tasks.rs`: SHA-256 over the canonical serialization of `title_digest, goal_digest, status` ++ criteria sorted by `(ordinal, id)` ++ blockers sorted by `id`, taking no timestamp and no counter as input, with tests asserting order-independence, clock-independence and counter-independence (FR-493, D80)
- [ ] T016 Add the budget reserve to `crates/cairn-core/src/budget.rs`: `with_reserve`, `try_spend_reserved` (reserve first then general pool) and `release_reserve` (unspent reserve returns to the general pool), leaving `try_spend`'s measure-before-emit contract untouched, with the existing never-exceed property test extended to the reserved path (FR-442, FR-445)
- [ ] T017 Extend `crates/cairn-core/src/wire.rs` with the Feature 003 request variants, payload fields and the new error codes from every contract's error table — `invalid_topic_key`, `value_without_topic`, `subject_not_found`, `not_conflicted`, `relation_conflict`, `reconciliation_deferred`, `corroborating_member`, `evidence_excluded`, `evidence_outside_worktree`, `evidence_too_large`, `absolute_locator`, `verifier_unavailable`, `verification_inconclusive`, `attested_not_sufficient`, `imported_not_sufficient`, `verify_pass_yielded`, `pin_budget_exhausted`, `checkpoint_not_found`, `checkpoint_unresolvable`, `no_boundary_record`, `path_not_fingerprintable`, `revision_conflict`, `criterion_not_found`, `blocker_not_found`, `blocker_already_cleared`, `criterion_waived`, plus the ten promotion refusal classes — added to the single stable set with the existing exit-code mapping, and **no** `budget_exceeded` (FR-499, `contracts/mcp-tools.md` §Error codes)

**Checkpoint**: `cargo test -p cairn-core` passes with the corpus loader finding and running the
normalization, scope-overlap, symmetric-normalization, state-machine, digest and budget-reserve cases.
No database involved. `cargo test --workspace` still green.

---

## Phase 3: Local schema migration (blocking prerequisite)

**Purpose**: the additive local migration exactly as [migration.md](./migration.md) specifies. No
existing migration is edited; no existing row is rewritten beyond the two documented backfills.

- [ ] T018 Write `crates/cairn-store/migrations/0005_project_intelligence.sql` step 1 and step 4 — the additive columns on `memories` (`topic_key`, `value_key`, `content_norm_digest`, `importance`, `verification`, `verification_authority`, `last_verified_at`, `effective_from`, `superseded_at`, `stale_at`, `pinned`, `pinned_at`, `pinned_by_session`, `pin_reason`, `reinforcement_count`, `distinct_origin_count`), on `tasks` (`local_revision`), on `sessions` (`task_snapshot_at_bind`), on `outbox` (`blocked_reason`, `blocked_at_capability`) and on `sync_meta` (`server_capability`); the eleven new tables with their DDL `CHECK` constraints; and the six new `memories` indexes — registering it as version 5 in `crates/cairn-store/src/migrate.rs` and leaving `memory_fts` and its three triggers untouched (migration.md §Steps 1 and 4, data-model.md §2, §3, §7a)
- [ ] T019 Add the migration's two backfills and the supersession-relation conversion to `0005_project_intelligence.sql`: `effective_from = created_at` (exact), `superseded_at = updated_at WHERE state = 'superseded'` (the feature's **single** documented approximation, with the SQL comment stating its bound), and one `supersedes` relation per existing `superseded_by_id` with `basis = 'explicit_user'` and a `rationale` naming the migration; leave `topic_key`, `value_key`, `content_norm_digest`, `verification_authority` and `stale_at` NULL because inferring any of them is what FR-315 and FR-317 forbid (migration.md §Steps 2 and 3, FR-513, FR-515, D74, D82)
- [ ] T020 Add the criteria migration to `0005_project_intelligence.sql`: one `task_criteria` row per element of each non-deleted task's `acceptance_criteria` in position order with `ordinal`, `label = AC-<ordinal>`, `state = 'pending'`, `verification = 'unverified'`, `revision = 1` and `created_at` taken from the **task** rather than from now; duplicate strings produce distinct rows; empty arrays produce none; and `tasks.acceptance_criteria` is **not** modified because it is already the projection (migration.md §Step 5, FR-481, FR-492)
- [ ] T021 Implement the code-enforced constraints in `crates/cairn-store/src/repo.rs` and `crates/cairn-store/src/outbox.rs` for the predicates SQLite cannot add to an existing table without rewriting every row — `value_key` requires `topic_key`, the `importance`/`verification`/`verification_authority` domains, `verification <> 'verified'` implies a NULL authority, `pinned = 0` implies NULL pin fields, `superseded_at` implies `state = 'superseded'`, and the `outbox.state` domain including `blocked` — each asserted by a test that the same predicate a `CHECK` would express is refused at the repository boundary (data-model.md §2.1 constraints note, migration.md §Step 1)
- [ ] T022 Write `tests/tests/migration_alpha4.rs` asserting all sixteen migration properties against the T003 fixture: unchanged row counts for every pre-existing table, byte-identical pre-existing column values except the two documented backfills, every new column at its documented default, one `supersedes` relation per pre-existing link and no others, `rebuild_supersession` reproducing the pre-existing `state`/`superseded_by_id` exactly, `task_criteria` count equal to the total element count with labels and ordinals in position order, `rebuild_criteria_projection` byte-equal to every `tasks.acceptance_criteria`, outbox rows untouched and still deliverable with `pull_cursor` unchanged, `local_only` memories still producing no outbox row, FTS returning every memory it returned before with the same ranking, an injected mid-script failure leaving `schema_migrations` at 4 and the store usable by the old build, running the migration twice being a no-op, no pre-existing outbox row becoming `blocked`, and `stale` memories carrying `stale_at IS NULL` — together establishing that the migration is lossless and requires no user action (FR-514, SC-322, migration.md §Proof)
- [ ] T023 Add the schema-version guard test to `tests/tests/migration_alpha4.rs`: a build supporting schema 4 refuses to open a schema-5 database with the existing `TooNew` error rather than writing against it, and a schema-5 build opens a schema-4 store by migrating it (FR-516)

**Checkpoint**: `cargo test -p cairn-e2e --test migration_alpha4` passes. A real alpha.4 store migrates
losslessly and both Feature 001 and Feature 002 end-to-end suites pass against the migrated store.

---

## Phase 4: US1 + US2 + US3 — Canonical knowledge (Priority: P1) 🎯 First usable slice

**Goal**: repeated equivalent knowledge reconciles into one useful answer; a coarse value key never
merges two different claims; incompatible knowledge produces a visible conflict with no winner; history
and temporal truth survive.

**Independent Test**: `cairn memory add` three equivalent claims, a coarse-value-key pair and an
incompatible pair, then `cairn memory subject <key>` and `cairn memory search --as-of` — one canonical
answer, a `Corroborated` subject retaining both statements, a `Conflicted` subject with no winner, and a
correct historical answer. Nothing about evidence, context or sync is needed.

### Tests first — the negatives this slice exists to prevent

- [ ] T024 [P] [US1] Populate `tests/knowledge/reconciliation/coarse_value_key/` with ≥15 adversarial cases where one topic and one value key carry materially different statements — `auth.strategy=jwt` HS256-shared-secret against RS256-rotating-keys, `service.api_port=8080` tcp against udp, `infrastructure.production_database=postgresql` "14" against "16" — each asserting `Corroborated`, **zero** relations recorded, and every statement retained (FR-327, SC-301, D77)
- [ ] T025 [P] [US1] Write `tests/tests/us1_reconciliation.rs::no_automatic_reinforcement` and `::corroboration`: no code path writes a `reinforces` relation without an explicit request, and every `coarse_value_key/` case yields `Corroborated` with both answers returned — the test that must fail before T032 exists and pass after (FR-321, SC-301 metrics 2a/2b)
- [ ] T026 [P] [US3] Write `tests/tests/us3_conflict.rs::no_clock_arbitration`: build a subject whose members' `created_at`, `updated_at` and UUID order all disagree with each other, derive it, then rebuild it with every timestamp inverted and assert a byte-identical `SubjectView` — the mutation-style proof that no clock or id ordering decides a winner (FR-303, D49)
- [ ] T027 [P] [US1] Populate `tests/knowledge/reconciliation/{equivalent,distinct,duplicate_content,free_form}/` with ≥20 paired positive/negative cases each, and `tests/knowledge/conflict/{real,scope_exception,disjoint}/` with ≥15/≥10/≥10, so "zero false merges" and "zero false conflicts" are measured against cases that look mergeable and must not be; the `free_form/` cases additionally assert that a memory with no topic key is never merged, superseded or reinforced on the basis of similarity and stays searchable and briefable exactly as in Feature 001 (FR-313, FR-317, SC-301, SC-302)
- [ ] T028 [P] [US2] Populate `tests/knowledge/supersession/` with history-preservation cases including chains ≥3 deep, `as_of` answers either side of each supersession, and a case whose `stale_at` is NULL asserting the historical answer reports `applicability: unknown` rather than a bounded interval (SC-305, FR-342, D82)

### Storage and derivation

- [ ] T029 [US1] Add `crates/cairn-store/src/knowledge.rs`: the `memory_relations` repository — insert with endpoint normalization applied for symmetric kinds and `INSERT OR IGNORE` on the primary key so recording a decision twice changes nothing, plus queries by memory, by project and by kind (FR-304, FR-305, FR-336, depends on T011, T018)
- [ ] T030 [US1] Add the subject query to `crates/cairn-store/src/knowledge.rs`: one indexed read over active, topic-keyed memories in the applicable scopes plus their relations, bounded by `subject_warning_scan_max` with the highest-precedence scopes examined first and the remainder reported so assembly can mark itself degraded — feeding `derive_subject` and storing nothing (FR-302, FR-315, FR-474, depends on T012)
- [ ] T031 [US1] Extend `crates/cairn-store/src/repo.rs::create_memory` to accept `topic_key`, `value_key` and `importance`, compute `content_norm_digest`, and run bounded reconciliation in the same transaction: record `duplicates` on identical normalized content, detect `conflicts_with` on an incompatible value key in an overlapping scope, and return a `reconciliation` outcome of `created`/`duplicate`/`corroborating`/`conflict_detected`/`deferred` naming the matched member — writing **no** relation for the corroborating case (FR-301, FR-306, FR-308, FR-316, FR-321, FR-327, FR-474)
- [ ] T032 [US1] Add the explicit `reinforce` path to `crates/cairn-store/src/knowledge.rs`: record a `reinforces` relation with `basis = explicit_agent` or `explicit_user`, and maintain `reinforcement_count` and `distinct_origin_count` as documented derivations that never present repetition as independent confirmation (FR-321, FR-322, FR-406, closes T025)
- [ ] T033 [US2] Extend `crates/cairn-store/src/repo.rs::supersede_memory` to record the `supersedes` relation, set `state`, `superseded_by_id` and `superseded_at`, and clear the predecessor's pin — all in one transaction, so Feature 001's link stays the accurate view of the relation and never disagrees with it (FR-323, FR-324, FR-325, FR-341, FR-456)
- [ ] T034 [US3] Add the explicit reconciliation path to `crates/cairn-store/src/knowledge.rs` for `narrows`, `not_applicable_to` and conflict-resolving `supersedes`, each requiring a `basis` and refusing a relation that contradicts an existing one with `relation_conflict` — including the mutual-supersession cycle, which the derivation reports as `Conflicted` rather than resolving (FR-335, `contracts/records-and-rebuild.md` §Fail-closed)
- [ ] T035 [US2] Add `stale_at` maintenance to `crates/cairnd/src/recover.rs`: `mark_stale_scopes` records the instant it marks a memory stale, going forward only, leaving NULL to mean **unknown** and never inferring a value for a row that was already stale (FR-341, D82)
- [ ] T036 [US2] Extend `crates/cairn-store/src/search.rs` with the `topic_key` exact-and-prefix filter, the `conflicted` and `corroborated` subject filters, and the `as_of` temporal predicate `effective_from <= T AND (superseded_at IS NULL OR superseded_at > T)` — echoing `as_of` in the response and reporting `applicability: unknown` where `stale_at` is NULL, with FTS and Feature 001's ranking untouched (FR-342, FR-343, closes T028)
- [ ] T037 [US2] Add merged-branch detection to `crates/cairn-git/src/lib.rs` (is the branch tip an ancestor of the target tip) and surface branch-scoped topic-keyed memories on a merged branch as **elevation candidates** in `cairn status` and `cairn memory subject` — reported, verified against the current target branch, and **never applied**; assert in `tests/tests/us2_temporal.rs::no_automatic_elevation` that a merge elevates nothing to project scope on its own, and that branch deletion still marks branch-scoped memory `stale` rather than deleting it (FR-382, FR-383)
- [ ] T038 [US1] Implement `rebuild_supersession` and `rebuild_reinforcement` in `crates/cairn-store/src/knowledge.rs` and write `tests/tests/rebuild_equivalence.rs` covering both plus `relation_order_invariance` — shuffling the relation set and asserting an identical derivation, and discarding each cached value and asserting the rebuild equals it (FR-302, FR-517, SC-324)

### Surfaces

- [ ] T039 [US1] Add `cairn memory subject <key> [--scope]` to `crates/cairn/src/main.rs`: members, the canonical answer or answers, the reconciliation state and the decisions that produced it, in the existing `--json` envelope (FR-307)
- [ ] T040 [US1] Add `--topic-key`, `--value-key` and `--importance` to `cairn memory add` and `cairn memory supersede`, plus `cairn memory reinforce <id>` and `cairn memory reconcile --from --to --relation --basis [--rationale]`, reporting `invalid_topic_key` and `corroborating_member` as `ok: true` notes with the matched member named and the exact call that would collapse it (FR-312, FR-318, FR-327, FR-335, FR-499)
- [ ] T041 [US2] Add `--topic-key`, `--as-of`, `--conflicted` and `--corroborated` to `cairn memory search`, keeping `state` defaulting to `active` and every existing flag unchanged (FR-342, FR-499)
- [ ] T042 [US1] Write `tests/tests/us1_reconciliation.rs`, `tests/tests/us2_temporal.rs` and `tests/tests/us3_conflict.rs` against a real store and daemon: three equivalent claims yield one canonical answer with three distinct origins; a coarse-key pair yields `Corroborated`; an incompatible pair yields `Conflicted` with both answers and no winner; a project/task pair yields a scope exception; supersession preserves the predecessor byte-identically and `as_of` returns the historical answer (SC-301, SC-302, SC-305)
- [ ] T043 [US3] Write `tests/tests/us3_conflict.rs::concurrent_proposals`: 32 separate processes propose against one subject through the daemon socket, asserting 32 persisted proposals, zero lost writes, and a derivation whose outcome does not depend on commit order — and asserting the two conflict kinds stay separate, a **semantic** conflict being a derived subject state while a **concurrent write** is absorbed by `BEGIN IMMEDIATE` and the relation primary key with no second mechanism introduced (FR-331, FR-336, SC-303)

**Checkpoint**: the US1/US2/US3 quickstart sections run end to end. Duplicate accumulation stops,
disagreement is visible, a coarse value key merges nothing, and history is answerable. This is
shippable on its own.

---

## Phase 5: US4 — Evidence, verification and authority (Priority: P2)

**Goal**: a memory can carry bounded, redacted, attributable evidence and report a deterministic
verification state — with an **authority** that says what established it, and which an agent's own
attestation can never counterfeit.

**Independent Test**: attach a configuration evidence fact, run `cairn verify`, see
`verified · authority: cairn` with what was checked and at which commit. Attest a runtime fact, see
`verified (attested) · authority: attested`, and watch criterion verification and promotion refuse it.

### Tests first — authority cannot be counterfeited

- [ ] T044 [P] [US4] Populate `tests/knowledge/verification/authority/` with cases for every authority value and every strict consumer's refusal: local `cairn`, local `attested`, imported `remote_cairn`, imported `remote_attested`, a memory with both bases where the deterministic one wins, and the attested/imported refusals for criterion verification and promotion (FR-370, SC-328, SC-329)
- [ ] T045 [P] [US4] Write `tests/tests/us4_evidence.rs::authority_is_never_collapsed`: for every pair of (state, authority) the rendered CLI line, the MCP result field and the context representation all differ between `cairn` and `attested`, and no code path produces a `verified` state without an authority — the test that makes attestation-as-deterministic-check structurally visible (FR-370, SC-329)
- [ ] T046 [P] [US4] Write `tests/tests/us4_evidence.rs::state_machine` driving every documented transition in the `contracts/evidence-verification.md` table and asserting every **undocumented** one is unreachable — including that supersession changes no verification state, staleness changes none, and `drifted → verified` without a run is impossible (FR-375, SC-306)
- [ ] T047 [P] [US4] Write `tests/tests/us4_evidence.rs::cairn_runs_nothing`: no verifier path spawns a process, opens a socket or reads outside the worktree, asserted by a locator-validation table plus a network-isolated run of the whole verification suite (FR-365, FR-477)

### Storage

- [ ] T048 [US4] Add `crates/cairn-store/src/evidence.rs`: the `evidence_facts` repository with locator validation refusing an absolute POSIX path, a Windows drive path, a UNC path, a `..` traversal and anything resolving outside the worktree; `observed_value` and `source_locator` bounded **after** redaction through Feature 001's `redact.rs`; the `(project_id, source_locator)` index the drift lookup needs; and tombstone-on-delete clearing value, digest, locator and fingerprint while identity, kind, timestamps and provenance survive (FR-351, FR-352, FR-353, FR-354, FR-358, depends on T018)
- [ ] T049 [US4] Add the evidence link tables to `crates/cairn-store/src/evidence.rs`: `memory_evidence_facts` with the `supports`/`contradicts` role in the primary key and `criterion_evidence`, both surviving deletion of the fact so the reference resolves to "evidence deleted" rather than disappearing — leaving Feature 001's `memory_evidence` table entirely unmodified (FR-356, FR-359, FR-505)
- [ ] T050 [US4] Add the append-only `verification_runs` repository to `crates/cairn-store/src/evidence.rs` recording verifier, evidence, expected and observed digest, result, bounded redacted detail, branch, commit and trigger — with a test asserting a later run never rewrites an earlier one, that only the cached state moves, and that `last_verified_at` is maintained independently of `created_at`, `updated_at`, `effective_from`, `superseded_at` and `stale_at` (FR-344, FR-363, FR-364)
- [ ] T051 [US4] Implement `rebuild_verification` and `derive_authority` persistence in `crates/cairn-store/src/evidence.rs`, extending `tests/tests/rebuild_equivalence.rs` so the cached `verification`, `last_verified_at` and `verification_authority` each equal their rebuild from the runs and the evidence collectors (FR-370, FR-517, SC-324)

### Verifiers and scheduling

- [ ] T052 [US4] Add the ref, commit and ancestry reads the Git verifiers need to `crates/cairn-git/src/lib.rs`, returning an unresolvable ref or an absent commit as a distinguishable outcome rather than an error, since both mean `inconclusive` (FR-361, FR-366)
- [ ] T053 [US4] Implement the Cairn-collected verifiers in the new `crates/cairnd/src/verify.rs` — `file_exists`, `file_digest`, `git_ref`, `git_commit`, `configuration`, `schema_version`, and `test_outcome`/`command_outcome` read from a **captured** observation's recorded outcome and exit code — each honouring `excluded_paths`/`excluded_commands` by returning `evidence_excluded` rather than reading, and each producing the fingerprint its row in `contracts/evidence-verification.md` specifies (FR-361, FR-365, FR-366, D52)
- [ ] T054 [US4] Implement the agent-attested path in `crates/cairnd/src/verify.rs`: `runtime_state` and observation-less `test_outcome`/`command_outcome` stored with `collector = agent`, able to reach a memory's `verified` with authority `attested`, never re-collected by Cairn — so a recheck of attested evidence yields `needs_recheck` until the agent attests again (FR-355, FR-367, FR-370)
- [ ] T055 [US4] Add the bounded verification pass to the existing 15-minute maintenance tick in `crates/cairnd/src/main.rs`: `needs_recheck` first, oldest `last_verified_at` first, pinned before unpinned, `project` scope before narrower, capped at `verify_pass_evidence_max`, `verify_pass_runs_max` and `verify_pass_wall_ms` with concurrency 1, yielding rather than overrunning and reporting `verify_pass_yielded` — introducing no scheduler (FR-472, FR-476, SC-320)
- [ ] T056 [US4] Write `tests/tests/perf_intelligence.rs::no_verification_at_session_open` asserting **zero** verification runs occur during a `session_opened` canonical event on a project with 10,000 evidence facts, and that the context deadline is unchanged (FR-471, SC-320)

### Surfaces

- [ ] T057 [US4] Add `cairn evidence add|list|show` and `cairn verify [--memory|--task|--all] [--explain]` to `crates/cairn/src/main.rs`, rendering the authority on every line that shows a verification state — `verified`, `verified (attested)`, `verified elsewhere`, `verified elsewhere (attested)` — and `verification_inconclusive` as an `ok: true` outcome (FR-473, FR-499, closes T045)
- [ ] T058 [US4] Extend `crates/cairn-store/src/search.rs` and the `cairn memory search` flags with `--verification` and `--authority`, keeping a `drifted` memory returned by default because it stays lifecycle-`active` (FR-373, FR-499)
- [ ] T059 [US4] Write `tests/tests/us4_evidence.rs` end to end: a memory backed by a configuration value and a Git ref reports `verified` with authority `cairn` and names what was checked, against which evidence, when, and at which branch and commit; an assertion of importance stays `unverified`; a credential-bearing configuration file yields a redacted bounded fact and never the raw text (SC-306, SC-316 companion)

**Checkpoint**: the US4 quickstart section runs. Verification is deterministic, attestation is useful
and visibly weaker, and nothing verifies at session open.

---

## Phase 6: US5 — Drift (Priority: P2)

**Goal**: when supporting evidence changes, the claim it supports becomes `needs_recheck` and then
`verified` or `drifted` — and the memory itself is never rewritten.

**Independent Test**: verify a memory against `config/app.yml`, change the file, watch the state move
to `needs_recheck` and then to `drifted`, and diff the memory row to confirm it is byte-identical.

- [ ] T060 [P] [US5] Populate `tests/knowledge/drift/` with the full transition set: fingerprint change from `verified`, from `drifted` and from `conflicted`; re-verification finding the same value, a different value and an unreadable target; and the deleted-evidence case (SC-307, FR-375)
- [ ] T061 [P] [US5] Write `tests/tests/us5_drift.rs::marks_only_verification`: after a drift marking, every column of the memory row except `verification` and `last_verified_at` is byte-identical, no memory was created, and lifecycle state is untouched — the negative that makes FR-371 real (FR-371, FR-372, SC-307)
- [ ] T062 [US5] Implement drift marking in the new `crates/cairnd/src/drift.rs`: on a stored `file_changed` observation, an exact-equality indexed lookup of `evidence_facts` by `(project_id, source_locator)` capped at `evidence_lookups_per_event_max`, recomputing the fingerprint and setting supported memories to `needs_recheck` and nothing else; on a branch or commit change, the same for commit-pinned facts — exceeding the cap defers to the background pass and is not an error (FR-357, FR-371, FR-374, FR-384)
- [ ] T063 [US5] Wire drift marking into `crates/cairnd/src/capture.rs` inside Feature 001's 250 ms capture deadline with its always-exit-0 fail-soft rule unchanged, and extend `tests/tests/perf_intelligence.rs` to measure capture latency per adapter with 10,000 evidence facts present against the Feature 001 baseline (FR-374, FR-475, SC-319)
- [ ] T064 [US5] Write `tests/tests/us5_drift.rs` end to end: the `service.api_port` walkthrough from `contracts/evidence-verification.md` — verified, configuration changed, `needs_recheck`, background pass, `drifted`, warning present, memory unchanged, and the superseding memory created only by an explicit act (SC-307, FR-373)

**Checkpoint**: the US5 quickstart section runs. Evidence moving never rewrites knowledge.

---

## Phase 7: US11 — Evidence-aware tasks (Priority: P2, secondary)

**Goal**: stable criterion identity, blockers, derived progress and readiness, and a task state
identity that means the same thing on two machines. Scope is exactly
[contracts/task-model.md](./contracts/task-model.md) — nothing more.

**Why here**: Level 0 (Phase 8) and continuity (Phase 9) both consume this slice's output. See the
phase map note. The capability remains secondary.

**Independent Test**: two sessions update different criteria and both survive; a criterion reaches
`verified` only on Cairn-collected evidence; every criterion verified reports `ready` and changes no
status.

### Tests first — no silent overwrite, no self-certification

- [ ] T065 [P] [US11] Write `tests/tests/us11_task_criteria.rs::no_silent_overwrite`: two sessions updating different criteria both persist and neither resets the other; two updating the same criterion with `expected_revision` supplied refuse the loser with `revision_conflict`; two with no revision supplied both land and both appear in `task_changes` with `blind_write = true` — so no assertion is lost and no overwrite is invisible (FR-337, FR-490, SC-317)
- [ ] T066 [P] [US11] Write `tests/tests/us11_task_criteria.rs::attested_is_not_enough` and `::no_percentage_field`: a criterion never reaches `verified` on attested evidence or on any imported verification, refused with `attested_not_sufficient` and `imported_not_sufficient`; and no field exists anywhere in the task model, the wire payload or the CLI in which an agent could store a completion percentage (FR-483, FR-484, FR-486, SC-328)
- [ ] T067 [P] [US11] Populate `tests/knowledge/tasks/` with the criterion state × verification matrix, the derived progress and readiness cases, and the action-order cases Level 0 will consume (FR-482, FR-486, FR-487)

### Implementation

- [ ] T068 [US11] Add `crates/cairn-store/src/criteria.rs`: the `task_criteria` repository with stable UUIDv7 identity, `ordinal`, a `label` that is **not** renumbered when a criterion is added or removed, the independent `state` and `verification` axes, and a per-criterion `revision` for comparison (FR-481, FR-482, depends on T018, T020)
- [ ] T069 [US11] Add the `task_blockers` repository to `crates/cairn-store/src/criteria.rs`: append-only with the single `open → cleared` transition, both ends attributed, no edit and no delete-and-recreate, so reopening creates a new blocker (FR-485)
- [ ] T070 [US11] Add the local counter, the change log and the retained projection to `crates/cairn-store/src/criteria.rs`: `tasks.local_revision` advanced in the same transaction as any task, criterion or blocker change; a `task_changes` row naming author, kind, prior and new value; and `tasks.acceptance_criteria` rewritten as the ordinal-ordered projection — all in one transaction, with `rebuild_criteria_projection` added to `tests/tests/rebuild_equivalence.rs` (FR-488, FR-492, SC-324)
- [ ] T071 [US11] Wire criterion verification into `crates/cairnd/src/verify.rs` accepting only a local `cairn`-authority verification over `collector = cairn` evidence, and add derived progress and `completion_readiness` to `crates/cairn-store/src/criteria.rs` as read-time computations that never change `tasks.status` (FR-484, FR-486, FR-487, closes T066)
- [ ] T072 [US11] Add the bind snapshot and divergence derivation in `crates/cairn-store/src/criteria.rs` and `crates/cairnd/src/handlers.rs`: `sessions.task_snapshot_at_bind` written by `bind_task` and by `start_session` with a task, and a divergence report produced by **diffing that snapshot against the current synchronized criteria and blockers** — not by reading `task_changes` — with each change attributed `this_machine` or `another_machine`, and a test asserting a session bound at an earlier state is told that the task advanced and what materially changed in 100% of cases — including changes learned through synchronization (FR-489, FR-493, SC-318, D80, depends on T015)
- [ ] T073 [US11] Add `cairn task criterion add|set|verify|remove`, `cairn task blocker open|clear`, `cairn task readiness` and `cairn task history` to `crates/cairn/src/main.rs`, and extend `cairn task get` with `local_revision`, `state_digest`, criteria with their authority, blockers, progress and readiness (FR-499)
- [ ] T074 [US11] Make `cairn task update --acceptance-criteria` diff the whole list by text — preserving ids for unchanged entries, adding for new, tombstoning for removed, logging each as its own change — and assert in `tests/tests/us11_task_criteria.rs` that all five Feature 001 readers still work unchanged: `context.rs::admit_task`, the `cairn task` renderers, `outbox::task_payload`, the server's `upsert_task` and the web task page (FR-492, SC-323)
- [ ] T075 [US11] Assert in `tests/tests/us11_task_criteria.rs::no_project_management` that no sprint, epic, story-point, assignee, estimate, board or inter-task-dependency field or command exists anywhere in the task model, the wire types or the CLI (FR-491)

**Checkpoint**: the US11 quickstart section runs. Two sessions edit different criteria and both
survive; readiness is derived; completion stays explicit.

---

## Phase 8: US10 — Minimum-safe context, pins and explainability (Priority: P1)

**Goal**: a project with thousands of memories still leads with what an agent cannot work without, at
any budget from the documented minimum upwards, with a guarantee that is finite and therefore keepable.

**Independent Test**: 5,000 memories, a forty-criterion task, a pinned constraint, a drift warning and a
conflicted subject, assembled at 800 tokens — the guaranteed work state is complete, omissions are
counted with a retrieval path, and the budget is not exceeded.

### Tests first — the guarantee and the property

- [ ] T076 [P] [US10] Populate `tests/knowledge/budget/` across memory populations 0, 10, 500 and 5,000 × budgets 200…12,000, and `tests/knowledge/budget/oversized_task/` with tasks of 5, 40 and 200 criteria whose text alone exceeds the whole budget (SC-308, SC-309)
- [ ] T077 [P] [US10] Write `tests/tests/us10_min_safe_context.rs::budget` as a property test asserting `estimated_tokens <= budget` in 100% of assemblies across the whole matrix, and `::no_regression` asserting that with no task, no warnings, no pins and no checkpoint the `briefing` object, `estimated_tokens`, `truncated` and `omitted_sections` are byte-identical to the T004 baseline (FR-445, SC-308)
- [ ] T078 [P] [US10] Write `tests/tests/us10_min_safe_context.rs::critical_content_survives`: with 5,000 memories at the documented minimum budget, an unbounded number of low-priority memories can never displace the guaranteed work state, the relevant pinned constraint, the drift warning or the conflict warning — the negative that makes the reserve real (FR-442, FR-443, SC-309)

### Implementation

- [ ] T079 [US10] Restructure `crates/cairn-core/src/context.rs` into the three levels with Level 0 split in two: **Tier 0a** the guaranteed O(1) work state — task id, goal truncated to `goal_max_tokens`, status, derived progress counts, `completion_readiness`, open-blocker count plus the single most actionable blocker, `next_action` or `previous_next_action` with its divergence statement, critical warning **kinds** with counts, and repository state — and **Tier 0b** the bounded detail tier, drawing from the reserve first and then the general pool, releasing unspent reserve back so a project with no Level 0 content is unchanged (FR-441, FR-442, FR-443, FR-447, depends on T016, T071)
- [ ] T080 [US10] Implement Tier 0b's deterministic admission in `crates/cairn-core/src/context.rs`: warning detail highest-precedence first capped at `warnings_in_context_max` in the order divergence → task → conflict → drift, pinned constraints capped at `pins_in_context_max`, criterion text in **action order** (`blocked` → `satisfied but unverified` → `pending` → `verified` → `waived`, ties by ascending ordinal), then further blockers — with omissions counted by kind and a retrieval path stated, and Level 2 having no automatic admission path at all (FR-444, FR-446, FR-448, closes T078)
- [ ] T081 [US10] Add the pin repository to `crates/cairn-store/src/repo.rs`: `pinned`, `pinned_at`, `pinned_by_session` and a bounded redacted `pin_reason`; `pin_budget_project` and `pin_budget_per_scope` enforced by refusal with `pin_budget_exhausted` listing the current pins and unpinning nothing; scope never widened; and the predecessor's pin cleared in the same transaction as a `supersedes` relation while a drifted memory keeps its pin (FR-451, FR-452, FR-453, FR-454, FR-456)
- [ ] T082 [US10] Implement selection reasons and omission reasons in `crates/cairn-core/src/context.rs` as the closed enums from T006, returned only when `explain` is requested so they cost no budget otherwise — while the warnings themselves remain Level 0 content present whether or not `explain` is set (FR-461, FR-462, FR-463, FR-464)
- [ ] T083 [US10] Wire the subject and warning derivation into `crates/cairnd/src/briefing.rs`: applicable-scope subject reads feeding conflict and corroboration state, drift and attested/remote-verification notes, and the `degraded` flag when `subject_warning_scan_max` binds — reusing Feature 001's existing flag rather than adding one (FR-334, FR-373, FR-464, depends on T030, T062)
- [ ] T084 [US10] Add `cairn memory pin <id> [--off] [--reason]` and `--explain`/`--depth` to `cairn context` in `crates/cairn/src/main.rs`, rendering the selection table that answers "why did Cairn tell the agent this?" (FR-455, FR-462, FR-499)
- [ ] T085 [US10] Write `tests/tests/us10_min_safe_context.rs::oversized_task`, `::action_order` and `::pins`: a 200-criterion task at the minimum budget keeps Tier 0a complete and reports counted omissions with a retrieval path; admission follows the documented action order; the pin budget refuses at the edge and unpins nothing; and a superseded memory loses its pin while a drifted one keeps it with its warning (SC-309, FR-454, FR-456)

**Checkpoint**: the US10 quickstart section runs. 5,000 memories and forty criteria at 800 tokens — the
state survives, and what was dropped is named.

---

## Phase 9: US6 — Compression-safe continuity (Priority: P1)

**Goal**: after any number of compactions the agent still knows the goal, the state, what is blocking
it and what to do next — from Cairn, not from a summariser — and is told when the ground moved beneath
it, whoever moved it.

**Independent Test**: ten compaction cycles preserving every continuity field; a second session moving
the head and the task producing a divergence report instead of a stale instruction; a file edited with
no Cairn session involved still detected.

### Tests first — staleness must not read as current

- [ ] T086 [P] [US6] Populate `tests/knowledge/staleness/` with every divergence class alone and in combination, and `tests/knowledge/staleness/external_edit/` with relevant paths changed by an editor, a formatter, `git apply` and an IDE refactor — plus privacy-excluded and over-cap paths (SC-311, FR-432)
- [ ] T087 [P] [US6] Write `tests/tests/us6_continuity.rs::external_edit` and `::not_fingerprintable`: a relevant path modified with **no** Cairn session involved and the commit unmoved is still reported changed, and a path that cannot be fingerprinted is reported as such and never as `unchanged` — the negative that makes observation-independent detection real (FR-432, SC-311)
- [ ] T088 [P] [US6] Write `tests/tests/us6_continuity.rs::stale_action_is_labelled`: a diverged checkpoint emits `previous_next_action` with its recorded commit and **never** `next_action`, asserted for every divergence class (FR-434, SC-311)

### Implementation

- [ ] T089 [US6] Add `crates/cairn-store/src/continuity.rs`: the `continuity_checkpoints` repository anchored to a `handoff_id`, carrying the assumption set (`assumed_branch`, `assumed_commit`, `assumed_task_id`, `assumed_task_state_digest`), the bounded `relevant_paths` and `path_fingerprints`, the criteria snapshot, open blockers, pinned constraints, the derived `next_action` and the restore counters — deriving nothing the anchored handoff already carries (FR-421, FR-422, FR-423, FR-424, depends on T015, T018, T068)
- [ ] T090 [US6] Implement bounded path fingerprint capture in the new `crates/cairnd/src/continuity.rs`: `digest` by default, `size` above `payload_cap_bytes`, `unknown` when privacy-excluded, unreadable or absent — capped at the checkpoint's 32 paths with no globbing, no directory walk, no repository scan and no execution, recording the class so a weaker comparison is visible rather than implied (FR-432, D79)
- [ ] T091 [US6] Write the checkpoint at the boundaries Cairn actually gets, in `crates/cairnd/src/handlers.rs` and the sealed-close synthesis step: `context_compacting` where the adapter provides it and `session_closed` always, inside the same step that produces the handoff so the existing pending-handoff sweep covers both — and never at a turn checkpoint (FR-425, Feature 002 D22)
- [ ] T092 [US6] Implement restoration in `crates/cairnd/src/continuity.rs`: classify against current Git, the current derived task state digest and recomputed path fingerprints; report the specific differences including the recorded and current commit, the task changes with their origin, and which paths changed with the session that last touched them where known; deliver the task-independent fields when `unresolvable`; and increment `restore_count` (FR-431, FR-433, FR-435, closes T087, T088)
- [ ] T093 [US6] Derive `continuity_mode` in `crates/cairnd/src/handlers.rs` from Feature 002's existing `LifecyclePreCompaction` and `LifecyclePostCompaction` capabilities — `automatic`, `agent_initiated` naming `cairn_context(reason=post_compaction)`, or `unavailable_automatic` — adding no canonical event and no capability, and never claiming a rehydration guarantee an adapter cannot provide (FR-426, FR-427)
- [ ] T094 [US6] Add `cairn session checkpoint` and `--reason post_compaction` to `cairn context` in `crates/cairn/src/main.rs`, deriving the boundary record when none exists rather than raising `no_boundary_record`, and add `--include-checkpoint` to `cairn handoff latest` (FR-425, FR-499)
- [ ] T095 [US6] Write `tests/tests/us6_continuity.rs` end to end: ten consecutive compaction cycles asserting after **each** one that every FR-422 field present in recorded state is delivered, and that nothing is carried in the conversation — plus the `agent_initiated` and `unavailable_automatic` degradation paths reporting honestly (SC-310, FR-428, US6 #3/#4)

**Checkpoint**: the US6 quickstart section runs, including the change nobody told Cairn about.

---

## Phase 10: US7 — Multi-device synchronization (Priority: P1)

**Goal**: two machines, offline, both changing project knowledge and one task — nothing lost, nothing
decided by whose clock was later, and both converging on the same answer.

**Independent Test**: two independent stores linked to one shared project, disjoint offline writes,
sync both ways, then the same run with the clocks reversed — identical merged state both times.

### Tests first — no clock, no lost write, no duplicate edge

- [ ] T096 [P] [US7] Populate `tests/knowledge/merge/` with two-store offline scenarios each having a clock-reversed twin, plus `merge/symmetric_relation/` (the same conflict detected independently on both stores) and `merge/task_divergence/` (different criteria changed offline on each) (SC-304, SC-324, SC-330)
- [ ] T097 [P] [US7] Write `tests/tests/clock_swap_invariance.rs` running the whole merge corpus against **two real stores** with the machines' clocks reversed relative to each other, asserting a byte-identical merged canonical state and an identical conflict set — and `::symmetric_relation` asserting one symmetric decision recorded independently on both stores converges to exactly **one** durable relation (SC-304, SC-324)
- [ ] T098 [P] [US7] Write `tests/tests/us11_task_criteria.rs::offline_convergence` against two real stores: machine A changes AC-1 offline, machine B changes AC-2 offline, both sync, and every criterion and blocker change is present on both with neither overwritten, both computing an identical `task_state_digest` while their local counters differ and are never compared (SC-330, FR-490, FR-493)

### Implementation

- [ ] T099 [US7] Add the three new outbox entity types and the extended payloads to `crates/cairn-store/src/outbox.rs`: `memory_relation` (stripping `basis_evidence_id` and `rationale`), `task_criterion` and `task_blocker`; and the memory payload's `topic_key`, `value_key`, `importance`, `effective_from`, `superseded_at`, `stale_at`, `pinned`, `reinforcement_count`, `distinct_origin_count` and the **five-key** `verification` object carrying `state`, `authority`, `last_verified_at`, `fact_count` and `basis` — with `authority` sent only as `cairn` or `attested`, never `remote_*`, and a test asserting exactly five keys (FR-413, FR-414, FR-502, SC-329)
- [ ] T100 [US7] Write `crates/cairn-server/migrations/0002_project_intelligence.sql`: additive columns on `memories` including `verification_authority` and `stale_at`, the `memory_relations`, `task_criteria` and `task_blockers` tables, and **no** column on `tasks` — the local counter is not transmitted and the state digest is derived on both sides (FR-415, D80, `contracts/privacy-sync.md` §Server schema delta)
- [ ] T101 [US7] Extend `crates/cairn-server/src/sync.rs` with the three new entity upserts and the extended memory upsert, and extend the wire allowlist with the sixteen forbidden field names and six forbidden entity types so a payload carrying evidence, diagnostic or checkpoint content is refused on the wire rather than trusted not to exist (FR-506, SC-316)
- [ ] T102 [US7] Extend `GET /api/sync/changes` in `crates/cairn-server/src/sync.rs` with the optional `relations`, `criteria` and `blockers` arrays under one cursor over `updated_at`, holding and retrying a relation whose memory has not arrived rather than dropping it (FR-413)
- [ ] T103 [US7] Rework import in `crates/cairnd/src/sync.rs`: keep `import_memory`'s `INSERT OR IGNORE` so no local row is ever overwritten, import relations separately with `INSERT OR IGNORE` on the normalized primary key, upsert criteria and blockers by id, then **re-derive** local `state`/`superseded_by_id`, reinforcement counts, the criteria projection and the task state digest from the imported records — which is what makes a remotely decided supersession finally land (FR-411, FR-412, FR-413, FR-416, B2, R5)
- [ ] T104 [US7] Map imported verification authority in `crates/cairnd/src/sync.rs`: `cairn` → `remote_cairn` and `attested` → `remote_attested`, never storing the sender's value verbatim, never counting an imported verification toward local readiness or promotion, and rendering it as verified elsewhere with the peer's authority named (FR-368, FR-370, SC-329)
- [ ] T105 [US7] Write `tests/tests/us7_offline_merge.rs` against two real stores and a real server: incompatible proposals from two offline machines both survive with their provenance and produce `Conflicted` on both; a supersession decided on one lands on the other from the recorded decision; two worktrees' concurrent proposals are each attributed; and `::authority_survives` asserts a peer never renders an attested verification as a deterministic one (SC-304, SC-329, FR-417)

**Checkpoint**: the US7 quickstart section runs through "a peer's verification says how it was
established". Merge is clock-independent and loses nothing.

---

## Phase 11: US7 — Mixed-version recovery (Priority: P1)

**Goal**: work an older server refuses for lack of capability is retained, not retried, not marked
failed, and delivered exactly once after the server is upgraded — with no manual repair.

**Independent Test**: queue a relation against a schema-1 server, watch it become `blocked` while
Feature 001 entities keep syncing, upgrade the server, and watch it deliver exactly once and both peers
converge.

- [ ] T106 [P] [US7] Populate `tests/knowledge/merge/blocked_recovery/` with the capability-refusal scenario, its upgrade step and the expected delivery outcome (SC-331)
- [ ] T107 [P] [US7] Write `tests/tests/sync_degradation.rs::no_futile_retry` and `::never_permanently_failed`: a capability-refused row is retried **zero** times against a server known to lack the capability, and is never marked `failed` nor reported `delivered` — the negatives that make the state recoverable rather than decorative (FR-418, SC-326)
- [ ] T108 [US7] Add the `blocked` outbox state to `crates/cairn-store/src/outbox.rs`: `mark_blocked` recording `blocked_reason` and `blocked_at_capability`, `claim` excluding `blocked` by an explicit predicate, `release_blocked` returning rows to `pending` while preserving the original idempotency key and payload, and counts exposed for status — keeping the key, the claim mechanism, the stale-claim timeout and the drainers unchanged (FR-418, depends on T018)
- [ ] T109 [US7] Classify rejections in `crates/cairnd/src/sync.rs`: a **content** rejection stays permanently `failed` exactly as today, and a **capability** rejection (`unknown_entity_type`, `unknown_field`, `schema_older`) becomes `blocked` with its class recorded — the correction to the pre-existing behaviour that stranded refused work (FR-415, FR-418, D81)
- [ ] T110 [US7] Add `schema_version` and a `capabilities` array to the existing public `GET /api/version` in `crates/cairn-server/src/version.rs`, additively and unauthenticated, so an older server's **absence** of both fields is itself the answer — with a test asserting the current web UI consumer is unaffected (FR-415, FR-418, D81)
- [ ] T111 [US7] Add the capability probe to `crates/cairnd/src/sync.rs`: at most once per drain cycle, cached in `sync_meta.server_capability`, and on a change release the blocked rows whose class the new capability supports so the ordinary drain delivers them with their original idempotency keys under the server's unchanged `sync_state` claim (FR-418, closes T107)
- [ ] T112 [US7] Report degradation in `cairn sync status` and `cairn doctor` in `crates/cairn/src/main.rs`: the blocked count, the named capability gap, what is still syncing normally, and that the retained items will be delivered automatically when the server is upgraded (FR-415, FR-499)
- [ ] T113 [US7] Write `tests/tests/sync_degradation.rs` end to end against a real schema-1 server and then a real schema-2 server: 100% of the Feature 001 payload delivered throughout, every refused item retained, the upgrade releasing them, delivery **exactly once**, both peers converged, and no manual repair of stored data at any point (SC-326, SC-331)

**Checkpoint**: the US7 quickstart section's "an old server, and then an upgraded one" runs.

---

## Phase 12: US8 + US9 — Reusable cross-project patterns (Priority: P3)

**Goal**: a verified project solution can become a sanitized, applicability-bounded pattern that helps
another project without claiming it is true there — and cannot become trusted by repetition, by its
origin project, or by Cairn's own suggestion.

**Independent Test**: promote a verified procedure, see it offered in another project labelled
unverified there, record a counterexample and watch it become contested, and confirm ten same-project
applications yield a distinct-project count of 1.

### Tests first — the gate and the accounting

- [ ] T114 [P] [US8] Populate `tests/knowledge/patterns/{promote,refuse}/` with gate-passing candidates and one case per refusal class — including `attested_not_sufficient` and `imported_not_sufficient`, which are the two ways an agent could otherwise launder its own claim into cross-project knowledge (FR-396, SC-328)
- [ ] T115 [P] [US9] Populate `tests/knowledge/patterns/{independence,counterexample}/` with one project × ten sessions, three projects × one session, suggested-only-without-evidence, and a not-applicable outcome carrying an alternative cause (SC-313, SC-314)
- [ ] T116 [P] [US8] Write `tests/tests/privacy_promotion.rs` over the T005 adversarial corpus asserting 100% refusal, that no refusal echoes the offending value, and that no partial pattern exists afterwards (FR-397, SC-315)
- [ ] T117 [P] [US9] Write `tests/tests/us9_counterexamples.rs::one_incident_counts_once` and `::suggested_does_not_validate`: ten same-project applications yield a distinct-project count of 1, and a `cairn_suggested` application without deterministic local evidence never advances trust (FR-402, FR-403, SC-314)

### Implementation

- [ ] T118 [US8] Add `crates/cairn-store/src/patterns.rs`: the `reusable_patterns` repository with **no `project_id`**, bounded `signals` with a `signal_digest` over the sorted normalized set used for both matching and duplicate detection, `root_cause_digest`, the trust enum, a machine-salted opaque `origin_ref`, `origin_deleted`, the `sanitization_report` naming classes and never values, and the unique `(signal_digest, root_cause_digest)` index that makes duplicate refusal structural (FR-391, FR-392, FR-393, depends on T018)
- [ ] T119 [US8] Implement the ten-check promotion gate in `crates/cairn-store/src/patterns.rs` in its fixed order so the reported reason is stable — source active, `verified` with `cairn` authority, ≥1 evidence fact, not `local_only`, subject not conflicted, transferable type, no secret surviving redaction, no absolute path or project identifier, ≥`pattern_signals_min` signals, no duplicate — refusing with the class named, echoing no value, and writing nothing (FR-395, FR-396, FR-397, FR-507, closes T114, T116)
- [ ] T120 [US9] Add `pattern_applications` and trust derivation to `crates/cairn-store/src/patterns.rs`: unique on `(pattern_id, project_id, signal_digest)` so one incident counts once; `discovery` set by the **daemon** from whether this session received the pattern in its context rather than by the agent; `is_origin` excluded from trust; and `rebuild_pattern_trust` evaluating the staged ladder `candidate → sanitized → validated` with `contested` evaluated **before** `validated`, so a pattern carrying both successes and counterexamples reports both sides (FR-394) (FR-401, FR-402, FR-403, FR-404, FR-405, closes T117)
- [ ] T121 [US8] Implement signal-matched suggestion in `crates/cairnd/src/briefing.rs`: normalized tokens from `error` observations and `failure`-type memories in the applicable scopes, overlapping by ≥`pattern_signals_min`, capped at `patterns_in_context_max`, always labelled unverified in this project with its applicability, caveats and any known alternative cause — as a separate array never mixed into memory results (FR-398, FR-405, SC-312)
- [ ] T122 [US8] Handle deletion and origin in `crates/cairn-store/src/patterns.rs`: deleting the source memory or the origin project leaves the pattern with `origin_deleted = 1` and an origin reference resolving to "origin deleted", never a dangling reference and never restored content (FR-399, FR-505)
- [ ] T123 [US8] Add `cairn pattern list|show|promote|outcome|forget` to `crates/cairn/src/main.rs` with `--dry-run` on promote reporting the gate outcome without writing, and add `--include-patterns` to `cairn memory search` returning patterns in a separate array; report counts as `applications N · distinct projects N · independently validated in N · counterexamples N` and never as a number of verifications (FR-406, FR-499)
- [ ] T124 [US8] Write `tests/tests/us8_patterns.rs` and `tests/tests/us9_counterexamples.rs` end to end across two projects in one store: a verified procedure promotes, is suggested in the second project labelled unverified there, a counterexample makes it contested without decreasing anything or deleting it, and later suggestions carry the alternative cause and what to check first (SC-312, SC-313)

**Checkpoint**: the US8 and US9 quickstart sections run.

---

## Phase 13: Agent surface — MCP, contract and Skill (cross-cutting)

**Purpose**: the six-tool surface extended backward-compatibly, and the obligations taught where agents
actually read them. Each slice above already added its own CLI; this phase is what spans them.

- [ ] T125 Extend the six tool definitions in `crates/cairn/src/mcp.rs` with the Feature 003 actions and parameters from `contracts/mcp-tools.md` — `cairn_remember` gaining `reinforce`, `attach_evidence`, `verify`, `pin`, `reconcile`, `promote`, `record_outcome`; `cairn_task` gaining `add_criterion`, `update_criterion`, `blocker`, `readiness`; `cairn_session` gaining `checkpoint`; `cairn_context` gaining `explain`, `depth`, `include_patterns` and `reason=post_compaction`; `cairn_search` gaining `verification`, `authority`, `corroborated`, `conflicted`, `topic_key`, `as_of`, `pinned`, `include_patterns`; `cairn_handoff` gaining `include_checkpoint` — one discriminator per tool and **no** nested sub-operation (FR-495, FR-496)
- [ ] T126 Add the Feature 003 read-only result fields to `crates/cairn/src/mcp.rs`: `minimum_safe` split into `guaranteed`, `detail` and `omitted`; `continuity` with its mode, checkpoint and typed divergences; `warnings`; `patterns` always carrying `verified_in_this_project: false`; `selection` only when `explain` is set; and per-search-result `topic_key`, `value_key`, `importance`, `pinned`, the five-key `verification` object with its authority, `temporal`, `reinforcement` and `subject` (FR-496, FR-497, SC-312)
- [ ] T127 Write `tests/tests/mcp_backward_compatibility.rs` replaying the T004 recorded corpus of Feature 001/002 tool calls against the Feature 003 server and comparing **every pre-existing response field** for equality, plus asserting the surface is still exactly six tools and that `stop` is still absent from `cairn_handoff`'s triggers (FR-495, FR-497, SC-323)
- [ ] T128 Update the canonical usage contract at `crates/cairn-integrate/assets/agent-contract.md` and the Skill at `skills/cairn/` with the four new obligations — give durable project facts a topic and value key specific enough to state the whole claim; attach evidence rather than asserting importance; if Cairn reports a corroborating member and it is the same claim, reinforce it; record a pattern outcome including a negative one — staying inside the existing documented size bound and letting the revision digest change as it does for any content change (FR-498, Feature 002 FR-123/FR-125)
- [ ] T129 Add the one-clause-per-tool descriptions from `contracts/mcp-tools.md` §Tool descriptions to `crates/cairn/src/mcp.rs`, and assert in `tests/tests/mcp_backward_compatibility.rs` that the MCP `instructions` string is still generated from the single canonical contract source and still within its bound (FR-498, Feature 002 FR-129)
- [ ] T130 Report `continuity_mode` per agent in `cairn agents` and `cairn doctor` in `crates/cairn/src/main.rs`, derived from Feature 002's capability profile with no capability added, and assert in `tests/tests/us6_continuity.rs` that Claude Code and Codex report `automatic`, OpenCode `agent_initiated` and generic MCP `unavailable_automatic` as outputs of the rule rather than a maintained table (FR-426, FR-427)
- [ ] T131 Add the subject-adoption metric to `cairn status` in `crates/cairn/src/main.rs` — the share of project-scoped memories carrying a subject identity, plus the conflicted, needs-recheck and drifted counts and any sync degradation — so the mechanism's actual reach is observable in every project without anyone running an evaluation (FR-499)

**Checkpoint**: a Feature 001 caller sees byte-identical pre-existing fields; the surface is six tools;
every agent reports its continuity mode honestly.

---

## Phase 14: Privacy, migration and compatibility evidence (cross-cutting)

**Purpose**: the properties that span every story, asserted rather than reviewed.

- [ ] T132 Extend `tests/tests/privacy_payloads.rs` with a rejection case for **every** newly forbidden field name (`observed_value`, `source_locator`, `value_digest`, `fingerprint`, `relevant_paths`, `path_fingerprints`, `criteria_snapshot`, `task_snapshot_at_bind`, `sanitization_report`, `origin_ref`, `alternative_cause`, `signal_digest`, `pin_reason`, `rationale`, `basis_evidence_id`, `detail`, `prior_value`, `new_value`, `content_norm_digest`) and **every** newly forbidden entity type (`evidence_fact`, `verification_run`, `continuity_checkpoint`, `reusable_pattern`, `pattern_application`, `task_change`), asserting the rejection names the field (FR-506, SC-316)
- [ ] T133 [P] Write `tests/tests/privacy_integration.rs::nothing_local_escapes` asserting structurally that no outbox row can be constructed for an evidence fact, an evidence link, a verification run, a continuity checkpoint, a reusable pattern, a pattern application, a task change or a criterion evidence link; that a `local_only` memory still produces no outbox row and nothing derived from it leaves either — including a pinned one, whose `pinned` flag may travel while `pin_reason` and every other free-text field do not; and that the server schema contains none of those tables (FR-457, FR-501, FR-502, FR-503, FR-504, FR-508, SC-316)
- [ ] T134 [P] Write `tests/tests/privacy_integration.rs::deleted_origin_reports_deletion` covering every row of the `contracts/privacy-sync.md` §Deletion table — observation, evidence fact, memory, session, task and project — asserting no deletion leaves a dangling reference and none restores content (FR-505)
- [ ] T135 [P] Extend `tests/tests/scope_audit.rs` to assert no memory scope, partition, ownership domain or retrieval filter was added beyond project, branch, task and session, that no `importance`, `pinned`, `verification` or `verification_authority` value can change scope precedence; that no vocabulary, taxonomy or registry of topic keys exists anywhere in the source or the assets; and that no valid-time table, retroactive interval correction or branching history exists — the properties this feature guarantees by **absence** (FR-314, FR-345, FR-381, SC-327)
- [ ] T136 [P] Extend `tests/tests/ci_hermeticity.rs` to assert no language-model client, embedding library, vector database or graph database appears in any manifest, that no retrieval or verification path reaches the network; that every agent-originated proposal is stored with `basis = explicit_agent` and passes a deterministic gate before it affects stored state; and that **no required CI check and no release gate reads a model's judgement**, the `evals/` tree being excluded from the gating test set by construction (FR-511, FR-512, SC-321, SC-325)
- [ ] T137 Write `tests/tests/sync_degradation.rs::older_daemon_newer_server` asserting the reverse direction needs nothing: a daemon that sends no Feature 003 field works against a schema-2 server, and unknown read-back arrays are ignored rather than failing (FR-415)
- [ ] T138 Run the full Feature 001 and Feature 002 end-to-end suites against a migrated alpha.4 store and record the result in `tests/tests/migration_alpha4.rs`, asserting every behaviour a developer depends on still holds (FR-519, SC-323)

**Checkpoint**: every privacy and compatibility property is a passing test, not a review note.

---

## Phase 15: Corpus completion and performance evidence (cross-cutting)

**Purpose**: the metric table in `contracts/evaluation.md` §Metrics and gates, measured with numbers,
and the bounds asserted at their defaults.

- [ ] T139 Complete the corpus metric harness at `tests/knowledge/metrics.rs`: run every corpus directory, emit the 36-row metric table with actual numbers, and fail the run if any required row misses its target — so the release notes carry measurements rather than assertions (`contracts/evaluation.md` §Metrics and gates)
- [ ] T140 Write `tests/tests/bounds.rs` asserting every one of the sixteen D75 bounds is honoured at its default and that exceeding each one produces the documented deferral or refusal rather than unbounded work (FR-500, SC-320)
- [ ] T141 Extend `tests/tests/perf_intelligence.rs` with the loaded-project fixture — 5,000 memories, 500 topic-keyed subjects, 10,000 evidence facts, 1,200 relations, 200 verification runs, 40 patterns, 30 tasks × 6 criteria — and measure session open inside the 1,500 ms context deadline, capture at ≤10 ms median and ≤25 ms p95 inside 250 ms per adapter, and session close inside its budget, against the Feature 001 baselines and treating a saturated host as an invalid measurement rather than a failure (SC-319)
- [ ] T142 [P] Add subject-derivation, drift-marking and background-pass measurements to `tests/tests/perf_intelligence.rs`: the applicable-scope subject read bounded by `subject_warning_scan_max`, drift marking at ≤8 indexed lookups per observation, and the background pass never exceeding its three caps (FR-472, FR-474, SC-320)
- [ ] T143 [P] Add multi-device sync measurement to `tests/tests/us7_offline_merge.rs`: merging a loaded project's relations, criteria and blockers between two stores stays bounded, and the re-derivation after import is a bounded indexed pass rather than a full scan (FR-417, SC-319)

**Checkpoint**: the metric table has numbers on both CI platforms, and every bound is asserted.

---

## Phase 16: Convergence and release readiness

**Purpose**: the evidence a release needs, and the one place a human is genuinely required.

- [ ] T144 Add `cairn doctor --rebuild-derived [--project]` to `crates/cairn/src/main.rs` recomputing every derived value, reporting how many differed and exiting non-zero if any did — a release where a derived value disagrees with its rebuild ships a known inconsistency (FR-478, FR-518, SC-324)
- [ ] T145 Update `CHANGELOG.md` with the Feature 003 entry and the user-visible result per user story, and record the non-blocking notes from `compatibility.md` §Open notes in `docs/feature-003-followups.md` so they are not rediscovered from scratch (plan.md §Reconciliation)
- [ ] T146 [MANUAL] Run the `specs/003-project-intelligence/quickstart.md` walkthrough end to end on a real repository with a live agent, section by section for all eleven user stories. **Passes when**: every section produces the output it documents. **Record**: the transcript and any deviation, in the release evidence. **Blocking**: yes — this is the constitution's "runs on a real repository" gate (Constitution I, VII)
- [ ] T147 [MANUAL] Build the topic-key effectiveness evaluation at `evals/topic-key-effectiveness/` — `corpus.md` (curated durable project facts in prose, per project archetype), `protocol.md` (fresh session per agent, the corpus prompts, what to record), `RESULTS.md` and `analysis.md`. **Do**: run the corpus against **Claude Code**, **Codex** and **OpenCode**, each with its native integration, recording per agent the topic-key adoption rate, value-key specificity, same-fact cross-session consistency, cross-agent consistency, missed grouping, false grouping and safely-reconcilable share. **Passes when**: the table is complete and dated for all three agents — there is **no threshold**. **Record**: `RESULTS.md`, dated, per release. **Blocking**: **NO — informational effectiveness evidence, NOT a deterministic correctness gate.** It cannot fail a build and no threshold is defined for it. A low adoption rate is a *product* finding that sends us to the usage contract, the Skill and the tool descriptions — never to a similarity heuristic, which D46 rejects on correctness grounds. Its only permitted effect on the deterministic system is to propose corpus cases for a human to review. **One exception is a real finding**: a non-zero false-grouping count is a design defect, because only identical content merges, and it must be raised as such rather than recorded as a number (FR-499, D73, `contracts/evaluation.md` §Topic-key effectiveness)
- [ ] T148 [MANUAL] Run the tier-5 live-agent continuity walkthrough for each connected agent: drive a real compaction on Claude Code, Codex and OpenCode and confirm the reported `continuity_mode` matches what actually happens. **Passes when**: each agent's observed behaviour matches its derived mode, and no agent claims a rehydration guarantee it does not deliver. **Record**: per-agent notes in the release evidence. **Blocking**: yes for the honesty claim in FR-426 — a mode that over-claims is a defect, not a note (FR-426, `contracts/evaluation.md` §Release evidence)

**Checkpoint**: `cargo test --workspace` green on both CI platforms; the quickstart runs; the metric
table and the effectiveness table are recorded.

---

## Dependencies & execution order

### Phase dependencies

```text
Phase 1  Setup ──────────────────────────────┐
                                             ▼
Phase 2  Foundational (pure) ─── BLOCKS every story phase
                                             ▼
Phase 3  Local migration ─── BLOCKS every storage phase
                                             ▼
Phase 4  US1/US2/US3  Canonical knowledge  ◄── the first shippable slice
                          │
            ┌─────────────┼──────────────┐
            ▼             ▼              ▼
Phase 5  US4         Phase 7  US11    (Phase 8 needs both)
Evidence & authority  Tasks
            │             │
            ▼             │
Phase 6  US5 Drift        │
            │             │
            └──────┬──────┘
                   ▼
Phase 8  US10  Minimum-safe context, pins, explainability
                   ▼
Phase 9  US6   Compression-safe continuity
                   ▼
Phase 10 US7   Multi-device synchronization
                   ▼
Phase 11 US7   Mixed-version recovery
                   ▼
Phase 12 US8/US9  Reusable patterns
                   ▼
Phase 13 Agent surface  ──▶  Phase 14 Privacy/migration evidence
                   ▼                        ▼
Phase 15 Corpus + performance  ──▶  Phase 16 Convergence
```

The graph is **acyclic**. The one cycle the design could have had — a checkpoint needing a task state
digest while the task slice needed a bind snapshot — is broken by `derive_task_state_digest` being a
pure function in Phase 2 (T015) and the task slice running before continuity.

### Story dependencies

| Story | Depends on | Independently testable after |
|---|---|---|
| US1, US2, US3 | Phases 1–3 | Phase 4 |
| US4 | US1 (evidence links to memories) | Phase 5 |
| US5 | US4 (drift marks verification state) | Phase 6 |
| US11 | US4 for criterion verification only | Phase 7 |
| US10 | US11 (Tier 0a is task work state), US3 + US5 for warnings | Phase 8 |
| US6 | US11 (task state digest, criteria snapshot), US10 (Level 0 shape) | Phase 9 |
| US7 | US1 (relations), US11 (criteria) | Phases 10–11 |
| US8, US9 | US4 (promotion needs `cairn` authority), US10 (suggestion is Level 1) | Phase 12 |

### Within a phase

- Corpus fixtures and the negative tests they drive come **first** and must fail for the right reason
- Domain and pure functions before storage
- Storage before daemon behaviour
- Daemon before CLI and MCP
- The slice's end-to-end test last, closing the phase

### Parallel opportunities

`[P]` is used only where the tasks touch different files, share no schema change, share no module and
share no contract. Genuine clusters:

- **Phase 1**: T002–T005 — four independent fixture directories
- **Phase 2**: T008 and T013–T015 — separate new modules with no cross-dependency. T009–T012 are **not**
  parallel with each other: all four write `crates/cairn-core/src/knowledge.rs`, and T012 additionally
  consumes T009–T011. T016 and T017 are **not** parallel with each other or with T006, which they both
  consume
- **Phase 4**: T024–T028 — five independent corpus directories and test files
- **Phase 5**: T044–T047 — four independent test files
- **Phase 6**: T060–T061, **Phase 7**: T065–T067, **Phase 8**: T076–T078, **Phase 9**: T086–T088,
  **Phase 10**: T096–T098, **Phase 11**: T106–T107, **Phase 12**: T114–T117 — each phase's test-first
  cluster
- **Phase 14**: T133–T136 — four independent test files
- **Phase 15**: T142–T143 — different test files

Deliberately **not** parallel, and why:

| Would-be pair | Why not |
|---|---|
| T018, T019, T020 | all edit `0005_project_intelligence.sql` |
| T009–T012 | all four write `crates/cairn-core/src/knowledge.rs` |
| T029–T038 | most edit `crates/cairn-store/src/knowledge.rs` or `repo.rs` |
| T048–T051 | all edit `crates/cairn-store/src/evidence.rs` |
| T079, T080, T082 | all edit `crates/cairn-core/src/context.rs` — the central assembler |
| T099, T108 | both edit `crates/cairn-store/src/outbox.rs` |
| T101, T102 | both edit `crates/cairn-server/src/sync.rs` |
| T125, T126, T129 | all edit `crates/cairn/src/mcp.rs` — the same contract |
| T053, T054, T055 | all edit `crates/cairnd/src/verify.rs` |
| Any CLI pair | `crates/cairn/src/main.rs` is one file |

## Implementation strategy

### First shippable slice — Phase 4

1. Phases 1–3: setup, pure domain, migration
2. Phase 4: canonical knowledge
3. **Stop and validate**: the US1/US2/US3 quickstart sections. Duplicate accumulation stops,
   disagreement becomes visible, a coarse value key merges nothing. That is a shippable improvement over
   v0.1.0-alpha.4 on its own.

### Incremental delivery

Each phase from 4 onward ends with a quickstart section that runs. In order: canonical knowledge →
evidence and authority → drift → evidence-aware tasks → minimum-safe context → continuity →
multi-device → mixed-version recovery → patterns. Nothing later breaks anything earlier, and every
checkpoint is demoable.

### Parallel team strategy

After Phase 3, two tracks can run concurrently with one integration point:

- **Track K** (knowledge): Phase 4 → 5 → 6
- **Track T** (tasks): Phase 7, needing only T044–T059's criterion-verification path from Track K

Both converge at Phase 8, which needs Tier 0a's task work state and the warnings from both tracks.
Phases 10–12 are sequential after that; Phases 13–16 are cross-cutting and largely parallel among
themselves.

## Notes

- `[P]` means different files, no shared schema change, no shared module, no shared contract
- Every task names its owning requirement and the test or artifact that closes it. **A checked box is
  never evidence** — the named test is
- Negative requirements are tested before the code they constrain, and the test must be seen to fail for
  the right reason
- Commit after each task or logical group; stop at any checkpoint to validate the slice
- No new crate, no new dependency, no new service, no new datastore, no seventh MCP tool
- Avoid: vague tasks, same-file conflicts, cross-story dependencies that break independence, and
  fake parallelism
