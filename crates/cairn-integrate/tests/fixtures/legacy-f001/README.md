# A Feature 001 installation, as it was left on disk

Used by the `quickstart.md` US2 walkthrough: copy these into a repository, run
`cairn connect claude-code`, and nothing should be duplicated, relocated, or
disturbed.

What each part is here to prove:

- The six hook events Feature 001 registered, in its exact entry shape. Cairn
  recognizes them by that shape and adopts them **in place**, at the scope they
  are already at — `.claude/settings.json` is project-shared, and the upgrade
  does not move it to the Feature 002 default (FR-217).
- `.mcp.json` holds a Cairn entry beside a developer's own server. Exactly one
  Cairn entry exists afterwards, and `internal` is untouched (SC-103).
- The developer's own `PostToolUse` matcher, their permissions and their model
  choice are unrelated settings that must come through byte-identical.
- The `UserPromptSubmit` hook merely *mentions* `cairn hook` inside a longer
  shell command. It is not Cairn's, it is never adopted, and it is never
  removed — the legacy bridge matches exact shapes and nothing else (FR-139).
