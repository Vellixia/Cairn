# When Cairn reports a problem

Cairn diagnoses its own integration. It reports what is wrong per resource rather than failing
generically.

## What to do

Run `cairn doctor`. It reports, for each connected agent: whether it is detected, its version
and compatibility, its integration level, which lifecycle coverage is present and which is
absent, and the state of every Cairn-owned resource.

Each reported condition names its own remedy. Common ones:

- **outdated** — a Cairn artifact is behind this build. `cairn repair` upgrades it in place.
- **missing** — a resource Cairn recorded is gone. `cairn repair` restores it.
- **modified** — someone edited Cairn's own block by hand. Cairn reports it and changes
  nothing by default, because that edit may have been deliberate.
- **installed_not_activated** — the handlers are installed but the agent has not been told to
  trust them. The remedy names the exact step to run inside that agent.
- **conflicting_owner** — the resource exists under an owner Cairn did not record. Cairn will
  not adopt or delete it; a human decides.

## What not to do

Do not edit the content between Cairn's `cairn:managed` markers by hand. Do not delete
configuration to "reset" the integration — `cairn repair` exists precisely so that is never
necessary, and `cairn disconnect` removes Cairn's own resources without touching anything else.

Nothing in this file is a reason to touch another tool's configuration.
