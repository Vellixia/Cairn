# Results — topic-key effectiveness

One table per release, dated. **Informational: there is no threshold and this
cannot fail a build.**

---

## Unreleased (Feature 003) — NOT COLLECTED

**Status: not run.** The harness, the corpus and the protocol are complete; the
results are not, because collecting them needs three live agents.

**What running it requires**

- Claude Code, Codex and OpenCode installed and authenticated on a real
  machine, each with its **native** Cairn integration (`cairn connect <agent>`),
  not generic MCP.
- A clean repository per agent, so cross-agent consistency measures convergence
  rather than one agent reading another's memories.
- One fresh agent session per corpus item — 26 items × 3 agents.

**What is therefore unknown**

Every measure below. In particular the false-grouping count is **unknown, not
zero**: no corpus item has been run, so nothing has been observed either way.
Since a non-zero false-grouping count would be a design defect rather than a
number, the honest statement is that this check has not been performed.

| Agent | Adoption | Value-key specificity | Cross-session | Cross-agent | Missed grouping | False grouping | Safely reconcilable |
|---|---|---|---|---|---|---|---|
| Claude Code | — | — | — | — | — | **not observed** | — |
| Codex | — | — | — | — | — | **not observed** | — |
| OpenCode | — | — | — | — | — | **not observed** | — |

**What is known without running it**

The deterministic half of the same concern is fully covered and green:

- `reconciliation/coarse_value_key/` — 17 adversarial cases where one topic and
  one value key cover materially different statements. Every one is reported as
  `Corroborated` with both statements retained, and none merges.
- `reconciliation/distinct/` — 22 cases that look mergeable and must not merge.
  Zero false merges.
- `cairn status` reports the adoption metric in every project, so a real
  deployment's number is observable without this evaluation.

That is why this is informational: what the corpus above measures is whether
agents *use* the mechanism well, not whether the mechanism is correct when they
use it badly.
