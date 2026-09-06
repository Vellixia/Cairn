# SC-701 acceptance trials — recorded results (T157)

## The run this file records

| | |
|---|---|
| **Tested commit** | `28c0ac114e82ae7d326eac24ef38f83e19843e06` |
| **Tree state** | clean — `git status --porcelain` empty before the run |
| **Command** | `cargo test -p cairn-e2e --test feature005_acceptance_trials -- --nocapture --test-threads=4` |
| **Database** | PostgreSQL 17, `CAIRN_TEST_DATABASE_URL` |
| **Machine** | Apple silicon, macOS (Darwin 25.6.0), debug build |
| **Date** | 2026-09-06 |
| **Wall time** | 27.5 s |

`tests/tests/feature005_acceptance_trials.rs` **exists at the tested commit** —
verified with `git cat-file -e 28c0ac1:tests/tests/feature005_acceptance_trials.rs`
— so the run is reproducible from the SHA alone.

**This file is committed after the tested commit, and is evidence only.** An
earlier version of it recorded `8e8a0dc` as the tested SHA, but the trials test
did not exist in that commit: the run had been made from an uncommitted working
tree while `HEAD` still pointed there, and the SHA alone could not reproduce it.
The order is now: commit the code, verify the test exists in it, run from the
clean tree, then commit this document. Do not read the repository's final `HEAD`
as the tested SHA — later commits may follow this one.

## Mechanical results

Ten independent sessions per agent, thirty in total. The five criteria the
rubric (`tests/feature005/us1-accuracy-rubric.json`) marks `mechanically` are
evaluated per record and asserted by the test.

| Agent | Trials | Trials producing a durable record | Records | kind_is_one_of_five | claim_is_non_empty | keys_are_canonical | provenance_resolves |
|---|---|---|---|---|---|---|---|
| claude_code | 10 | 10 | 20 | 20/20 | 20/20 | 20/20 | 20/20 |
| codex | 10 | 10 | 21 | 21/21 | 21/21 | 21/21 | 21/21 |
| opencode | 10 | 10 | 11 | 11/11 | 11/11 | 11/11 | 11/11 |

`nothing_asked_for_it` — zero explicit records and zero supersessions across all
thirty sessions — held for every agent. It is evaluated once per agent because
it is an absence claim about the whole run, not a property of a record.

OpenCode produces fewer records because it emits no semantic signals at all
(FR-838b), so its knowledge is structural. That is the behaviour, not a
shortfall.

## Review material, as exported by the run

One representative record per agent, with the evidence it cites. This is what an
independent reviewer sees — the claim and the cited safe events, and nothing
else.

**claude_code** — `fact`
> The test command for this project has the verb cargo.

cited: `test_executed` · `{"TestInvocation": {"test_command": "cargo test -p widget"}}`

**codex** — `decision`
> This project decided to adopt parser for widget.

cited: `decision_signal` · `{"Decision": {"decision_kind": "adopt", "subject_token": "widget", "object_token": "parser"}}`

**opencode** — `fact`
> The test command for this project has the verb cargo.

cited: `test_executed` · `{"TestInvocation": {"test_command": "cargo test -p widget"}}`

## The two `by_review` criteria

SC-701's accuracy judgement is made by an **independent reviewer** — independent
meaning *not the implementation or test process being graded*. The test never
scores these two criteria; a mechanical assertion here would be the test
agreeing with itself.

| | |
|---|---|
| **Reviewer type** | independent external AI reviewer |
| **Not** | a human; not the repository owner; not this implementation or its tests |
| **Delegated by** | the project owner, who delegated this review rather than performing it |
| **Date** | 2026-09-06 |
| **Saw** | the durable claim and the safe events it cites |

| Criterion | Verdict |
|---|---|
| `claim_is_supported_by_its_evidence` | **PASS** |
| `claim_says_no_more_than_the_events_establish` | **PASS** |

Reviewed record forms and the reviewer's reasoning as supplied:

1. `This project decided to adopt parser for widget.` — cites
   `Decision(adopt, subject=widget, object=parser)`. Supported directly; no
   rationale or cause is invented.
2. `Tests were failing and passed after changes to parser.` — cites a failed
   test, a change to `parser`, and a passing test. The statement is temporal
   ("after"), not causal ("because of"), and is supported without asserting
   unstated reasoning.
3. `The test command for this project has the verb cargo.` — cites
   `cargo test -p widget`. Directly supported.

**Attribution is recorded exactly as it stands.** No human has reviewed these
records, and this document does not claim one has. Neither does it attribute the
review to the repository owner personally. The reviewer was an external agent,
independent of the process that produced the records, which is the property
SC-701 requires — see the spec's note that humanity was never what made the rule
work.

## What this file does not cover

SC-708 (delivery) and SC-715 (offline deadlines) are also cited on T157 in
`tasks.md`. They are proved by their own tests —
`feature005_us2_automatic_recall.rs` and `feature005_delivery.rs` for delivery,
`feature005_outage.rs`, `feature005_us4_fail_soft.rs` and
`feature005_performance.rs` for deadlines — rather than re-derived here. That is
a scope interpretation and is recorded as one.

All thirty trials use one fixture scenario per agent. They show the mechanism is
reliable when driven with that scenario; they do not speak to varied real
repository work.
