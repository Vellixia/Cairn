# T157 — acceptance trials and the human accuracy rubric

Recorded 2026-09-06, from a real run of `tests/tests/feature005_acceptance_trials.rs`.
This is a human transcription of that test's own printed report (run with
`--nocapture`), not a file the test writes itself — see that file's module
doc comment for why: a test that wrote its own results file would be
reviewing itself, which is exactly what SC-701's `by_review` criteria exist
to rule out.

## What this covers, and what it does not

T157 is filed against SC-701, SC-708 and SC-715. This document and its test
cover **SC-701 only** — ten real-repository trials per agent, feeding the
pre-registered accuracy rubric (`tests/feature005/us1-accuracy-rubric.json`).
SC-708 (automatic delivery to a second session) and SC-715 (offline agent
operations meeting their deadline) are not re-derived here; they already have
dedicated tests and are not duplicated by this file:

- SC-708 (delivery): `tests/tests/feature005_us2_automatic_recall.rs`,
  `tests/tests/feature005_delivery.rs`.
- SC-715 (offline deadlines): `tests/tests/feature005_outage.rs`,
  `tests/tests/feature005_us4_fail_soft.rs`, and the capture-deadline
  measurement in `tests/tests/feature005_performance.rs`
  (`tests/feature005/performance-measurements.md`, §1).

## Run context

- Commit: `8e8a0dc78e2308d7a9d721a4aed310598d3bc04e` (branch `feature-005-spec`;
  several sibling Polish-phase commits landed on top of this in parallel while
  this trial run and this document were being produced — none touch
  `crates/cairn-server/src/consolidate.rs`, `extract.rs`, or the schema this
  test reads, so they do not bear on the numbers below).
- Machine: MacBook Air (Apple M1), 8 cores, macOS 26.6.2 (Darwin 25.6.0),
  arm64. Debug build (`cargo build --workspace`, no `--release`).
- PostgreSQL 17 (Docker, `postgres:17-alpine`) on `localhost:5433`, reachable
  at `CAIRN_TEST_DATABASE_URL=postgres://cairn:cairn@localhost:5433/cairn`.
- Command: `cargo test -p cairn-e2e --test feature005_acceptance_trials --
  --nocapture --test-threads=4`.
- Total wall time: **19.7 s** for all four tests (the rubric-drift check plus
  the three agents' ten trials each), run concurrently — well inside the
  five-minute budget. The extractor this suite runs is the deterministic
  heuristic one (no vendor API calls), which is why ten sessions per agent
  cost a handful of 100 ms consolidation-poll cycles rather than ten times a
  real inference latency. No trade-off was needed to fit the budget on this
  machine; if a slower machine ever crowds it, the fix is `CONSOLIDATION_SETTLE`
  in the test file, not a weaker assertion.

## Results per agent

Ten trials per agent, one project per agent, distinct `session_id` per trial.
"Trials with a durable record" counts a trial as a hit if consolidation
produced at least one candidate resolving to a knowledge record from that
trial's own session (joined through `consolidation_runs.session_id` →
`knowledge_candidates.run_id`, `result_knowledge_id IS NOT NULL` — never by
assuming a record's id equals a particular trial's candidate id, since a
reinforcement points back at an earlier trial's record).

| Agent | Trials run | Trials with ≥1 durable record | `kind_is_one_of_five` | `claim_is_non_empty` | `keys_are_canonical` | `provenance_resolves` |
|---|---|---|---|---|---|---|
| claude_code | 10 | **10/10** | 21/21 | 21/21 | 21/21 | 21/21 |
| codex | 10 | **10/10** | 21/21 | 21/21 | 21/21 | 21/21 |
| opencode | 10 | **10/10** | 11/11 | 11/11 | 11/11 | 11/11 |

The checked/passed counts are equal in every cell because each criterion is
also asserted in the test — a failure would have aborted that agent's run
rather than produced a partial count. The denominators differ because
`claude_code` and `codex` emit semantic signals (FR-727e) and so produce
**three** distinct records in trial 0 (a `fact`, a `failure`, a `decision`) —
21 = 3 (trial 0) + 2×9 (trials 1–9, where the same decision keeps getting
reinforced rather than re-created) — while `opencode` emits no semantic
signals (FR-838b) and so produces only the two structural records (`fact`,
`failure`) from R1–R6: 11 = 2 (trial 0) + 1×9.

`nothing_asked_for_it` (the fifth mechanical criterion) held for all three
agents: zero `explicit` records and zero superseded records appeared in any
of the thirty sessions, none of which invoked a Cairn tool.

## Review material — one representative record per agent

Exported verbatim from the test's own query against `knowledge_candidates` /
`candidate_source_events` / `safe_events`, so a reviewer can judge the two
`by_review` criteria against real cited evidence rather than a paraphrase.

### claude_code — trial 0, record `a42ab97b-9a78-5db3-85ff-6b1144a86b4f` (decision)

> This project decided to adopt parser for widget.

Cited safe events (`decision_signal`, ×10 across the ten trials that
reinforced this same record):

```json
{"Decision": {"object_token": "parser", "decision_kind": "adopt",
  "subject_token": "widget", "lexicon_version": 1, "justified_by_seq": 6}}
```

### claude_code — trial 0, record `6323b42b-093c-5bee-a2a7-2ce9cc6ed0a9` (failure)

> Tests were failing and passed after changes to parser.

Cited safe events, in session order: ten `test_executed`
(`{"TestInvocation": {"test_command": "cargo test -p widget"}}`), ten
`test_result` with `exit_status: 1` / `test_outcome: "failed"`, ten
`file_changed` (`src/widget/parser.rs`, `modified`), then ten `test_result`
with `exit_status: 0` / `test_outcome: "passed"`.

### codex — trial 0, record `079c3cbd-b77f-5804-9bcd-48fd9bccccfd` (failure)

> Tests were failing and passed after changes to parser.

Same cited-event shape as claude_code's failure record above:
`test_executed` → `test_result` (failed) → `file_changed` → `test_result`
(passed), all against `cargo test -p widget` / `src/widget/parser.rs`.

### opencode — trial 0, record `5b2960fe-49e2-5955-ac90-c6591d90ef92` (fact)

> The test command for this project has the verb cargo.

Cited safe events: twenty `test_executed`
(`{"TestInvocation": {"test_command": "cargo test -p widget"}}`) — structural
evidence only, since OpenCode's session carries no `decision_signal` or any
other semantic event (FR-838b).

### opencode — trial 0, record `87a52ab3-f785-53a5-9cfe-1dcbfe7ad94c` (failure)

> Tests were failing and passed after changes to parser.

Same cited-event shape as the other agents' failure record: `test_executed` →
`test_result` (failed) → `file_changed` → `test_result` (passed).

## `by_review` — not evaluated, and what a reviewer does next

**The two `by_review` criteria below have NOT been evaluated by this test,
by this document, or by any automated process. No pass count, no fraction,
no score is recorded for either one. Any number that appeared here would be
manufactured evidence.**

- **`claim_is_supported_by_its_evidence`**: *"A reader given only the cited
  safe events would judge the claim true of this project."* A human reviewer
  must read each record's claim (above) against only its cited events (above)
  — not the test fixture's `Session` source, not this document's framing —
  and judge, for each of the records exported above (and, ideally, a larger
  sample drawn from the full set of records these thirty sessions produced),
  whether the claim is a fair reading of that evidence.
- **`claim_says_no_more_than_the_events_establish`**: *"The claim does not
  assert reasoning, intent or a cause that the event stream does not carry."*
  The same reviewer must check that no record's claim smuggles in a *why*
  the cited events do not themselves state — contracts/extraction.md §13.9
  requires that a decision is recorded as taken and about what, never why.

The reviewer's judgement, once made, belongs in a follow-up revision of this
document (or a linked record), attributed to the person who made it and
dated, exactly as SC-701 requires — never folded back into
`feature005_acceptance_trials.rs` as an assertion, which would make the test
grade its own homework.
