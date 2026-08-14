# Contract: Reusable Cross-Project Patterns

**Feature**: `003-project-intelligence`

A project memory must never simply become a global memory. A **reusable pattern** is a different
record, with no project identity, promoted only through a deterministic fail-closed gate, and
trusted only by evidence from projects other than the one it came from.

```text
project knowledge (verified, evidence-backed, active, unconflicted)
    │
    │  explicit proposal — never automatic (FR-395)
    ▼
promotion gate — ten deterministic checks, order fixed, fail closed
    │                                    │
    │ pass                               │ any fail → refused, class named,
    ▼                                    │            value never echoed,
sanitized pattern                        └──────────── nothing written
    │
    │  signal match in another project
    ▼
suggested — always "unverified in this project"
    │
    ├── resolved in a distinct non-origin project, independently or with local evidence → validated
    └── not applicable / failed                                                          → contested
```

Patterns are **local to the machine and never synchronize** (FR-508). They have no outbox entity type
and no server table, which is what makes that a property of the schema rather than a promise.

## Anatomy

| Field | Bound | Notes |
|---|---|---|
| `title` | ≤128 | Sanitized |
| `problem` | ≤1024 | What goes wrong |
| `signals` | 2–16 entries, each ≤128 | Normalized symptom tokens and error signatures |
| `signal_digest` | 64 hex | SHA-256 over the sorted normalized signal set. Used for matching **and** duplicate detection — one representation, so the two cannot disagree |
| `applicability` | ≤8 entries | The conditions under which it applies |
| `root_cause` | ≤1024 | |
| `root_cause_digest` | 64 hex | With `signal_digest`, the duplicate key |
| `approach` | ≤2048 | The known resolution |
| `constraints` | ≤8 entries | Caveats — what the approach does *not* do |
| `trust` | enum | `candidate \| sanitized \| validated \| contested` — derived |
| `origin_ref` | 64 hex | **Opaque.** A machine-salted digest of the source project id. Never a name, path or remote |
| `origin_deleted` | bool | Set when the origin project or source memory is deleted |
| `sanitization_report` | JSON | Which gate classes ran and passed. Names classes, never values |

Worked example — the brief's case:

```text
title           Docker cannot allocate a non-overlapping bridge network
problem         Container creation fails because Docker cannot allocate a bridge
                subnet that does not overlap an existing one
signals         ["could not find an available non-overlapping ipv4 address pool",
                 "docker bridge network create failure"]
applicability   ["Docker bridge networking in use",
                 "the error names address-pool allocation",
                 "the configured default address pools are actually exhausted"]
root_cause      The daemon's default-address-pools are fully allocated
approach        Expand default-address-pools in the daemon configuration and restart
constraints     ["existing networks are not migrated to the new pool",
                 "verify the ranges are genuinely exhausted before changing daemon config"]
trust           validated
origin_ref      4d1c…  (opaque)
```

Note what is absent: no project name, no repository, no absolute path, no host, no user. That absence
is enforced, not intended (gate check 8).

## The promotion gate

Ten checks, in this fixed order so the reported reason is stable. Any failure refuses, names the
class, never echoes the offending value, and writes nothing (FR-396, FR-397).

| # | Check | Refusal class |
|---|---|---|
| 1 | Source memory lifecycle is `active` | `source_not_active` |
| 2 | Source `verification = 'verified'` | `source_unverified` |
| 3 | Source has ≥1 supporting evidence fact | `no_evidence` |
| 4 | Source is not `local_only` | `local_only_memory` |
| 5 | Source's subject is not `Conflicted` | `source_conflicted` |
| 6 | Source is transferable: `type` ∈ `procedure \| failure \| decision`, **or** a `convention` with no `topic_key` bound to project configuration. A `fact` is never transferable | `not_transferable` |
| 7 | Content, after redaction, still matches the redaction pattern set | `possible_secret` |
| 8 | Content contains an absolute path, the project name, the normalized `repository_remote`, the `server_project_id`, the `git_common_dir`, or an email address | `project_identifying` |
| 9 | `signals` normalizes to ≥`pattern_signals_min` (2) entries | `insufficient_specificity` |
| 10 | No existing pattern has the same `(signal_digest, root_cause_digest)` | `duplicate_pattern` |

Check 6 is the one that earns its place: "production database is PostgreSQL" is true, verified,
evidence-backed and completely untransferable. A pattern must describe a *problem and its
resolution*, not a project's configuration.

Check 10 is enforced by a unique index as well as by the check, so a race cannot create a duplicate.

The `sanitization_report` records which classes ran and passed:

```json
{ "checks": ["source_active","source_verified","evidence_present","not_local_only",
             "not_conflicted","transferable_type","secret_scan","project_identifier_scan",
             "signal_specificity","duplicate_scan"],
  "redactions_applied": 2, "outcome": "passed" }
```

`redactions_applied` is a count, never the redacted text.

## Suggestion

A pattern reaches Level 1 context only when the **receiving project's own recorded signals** match
(FR-398):

```text
project signals = normalized tokens from
    • `error` observations in the current and previous session
    • `failure`-type memories in the applicable scopes
    ▼
overlap with pattern.signals ≥ pattern_signals_min (2) tokens, lexically
    ▼
at most patterns_in_context_max (2), highest overlap first
```

Every suggestion is labelled (SC-312):

```text
PRIOR PATTERN (unverified in this project)
  Docker cannot allocate a non-overlapping bridge network — trust: contested
  Applies when: Docker bridge networking · the error names address-pool allocation
                · the pools are actually exhausted
  Known approach: expand default-address-pools and restart
  Caveat: existing networks are not migrated
  ⚠ Known alternative cause: a VPN route collision produced the same symptom elsewhere.
    Check this first: verify the configured network ranges are genuinely exhausted.
```

Never presented as fact, never presented as verified here, and the applicability conditions travel
with it so the agent can rule it out cheaply.

## Applications, independence and trust

`pattern_applications` is unique on `(pattern_id, project_id, signal_digest)`. That single key is the
anti-poisoning mechanism: ten sessions in one project describing one incident produce **one** row, so
the distinct-project count is 1 (SC-314).

| Field | Values |
|---|---|
| `outcome` | `resolved \| not_applicable \| failed` |
| `discovery` | `independent \| cairn_suggested` |
| `evidence_id` | Deterministic evidence collected **in the applying project** |
| `is_origin` | True when the applying project is the pattern's origin — excluded from trust |
| `alternative_cause` | Bounded; present on `not_applicable` where known |

Trust derivation (`rebuild_pattern_trust`):

```text
counters (computed, never stored as authority):
  applications              = count(*)
  distinct_projects_applied = count(distinct project_id)
  qualifying_successes      = count(distinct project_id) where
                                 outcome = resolved
                             AND is_origin = 0
                             AND (discovery = independent OR evidence_id IS NOT NULL)
  counterexamples           = count(*) where outcome IN (not_applicable, failed)

trust:
  gate not passed                                → candidate
  counterexamples > 0                            → contested
  qualifying_successes >= 1                       → validated
  otherwise                                       → sanitized
```

`contested` is evaluated before `validated`, so a pattern with both successes and counterexamples
reports `contested` and both sides are stated (FR-405). It is never deleted and never demoted below
the evidence.

Three things that deliberately do **not** advance trust:

1. **Repetition.** Ten applications in one project collapse to one row.
2. **The origin project.** `is_origin` applications are excluded — a pattern cannot validate itself
   where it came from.
3. **Cairn's own suggestion, unaided.** `discovery = cairn_suggested` with no `evidence_id` counts as
   an application and not as a validation (FR-403). This closes the loop the brief warns about: an
   agent reading Cairn's suggestion and agreeing with it is not independent confirmation.

No count is ever presented as a number of independent verifications, anywhere (FR-406). The reported
shape is explicit:

```text
applications 12 · distinct projects 3 · independently validated in 1 · counterexamples 2
```

## Counterexamples

A counterexample is an application, not a deletion (FR-404, D64):

```text
project B: same symptom, different cause
  cairn_remember action=record_outcome pattern_id=… outcome=not_applicable \
    alternative_cause="VPN route collision, not pool exhaustion"
    ▼
  no success count increases
  pattern retained, trust → contested
  future suggestions carry the alternative cause and a "check this first" line
```

The "check this first" line is derived from the alternative cause's own signals, so it says what to
rule out rather than repeating the caveat.

## Deletion and origin

| Event | Effect on the pattern |
|---|---|
| Source memory deleted | Pattern survives; `source_memory_id` cleared, `origin_deleted = 1` |
| Origin project deleted or unlinked | Pattern survives unchanged — it never held project identity |
| Origin reference resolved | Reports **origin deleted**, never a dangling reference and never restored content (FR-399, FR-505) |
| Pattern deleted | Tombstoned; applications survive as history with their text cleared |

## Surfaces

| Surface | Operation |
|---|---|
| `cairn pattern list [--trust <t>] [--signal <token>]` | List, with counters |
| `cairn pattern show <id>` | Full text, applications, counterexamples, sanitization report |
| `cairn pattern promote --memory <id> [--signal …] [--dry-run]` | Propose; `--dry-run` reports the gate outcome without writing |
| `cairn pattern outcome <id> --outcome <o> [--alternative-cause …]` | Record an application |
| `cairn pattern forget <id>` | Tombstone |
| `cairn_remember action=promote` | Agent-proposed promotion |
| `cairn_remember action=record_outcome` | Agent-recorded outcome |
| `cairn_search include_patterns=true` | Patterns as a separate `patterns` array, never mixed into `results` |
| `cairn_context include_patterns` | Level 1, signal-matched, capped, labelled |

`--dry-run` on promote matters: the gate's whole value is that it explains a refusal, and a developer
should be able to ask before committing to the wording.

## Error codes

| Code | Meaning |
|---|---|
| `source_not_active`, `source_unverified`, `no_evidence`, `local_only_memory`, `source_conflicted`, `not_transferable`, `possible_secret`, `project_identifying`, `insufficient_specificity`, `duplicate_pattern` | The ten gate refusals |
| `pattern_not_found` | |
| `outcome_already_recorded` | An application for this `(pattern, project, signal_digest)` exists; use a new signal set or amend |

Every refusal is `ok: false` — promotion is a configuration-class operation and fails loudly rather
than soft (Feature 002 FR-196's discipline applied here).
