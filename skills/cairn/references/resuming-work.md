# Resuming existing work

Cairn writes a handoff at every session boundary: before compaction, and when a session ends.
The handoff is the record of what the previous session actually did — not a summary someone
wrote by hand.

## Do this first

1. Read the handoff Cairn delivered in your context. It names changed files, tests that ran,
   failures, decisions, work completed and remaining, and a next step.
2. Continue from the next step. Do not re-derive the state of the branch by reading the whole
   tree — the handoff already did that work, from evidence.
3. Where the handoff names a failure, check whether it still reproduces before assuming it does.

## What a handoff is not

It is not a task list you must finish, and it is not authoritative about intent. It is an
account of what happened. If it conflicts with what you observe now, what you observe wins —
and that conflict is itself worth recording.

## When there is no handoff

A first session on a repository has none. That is normal. Search memory instead, then proceed.
