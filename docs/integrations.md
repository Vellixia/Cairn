# Agent integration ownership

`cairn setup` installs supported-agent hooks, MCP configuration, instructions, and
skill resources needed by the detected repository. No separate connect, repair,
doctor, distribution, or disconnect command exists.

## Ownership rule

Cairn records the exact resources and bytes it wrote. A later explicit `cairn setup`
rerun may update a resource only while its Cairn-owned portion still matches that
record. Unrelated user configuration remains untouched.

If a user edits Cairn-owned bytes, setup reports a conflict and leaves the file
unchanged. There is no background repair loop and no force-overwrite path. Resolve
the conflict, then rerun setup.

Removing Cairn is manual: remove only resources named in the last setup report, after
checking their current contents. Memory and server data are unaffected.

## Installed machine adapters

Supported integrations invoke two hidden commands:

- `cairn hook <event>` for bounded lifecycle and capture events;
- `cairn mcp` for the five MCP tools.

Hooks return after local spool acceptance or explicit rejection. `cairnd` owns later
delivery and retry, so capture does not depend on the agent process remaining alive.

MCP configuration exposes only `cairn_context`, `cairn_search`, `cairn_remember`,
`cairn_session`, and `cairn_handoff`.

## Repository scope

Setup requires a Git repository with a configured remote. Server project selection is
limited to projects the credential already belongs to and whose registered remote
matches. A clone does not install anything automatically; each machine runs
`cairn setup` explicitly.

Committed `.claude/settings.json`, `AGENTS.md`, and `CLAUDE.md` remain user-owned and
are not rewritten by V1 setup.

## Credentials

Create a revocable API token in web **Settings**. Pass it to setup through protected
environment input or JSON stdin. Setup verifies the token with the server, derives the
account from authentication, saves the token with private filesystem permissions, and
cannot create accounts or membership.

Revoking the token stops delivery. Authentication denial invalidates matching cached
context immediately. Ordinary network failure may use only eligible finite-age cache.
