# Contract — Knowledge Commands and the Post-Cutover Mutation Model

How durable knowledge is mutated once the server owns it. This contract exists because
FR-701 and FR-712 cover project, personal **and** team knowledge, and the migration contract
originally retired only the personal/team lanes.

## 1. What current `main` actually allows

Not a design assumption — an audit of `crates/cairn-server/src/sync.rs` at `f76a9fe`.

For `entity_type = "memory"`, the upsert (`sync.rs:669-693`) lets a client-supplied payload set
or overwrite: `type`, `scope`, `scope_key`, `content`, `state`, `superseded_by_id`,
`observation_ids`, `evidence_count`, `topic_key`, `value_key`, `importance`, `pinned`,
`reinforcement_count`, `distinct_origin_count`, `verification`, `verification_authority`,
`last_verified_at`, `verification_basis`, `evidence_fact_count`.

Two consequences worth stating plainly:

- **Project memory is not server-authoritative today.** The conflict predicate is
  `ON CONFLICT (id) DO UPDATE SET … WHERE memories.project_id = $2` (`sync.rs:678,693`) —
  scoped to the project, not to the author. Any member of a project can overwrite any other
  member's memory content, state and verification by naming its id.
- **Derived values are client-supplied.** `reinforcement_count` and `distinct_origin_count` are
  bound as raw integers (`sync.rs:716-717`); verification state, authority, basis and fact count
  are taken verbatim from the payload (`sync.rs:718-731`) with no parsing into the Rust enums
  and no re-derivation. The doc comment says the receiving daemon maps it; nothing enforces that.

So the claim "the server already owns project memory" would be false, and this contract does
not make it.

## 2. Which sync writes are refused after cutover

The endpoint refuses **knowledge-bearing** entity types and keeps the rest.

| `entity_type` | After cutover | Why |
|---|---|---|
| `memory` | **refused** — `upgrade_required` | durable project knowledge (FR-701) |
| `memory_relation` | **refused** | a decision about knowledge |
| `personal_knowledge` | **refused** | durable personal knowledge |
| `team_knowledge` | **refused** | durable team knowledge |
| `delete` naming any of the four | **refused** | a tombstone is a knowledge mutation |
| `project` | allowed | project metadata (name, remote) |
| `task`, `task_criterion`, `task_blocker` | allowed | work tracking, not durable knowledge |
| `session` | allowed | session bookkeeping |
| `handoff` | allowed | continuity artifact, not the knowledge store |
| `delete` naming those | allowed | same reasoning |

FR-701 is about durable knowledge. Tasks, sessions and handoffs are not durable knowledge and
their sync path is untouched, which is also what keeps the blast radius of cutover small.

## 3. The replacement: commands, not upserts

Every refused write has a command API. The difference is the point: a command states an
**intent** and the server computes the consequences. A client can no longer send a row and have
the server store it.

| Command | Method / path | Replaces |
|---|---|---|
| Create project memory | `POST /api/projects/{id}/memories` | `memory` upsert (create) |
| Supersede | `POST /api/memories/{id}/supersede` | client-set `state` + `superseded_by_id` |
| Reinforce | `POST /api/memories/{id}/reinforce` | client-set `reinforcement_count` |
| Record relation | `POST /api/projects/{id}/memory-relations` | `memory_relation` upsert |
| Pin / unpin | `POST /api/memories/{id}/pin` | client-set `pinned` |
| Forget project memory | `DELETE /api/memories/{id}` *(exists)* | `delete` tombstone |
| Create personal knowledge | `POST /api/personal/knowledge` | `personal_knowledge` upsert |
| Forget personal knowledge | `POST /api/personal/knowledge/{id}/forget` | tombstone |
| Propose team knowledge | `POST /api/team/knowledge` | `team_knowledge` upsert |
| Ratify / retire | `POST /api/team/{id}/ratify` · `/retire` *(exist)* | — |
| Report a verification run | `POST /api/verification/reports` | client-set verification fields |
| Promote a reusable pattern | `POST /api/patterns` | *(no prior path — patterns never synced)* |
| Forget a pattern | `POST /api/patterns/{id}/forget` | — |

### 3.1 What a command may not carry

A command body carries the author's **intent** only. These are computed server-side and are
rejected if present in a request:

`state`, `superseded_by_id`, `superseded_at`, `stale_at`, `reinforcement_count`,
`distinct_origin_count`, `evidence_count`, `evidence_fact_count`, `verification`,
`verification_authority`, `last_verified_at`, `verification_basis`, `created_at`, `updated_at`.

Identity is bound from the credential, never the body (Principle XI): `origin_session_id` is
resolved from the caller's session, `owner_user_id` for personal knowledge from the
authenticated account, `proposed_by_user_id` for a team proposal likewise.

### 3.2 Authorization, per command

| Command | Check |
|---|---|
| create / relate / pin / reinforce project memory | `require_member` on the project |
| supersede | `require_member`; both memories in the same project |
| delete project memory | `require_member`; the existing route's own check |
| personal create / forget | caller **is** the owner; there is no other owner to name |
| team propose | authenticated; attribution bound from the credential |
| ratify / retire | `AdminUser`, and the existing compare-and-swap on current state |

Cross-member overwrite disappears with the upsert: there is no command that replaces another
member's memory content. Correcting someone else's knowledge is `supersede`, which creates a
new record and links it — visible, attributed and reversible, rather than a silent overwrite.

## 3.3 Reusable patterns

Patterns are the one domain with no prior server path at all: `reusable_pattern` is a refused
entity type and its local row carries three refused field names, so it has never travelled.
FR-708b requires the refusal to be lifted **only** for a redefined representation, and that is
what `shared_patterns` is (`data-model.md` §6). The local table is not relaxed and is not sent.

**Promotion.** `POST /api/patterns` carries the safe shape only: title, problem, root cause,
approach, constraints, applicability. The client derives it from the local row and **drops**
`signals`, `signal_digest`, `origin_ref`, `sanitization_report`, `source_memory_id` and
`origin_deleted`. Promotion passes the existing global-content validator, so a pattern that
names its source project is refused exactly as personal or team knowledge would be. `trust`
may be `sanitized`, `validated` or `contested` — never `candidate`, which is a local-only
state meaning "not yet fit to leave".

**Authorship.** `account_id` is bound from the credential (Principle XI). The originating
project is never transmitted; the local salted origin digest stays local (FR-708a).

**Retrieval and cache.** Patterns keep exactly the budget treatment they have today: the
`patterns` section, in its existing `SECTION_ORDER` position, admitted from the general pool by
`take_while_fits` (`crates/cairn-core/src/context.rs:270-274`). They are **not** under
`GLOBAL_SHARE_MAX`, which applies only to `personal_notes` and `team_guidance`
(`admit_global`, `context.rs:439-445`). Making patterns server-durable changes where they are
stored, not how they are budgeted; moving them under the global cap would be a behaviour change
no requirement asks for. Locally they are cache, refilled from the server like any other
non-authoritative copy, and labelled as such.

**Deletion.** `POST /api/patterns/{id}/forget` sets `forgotten_at` and clears the content
columns, the same tombstone shape personal knowledge uses. Only the author may forget their own
pattern. A forgotten pattern stops being retrieved and stops being served in the changes feed.

**Pattern applications stay local** (FR-707). They record what happened on one machine when a
pattern was applied, are evidence rather than knowledge, and have no server table.

**Migration.** Existing local patterns are promoted during migration phase 2 like any other
knowledge, through the drain route, and are subject to the same possession verification before
any local demotion. A pattern that fails validation — most likely because it names its source
project — is **retained local** with reason `server_refused` and reported individually, never
silently dropped (FR-871).

## 4. Offline behaviour — commands queue, they do not fail

FR-781 and FR-815a still bind: an agent operation must not block on the server, and an explicit
creation made offline must become a queued write rather than a local durable record.

A command issued while the server is unreachable is written to a **`command_spool`** in SQLite,
alongside `event_spool` and using the same claim protocol:

- `command_id` is deterministic — `UUIDv5(CAIRN_COMMAND_NS, scope_kind ‖ scope_key ‖
  command_seq)` from a durable counter, exactly as event identity works (`safe-events.md` §4).
  Replay is therefore idempotent: the server answers `duplicate` and applies nothing twice.

### 4.1 Sessionless commands

Not every command has a session. The CLI permits memory and task operations outside one, and
shipped code represents that honestly: `authoring_session` returns the nil UUID meaning "no
session", which `cairn task history` renders as an unattributed change
(`crates/cairnd/src/handlers.rs:2522-2545`). Its own comment states the reasoning — *"a CLI
invocation outside any session genuinely has no author to name, and naming a throwaway one
would be worse than naming none."*

The command spool must be able to represent that, so:

- `session_id` is **nullable**. A sessionless command carries no session and no fake one is
  invented.
- Identity is scoped rather than session-keyed. `scope_kind` is `session` or `store`;
  `scope_key` is the session id or the store's `writer_id`. Each scope has its own durable
  counter, so both kinds get a stable, gapless, restart-safe ordinal.
- Ordering guarantees are per scope. Session-bound commands are delivered in session order; a
  sessionless command is ordered against other sessionless commands from the same store. No
  cross-scope ordering is claimed, and none is needed — a sessionless command by definition
  does not participate in a session's causal chain.
- The server records provenance as **unattributed to a session** rather than attributing it to
  one. `origin_session_id` is left null for such a record, which is what the local store already
  does.

This keeps `command_id` stable across retries in both cases and requires no synthetic session
row, which would leave exactly the second active session in the worktree that the shipped
comment warns makes the next agent's context ambiguous.
- Rows are claimed with an **exact** `account_id` match; a row with no recorded author is never
  deliverable under whichever account is signed in.
- The caller is told the command was **accepted for delivery**, not that it is durable
  (FR-815a). `cairn status` shows the pending count.
- A command the server later refuses surfaces to the user; it is not retried forever and it is
  not silently dropped.
- Ordering within a session is preserved by `command_seq`, so a supersede queued after its
  target's creation is delivered after it.

What a queued command is **not**: a local durable record. Nothing in the local store becomes
authoritative because a command is waiting (FR-709, FR-787).

## 5. Migration drains without reopening local authority

Phase 2 of migration delivers legacy-shaped knowledge rows, which §2 refuses. The exemption is
narrow and self-closing (`migration-cutover.md` §12.1):

- `POST /api/migration/drain` accepts the four knowledge entity shapes **only** for a store that
  has registered a migration and presents its migration token.
- It is refused for any store without a registered migration, so it is not a general bypass.
- It closes when the migration completes, so a migrated store cannot keep using it.
- It writes the records as they were authored, preserving author, domain, scope and keys
  (FR-867) — it is a transfer of existing knowledge, not an authoring path.

The distinction that matters: the drain moves knowledge that was already authored under Feature
004's rules into the server's custody. It never lets a client author new knowledge with
client-computed derived state. Once the drain closes, §3's commands are the only way in.

## 6. What this does not change

`cairn_remember`, `cairn_search` and `cairn_context` keep their agent-facing shape. The tool
surface does not grow (a standing product constraint); what changes is that `cairn_remember`
becomes a command to the server rather than a local write that syncs later. Offline it queues,
which is what it already does for everything else.
