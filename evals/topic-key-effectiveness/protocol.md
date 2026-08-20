# Protocol — topic-key effectiveness

**Informational. Not a gate.** Nothing here can fail a build, and there is no
threshold. See [contracts/evaluation.md](../../specs/003-project-intelligence/contracts/evaluation.md)
§Topic-key effectiveness for why.

The question this answers is a **product** question: when an agent is told to
give a durable fact a topic key and a value key, does it? And do two agents,
or two sessions of one agent, choose keys specific enough to meet each other?

The question it does **not** answer is whether Cairn is correct. Cairn's
correctness does not depend on agents choosing well: a key that is too coarse
costs deduplication, not truth, because a shared value key with differing
content is reported as corroborating and never merged.

## What is measured

Per agent, over the corpus in [corpus.md](./corpus.md):

| Measure | How it is counted |
|---|---|
| Topic-key adoption | memories written with a `topic_key` ÷ memories written |
| Value-key specificity | value keys that state the whole claim ÷ value keys written, judged against the corpus's own "the claim is" column |
| Same-fact cross-session consistency | the same fact recorded in two sessions of one agent lands on one subject |
| Cross-agent consistency | the same fact recorded by two different agents lands on one subject |
| Missed grouping | two records of one fact that did **not** meet — a cost, not a defect |
| False grouping | two records of **different** facts that did meet |
| Safely reconcilable share | subjects where one `reinforce` or one `supersede` would resolve the duplication |

## The one result that is a defect

**A non-zero false-grouping count is a design defect, and must be raised as
one** — not recorded as a number.

Only identical content after normalization merges. If two different facts ended
up as one canonical answer, either the normalization is wrong or something is
merging that should not, and the finding goes to the code rather than to this
table.

Everything else is a product finding. A low adoption rate sends us to the usage
contract, the tool descriptions and the Skill — never to a similarity
heuristic, which D46 rejects on correctness grounds.

## Running it

Three agents, each with its **native** integration installed, against a real
repository. A fresh session per corpus item, because a session that has already
seen Cairn's context has been influenced by it and is no longer measuring what
an agent does unprompted.

```
1. Create an empty repository and `cairn init`.
2. `cairn connect <agent> --yes` — the native integration, not generic MCP.
3. For each corpus item:
     a. Start a *fresh* agent session.
     b. Give it the item's prompt verbatim.
     c. Let it work and record whatever it records.
     d. End the session.
4. `cairn --json memory search --limit 500 > raw/<agent>.json`
5. `cairn --json status` — record the adoption metric it reports.
6. Repeat from 1 with a clean repository for the next agent.
```

Step 6 matters: a shared repository would let the second agent read the first
agent's memories, and cross-agent consistency would measure imitation rather
than convergence.

## Recording

[RESULTS.md](./RESULTS.md), dated, one table per release. Raw exports beside it
under `raw/`, so a number can be traced back to the memories it came from.

[analysis.md](./analysis.md) carries what the numbers mean and what, if
anything, they should change. A finding there is a proposal for a human to
review — its only permitted effect on the deterministic system is to suggest
corpus cases.
