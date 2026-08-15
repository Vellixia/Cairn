# Feature 003 implementation log

The recovery record for a long implementation run. Written so that a session picking this up — after
a compaction, a restart, or a week — can re-establish exactly where the work stands without
re-deriving it.

`tasks.md` is the ledger and stays authoritative for *what is done*. This file records *how it was
proved*, what moved, and what the next action is.

## Baseline, established 2026-08-15

| Fact | Value |
|---|---|
| `origin/main` at start | `0b79b314f616df27409370b8dce54193c092a1fc` |
| Design baseline named in spec.md | `0b79b314f616df27409370b8dce54193c092a1fc` |
| **Did main move?** | **No.** No rebase, no reconciliation needed |
| Branch | `claude/cairn-feature-003-spec-qidkpr` |
| Branch HEAD at start | `893240029bf02fb19ea52d7917e7edbda0184ed6` (the task-generation commit) |
| Ahead / behind main | 3 / 0 |
| Ledger at start | 148 tasks, T001–T148, no gaps, no duplicate ids, **0 checked** |
| Working tree at start | clean |

Implementation had not begun. Nothing was reset, cleaned, restored or force-checked-out.

### Where the work happens

A separate worktree at `.claude/worktrees/feature-003`, because the developer's checkout carries
substantial uncommitted work on `fix/daemon-cwd-and-codex-lifecycle` that must not be disturbed.
Both share one `CARGO_TARGET_DIR` (the repository's `target/`), so the 11 GB dependency cache is
reused rather than duplicated — the host has single-digit gigabytes free.

Commits are unsigned, matching the three commits already on this branch.

## Checkpoints

### Checkpoint A — Phase 1 (T001–T005): corpus scaffolding, fixtures, pre-feature baseline

| | |
|---|---|
| Tasks | T001, T002, T003, T004, T005 |
| Commit | `e5d4ff1` — pushed |
| Suite | `cargo test --workspace` — exit 0, 0 failures across every target |

**What landed**

- `tests/knowledge/` — the full corpus tree from `contracts/evaluation.md` §The corpus, 32
  directories, each with a `README.md` stating the rule its cases follow, and a root README
  recording the paired-corpus rule.
- `crates/cairn-core/src/corpus.rs` — the loader. Serde case types, a recursive walker, and a
  failure message that names the fixture file. Fixture labels (`m1`, `rs256`) are assigned UUIDs in
  **sorted label order**, so lexicographic label order and identifier order agree — which is what
  lets a fixture express `derive_subject`'s lowest-identifier tiebreak without writing a UUID by
  hand.
- `crates/cairn-core/tests/knowledge.rs` — the tier-2 target. Asserts every contract-named group
  exists with its README and every fixture in the tree parses.
- `cairn_store::migrate::run_to(pool, target)` — `run` now delegates to it. This is what lets the
  fixture stand a store up at schema 4 through the **real** migration scripts instead of
  hand-written DDL, and what makes the schema-version guard testable (T023).
- `cairn_e2e::alpha4` — the alpha.4 store fixture: migrations 1–4, then the state
  `migration.md` §What an existing store actually contains names. Fixed identifiers and timestamps,
  so a later diff attributes every difference to the migration rather than to the fixture.
- `cairn_e2e::baseline` — read/write plus `normalize`, which replaces exactly the values that differ
  between two runs (identifier, timestamp, absolute path, commit sha, sandbox project name) and
  leaves every field *name* untouched.
- `tests/knowledge/baseline/` — the pre-feature briefing subset and a 12-call Feature 001/002 MCP
  corpus, captured against the pre-feature build.
- `tests/knowledge/privacy/` — 30 seeded adversarial cases, one per class the promotion gate must
  refuse, with `generate.py` beside them so the corpus is regenerable and auditable.

**Evidence**

| Task | Proof |
|---|---|
| T001 | `cairn-core --test knowledge::every_contract_named_group_exists` |
| T002 | 8 unit tests in `corpus.rs` + the three tier-2 target tests |
| T003 | `migration_alpha4::the_fixture_stands_at_schema_four_and_carries_a_store_in_use`, `::the_fixture_is_reproducible` |
| T004 | `baseline_capture::the_committed_baseline_is_present`; capture run twice and diffed — byte-identical |
| T005 | 30 cases load; 18 `possible_secret`, 12 `project_identifying` |

**Deliberate deviations, recorded rather than silent**

1. **`corpus.rs` is a plain `pub mod`, not `#[cfg(test)]`** (T002 names the latter). Tier 2 is an
   *integration* target — `cargo test -p cairn-core --test knowledge`, as
   `contracts/evaluation.md` §Tiers documents it — and an integration target links the library as an
   ordinary dependency, where a `#[cfg(test)]` module is not visible. Gating it behind a feature
   would change the documented command. The repository already does exactly this for
   `cairnd/src/testsupport.rs`. `ci_hermeticity` will assert no product path calls it (T136).

2. **Gate check 7's operative reading**, recorded in `tests/knowledge/privacy/README.md`.
   `contracts/patterns.md` states the check as "content, *after* redaction, still matches the
   redaction pattern set", which catches what redaction missed — correct for a memory that reached
   the store through Feature 001's pipeline, where redaction already ran. Read alone it would let a
   candidate carrying a live provider key be laundered through redaction and promoted, which
   SC-315 forbids. The check is therefore
   `still_secret_shaped(content) || redact(content) != content`. The second disjunct is a no-op for
   anything that came through the normal path (redaction is idempotent and already ran), so this
   adds no behaviour for legitimate flows and makes "refuses 100% of violating candidates" hold.
   No requirement changed; no artifact needed editing.

3. **Baselines are compared normalized, not raw.** A response carries identifiers, timestamps,
   sandbox paths and commit shas that differ every run; comparing those would fail for reasons that
   have nothing to do with regression. `baseline::normalize` replaces those by shape and touches no
   field name, so a dropped or renamed field — the actual regression — still fails the comparison.

### Checkpoint B(i) — Phase 2 (T006–T017): domain types, bounds, and the pure functions

| | |
|---|---|
| Tasks | T006–T017 |
| Suite | `cargo test -p cairn-core` — 181 unit tests, 5 corpus-target tests, 0 failures |

Plan phase A. Everything here is pure data or a pure function with no I/O, testable with no
database — which is what lets the reconciliation derivation, the verification state machine and the
staleness comparison be interrogated by a JSON corpus rather than by an end-to-end run.

**What landed**

- `domain.rs` — 29 new enums plus three `OutboxEntityType` variants and the fifth `OutboxState`.
  The `text_enum!` macro gained per-variant doc comments and `Ord` (declaration order, for stable
  output only — nothing decides correctness by comparing enums).
- `knowledge.rs` — `normalize_topic_key`, `normalize_value_key`, `content_norm_digest`,
  `scope_overlap`, `normalize_relation_endpoints`, `derive_subject`. The contract's normalization
  table is the test table, verbatim.
- `verify.rs` — the total verification state machine as an explicit transition table,
  `derive_authority`, and the per-verifier fingerprint forms.
- `continuity.rs` — `classify_checkpoint` and `compare_path_fingerprint`.
- `tasks.rs` — `derive_task_state_digest`, derived progress, completion readiness, the action order
  and the criteria projection.
- `budget.rs` — `with_reserve` / `try_spend_reserved` / `release_reserve`, with the never-exceed
  property test extended to interleaved reserved and general spending.
- `config.rs` — the seventeen D75 bound fields, and `context_reserve`.
- `wire.rs` — 38 Feature 003 error codes in the existing stable set, with no `budget_exceeded`.

**Decisions taken while implementing, and why**

1. **`derive_subject` reports a mutual supersession rather than resolving it.** Step 1 of the
   contract's algorithm drops every member a `supersedes` points at. Applied literally to a cycle —
   A supersedes B and B supersedes A, which two offline machines can each decide — it drops both
   and the subject reads as `Historical`. That would let two mutually exclusive decisions annihilate
   a subject. T034 already requires the cycle to report `Conflicted`, so the derivation returns
   `Conflicted` with both members when the drop would empty an otherwise non-empty active set.

2. **`SubjectView.narrowed_by` comes from recorded `narrows` relations.** The contract's step 6 says
   "from the caller's applicable set", but the declared signature takes only members and relations.
   The durable record of a scope exception *is* the `narrows` relation (data-model §3.1), so that is
   what the derivation reads. Selection-time precedence — the project/task-fixture case — is
   `scope_overlap` plus the assembler, not this function.

3. **Reinforcement accounting is a separate field from `answers`.** A duplicate is dropped from the
   answer set but stays individually retrievable and still counts toward `distinct_origin_count`.
   Collapsing the two would lose exactly the accounting FR-406 forbids misreporting.

4. **Deviation: `unicode-normalization` added to `cairn-core`.** The plan says "No dependency is
   added". `contracts/knowledge.md` specifies Unicode NFC in all three normalizers, and without it
   `content_norm_digest` differs for two canonically equal strings — so exact-duplicate detection
   misses them and FR-326 is silently weakened. The crate was already in the workspace lockfile via
   `stringprep`; adding it produced a **one-line** lockfile change (a new edge, zero new packages),
   measured rather than assumed. The dependency the plan actually rejects is a *similarity* library,
   rejected by D46 on correctness grounds; this one is the opposite — it is what makes the digest
   deterministic. T136's hermeticity assertion must be a denylist of model/embedding/vector/graph
   clients rather than an allowlist of current dependencies, or it will trip on this.

5. **`PathFingerprint` carries an explicit `exists` flag.** The contract's three classes conflate
   "absent when the checkpoint was taken" with "excluded" and "unreadable" under `unknown`, which
   leaves `PathOutcome::Added` and `Removed` underivable — and both are in the enum the data model
   names. The flag is local-only (checkpoints never synchronize), so it costs nothing on the wire.

### Checkpoint B(ii) — Phase 3 (T018–T023): the additive local migration

| | |
|---|---|
| Tasks | T018–T023 |
| Commit | `c4177aa`, plus the review fixes below |
| Suite | `migration_alpha4` 14 passed · `cairn-store` 54 passed · `cargo test --workspace` green |

**What landed**

- `migrations/0005_project_intelligence.sql` — sixteen columns on `memories`, one on `tasks`, one on
  `sessions`, two on `outbox`, one on `sync_meta`; eleven new tables with their DDL `CHECK`s; six new
  `memories` indexes; the two backfills; the supersession-relation conversion.
- `migrate::run_to` gained a Rust **finisher** step, inside the migration's own transaction.
- `cairn-store/src/constraints.rs` — the predicates SQLite cannot add to an existing table.
- `repo::set_memory_intelligence` — the single boundary those predicates are enforced at.
- `tests/tests/migration_alpha4.rs` — fourteen tests.

**Decisions**

1. **The criteria conversion is Rust, not SQL.** A criterion's `id` is a UUIDv7 by the convention
   every other identifier in this schema follows, and SQLite can only produce a random value *shaped*
   like one. A migration that claims a time ordering its values do not have is a small lie in a table
   other code sorts by. The step runs inside the same transaction, so
   `an_interrupted_migration_rolls_back_entirely` still holds — proved by injecting a mid-script
   failure and asserting `schema_migrations` stayed at 4 and the store still opens.

**Review findings, raised against the committed migration and fixed**

| # | Finding | Fix |
|---|---|---|
| A1 | `set_memory_intelligence` validated `state` and never wrote it, so the `superseded_at` predicate was an assertion about what the caller *claimed* rather than about the row — and `supersede_memory` (T033) would bypass the boundary entirely | `MemoryColumns` narrowed to exactly the fields the function writes. The two-column predicate moved to `check_supersession`, which T033 calls where it writes both |
| A2 | `pinned: Some(1)` with no metadata set the pin **and cleared** `pinned_at`, `pinned_by_session` and `pin_reason` — a pin nobody could account for, erasing a previous pin's author | `pinned = 1` now requires all three (FR-452), refused at the boundary |
| A3 | `json_array_length(signals) BETWEEN 2 AND 16` is a `CHECK` calling a JSON1 function. `CREATE TABLE` accepting it says nothing about whether it constrains at insert | `the_new_tables_enforce_their_check_constraints` exercises the refusal — one signal, seventeen signals, and the duplicate-identity index |
| A4 | `memory_relations` had foreign keys on both endpoints, but `records-and-rebuild.md` §Fail-closed requires a relation naming a memory that has not synced yet to be **ignored and reported**, never refused. A hard FK refuses the insert, so T103's `INSERT OR IGNORE` would have dropped it silently | The foreign keys are gone from those two columns, with the reason in the DDL. `a_relation_may_reference_a_memory_that_has_not_arrived` asserts it |

A4 was found before Phase 10 could inherit it, which is the whole value of reviewing a migration
while it is still the newest thing in the tree. **Open for T102/T103**: `task_criteria.task_id` and
`task_blockers.task_id` keep their foreign keys, matching Feature 001's existing precedent
(`sessions.task_id` references `tasks`). If sync can deliver a criterion before its task, that
ordering needs the same hold-and-retry T102 already specifies for relations.

### Disk

`target/debug/incremental` had reached 7.9 GB and was the real pressure, not the dependency cache.
Removing it freed 5.8 GB. Every cargo invocation from here passes `CARGO_INCREMENTAL=0` so it does
not regrow; the cost is slower rebuilds of changed crates, which is the right trade on this host.

### Checkpoint C (partial) — Phase 4 (T024, T027, T029–T031)

| | |
|---|---|
| Commits | `446eaa3` (corpus) · `f508df7` (storage) |
| Suite | `cairn-core --test knowledge` 11 passed over 150 fixtures · `cairn-store` 66 passed · `cargo test --workspace` green |

**What landed**

- 150 corpus fixtures across nine directories, generated by
  `tests/knowledge/generate_reconciliation.py` and **checked** by the tier-2 harness against the
  real `derive_subject` and `classify_proposal`. A fixture with a wrong expectation fails the run.
- `cairn-core::knowledge::classify_proposal` — the pure automatic-reconciliation decision.
- `cairn-store::knowledge` — `record_relation`, `relations_touching`, `subject`,
  `subject_members_tx`, `rebuild_reinforcement`, `rebuild_supersession`.
- `repo::create_memory_reconciled` — the proposal and its decision in one transaction, with
  `CreateOutcome` carrying the reconciliation and its notes.

**Metrics now measured rather than asserted**: 1, 2, 2a, 2b, 3, 4.

**One ambiguity settled while implementing.** Whether exact-content duplication applies to a
**free-form** memory. `plan.md`'s risk table says it works "without any key at all", but FR-321
scopes duplication to "an existing member of **the same subject**", a subject requires a topic key
(FR-315), and FR-313 requires a free-form memory to behave exactly as it does in Feature 001 — where
two identical memories are two memories. The requirements govern, and the conservative reading is
also the one that cannot suppress a claim.

*Consequence worth reporting*: the mitigation plan.md offers for its HIGH risk "agents never attach
topic keys, so reconciliation almost never fires" is weaker than the plan claims. Exact-content
duplication does **not** catch the commonest accidental repeat without a key. The remaining
mitigations — the usage contract, the Skill, the tool descriptions, and the adoption metric on
`cairn status` (FR-499) — are unaffected. This is a product finding, not a defect.

### Checkpoint C — Phase 4 (T024–T043) complete: canonical knowledge

| | |
|---|---|
| Commit | `42c005f` — pushed |
| Suite | `cargo test --workspace` green, 57 suites, 0 failures |

The first shippable slice. Duplicate accumulation stops, disagreement is visible, a coarse value key
merges nothing, and history is answerable.

**What landed beyond the storage layer already recorded**

- `cairn-store::knowledge` — `reinforce`, `reconcile`, `reconcile_as`, `supersedes_transitively`,
  `branch_scoped_subjects`, and the two rebuild procedures.
- `repo::supersede_memory` now records the `supersedes` relation, sets `superseded_at` and clears the
  predecessor's pin, all in one transaction.
- `repo::mark_stale_scopes` records `stale_at` going forward.
- `search` gained `topic_key` (exact or prefix), `as_of`, `conflicted` and `corroborated`, plus the
  `temporal` block on a result.
- `cairn-git` gained `is_merged_into`, `resolve_ref` and `commit_present`.
- Wire: `MemorySubject`, `MemoryReinforce`, `MemoryReconcile`; `topic_key`/`value_key`/`importance`
  on create and supersede; `Temporal` and `Applicability`.
- CLI: `memory subject`, `memory reinforce`, `memory reconcile`, and the new flags on
  `memory add`/`supersede`/`search`. `render::subject` explains an answer, or why there is not one.
- Tests: `us1_reconciliation` (10), `us2_temporal` (11), `us3_conflict` (6),
  `rebuild_equivalence` (6).

**Decisions taken while implementing**

1. **`reconcile` refuses three contradictions** rather than letting the derivation clean up after
   them: a supersession that closes a cycle (directly or through a chain, bounded walk), a second
   successor for a memory that already has one, and a self-reference. Without the second,
   `rebuild_supersession` would pick a successor arbitrarily.
2. **A conflict cannot be *declared*.** `memory reconcile --relation conflicts_with` is refused with
   `not_conflicted`: a conflict is detected automatically and left standing until a supersession, a
   narrowing, or a distinguishing verification resolves it.
3. **`cairn memory add` still defaults to branch or task scope**, unchanged from Feature 001, while
   `cairn memory subject` defaults to project scope. The tests say which scope they mean rather than
   relying on a default that differs between the two commands.
4. **A derived subject filter reads candidates and derives**, bounded by `SUBJECT_FILTER_SCAN_MAX`
   (512), because a subject state cannot be a SQL predicate. The limit is applied after derivation,
   or it would cut the set the derivation is computed over.

### Checkpoint D — Phase 5 (T044–T059) complete: evidence, verification and authority

| | |
|---|---|
| Suite | `cargo test --workspace` green, 59 suites, 0 failures |

**What landed**

- `cairn-store::evidence` — the `evidence_facts` repository with locator validation, redaction
  **before** bounding, the tombstone that keeps a reference resolvable, the two link tables, the
  append-only `verification_runs`, `rebuild_verification`, and the five-key sync `summary`.
- `cairnd::verify` — the verifier catalog (`file_exists`, `file_digest`, `git_ref`, `git_commit`,
  `configuration`, `schema_version`, `test_outcome`, `command_outcome`), the attested path, and the
  bounded pass wired onto the existing 15-minute maintenance tick.
- Wire and CLI: `EvidenceAdd/List/Show`, `Verify`; `cairn evidence add|list|show`,
  `cairn verify [--memory|--all] [--explain]`; `--verification` and `--authority` on
  `memory search`. `render::authority_line` gives the four authorities four distinct renderings.
- Corpus: `verification/authority/` — 15 cases, checked by three tier-2 tests.
- Tests: `us4_evidence` (8), `perf_intelligence` (3).

**Decisions taken while implementing**

1. **A configuration locator names its key after a `#`** — `config/app.yml#server.port`. The reader
   handles `key: value`, `key = value` and `"key": value`, and matches a dotted key on its **last
   segment**. That can find the wrong `port` in a file with two, which makes the check
   inconclusive-prone rather than wrong — and a wrong verification is the failure that matters.
2. **`set_verification` refuses to write `verified`.** Reaching `verified` is only expressible by
   recording a run and rebuilding, so no code path can mark a memory verified without a durable
   record behind it.
3. **`file_exists` treats absence as a result, not a failure to look.** `exists:0:0` is a legitimate
   fingerprint, so a file that has gone has *drifted* rather than become inconclusive.
4. **The verifier module is asserted to contain no process or socket call at all.**
   `us4_evidence::cairn_runs_nothing` reads the source and refuses `Command::new`, `TcpStream`,
   `reqwest` and the rest. It is a blunt check, and it is the one that would actually catch someone
   adding a shell-out to make a verifier "work".

### Checkpoint E — Phase 6 (T060–T064) complete: drift

| | |
|---|---|
| Suite | `cargo test --workspace` green, 60 suites, 0 failures |

**What landed**

- `cairnd::drift` — `mark_for_path` (exact locator equality, capped at
  `evidence_lookups_per_event_max`, deferring rather than continuing) and `mark_for_commit_change`
  (only facts pinned to a commit that is no longer current).
- Marking wired into the observe path for a `file_changed` observation, and the commit-change form
  onto the maintenance tick immediately before the verification pass.
- Corpus: `drift/` — 15 cases, each naming a `(state, trigger)` pair and the state it produces, or
  `null` where the contract documents no transition. Checked against the real `transition`.
- Tests: `us5_drift` (4), `cairnd::drift` (8).

**Decisions**

1. **An `unverified` memory is never marked.** There is nothing to recheck, and moving it would
   claim a verification it never had. `mark_supported` filters to `verified`, `drifted` and
   `conflicted`.
2. **`mark_for_commit_change` takes the current head** and marks only facts recorded at a
   *different* commit. Marking every commit-pinned fact on a branch would recheck things that have
   not moved.

### Checkpoint F — Phase 7 (T065–T075) complete: evidence-aware tasks

| | |
|---|---|
| Suite | `cargo test --workspace --all-targets` green, 914 tests, 0 failures |
| Gates | `cargo fmt --all -- --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` exit 0 |

**What landed**

- `cairn-store::criteria` — the `task_criteria` and `task_blockers` repositories, the local
  counter, the change log, the retained projection, derived progress and readiness, the bind
  snapshot and the divergence derivation. Every mutation funnels through one `commit_changes`
  helper inside one transaction, which is why `local_revision`, `task_changes` and
  `tasks.acceptance_criteria` cannot disagree with the criterion rows.
- `repo::update_task` now delegates to `criteria::update_task`, so there is exactly one task write
  path. `repo::create_task` seeds criterion rows in its own transaction, and `bind_task` and
  `start_session` record `task_snapshot_at_bind`.
- `cairnd::verify::verify_criterion` — criterion verification admitting only a local
  `cairn`-authority result over `collector = cairn` evidence.
- `cairn task criterion add|set|verify|remove`, `task blocker open|clear`, `task readiness`,
  `task history`, and `task update --acceptance-criteria`; `task get` gained `local_revision`,
  `state_digest`, criteria, blockers, progress and readiness.
- Corpus: `tasks/` — 28 cases across the criterion state × verification matrix, derived readiness
  and the action order, checked against the real pure functions, plus an exhaustiveness test that
  enumerates the product from the enums themselves.
- Tests: `us11_task_criteria` (9), `rebuild_equivalence::rebuild_criteria_projection_equals_the_stored_array`,
  `cairn-core::knowledge` (2 new).

**Decisions**

1. **`StoreError::Refused { code, message }`.** The store now carries a refusal's stable wire code
   instead of the daemon matching on message text, which is how `revision_conflict` stays
   distinguishable from `storage_unavailable` at the agent surface.
2. **Ordinals are allocated over every row including tombstoned ones.** The unique index is partial
   (`WHERE deleted_at IS NULL`), so a maximum over live rows alone would reissue AC-3 after AC-3 was
   removed and never trip the constraint — silently minting a second AC-3 (FR-481).
3. **Task-model writes resolve an author without creating one** (`authoring_session`).
   `ensure_session_for_memory` starts a `cairn-cli` session when there is none, which is right for a
   memory and wrong here: `cairn task new` has never needed a session, and inventing one leaves a
   second active session in the worktree that makes the next agent's `cairn_context` ambiguous. The
   nil UUID means "no session" and renders as an unattributed change.
4. **Attested evidence is refused by name, not by derived authority alone.** Cairn never re-collects
   an agent's observation, so an attested fact yields an *inconclusive* run and would otherwise be
   reported as `source_unverified` — as if no evidence existed. When the only evidence offered is
   attested, that is the reason, and `attested_not_sufficient` says so (FR-370).
5. **Divergence is reported on `context` for now.** T072 requires the derivation and a handler; Phase
   8 places it in the Level 0 tier. It is attached as `task_divergence` so no session is presented as
   having worked against the current state in the meantime.
6. **Criteria and blockers are not yet on the wire.** Sync of the rows themselves is Phase 10
   (T099, T103). Phase 7 enqueues only the task's own payload, which carries the retained projection.

**Two pre-existing gate failures fixed**

`6a33f85` did not pass `cargo fmt --all -- --check` or
`cargo clippy --workspace --all-targets -- -D warnings`. Both were failing before any Phase 7 work:
a stranded `#[allow(clippy::too_many_arguments)]` and an orphaned doc comment in
`cairnd/src/handlers.rs`, several `sort_by` calls that clippy wants as `sort_by_key`, and unformatted
files across `cairn-core`, `cairn-store`, `cairnd` and the test crate. They are fixed here rather
than carried forward, which is why the diff touches files Phase 7 otherwise has no business in.

## Where the run stands

**75 of 148 tasks complete**, each with its named evidence passing.

| Phase | Tasks | State |
|---|---|---|
| 1 Setup | T001–T005 | complete |
| 2 Foundational | T006–T017 | complete |
| 3 Local migration | T018–T023 | complete |
| 4 Canonical knowledge | T024–T043 | complete |
| 5 Evidence & authority | T044–T059 | complete |
| 6 Drift | T060–T064 | complete |
| 7 Evidence-aware tasks | T065–T075 | complete |
| 8–16 | T076–T148 | not started |

## Next action

**Phase 8 (US10 — minimum-safe context, pins and explainability), starting at T076**: the test-first
cluster T076–T078 — the `tests/knowledge/budget/` corpus across memory populations 0, 10, 500 and
5,000 × budgets 200…12,000 plus `budget/oversized_task/`, then `us10_min_safe_context::budget` as a
property test and `::no_regression` against the T004 baseline — then T079–T085.

Phase 8 consumes Phase 7's output directly: Tier 0a is task work state, and `action_order` is what
Level 0 admits criteria in. Both are in place and tested.

`contracts/continuity-context.md` governs the phase and has **not** been read yet in this run.

### Superseded next-action note

**Phase 7 (US11 — evidence-aware tasks), starting at T065**: the test-first cluster T065–T067
(`no_silent_overwrite`, `attested_is_not_enough` / `no_percentage_field`, and the
`tests/knowledge/tasks/` corpus), then T068–T075.

The pure half is already implemented and tested in `cairn-core::tasks`:
`derive_task_state_digest`, `progress`, `completion_readiness`, `action_order`,
`criteria_projection` and `criterion_label`. Phase 7 is the `task_criteria` and `task_blockers`
repositories, the local counter and change log, the retained projection, criterion verification
restricted to `cairn` authority, the bind snapshot, and the `cairn task` surfaces.

`contracts/task-model.md` has already been read and governs the phase.
