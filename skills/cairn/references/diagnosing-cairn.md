# When Cairn reports a problem

Alpha.9 has one human CLI command: `cairn setup`. It links the current Git
repository to an authorized server project and installs Cairn-owned agent
integration. Hooks and MCP tools are machine adapters, not administration
commands.

## Check and repair

From the affected repository, run `cairn --json setup` with valid server access.
Read `data.project.linked` and `data.integrations.warnings` in the result. A
rerun repairs missing or outdated Cairn-owned bytes and leaves matching bytes
unchanged. It does not take ownership of someone else's configuration or
overwrite a user edit: a warning names that conflict, and a human must resolve
it before retrying. Keep the prior file when resolving a conflict.

If setup cannot link the project, check its Git remote against the project
created in web **Settings**, then check the account's membership and API token.
Fix access in web and rerun setup; do not treat an installed hook or a printed
plan as proof that the server accepted anything.

If a hook returned reduced context, use the installed `cairn_context` MCP tool
with the exact `agent_session_key` and `cwd` named in that fallback. Search
with `cairn_search` only after access is restored. An outage or stale cache is
not a successful delivery claim.

## Removing an installation

Alpha.9 has no `disconnect` CLI. Use the last setup report to identify the
Cairn-owned entries, then remove only those entries manually. Preserve other
MCP servers, hooks, instructions, skills, credentials, and user edits. Removing
the local integration does not delete server knowledge or project history.
