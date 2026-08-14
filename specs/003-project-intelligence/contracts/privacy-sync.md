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
| Task (revision) | ✓ | ✓ | Existing `Task` payload, one field added |
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

The whole of it. Four fields, no more (FR-502, D66):

```json
{ "verification": { "state": "verified",
                    "last_verified_at": "2026-08-14T09:12:04Z",
                    "fact_count": 2,
                    "basis": ["configuration", "git_ref"] } }
```

`basis` carries **verifier kind names only**. Never a subject, an observed value, a locator, a digest
or a fingerprint.

What a teammate learns: *this memory was verified here, at this time, against a configuration value
and a Git reference, by two facts.* What they do not learn: which file, which key, which value, which
ref.

This is a strict extension of the rule Feature 001 already enforces for observations — identifiers and
a count, never the rows behind them. If they need the value, they look on the machine that verified
it. That is the trade, and it is deliberate.

`verification_origin` is **not** in the payload. A receiving peer sets it to `remote` itself, because
"verified here" is a claim only the local machine can make (FR-368).

## The extended memory payload

Added to `outbox::memory_payload`:

| Field | Notes |
|---|---|
| `topic_key`, `value_key` | Normalized, bounded, no free text beyond the key |
| `importance` | enum |
| `effective_from`, `superseded_at` | timestamps |
| `pinned` | bool. `pin_reason` is **not** sent — it is free text about local context |
| `reinforcement_count`, `distinct_origin_count` | integers |
| `verification` | the four-field object above |

Not sent: `content_norm_digest` (a local index), `verification_origin` (local by definition),
`pin_reason` (free text).

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

**Column added to `tasks`**: `revision`.

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
    verification_origin = 'remote'      for any imported verification state
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
1. classify the rejection: unknown_entity_type | unknown_field | schema_older
2. record the class against the project in sync_meta
3. stop sending that class — do not retry it, and do not fail the batch
4. keep delivering everything the server does accept
5. report it in `cairn sync status` and `cairn doctor`
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
| A shared memory says only state/instant/count/kinds about evidence | Payload construction test asserting exactly four keys under `verification` |
| `local_only` never transmits | Existing: no outbox row is produced at all |
| Promotion leaks nothing | Ten-check gate + seeded adversarial corpus (SC-315) |
| No absolute path is stored in evidence | Column constraint + locator validation + `privacy_payloads` |
| A deleted origin reports deletion | Deletion table above, one test per row |
| An older server degrades and says so | `sync_degradation` end-to-end against a schema-1 server (SC-326) |
