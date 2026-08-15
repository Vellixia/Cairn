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

## Where the run stands

**28 of 148 tasks complete**, each with its named evidence passing. Phases 1–3 are complete; Phase 4
is part-done.

| Phase | Tasks | State |
|---|---|---|
| 1 Setup | T001–T005 | complete |
| 2 Foundational | T006–T017 | complete |
| 3 Local migration | T018–T023 | complete |
| 4 Canonical knowledge | T024–T043 | T024, T027, T029–T031 complete |
| 5–16 | T044–T148 | not started |

## Next action

**T025 and T026** — the two named e2e negatives, `tests/tests/us1_reconciliation.rs` and
`tests/tests/us3_conflict.rs`. Their tier-2 equivalents already pass (`no_automatic_reinforcement`,
`corroboration`, and the clock-inversion unit test in `knowledge.rs`); what is missing is the
end-to-end form against a real store and daemon.

Then T028's `as_of` expectations (which need T036), T032–T038 (explicit reinforce, supersession,
stale_at, search filters, merged-branch elevation, the rebuild-equivalence suite), and T039–T043
(the CLI surfaces and the end-to-end tests that close the phase).
