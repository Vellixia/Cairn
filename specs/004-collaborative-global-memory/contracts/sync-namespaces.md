# Contract: Synchronization Namespaces

**Feature**: `004-collaborative-global-memory`

This contract guarantees that project, personal and team synchronization are independent
lanes — a failing or capability-blocked namespace never slows or stalls another — that every
namespace is checked for incoming work on its own schedule rather than only after it has
something to push, that two devices producing byte-identical content are never mistaken for
one write, and that convergence continues to rest on immutable records and deterministic
reconciliation rather than on any clock. It replaces the single process-global cursor and
backoff with per-namespace state, and it fixes the one defect that would otherwise make team
knowledge invisible to a quiet machine forever.

## 1. Three namespaces

**FR-486.** A namespace names one independent synchronization lane:

| Namespace | Identity component | What it carries |
|---|---|---|
| `project:<project_uuid>` | the server-assigned shared project id | memories, tasks, sessions, handoffs, relations, criteria, blockers — unchanged |
| `personal:<server_instance_id>:<user_uuid>` | **both** the server instance and the authenticated user's own id on it (D438, FR-568) | `personal_knowledge`, `personal_knowledge_applicability`, `personal_knowledge_relations` |
| `team:<server_instance_id>` | the server's single, immutable instance id (`identity-administration.md` §8) | `team_knowledge`, `team_knowledge_applicability`, `team_knowledge_relations` (D431) |

**Why `personal:*` carries two components, not one (FR-568).** A user account is per-server —
the same human authenticating to two different `cairnd`-linked servers holds two different
`users.id` values, one on each server, because `users` has no cross-server identity concept at
all. Keying the personal namespace by `user_uuid` alone would silently merge those two
accounts' personal knowledge the moment a local store happened to see the same `user_uuid` on
two different servers — which cannot happen today (ids are server-minted UUIDs, so a
collision is astronomically unlikely), but keying by the pair costs nothing and removes the
assumption entirely rather than resting on "ids don't collide in practice." §10 covers what
this partitioning means for storage and recall.

A machine linked to one project has exactly one `project:*` namespace, exactly one
`personal:*` namespace **per identity it has ever linked** (see §10 — a store may hold more
than one), and exactly one `team:*` namespace per server it is linked to.

## 1a. A `team:*` cursor is a position in one caller's feed (FR-592)

The table above is also a table of what each key *omits*, and one omission has a
consequence the others do not.

`personal:<instance>:<user>` carries its owner, so two identities on one machine get two
lanes and two cursors, and neither can walk into the other's rows. `team:<instance>` carries
no identity at all — deliberately, because a store binds to exactly one server's team corpus
(§10, FR-496) and a second `team:*` lane is what that binding forbids. But the team **feed**
is not the same feed for every caller: a `proposed` row is visible to its author and to any
admin, and to nobody else (`global-memory.md` §5b). So a `team:*` cursor is a position in a
feed whose contents depend on who asked, recorded under a key that does not say who asked.

Two events widen a caller's view without changing a single row:

| Event | What becomes newly visible |
|---|---|
| a member is promoted to admin | every other member's pending proposals |
| the machine authenticates as a different account | that account's own pending proposals |

A monotonic cursor cannot recover from either. Pending rows the old view excluded are older
than the cursor, so "everything after this cursor" never asks for them again — and a pull is
the only way they can arrive. An admin's store would be permanently missing the proposals
that admin exists to ratify.

**The contract.** `GET /api/sync/changes/team` returns a `visibility` field alongside
`cursor`: an opaque string derived from the **authenticated** caller — `SettledUser`, never
anything the caller sent — that differs whenever the filter above would differ. The client
stores it beside the cursor (`sync_cursor.visibility_context`). When the reported value
differs from the stored one **and** a cursor is already recorded, the client discards the
cursor instead of advancing it and re-reads the lane from the beginning.

The re-read is cheap in the only currency that matters here: every team merge is idempotent
by id, and team content is written once and never rewritten, so re-delivering a row is a
no-op. A lane with no cursor yet is already reading from the beginning, so it records the
context and advances normally rather than restarting for nothing.

A server that reports no `visibility` predates the field. There is nothing to compare, the
cursor behaves exactly as it did before, and no lane resets on every pull.

`personal:*` needs none of this: the server filters that feed by the owning account, and the
lane key already names it, so a different identity is a different lane by construction.

## 1b. An endpoint is not an identity (FR-601)

§11a and §10 pull in opposite directions, and the seam between them is where a lane keyed by
a *provisional* id sits.

A server below schema 3 reports no instance id, so a lane opened against one is keyed by an id
derived from the configured endpoint. §11a then promises liveness: when "that peer is replaced
by a supporting server **at the same configured endpoint**", the content held for it is
released. §10 promises isolation: a store binds to exactly one server's team corpus, and
blending two deployments' guidance is what FR-496 forbids. Read together, the provisional id
is asked to mean both "whichever server answers here" and "this particular server".

The reconciliation is a division of labour, not a compromise:

| | decides identity | requires identity |
|---|---|---|
| `establish_global_namespaces` | re-keys a provisional lane to the id the peer reports, once | — |
| `drain_global`, `pull_global` | — | the lane's instance must equal the peer's, exactly |

**Establishment decides; operations require.** A provisional lane is a lane whose identity is
not yet known, and resolving it is establishment's job. Once resolved, the lane names a real
server and every later operation compares against that and nothing else.

This was briefly the other way round: operations accepted a lane whose instance matched the
provisional id derived from the endpoint, because the background worker only ran establishment
when a store had *no* global lanes at all — so a provisional lane never got re-keyed on that
path, and refusing it would have stranded the very content §11a exists to release. Buying
liveness there cost isolation: a deployment replaced or restored at the same URL matched the
same provisional id, and inherited its predecessor's lane. The worker now runs establishment on
its own cadence whether or not global lanes exist, which makes the re-key reliable and lets the
operations be strict.

**What this does and does not prevent.** A lane naming a *real* instance is refused against any
other server, at the same URL or elsewhere — that is the case with a corpus to protect. A lane
still keyed provisionally is adopted by whatever schema-3 server next answers at that endpoint,
which is exactly §11a's scenario and is not a leak: its predecessor was below schema 3 and had
no global corpus at all, so there is nothing of another server's to inherit. The content such a
lane holds is the store's own, queued and never delivered.

## 2. `sync_cursor` replaces `sync_meta`'s single cursor

Today, `sync_meta` (`crates/cairn-store/migrations/0001_init.sql:180-184`) is keyed
`project_id TEXT PRIMARY KEY` — one row, one cursor, per project:

```sql
CREATE TABLE IF NOT EXISTS sync_meta (
    project_id      TEXT PRIMARY KEY REFERENCES projects(id),
    last_success_at TEXT,
    pull_cursor     TEXT
    -- server_capability added by 0005
);
```

**New table, migration `0007_collaborative_global_memory.sql`:**

```sql
CREATE TABLE sync_cursor (
    namespace    TEXT PRIMARY KEY,   -- 'project:<uuid>' | 'personal:<uuid>' | 'team:<uuid>'
    pull_cursor  TEXT,
    server_capability TEXT
);
```

**Backfill**: for every existing `sync_meta` row, insert
`sync_cursor(namespace = 'project:' || server_project_id, pull_cursor = pull_cursor,
server_capability = server_capability)`. `sync_meta` itself is retained unchanged — nothing
reads it going forward for cursor purposes, but dropping it is not required for this feature
and preserving it costs nothing (FR-523's "assign every new field a documented default"
extends naturally to "leave the row that already worked alone" where a full removal is not
required by any requirement). `personal:*` and `team:*` rows are absent until the first
successful pull in each, at which point they are inserted exactly as a fresh project's
`project:*` row is inserted today (`repo::set_pull_cursor`,
`crates/cairn-store/src/repo.rs:1705-1715`, generalized to take a namespace string instead of
a project id).

**Per-namespace cursors are independent (FR-487).** Advancing or resetting
`project:<A>`'s cursor writes exactly one row's `pull_cursor` column; it has no effect on
`personal:<u>` or `team:<s>`'s rows, because they are different primary keys in the same
table, not different columns of one row.

## 3. `writer_identity` — the missing piece behind multi-device convergence

**D407, FR-490.** New local-only, single-row table:

```sql
CREATE TABLE IF NOT EXISTS writer_identity (
    -- The singleton guard. There is no way to say "exactly one row, forever" in
    -- SQLite DDL, and every other Cairn singleton is singleton-per-key rather
    -- than per-table, so this is the one place the constraint has to be written
    -- as a pinned primary key (`data-model.md` §2.10).
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    writer_id  TEXT NOT NULL,
    created_at TEXT NOT NULL
);
```

Generated once, on first use, the same "create if absent, atomic against concurrent daemon
and CLI processes" discipline `paths::machine_salt` already uses for the pattern-promotion
salt (`crates/cairn-core/src/paths.rs`, "Creation is atomic, and it has to be. The daemon and
the CLI are separate processes over one state directory"). `writer_identity` is **not** a
device registry: there is no server-side table of writers, no name, no user-visible list, no
lifecycle beyond "exists" — the existing per-device credential remains the API token
(`api_tokens`, unchanged), and `writer_id` never crosses the wire as an entity of its own. It
exists solely to be mixed into two things: the outbox idempotency key (§5) and each personal
or team record's `writer_seq` (§6).

## 4. Per-namespace backoff, drain and capability state

**FR-497, D427.** Today's backoff is process-global:

```rust
// crates/cairnd/src/sync.rs:39-118 (run_worker)
pub async fn run_worker(daemon: std::sync::Arc<Daemon>) {
    let mut backoff = BACKOFF_MIN;          // one variable, shared across every project
    loop {
        for project in daemon.linked_projects().await {
            ...
            if hit_transient { tokio::time::sleep(backoff).await; backoff = (backoff*2).min(BACKOFF_MAX); }
            else { backoff = BACKOFF_MIN; }
        }
    }
}
```

One unreachable project throttles every project's retry timing, because `backoff` is one
`Duration` shared by the whole loop body, not one per project — and this feature widens the
loop body to iterate namespaces, not just projects, so the same sharing would now throttle
personal and team sync whenever a project namespace hit a transient error, or vice versa.

**The fix**: `backoff` becomes a `HashMap<SyncNamespace, Duration>` (or an equivalent
per-namespace field on whatever tracks a linked project today), each entry independently
doubling from `BACKOFF_MIN` (500ms) to `BACKOFF_MAX` (30s) on that namespace's own transient
failures and resetting to `BACKOFF_MIN` on that namespace's own success. A `project:*`
namespace hitting the server's rate limit backs off on its own schedule while `personal:*`
and `team:*` continue retrying at `BACKOFF_MIN` on theirs.

The same widening applies to drain/claim state: `outbox::claim` already scopes by
`project_id` in its `WHERE` clause (`crates/cairn-store/src/outbox.rs:107-135`); this feature
adds a `namespace` predicate alongside it (§5) so a claim against `personal:*` never competes
with, or blocks on, a claim against `project:*`.

**Releasing claimed work at daemon start (FR-502, FR-562).** `release_all_claims`
(`crates/cairn-store/src/outbox.rs:170-177`) already resets every `in_flight` row to
`pending` unconditionally — it has no `project_id` filter today, so extending the outbox to
carry a `namespace` column requires no change to this function's behavior at all: it already
applies "independently across all namespaces" simply because it was never scoped to begin
with. What changes is only that the rows it releases now include `personal:*` and `team:*`
rows, which previously could not exist. **FR-562** states the property this unscoped release
already gives for free, so it is not accidentally lost during this feature's per-namespace
widening elsewhere in this contract: a namespace whose claim was interrupted (daemon killed
mid-drain, process crash) resumes on the very next start with no dependency on any other
namespace's claim state — `release_all_claims` runs once, over every namespace's `in_flight`
rows, in one pass, so a `team:*` claim interrupted mid-flight is never left waiting on
`project:*`'s claim to be released first, or on any ordering between namespaces at all.

## 4a. Two refusals, opposite retry semantics (D447, FR-577, FR-581)

§4's `blocked` state and its backoff exist for one kind of refusal: the server cannot accept an
entity type it has never heard of. That whole mechanism rests on an assumption worth saying out
loud — **the same bytes will be acceptable later**. Upgrade the server and the held item goes
through unchanged, which is why holding it, re-probing, and never retrying it against a peer
that cannot take it is the right behavior (FR-499, FR-561).

Server-side content validation (`promotion-privacy.md` §2c, the fifth entry point) produces a
refusal with the opposite property. The item is refused for what it *contains*. No server
upgrade changes that; retrying the identical payload fails identically, forever. Sending it down
the capability path would park an item that can never be released, mark a namespace blocked that
nothing can unblock, and apply a backoff that throttles every later item in that lane behind one
record that will never leave it. The liveness cycle of §11a would run forever against a
condition it cannot resolve.

They are therefore separate paths:

| | Capability refusal (§4, §11a, D423) | Ingest refusal (D447) |
|---|---|---|
| Cause | The server's schema does not know this entity type | The content failed `validate_global_content` |
| Wire form | `409` / `unknown_entity_type` | `422` / `content_rejected`, with a `class` naming the rejection |
| Item's fate | Held, undelivered, unfailed — recoverable (FR-499) | Permanently failed; never delivered, never acknowledged as delivered (FR-581) |
| Namespace state | `blocked`, re-probed on a backed-off schedule (FR-561) | Unchanged — eligible, unthrottled |
| Retry unchanged | Succeeds after the server is upgraded (FR-562) | Can never succeed |
| Backoff applied | Yes, per namespace (§4) | **No** |
| Operator action | None; resolves itself (FR-563) | Edit or discard the offending record |

**The discriminator is a typed field, never a message string** (SC-456). A client that chose its
path by substring-matching an error message would work until someone reworded the message, and
then fail silently in one of two ways — a permanent refusal retried forever, or a recoverable
item discarded. Both are worse than a hard parse error, because neither surfaces. `code` decides;
`message` is for humans.

**The refusal carries a class, never the content** (FR-547, FR-577). The `class` field holds one
of the nine rejection-class names. The client already has the record, so the rule it broke is all
it needs; the offending substring appears in no response body, no server log line and no client
diagnostic (SC-439).

**One bad record does not stall the batch.** A refused item is refused individually; the rest of
the push is applied and the cursor advances past all of it. Refusing the whole push on one bad
item reintroduces the stall from the other direction, which is exactly what per-namespace
backoff exists to prevent. The push response therefore reports per-item outcomes, not one batch
verdict. `compatibility.md` §2b states the same distinction from the client's point of view.

## 5. The conditional-pull defect, and its fix

**FR-489, quoted exactly.** The current guard:

```rust
// crates/cairnd/src/sync.rs:64-86
let (pending, _) = match outbox::counts(&daemon.store, project.id).await { Ok(c) => c, Err(_) => continue };
if pending == 0 {
    let blocked = outbox::blocked_count(&daemon.store, project.id).await.unwrap_or(0);
    if blocked == 0 || !probe_due { continue; }
    probed = true;
}
match drain(&daemon, project.id, server_project_id).await {
    Ok(...) => { ...; let _ = pull(&daemon, project.id, server_project_id).await; }
    ...
}
```

`pull` is reached only *after* `drain`, and `drain` is only called when there is something
`pending` to push or something `blocked` worth re-probing. **A linked project with an empty
outbox and nothing blocked never calls `pull` in the background at all.** `pull` is otherwise
reachable only from `sync_now` (an explicit `cairn sync now`). A consume-only machine — one
that never writes anything of its own to a shared project, personal, or team namespace — has
no outbox activity to trigger this branch, ever. Without a fix, **team knowledge never
arrives on such a machine**: a member who only reads team guidance and never proposes
anything would never see an admin's ratification, because nothing on that machine's side
would ever call `pull`.

**The fix**: the per-namespace loop pulls **unconditionally**, on its own schedule,
independent of whether that namespace has anything pending or blocked:

```text
for each namespace this store knows about (project:*, personal:*, team:*):
    if namespace has pending or blocked outbox work:
        drain(namespace)
    always:
        if namespace's own pull-due timer has elapsed:
            pull(namespace)
```

The pull-due timer is a per-namespace interval, and it **is** a new constant this contract
must name: `PULL_INTERVAL_SECONDS = 30` (FR-589). An earlier draft of this section declined to
invent one, on the grounds that `WORKER_TICK = 500ms` already provides a cadence and the pull
call only needs moving outside the `pending == 0` short-circuit. That reasoning produces an
unbounded poll. With no interval of its own, "the pull-due timer has elapsed" is true on every
tick, so each namespace would pull twice a second against the server — three namespaces, six
requests per second per machine, forever, whether or not anything changed. Backoff does not
save it, because backoff only engages on failure and these requests succeed.

So `WORKER_TICK` stays the *check* frequency and `PULL_INTERVAL_SECONDS` is the *pull*
frequency: the tick asks whether 30 seconds have passed since this namespace last pulled. The
number is stated here rather than left to implementation because `SC-412` asserts against it —
a criterion reading "within the documented background interval" has no referent unless some
document documents it, and until this paragraph existed, none did. `SC-412` asserts 60 seconds,
twice the interval, so a passing test does not depend on landing inside a single window.

The rest of the correction is as described: the existing tick loop simply needs its pull call
moved outside the `pending == 0` short-circuit. This is the one change in this contract that is not additive
scaffolding but a correction to existing control flow, and it is required precisely because
personal and (especially) team knowledge is the first kind of content a machine can
legitimately *only ever consume* — every prior synced entity type had at least the
possibility of being produced locally.

## 6. The outbox: `namespace` column and widened `entity_type`

**FR-528, D426.** `outbox` is rebuilt (as it already was once, at
`0005_project_intelligence.sql:452-492`, establishing the precedent and the proof method
this feature reuses) to add:

```sql
CREATE TABLE outbox_new (
    id                    TEXT PRIMARY KEY,
    project_id            TEXT,                 -- now nullable: absent for personal/team rows
    server_project_id     TEXT,                 -- now nullable, same reason
    namespace             TEXT NOT NULL,         -- 'project:<uuid>' | 'personal:<uuid>' | 'team:<uuid>'
    entity_type           TEXT NOT NULL CHECK (entity_type IN (
        'project','task','session','memory','handoff',
        'memory_relation','task_criterion','task_blocker',
        'personal_knowledge','personal_knowledge_relation',
        'team_knowledge','team_knowledge_relation')),
    entity_id             TEXT NOT NULL,
    operation             TEXT NOT NULL CHECK (operation IN ('upsert','delete')),
    idempotency_key       TEXT NOT NULL UNIQUE,
    payload               TEXT NOT NULL,
    state                 TEXT NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending','in_flight','delivered','failed','blocked')),
    attempts              INTEGER NOT NULL DEFAULT 0,
    last_error            TEXT,
    created_at            TEXT NOT NULL,
    delivered_at          TEXT,
    claimed_at            TEXT,
    blocked_reason        TEXT,
    blocked_at_capability TEXT
);
```

`OutboxEntityType` (`crates/cairn-core/src/domain.rs:164-174`) gains four variants:
`PersonalKnowledge`, `PersonalKnowledgeRelation`, `TeamKnowledge`, `TeamKnowledgeRelation`
(the last added for `team_knowledge_relations`, D431) — twelve total, up from eight. The
existing `outbox_cannot_carry_observations`-style test
(`crates/cairn-core/src/domain.rs:975-1012`) is extended to assert the new variants exist and
that the still-forbidden ones (`observation`, `evidence_fact`, etc.) still do not.

**FR-530 — the proof obligation.** Widening the CHECK is proven exactly as `0005` proved its
own outbox rebuild: build the store through its **actual** migration history 1 through 6,
insert a representative row of every pre-existing `entity_type`, run migration 7, and assert
that every pre-existing row survives with byte-identical column values (row count and content
equality), plus a fault-injection test asserting that an interrupted migration 7 (killed
mid-transaction) leaves the store on schema version 6, fully functional, never on a
partially-migrated 7. This is not a new proof technique — it is `0005`'s own precedent,
applied to `0007`.

`outbox::claim` (`crates/cairn-store/src/outbox.rs:107-135`) gains a `namespace` predicate in
its `WHERE` clause, scoping a claim to one namespace at a time, mirroring today's
`project_id = ?2` predicate exactly.

## 7. The idempotency key, and why `writer_id` must join it

**Today** (`crates/cairn-store/src/outbox.rs:60-63`):

```rust
let key = cairn_core::digest(&format!(
    "{entity_type}:{entity_id}:{operation}:{}", cairn_core::digest(&body)));
```

**The defect this exposes**: `sync_state`'s primary key on the server
(`crates/cairn-server/migrations/0001_init.sql:144-150`) is the idempotency key **alone**,
with no writer, device, or user dimension. Two different local stores that independently
produce a byte-for-byte identical `(entity_type, entity_id, operation, payload)` — which is
exactly the shape of two devices each proposing the same personal fact with the same
`entity_id` never having synced with each other, or two devices under the same user account
each recording an identical personal entry — collide on this key. The **second** to arrive is
reported `duplicate` and its outbox row is marked `delivered`
(`crates/cairnd/src/sync.rs:723-728`) even though it came from a genuinely different writer
and represents a genuinely separate write. This was harmless for project memory because a
`memory`'s `entity_id` is a UUIDv7 minted once by whichever session created it — two devices
never independently mint the *same* id for *different* content, so the collision case (same
id, same content) really was the same write. Personal and team knowledge introduce no new
risk of colliding ids for genuinely different content either, but D407 closes the gap anyway,
because "two devices happened to produce byte-identical content for the same id" should read
as two writes if it should read as anything durable — the reconciliation machinery
(`global-memory.md` §6) already has a well-defined answer for "two records, identical
content" (`Duplicate`), and that answer should be reachable rather than pre-empted by an
idempotency collision that never lets the second record exist to be reconciled against.

**The fix**: `writer_id` (§3) joins the key input for personal and team rows:

```rust
let key = cairn_core::digest(&format!(
    "{writer_id}:{entity_type}:{entity_id}:{operation}:{}", cairn_core::digest(&body)));
```

applied only where a `writer_id` is meaningful — project-namespace entities keep today's key
shape unchanged (no behavior change for existing entity types; this is additive to the two
new domains only, so no existing `sync_state` row's key is invalidated). Two stores now
producing identical content for the same personal or team `entity_id` compute two distinct
keys (their `writer_id`s differ), so both rows reach the server, both are accepted, and
`classify_proposal` (`global-memory.md` §6) is the thing that decides they are duplicates —
correctly, deterministically, and visibly, rather than one of them silently vanishing at the
transport layer before reconciliation ever sees it.

## 8. Multi-device concurrency: what this feature deliberately does not add

**FR-493.** Disagreement between two writers' personal or team knowledge on the same subject
is resolved exactly the way project knowledge already is: immutable records, `INSERT OR
IGNORE` import (`crates/cairn-store/src/repo.rs:927-953`'s existing pattern, extended to the
two new tables), and `classify_proposal`/`derive_subject` deciding what the records mean once
they exist — never by comparing wall-clock time or arrival order.

**Explicitly rejected, and named so a later contributor does not "helpfully" add them:**

| Mechanism | Why not |
|---|---|
| Vector clocks | Do not exist anywhere in the codebase today (`crates/`-wide grep, confirmed); would require a per-writer version vector on every record, replicated through every sync payload, purely to answer a question immutability plus reconciliation already answers without one |
| Hybrid logical clocks (HLC) | Same objection — a clock exists to order events; this design does not order events, it classifies content |
| Last-write-wins on any timestamp | The one thing this design is built to make structurally impossible for a subject's canonical answer — see `global-memory.md` §6 and 003's own "no timestamp is compared" discipline (`crates/cairn-core/src/knowledge.rs:10-17`) |

**The four existing last-arrival overwrites are the anti-pattern, not the precedent, and this
feature must not add a fifth:**

| # | Site | What it overwrites, unconditionally |
|---|---|---|
| 1 | `crates/cairn-server/src/sync.rs:509-523` | `upsert_memory`'s `ON CONFLICT (id) DO UPDATE SET content = EXCLUDED.content, ...` |
| 2 | `crates/cairn-store/src/criteria.rs:239-242`-equivalent (`import_criterion`) | Criterion fields, `ON CONFLICT ... DO UPDATE SET ... = excluded.*` |
| 3 | `crates/cairn-store/src/criteria.rs`-equivalent (`import_task`) | Task title/goal/status, same pattern |
| 4 | `crates/cairnd/src/sync.rs:1167-1182` (`import_verification`) | Verification state, guarded only by "does this machine hold local runs," not by any timestamp |

Personal and team knowledge have no analog to any of these four, by construction: there is no
`content` column ever targeted by an `UPDATE` after insert (§ immutability, `global-memory.md`
§3), so there is no unconditional overwrite for personal or team content to fall into. The
one `UPDATE` this feature does introduce — the team state compare-and-swap
(`global-memory.md` §3) — is exactly the opposite pattern: conditional on `expected_state`,
refusing rather than overwriting on mismatch.

## 9. `writer_seq` — gap detection only, never a tiebreak

**FR-492, D408.** Every personal and team record carries the writer's sequence number at the
moment it was created — a per-writer monotonic counter, incremented locally before each
write, stored alongside the record. Its **only** sanctioned uses:

1. **Gap detection**: if a personal record with `writer_seq = 5` arrives before one with
   `writer_seq = 3` from the same `writer_id`, or `writer_seq = 3` never arrives at all, that
   is visible (for diagnostics — `cairn sync status`, not for any correctness decision) as a
   gap in that one writer's own stream.
2. **Dedup within one writer's stream**: two deliveries of the same `(writer_id, writer_seq)`
   pair are the same original write, useful as a cheap pre-filter before the full idempotency
   key comparison, never as a replacement for it.

**Never**: compared across two different `writer_id`s to decide which record "came first," and
never used to break a tie between two disagreeing records — that is exactly
`derive_subject`'s job (`global-memory.md` §6), and it does not read `writer_seq` any more
than it reads `created_at`. This mirrors `tasks.local_revision`'s existing discipline
("a monotone counter for THIS store only... never transmitted,"
`crates/cairn-store/migrations/0005_project_intelligence.sql:371-374`) with one difference:
`writer_seq` *is* transmitted (it travels with the record, because a peer needs it for gap
detection against that same writer), but it is transmitted as inert diagnostic metadata, never
as an ordering key any importer consults to decide what a record means.

**Both fields cross the wire and both gain server columns (D448, FR-582).** An earlier draft of
this design listed `writer_id` and `writer_seq` among the never-transmitted fields while the
local schema declared both `NOT NULL` under `UNIQUE (writer_id, writer_seq)` — an invariant no
pulled record could satisfy, since a record this store did not write would have no value for
either column. The reflex repair is to make the local columns nullable, and it is the wrong one:
it destroys the gap detection the fields exist for. The deeper reason the original
classification was wrong is that it reasoned by analogy to `tasks.local_revision`, and the
analogy does not hold. `local_revision` is meaningful only to its own store because nothing else
acts on it; a writer sequence is useful *only* to someone else. Its entire purpose is to let a
second party notice that record 7 arrived and record 6 never did.

So the serialized form of a `personal_knowledge` or `team_knowledge` item carries both fields,
and the server's tables declare them with the same `UNIQUE (writer_id, writer_seq)` constraint
the local store carries (`data-model.md` §4b, `migration.md`). Two enforcement points, one
invariant — a column with the constraint dropped on one side is just a place to store a
violation. Neither value identifies a person, a machine name, or a path: `writer_id` is an
opaque per-store identifier and `writer_seq` is a counter.

**Diagnostic-only, asserted structurally (FR-583, SC-455).** "No importer consults it" is a
sentence, and sentences are what a contributor who has not read this file overrides. The
reconciliation input type carries **no** `writer_seq` field, so a tiebreak that consulted one
would not compile — the same discipline that keeps `created_at` out of `MemoryFacts` (D-U2).
`SC-455` supplies the behavioral half: replaying one corpus under permuted, withheld and
renumbered sequences produces byte-identical derived output. `SC-450` covers the other
direction — a record created on one device pulls into another with identity and sequence intact,
and a deliberately withheld middle record is reported as a *detected gap* rather than silently
ignored, because a gap nobody reports is indistinguishable from a stream that had no gap.

## 10. Team is server-bound and refused on mismatch; personal is server-partitioned, never refused (D438)

**FR-495, FR-496, FR-567, FR-568.** Team knowledge is a server-wide artifact — an
`authoritative` row means "this server's ratified default," and that claim is meaningless
detached from the server that ratified it. Personal knowledge belongs to the user, not to any
server, and "personal knowledge follows the user" would be contradicted outright by refusing
it on a server-instance mismatch. **These are opposite rules for opposite reasons, and this
contract states both explicitly so neither is ever applied to the wrong domain** — an earlier
draft task (rewritten per this addendum) had both domains refused identically on mismatch,
which is wrong for personal knowledge specifically.

### Team: bound to the server instance, refused on mismatch (FR-496)

The local store records `linked_server_instance_id` (a column on the local `projects` row for
the `project:*` link, or a dedicated single-row table for the `team:*` namespace specifically,
since team knowledge is linked to a server instance independent of any one project) the first
time it successfully pulls from a server's `GET /api/version`
(`identity-administration.md` §8). Every subsequent pull for `team:*` compares the server's
advertised `server_instance_id` against the recorded one:

```text
if recorded_instance_id is Some(id) and id != observed_instance_id:
    refuse to merge; report a server-instance mismatch to the user; do not write any
    pulled team_knowledge row; do not advance the team:* cursor
```

**The refusal is reported, not silently dropped (FR-496).** A mismatch surfaces through the
same reporting path a capability block already uses (`cairn sync status`, a daemon log line,
whatever the surface — the requirement is that it is observable, not that it uses one specific
channel) — a store that silently discarded mismatched team pulls forever would look identical
to one with no team knowledge at all, which is indistinguishable from "everything is fine" and
therefore useless as a signal to the operator who pointed `cairn` at the wrong server.

This is the guard against a genuinely different scenario than capability blocking (§11a): a
capability block means "this server is the right one, but too old." An instance mismatch means
"this is not the server this store's team knowledge came from at all" — pointing `cairn` at a
different server (a different deployment, a restored-from-backup instance that regenerated its
`server_instance` row, a staging server) must not silently blend that server's team guidance
into the local store's existing team corpus, because "authoritative team-wide default" only
means something relative to one specific server's ratification history.

### Personal: never refused, partitioned by owning identity instead (FR-567, FR-568)

Personal knowledge pulled from a server instance different from the one recorded for this
store's `personal:*` namespace is **not** refused — there is no mismatch check on the personal
pull path at all, because there is nothing to mismatch against: §1's namespace key,
`personal:<server_instance_id>:<user_uuid>`, already makes the server instance part of the
namespace's own identity rather than something to compare a pulled row against after the fact.
A local store MUST be able to hold the personal knowledge of more than one identity — the same
human's account on server A and the same human's account on server B are two different
`(server_instance_id, user_uuid)` pairs, hence two different `personal:*` namespaces, hence two
disjoint sets of rows, never merged and never compared for a mismatch.

**Recall surfaces only the currently linked identity (FR-567).** A local store holding personal
knowledge for two identities does not present both at once: `cairn_search`, `cairn_context`,
and every other read path filter to the `personal:*` namespace of whichever identity the store
is currently authenticated as. Switching which account `cairn auth` holds a token for changes
which identity's personal knowledge recall can see; it does not delete, merge, or expose the
other identity's rows — they remain in the store, simply not the one currently surfaced.

**The subtlety this makes precise (FR-568)**: "personal knowledge follows the user" sounds like
it should mean one flat pool of personal knowledge per human. It does not, because user
identity is per-server — the same human is a different account, with a different `user_uuid`,
on every server they link to. Partitioning by `(server_instance_id, user_uuid)` is what makes
"personal knowledge follows the user" true *without* requiring Cairn to solve the harder,
out-of-scope problem of recognizing that two accounts on two different servers belong to the
same human. Each account's personal knowledge follows that account faithfully; nothing here
claims to unify two accounts that happen to share an owner.

## 11. Capability advertisement — one-way, additive, unchanged in kind

**D428, FR-498, FR-499, FR-500.** Two new capability names,
`personal_knowledge` and `team_knowledge`, added to a new `SCHEMA_3_CAPABILITIES` constant
(`crates/cairn-server/src/version.rs`, alongside the existing `SCHEMA_2_CAPABILITIES`,
`version.rs:32-38`), returned once `schema_version >= 3`. `ENTITY_CAPABILITIES`
(`crates/cairnd/src/sync.rs:871-879`) gains four entries — one per new entity type, each
relation gated by the same capability name as its parent entity, exactly as `MemoryRelation`
is gated by `memory_relations` rather than by a capability of its own today:

```rust
(OutboxEntityType::PersonalKnowledge,         &["personal_knowledge"]),
(OutboxEntityType::PersonalKnowledgeRelation, &["personal_knowledge"]),
(OutboxEntityType::TeamKnowledge,             &["team_knowledge"]),
(OutboxEntityType::TeamKnowledgeRelation,     &["team_knowledge"]),
```

No handshake, no probe endpoint, no client-declared version — the existing one-way
advertisement is reused exactly (`GET /api/version` already existed; the D81 rationale
already recorded at `crates/cairnd/src/sync.rs:786-790` — "its silence is the answer" —
applies unchanged to the two new capability names). An older server (`schema_version < 3`)
returns a `capabilities` list without either new name; `reject_beyond_capability`
(`crates/cairn-server/src/sync.rs:257-293`) on such a server returns `409
unknown_entity_type` for `personal_knowledge`/`team_knowledge` payloads exactly as it does
for schema-2 entity types today.

**Held-back items are retained, not lost (FR-499).** Classified by
`codes::CAPABILITY_REFUSALS` (`crates/cairn-core/src/wire.rs:178`) exactly as today:
`unknown_entity_type` marks the outbox row `blocked`, not `failed` — retained locally,
neither delivered nor permanently failed, and **not retried** against that server (the
existing per-namespace backoff, §4, simply stops advancing that namespace's retry clock for
blocked work the same way it already does for schema-2-blocked project work).

**Recovery is automatic (FR-500).** `refresh_capability`
(`crates/cairnd/src/sync.rs:791-856`) already probes once per drain cycle and releases
blocked rows whose every required capability is now present
(`sync.rs:838-845`, `ENTITY_CAPABILITIES.iter().filter(...)`) — this logic requires no change
beyond the two new table entries above, because it was already written generically over
`OutboxEntityType`, not hand-enumerated per type. Once a server is upgraded to schema 3,
the next drain cycle's capability probe observes the new names, releases every `blocked`
`personal_knowledge`/`team_knowledge` row back to `pending` with its **original idempotency
key** (`outbox::release_blocked`, `crates/cairn-store/src/outbox.rs:253-273`, unchanged), and
the ordinary drain delivers it — applied exactly once, because the idempotency key and the
server's `sync_state` claim are exactly what already made this safe for schema-2 entities
(SC-331's precedent, restated for schema 3).

## 11a. The blocked-namespace liveness state machine (D437, FR-561–FR-563)

**The gap D437 closes**: FR-499 said held items must never be retried against a server that
cannot accept them; FR-500 said they deliver automatically once an upgrade happens. Neither
requirement, nor §11 above, ever named the state *between* those two — how a namespace notices
the server changed at all, on what schedule, and what exactly is repeated to find out. This
section is that missing state machine.

```text
        capability refusal                 probe still absent
             (409)                        (bounded backoff)
   ┌──────────────┐    ┌──────────────────────────────────┐
   │              ▼    │                                    ▼
ELIGIBLE ─────▶ BLOCKED ──────────────────────────────▶ BLOCKED
   ▲                                                        │
   │                    probe observes the capability       │
   └────────────────────────────────────────────────────────┘
        held entries released, original idempotency keys
```

| Transition | Trigger | What happens |
|---|---|---|
| `eligible → blocked` | A drain against this namespace is refused with `unknown_entity_type` (§11) | The item's outbox row is marked `blocked` (not `failed`); the namespace itself is marked blocked |
| `blocked → blocked` | A capability re-probe runs and the required name is still absent | Nothing changes except the backoff timer (§4) advancing on its own bounded schedule; held items are untouched |
| `blocked → eligible` | A capability re-probe observes the required name present | The namespace returns to eligible; every `blocked` row for it is released to `pending`, each keeping its original idempotency key (`outbox::release_blocked`) |

**FR-561 — the probe is a capability read, not an item retry.** While a namespace is blocked,
the client re-probes the server's advertised capabilities (`GET /api/version`, already polled
every drain cycle — no new endpoint) on a bounded, backed-off schedule, exactly like any other
transient-failure backoff (§4: 500ms doubling to 30s). **This is the distinction FR-499
depends on and this section states explicitly**: the probe reads `capabilities` — it is
*about* the server, not a delivery attempt — and it is not, under any circumstance, a resend
of a `blocked` item's payload. A blocked `personal_knowledge` row is not retried against the
server it was blocked by; only the capability list is re-read. This is why FR-561 and FR-499
are the same guarantee stated twice, once as the state machine (here) and once as the outcome
(§11): re-probing a capability and retrying a held item are different operations, and only the
first ever happens while a namespace is blocked.

**FR-562 — release preserves the idempotency key.** When a probe observes the capability, the
namespace transitions to eligible and every entry held for it releases for delivery with the
**same** idempotency key it was created with (§7, §11's `outbox::release_blocked`) — so an
entry that had been partially delivered before the capability disappeared (a rare but possible
sequencing: capability present, delivery in flight, capability advertisement flips between two
polls) is applied exactly once when it is finally accepted, because the server's `sync_state`
still recognizes the same key.

**FR-563 — no user action required.** The return to eligible needs no local write, no CLI
command, and no daemon restart — it is a pure consequence of the next scheduled probe
observing a changed `capabilities` list, exactly as `refresh_capability` already behaves for
schema-2 entities (§11).

**FR-563 — a blocked namespace is invisible to every other namespace.** Blocking is scoped
to the one namespace whose capability is missing; it neither delays, throttles, nor
interrupts `project:*` synchronization or any other namespace's drain and pull, which continue
at full speed throughout — the same per-namespace backoff independence §4 already establishes
for ordinary transient failures applies identically to a capability block, because a
capability block is implemented as exactly that: one more per-namespace backoff-and-retry
state, distinguished only by what it is waiting to observe (a capability name, not merely a
successful response).

**SC-445** is this whole state machine's verification obligation, stated as one scenario:
personal and team content queued against a server that does not support it is held while
project sync continues; after that peer is replaced by a supporting server at the same
configured endpoint, and with no new local write and no restart, the held content delivers
automatically and exactly once.

## 12. `sync_deferred` for the new types

**FR-501, updated for D431.** `0006_sync_deferred.sql`'s `kind` column
(`CHECK IN ('relation','criterion','blocker')`) gains **two** additions:
`'personal_knowledge_relation'` and `'team_knowledge_relation'` — the second exists because
`team_knowledge_relations` (`global-memory.md` §2, D431) has exactly the same "names another
row by id, which may not have arrived yet" shape `memory_relations` and
`personal_knowledge_relations` both have. A pulled relation row of either kind whose `from_id`
or `to_id` has not yet arrived locally is held exactly as a `memory_relation` is today
(`Placement::AwaitingParent`, `crates/cairnd/src/sync.rs:1191-1200`), replayed oldest-first
after every later pull, bounded by the same `DEFERRED_REPLAY_BATCH = 500`
(`crates/cairnd/src/sync.rs:1206`). `team_knowledge_applicability` needs no deferral entry
of its own: its rows arrive with their parent `team_knowledge` row in the same payload, never
separately, so there is no ordering to defer. No expiry is added here, matching the explicit
out-of-scope decision already recorded for `sync_deferred` generally (§7 of the brief) —
this feature does not fix that pre-existing gap, only extends the existing mechanism to two
more `kind` values.

## Invariants

1. `sync_cursor` has exactly one row per namespace this store has ever pulled from; no row's
   `pull_cursor` update ever touches another namespace's row (FR-487).
2. A synchronization failure or capability block confined to one namespace never prevents
   another namespace's drain or pull from proceeding on its own schedule (FR-488, D427).
3. Every namespace with nothing queued to push still has its own pull scheduled and
   executed; `pull` is never gated on `pending == 0` for any namespace (FR-489).
4. `writer_identity` holds exactly one row per local store, written once, and is never
   transmitted as an entity of its own (FR-490).
5. The idempotency key for a `personal_knowledge` or `team_knowledge` outbox row includes
   this store's `writer_id`; two different stores producing byte-identical content for the
   same entity id never collide as `duplicate` (FR-491).
6. `writer_id` and `writer_seq` travel with every personal and team record and are stored on
   the server under the same `UNIQUE (writer_id, writer_seq)` constraint the local store
   carries (FR-582). The `writer_identity` *table* remains local-only: what travels is the
   stamp, not the registry that minted it (D448).
6a. `writer_seq` is read only for gap detection and within-writer dedup; the reconciliation
   input type has no field for it, so no code path can compare it across two different
   `writer_id`s to order or arbitrate between them (FR-583, SC-455). A withheld record is
   reported as a detected gap, never silently ignored (SC-450)
6b. An ingest refusal never sets `blocked`, never applies namespace backoff, and is
   distinguishable from a capability refusal by a typed field rather than by message text
   (FR-581, SC-456); a refused item is never acknowledged as delivered
   (FR-492, D408).
7. No comparison of wall-clock time, arrival order, or any timestamp ever decides between
   two disagreeing personal or team records on the same subject (FR-493).
8. A team knowledge state transition applies only when the request's `expected_state`
   matches the row's actual current state at the instant of the attempt; a mismatch is
   refused and reports the actual state (FR-454).
9. Team knowledge pulled from a server instance different from the one already recorded for
   this store is refused, written nowhere, and the refusal is reported rather than silently
   dropped; personal knowledge is never refused on server-instance grounds — it is
   partitioned by `(server_instance_id, owning user_uuid)` instead, and a local store may
   hold more than one identity's personal knowledge at once (FR-495, FR-496,
   FR-567, FR-568, D438).
10. Backoff after a failed synchronization attempt is tracked independently per namespace;
    no namespace's failures slow another namespace's retry timing (FR-497).
11. `personal_knowledge` and `team_knowledge` are each gated by their own capability name
    under the existing one-way advertisement, and their respective relation entity types
    are gated by the same name as their parent; a server lacking either name blocks only
    the matching entity types, never the project namespace (FR-498).
12. A personal or team item blocked by an older server's capability refusal is retained
    locally, retried against no server until that server's advertised capability changes,
    and delivered exactly once — with its original idempotency key — once it does
    (FR-499, FR-500).
13. Replaying an entire outbox against a server that has already applied every item
    produces the same converged state as applying it once; every operation in this
    contract is idempotent under redelivery (`INSERT OR IGNORE` / compare-and-swap
    throughout, never a blind overwrite).
14. A blocked namespace re-probes the server's advertised capabilities, never the held
    items themselves, on a bounded backed-off schedule; a probe is a capability read, and a
    held item is never retried against a server that has not yet advertised the capability
    it needs (FR-561, D437).
15. When a probe observes the required capability, the namespace returns to eligible with
    no local write, no user command, and no daemon restart, and every held entry releases
    with its original idempotency key (FR-562, FR-563).
16. A namespace blocked on a missing capability never delays, throttles, or interrupts any
    other namespace's drain or pull (FR-563).
17. On daemon start, every namespace's unfinished claims release independently of every
    other namespace's; no namespace waits on another's claim to be released first
    (FR-562).
