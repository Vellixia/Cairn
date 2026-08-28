# Knowledge domains: project, personal, team

Cairn holds three kinds of durable knowledge. They are not scopes. A **scope** (`project`,
`branch`, `task`, `session`) says how long a project memory stays relevant; a **domain** says
whose knowledge it is and how far it travels. The two are independent, and nothing you do here
changes a scope.

| Domain | Whose | Travels to | Authored by |
|---|---|---|---|
| `project` | this repository | everyone with access to the project | you, directly — the default |
| `personal` | your account | every project and machine you sign in from | you, directly or by promotion |
| `team` | everyone on the server | every account, regardless of project membership | proposed by anyone, made authoritative only by a human administrator |

## Recording personal knowledge

Use `domain: "personal"` on `cairn_remember` `action: "create"` when what you learned is true
of *you* rather than of this repository — a habit, a preference, a lesson that would apply on
your next project too.

```json
{ "action": "create", "domain": "personal", "type": "convention",
  "content": "Read the failing test before the implementation, not after" }
```

If it is true of *this* repository, it is project memory and the domain field does not belong
on it. "The retry backoff here is exponential" is project knowledge. "I always check the
backoff before assuming a timeout" is personal.

Personal records are immutable once written. The one permitted change is forgetting them —
`action: "forget"` with `domain: "personal"` — which clears the content and leaves the record
as a tombstone, so the machine that already synchronized it learns it is gone.

## Promoting a project memory

`action: "promote"` with `target: "personal"` copies a project memory into your personal
domain. `target: "team"` proposes it as team guidance. In both cases the original project
memory is untouched and stays where it is: promotion **copies**, and nothing links the two
afterwards, so forgetting the original later leaves the promoted record alone.

Add conditions with `applicability_facts`, as `kind=value` strings. `kind` is `language` or
`tool` and nothing else — anything outside that is refused, not quietly dropped, because a
dropped condition means the record starts applying more widely than you asked for.

```json
{ "action": "promote", "target": "personal", "memory_id": "…",
  "applicability_facts": ["language=rust", "tool=cargo"] }
```

A record with no conditions applies everywhere. That is usually what you want.

## Team knowledge, and the line you cannot cross

**No agent action makes team guidance authoritative.** `promote` with `target: "team"` creates
a *proposal*, invisible to every recall path including your own, and it stays that way until a
human administrator runs `cairn team ratify`. There is no tool action shaped like
ratification, and this is deliberate: an agent may propose how a whole team should work; only a
person decides that it does.

`domain: "team"` is refused on `create` for the same reason. Proposals come from `promote`, or
from a person running `cairn team propose`.

## What these two domains will not accept

Personal and team records are stripped of anything that identifies where they came from. A
write is refused — locally when you make it, and again at the server when it arrives — if it
carries:

- an absolute path, a home-directory reference, a drive-letter path or a `file://` URL
- a URL with credentials in it, or an environment-variable assignment
- a long run that looks like an encoded secret
- a token that names the project you are working in
- a shell command invocation

The refusal names which of those it tripped and never quotes your content back. Rewrite the
claim so it stands on its own: "clear the build cache when a stale artifact is suspected" says
the same useful thing as a sentence naming a path under your home directory, and it is true on
every machine rather than one.

## Reading them back

`cairn_search` returns three sibling arrays — `results` for project memory, `personal` and
`team` — never merged, and `total` counts project results alone. Narrow with `domains`.

`cairn_context` includes both domains last, after every project section, capped at a small
share of the budget, and excludes them entirely at `depth: "minimum"`. You do not need to
manage that: it is arranged so that personal and team knowledge can never take space this
project's own context would have used.
