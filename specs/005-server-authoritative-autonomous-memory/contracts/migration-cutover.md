# Contract: Migration and Cutover — Feature 004 to Feature 005

**Feature**: `005-server-authoritative-autonomous-memory`

Authority for personal and team knowledge moves from per-device dual-authority replicas to
the server on a schedule the operator controls. No existing record is lost, duplicated, or
reassigned; an interrupted store resumes; a client that has not migrated keeps working until
its server closes authority, at which point it is told plainly, permanently, and without a
byte of its local store touched.

Two mechanisms stay separate throughout: the **server's authority mode** (§1–§3,
admin-triggered, server-wide) and the **client's migration path** (§4–§9, per-store,
resumable). FR-876e requires retiring the first never to remove the second.

## 1. Authority mode — two tables, two state machines

| | Where | States | Who changes it |
|---|---|---|---|
| Server authority | `server_authority` (data-model.md §6), one row (`id=1`) | `pre_cutover` → `server_authoritative` | An admin, once, via §2 |
| Client migration state | `authority_mode` (data-model.md §5), one row per store | `feature_004` → `migrating` → `server_authoritative` | The client itself, via §4–§9 |

The server's machine is binary: either the fleet's dual-authority write path is open or
closed (FR-876). The client's has a middle state because its migration is a multi-phase,
resumable procedure (§7) independent of the server's decision. A client can reach
`server_authoritative` locally **before** its server cuts over (FR-876a, "migrate normally")
or **after** (FR-876d) — same end state, different timing.

**Advertisement.** `GET /api/version` (unauthenticated, `api.rs:151`) gains one field:

```json
{ "current": "...", "latest": null, "update_available": false, "checked_at": null,
  "schema_version": 4,
  "capabilities": ["memory_relations", "personal_knowledge", "safe_events"],
  "server_instance_id": "3f9a…",
  "authority": { "mode": "pre_cutover", "cutover_at": null } }
```

`schema_version`, `capabilities` and `server_instance_id` are already shipped in this payload
(`crates/cairn-server/src/version.rs:139-150`) and are shown because §9's anti-spoof argument
depends on `server_instance_id` being here.

After cutover, `mode` reads `server_authoritative` and `cutover_at` carries
`server_authority.cutover_at`. A client probes this the way it already probes schema
capability (`sync-namespaces.md`'s re-probe, D437) — no new polling mechanism.

## 2. The cutover procedure

`POST /api/admin/cutover`, `AdminUser` only (FR-876 — "a deliberate admin action, not
emergent"). One statement decides it, the compare-and-swap shape `ratify_team` / `retire_team`
already establish (`global.rs:1068-1160`):

```sql
UPDATE server_authority
   SET mode = 'server_authoritative', cutover_at = now()
 WHERE id = 1 AND mode = 'pre_cutover'
RETURNING cutover_at;
```

- **Zero rows affected** ⇒ already cut over; the handler returns the existing `cutover_at`
  rather than an error. No route back, matching `retire_team`'s posture.
- **Preconditions**: `AdminUser` only. Cutover does not wait for every bound store to have
  migrated — FR-876e supersedes an earlier draft making retirement contingent on a device that
  might never return. An operator judges fleet readiness (§11); the server enforces nothing.
- **Postcondition**: every request matching §3.1 is refused from this instant, read at request
  time — no restart, no other code path.

## 3. The refusal contract

### 3.1 What is refused

Once `server_authority.mode = 'server_authoritative'`:

| Route | Refused when |
|---|---|
| `POST /api/sync/batch` | any item with `entity_type ∈ {personal_knowledge, team_knowledge}` |
| `GET /api/sync/changes/personal` | always |
| `GET /api/sync/changes/team` | always |

Every other `entity_type` (`memory`, `task`, `session`, `handoff`, relations, criteria,
blockers) is **not** refused (FR-877) — project memory authority was already server-side
before 005; only the personal/team dual-authority path retires.

This is a **shape-based** refusal, not a client-version check — the server cannot reliably
learn a caller's binary version. A caller emitting one of these three requests is, by
construction, still speaking the pre-005 dual-authority protocol: a store that has completed
§4–§9 stops emitting them, since personal/team sync is no longer this call's job once its own
`authority_mode` reads `server_authoritative`. This also covers FR-877's harder case — a
Feature 005 binary that has simply not run its own migration — with no special case: on the
wire it is indistinguishable from a Feature 004 client, speaking the same protocol.

### 3.2 Exact shape

```
HTTP 409 Conflict
{ "error": { "code": "upgrade_required",
             "message": "this server has completed its Feature 005 cutover; personal and \
                          team knowledge synchronization now requires a migrated client" } }
```

`upgrade_required` is a distinct `&'static str` from `unknown_entity_type` (the existing
capability-deferral code, `sync.rs:371`, unchanged). Same HTTP status, deliberately: FR-876b1
holds the server cannot control how an already-shipped Feature 004 binary reacts to a code it
has never seen, and that binary already has exactly one branch for a `409` — the
blocked/deferred branch it uses for `unknown_entity_type` today. Reusing the status lets an
old binary retry harmlessly instead of crashing on an unrecognized one: §3.1 always matches,
so the row is never delivered and no forward progress is made, but no damage is done either.

**An upgraded client MUST distinguish the two codes** (FR-876b1): `unknown_entity_type` means
"wait, this server isn't ready"; `upgrade_required` means "stop retrying — this store must
migrate." A Feature 005 client checks `error.code` and, on `upgrade_required`, halts its retry
loop for these namespaces and surfaces the upgrade rather than backing off forever.

### 3.3 What the refusal guarantees (FR-876c)

Read-only from the server's point of view: no row in `personal_knowledge` or `team_knowledge`,
local or canonical, is touched by producing it. Cairn never deletes, demotes, truncates or
rewrites the refused client's **local** store as a side effect — the refusal is the entire
action. "The client is out of date, not wrong" is enforced by there being no other code path
from this handler.

## 4. Migration phases — overview

Client-side, driven by `cairnd` (`migrate005.rs` per plan.md), tracked one row per phase in
`migration_state` (data-model.md §5): `phase ∈ {inspect, drain, verify_possession,
switch_authority, demote}`, `state ∈ {pending, running, done, blocked}`. `blocked` is FR-870's
"clearly reported failure state" — stopped rather than proceeded; `detail_count` names how
many records are implicated. Phases run strictly in order, each precondition being the
previous postcondition; re-running after any interruption re-enters at the first phase not
`done` (§7), never skipping ahead.

| # | Phase | Precondition | Action | Postcondition | On failure |
|---|---|---|---|---|---|
| 1 | Inspect | `authority_mode='feature_004'` | Read-only scan of local knowledge + outbox; emit the report of §5 | Report emitted; mode → `migrating` | Cannot fail — read-only |
| 2 | Drain | phase 1 done | Push queued `personal_knowledge`/`team_knowledge` outbox rows via the existing per-author claim (§6) | Every eligible row `delivered`; every ineligible row `blocked`, reported (§6.2) | Rows remain blocked ⇒ phase `blocked`; migration proceeds for what did drain |
| 3 | Verify possession | phase 2 done or blocked | Call §5's possession endpoint for every delivered id plus every pre-existing id this store believed canonical | Every checked id confirmed `held`, or `missing` and excluded | Any `missing` ⇒ phase `blocked`; that id retained (§7), excluded from 4–5 |
| 4 | Switch authority | phase 3 done for the records concerned | `UPDATE authority_mode SET mode='server_authoritative', changed_at=now() WHERE id=1` | Local reads/writes for personal+team knowledge now treat the server as authoritative | Transaction commits or not — no partial state |
| 5 | Demote | phase 4 done | For every id confirmed `held`, mark its local replica non-authoritative (cache only) | Confirmed replicas demoted; unconfirmed/refused untouched | A record skipped at phase 3 is simply not demoted |

### 4.1 Inspect (FR-863)

Before anything changes, the client counts and reports: `personal_knowledge`/`team_knowledge`
row counts by state; outbox rows for those entity types by state; outbox rows with no
recorded `authored_by_user_id`; local-only project memories with no canonical counterpart
(§6 candidates). Phase 1 writes nothing but this report and its own `migration_state` row.

### 4.2 Drain (FR-864, FR-864a)

Reuses `outbox::claim_namespace_for_author` (`outbox.rs:533-540`) unchanged, over the
`personal:<instance>:<user>` and `team:<instance>` namespaces this store holds — the existing
per-author claim, called from a migration phase rather than the sync loop, never a
namespace-wide sweep with the author filter removed. A bulk sweep bypassing that filter would
deliver a row under whichever account happens to be signed in during migration — the exact
misattribution `outbox.rs:522-531` records as introduced and fixed twice already. A row with
**no recorded author** is drained by no one's migration; it is reported (§4.3) and left
`pending`.

### 4.3 Blocked-row reporting (FR-873)

Every row the claim does not deliver is reported individually — `entity_type`, `entity_id`,
and one of: `no_recorded_author` | `author_mismatch` | `server_rejected: <reason>` |
`capability_blocked`. `server_rejected` carries the server's own rejection reason verbatim.

## 5. Possession verification (FR-865)

"Delivered" and "durably held" are different facts; only the second authorizes demotion.

```
POST /api/migration/possession
Authorization: Bearer <token>

{ "records": [ { "kind": "personal_knowledge", "id": "1a2b..." },
               { "kind": "team_knowledge",     "id": "3c4d..." },
               { "kind": "memory",             "id": "5e6f..." } ] }
```

Bounded to 500 records per call (matching `safe-events.md` §7's batch-bounding discipline); a
larger local set is checked over multiple calls.

```json
{ "held": ["1a2b...", "3c4d..."], "missing": ["5e6f..."] }
```

| `kind` | Held requires |
|---|---|
| `personal_knowledge` | row exists **and** `owner_user_id` = the authenticated caller |
| `team_knowledge` | row exists (server-global, no owner; any authenticated account may verify a team id — FR-888's domain is visible, not secret) |
| `memory` | row exists **and** the caller is a member of its `project_id` |

A record failing its ownership check is reported `missing`, not refused with an error — the
migrating client needs only "the server does not hold this for me." Only ids reported `held`
proceed to phase 5; `missing` ids are retained (§6) and re-checked on the next run.

## 6. Retained, not demoted (FR-871)

| Category | Why unconfirmed | Disposition |
|---|---|---|
| Local-only project memory (never synced, or refused by validation) | Nothing canonical to defer to | Retained, reported, permanently excluded from demotion |
| A record the server explicitly refuses on redelivery | Same | Retained, reported with the server's reason |
| A record reported `missing` at §5 | Possession unconfirmed | Retained, re-checked next run |
| A blocked outbox row (§4.3) | Never reached the server | Retained, reported, drain retried next run |

None of these four is ever discarded, truncated, or merged into a canonical record it does not
match (FR-866). "Excluded from the authority switch" means phase 4's `UPDATE` still runs for
the store as a whole — one unconfirmed record does not hold a store hostage — but phase 5's
demotion never touches these rows: the local copy stays the only copy.

## 7. Resumability (FR-868, FR-869)

Interruption at any point — kill, crash, network loss, server 5xx — leaves `migration_state`
at whatever phase last committed `done`. Re-running:

1. Finds the first phase not `done`, re-enters from its precondition, not from scratch.
2. Each phase is independently idempotent: phase 2's claim skips rows already `delivered`;
   phase 3's check is a pure read; phase 4's `UPDATE` no-ops if already applied; phase 5's
   demotion no-ops for an already-demoted row.
3. Produces **no duplicate canonical knowledge** (FR-868): delivery is idempotent by
   construction — `(writer_id, writer_seq)` uniqueness on `personal_knowledge`/`team_knowledge`
   (004's migration §4 step 6) cannot be violated into a second row by a resend, and drain
   never re-enqueues a row already `delivered`.

A migration that never reaches phase 4 leaves the store fully functional under Feature 004
semantics (FR-877) — resumability is the ordinary mode, not a fallback.

## 8. Key normalization during migration (FR-867a)

Migration normalizes the `topic_key`/`value_key` of every existing record it moves authority
for, using the **same** `normalize_topic_key` / `normalize_value_key`
(`cairn-core/src/knowledge.rs:58,99`) that `consolidation.md` §5 applies to new candidates —
not a copy, not a migration-local reimplementation.

**Why it is load-bearing.** Duplicate and conflict detection match on normalized keys
(`consolidation.md` §5 rows 6–7). A legacy record still carrying an un-normalized key stops
colliding with a normalized candidate that means the same thing, and detection silently
degrades against the knowledge users already have (SC-750).

**What it is not.** Normalizing rewrites only `topic_key`/`value_key` — never a reassignment
of domain, scope, or authorship (FR-867); those columns are untouched.

**Collisions.** Where two existing records normalize to one key, `consolidation.md` §5 rows
6–7 decide it exactly as for any other candidate: same key, same `value_key` ⇒ ordinary
duplicate/reinforcement; differing `value_key` ⇒ a `conflicts_with` relation, basis
`deterministic_rule`, surfaced through team curation or memory detail. **Migration never
discards one side to resolve a collision** — it becomes a conflict record, never a deletion.

## 9. Provenance retained, never authoritative (FR-874) — instance binding survives (FR-875)

`writer_id`/`writer_seq` continue to exist post-migration, answering the same two questions
they always did — who wrote this, and did anything of theirs go missing (research.md §2.5) —
and gain no new use. **No migration step compares two records' `writer_seq` to decide which is
authoritative.** Authority after migration is a single fact per record — the server holds it,
or it does not (§5) — never a comparison between replicas, because that comparison is exactly
what §10 retires.

A refused client (§3) can still call the existing instance-binding check
(`identity-administration.md` §8, unchanged), never gated by `server_authority.mode` and never
part of the refusal path — closing a specific spoof: an attacker cannot manufacture an
`upgrade_required` refusal by pointing a client at a different, non-cutover server, since the
client's record of which instance it is bound to is independent of any single response.

## 10. Two mechanisms, not one (FR-876e)

| | Dual-authority convergence | Migration path |
|---|---|---|
| What it is | Offline multi-writer merge over knowledge each device once treated as its own | Draining queued writes, establishing possession (§4–§9) |
| Owned by | The pre-005 sync protocol | The upgraded client and its migration tooling |
| Fate at cutover | **Retired** — a returning pre-005 device is refused (§3), not merged | **Survives** — FR-876d requires it runnable *after* cutover |
| Why safe to retire alone | A refused device leaves no write for convergence to reconcile | An upgraded device still has local state to hand over, either side |

Retiring convergence does not wait for every bound store to have migrated (superseding an
earlier, contingent condition) — a device that never returns simply refuses if it ever tries.

## 11. Verification obligations (FR-878)

| Guarantee | Asserted by |
|---|---|
| 100% of pre-005 writes post-cutover receive `upgrade_required`, none silently accepted | SC-746 |
| An upgraded client recognizes the code and stops retrying | SC-746 |
| Zero legacy records deleted/demoted/truncated/rewritten by a refusal, compared record-by-record | SC-747 |
| 100% of pre-existing records carry normalized keys post-migration; a normalized candidate collides with its legacy match rather than duplicating | SC-750, over a corpus seeded with un-normalized legacy keys |
| Migration demonstrable on a populated store, guarantees asserted by test | FR-878, following `migration.md` §8's proof discipline: real prior schema (not a hand-written approximation), row/byte equality for untouched tables, an injected mid-migration failure leaves the store on its prior state |

---

## 11.9 Reads are never refused — the refusal is write-shaped

`upgrade_required` applies to **knowledge-bearing writes**. It does not apply to reads.

`GET /api/sync/changes`, `/changes/personal` and `/changes/team` stay available after cutover,
to every client. This is not an oversight to be tightened later: after cutover the local
personal and team replicas are demoted to cache, and a cache with no read path can never
refill. Refusing the read would mean a migrated store loses personal and team knowledge on
local-store loss, contradicting FR-704 and FR-710a — the opposite of what the authority change
promises.

What the reads return is unchanged. What changes is that a client may no longer write back
through the sync path; it uses the commands in `knowledge-commands.md` §3.

A pre-005 client is therefore refused its writes and keeps its reads, which is exactly the
state that lets it keep working while the user upgrades (FR-877).

## 12. Drain exemption, partial migration, possession and re-keying

These rules govern wherever an earlier section is less specific.

### 12.1 The drain path must survive its own refusal

As written, §3.1 refused every `personal_knowledge` / `team_knowledge` sync item after cutover
with no exemption — and phase 2 (drain) delivers exactly those entity types over exactly that
path. A client upgrading after cutover would be refused its own migration, phase 3 would report
every queued record missing, and they would be retained forever. That contradicts FR-876d and
FR-876e.

Correction: drain uses a **distinct, migration-scoped ingest route**,
`POST /api/migration/drain`, which:

- is authenticated, and additionally requires the caller's store to be in `migrating` state,
  asserted by a migration token the server issues when phase 1 registers the migration;
- accepts only the entity types the migration is draining, from that one store;
- is **exempt** from the cutover refusal, because it is the mechanism by which a client stops
  being pre-005 — refusing it would make the refusal self-perpetuating;
- is refused for a store that has not registered a migration, so it is not a general bypass of
  `upgrade_required`;
- closes when the migration completes, so a migrated store cannot keep using it.

`POST /api/sync/batch` remains refused after cutover, unconditionally.

### 12.2 Partial migration: the authority unit is the record, and retained records are named

A single store-wide flag cannot express FR-871. "Excluded from the authority switch" behind a
global `server_authoritative` flag is not a state — the read path would treat the server as
authoritative for a domain that includes records the server provably does not hold, and those
records become unreachable. That is the loss FR-871 and SC-723 exist to prevent, arriving
through the read path instead of the delete path.

**Chosen semantics: per-record retained-local state, with defined read and write behaviour.**
Not all-or-nothing, because one un-transferable record must not block a whole store — which is
also what User Story 7 describes: migration completes, and what could not move is *reported*,
not fatal.

A new local table names the exceptions explicitly:

```sql
CREATE TABLE retained_local (
  domain       TEXT NOT NULL CHECK (domain IN ('project','personal','team')),
  knowledge_id TEXT NOT NULL,
  reason       TEXT NOT NULL,   -- local_only | server_refused | possession_indeterminate
  detected_at  TEXT NOT NULL,
  PRIMARY KEY (domain, knowledge_id)
);
```

`authority_mode` becomes `server_authoritative` for the store. A record in `retained_local` is
the stated exception to it, and its behaviour is defined rather than implied:

| Aspect | Retained record |
|---|---|
| **Read** | served from the local store, and labelled *retained-local* wherever it is read or displayed, so it is never mistaken for canonical knowledge |
| **Recall** | included in briefings from the local corpus, since it is real knowledge this machine holds |
| **Write** | permitted **only** for `local_only` and `server_refused`. A record retained as `possession_indeterminate` — the server may hold it but would not confirm to this caller — is **read-only locally**, because writing a record the server may own would create a second truth (FR-712). |
| **Demotion** | never (FR-872) |
| **Deletion** | only by the user, explicitly |
| **Sync** | never re-attempted automatically; `cairn migrate --retry-retained` re-attempts on demand |
| **Reporting** | listed individually with its reason by `cairn migrate --status` and `cairn status --durability` (SC-723) |
| **Durability** | explicitly **outside** FR-703's guarantee — it is named in the durability-loss list, because deleting the local store destroys it |

The set is expected to be small and mostly one reason: `local_only` records, which the user
chose to keep local and which were never eligible to move. A `server_refused` entry is a defect
signal and is surfaced as one.

When the set empties — every retained record either transferred on retry or deleted by the
user — the store is fully server-authoritative with no exceptions, and `cairn migrate --status`
says so.

### 12.3 Possession is re-checked at demotion

Phases 3 and 5 are separate, so a server-side loss between them could demote the last copy.
Demotion re-runs the possession check for the records it is about to demote, in the same step,
and demotes only what comes back held. FR-872 is then satisfied at the moment that matters
rather than only at the moment of the earlier check.

### 12.4 Re-keying: both key kinds, with the shipped topic normalizer unchanged

FR-867a and SC-750 require existing records' **topic and value** keys to be normalized, so the
phase re-runs both:

- **Topic keys** are re-normalized with the **shipped** `normalize_topic_key` unchanged. It is
  idempotent, so an already-normalized key is untouched and a legacy un-normalized one is
  corrected. What the phase must **not** do is apply a variant that folds `.`: `.` is a segment
  separator (`crates/cairn-core/src/knowledge.rs`, split before `normalize_segment`), and
  folding it would rewrite `test.command` to `test_command` across every record.
- **Value keys** are re-normalized with the **new** folding from `contracts/extraction.md` §7,
  which is the behaviour change this feature introduces.

Where two records collide on a normalized key, the collision is surfaced through the ordinary
conflict machinery; neither record is discarded.

### 12.5 Possession is not an existence oracle

The possession check answers only for records the caller could already see: project records in
projects the caller is a member of, personal records the caller owns, and team records visible
to the caller under the existing team visibility rules. A `proposed` team record the caller may
not see is answered **indeterminate** — never "not held", which would be a lie the client would
act on by retaining a writable copy. An indeterminate record is retained read-only (§12.2) and
is re-checked on the next `--retry-retained`, so possession cannot be used to probe for other
people's proposals and cannot silently fork a record the server owns.
