# Quickstart — Feature 005

How Server-Authoritative Autonomous Memory is demonstrated on a real repository. Principle I
requires the feature to end in something a developer can install and use; Principle VII
requires that demonstration to be defined rather than improvised. This is it.

The rule that makes the demonstration meaningful: **no Cairn tool is invoked to make it pass.**
Typing `cairn_remember`, `cairn_search` or `cairn_context` to produce the expected result
invalidates the run.

## 0. Prerequisites

```bash
cargo build --workspace          # NOT --tests: the harness spawns prebuilt binaries
cairn-server migrate             # server schema v3 → v4
cairn init                       # local schema v7 → v8
cairn link --server https://<host>
cairn integrate claude-code      # or codex
```

Confirm the starting state:

```bash
cairn status                     # authority mode, spool depth, server reachability
cairn memory list                # expect: empty
```

## 1. Agent A does real work

Open a session in a supported agent on a real repository with a failing test. Let the agent
investigate, read the relevant code, change the implementation, and run the suite until it
passes. Do not mention Cairn.

**Watch the events arrive:**

```bash
cairn events tail                # local spool, live
curl -s $SERVER/api/projects/$PID/activity?limit=20 | jq  # server side
```

Expect `session_opened`, `file_read`, `file_changed`, `test_executed`, `test_result(failed)`,
more `file_changed`, `test_result(passed)`, `session_closed`.

**Watch consolidation run:**

```bash
curl -s $SERVER/api/projects/$PID/funnel | jq            # consolidation runs + counts
```

A run appears within 10 minutes of the first event, or immediately when the session closes.

**Expected knowledge** — rule R1 in `contracts/extraction.md` fires on the
failed → changed → passed shape:

```bash
cairn memory list --origin consolidated
```

A **FAILURE** record naming the tests that were failing and the files whose change fixed them,
with `origin_kind = consolidated` and provenance resolving to the session's events.

**Verify provenance is real, not decorative:**

```bash
cairn memory show <id> --provenance     # lists the source event ids
curl -s $SERVER/api/projects/$PID/activity?event=<event_id>  # each one resolves
```

## 2. Agent B benefits

Open a **new** session in a related area. Do not search Cairn.

At session open, and again when you submit your first prompt, the briefing reaches the agent's
own context surface — for Claude Code and Codex CLI, the two agents Feature 005 commits to
automatic delivery (FR-838a).

```bash
curl -s $SERVER/api/projects/$PID/retrieval-traces?session=$SID | jq
```

Two traces: one `session_open`, one `prompt_submit`. Check the property that matters —
**the second does not restate the first**:

```bash
curl -s $SERVER/api/retrieval-traces/$TRACE2 | jq '.items[] | "\(.domain):\(.knowledge_id)"'
```

No id appears in both traces unless its `updated_at` moved between them
(`contracts/retrieval-delivery.md`).

**OpenCode**: capture works and events arrive; automatic delivery does not, and health reports
`declined_by_cairn` with the reason — OpenCode 2's hooks exist but are beta (FR-838b). Use
`cairn_context` manually there.

## 3. The web tells the story

Using only the web interface, trace the whole path:

| Screen | What it proves |
|---|---|
| Dashboard | The funnel: events received → candidates → knowledge accepted, with failures |
| Activity | What Cairn received, semantically, not a firehose |
| Memory detail | Content, provenance, evidence summary, relations, verification, reinforcement, retrieval usage, and whether it was explicit or consolidated |
| Retrieval traces | Trigger, candidates, selection and its explanation, budget, delivery state |
| Agents | Per-agent, per-capability, per-stage health — configuration distinguished from runtime capture |

Receipt shows `unavailable / no evidence` for every agent. That is the truthful answer: no
acknowledgement mechanism was established from the vendor documentation reviewed, which is not
the same as a vendor stating none exists (FR-838e).

## 4. The server goes away

```bash
sudo pfctl -e                    # or stop the server
```

Keep working in the agent. Expect: **no stall, no error, no slowdown.**

```bash
cairn status                     # spool depth rising, oldest entry, reason
```

Restore the connection:

```bash
cairn status                     # spool drains
curl -s $SERVER/api/projects/$PID/activity?limit=5 | jq  # each event present once
```

**Prove replay is idempotent** — force a redelivery:

```bash
cairn events replay --session $SID --force
```

Every result is `duplicate`, and the server event count is unchanged. `duplicate` is a success:
at most one canonical event exists however many times delivery is retried
(`contracts/safe-events.md` §7).

## 5. Restart consolidation mid-flight

```bash
# with a backlog present
systemctl restart cairn-server
curl -s $SERVER/api/system/health | jq '.consolidation'
```

The backlog is reclaimed and completes. Compare the resulting knowledge, relations and
reinforcement counts against an uninterrupted run over the same events: **identical**.
Re-execution after an abandoned claim is expected; a second durable effect from it is a defect
(`contracts/consolidation.md` §4).

## 6. Destroy the local store

With canonical data confirmed server-side:

```bash
cairn memory list > /tmp/before.txt
rm -rf ~/.cairn/cairn.db*
cairn init
cairn link --server https://<host>
cairn memory list > /tmp/after.txt
diff /tmp/before.txt /tmp/after.txt      # durable knowledge: identical
```

Project, personal and team knowledge survive. What does **not** survive, and what Cairn names
rather than hiding:

```bash
cairn status --durability
```

- events spooled but not yet accepted
- machine-local integration state (re-run `cairn integrate`)
- the bounded briefing cache (refills on next retrieval)
- machine-local capture disposition counters (the server's own counters are unaffected)
- local-only knowledge (**gone permanently** — the local-only choice said so at the time)
- retained-local records the server could not accept (**gone permanently**; listed individually
  by `cairn migrate --status`, and outside FR-703's guarantee by construction)
- observations, evidence facts, verification runs, continuity checkpoints (local by design)

Note what is **not** lost: per-session delivered-context tracking lives on the server, so a
session is not re-briefed with everything it already received.

## 7. Migration and cutover

On an installation still at Feature 004:

```bash
cairn migrate --dry-run          # inspect: what exists, what will move, what cannot
cairn migrate                    # drain → verify possession → switch → demote
cairn migrate --status
```

Nothing is demoted before canonical possession is confirmed for the records concerned. Records
the server cannot accept are retained locally and reported individually.

After the admin cuts the server over:

```bash
cairn-server authority cutover
```

A legacy client's knowledge sync is refused with `upgrade_required`, and its local data is
untouched:

```bash
cairn sync                       # → upgrade_required
cairn memory list                # → unchanged, still readable
```

## 8. Success gates

The run has passed when all of these hold:

- [ ] Durable knowledge exists from a session that invoked no Cairn tool
- [ ] Its provenance resolves to real, retrievable events
- [ ] A second session received relevant knowledge with no tool call
- [ ] The prompt-time briefing did not restate the session-open briefing
- [ ] Every briefing was within budget
- [ ] The web interface traced the whole path without a log or a database query
- [ ] The agent never stalled while the server was gone
- [ ] Replay produced only `duplicate`
- [ ] A mid-flight restart changed no durable outcome
- [ ] Destroying the local store lost nothing the server had accepted
- [ ] Cairn named exactly what the deletion did lose
- [ ] Health reported configuration and runtime capture as different things
- [ ] Receipt reported `unavailable / no evidence`, not a green check
