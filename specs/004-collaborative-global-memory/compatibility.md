# Compatibility Matrix: Cairn Collaborative Global Memory

**Feature**: `004-collaborative-global-memory` | **Baseline**: `main` @ `96178fc` (v0.1.0-alpha.5)

The question this document answers: what happens, precisely, in every combination of a client and a
server that may or may not know about personal and team knowledge.

## 1. The four combinations

| | Old server (schema 1) | New server (schema 2, `SCHEMA_3_CAPABILITIES`) |
|---|---|---|
| **Old client** (pre-004) | Everything behaves exactly as it does on `main` today. Neither side has ever heard of personal or team knowledge, so there is nothing to hold back and nothing to advertise. | The server advertises `personal_knowledge`/`team_knowledge`; the old client never asks for `GET /api/version`'s new fields and has no code path that could produce a `personal_knowledge`/`team_knowledge` outbox row in the first place — `OutboxEntityType` on that build has only eight variants. Unaffected. |
| **New client** (004) | **§2, the critical case.** Project sync continues at full speed; personal and team namespaces sit `blocked`. | Everything syncs: project, personal and team namespaces all drain and pull normally, each on its own cursor and backoff (D426, D427). |

The interesting cell is new-client-old-server, because it is the one case where the two sides disagree
about what exists and the system must degrade gracefully rather than fail. It is traced in full below
(§2). The other three are stated in full here because "unaffected" is a claim, not a given:

**Old client, old server.** Neither side has run any 004 migration. `OutboxEntityType` on the old
client's build has eight variants, full stop — there is no code path that could construct a
`personal_knowledge` or `team_knowledge` outbox row, because the type that would carry it does not
exist in that binary. `GET /api/version` returns `SCHEMA_2_CAPABILITIES` (or, on a genuinely
pre-003 server, none at all) and the old client's `refresh_capability` reads it exactly as it does on
`main` today. This cell is not merely "compatible" — it is **identical** to pre-004 behavior, because
nothing 004 added is reachable from either binary.

**New client, new server.** Every namespace syncs: project on `sync_cursor`'s `project:<id>` row exactly
as before (renamed from `sync_meta`, §migration.md), personal on `personal:<user_id>`, team on
`team:<server_instance_id>` — each with its own cursor, its own backoff, and its own capability
fingerprint (D426, D427). `reject_beyond_capability` passes every item through, because
`schema_version >= 3` and every entity type the client can construct is one the server accepts. This is
the fully-upgraded steady state the other three cells are transitional relative to.

**Old client, new server.** The reverse-direction case, and — as with 003 — nothing has to be built for
it. The old client's `refresh_capability` reads whichever capability names the server returns and
matches them against `ENTITY_CAPABILITIES` as that build defines it: two names it has never heard of
(`personal_knowledge`, `team_knowledge`) simply never appear in its own `ENTITY_CAPABILITIES` map, so
they release nothing and block nothing — they are inert strings to that binary. The old client never
constructs a personal or team push, and never asks for a personal or team pull, so the new read-back
arrays a schema-3 server might add to `GET /api/sync/changes` are additive fields an old client's
deserializer never looks for (the same `#[serde(default)]` pattern 003 already relies on for its own
added arrays, `wire.rs:1591-1606`). An old client sees a server that behaves, from its point of view,
exactly like a schema-2 server.

## 1a. Upgrade-in-place: a schema-3 server replaces a schema-2 server at the same endpoint (D437)

A distinct case from the four steady-state combinations in §1's table: a client whose personal or team
namespace is already `blocked` against a schema-2 server (§2's traced mechanism) keeps running, unmodified,
while the operator retires that server process and starts a schema-3 server bound to the same configured
endpoint — same host, same port, same client configuration; nothing on the client changes. What the client
observes, in order:

1. Nothing changes on the client until its next scheduled drain cycle for that namespace; the blocked
   entries remain retained, untouched, exactly as before the server was replaced (§2 step 5). The client
   has no way to know, and does not need to know, that the process answering its configured endpoint is a
   different binary than the one that refused it.
2. On that next drain cycle, `refresh_capability`'s `GET /api/version` request (`cairn-server/src/
   api.rs:19`, `version.rs:54-72`) is answered by the new schema-3 process. Nothing about the request
   changed; only the response did — `capabilities` now includes `personal_knowledge`/`team_knowledge`
   (§5).
3. `refresh_capability` (`sync.rs:791-856`) sees the capability fingerprint change (the reverse of §2 step
   2) and runs the release step, `release_all_capability_blocked` (`sync.rs:838-845`), which re-checks
   every `blocked` row's entity type against `ENTITY_CAPABILITIES` and returns the newly-eligible ones to
   `pending`.
4. The next drain cycle delivers them, preserving their original idempotency keys (`outbox.rs:248-252`) —
   the same exactly-once guarantee §2 step 6 already describes for the ordinary same-process upgrade case.

The client cannot distinguish "this server process was migrated to schema 3 in place" from "the process
was replaced by a different one that happens to answer the same endpoint with a higher schema version" —
and it does not need to, because nothing in the mechanism above depends on process identity, only on what
the endpoint currently answers. This is the scenario D437 (FR-561–FR-563) formalizes as the blocked →
eligible transition; see §2a.

## 1b. Old client, new server: two routes are gone (D454)

§1's table and its "Old client, new server" paragraph cover the direction this feature *adds*
capabilities in — a pre-004 client meeting a schema-3 server sees, from its own point of view, exactly a
schema-2 server, because it never asks for anything schema 3 added. That paragraph is still true, but it
is not the whole story: the security prerequisite this feature builds on (spec.md "Assumptions") does not
add anything, it **removes** two routes — `POST /api/auth/register` and `POST /api/projects/{id}/join` —
and removal is a compatibility event for every client built before the removal, whether or not that
client has ever heard of personal or team knowledge.

**Project synchronization is unaffected.** Nothing about `refresh_capability`, `reject_beyond_capability`,
or any sync route changed to remove these two routes; they belong to account creation and self-service
membership, not to sync. A pre-004 client continues to push, pull, and advance its cursor exactly as
before, with no namespace degraded, delayed, or moved to `blocked` on account of either removed route
(FR-586). This is verified against a **real pre-004 client binary**, not a hand-rolled request that
happens to omit whatever fields changed — a simulated client can accidentally prove less than it claims
by omitting something a real shipped binary still sends (SC-457).

**A removed route answers `410 Gone`, never a bare `404`.** This is a deliberate, stated choice, not an
accident of routing: a `404` is indistinguishable from a typo'd URL or a route that was never built,
while `410 Gone` is the one status whose entire meaning is "this existed and was deliberately retired" —
exactly the fact an operator debugging a suddenly-failing client needs, and the one a bare not-found
withholds (FR-587). The body is a JSON object naming the replacement, in the same error shape every other
refusal in this API already uses:

```json
{ "code": "route_removed",
  "message": "self-registration is disabled; an administrator creates accounts with `POST /api/admin/users` (`cairn user create`)" }
```

```json
{ "code": "route_removed",
  "message": "self-join is disabled; an existing member or admin adds you with `POST /api/projects/{id}/members` (`cairn project member add`)" }
```

An operator who only has the response in front of them — no source, no release notes open — can read the
replacement route and the CLI verb straight off it (SC-458).

**The release documents the removal in operator-facing terms** (FR-588): that self-registration is gone,
that self-join is gone, that every account is now created by an administrator, and what an operator whose
users relied on either must do instead — run `cairn user create` (or `POST /api/admin/users`) for a user
who used to register themselves, and `cairn project member add` (or `POST /api/projects/{id}/members`)
for a user who used to join a project themselves. SC-458 verifies both the response body and that the
shipped documentation states all three facts, not only the response.

**This document does not itself specify `quickstart.md`'s or the API contract's exact route table** —
that ownership is elsewhere — but records here, for whoever reconciles them, that a `POST
/api/auth/register` returning a bare, unexplained `404` (as an earlier walkthrough draft showed) does not
satisfy FR-587 as stated: "not refused with a body explaining why" and "a message naming its replacement"
describe two different responses, and this document's decision is the latter.

## 2. New client, old server — the mechanism, traced

1. **The server tells the truth about itself, unasked.** `GET /api/version` is unauthenticated
   (`cairn-server/src/api.rs:19`, `auth.rs`'s `CurrentUser` extractor is not applied to it) and returns
   `VersionPayload { schema_version: 1, capabilities: SCHEMA_2_CAPABILITIES, .. }`
   (`cairn-server/src/version.rs:54-72`, `capabilities_for`, `version.rs:46-52`). `SCHEMA_2_CAPABILITIES`
   (`version.rs:32-38`) lists five names — `memory_relations, task_criteria, task_blockers,
   memory_subject_identity, memory_verification` — and neither `personal_knowledge` nor
   `team_knowledge` is among them, because this server has never run migration `0003`.
2. **The daemon's per-drain-cycle capability refresh sees no new names.**
   `refresh_capability` (`cairnd/src/sync.rs:791-856`) builds
   `format!("schema={schema};capabilities={}", names.join(","))` from whatever the server just returned.
   Compared against the cached fingerprint in `sync_cursor.server_capability` (data-model.md §2.10,
   replacing `sync_meta.server_capability`), the two new capability names are simply absent from the
   comparison set — there is nothing to release yet.
3. **A personal or team push is rejected by name, not silently dropped.** When the daemon pushes an
   outbox item whose `entity_type` is `personal_knowledge` or `team_knowledge`,
   `reject_beyond_capability` (`cairn-server/src/sync.rs:257-293`) runs before anything is applied
   (`sync.rs:158`): `schema_version >= 2` is false for this server, the entity type is in the
   schema-gated set, so the item is refused with `409 CONFLICT` and code `unknown_entity_type`
   (`sync.rs:261-271`) — the identical mechanism that already refuses `memory_relation` from an
   old-schema-1-vs-schema-2 mismatch today, extended by adding two names to the same gated set
   (`SCHEMA_3_ENTITY_TYPES`, the analog of today's `SCHEMA_2_ENTITY_TYPES` at `sync.rs:208`).
4. **The daemon classifies the refusal as recoverable, not permanent.**
   `codes::CAPABILITY_REFUSALS` (`wire.rs:178`) already contains `unknown_entity_type`; the client-side
   match (`cairnd/src/sync.rs:744-759`) routes any code in that set to `outbox::mark_blocked`, never to
   `outbox::mark_failed`. No new code is added for 004 — the same three-name set
   (`unknown_entity_type`, `unknown_field`, `schema_older`) already covers this case.
5. **`blocked` rows are excluded from claiming, twice over.** `OutboxState::is_claimable`
   (`domain.rs:205-207`) excludes `blocked`, and the claim SQL carries an explicit
   `AND state != 'blocked'` predicate (`outbox.rs:117-121`) as a second, independent gate. A blocked
   personal or team item is retained — payload and idempotency key intact — and simply stops being
   offered to the drainer, satisfying FR-499 ("retained locally in a recoverable state, neither
   delivered nor permanently failed, and MUST NOT be retried against that server").
6. **`refresh_capability` releases them once the server is upgraded.** The next drain cycle after the
   server runs migration `0003` sees `personal_knowledge`/`team_knowledge` in the returned capability
   list; the fingerprint comparison changes, and `release_all_capability_blocked` (the 004 extension of
   `refresh_capability`'s existing release step, `sync.rs:838-845`) re-checks every `blocked` row's
   entity type against `ENTITY_CAPABILITIES` and returns it to `pending`, **preserving the original
   idempotency key** (`outbox.rs:248-252`) — so the eventual delivery is exactly the same write the user
   made when it was first queued, not a re-derived one, satisfying FR-500's "applied exactly once."

## 2a. The blocked-namespace liveness cycle (D437)

Steps 1–6 of §2 above show one full pass through the cycle; this section states it as an explicit state
machine, since the transition itself — what actually moves a namespace from `blocked` back to `eligible` —
was never specified before the repair addendum resolved this as Finding 5.

**States**: `eligible` (the namespace pushes and pulls normally) and `blocked` (the namespace is held;
nothing in it is offered to the drainer, §2 step 5).

**Transitions**:

| From | To | Trigger |
|---|---|---|
| `eligible` | `blocked` | A push is refused with `409 CONFLICT` and code `unknown_entity_type` — a capability refusal (`reject_beyond_capability`, `sync.rs:257-293`; `codes::CAPABILITY_REFUSALS`, `wire.rs:178`; routed by `sync.rs:744-759`), not a transient error. |
| `blocked` | `blocked` | A capability probe (`GET /api/version`, read via `refresh_capability`, `sync.rs:791-856`) still does not list the required capability. The namespace stays held; the next probe is scheduled on a bounded, backed-off interval, using D427's per-namespace backoff state (§3) so this namespace's wait never affects any other namespace's schedule. |
| `blocked` | `eligible` | A capability probe observes the required capability present. Held entries release for delivery, preserving their original idempotency keys (`outbox.rs:248-252`) — the same release path §2 step 6 describes. |

FR-561–FR-563 govern this cycle:

- **FR-561** — the re-probe is a capability read, never a retry of the held items: it reads `GET
  /api/version` only, and never touches a blocked namespace's outbox rows. The held items are still never
  retried against a server that has already refused them.
- **FR-562** — on a probe observing the capability, the namespace returns to eligible and held entries
  release preserving their original idempotency keys.
- **FR-563** — the blocked → eligible transition requires no local write, no user command, and no daemon
  restart: it is a background effect of the daemon's own ordinary drain-cycle polling, the same polling
  that produced the original block.
- **FR-488** — a blocked namespace never delays, throttles, or interrupts any other namespace; this is
  what D427's per-namespace backoff/drain state (§3) exists to guarantee — without it, a blocked
  namespace's backoff schedule would still be the single process-global `backoff` variable every namespace
  shares. (This obligation predates this feature's own FR-561–FR-563 block and is stated once, in the base
  Synchronization section, rather than restated here under a feature-local number.)
- **FR-502** — on daemon start, unfinished claims are released per namespace, so a namespace whose claim
  was interrupted mid-drain resumes without waiting on any other namespace's state. (Same note: this is a
  base-section requirement, not one of the FR-561–FR-563 repair-round additions.)

**Exactly-once, not merely eventually-once.** An entry can be partially delivered before its namespace is
blocked — for example, the server accepts a first push in a batch, then refuses a later item's entity
type mid-batch. Because the release step reuses the entry's *original* idempotency key rather than a
newly derived one, `INSERT ... ON CONFLICT (idempotency_key) DO NOTHING` (`cairn-server/src/
sync.rs:171-186`) makes redelivering an already-applied entry a no-op rather than a duplicate. An entry
that was applied once before the block is applied once, not twice, after the release — whether the block
took effect before or after the server actually processed it.

## 2b. Ingest refusal is not capability refusal (D447)

Everything in §2 and §2a describes one kind of refusal: the server cannot accept an entity type it has
never heard of, the client parks the item, marks the namespace `blocked`, and re-probes until the server
can. That mechanism is built on an assumption worth stating out loud — **the same bytes will be
acceptable later**. Upgrade the server and the held item goes through unchanged.

Server-side content validation (D447) produces a refusal with the opposite property. The item is refused
because of what it *contains*, and no server upgrade changes that. Retrying the identical payload against
the identical endpoint fails identically, forever. Routing it down the capability path would park an item
that can never be released, mark a namespace blocked that nothing can unblock, and apply a backoff that
throttles every subsequent item in that lane behind one record that will never leave it.

So the two are separate paths, and the difference is visible to the client:

| | Capability refusal (§2, D423) | Ingest refusal (D447) |
|---|---|---|
| Cause | The server's schema does not know this entity type | The content failed `validate_global_content` |
| Wire form | `409` with `unknown_entity_type` | `422` with `content_rejected` and a `class` naming the rejection |
| Item's fate | Held locally, undelivered, unfailed — recoverable (FR-499) | Permanently failed; never delivered, never acknowledged as delivered (FR-581) |
| Namespace state | `blocked`; re-probed on a backed-off schedule (FR-561) | Unchanged — stays eligible, unthrottled |
| Retry unchanged | Succeeds once the server is upgraded (FR-562) | Can never succeed |
| Operator action | None; it resolves itself (FR-563) | Edit or discard the offending record |

**The distinction is a typed field, not a message string** (SC-456). A client that decided which path to
take by substring-matching an error message would work until someone reworded the message, and then fail
silently in one of two ways: a permanent refusal retried forever, or a recoverable item discarded. Both
are worse than a hard parse error, because neither surfaces. `code` is the discriminator; `message` is for
humans.

**The refusal carries a class, never the content** (FR-547, FR-577). The `class` field holds one of the
nine rejection-class names — the server tells the pushing client *which rule* it broke, and the client
already has the record, so it needs nothing else to fix it. The offending substring appears in no
response body, no server log line, and no client diagnostic (SC-439).

**A refused item does not stall the batch.** One rejected record in a push of many is refused
individually; the rest of the batch is applied and the cursor advances past all of it. Rejecting the whole
push on one bad item would reintroduce the stall from the other direction — a single malformed record
holding up a namespace indefinitely, which is precisely what per-namespace backoff (§3) exists to
prevent. The push response therefore reports per-item outcomes, not a single batch verdict.

**What an old client sees.** A pre-004 client never pushes personal or team items, so it never meets this
path at all — its project sync is untouched (§1b, FR-586). A 004 client that has *skipped* its own
validation, which is the case D447 exists for, receives the `422` and must surface it; it has no
pre-existing handler to fall back on, because before this feature the server accepted whatever it was
given.

## 3. The load-bearing conclusion: per-namespace backoff is mandatory, not cosmetic

Steps 1–6 above only produce "project sync keeps flowing at full speed" because backoff and drain state
become **per-namespace** (D427, FR-497). Today, `cairnd/src/sync.rs:56-118` keeps one `hit_transient`
flag and one `backoff` value across the *entire* project loop:

```rust
if hit_transient { tokio::time::sleep(backoff).await; backoff = (backoff * 2).min(BACKOFF_MAX); }
else { backoff = BACKOFF_MIN; }
```

If personal and team namespaces reused this single process-global backoff unchanged, every `blocked`
push attempt against an old server would still count as a transient hit against the *one* shared
`backoff` variable, and the doubling delay (`500ms` up to `30s`) would apply to the next tick of
**every** project's drain, not just to the namespace that failed. A quiet team namespace sitting
`blocked` against an old server would throttle an active project's sync to a crawl for no reason
connected to that project at all. This is exactly why FR-497 ("Backoff after a failed synchronization
attempt MUST be tracked per namespace") and D427 are mandatory changes rather than a nicety: without
them, this feature would make ordinary project sync measurably *worse* on any fleet with even one
schema-1 server anywhere in it, which is the opposite of "an old server continues to serve project
synchronization at full speed" (FR-522). `sync_cursor.backoff_until` (data-model.md §2.10) is where the
now-per-namespace state lives.

## 4. No handshake is added

The one-way advertisement is preserved exactly, with no new protocol machinery (D428, FR-529):
`GET /api/version` already existed and gained a `capabilities` field additively for 003; 004 adds two
more names to the same array and changes nothing else about the endpoint, its authentication (none), or
its shape. The existing rationale applies unchanged (`cairnd/src/sync.rs:786-790`):

> A server that answers without `capabilities` is a server from before the field existed, and its
> silence is the answer: it can hold none of this. That is why there is no probe endpoint and no
> negotiation — `GET /api/version` already existed, and adding to it additively meant an old server
> needed no change at all (D81).

There is still no protocol version header, no `Accept-Version`, and no client-to-server capability
declaration. The client never tells the server what it is; it only ever reads what the server says
about itself.

## 5. Capability and entity-type additions

`SCHEMA_3_CAPABILITIES` extends `SCHEMA_2_CAPABILITIES` (`version.rs:32-38`) additively:

```text
SCHEMA_2_CAPABILITIES = [memory_relations, task_criteria, task_blockers,
                         memory_subject_identity, memory_verification]
SCHEMA_3_CAPABILITIES = SCHEMA_2_CAPABILITIES + [personal_knowledge, team_knowledge]
```

`capabilities_for` (`version.rs:46-52`) gains one more tier: `schema_version >= 3 → SCHEMA_3_CAPABILITIES`
else `schema_version >= 2 → SCHEMA_2_CAPABILITIES` else `&[]` — additive, and a schema-2 server (has run
003 but not yet 004) correctly reports the schema-2 set only.

`ENTITY_CAPABILITIES` (`sync.rs:871-879`) gains two entries:

```text
PersonalKnowledge -> ["personal_knowledge"]
TeamKnowledge     -> ["team_knowledge"]
```

Each entity type needs only its own capability name present — unlike `Memory`, which needs **both**
`memory_subject_identity` and `memory_verification` before it is released. Personal and team knowledge
have no such compound requirement because each is a single self-contained entity type with no
003-era split concept to reunite.

## 5a. A mixed fleet: two devices for one user, one upgraded and one not

The same user's laptop (new client) and workstation (still old) point at the same, already-upgraded
(schema-3) server. The laptop pushes and pulls personal knowledge normally. The workstation's older
binary never constructs a personal knowledge row and never asks the server for one, so it neither
contributes to nor observes that domain — it continues to see only project knowledge, exactly as it did
before this feature existed on either machine. Nothing about this is a degraded or error state for the
workstation: FR-481 ("A caller with no personal or team knowledge of their own MUST see zero difference
... relative to a caller who never touches either domain") describes the intended shape of exactly this
situation, even though here the cause is an old binary rather than an unused feature.

## 6. The `server_instance_id` mismatch rule

A local store records the `server_instance_id` of the server it last synced team knowledge from
(FR-495; `data-model.md`'s local schema does not add a dedicated column for this because it is derived
from the one team-knowledge namespace's identity — `team:<server_instance_id>` in `sync_cursor.namespace`
already carries it). Before importing a pull response's team knowledge, the daemon compares the
response's server instance against the value embedded in the `team:` namespace it is currently
synchronizing; a mismatch means this store has previously linked its team namespace to a different
server instance than the one now answering.

**What the user sees**: the pull is refused for the team namespace only — project and personal
namespaces are unaffected, per the same per-namespace isolation as §3 — and `cairn sync status`/
`cairn doctor` report it by name, e.g. `team knowledge blocked: server instance mismatch (recorded
<id-a>, server reports <id-b>)`, distinct from an ordinary capability-blocked report so an operator does
not mistake "wrong team" for "old server" (FR-496, SC-428). This is treated as a capability boundary,
not silently resolved by combining the two teams' guidance — exactly the framing the spec's edge cases
section states: "a capability boundary rather than silently combining two teams' guidance."

**Where the comparison value comes from on the wire**: `GET /api/version` already carries
`server_instance_id` unauthenticated (FR-416, data-model.md §3.2), so the check runs *before* any team
pull is even attempted — the same per-drain-cycle capability refresh that reads `capabilities` also
reads this field, and a mismatch is caught at that point rather than after a pull response has already
arrived. This mirrors how `refresh_capability` already reads `schema_version` and `capabilities` in the
same response for the existing capability-block mechanism (§2 step 2); `server_instance_id` is a third
field read from the identical response, not a second round trip.

**How this differs from an ordinary capability block**: a capability block is *temporary and
self-resolving* — the same server will eventually be upgraded, and the held work is released (§2 step
6). A server-instance mismatch is *not* expected to self-resolve, because it means this store's team
namespace was linked against a different server than the one currently answering; resolving it is an
operator action (re-linking, or restoring the correct server), not a wait. `cairn doctor` distinguishes
the two so a user does not wait indefinitely for a mismatch to clear on its own the way a capability
block eventually does.

## 7. Corrections to `specs/003-project-intelligence/contracts/privacy-sync.md`

The existing contract already misdescribes the *003* capability mechanism this feature extends, and its
description does not become more correct by 004 shipping — inv-B §10 documents each one against the
live code. 004 does not edit that file (it belongs to a shipped feature); the corrections it would need
if a maintainer later reconciles it are listed here so they are not rediscovered:

| # | The contract says | The code says |
|---|---|---|
| 1 | The field is `capability` (singular), with three values. | The field is `capabilities` (plural, `version.rs:32-38`), with **five** differently-spelled names in schema 2 (`memory_relations`, `task_criteria`, `task_blockers`, `memory_subject_identity`, `memory_verification`) — soon to be seven under `SCHEMA_3_CAPABILITIES`. The client string-matches these exact names (`sync.rs:871-879`); the contract's three singular names match none of them. |
| 2 | The wire check is "an explicit field allowlist enforced on the wire." | `reject_forbidden_fields` (`sync.rs:296-322`) is a **non-recursive, top-level-key denylist** — corrected as part of this feature's own FR-532/FR-535 repair, which additionally makes the check recurse into nested payload structures rather than only fixing the documentation. |
| 3 | 16 (or, elsewhere in the same document, 19) forbidden field names; 6 forbidden entity types. | The live lists have **27** forbidden field names and **9** forbidden entity types (`sync.rs:21-56`, `sync.rs:63-73`). Neither stated count in the contract is correct, and they disagree with each other within the same document. |
| 4 | Silent on the count growing further. | 004 adds no new forbidden entity type. D419 over-claimed why: only the denylist's identifier-shaped fields — observation identifiers, evidence references, project identifiers — have no column on `personal_knowledge` or `team_knowledge` at all (Layer A). The denylist's path-shaped and command-shaped fields are a different mechanism: they can appear inside `content`, `topic_key`, `value_key`, or an applicability value, all free text, and are kept out by `validate_global_content` running at all five entry points including server-side ingest, not by column absence (Layer B, FR-550, SC-467). 004 also adds capability-gated entity types, which is a distinct mechanism from the denylist and should be documented as such rather than folded into the same count. |

A maintainer correcting `privacy-sync.md` should update it to the **live counts and live field names**
at the time of that edit, not to the counts in this table frozen at 004's baseline — inv-B's own finding
was that the contract had already drifted once before it was ever wrong twice.

## 8. Compatibility guarantees checklist

| Guarantee | Requirement | How this document shows it holds |
|---|---|---|
| Old server keeps serving project sync at full speed | FR-522 | §2 step 5, §3 |
| Degradation is reported by name, not silently absorbed | FR-522, SC-425 | §2 step 5, §6 |
| A blocked item is retained, not lost or permanently failed | FR-499 | §2 steps 4–5 |
| A blocked item delivers exactly once after upgrade, unattended | FR-500, SC-426 | §2 step 6 |
| One namespace's failure never stalls another, including a blocked namespace under repeated re-probing | FR-488, FR-497 | §3, §2a |
| A quiet namespace still eventually pulls | FR-489 | migration.md §3 (`sync_cursor` per-namespace polling replaces `sync_meta`'s single conditional pull) |
| No handshake, no negotiation, purely additive advertisement | FR-529 | §4 |
| A local store never mixes two server instances' team knowledge | FR-495, FR-496, SC-428 | §6 |
| Introducing domains requires no change to `MemoryScope` | FR-521 | data-model.md §5 |
| Existing six-tool MCP surface is unaffected by any of this | FR-527, SC-430 | No tool is added; §1's table shows neither old nor new binaries gain a seventh tool as a compatibility side effect |
| A blocked namespace re-probes and self-releases with no local write, command or restart | FR-561, FR-562, FR-563, SC-445 | §2a |
| A released entry is applied exactly once, including one partially delivered before the block | FR-562, SC-445 | §2a ("Exactly-once, not merely eventually-once") |
| An interrupted claim releases per-namespace at daemon start, without waiting on other namespaces | FR-502 | §2a |
| An ingest refusal is permanent and distinguishable from a retryable capability refusal | FR-577, FR-581, SC-449, SC-456 | §2b |
| A pre-004 client synchronizes projects unchanged against a 004 server | FR-586, SC-457 | §1, §1b |
| A removed route names its replacement instead of answering a bare not-found | FR-587, SC-458 | §1b |
| The release documents self-registration and self-join removal in operator-facing terms | FR-588, SC-458 | §1b |

Nothing in this document depends on a client or server knowing the other's exact version beyond what
`GET /api/version`'s `schema_version` and `capabilities` fields already say — which is the same
one-directional amount of knowledge 003 established and 004 does not increase.

## 9. Every compatibility claim in this document has an executable test

No claim above stands on documentation alone. The blocked-namespace liveness cycle (§2a) in particular is
proved by an end-to-end scenario, not by unit-testing `refresh_capability` in isolation: queue personal
and team content against a schema-2 server; connect and observe both namespaces enter `blocked` while
project sync continues on its own, unaffected namespace; replace the peer at the same configured endpoint
with a schema-3 server (§1a); perform no new local write and issue no user command; observe both
namespaces return to `eligible` automatically on their own next probe; confirm the queued content is
delivered exactly once (SC-445). Any future change to this mechanism that this scenario cannot exercise
end-to-end is a gap in this document, not only in the test suite.
