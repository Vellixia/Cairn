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
