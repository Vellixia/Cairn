# Contract: Privacy Boundary and Synchronization

**Feature**: `003-project-intelligence`

Feature 001 drew its privacy boundary **structurally**: there is no observation entity type in the
outbox, no observations table on the server, and an explicit field allowlist enforced on the wire.
Feature 003 keeps that method. Everything it declines to share, it declines by having nowhere to put
it — not by a rule someone must remember.

## The boundary, by record

| Record | Local | Syncs | Mechanism that makes it so |
|---|---|---|---|
| Observation | ✓ | **never** | No `OutboxEntityType` variant; no server table (Feature 001) |
| Memory | ✓ | ✓ (unless `local_only`) | Existing `Memory` entity type, payload extended |
| Memory relation | ✓ | ✓ | **New** `MemoryRelation` entity type |
| Task criterion | ✓ | ✓ | **New** `TaskCriterion` entity type |
| Task blocker | ✓ | ✓ | **New** `TaskBlocker` entity type |
| Task (title, goal, status) | ✓ | ✓ | Existing `Task` payload, unchanged |
| Task local counter | ✓ | **never** | Removed from the payload and from the server schema — it is a local concurrency token (D80) |
| Task state digest | ✓ | **derived** | Computed on both sides from converged records; nothing transmits it |
| Evidence fact | ✓ | **never** | No entity type, no server table |
| Evidence link | ✓ | **never** | No entity type, no server table |
| Verification run | ✓ | **never** | No entity type, no server table |
| Continuity checkpoint | ✓ | **never** | No entity type, no server table |
| Reusable pattern | ✓ | **never** | No entity type, no server table |
| Pattern application | ✓ | **never** | No entity type, no server table |
| Task change log | ✓ | **never** | No entity type, no server table |
| Criterion evidence link | ✓ | **never** | No entity type, no server table |
| Selection diagnostics | ✓ | **never** | Computed, never persisted |
| Integration record (Feature 002) | ✓ | **never** | Unchanged |

`OutboxEntityType` gains exactly three variants. The existing
`outbox_cannot_carry_observations` test is extended to assert that none of the local-only records
above has one either — so adding a variant later is a visible, deliberate act that fails a test until
it is reviewed.

## What a shared memory says about evidence

The whole of it. Five fields, no more (FR-502, D66, D76):

```json
{ "verification": { "state": "verified",
                    "authority": "cairn",
                    "last_verified_at": "2026-08-14T09:12:04Z",
                    "fact_count": 2,
                    "basis": ["configuration", "git_ref"] } }
```

Five keys, no more. `authority` is `cairn` or `attested` — what established the state on the sending
machine. `basis` carries **verifier kind names only**: never a subject, an observed value, a locator, a
digest or a fingerprint.

**Why `authority` has to be on the wire.** Without it a peer cannot tell a deterministic check from an
attestation, because `basis` does not settle it: `test_outcome` and `command_outcome` are each reachable
either way — from a captured observation, or from an agent's submission. An attested claim would arrive
as `{state: verified, basis: ["test_outcome"]}` and be rendered exactly like a peer that had really run
the tests. That is the gap this closes (FR-370, D76, SC-329).

`authority` is an enumerated value carrying no content, so it costs nothing against the privacy
boundary — and it is required *by* that boundary's purpose, which is honesty about what is known.

A receiving peer maps it: `cairn` → `remote_cairn`, `attested` → `remote_attested`. It never stores the
sender's value verbatim, because "verified here" is a claim only the local machine can make (FR-368).

What a teammate learns: *this memory was verified on another machine at this time, by a deterministic
check, against a configuration value and a Git reference, by two facts.* What they do not learn: which
file, which key, which value, which ref.

This is a strict extension of the rule Feature 001 already enforces for observations — identifiers and a
count, never the rows behind them. If they need the value, they look on the machine that verified it.
That is the trade, and it is deliberate.

## The extended memory payload

Added to `outbox::memory_payload`:

| Field | Notes |
|---|---|
| `topic_key`, `value_key` | Normalized, bounded, no free text beyond the key |
| `importance` | enum |
| `effective_from`, `superseded_at` | timestamps |
| `pinned` | bool. `pin_reason` is **not** sent — it is free text about local context |
| `reinforcement_count`, `distinct_origin_count` | integers |
| `verification` | the five-field object above, including `authority` |

Not sent: `content_norm_digest` (a local index), `local_revision` on a task (local by definition, D80),
`pin_reason` (free text), and the memory's *local* authority value — the sender transmits `cairn` or
`attested`, never `remote_*`, because relaying a third machine's authority would be a claim it cannot
support.

## The relation payload

```json
{ "from_memory_id": "…", "to_memory_id": "…", "kind": "supersedes",
  "decided_by_session": "…", "decided_at": "…", "basis": "evidence" }
```

`basis_evidence_id` is **stripped**. `rationale` is **stripped** — it is bounded and redacted, but it
is free text a session wrote about local context, and the conservative default applies.

A peer receiving `basis: "evidence"` with no identifier reads it correctly: the decision was
evidence-backed on another machine. Truthful, and it leaks nothing.

## Server schema and allowlist delta

`cairn-server/migrations/0002_project_intelligence.sql` — additive only.

**Columns added to `memories`**: `topic_key`, `value_key`, `importance`, `verification`,
`last_verified_at`, `verification_basis JSONB`, `evidence_fact_count`, `effective_from`,
`superseded_at`, `pinned`, `reinforcement_count`, `distinct_origin_count`.

**No column added to `tasks`.** The local counter is not transmitted and the state digest is derived,
so the server's `tasks` table is untouched (D80).

**Tables added**: `memory_relations` (no `basis_evidence_id`, no `rationale`), `task_criteria`,
`task_blockers`.

**Tables deliberately absent** — this list *is* the privacy boundary:

```text
evidence_facts            memory_evidence_facts     verification_runs
continuity_checkpoints    reusable_patterns         pattern_applications
task_changes              criterion_evidence        observations (Feature 001)
```

**`FORBIDDEN_OBSERVATION_FIELDS` extended** with the Feature 003 field names that would carry evidence
or diagnostic content, so a malformed or malicious payload is refused on the wire rather than trusted
not to exist (FR-506):

```text
observed_value   source_locator   value_digest    fingerprint
relevant_paths   criteria_snapshot                sanitization_report
origin_ref       alternative_cause                signal_digest
pin_reason       rationale        basis_evidence_id
path_fingerprints                 task_snapshot_at_bind
detail           prior_value      new_value       content_norm_digest
```

**New forbidden entity types**: `evidence_fact`, `verification_run`, `continuity_checkpoint`,
`reusable_pattern`, `pattern_application`, `task_change` — refused outright by name, exactly as
`observation` is (`reject_forbidden_fields`).

Asserted by `privacy_payloads`, extended: for every forbidden field name and every forbidden entity
type, a crafted payload is rejected and the rejection names the field.

## Read-back

`GET /api/sync/changes` gains optional arrays. Existing `memories` is unchanged in shape apart from
the new memory fields:

```json
{ "memories": [ … ], "relations": [ … ], "criteria": [ … ], "blockers": [ … ],
  "cursor": "2026-08-14T09:12:04Z" }
```

One cursor over `updated_at` across all four, so a partial read cannot leave a relation whose memory
has not arrived — relations for an unknown memory are held and retried rather than dropped.

`GET /api/projects/{id}/memories` gains `verification` and `subject` state so the web UI can show
what is verified and what is conflicted, from the fields the server legitimately holds.

## Import, and why nothing is overwritten

```text
memories   INSERT OR IGNORE            (unchanged — Feature 001 behaviour, research B2)
relations  INSERT OR IGNORE on PK      (idempotent by construction)
criteria   upsert by id, per-criterion (different criteria never collide)
blockers   upsert by id, one transition
    ▼
re-derive locally:
    memories.state / superseded_by_id  ← from `supersedes` relations
    reinforcement counts               ← from `reinforces` / `duplicates` relations
    verification_authority              ← 'cairn' → remote_cairn, 'attested' → remote_attested
    task_state_digest                   ← recomputed from the converged criteria and blockers
    tasks.acceptance_criteria           ← from criteria
```

The correction this makes to the baseline: today `import_memory` returns early when the row exists, so
a supersession decided on another machine **never lands locally**. Importing the *decision* and
re-deriving fixes it without introducing row overwriting (research B2, D67, R5).

Nothing in this path compares timestamps to choose a value (FR-411). `clock_swap_invariance` runs the
whole merge corpus with the two stores' clocks reversed and asserts a byte-identical result
(SC-304).

## Degradation against an older server

An alpha deployment will run mixed versions. When the server rejects a Feature 003 field or entity
type (FR-415, SC-326):

```text
1. classify the rejection
     capability: unknown_entity_type | unknown_field | schema_older
     content:    everything else (a malformed or forbidden payload)
2. a CONTENT rejection is permanent, exactly as today → outbox state `failed`
3. a CAPABILITY rejection is NOT permanent → outbox state `blocked`
     the row keeps its idempotency key and its payload
     `blocked_reason` records the class
     `blocked_at_capability` records the server capability observed at the time
4. record the class against the project in sync_meta
5. stop sending that class — do not retry it, and do not fail the batch
6. keep delivering everything the server does accept
7. report it in `cairn sync status` and `cairn doctor`
```

### `blocked` — and how it recovers

The first design stopped at step 5, which stranded the work: the daemon marks a rejection `failed`, and
`outbox::claim` only ever takes `pending` rows and stale `in_flight` ones. A relation refused by an old
server would have stayed refused forever, even after the server was upgraded (D81).

`blocked` is a fifth outbox state, distinct from both `pending` and `failed`:

| State | Claimable | Meaning |
|---|---|---|
| `pending` | yes | Waiting to be delivered |
| `in_flight` | on stale claim | A drainer holds it |
| `delivered` | no | Applied by the server |
| `failed` | no | **Permanently** refused — the content is not acceptable |
| `blocked` | **not until the server changes** | Refused for lack of server capability; retained, deliverable later |

**Unblocking.** The server's existing public `/api/version` endpoint gains a `capability` block —
additive, unauthenticated, already called by the web UI:

```json
{ "current": "0.2.0", "schema_version": 2,
  "capabilities": ["memory_relation", "task_criterion", "task_blocker"] }
```

An **older** server returns no such fields, and that absence is itself the answer: no Feature 003
capability. So the probe works against the very servers it needs to detect.

```text
sync worker, once per drain cycle at most, cached in sync_meta:
    probe /api/version → observed_capability
    if observed_capability differs from the recorded one:
        UPDATE outbox SET state = 'pending', blocked_reason = NULL
         WHERE state = 'blocked'
           AND blocked_reason names a class the new capability now supports
```

Then the ordinary drain delivers them, with their **original idempotency keys**, so the server's
`sync_state` claim makes the delivery apply exactly once even if the row was partially delivered
earlier (FR-418, SC-331).

No user intervention, no manual repair of stored data, and no continuous retry against a server known to
lack the capability. `cairn sync status` counts blocked rows so the state is visible rather than
mysterious:

```text
$ cairn sync status
  pending 0 · blocked 12 · failed 0 · last success 2026-08-14T09:14:02Z
  ⚠ degraded: this server does not accept memory relations or task criteria
    (server schema 1, this build expects 2). 12 items are retained and will be
    delivered automatically when the server is upgraded. Memories, tasks,
    sessions and handoffs are syncing normally.
```

```text
$ cairn sync status
  pending 0 · failed 0 · last success 2026-08-14T09:14:02Z
  ⚠ degraded: this server does not accept memory relations or task criteria
    (server schema 1, this build expects 2). Memories, tasks, sessions and handoffs
    are syncing normally. Upgrade the server to share reconciliation decisions.
```

The reverse direction — an older daemon against a newer server — needs nothing: the daemon does not
send the new fields and ignores the new read-back arrays, because `serde` defaults handle absent
fields and unknown arrays are not read.

## Promotion — the highest-risk path

FR-507 names cross-project promotion the feature's largest privacy risk, so its gate is
deterministic and fails closed. Full check list in [patterns.md](./patterns.md); the privacy-relevant
ones:

| Check | Refuses when |
|---|---|
| `local_only_memory` | The source is `local_only` (FR-504) |
| `possible_secret` | The content still matches the redaction pattern set **after** redaction |
| `project_identifying` | The content contains an absolute path, the project name, the normalized `repository_remote`, the `server_project_id`, the `git_common_dir`, or an email address |

A refusal names the class and **never echoes the value** (FR-397). Nothing partial is written.

`origin_ref` is a machine-salted digest of the source project id: stable on this machine, meaningless
off it, and not reversible to a project name (FR-393).

`privacy_promotion` runs a seeded adversarial corpus — provider keys in every shape the redaction set
knows, PEM blocks, JWTs, connection strings, absolute POSIX and Windows paths, UNC paths, the project
name in several casings, the remote with and without credentials, a `server_project_id`, and an email
— asserting 100% refusal, no echoed value, and no partial pattern (SC-315).

## Deletion

Feature 001's semantics, extended to every new reference type (FR-505):

| Deleted | Effect |
|---|---|
| Observation | Evidence facts referencing it report **evidence deleted**; supported memories → `needs_recheck` |
| Evidence fact | Tombstoned: identity and provenance survive, value/locator/digest/fingerprint cleared; links survive and resolve to **evidence deleted** |
| Memory | Existing tombstone. Relations naming it survive and resolve to **memory deleted**; a promoted pattern survives with `origin_deleted = 1` |
| Session | Existing tombstone. Its checkpoints are tombstoned with it; relations it decided survive with the session reported deleted |
| Task | Existing tombstone. Criteria, blockers and changes tombstoned with it; task-scoped memory follows the existing `mark_stale_scopes` behaviour |
| Project | Existing tombstone. Patterns promoted from it survive, origin-opaque, with `origin_deleted = 1` |

No deletion ever leaves a dangling reference, and none ever restores content.

## Verification table

| Guarantee | How it is proved |
|---|---|
| Raw observations never sync | No entity type; no server table; `reject_forbidden_fields`; `outbox_cannot_carry_observations` |
| Evidence content never syncs | No entity type; no server table; 16 forbidden field names; `privacy_payloads` |
| Verification runs, checkpoints, patterns, applications, task changes never sync | No entity type; no server table; forbidden entity-type names |
| A shared memory says only state/authority/instant/count/kinds about evidence | Payload construction test asserting exactly five keys under `verification` |
| An attested verification never arrives as a deterministic one | `authority` on the wire; `us7_offline_merge::authority_survives` (SC-329) |
| Capability-refused work is never stranded | `blocked` outbox state + capability probe; `sync_degradation::recovers_after_upgrade` (SC-331) |
| `local_only` never transmits | Existing: no outbox row is produced at all |
| Promotion leaks nothing | Ten-check gate + seeded adversarial corpus (SC-315) |
| No absolute path is stored in evidence | Column constraint + locator validation + `privacy_payloads` |
| A deleted origin reports deletion | Deletion table above, one test per row |
| An older server degrades and says so | `sync_degradation` end-to-end against a schema-1 server (SC-326) |
