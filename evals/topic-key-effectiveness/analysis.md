# Analysis — topic-key effectiveness

What the numbers mean, and what they may and may not change.

## No results yet

See [RESULTS.md](./RESULTS.md). Nothing has been collected, so there is nothing
to analyse. This file states in advance how a result would be read, which is
worth writing down before the numbers exist: an analysis written after seeing
them is an analysis that can be steered by them.

## How a low adoption rate would be read

As a **product** finding about three surfaces, in this order:

1. **The tool description.** `cairn_remember` states the obligation in one
   clause. If agents are not carrying it out, that clause is the cheapest thing
   to change and the first thing an agent reads.
2. **The always-on contract.** Four Feature 003 obligations sit in every agent's
   instructions, inside a 1,200-character bound. If one is being ignored, the
   question is whether it is stated as an action or as a principle.
3. **The Skill.** `references/recording-knowledge.md` explains *why* a key can
   be too fine as well as too coarse. If agents adopt keys but choose them
   badly, this is where the fix goes.

It would **not** be read as a reason to infer subjects from content. That is
D46, and it is refused on correctness grounds rather than on effort: an
inferred subject makes a wrong merge possible, and a wrong merge destroys a
distinction a session deliberately recorded.

## How a missed grouping would be read

As the expected cost of the design. Two records of one fact that did not meet
are two memories where one would do — the reader sees both, which is
redundant rather than wrong. `cairn memory subject` and the corroboration
prompt exist to make collapsing them cheap when someone notices.

## How a false grouping would be read

As a **defect**, immediately, and not as a number in a table.

Only content identical after normalization merges. If two materially different
claims became one canonical answer, then either `normalize_content` is losing a
distinction it should keep, or something is merging that the gate should have
stopped. The finding goes to `crates/cairn-core/src/knowledge.rs` and to the
`reconciliation/distinct/` corpus, not here.

## What this evaluation may change in the product

Only one thing, and only through a person: it may **propose corpus cases**. A
grouping an agent got wrong is a case worth adding to
`reconciliation/distinct/` or `reconciliation/coarse_value_key/`, where it
becomes a deterministic assertion that runs on every build.

It may not change a threshold, because there is no threshold. It may not gate a
release. And it may not introduce a heuristic.
