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
| Commit | see `git log` for `feat(003): corpus scaffolding, the alpha.4 fixture and the pre-feature baseline` |

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

## Next action

Phase 2 (Foundational), starting at **T006**: the Feature 003 domain enums in
`crates/cairn-core/src/domain.rs`. Phase 2 blocks every story phase.

Read before starting: `contracts/evidence-verification.md` (T013's state machine and
`derive_authority`), `contracts/continuity-context.md` (T014's divergence classes and T016's
reserve), `contracts/task-model.md` (T015's digest). `contracts/knowledge.md` has already been read
and governs T009–T012.
