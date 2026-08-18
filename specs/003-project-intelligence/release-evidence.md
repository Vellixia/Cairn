# Feature 003 — release evidence

What was run, what was found, and — for the three tasks that need a live agent
on a real machine — what was **not** run and exactly what running it needs.

A reader should be able to tell "this was run and passed" from "nobody ran
this" without reading anything else.

## Deterministic evidence — RUN, PASSING

| | |
|---|---|
| Suite | `CARGO_INCREMENTAL=0 cargo test --workspace` |
| Result | **1,041 passed, 0 failed, 1 ignored** |
| Database | PostgreSQL 17.10 (`postgres:17-alpine`), the image and credentials CI uses |
| Server suite | **0 skipped** — `CAIRN_TEST_DATABASE_URL` was set, so every server test ran rather than early-returning |
| Gates | `cargo fmt --all -- --check` clean · `cargo clippy --workspace --all-targets -- -D warnings` exit 0 · `git diff --check` clean |
| Platform | macOS / aarch64. CI additionally runs Linux. |

The count rose from the 1,022 of the first pass because the independent review
(implementation-log Checkpoint O) added 17 regression tests and 1 renderer
test — and because the one ignored test is `baseline_capture`, which
regenerates a committed artifact on purpose and is not a skipped check.

**"0 skipped" is the load-bearing detail.** The server tests early-return with a
`SKIPPED:` line when `CAIRN_TEST_DATABASE_URL` is unset, and a run that skips
them still reports every test as passing. The grep for `SKIPPED` returned zero,
so the sync, privacy, offline-merge and degradation suites really executed
against real PostgreSQL.

Tiers 1–4 of `contracts/evaluation.md` §Tiers all run. Tier 5 is live-agent
evidence and is the section below.

### Server migration, against a real PostgreSQL

Run directly against PostgreSQL 17.10 with the real `cairn-server` binary — not
a harness — on a database created for the purpose:

| Step | Observed |
|---|---|
| Held back at schema 1 (`CAIRN_MAX_SCHEMA_VERSION=1`) | server starts; `GET /api/version` reports `schema_version: 1`, `capabilities: []` |
| Upgraded to schema 2 on the **same** database | `schema_version: 2`, capabilities `memory_relations`, `task_criteria`, `task_blockers`, `memory_subject_identity`, `memory_verification` |
| Restarted on the migrated database | `schema_version: 2`, `schema_migrations` still holds exactly 2 rows — the migration is not re-applied |
| Tables present afterwards | `memory_relations`, `task_criteria`, `task_blockers` alongside the Feature 001/002 tables |

```text
1|init|2026-08-18 16:11:46.379631+00
2|project_intelligence|2026-08-18 16:11:46.850166+00
```

A held-back deployment therefore advertises the schema it actually applied
rather than the one its binary was compiled with (FR-415). That migration files
on disk are all registered — the failure mode where a server starts, reports
success and serves a schema missing a whole feature — is asserted separately by
`cairn_server::db::tests::every_migration_file_is_registered`.

### Mixed-version recovery, against real held-back servers

`Server::start_at_schema(1)` spawns the **real server binary** with
`--max-schema-version 1` against real PostgreSQL, so "an old server" means a
server that applied an old schema, which is what makes the test honest. With
`CAIRN_TEST_DATABASE_URL` set, all of these ran:

| Test | What it establishes |
|---|---|
| `no_futile_retry` | refused work is not retried against a server that cannot hold it |
| `never_permanently_failed` | a capability refusal never becomes a terminal failure |
| `release_preserves_identity` | released work keeps its identity, so delivery stays exactly-once |
| `a_partial_upgrade_releases_only_what_it_covers` | a partial upgrade releases only what the new schema supports |
| `recovers_after_upgrade` | refuse, retain, upgrade, deliver exactly once — with `blocked` reaching 0 and no manual repair |
| `older_daemon_newer_server` | the reverse direction needs nothing |
| `the_background_worker_delivers_retained_work_after_an_upgrade` | retained work is drained without a user command |

**The metric table** is emitted with actual numbers by
`cargo test -p cairn-e2e --test metrics -- --nocapture`. Every one of its rows
names a test that exists — asserted mechanically, so the table cannot drift
away from the suite.

**The corpus** holds 341 cases across 24 groups, every group non-empty, every
fixture parsing, and every contract-stated minimum met or exceeded:

| Group | Cases | Contract |
|---|---|---|
| `reconciliation/equivalent` | 22 | ≥20 |
| `reconciliation/distinct` | 22 | ≥20 |
| `reconciliation/coarse_value_key` | 17 | ≥15 |
| `conflict/real` | 16 | ≥15 |
| `conflict/scope_exception` | 12 | ≥10 |
| `conflict/disjoint` | 12 | ≥10 |
| `privacy` | 30 | ≥30 |
| `patterns/refuse` | 12 | ≥10 |

**Performance** is measured against a scaled loaded-project fixture, with a
host-saturation guard: a saturated host reports an **invalid measurement**
rather than a failure, per `contracts/evaluation.md` §Performance measurement.
The population measured is printed beside the stated one, so a number can be
compared to the conditions that produced it.

**Derived-value consistency**: `cairn doctor --rebuild-derived` recomputes all
six derived values — memory lifecycle state, reinforcement counts, verification
state and authority, the criteria projection, the task state digest and pattern
trust — and exits non-zero if any differs. It is asserted to cover all six by
name, and asserted to fail when one disagrees.

---

## Live-agent evidence — NOT RUN

The three tasks below cannot be performed in this environment. Each is written
out so it can be run without re-deriving anything, and each states precisely
what it needs.

**None of the three passes.** One of them — T148 — was **partially run against a
live agent in this environment**, and what it produced was a defect rather than
a confirmation. The status of each is stated exactly below.

> **FEATURE_003_MANUAL_EVIDENCE_PENDING**
>
> | Task | Status |
> |---|---|
> | T146 quickstart walkthrough | **NOT RUN** — blocking |
> | T147 topic-key effectiveness | **NOT RUN** (harness complete) — not blocking |
> | T148 live continuity walkthrough | **PARTIALLY RUN, DOES NOT PASS** — blocking; Claude Code driven live and found over-claiming, Codex and OpenCode not driven |

### T146 — the quickstart walkthrough · BLOCKING · NOT RUN

**What it is.** `specs/003-project-intelligence/quickstart.md` end to end on a
real repository with a live agent, section by section, for all eleven user
stories. The constitution's "runs on a real repository" gate (Constitution I,
VII).

**What running it needs**

- A real Git repository with genuine history — not a fixture.
- One installed, authenticated agent with its native Cairn integration
  (`cairn connect claude-code --yes`).
- A PostgreSQL the machine can reach, for the US7 sections.
- An operator willing to drive an agent session per section and read the output.

**Passes when** every section produces the output it documents.

**Record** the transcript and any deviation, here.

**Status: NOT RUN as the gate.** The eleven sections were not walked end to end
by an agent, so this task does not pass.

**What was exercised, and what it found.** A real Cairn instance was built on a
real Git repository with real hooks installed, and parts of the walkthrough were
driven against it. That partial exercise is not the gate, but it was not
worthless — it found three defects the whole deterministic suite had missed:

| Section | Observed |
|---|---|
| US1/US3 — a conflicted subject | The subject derived `Conflicted` with two answers and a recorded `conflicts_with`, and `cairn context` produced **no warnings at all** (F7). |
| US5 — a drifted memory with no task bound | No drift warning: the query asked about the nil project (F8). |
| The rendered briefing | The payload carried the warnings and the rendering dropped every one, along with pinned constraints and patterns (F9). |

All three are fixed, and the fixes were confirmed against that same live store —
`⚠ CONFLICT deploy.queue_backend` now appears in `cairn context` exactly as the
quickstart documents it.

**Deviations found while walking it** are recorded as D1–D4 in
[implementation-log.md](./implementation-log.md) Checkpoint O — chiefly that
`cairn memory supersede` does not exist as a CLI verb (US2 documents it) and
that `cairn memory add` renders none of the reconciliation outcome it returns.
Those remain open, and a full T146 run will hit them.

**What a passing run still needs.** An operator to drive an agent session per
section, for all eleven, and read the output against the document.

### T148 — the live-agent continuity walkthrough · BLOCKING · NOT RUN

**What it is.** Drive a **real** compaction on Claude Code, Codex and OpenCode
and confirm each one's reported `continuity_mode` matches what actually
happens.

**What running it needs**

- All three agents installed, authenticated and connected natively.
- A session long enough in each to trigger the agent's own compaction — not a
  simulated event.
- For OpenCode specifically: **both** a build that exposes
  `experimental.session.compacting` and one that does not, because its mode is
  `agent_initiated` precisely because that capability is conditional.

**Passes when** each agent's observed behaviour matches its derived mode, and
no agent claims a rehydration guarantee it does not deliver.

**Record** per-agent notes, here.

**What is already known.** The derivation itself is asserted:
`us6_continuity::each_agents_mode_is_the_rule_applied_to_its_capabilities`
checks each agent's mode **and** re-derives it from the two capabilities it
reads, so a change has to be a change to the agent rather than to a table.

This run also corrected an over-claim found by that test: OpenCode reported
`automatic` while its pre-compaction warning is conditional, which meant a
build without the experimental hook would never warn Cairn — and the agent had
been told not to worry. It now reports `agent_initiated`.

What remains unverified is the **observation**: that each agent behaves the way
its capability profile says it does. That is what T148 is for, and it is why
this task is blocking even though the rule is tested.

#### What was run — Claude Code, live · FOUND A DEFECT

A real compaction was driven in Claude Code against a real Feature 003 store:
the agent run headlessly with a reduced auto-compact window over a large file,
with `CAIRN_HOME` and `PATH` pointed at this build, in a real Git repository
with the real hooks installed.

| Observation | Result |
|---|---|
| A real compaction occurred | yes |
| `PreCompact` fired and Cairn wrote a checkpoint | yes — `trigger = context_compacting` |
| The checkpoint was restored | **no — `restore_count = 0`** |
| Reported mode at the time | `automatic` — "continuity is restored automatically after compaction" |

The mode over-claimed, which FR-426 makes a defect rather than a note. Both
causes and the fix are written up as **F10** in
[implementation-log.md](./implementation-log.md) Checkpoint P. Claude Code now
derives `agent_initiated`, which is what it actually delivers.

A second, separate observation from the same run: a `session_closed` checkpoint
was written when the agent's session ended. That is the close-boundary guarantee
("a checkpoint exists at session close and on demand") holding on a live agent —
the one continuity claim verified end to end here.

**T148 still does not pass.** Its pass condition is *each* agent's observed
behaviour matching its derived mode. One agent was driven, and it failed; the
fix is in, and the corrected mode has not itself been re-verified against a
fresh live compaction.

#### What was not run, and why

| Agent | Status | Reason |
|---|---|---|
| Codex | **not driven** | `cairn connect codex` refuses with `resource_modified` against the operator's hand-edited global `~/.codex/config.toml`. That refusal is Feature 002's ownership protection working correctly, and the file was verified untouched. Connecting would mean overwriting a configuration this run does not own. |
| OpenCode | **not driven** | Its integration installs a plugin into the operator's own OpenCode configuration directory. Same reason: this run is not authorized to modify the operator's agent configuration. |

Neither is a Cairn defect, and neither can be worked around from inside this
run: both need an operator who owns those configurations to connect the agent
and drive a compaction.

### T147 — topic-key effectiveness · NOT BLOCKING · NOT RUN

**What it is.** The evaluation at `evals/topic-key-effectiveness/`, run against
Claude Code, Codex and OpenCode.

**Status.** The corpus (26 items across five project archetypes, with paired
and near-miss cases), the protocol and the recording structure are **complete**.
The results are **not collected**.

**What running it needs**

- Three agents, each natively integrated.
- A clean repository per agent, so cross-agent consistency measures convergence
  rather than imitation.
- A fresh session per corpus item: 26 × 3.

**It is explicitly not a gate.** No threshold is defined, it cannot fail a
build, and `ci_hermeticity::no_gating_check_reads_a_models_judgement` asserts
that no workflow references it.

**The one exception, stated honestly.** A non-zero false-grouping count would be
a design defect rather than a number. Since no corpus item has been run, that
count is **unknown, not zero** — the check has not been performed, and nothing
here should be read as it having passed.

---

## Summary

| | |
|---|---|
| Automated tasks (T001–T145) | complete, with evidence |
| Deterministic gates | passing — 1,041 passed, 0 failed, 0 skipped, against real PostgreSQL |
| Inherited Feature 001 / 002 regressions | passing, including against a migrated alpha.4 store |
| Server migration and mixed-version recovery | verified against a real PostgreSQL and real held-back servers |
| Independent adversarial review | **performed this run** — see implementation-log Checkpoints O and P |
| Unresolved CRITICAL / HIGH findings | **none open.** Ten were found and fixed this run: F1–F8 by review, F9–F10 by driving a real agent |
| Open deviations | D1–D4, documentation against implementation — recorded, not fixed |
| **T146 — quickstart walkthrough** | **NOT RUN — needs an operator to drive all eleven sections with a live agent** |
| **T148 — live continuity walkthrough** | **PARTIALLY RUN, DOES NOT PASS — Claude Code driven live and found over-claiming; Codex and OpenCode not connectable by this run** |
| T147 — topic-key effectiveness | harness complete, results NOT COLLECTED — non-blocking. False-grouping count **unknown, not zero** |

Two release-blocking gates have not passed. Until they have, this feature is
implementation-complete and **not** release-ready.

The honest summary of this run is that the earlier "0 unresolved CRITICAL/HIGH"
was true of the tests and false of the code. A separate review found eight
defects in a green branch, and driving one real agent found two more — including
a build that told Claude Code its continuity was automatic and then dropped it
silently. That is the argument for tier 5 in one sentence, and it is why T146
and T148 remain blocking rather than being written off as formalities.
