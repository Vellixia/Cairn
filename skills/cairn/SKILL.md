---
name: cairn
description: Use Cairn's persistent project memory — resume prior work, search before investigating, record durable decisions and failures, choose the right memory scope, and bind work to a task.
metadata:
  cairn_skill_schema: 1
  cairn_skill_revision: b3c2d88c7b25
---

# Cairn

Cairn is durable memory for this repository, shared by every agent working on it, so what one
session learns the next one already knows.

Most of it is project memory. Two smaller domains are not scoped to the project at all:
**personal** knowledge follows your account across every project and machine, and **team**
guidance is a server-wide default every account sees. Both are deliberately stripped of
anything that identifies where they came from — see
[knowledge-domains](references/knowledge-domains.md).

Use this Skill when you need more than the always-on rules: when you are resuming someone
else's work, deciding whether something is worth recording, choosing a scope, or working out
why Cairn is reporting a problem.

## When to reach for which reference

| Situation | Reference |
|---|---|
| A session is starting and work already exists | [resuming-work](references/resuming-work.md) |
| You are about to investigate something | [searching-first](references/searching-first.md) |
| You learned something worth keeping | [recording-knowledge](references/recording-knowledge.md) |
| You are recording and must pick a scope | [choosing-scope](references/choosing-scope.md) |
| Session or task binding is unclear | [sessions-and-tasks](references/sessions-and-tasks.md) |
| Cairn reports a problem | [diagnosing-cairn](references/diagnosing-cairn.md) |
| What you learned is about you, or about the whole team, rather than this repository | [knowledge-domains](references/knowledge-domains.md) |

## The rules that always apply

The always-on Cairn contract is already in your instructions. This Skill never repeats it —
it explains how to act on it. If the two ever disagree, the contract wins.
