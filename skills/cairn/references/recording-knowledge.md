# Recording durable knowledge

Record what a future session would otherwise have to rediscover.

## Worth recording

- **Decision** — a choice with alternatives that were rejected, and why.
- **Failure** — something that did not work, with enough detail to recognise it again.
- **Convention** — how this project does a thing, where that is not obvious from the code.
- **Procedure** — a sequence that is easy to get wrong and that you had to work out.
- **Fact** — a durable property of the system that is expensive to establish.

## Not worth recording

- Routine tool calls. Cairn already captures those as observations.
- Restatements of what the code plainly says.
- Anything you have not actually established.
- Anything that will be false after the next commit.

## Rules that are absolute

- Never invent an evidence observation identifier. Cite only observations that exist.
- Never record secrets, credentials, tokens, raw prompts, or unbounded command output.
- Record the thing you learned, not the transcript of learning it.

## Give a durable fact a subject

A **topic key** names what the fact is about; a **value key** names what it asserts. Together
they must state the whole claim, because that pair is what Cairn compares.

```text
topic_key  infrastructure.production_database
value_key  postgresql
```

Specific enough matters in both directions. `database` is too coarse — a cache and a queue
would land in the same subject and be reported as disagreeing. `infrastructure.production_database.primary.postgresql.16.2`
is too fine — nothing else ever lands there, and the fact never meets the claim it
contradicts.

A key that will not normalize does not lose you the memory: it is stored free-form, exactly
as it would have been before, and Cairn tells you it did that.

## Attach evidence rather than asserting importance

`importance` ranks within a bucket. It does not make a memory truer, does not change scope
precedence, and does not admit anything into reserved context.

What does change how a memory is treated is **evidence**: a file, a configuration key, a Git
ref, a command outcome. Attach one and Cairn can check it later and tell you when the world
moved. Assert `importance: high` instead and you have told the next session to trust
something nobody can re-check.

An attestation — you reporting that you checked — is recorded and labelled as yours. It is
worth having, and it is not the same as a check Cairn ran itself.

## When Cairn names a corroborating member

Writing a memory can come back with `corroborating_member`: the same subject, the same value,
different words. Cairn will not merge them, because only you can read both and say whether
they are one claim.

- Same claim, said differently → `reinforce` the existing memory. One claim, now recorded as
  confirmed by two sessions.
- Genuinely different claims that happen to agree on the value → leave both. The subject
  reports them as corroborating, which is what they are.

Doing nothing is the third option and it is the worst one: two memories, neither aware of the
other, both surfacing forever.

## Record what happened to a pattern

A reusable pattern from another project is a suggestion, never an answer. When you act on
one, say what happened — including when it did not help.

- It resolved the problem → `record_outcome` with `resolved`.
- Same symptom, different cause → `not_applicable`, with the cause you actually found.
- The approach was right and it still failed → `failed`.

A negative outcome is the most valuable one to record. It is what stops the next session in
another project from spending an afternoon on a lead that was already ruled out, and it
never deletes the pattern or reduces what it has done elsewhere.
