# Contract: Server HTTP API

**Feature**: `001-cairn-mvp` | Axum over HTTPS, JSON in and out.

Two audiences: the local daemon (personal API token, `Authorization: Bearer <token>`)
and the web UI (session cookie from email/password sign-in). Both resolve to a user;
every project-scoped route checks membership and returns `403` — not an empty list — to
non-members (FR-057).

**The server accepts only the allowlist in FR-055.** There is no route that accepts
observation content, and any sync item carrying an observation field is rejected with
`rejected`. Project keys on the wire are always the server-assigned `project_id`, never a
local identifier or a filesystem path (FR-064).

Errors are uniform:

```json
{ "error": { "code": "forbidden", "message": "…" } }
```

Codes: `unauthorized`, `forbidden`, `not_found`, `invalid_request`, `conflict`,
`rejected` (permanent, do not retry), `rate_limited`, `internal`.

## Authentication

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/auth/register` | Create a user (email, display name, password) |
| `POST` | `/api/auth/login` | Sign in; sets the session cookie |
| `POST` | `/api/auth/logout` | End the session |
| `GET` | `/api/auth/me` | Current user |
| `GET` | `/api/tokens` | List the user's API tokens (never the plaintext) |
| `POST` | `/api/tokens` | Create a token; the plaintext is returned exactly once |
| `DELETE` | `/api/tokens/{id}` | Revoke |

Passwords are hashed with argon2; tokens are stored hashed and compared in constant time
(D10).

## Linking

How a local clone acquires a shared project identity (FR-064). Two machines at different
paths reach the same project by creating once and joining thereafter.

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/projects` | Create a shared project (name, normalized remote); returns the server-assigned `project_id`; the caller becomes a member |
| `POST` | `/api/projects/{id}/join` | Join an existing shared project by its identifier |
| `GET` | `/api/projects/lookup?remote=…` | Discovery *hint*: shared projects whose normalized remote matches, for the user to confirm |

`lookup` is advisory. It never links anything on its own, it returns only projects the user
may join, and a repository with no remote simply gets an empty list — the user then supplies
the identifier directly. The server never infers project identity from a path.

## Sync

The daemon's only write path. Everything else the daemon does is local.

### `POST /api/sync/batch`

```json
{
  "project_id": "…",
  "items": [
    { "idempotency_key": "…", "entity_type": "memory", "entity_id": "…",
      "operation": "upsert", "payload": { … } }
  ]
}
```

Response:

```json
{
  "results": [
    { "idempotency_key": "…", "status": "applied" },
    { "idempotency_key": "…", "status": "duplicate" },
    { "idempotency_key": "…", "status": "rejected",
      "error": { "code": "invalid_request", "message": "…" } }
  ]
}
```

Rules:

- Each item is applied at most once, keyed by `idempotency_key`. Replaying an entire
  batch returns `duplicate` for every item and changes nothing (FR-056, SC-009).
- The key is claimed atomically, in the same transaction as the change. Two deliveries of
  one key arriving at the same instant produce exactly one `applied` and one `duplicate`
  each thereafter — never an internal error, and never `rejected`.
- Items are applied independently; one rejection does not fail the batch.
- `rejected` is permanent. The daemon moves that outbox row to `failed` and surfaces it
  rather than retrying (FR-058). It is reserved for items that are genuinely invalid —
  observation content, a local-only session field, an unknown or non-member project, an
  unsupported entity or operation. A server-side fault is not one of these: such an item is
  **omitted from `results` entirely**, and the daemon retries any item it finds no result
  for. A transient fault must never be dressed as a permanent rejection.
- `entity_type` is one of `project`, `task`, `session`, `memory`, `handoff`. There is no
  observation entity type. A memory or handoff carries evidence as identifiers, a count, and
  an optional digest; an item whose payload contains observation content — `summary`, `path`,
  `command`, `details` — is `rejected` (FR-055).
- `project_id` is the server-assigned shared identifier. Items referencing an unknown or
  non-member project are `rejected`.
- Session payloads carry minimal provenance only; `worktree_path`, `agent_session_key`,
  `daemon_run_id`, and `last_event_at` are rejected fields.
- A `delete` item is a tombstone: it clears content server-side and is idempotent, matching
  the per-entity semantics in `data-model.md` (FR-052).
- Projects are **not** created implicitly by a sync batch. A batch for an unknown project
  is `rejected`; the daemon must link first through the Linking routes above.

### `GET /api/sync/changes?project_id=…&since=…`

Returns shared records produced by other members since a cursor, so a linked project can
read teammates' memory. Read-only; there is no server-driven write into local state
beyond shared memory and handoffs.

## Read API (web UI)

| Method | Path | Returns |
|---|---|---|
| `GET` | `/api/projects` | Projects the user is a member of |
| `GET` | `/api/projects/{id}` | Overview: repository, active branches, counts, recent activity |
| `GET` | `/api/projects/{id}/tasks` | Tasks, filterable by status |
| `GET` | `/api/projects/{id}/sessions` | Sessions, newest first, with handoff presence |
| `GET` | `/api/sessions/{id}` | Session detail |
| `GET` | `/api/sessions/{id}/handoff` | Latest handoff, full structure |
| `GET` | `/api/projects/{id}/memories` | Search: `q`, `scope`, `scope_key`, `type`, `state`, `limit` |
| `GET` | `/api/memories/{id}` | Memory with provenance: origin session, evidence identifiers and count — no evidence content exists server-side |
| `DELETE` | `/api/memories/{id}` | Soft-delete (FR-062) |
| `GET` | `/api/projects/{id}/sync-status` | Last applied batch, pending count reported by daemons, failures |

Memory search on the server uses PostgreSQL full-text search with the same scope-first
ranking as the local path (D3), so a query behaves the same in the UI and in the agent.

## Not in this API

No streaming, no webhooks, no server-initiated push, **no observation ingest of any
kind**, no organization or role routes. Those are all out of scope for this feature
(FR-055, FR-059, spec Out of Scope).
