# Feature 003 — release evidence

What was run, what was found, and — for the three tasks that need a live agent
on a real machine — what was **not** run and exactly what running it needs.

A reader should be able to tell "this was run and passed" from "nobody ran
this" without reading anything else.

## Deterministic evidence — RUN, PASSING

| | |
|---|---|
| Suite | `cargo test --workspace` |
| Result | **1,000+ passed, 0 failed, 1 ignored** |
| Database | PostgreSQL 17 (`postgres:17-alpine`), the image and credentials CI uses |
| Gates | `cargo fmt --all -- --check` clean · `cargo clippy --workspace --all-targets -- -D warnings` exit 0 |
| Platform | macOS / aarch64. CI additionally runs Linux. |

Tiers 1–4 of `contracts/evaluation.md` §Tiers all run. Tier 5 is live-agent
evidence and is the section below.

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

**None of them has been run. None is recorded as passing.**

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

**Why it cannot be run here.** No live agent is attached to this environment,
and the walkthrough's whole purpose is to exercise the agent-facing path rather
than the CLI that stands behind it. Simulating it with CLI calls would produce
a transcript that proves the thing the automated suite already proves, and
would not prove the thing this gate exists for.

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
| Deterministic gates | passing |
| Inherited Feature 001 / 002 regressions | passing, including against a migrated alpha.4 store |
| Unresolved CRITICAL / HIGH findings | none |
| **T146 — quickstart walkthrough** | **NOT RUN — needs a live agent on a real repository** |
| **T148 — live continuity walkthrough** | **NOT RUN — needs Claude Code, Codex and OpenCode, and a real compaction in each** |
| T147 — topic-key effectiveness | harness complete, results NOT COLLECTED — non-blocking |

Two release-blocking gates have not been run. Until they have, this feature is
implementation-complete and **not** release-ready.
