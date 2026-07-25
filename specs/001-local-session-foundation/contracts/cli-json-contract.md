# CLI JSON Contract (v1)

**Feature**: 001-local-session-foundation | **Date**: 2026-07-16
**Binary**: `cairn` | **Machine mode**: global `--json` flag (FR-030)

## Envelope

Every command in `--json` mode prints exactly one JSON object to stdout:

```json
{"schema": "cairn.cli.v1", "ok": true,  "command": "session.start", "data": { … }}
{"schema": "cairn.cli.v1", "ok": false, "command": "init", "error": {"code": "NOT_A_REPOSITORY", "message": "…", "data": { … }}}
```

Rules:
- `schema` is a stability marker: additive-only changes within `cairn.cli.v1`; field
  removal/retyping requires `cairn.cli.v2`.
- Diagnostics/log lines go to stderr only; stdout carries the envelope exclusively.
- Error `code` values are the IPC error codes ([ipc-contract.md](ipc-contract.md)) plus
  CLI-only codes: `DAEMON_UNAVAILABLE`, `USAGE`.
- Human mode (default) has no stability guarantee.

## Exit codes (stable, both modes)

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Operation failed (see `error.code` — incl. `LEASE_MISMATCH`, `LEASE_EXPIRED`, `GRACE_EXPIRED`, `WATCHER_START_FAILED`) |
| 2 | Usage error (bad flags/args) |
| 3 | Not a Git repository / bare repository (`NOT_A_WORKTREE`) / not registered |
| 4 | Ambiguous session selection (FR-036) |
| 5 | Daemon unavailable after auto-spawn retry |
| 6 | Local state corrupted (FR-033) |

## Commands (FR-029)

| Command | IPC method(s) | `data` payload |
|---|---|---|
| `cairn init` | `v1.repository.register` | `{repository, worktree, created}` |
| `cairn status [--ignored] [--cursor C]` | `v1.repository.inspect` (+ `v1.repository.ignored_files` when `--ignored`) | inspection object incl. `ignored_summary`; with `--ignored`: `{paths, next_cursor}` |
| `cairn session start --agent <type> [--agent-instance <uuid>] [--agent-pid <pid>]` | `v1.session.start` | `{session, resume_token, outcome}` — token emitted in `--json` only; success is emitted only after watcher-ready acknowledgement and post-install Git reconciliation; readiness failure emits `WATCHER_START_FAILED` with schema-constrained `data: {kind:"watcher_start_failure", stage:"install|reconcile"}`, exit 1 |
| `cairn session show [--session ID] [--agent-instance ID] [--agent-type T]` | `v1.session.get` | `{resolution, session?, candidates?}` — `resolution:"ambiguous"` sets exit code 4 |
| `cairn session heartbeat --session ID [--agent-instance ID]` + token via secure input | `v1.session.heartbeat` | `{state, last_heartbeat_at, lease_expires_at}` |
| `cairn session reattach --session ID [--agent-instance ID]` + token via secure input | `v1.session.reattach` | `{session, fresh_snapshot}` — new token in `--json` only |
| `cairn session stop [--session ID] [--agent-instance ID]` | `v1.session.stop` | `{session}` |
| `cairn daemon status` | `v1.daemon.status` | daemon status object |

Agent instance resolution order: `--agent-instance` flag → `CAIRN_AGENT_INSTANCE` env
var → absent (adaptive cardinality per FR-036; `session start` without an instance id
generates one and prints it in `data.session.agent_instance_id`).

### Resume token handling (FR-029, analysis U3)

- Tokens are NEVER accepted as ordinary command-line arguments (process-listing
  exposure) and NEVER printed in human-readable output or logs.
- Secure input, resolution order: `--resume-token-stdin` (read one line from stdin,
  recommended for scripts/adapters) → `CAIRN_RESUME_TOKEN` inherited environment
  variable → `--resume-token-file <path>` (0600-permission file).
- Token issuance: `session start` and `session reattach` return the token **only** in
  `--json` mode (`data.resume_token`); human mode prints `resume token issued — rerun
  with --json to capture, or use CAIRN_RESUME_TOKEN`.
- CLI heartbeat/reattach exercise the same daemon handlers agent adapters will call
  directly over IPC later.

## Examples

```console
$ cairn init --json
{"schema":"cairn.cli.v1","ok":true,"command":"init","data":{"repository":{"repository_id":"019…","repo_uuid":"7f3…","canonical_path":"D:/code/app","default_remote":{"name":"origin","url":"git@…"}},"worktree":{"worktree_id":"019…","path":"D:/code/app","is_main":true},"created":true}}

$ cairn session show --json      # two live sessions, no selector
{"schema":"cairn.cli.v1","ok":true,"command":"session.show","data":{"resolution":"ambiguous","candidates":[{"session_id":"019…","agent_type":"claude-code","agent_instance_id":"a1…","state":"active","started_at":"2026-07-16T04:12:09.120Z"},{"session_id":"019…","agent_type":"cursor","agent_instance_id":"b2…","state":"recovering","started_at":"2026-07-16T03:58:44.003Z"}]}}
# exit code: 4
```

## Contract tests

- Golden `--json` outputs for every command × success/failure path, validated against
  schemas exported by `cairn-protocol` (same DTOs as IPC ⇒ single source of truth).
- SC-008 test: scripted consumer parses all eight commands' outputs across repeated runs,
  including `WATCHER_START_FAILED` session-start envelopes for both `stage=install` and
  `stage=reconcile`.
- CLI JSON-envelope goldens cover both watcher stages, validate `kind` and `stage`, and
  assert no raw internal error details, paths, repository contents, environment values,
  or tokens leak.
- CLI integration tests prove both watcher stages map to exit code 1.
- The shared schema-breaking-change tripwire rejects removal/retyping or widening of the
  typed watcher-failure payload.
