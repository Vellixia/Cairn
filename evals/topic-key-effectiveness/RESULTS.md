# Results — topic-key effectiveness

One table per release, dated. **Informational: there is no threshold and this
cannot fail a build.**

---

## Unreleased (Feature 003) — 2026-08-20

**Status: RUN**, on a 12-item stratified sample of the 26-item corpus, against
three live agents in isolated environments. Raw exports in [raw/](./raw/).

### Configuration

Deliberately the **cheapest workable configuration for each agent**, chosen by
the operator. This is integration and effectiveness evidence, not a benchmark:
the question is whether the topic-key contract works in a real low-cost setup,
not which model is strongest.

| agent | version | provider | model | reasoning | isolated config root |
|---|---|---|---|---|---|
| codex | codex-cli 0.147.0 | openai | `gpt-5.4-mini` | low | `$ISO/.codex` (own interactive login) |
| opencode | 1.18.18 | opencode-zen | `opencode/hy3-free` (free) | n/a | `$ISO/oc-config/opencode` |
| claude-code | 2.1.237 | anthropic | `haiku` | n/a | repo `.claude` + `--mcp-config` |

Each model was verified available before use. **No fallback to a more expensive
model occurred.** Each agent ran in its own store and its own clean repository, a
fresh process per corpus item, with the prompt given verbatim — nothing mentioned
recording, memory or keys.

### Sample

12 of 26 items, stratified to preserve every measurement the full corpus makes:
three `↔` pairs (consistency), three `✗` near-misses (false grouping), two plain
items (adoption and specificity), one `failure`-type item, across all five
archetypes.

`A1 A2 A4✗ A5 · B1 B2 B4✗ · C1 C2 C4✗ · D5 · E1`

### Measures

| measure | codex | opencode | claude-code |
|---|---|---|---|
| items that produced a memory | 7 / 12 | 6 / 12 | 1 / 12 |
| memories written | 7 | 6 | 1 |
| **topic-key adoption** | **7 / 7** | **6 / 6** | **1 / 1** |
| value keys present | 7 / 7 | 6 / 6 | 1 / 1 |
| value-key specificity (states the whole claim) | 7 / 7 | 6 / 6 | 1 / 1 |
| same-fact cross-session consistency | 0 / 1 pairs met | 0 / 1 pairs met | **1 / 1 pairs met** |
| **false grouping** | **0** | **0** | **0** |
| missed grouping | 1 (C1↔C2) | 1 (A1↔A2) | 0 |
| safely reconcilable share | 1 / 1 | 1 / 1 | n/a |

**False grouping is zero for all three agents.** Nothing here is a correctness
finding; everything below is a product finding.

### The result that matters: value keys converge, topic keys do not

The same fact — *the production database is PostgreSQL* — was recorded by all
three agents, on **three different topic keys with the identical value key**:

| agent | topic_key | value_key |
|---|---|---|
| codex | `infrastructure.production_database` | `postgresql` |
| opencode | `database.production` | `postgresql` |
| claude-code | `production_database` | `postgresql` |

The same split appears on A4: `database.local_development` vs `database.local_dev`,
both with value key `sqlite`.

So **cross-agent consistency under this configuration is 0 of 1**: the fact never
landed on one subject, and not because any agent stated it badly. Every value key
states its whole claim, and the value halves agree exactly. What has no shared
convention is the topic namespace — whether the topic is the noun
(`production_database`), a dotted path from a domain (`database.production`), or a
dotted path from a layer (`infrastructure.production_database`).

Per D46 this goes to the **usage contract, the tool descriptions and the Skill** —
never to a similarity heuristic. The value-key guidance is evidently working; the
topic-key guidance does not say enough to make two agents pick the same namespace.

### Within one agent

Each of the two agents that wrote both halves of a `↔` pair split it:

- codex — C1 `batches.idempotency|replay_has_no_effect`, C2 `batch.replay|keyed.replay_is_safe`
- opencode — A1 `database.production|postgresql`, A2 `infra.database|prod_uses_postgres`

Both halves are correct and specific; they simply do not meet. One `supersede`
resolves each, so the safely-reconcilable share is 1 of 1 in both cases.

Claude Code is the exception and the reason is worth recording: on A2 it answered
*"already recorded in Cairn — PostgreSQL as the production database is documented
in the project memory"*. It had received Cairn's briefing at session open, saw the
A1 memory, and declined to write a duplicate. That is the intended outcome — one
subject for one fact — and the only instance of it in this run.

### Near-misses

Every `✗` item behaved correctly:

- A4 (`Local development uses SQLite`) stayed distinct from production for both
  agents that recorded it. OpenCode's content even cross-references it —
  *"while production uses PostgreSQL, per database.production decision"* — which
  is the topic key being used as a reference rather than a label.
- B4 (`the tool itself is versioned with semver`) was recorded by codex as its own
  subject `tool.versioning.scheme|semver`, not merged into plugin discovery.
- C4 (`the HTTP client times out at 30 seconds`) — the same number as C3's retry
  cap — was recorded by nobody, so it could not collide.

### Adoption is the open product question

Unprompted adoption ranges from 1 in 12 to 7 in 12. The prompts deliberately never
ask for anything to be recorded, so this measures what an agent does on its own,
and on the cheapest configuration of each vendor it does it less than half the
time. Claude Code on `haiku` mostly acknowledged the fact conversationally, and on
E1 asked whether to record it rather than recording it.

This is a finding about guidance and about model capability at this price point,
and it is not separable into those two causes by this run. It is **not** evidence
about Cairn's correctness: a fact that is never recorded costs recall, and Cairn's
answers remain deterministic over whatever was recorded.

### Honest limits of this run

- **12 of 26 items**, by instruction. The three archetypes' remaining `↔` pairs
  and near-misses were not exercised.
- **Cheapest configuration per agent**, and the three agents therefore ran on
  different models. Read the cross-agent row as *consistency under the selected
  real-world low-cost configuration*. It is **not** a controlled comparison of
  model quality, and no conclusion about relative model capability should be drawn
  from it.
- One run per item; no repetition, so nothing here separates a stable behaviour
  from a sampled one.
- Three harness defects had to be fixed first, each of which had produced a false
  zero. They are recorded in the implementation log rather than here, because they
  are facts about the integrations rather than about topic keys.
