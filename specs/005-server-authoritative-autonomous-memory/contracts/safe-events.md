# Contract — Safe Canonical Events

The boundary between a developer's machine and Cairn Server. Everything the server learns
about agent activity arrives here, in this shape, or not at all.

## 1. Why this is not `/api/sync/batch`

The existing synchronization boundary refuses `path`, `command`, `summary`, `exit_code`,
`details` and twenty other field names **recursively at any depth**, and refuses nine entity
types outright (`crates/cairn-server/src/sync.rs:27-78`). Its own comment states the intent:
*"This list **is** the privacy boundary, stated once."*

A semantic event cannot be expressed under those refusals. Reusing that endpoint would mean
weakening the one place the privacy boundary is written down. So Feature 005 adds a second,
separately-governed boundary and leaves the first exactly as it is (FR-765, FR-766, SC-731).

## 2. Field naming map

No field name here may collide with a refused name (FR-777a1). The substitutions are fixed
once, in this table, rather than settled per implementation.

| Meaning | Refused name | Field used here |
|---|---|---|
| file identity | `path`, `path_fingerprints` | `repo_file` |
| rename source | — | `repo_file_from` |
| shell command | `command` | `command_line` |
| test invocation | `command` | `test_command` |
| process exit status | `exit_code` | `exit_status` |
| test verdict | `outcome` | `test_outcome` |
| failure description | `details`, `detail` | `failure_note` |
| failure classification | — | `failure_kind` |
| human-readable gist | `summary` | *(no equivalent — deliberately absent)* |

`close_reason`, `compaction_trigger`, `open_trigger`, `change_kind`, `file_identity`,
`failure_kind`, `test_outcome`, `resource_kind` and `tool_class` are **closed enumerations**,
not free strings. `vendor_event`, `vendor_tool` and `subagent_kind` are sanitized to
`[A-Za-z0-9_.-]` and truncated, as `normalize_vendor_tool` already does. `subagent_ref` is an
opaque identifier, sanitized the same way — it MUST NOT be sourced from any vendor field that
carries an assistant- or user-authored description. Only three fields are free text, and all
three pass redaction on the client and screening on the server: `command_line`, `test_command`,
`failure_note`.

There is no substitute for `summary`. A per-event human-readable gist is the field into which
transcript content would inevitably leak, and the model does not need it: kind plus typed
content carries the semantics.

## 3. Envelope

```json
{
  "event_id": "0e5f…",
  "contract_version": 1,
  "kind": "file_changed",
  "agent": "claude_code",
  "vendor_event": "PostToolUse",
  "session_id": "9f1c…",
  "session_seq": 42,
  "occurred_at": "2026-08-30T09:14:22Z",
  "content": { "repo_file": "crates/cairnd/src/sync.rs",
               "change_kind": "modified",
               "file_identity": "present" }
}
```

`agent_session_key` is absent by design: it is on `FORBIDDEN_SESSION_FIELDS` and the server
has never had a column for it. The **synced session UUID** travels instead.

`project_id` and `account_id` are absent by design. They are bound server-side from the
authenticated credential and the verified session (FR-769, FR-769a, Principle XI). A client
that could name them could attribute another account's work.

## 4. Identity

```
event_id = UUIDv5(CAIRN_EVENT_NS, session_id ‖ session_seq)
```

`session_seq` is assigned by the **daemon**, not the hook, inside the same SQLite transaction
that spools the event:

```sql
BEGIN IMMEDIATE;
  UPDATE session_event_seq SET next_seq = next_seq + 1
   WHERE session_id = ? RETURNING next_seq - 1;   -- durable; never MAX() over the spool
  INSERT INTO event_spool (...) VALUES (...);
COMMIT;
```

The counter is durable and independent of the spool. Deriving it from `MAX(session_seq)` over
`event_spool` would reset once rows drained or were shed under the capacity policy, re-deriving
a used `event_id` that the server answers `duplicate` — silently discarding a real event.
Identity keys on `session_id`, never the vendor key, because shipped code frees the vendor key
on deletion so it can be reused (`crates/cairn-store/src/repo.rs:1610-1616`).

This is the whole solution to "stable identity across separate hook invocations and retries":

- **Separate hook processes** cannot share a counter. They do not need one — each sends a
  canonical event to the daemon, which is a single long-lived process with a transactional
  store. The `UNIQUE (session_id, session_seq)` constraint makes the assignment safe under
  concurrency without any additional locking.
- **Retries** re-send a spooled row whose `event_id` was fixed at spool time. However many
  times it is delivered, it carries one identity, so the server's primary key collapses it
  (FR-770).
- **A genuinely repeated act** — reading the same file twice — gets the next ordinal, so it is
  a distinct event and is not suppressed as a duplicate (FR-738).
- **The clock is not involved.** `occurred_at` is advisory and never participates in identity
  or ordering (FR-780).

## 5. Bounds

Stated as numbers so SC-743 and SC-733 can fail (FR-773).

| Bound | Value | Enforced |
|---|---|---|
| `repo_file` | 1024 bytes, 64 segments | client + server |
| `command_line`, `test_command`, `failure_note` | 512 bytes each | client + server |
| `vendor_event`, `vendor_tool`, `subagent_kind` | 64 chars | client + server |
| event `content` serialized | 8 KiB | client + server |
| whole event serialized | 16 KiB | client + server |
| events per batch | 256 | server |
| request body | 1 MiB | server |

Over-bound values are refused, never truncated. Truncating a `repo_file` could turn a path
outside the repository into one that looks inside it.

## 6. `repo_file` validation

Applied identically on both sides (FR-777d). The client producing it correctly is not the
mechanism by which the rule holds; the server's own check is.

1. non-empty
2. no leading `/`
3. no `..` segment
4. no drive-letter prefix (`C:`)
5. no UNC prefix (`\\`)
6. separators normalized to `/`
7. no empty interior segment
8. within both bounds

Four dispositions (FR-777e–g). An absolute path **inside** the repository is the ordinary case
and is relativized locally against the repository root; the root itself is machine
configuration and never crosses (FR-753).

| Vendor supplies | `file_identity` | `repo_file` |
|---|---|---|
| absolute path inside repo | `present` | relativized locally |
| path outside repo | `out_of_repository` | absent |
| nothing | `unavailable_from_vendor` | absent |
| absolute value on the wire | *(refused)* | server rejects |

## 7. Ingest API

```
POST /api/events/batch
Authorization: Bearer <token>
Content-Type: application/json

{ "contract_version": 1, "events": [ …≤256 events… ] }
```

Response — per-event outcomes so a client can retry precisely what needs retrying (FR-771):

```json
{ "results": [
  { "event_id": "0e5f…", "status": "accepted"  },
  { "event_id": "1a2b…", "status": "duplicate" },
  { "event_id": "3c4d…", "status": "rejected", "reason": "repo_file_absolute" }
]}
```

`duplicate` is a success. A retry that returns `duplicate` has achieved exactly what it was
for: at most one canonical event exists (FR-770, FR-786).

### 7.1 Server-side validation order

Each event, independently, in one transaction:

1. **Schema** — unknown field ⇒ `rejected: unknown_field` (FR-767). Strict deserialization;
   the schema is closed.
2. **Refused-name check** — the event carries a name the sync boundary refuses ⇒
   `rejected: forbidden_field_name`. This is enforced independently of the client (FR-777, FR-777a1).
3. **Bounds** — §5.
4. **`repo_file`** — §6.
5. **Identity re-derivation** — the server recomputes
   `UUIDv5(CAIRN_EVENT_NS, session_id ‖ session_seq)` and refuses a mismatch
   (`event_id_mismatch`). Idempotency must not be client-controlled: otherwise a client could
   submit a colliding id, be answered `duplicate`, and suppress a genuine event.
6. **Session binding** — `session_id` must resolve to a session that exists, and the
   authenticated account must be a member of that session's project. The project is *derived*
   from the session, never asserted. A caller who is not a member of the derived project gets
   `403` for the request, not a per-item rejection — a per-item answer would confirm the
   session's existence to a non-member (FR-769a, FR-894a).
7. **Content screening** — the server applies the **same secret-pattern check** to
   `command_line`, `test_command` and `failure_note` that the client applied, and refuses a
   match (`content_screening_failed`). Client-side redaction is where secrets are removed; this
   is where the boundary is *enforced*. FR-777 requires the server to enforce the privacy
   restrictions independently of the client, and a credential inside an approved text field is
   exactly the case SC-741 names.
8. **Insert** — `INSERT … ON CONFLICT (event_id) DO NOTHING`. Zero rows affected ⇒ `duplicate`.
9. **Enqueue** — `INSERT INTO consolidation_work (event_id, state) VALUES (?, 'pending')`, in
   the same transaction, so an accepted event is always eventually consolidated and a rolled
   back event never is.

### 7.2 Rejection vocabulary

`unknown_field`, `forbidden_field_name`, `bound_exceeded`, `repo_file_absolute`,
`repo_file_traversal`, `repo_file_malformed`, `event_id_mismatch`, `session_not_found`,
`content_screening_failed`, `unsupported_kind`, `contract_version_unsupported`.

Project non-membership is a request-level `403`, not an item rejection (step 6).

A rejection record carries the reason and never the content that caused it (FR-741).

### 7.3 Versioning

`contract_version` is per-event. A server that does not support a version refuses those events
with `contract_version_unsupported` — a permanent refusal the client can recognise and defer,
matching how the existing capability mechanism handles entities it cannot yet store (FR-774,
FR-775). Adding a kind does not invalidate stored events (FR-743).

## 8. Edge spool

`event_spool` (data-model.md §5) reuses the outbox claim protocol, which already solves
per-author claiming, stale-claim reclaim and backoff.

- **Claim**: `UPDATE … WHERE state='pending' AND account_id = ? AND next_attempt_at <= now()
  … RETURNING *`, oldest first. The `account_id` match is **exact**. A row with no recorded
  author is *not* deliverable under whichever account is signed in — that specific regression
  has been introduced and repaired twice in this repository (FR-790, FR-864a).
- **Backoff**: exponential from 1s to 5 minutes, per project namespace.
- **Permanent refusal**: a `rejected` result moves the row to `refused`. It is never retried
  and becomes visible (FR-772, FR-784).
- **Capacity**: 50,000 events or 256 MiB, whichever binds first. On overflow, drop the
  **oldest capture-class** rows; never drop a row with `boundary_class = 1` (session open and
  close, compaction), because those route everything else. Each drop increments
  `spool_overflow_dropped` and is reportable (FR-785, FR-792).
- **Credential change**: rows stay bound to the account that authored them. A different signed-in
  account neither delivers nor sees them.

## 9. Fail-soft and capture health

A capture-class event that misses its deadline is dropped. The hook still exits successfully
and never blocks the agent (FR-749b) — **and** Cairn records `capture_deadline_exceeded` and
surfaces it in health and counters (FR-749c). Fail-soft describes what the agent experiences,
not what Cairn is permitted to know about itself.

The disposition record carries the kind, agent and session, and nothing from the payload being
processed when the deadline expired (FR-749d).

## 10. What never crosses

Raw vendor payloads, conversation transcripts, raw tool output, secrets, absolute local paths,
machine configuration, arbitrary vendor JSON. The local pipeline is: parse → normalize →
redact → deterministic privacy checks → construct → spool. Raw material lives in memory for the
duration of that work and is never written to durable local storage (FR-730, FR-763).
