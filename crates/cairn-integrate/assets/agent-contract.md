schema = 1
heading = Cairn — persistent project memory
lede = Cairn is durable, project-scoped memory for this repository, shared by every agent working on it.
mcp_lede = Cairn is durable, project-scoped memory for this repository, shared by every agent working on it.

[rule context]
block = Read the Cairn context you were given before re-deriving the project.
mcp = Call `cairn_context` first and use what it returns before re-deriving the project.

[rule search]
block = Search Cairn memory before repeating an investigation you may already have done.
mcp = Call `cairn_search` before repeating an investigation you may already have done.

[rule record]
block = Record durable facts, decisions, conventions, failures and procedures — never routine tool calls.
mcp = Record durable facts, decisions, conventions, failures and procedures with `cairn_remember` — never routine tool calls.

[rule scope]
block = Use the narrowest correct scope: task, else branch, else project.
mcp = Use the narrowest correct scope: task, else branch, else project.

[rule evidence]
block = Never invent an evidence observation identifier.
mcp = Never invent an evidence observation identifier.

[rule secrets]
block = Never put secrets, credentials, raw prompts or unbounded output into memory.
mcp = Never put secrets, credentials, raw prompts or unbounded output into memory.

[rule lifecycle]
block = Session boundaries, checkpoints and handoffs are automatic here. Do not hand-roll them.
mcp = Open and close sessions with `cairn_session`; this client has no automatic lifecycle.

[rule task]
block = Bind work to a Cairn task when one applies.
mcp = Bind work to a Cairn task when one applies.

[rule depth]
block = For deeper workflows, use the Cairn Skill.
mcp = For deeper workflows, call `cairn_handoff` for what the last session left you.
