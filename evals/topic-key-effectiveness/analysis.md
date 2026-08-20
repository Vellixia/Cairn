# Analysis — topic-key effectiveness

What the numbers in [RESULTS.md](./RESULTS.md) mean, and what they should change.
A finding here is a **proposal for a human to review**. Its only permitted effect
on the deterministic system is to suggest corpus cases (D46).

---

## 2026-08-20 — first real run

### The finding

Value keys converge; topic keys do not.

Three agents recorded *the production database is PostgreSQL*. All three chose the
value key `postgresql`. All three chose a different topic key:
`production_database`, `database.production`, `infrastructure.production_database`.
A4 repeats the pattern — `database.local_dev` against `database.local_development`,
both with value key `sqlite`.

This is the useful shape of the result, because it separates two things the
metric used to blur together. Specificity was 100% everywhere: every value key
stated its whole claim, which is what the guidance asks for and evidently
communicates well. Grouping still failed, and it failed entirely on the topic
half.

### Why this is not a correctness problem

False grouping was **0** for all three agents, which is the only outcome the
protocol treats as a defect. Nothing merged that should not have. Every `✗`
near-miss stayed separate, including the two designed to be hardest: A4, which
shares a topic with A1 but differs in scope, and C4, which shares a *number* with
C3 but nothing else.

Missed grouping costs deduplication, not truth. Two records of one fact on two
subjects are reported as two subjects, both correct, neither merged — and one
`supersede` resolves each. Safely reconcilable was 1 of 1 in both cases where it
applied.

### What it suggests changing

The topic-key guidance does not say what a topic *is*. Three defensible readings
appeared in three agents:

- the bare noun the claim is about — `production_database`
- a dotted path from the domain — `database.production`
- a dotted path from an architectural layer — `infrastructure.production_database`

All three are reasonable, and no two of them meet. A proposal for review: the
usage contract and the `cairn_remember` tool description should state a rule
concrete enough that two agents who have never met pick the same string — most
plausibly "the narrowest noun phrase naming what the claim is about, singular,
no layer or category prefix" — and say so with a worked pair, since the corpus
shows agents follow the *example* more reliably than the prose.

This is a documentation change, and it must stay one. Making Cairn tolerant of
divergent topic keys by matching them heuristically is the option D46 rejects,
and this run gives no reason to revisit that: the reconciliation behaved exactly
as designed on every subject it was given.

One counter-observation worth keeping. OpenCode's A4 memory reads *"Local
development uses SQLite (while production uses PostgreSQL, per
database.production decision)"* — it used a topic key as a **reference to another
subject**, unprompted. If the contract ever gains a "related subject" notion,
that is evidence agents will reach for it.

### Adoption, and what it does not tell us

Unprompted adoption was 7/12, 6/12 and 1/12. The prompts never ask for anything
to be recorded, so this is what each agent does on its own initiative on the
cheapest configuration its vendor offers.

The 1/12 deserves care rather than a conclusion. Claude Code on `haiku` mostly
acknowledged facts conversationally, and on E1 it asked whether to record rather
than recording. But its single write was also the run's **best** consistency
result: on A2 it read Cairn's briefing, recognised the fact was already recorded,
and declined to duplicate it — one subject for one fact, which neither other
agent achieved. Low adoption and good discipline are not the same axis, and this
run has one instance of each in the same agent.

Two causes are entangled here and this run cannot separate them: how strongly the
guidance asks for a durable record, and how much instruction-following capacity a
deliberately cheap model has. Separating them needs the same corpus at a second
price point per vendor, which is a bigger run than this one.

### Read the cross-agent number carefully

The three agents ran on three different models — `gpt-5.4-mini` at low reasoning,
`opencode/hy3-free`, and `haiku` — because the run was scoped to the cheapest
workable configuration for each. The cross-agent consistency figure is therefore
**consistency under the selected real-world low-cost configuration**, which is a
useful thing to know and is *not* a controlled comparison of model quality. No
ranking of these agents or models should be taken from this table.

### Suggested corpus cases

Two gaps this run exposed, offered as corpus additions rather than conclusions:

1. A pair whose fact has an obvious layer *and* an obvious domain, to test the
   namespace rule directly rather than incidentally.
2. A `↔` pair where the second half arrives in a session that has already been
   given the first half's memory. Claude Code hit this by accident and handled it
   well; nothing in the corpus asks for it deliberately, so the good outcome was
   unmeasured.
