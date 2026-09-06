# Quickstart run — Feature 005 (T164)

Executed 2026-09-06 at commit `8e8a0dc`, on macOS (Darwin 25.6.0, Apple silicon),
against `cairn` and `cairn-server` built from that tree.

## What this file is

`quickstart.md` is the operator-facing walkthrough. T164 requires it to be run
rather than read, because a walkthrough nobody executes drifts from the product
silently — and this one had. Every command below was executed; the ones that did
not exist are recorded as corrections made to `quickstart.md`, not as failures
of the run.

## Corrections made to `quickstart.md`

The walkthrough named 10 commands the shipped CLI does not have. Each was
replaced with the surface that exists, and the walkthrough now matches
`cairn --help`.

| Quickstart said | The shipped surface | Why it differs |
|---|---|---|
| `cairn integrate claude-code` | `cairn connect claude-code --yes` | `integrate` was never the verb; `integration` is the rare-operations group |
| `cairn memory list` | `cairn memory search` | there is no `list`; `search` with no filter is the listing |
| `cairn memory list --origin consolidated` | `cairn memory search --json \| jq …origin_kind` | `--origin` is not a filter flag; the field is on the record |
| `cairn events tail` | `cairn sync status --json \| jq '.namespaces'` | there is no `events` surface. Safe events are not read locally one at a time; spool depth and the server's activity feed are what the product offers |
| `cairn events replay --session … --force` | `cairn sync now` | redelivery is a drain, and it is idempotent by construction |
| `cairn status --capture` | `cairn doctor --json \| jq '.dispositions'` | `status` takes only `--json` |
| `cairn status --durability` | `cairn doctor --durability` | the durability inventory lives on `doctor` |
| `cairn memory show <id> --provenance` | `cairn memory show <id> --json \| jq '.provenance'` | `show` takes only `--json` |
| `cairn migrate --dry-run` / bare `cairn migrate` | `cairn migrate --inspect` / `--run` | the migration surface is five explicit flags, one per invocation |
| `cairn-server authority cutover` | `POST /api/admin/cutover` | the server's only CLI group is `users`; cutover is an admin route |

## Observations, as run

On a fresh repository with no server attached:

```
$ cairn init
Cairn is tracking tmp.3irSVrOGZo.

$ cairn status
Project      tmp.3irSVrOGZo (01a07346-…)
Sharing      local only
Integration  manual-mcp
Daemon       running
Recorded     0 observations, 0 memories

$ cairn sync status
Linked       no
Pending      0
Namespaces
  project:01a07346-…  current  pending=0 failed=0 blocked=0

$ cairn memory search
No matching memory.
```

The starting state the scenario asks for — zero durable memories — is what the
product reports, and it reports "local only" rather than implying a server.

```
$ cairn doctor --durability
lost for good if this store is deleted:
  writer identity                         1
  authority and migration state           1

restored from the server on the next pull:
  projects                                1
  accounts                                1

queued, accepted for delivery, not yet durable:
  (nothing)

caches:
  (no lane has been established yet)
```

The four headings are the durability answer: what is lost, what returns, what is
in flight, and what is a cache. A category with no rows still prints, which is
what stops an omission being read as an assurance (SC-714).

```
$ cairn migrate --inspect
migration inspect (nothing has been changed)
…
patterns eligible for an ownership claim: 0

$ cairn migrate --status
authority: migrating

phases:
  inspect                  done      0

retained locally: 0
```

`--inspect` is phase 1 and its postcondition is `migrating`, which `--status`
then reports. A fresh store has nothing to migrate and passes through the phases
without claiming to have moved anything.

## What the automated suites cover instead of this file

Sections 1–6 of the walkthrough — capture, consolidation, retrieval, the web
trail, the outage and the local-store loss — are executed end to end by
`tests/tests/feature005_end_to_end.rs` (T156) against a real server, and the
web trail by `web/e2e/feature005-control-plane.spec.ts`. This file records the
operator-visible surface; those record the behaviour.
