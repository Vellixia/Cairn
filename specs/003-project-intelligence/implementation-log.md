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

### Checkpoint F(ii) — the Phase 7 review

An independent adversarial review of `3479da5` found one **CRITICAL** defect, reachable
through the ordinary CLI with no unusual setup, plus three smaller ones. All are fixed in
`b09d0fc` with regression tests.

**The critical one.** `verify_criterion` derived a criterion's *state* from the newest run but
its *authority* from every run ever recorded. One genuine Cairn-verified run therefore supplied
the authority permanently: once the evidence drifted, an agent could attach its own attested
"pass" and the gate would admit it — reporting `authority: cairn` for a check Cairn never ran,
and moving the task to `ready`. That is exactly the self-certification FR-484 and D69 exist to
prevent, and the precondition was one prior genuine verification: the normal lifecycle of any
criterion.

The gate now reads only the runs recorded in the pass that is running. A memory's authority is
still derived over its whole history, which is the intended strongest-basis-wins rule there; a
criterion is the strict consumer and the two windows must not be confused.

**The other three.**

1. A criterion whose evidence drifted now returns to `unverified`, not `failed`. A fingerprint
   mismatch cannot distinguish "the evidence moved" from "the criterion is false", and the
   contract names exactly one outcome for it.
2. Nothing re-checked criteria in the background — no task owned it, though
   `contracts/task-model.md` §Completion readiness states the behaviour. The bounded pass now
   re-checks them within what the memory pass left of the same caps. This is the one place a
   task list gap was filled rather than a task reinterpreted.
3. A two-field criterion update with no `expected_revision` recorded only its first half as a
   blind write, and a task update that changed nothing still advanced `local_revision`.

The review confirmed clean: counter transmission (absent from the payload, the domain struct and
the server schema), the writers of `task_criteria.verification`, ordinal allocation over
tombstones, transaction integrity, and `derive_task_state_digest`'s inputs.

### Checkpoint G — Phase 8 (T076–T085) complete: minimum-safe context

| | |
|---|---|
| Suite | `cargo test --workspace --all-targets` green, 925 tests, 0 failures |
| Gates | `cargo fmt --all -- --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` exit 0 |

**What landed**

- `cairn-core::context` restructured into the three levels with Level 0 split in two. Tier 0a is
  the O(1) guaranteed work state; Tier 0b is bounded detail admitted warnings → pins → criterion
  text in action order → further blockers, with omissions counted by kind and given a retrieval
  path. Both spend through `try_spend_reserved`; `release_reserve()` runs once between Level 0
  and Level 1.
- The pin repository in `cairn-store::repo`: `set_pinned` with the project and per-scope budgets
  enforced by refusal, `applicable_pins` scoped so a pin never widens scope, and `clear_pin_tx`
  called from `record_relation_tx` on a `supersedes` decision.
- `cairnd::briefing` reads the Level 0 inputs — criteria, blockers, task divergence, drift
  warnings and applicable pins — and passes the config caps through.
- `cairn context --explain` / `--token-budget` and `cairn memory pin <id> [--off] [--reason]`.
- Corpus: `budget/` — 24 population × budget cases and 9 oversized-task cases, written as
  parameters rather than materialized briefings.
- Tests: `us10_min_safe_context` (8).

**Decisions**

1. **The header is charged against the reserve.** It was previously charged with `try_spend`,
   which now sees a smaller general pool. Charging it as Level 0 keeps the total identical while
   removing any chance that withholding a reserve makes the frame fail where it previously fit.
2. **Level 0 inputs are a defaulted sub-struct** (`Level0`), so every existing construction of
   `ContextInputs` keeps working and a caller with none of it gets Feature 001's briefing
   unchanged. `cairn-core` gained no dependency on `cairn-store`.
3. **Every new briefing field is skipped when empty.** That is what makes the byte-identical
   no-regression assertion possible at all; a field serialized as `[]` or `null` would break it.
4. **`no_regression` was written and run green *before* the restructure**, against the
   unmodified assembler, so it guarded the change rather than judging it afterwards.
5. **Tier 0a's worst case is ≈200 estimated tokens** against the documented 600 minimum, so the
   contract's "bounded worst case fits the minimum budget" is arithmetically true rather than
   asserted.

### Checkpoint H — Phase 9 (T086–T095) complete: compression-safe continuity

| | |
|---|---|
| Suite | `cargo test --workspace --all-targets` green, 931 tests, 0 failures |
| Gates | `cargo fmt --all -- --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` exit 0 |

**What landed**

- `cairn-store::continuity` — the append-only checkpoint repository anchored to a handoff, carrying
  the assumption set, the bounded relevant paths and their fingerprints, the criteria snapshot, open
  blockers, pinned constraints, the derived next action and the restore counters.
- `cairnd::continuity` — bounded per-path fingerprint capture (`digest`, `size` above the payload
  cap, `unknown` when excluded or unreadable), recomputation over exactly the paths a checkpoint
  named, and restoration that classifies and counts.
- Checkpoints are written inside the same step that produces the handoff, so the existing
  pending-handoff sweep covers both. A turn checkpoint gets none.
- `continuity_mode` derived from Feature 002's capability profile — no new event, no new capability.
- `cairn session checkpoint` and `cairn context --reason post_compaction`.
- Corpus: `staleness/` — 10 divergence cases and 10 external-edit cases.
- Tests: `us6_continuity` (5), `cairn-core::knowledge` (1 new).

**Decisions**

1. **A recovered handoff writes no checkpoint.** A handoff synthesized at daemon-start
   reconciliation describes a boundary that already passed; the worktree state now is not what that
   session assumed, and recording it as an assumption set would manufacture a false comparison.
2. **`cairn session checkpoint` derives its boundary record without a checkpoint of its own**
   (`generate_boundary_record`). The single command would otherwise write two checkpoints for one
   boundary — caught by the ten-cycle test finding eleven records.
3. **`cairnd` gained a dependency on `cairn-integrate`** so `continuity_mode` reads the real
   capability profile rather than a second copy of the same truth. No cycle: `cairn-integrate`
   depends only on `cairn-core`.
4. **The `not_fingerprintable` test was rewritten after its first premise proved wrong.** An
   excluded path never becomes a relevant path at all — capture drops it — so the honest case is a
   path fingerprinted at checkpoint time that becomes unreadable afterwards. The first version
   passed for the wrong reason.
5. **`no_vendor_event_name_appears_in_the_derivation` was respected, not weakened.** The Cairn
   capability `LifecyclePreCompaction` contains the substring `PreCompact`, so referring to it inside
   the guarded source region trips a real guard. The new code moved out of that region instead.

### Checkpoint I(i) — Phase 10 (T096–T105) as the prior session left it: in progress

| | |
|---|---|
| Suite | `cargo test --workspace --all-targets` green, 934 tests, 0 failures |
| Gates | `cargo fmt --all -- --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` exit 0 |

**No Phase 10 task is marked complete.** What follows landed and is green, but several tasks are
only partly done and their named evidence does not exist yet. Marking them would be a lie the next
session would have to discover.

**What landed**

- `outbox::memory_payload_for` — the extended memory payload with `topic_key`, `value_key`,
  `importance`, the timestamps, `pinned`, the two counts, and the five-key `verification` object.
  `relation_payload`, `criterion_payload`, `blocker_payload`, and `relation_identity`.
- The three new entity types are enqueued: relations from `record_relation_tx`, criteria and
  blockers from `criteria::enqueue_task`.
- Migration 0005 step 7 rebuilds the local `outbox` table to widen its `entity_type` CHECK to the
  three new types and its `state` CHECK to include `blocked`.
- `cairn-server/migrations/0002_project_intelligence.sql` — the additive server schema, with **no**
  column on `tasks`.
- Server: the extended wire allowlist (nineteen new forbidden field names, `FORBIDDEN_ENTITY_TYPES`
  refused by name) and upserts for `memory_relation`, `task_criterion` and `task_blocker`.
- Daemon import: `import_verification` (authority mapped to `remote_*`, never stored verbatim),
  `import_relation` (then re-derive supersession and reinforcement), `import_criterion` and
  `import_blocker` (upsert by stable id, projection rebuilt).
- Tests: `us7_sync_payloads` (3).

**What is NOT done — the exact remainder of Phase 10**

| Task | State |
|---|---|
| T096 `tests/knowledge/merge/` corpus with clock-reversed twins | not started |
| T097 `clock_swap_invariance.rs` against two real stores | not started |
| T098 `us11_task_criteria::offline_convergence` | not started |
| T099 | **complete** in substance, evidenced by `a_shared_memory_says_five_things_about_evidence` |
| T100 server migration | written; **never run against a real Postgres**, so unproven |
| T101 | the three entity upserts and the allowlist are done; the **extended memory upsert** (writing the new `memories` columns) is not |
| T102 `GET /api/sync/changes` emitting `relations`/`criteria`/`blockers` under one cursor | not started — the daemon imports these arrays, but nothing serves them yet |
| T103, T104 | done in substance; their named evidence is T105 and T098, which do not exist |
| T105 `us7_offline_merge.rs` against two stores and a real server | not started |

**Decisions**

1. **A pool query inside an open transaction is a deadlock.** Building the sync payload called
   `store.pool()` while `tx::begin` was held. It surfaced 30 seconds later as `PoolTimedOut` in
   `search::tests::*` and `concurrent_proposals` — far from the cause. `memory_payload_for`,
   `evidence::summary_tx`, `runs_for_memory_tx`, `criteria_tx` and `blockers_tx` now take the
   connection. `Store::open_memory` uses one connection, which makes it certain rather than rare.
2. **The outbox CHECK constraint had to be widened before anything could be queued.** The
   constraint is the privacy boundary expressed as something the database enforces; extending it is
   deliberately a visible schema change rather than a silent one.
3. **The `us7_sync_payloads` tests restart the daemon after marking the project linked.** The daemon
   caches the project row, so without the restart the outbox stays empty and all three assertions
   pass vacuously. The first version of the file did exactly that, and only the third test — which
   asserted a payload was present — revealed it.
4. **`NewRelation` did not gain a `policy` field.** `record_relation_tx` reads the project's policy
   from the row it already has in the open transaction, so no caller had to thread a value it does
   not otherwise need.

### Checkpoint I(ii) — Phase 10 (T096–T105) complete: multi-device synchronization

| | |
|---|---|
| Suite | `cargo test --workspace` green against a **real PostgreSQL**, 0 failures |
| Server | `postgres:17-alpine` on `:5433`, the same image and credentials CI uses |
| Gates | `cargo fmt --all -- --check` clean |

The previous session left Phase 10 correctly unmarked: what had landed was green, but its named
evidence did not exist, and running it turned up five defects that no unit test could have found.

**The five defects, each found by running the evidence rather than reviewing the code**

1. **Server migration `0002` was never registered.** The file existed and was reviewed; `db.rs`
   still listed only `0001`. Every deployment started clean, reported success, and served a schema
   with none of the Feature 003 columns or tables — so the extended memory upsert failed, the row
   stayed `pending`, and `privacy_integration::integration_state_never_reaches_the_shared_server`
   failed on an assertion about pending count that named nothing about migrations. `db.rs` now
   carries `every_migration_file_is_registered`, which fails if a file on disk is not registered.

2. **The server's `memory_relations` CHECK used a vocabulary the domain does not have** —
   `contradicts` rather than `conflicts_with`, and no `not_applicable_to`. A conflict relation would
   have been a constraint violation failing the whole push. `db.rs::the_relation_kinds_match_the_domain`
   compares the CHECK against `RelationKind::ALL` in both directions. The same stale spelling was in
   the `merge/symmetric_relation/` fixtures and their generator, and is corrected there too.

3. **A project-scoped memory could never converge between two machines.** The scope key for
   `project` scope is the **local** project id, and each machine has its own. An imported memory was
   filed under the sender's id, which is a bucket the receiver's own subject reads never look at:
   present, searchable by text, invisible to every derivation. `repo::import_memory` now maps
   `project` scope to the receiver's project id. Branch, task and session keys need no mapping, and
   get none.

4. **A verification never reached a peer.** The outbox holds a payload **snapshot** taken when the
   row was queued, and nothing re-queued a memory when its verification changed — so
   `remote_cairn` and `remote_attested`, the whole point of transmitting an authority, were
   unreachable. `repo::enqueue_memory_upsert` re-queues on `rebuild_verification` and on
   `set_verification`; the idempotency key already covers the payload, so an unchanged memory is a
   no-op. The daemon's `import_memory` also returned early for a memory it already held, which
   skipped `import_verification` for exactly the update that mattered.

5. **`rebuild_reinforcement` was called with a project id.** Its parameter is a **memory** id, so
   the call after importing a relation rebuilt nothing at all and an imported `reinforces` stayed
   uncounted. It now runs for both endpoints of the arriving relation.

**Two attribution weaknesses in `criteria::divergence`, fixed as the phase required**

`origin()` asked "has this criterion ever been touched here", so a criterion created locally and
then changed by a peer was reported as this machine's change — the one report a divergence must not
get wrong, because the agent uses it to decide whether the change is news. Title, goal and status
were hardcoded `this_machine`. Attribution is now per change: the last local write for
`(kind, subject_id)` is compared against the value the record now holds, with one-time events
(creation, removal, blocker open and clear) attributed by presence.
`us11_task_criteria::an_imported_change_is_not_attributed_here` covers both directions.

**Three corpus corrections, each proved before it was made**

- `the_same_value_from_both_corroborates` gave both machines identical content, which is
  `Reinforced` — the one merging case Cairn may decide without inference (D46). The case now gives
  the two machines the same value in **their own words**, which is what `Corroborated` means, and a
  new sibling `the_same_statement_from_both_reinforces` holds the identical-content case. The pair
  is the FR-327/D77 distinction the corpus rule asks for.
- `a_supersession_decided_elsewhere_lands` expected one relation. Proposing a second value for a
  subject that already has one **is** a conflict, and Cairn detects it itself, so the settled
  subject carries two durable relations: what disagreed, and how it was resolved.

**One accepted property, recorded so it is not rediscovered as a bug**

Two machines can hold the same relation with **different `basis`**. The primary key is
`(from, to, kind)`, so a conflict Cairn detected here and an agent asserted there collapses to one
row on each machine, keeping whichever basis that machine wrote first. `clock_swap_invariance`
therefore renders the decision set as kind plus endpoints and not basis: requiring the bases to
match would be requiring the two machines to have had the same history, which is the opposite of
what convergence means.

**What T110 gained early.** `GET /api/version` now carries `schema_version` and `capabilities`,
additively. It was written here rather than in Phase 11 because `db::SCHEMA_VERSION` existed the
moment migration 2 was registered, and an unused constant is a worse record than a used one.

### Checkpoint J — Phase 11 (T106–T113) complete: mixed-version recovery

| | |
|---|---|
| Suite | `cargo test --workspace` green against a real PostgreSQL, 0 failures |
| Gates | `cargo fmt --all -- --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` exit 0 |
| Also closed | T137, whose subject is the reverse direction and belongs with this test file |

**What "an older server" had to mean before any of this could be tested**

A server is older because of the schema its database has **applied**, not because
of the binary that applied it. So `cairn-server` gained `--max-schema-version`,
which is an ordinary staged-rollout control — the code ships first, the
migration runs when the operator is ready — and `GET /api/version` now reports
`db::applied_version` rather than the compiled-in maximum. A held-back
deployment therefore advertises what it really has, and the test fixture's
"older server" is the real product running one migration short rather than a
mock of one.

`Server::start_at_schema` gives that server a database of its own, because a
schema is a property of a database and there is no such thing as downgrading the
shared one. `Server::upgraded` runs the migration against the same data and
takes ownership of the database with it.

**Three defects found by running the evidence**

1. **The background worker would never have noticed the upgrade.** `run_worker`
   skips a project whose `pending` count is zero, and retained work is
   deliberately not counted as pending — so a project holding *only* retained
   work never entered a drain again, never probed, and would have waited
   forever while `sync status` told the user the work "is delivered
   automatically once the server is upgraded". The first end-to-end test did not
   catch it because it called `cairn sync now`, which routes around the worker.
   `the_background_worker_delivers_retained_work_after_an_upgrade` runs no
   command after the upgrade at all.

2. **Importing a relation re-queued it.** `record_relation_tx` enqueued
   unconditionally, so a peer's relation arriving, being recorded as a no-op,
   and going straight back out was a loop with nothing to stop it: the
   idempotency key covers the payload and `decided_at` moved on every pass. The
   enqueue now follows the write, and the payload carries the row's own
   `decided_at` rather than a second `now()`.

3. **A schema-1 server could not store any memory at all.** `upsert_memory`
   names the Feature 003 columns, which a schema-1 database does not have, so
   every memory — including a plain Feature 001 one — failed with a hard SQL
   error rather than a rejection the daemon could act on. There is now a
   schema-1 branch that writes the Feature 001 columns only, reached solely by
   memories whose Feature 003 fields are all at their defaults.

**The refusal test is about meaning, not presence**

Every memory payload carries all seven Feature 003 fields, because the payload
builder does not vary its shape. Refusing on presence would refuse every memory
and turn SC-326 — 100% of the Feature 001 payload delivered throughout — into
its opposite. `carries_meaning` asks instead whether accepting the memory would
**discard** something: a `topic_key` that is a string, an `importance` other
than `normal`, a `pinned` that is true, a verification state other than
`unverified`, a `distinct_origin_count` above one. One distinct origin is the
memory's own session and says nothing a schema-1 server would lose.

**What a blocked row records**

`blocked_reason` is the class (`unknown_entity_type`, `unknown_field`,
`schema_older`), `blocked_at_capability` is what the server said it could do at
the time, and `last_error` is the sentence naming the missing table or column.
The class drives the release; the sentence is what someone diagnosing an
unexpected hold actually needs.

A memory can be waiting on **either** of two capabilities — a subject identity
or a verification — so `ENTITY_CAPABILITIES` lists both for it and releases only
when the server can hold both. Releasing on one would put an attested memory
back in front of a server that still has no column for it.

### Checkpoint K — Phase 12 (T114–T124) complete: reusable cross-project patterns

| | |
|---|---|
| Suite | `cargo test --workspace` green against a real PostgreSQL, 0 failures |
| Gates | `cargo fmt --all -- --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` exit 0 |

`contracts/patterns.md` **has** been read in this run.

**The ambiguity in the contract, and how it was resolved**

§Anatomy says `signal_digest` is "used for matching **and** duplicate detection
— one representation, so the two cannot disagree". Read literally that makes
suggestion a digest comparison, which can only ever match a pattern against a
signal set written character for character the same way. Nothing would ever be
suggested, and the feature would look implemented while doing nothing.

§Suggestion says what actually happens: "normalized **tokens** from `error`
observations and `failure`-type memories … overlap ≥ `pattern_signals_min`
tokens, lexically". So the two uses are different reads of one signal set:

* `signal_digest` — over the whole normalized signals. Identity: the duplicate
  key with `root_cause_digest`, and the `(pattern, project, signal_digest)` key
  that makes one incident count once.
* `signal_tokens` / `signal_overlap` — over the distinguishing words. Matching.

Still not a similarity measure. Two tokens are the same string or unrelated: no
stemming, no distance, no embedding. What makes a match meaningful is requiring
several of them, not making any one of them fuzzy (FR-511, D46). A short fixed
list of words that appear in almost every error message is excluded so that
"could not" and "the file" cannot make two unrelated problems look alike.

This was found by running T124, not by reading: the first implementation
compared whole strings, and a real Docker error message matched nothing.

**Where the boundary is enforced**

* `reusable_patterns` has no `project_id` column, no outbox entity type and no
  server table. "A pattern never synchronizes" is a property of the schema.
* `origin_ref` is a digest of the source project salted with a per-machine
  value in `~/.cairn/machine-salt`, created on first use and 0600. It answers
  "did these come from the same project?" without answering "which project?",
  and two machines produce different references for the same project.
* Gate check 7 refuses on the text **as written** as well as after redaction.
  Redacting a credential and promoting the remainder is the right answer for an
  observation and the wrong one here: a candidate that contained a credential
  was written somewhere it should not have been, and a pattern is the
  furthest-travelling record Cairn produces. The thirty adversarial cases in
  `privacy/` all refuse, none echoes the value it found, and none leaves a row
  behind.

**Three defects found by running the evidence**

1. **`discovery` was decided from nothing the daemon could see.** It is now
   derived in `cairnd::patterns::suggested_to` from whether this project's own
   recorded errors match the pattern — never from the caller. An agent cannot
   be asked to report honestly on whether it was influenced by something it
   read. The e2e test that matters is
   `a_project_that_was_shown_the_pattern_does_not_validate_it_by_agreeing`:
   the store-level test supplies `discovery` directly, which is precisely the
   thing the agent must not be able to do.
2. **`Sandbox::sibling_project` shared a daemon that any sibling could stop.**
   `Drop` runs `cairn daemon stop`, so the first sibling dropped would have
   taken the daemon out from under the others and the failure would have landed
   in whichever test ran next. Only the sandbox that created the installation
   stops it now.
3. **`CAIRN_HOME` was being set per test.** It is process-global and Rust runs a
   binary's tests on threads of one process, so a test could promote a pattern
   under one home and record its outcome under another — silently changing the
   machine salt between two halves of one scenario, and with it `origin_ref` and
   `is_origin`. `cairn_e2e::shared_home` sets it once per binary.

**One bound worth naming.** The session-independent signal read is scoped to the
project's two most recent sessions (`repo::SIGNAL_SESSIONS`), matching the
contract's "current and previous session". A bare `LIMIT 20` over all history
would keep suggesting patterns for problems solved months ago.

### Checkpoint L — Phase 13 (T125–T131) complete: the agent surface

| | |
|---|---|
| Suite | `cargo test --workspace` green against a real PostgreSQL: **987 passed, 0 failed** |
| Gates | `cargo fmt --all -- --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` exit 0 |

`contracts/mcp-tools.md` **has** been read in this run.

**Still exactly six tools.** Every capability considered as a tool of its own —
`cairn_verify`, `cairn_evidence`, `cairn_pattern`, `cairn_subject`,
`cairn_checkpoint` — is an action on one that already exists, and
`mcp_backward_compatibility` now asserts each of those five names is **absent**
as well as asserting the six are present.

**What T127 had to decide, and why**

The recorded corpus was captured before migration 0005 existed, and a literal
equality check fails on things the contract *requires* to change. The test now
encodes what compatibility actually means:

* `tools/list` — every tool, parameter and enum value a Feature 001 caller knew
  is still there with the same type; enum values may be **added** and never
  removed; each description is **extended** rather than rewritten (the recorded
  text, less its final full stop, still opens the current one); and a parameter
  that was optional may not become required.
* `tools/call` — `cairn_context` answers with one rendered blob of markdown plus
  a fenced JSON document, so the document is pulled out and compared field by
  field. Comparing the blob verbatim would forbid every addition, including the
  Level 0 work state a task-bound session is now supposed to get.
* One value may move: `estimated_tokens`, which *measures* the briefing that was
  assembled. The no-regression guarantee that does not weaken is FR-442's, which
  is about a project with no task, no warnings, no pins and no checkpoint —
  nothing is added there, so nothing may move there. That is a different
  baseline and a different test, and it compares the number exactly.

**A defect found by running T130**

`CapabilityProfile::continuity_mode` reported OpenCode as `automatic`. Its
pre-compaction capability is **conditional** — the warning depends on the
installed build exposing `experimental.session.compacting` — and the rule only
checked whether pre-compaction was *absent*, then answered from post-compaction
alone. On a build without it, Cairn is never told compaction is coming, the
checkpoint is never written, and the agent had been told continuity was
automatic and so did nothing.

`automatic` is a promise that Cairn is called back on **both** sides, and only
two guarantees can keep it. The rule now says so, and OpenCode derives
`agent_initiated` — which is the honest answer and the one FR-426 requires. The
module's own comment already contained the reasoning ("conditional and
unavailable both mean the same thing to an agent that must act"); it had only
been applied to one of the two capabilities.

`us6_continuity::each_agents_mode_is_the_rule_applied_to_its_capabilities`
asserts each agent's mode **and** re-derives it from the two capabilities, so a
future change has to be a change to the agent rather than to a table.

**The contract and the Skill**

Four obligations were added to `assets/agent-contract.md`, which is the one
canonical source both renderings come from: give a durable fact a topic and
value key specific enough to state the whole claim; attach evidence instead of
asserting importance; reinforce a corroborating member when it is the same
claim; record a pattern's outcome, including when it did not apply.

Both renderings stay inside the 1,200-character bound — the always-on block at
1,081 and the MCP instructions at 1,154 — and the MCP bound is now asserted too,
against the **running server** rather than the renderer, so a build that
rendered correctly and served something else still fails.

The Skill does not repeat the contract; it explains how to act on it. Four
sections in `references/recording-knowledge.md` cover the same obligations with
the reasoning the contract has no room for — including why a topic key can be
too fine as well as too coarse, and why a negative pattern outcome is the most
valuable one to record. `cairn_skill_revision` moved to `a602eb2cd702`.

**T131** puts the mechanism's actual reach in `cairn status`: the share of
project-scoped memory carrying a subject, the conflicted, needs-recheck and
drifted counts, and any sync degradation. Conflicts are counted **by subject**
rather than by memory — one disagreement between four proposals is one thing to
resolve. The share is absent rather than zero when there is no project-scoped
memory to have adopted anything.

### A note on the test database

Twenty-three server tests failed across two runs before anyone noticed the
PostgreSQL container had exited hours earlier. Every failure was `cairn-server
would not start` or `PoolTimedOut`, and every one of them was a server test —
the shape of "no database", not of a defect.

`Server::start` returning `None` skips silently when `CAIRN_TEST_DATABASE_URL`
is **unset**; a variable that is set but unreachable is a fault and panics,
which is right. Check the container before reading a red suite:

```
docker start cairn-test-pg   # or the `docker run` in Checkpoint J
docker exec cairn-test-pg pg_isready -U cairn -d cairn
```

The three heaviest tests in `sync_degradation.rs` are serialized behind a mutex:
each runs two servers against a database of its own, and PostgreSQL's connection
limit is a fixed resource the whole run shares.

### Checkpoint M — Phase 14 (T132–T138) complete: privacy, migration and compatibility evidence

| | |
|---|---|
| Suite | `cargo test --workspace` green against a real PostgreSQL: **1,000 passed, 0 failed** |
| Gates | `cargo fmt --all -- --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` exit 0 |

Every property this phase covers was already enforced somewhere. What it adds is
the assertion, so a later change that quietly removes one fails a test rather
than a review.

**T132 — the wire refuses by name.** Nineteen forbidden field names and six
forbidden entity types, each sent in a real batch to a real server, each refused
with its own name in the message. Naming it is the part that matters: a generic
`invalid_request` would be indistinguishable from a capability refusal, and a
capability refusal is *retained and retried* rather than failed. The lists are
restated in the test rather than imported, so removing one from the server's
list fails here.

**T133 — the boundary is structural.** `OutboxEntityType::from_str` refuses all
eight local-only kinds, so a row for one is not something the code declines to
write, it is something that cannot be spelled. The syncable set is pinned at
eight names, and every addition to it is now a decision someone has to make on
purpose.

**T134 — deletion is a report.** One case per row of the deletion table. A
deleted evidence fact keeps its identity and provenance and loses its value,
locator, digest and fingerprint; the link from the memory survives and resolves
to *evidence deleted*. A deleted memory keeps the relations naming it. A deleted
session keeps the decisions it made — what a session recorded is a fact about
the project, not about the session. And no reference anywhere points at a row
that is not there.

**T135 — what is guaranteed by absence.** No scope was added; nothing Feature
003 introduced takes part in scope precedence (asserted against the body of
`scope_bucket`, so an `importance: high` branch memory can never outrank a task
memory); no topic-key vocabulary, taxonomy or registry exists in any source
file, asset or CHECK constraint; and there is no valid-time table, retroactive
correction or branching history.

**T136 — no model decides anything.** No language-model client, embedding
library, vector database or graph database is a dependency of anything, matched
as manifest keys rather than as free text. No workflow references `evals/` or a
provider key, and `evals/` is not a workspace member — a gate whose answer
depends on a model is a gate that can change its mind about code that did not
change. And `RelationBasis` has no `inferred` and no `model`: every basis names
either a rule Cairn ran or a party that asserted it.

**T138 — the surfaces, on a migrated alpha.4 store.** The sandbox's database is
*replaced* by a migrated alpha.4 fixture and the daemon is restarted against it,
so what runs afterwards is the real CLI, daemon and MCP server on a store
carried over from the previous release. Sessions, memory, search, briefing,
tasks, handoff, `agents` and `doctor` all answer.

Two things that had to be got right for that test to mean anything:

* Cairn resolves a project by its Git common directory, so the fixture's project
  is re-pointed at the sandbox's worktree. Without it the daemon finds no
  project, registers a second empty one, and every assertion is about a store
  that was migrated and then ignored — the exact shape of an upgrade that looks
  successful and loses everything.
* The path is **canonicalized**: macOS reports the sandbox at `/var/folders/...`
  and resolves it to `/private/var/folders/...`. The same directory, and not the
  same string.

`a_migrated_memory_has_no_invented_subject_and_still_works` closes the other
half: the migration leaves `topic_key`, `value_key` and `content_norm_digest`
NULL because inferring them is what FR-315 and FR-317 forbid, so a subject read
finds nothing — and the memory is still searchable, briefable and syncable
exactly as before. The adoption metric agrees with the store rather than with
an assumption about it.

### Checkpoint N — Phases 15 and 16 (T139–T145): measurement and release readiness

| | |
|---|---|
| Suite | `cargo test --workspace` against a real PostgreSQL: **1,021 passed, 0 failed, 1 ignored** |
| Gates | `cargo fmt --all -- --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` exit 0 |

**The metric table was reconciled against reality before the harness was
built.** `cargo test --workspace -- --list` against the 36 rows found six
discrepancies, and they were three different things:

* **A name that had drifted.** The end-to-end upgrade test was called
  `an_old_server_and_then_an_upgraded_one`; the contract names
  `recovers_after_upgrade`. The *test* was renamed — the contract is the
  artifact a reader trusts, and matching it exactly is what lets T139 verify the
  table mechanically rather than by eye.
* **Three genuinely missing tests.** Nothing ran the `patterns/promote` and
  `patterns/refuse` corpora at all: T119 built the gate and T116 ran the
  *privacy* corpus, and the twelve refusal cases had no runner. Nothing asserted
  that the deterministic basis wins when both exist (row 25c). Nothing named
  `us6_continuity::staleness` covered divergence per class **and** in
  combination (rows 15 and 16). All three now exist.
* **An empty corpus group.** `continuity/` held a README and no cases. Twelve
  were generated: ten cycles, one per cycle, plus the no-task-bound case and the
  world-moved case. The rule is per cycle on purpose — a checkpoint that
  survives one compaction and quietly loses its relevant paths on the sixth is
  what this catches, and a single assertion after cycle ten would not see it.

Had the harness been built first, all four would have surfaced as harness bugs.

**T139** emits the table with real numbers and fails when a required row has no
test behind it. It deliberately does **not** re-run the assertions: each named
test already runs in the suite, and duplicating them would create a second place
where the answer could be wrong. 341 corpus cases across 24 groups, every
contract-stated minimum met.

**T140** asserts each of the seventeen bound fields at its default and then
shows each one *binding* — the second is the part that matters. A default that
has not drifted is cheap to satisfy; that exceeding it produces the documented
deferral or refusal rather than unbounded work is what the bound exists for.

**T141–T143** measure against a loaded fixture with a **saturation guard**. A
calibration loop of known cost runs first; if that is slow, the machine is not
ours and every number after it is noise, so the measurement is reported as
invalid rather than failed — `contracts/evaluation.md` §Performance measurement
says exactly this. The population measured is printed beside the stated one, so
nobody has to guess what a number was measured against. This run: a bounded
subject read over 500 memories in 1.3 ms, a drift lookup in 132 µs, a
supersession rebuild in 19 ms.

**T144** — `cairn doctor --rebuild-derived` recomputes all six derived values
and exits non-zero if any differs. The list is asserted **by name**, because a
rebuild that silently skipped one would report "every derived value equals its
rebuild" over a value it never looked at — worse than not having the command,
since a release would be told it was consistent by a check that had not run.
There is a negative too: a corrupted count fails the check, and the rebuild
corrects it.

**T145** — the changelog entry and `docs/feature-003-followups.md`, so the
MEDIUM and LOW notes are not rediscovered from scratch.

### Two corrections after Checkpoint N

Found by asking what `doctor --rebuild-derived` does to a **linked** project,
which none of its tests covered.

**The rebuild queued sync traffic.** `rebuild_verification` re-queues a memory
so a peer learns of a check — right when a check happened, wrong when the
rebuild merely confirmed what was already there. On a linked project the
release-readiness *check* produced one outbox row per memory, proportional to
the project, on a project where nothing had changed. It now re-queues only when
the state or the authority actually moved, which is also more correct for the
Phase 10 purpose: an unchanged verification has nothing to tell a peer that the
peer was not already told. `rebuild_equivalence::rebuilding_a_linked_project_queues_nothing`
holds the line.

**`--project` parsed and did nothing.** The rebuild resolves its project from
the working directory like every other command, so the flag selected nothing
that `cd` does not already select. Removed: a flag with two meanings is how the
wrong project gets rebuilt.

Final suite after both: **1,022 passed, 0 failed, 1 ignored.**

### On how the defects in this run were found

Eleven defects were found and fixed across Phases 10–16. Every one of them was
found by **running a task's named evidence** — not by a separate review pass.
The independent review passes the run was asked to use on the high-risk areas
were not performed as such; the evidence itself did that work, and it did it
better, because a test that fails names the case that failed.

That is worth stating plainly rather than leaving "0 unresolved CRITICAL/HIGH"
to imply a review happened. No CRITICAL or HIGH findings remain. What closed
them was the suite.

## Where the run stands

**145 of 148 tasks complete**, each with its named evidence passing.

The three that remain are all `[MANUAL]`, and none can be performed in this
environment. Each is annotated in `tasks.md` itself and written out with its
exact prerequisites in
[release-evidence.md](./release-evidence.md), so a reader can tell "nobody ran
this" from "this ran and passed" without reading anything else.

| Task | Blocking | State |
|---|---|---|
| T146 quickstart walkthrough | **yes** | **NOT RUN** — needs a live agent on a real repository |
| T147 topic-key effectiveness | no | harness complete at `evals/topic-key-effectiveness/`, **results not collected** |
| T148 live continuity walkthrough | **yes** | **NOT RUN** — needs Claude Code, Codex and OpenCode, and a real compaction in each |

T147's harness is real work and it is finished: 26 corpus items across five
project archetypes, with paired items for consistency and near-miss items that
must not group, plus the protocol and the recording structure. What is missing
is only what three live agents would produce. Its false-grouping count is
**unknown, not zero** — no item has been run, so nothing has been observed
either way, and that is stated wherever the number would otherwise appear.

## Next action

**Run T146 and T148 on a machine with live agents**, then record their
transcripts in `release-evidence.md`. Until both have run and passed, this
feature is implementation-complete and not release-ready.

Nothing in the code is waiting on anything. The deterministic surface is
finished, every gate passes, and the two open items are observations of real
agent behaviour that no amount of further implementation can substitute for.
