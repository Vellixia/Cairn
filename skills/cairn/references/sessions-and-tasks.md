# Sessions and tasks

## Sessions

A Cairn session is one agent's working session on one repository. Where the integration
supports it, sessions open and close automatically and you should not manage them by hand.
Where it does not, open and close them explicitly through Cairn's tools.

Several sessions can be active on one repository at once — two agents, or two worktrees.
That is normal and supported.

## When Cairn reports an ambiguous session

Cairn refuses to guess which session a request belongs to. It returns an ambiguous-session
error naming the candidates. Resolve it by passing the session identifier you are working in,
not by picking one arbitrarily.

## Tasks

A task is a named piece of work with a goal and acceptance criteria. Bind your session to a
task when one applies: it scopes memory correctly and makes the handoff far more useful.

If no task exists and the work is substantial enough to outlive the session, create one first.
