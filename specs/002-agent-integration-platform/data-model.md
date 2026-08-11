# Data Model: Agent Integration Platform

**Feature**: `002-agent-integration-platform` | **Date**: 2026-08-11

Feature 002 adds the integration layer's own entities. It does **not** change Project, Task,
Session, Observation, Memory, Handoff, Repository State, User, Project Member, or Sync
Outbox — see [001's data model](../001-cairn-mvp/data-model.md). Two additive extensions to
existing entities are listed at the end.

## Conventions

- Identifiers are UUIDv7 where a record needs one, matching Feature 001.
- Timestamps are RFC 3339 UTC strings.
- **Privacy class** is one of:
  - `local` — lives on this machine only, never enters the outbox, never reaches the server;
  - `derived` — computed at read time, never persisted;
  - `shared` — already covered by Feature 001's sync allow-list.
  **Every entity in this document is `local` or `derived`.** Feature 002 adds nothing to the
  server (FR-183, FR-184).
- "Canonical hash" means the first 12 hex characters of the SHA-256 of a deterministic
  normalization: for text, trailing whitespace stripped per line and a single trailing
  newline; for structured entries, keys sorted and formatting removed.

---

## Desired-state entities (in memory, `derived`)

These are computed per invocation from the developer's choices, the local integration
record, and detection. They are never written to a project file in this feature (FR-202).
They are serializable so a later feature can expose them (FR-226).

### DesiredIntegrationState

The single canonical statement of intent that onboarding, preview, doctor, repair, and
migration all consume (FR-201).

| Field | Type | Notes |
|---|---|---|
| `schema` | integer | `desired_state_schema`, currently `1` (D26) |
| `agents` | list of `DesiredAgent` | One per agent the developer selected |
| `manager` | `DesiredManager` or null | Present when a manager participates |

**Invariants**

- Deterministic: identical inputs serialize byte-identically, on any machine (SC-135).
- Secret-free: contains no token, credential, or key, by construction — no field can hold
  one.
- Path-free: contains no machine-specific absolute path. Locations are named by
  `InstallationScope` plus resource kind; the concrete path is resolved at apply time from
  the scope matrix.
- Every `(agent, resource_kind)` appears exactly once.

### DesiredAgent

| Field | Type | Notes |
|---|---|---|
| `agent` | `AgentId` | `claude-code` \| `codex` \| `opencode` \| `generic-mcp` |
| `enabled` | bool | False means "disconnect this agent" |
| `resources` | list of `DesiredResource` | One per applicable resource kind |
| `requested_level` | `IntegrationLevel` or null | What the developer asked for; the achieved level is computed, never taken from here |

### DesiredResource

| Field | Type | Notes |
|---|---|---|
| `kind` | `ResourceKind` | |
| `owner` | `ResourceOwner` | Exactly one (FR-145) |
| `scope` | `InstallationScope` | |
| `desired_version` | `ArtifactVersion` | Schema + revision for versioned artifacts; null for the MCP entry and lifecycle handlers, which carry `adapter_version` |
| `desired_activation` | `ActivationState` | `not_applicable` \| `active` — states the intent that a trust-gated handler should end up running (FR-209) |

### DesiredManager

| Field | Type | Notes |
|---|---|---|
| `manager` | `ManagerId` | `cc-switch` |
| `target_apps` | list of string | The manager's own application identifiers, e.g. `claude`, `codex`, `opencode` |
| `resources` | list of `ResourceKind` | Only `mcp` and `skill` are manager-distributable in this feature |

---

## Enumerations

### ResourceKind

`mcp` | `lifecycle` | `instructions` | `skill`

The four things Cairn installs per agent. `generic-mcp` has only `mcp`.

### ResourceOwner

| Value | Meaning |
|---|---|
| `direct` | Cairn installed it and Cairn maintains it |
| `manager` | An integration manager distributes it; Cairn verifies but never removes it (FR-149) |
| `external` | It exists, Cairn did not install it, and Cairn does not manage it. Never adopted, never deleted (FR-150) |

### InstallationScope

| Value | Meaning |
|---|---|
| `project_shared` | Inside the repository, in a file normally committed |
| `project_local` | Inside the repository, in a file the agent treats as developer-local and gitignored |
| `user` | Per-user, outside any repository |

Scope is recorded alongside owner and is never inferred from where a file happens to be
found (FR-220). Concrete locations are in [`contracts/scope-matrix.md`](./contracts/scope-matrix.md).

### ActivationState

`not_applicable` | `pending_user_trust` | `active` | `invalidated`

`pending_user_trust` and `invalidated` occur only for agents whose handlers are trust-gated
(Codex, D24). `invalidated` means the handler was trusted and Cairn's upgrade reset that
trust.

### IntegrationLevel

`full` | `mcp_plus` | `mcp_only` | `unsupported`

Computed, never stored as intent (FR-109). See `CapabilityProfile` below.

### CompatibilityClassification

`verified` | `compatible_unverified` | `unsupported`

Default for any version Cairn has not pinned as broken is `compatible_unverified`
(FR-186).

### HealthCondition

| Value | Meaning |
|---|---|
| `healthy` | Present, owned as recorded, semantically equal to canonical |
| `missing` | Recorded as installed, not found |
| `modified` | Present and owned, but its content differs semantically from canonical |
| `outdated` | Present and owned, but its schema or revision is behind this build |
| `duplicated` | Two Cairn-owned copies of one logical resource exist and no migration is in progress |
| `conflicting_owner` | Present under an owner other than the one recorded, or under two owners, or shadowed by a higher-precedence location (D38) |
| `malformed_config` | The enclosing file could not be parsed; Cairn will not write it |
| `damaged_markers` | The managed block's markers are missing, unbalanced, or mismatched |
| `installed_not_activated` | Installed, but the agent will not run it until the user trusts or enables it |
| `migrating` | A recorded ownership migration is in progress (FR-228) |
| `manager_action_required` | Completing the operation needs a manager step Cairn cannot perform (FR-233) |
| `shared` | Healthy, and the resource this agent is bound to also serves other agents. Reported with the full consumer list (FR-243) |
| `unknown` | Detection could not determine the state |

### CanonicalLifecycleEvent

`session_opened` | `tool_succeeded` | `tool_failed` | `agent_quiesced` |
`context_compacting` | `context_compacted` | `session_closed`

| Field | Type | Notes |
|---|---|---|
| `event` | one of the above | |
| `agent` | `AgentId` | Provenance only, never a memory scope (FR-189) |
| `agent_session_key` | string | The vendor's own session identifier; required for routing (FR-118) |
| `cwd` | string | Resolved to a repository by the daemon |
| `source` | string or null | `session_opened` only: startup / resume / clear / compact / fork |
| `trigger` | string or null | compaction events only: manual / auto |
| `reason` | string or null | `session_closed` only |
| `observation` | `ObservationInput` or null | tool events only — the existing Feature 001 input type |

**Invariants**

- An adapter emits an event only when the vendor actually signalled it (FR-115). There is no
  synthesized `session_closed` and no synthesized `tool_failed`.
- `agent_quiesced` never carries an observation and never implies an outcome (FR-231).
- `context_compacted` never produces a second handoff for the same compaction (FR-119).

**Privacy class**: `derived`. The event is a transport shape; only the `ObservationInput`
inside it is persisted, and only through Feature 001's exclusion → redaction → bound
pipeline (FR-198).

---

## CapabilityProfile (`derived`)

Static per adapter along two dimensions, refined by what Cairn can establish (D19, D19a).

**Availability** — `guaranteed` | `conditional` | `absent` | `pending_activation` (FR-241).
`conditional` means the agent provides it only when a particular payload makes it
determinable; it never counts towards FULL and is always reported with what it depends on.

**Confidence** — `verified` | `expected` (FR-242). `verified` means Cairn established it on
this installation and holds an evidence row for it (see `CapabilityEvidence`). Everything else
is `expected`. Confidence never raises a level; it **withholds** FULL while any FULL-required
capability is only expected (FR-245), and doctor names every expected capability.

| Capability | Claude Code | Codex | OpenCode | Generic MCP |
|---|---|---|---|---|
| `mcp_user_scope` | guaranteed | guaranteed | guaranteed | guaranteed |
| `mcp_project_scope` | guaranteed | guaranteed | guaranteed | absent |
| `instructions_project` | guaranteed | guaranteed | guaranteed | absent |
| `skill_user` | guaranteed | guaranteed | guaranteed | absent |
| `skill_project` | guaranteed | guaranteed | guaranteed | absent |
| `lifecycle_session_open` | guaranteed | guaranteed | guaranteed | absent |
| `lifecycle_tool_success` | guaranteed | guaranteed | guaranteed | absent |
| `lifecycle_tool_failure` | guaranteed | guaranteed | **conditional** | absent |
| `lifecycle_quiesce` | guaranteed | guaranteed | guaranteed | absent |
| `lifecycle_pre_compaction` | guaranteed | guaranteed | conditional | absent |
| `lifecycle_post_compaction` | guaranteed | guaranteed | guaranteed | absent |
| `lifecycle_session_close` | guaranteed | guaranteed | **absent** | absent |
| `context_at_session_open` | guaranteed | guaranteed | guaranteed | absent |
| `stable_session_identifier` | guaranteed | guaranteed | guaranteed | absent |
| `handlers_require_trust` | no | **yes** | no | n/a |

`pending_activation` applies to Codex's lifecycle capabilities until the user trusts the hooks.

Two entries are `conditional` rather than `guaranteed`, and both are load-bearing:

- **OpenCode `lifecycle_tool_failure`** — `tool.execute.after` carries no outcome flag, and a
  tool that throws may not reach the hook at all. The adapter emits `tool_failed` where the
  output unambiguously establishes a failure and emits nothing where it does not (FR-117). It
  is neither present nor absent, and SC-110 tests exactly that: one payload that establishes
  failure produces the event, one ambiguous payload produces nothing.
- **OpenCode `lifecycle_pre_compaction`** — delivered by `experimental.session.compacting`. If
  the installed OpenCode does not expose it, the adapter reports it absent rather than assuming
  it.

### Level derivation

```
established(c) ≡ availability(c) = guaranteed ∧ confidence(c) = verified
                 # conditional never satisfies this; expected never satisfies this

FULL_REQUIRED_CONFIG  = { mcp, instructions, skill (unless the agent has no skills) }
                        evidence kind: introspection
FULL_REQUIRED_RUNTIME = { lifecycle_session_open, lifecycle_tool_success, lifecycle_quiesce,
                          lifecycle_session_close, context_at_session_open,
                          stable_session_identifier }
                        evidence kind: observation

full      ⟸ ∀c ∈ FULL_REQUIRED_CONFIG  ∪ FULL_REQUIRED_RUNTIME : established(c)
            ∧ completion_guarantee = demonstrated
mcp_plus  ⟸ mcp ∧ (instructions ∨ skill ∨ any lifecycle capability of any availability)
mcp_only  ⟸ mcp only
unsupported ⟸ detected but no safe integration
```

`completion_guarantee` is a separate tri-state — `demonstrated` | `not_demonstrated` |
`pending_activation` — and is `demonstrated` only when **all** hold: the adapter has a mechanism
that positively establishes that a session terminated (FR-207); that capability is
`established` per the rule above; and no boundary is currently owed a handoff (FR-240 clause 4).
**Cairn's inactivity timeout and daemon-start reconciliation never set it** (FR-229).

**The hole this closes**: gating only the completion guarantee on confidence allowed a vendor
update that removed tool capture to still produce FULL — static availability stayed
`guaranteed`, the missing capability stayed `expected`, and nothing consulted it. Requiring
every FULL-required capability to be `established` means the version change discards that
capability's observation evidence and FULL is withheld (FR-245, SC-138).

Under D31/D32 this yields: Claude Code `full` once one ordinary session has opened, used a tool,
gone quiet and closed — `mcp_plus` before that, with the awaited behaviors named; Codex the
same, additionally gated on hook trust and SC-128; OpenCode `mcp_plus` permanently under current
vendor behavior; generic MCP `mcp_only`. No level is hardcoded by agent name.

---

## Persisted local entities

All of these live in the existing local SQLite database, added by one additive migration
(`0004_integrations.sql`). **None of them has an outbox entity type, and the outbox
enqueue path is never called for them** (FR-183).

### AgentIntegration

One row per connected agent on this machine. Survives while the agent holds any binding,
including a binding to a manager-owned resource awaiting withdrawal (FR-244).

| Column | Type | Notes |
|---|---|---|
| `agent` | text, PK | `AgentId` |
| `adapter_version` | integer | The adapter build that wrote the current artifacts |
| `detected_version` | text or null | The agent's own version string, as reported |
| `compatibility` | text | `CompatibilityClassification` |
| `level` | text | Last computed `IntegrationLevel` |
| `completion_guarantee` | text | `demonstrated` \| `not_demonstrated` \| `pending_activation` |
| `connected_at` | text | |
| `last_verified_at` | text or null | Set by every doctor run |

**Privacy class**: `local`. **Invariant**: a row exists while the agent holds at least one
binding. Disconnect removes the row only after its last binding is gone — so an agent whose
only remaining resource is manager-owned keeps its record until the withdrawal is verified
(FR-244).

### ManagerIntegration

| Column | Type | Notes |
|---|---|---|
| `manager` | text, PK | `cc-switch` |
| `detected_version` | text or null | |
| `compatibility` | text | |
| `target_apps` | text | JSON array of the manager's application identifiers |
| `connected_at` | text | |
| `last_verified_at` | text or null | |

**Privacy class**: `local`. Holds no path into the manager's own storage and no manager
credential.

### InstalledResource

One **physical** thing Cairn installed: a file, a managed block, or a configuration entry.
Identified by where it is, not by who uses it. This is the record that makes ownership exact
rather than fuzzy (D25).

| Column | Type | Notes |
|---|---|---|
| `id` | uuid, PK | |
| `kind` | text | `ResourceKind` |
| `owner` | text | `ResourceOwner` |
| `scope` | text | `InstallationScope` |
| `location` | text | Absolute path, or `<manager>:<app>` when `owner = manager`. **Machine-local; never serialized into desired state** |
| `content_hash` | text or null | Canonical hash of exactly what Cairn wrote |
| `artifact_schema` | integer or null | `contract_schema` / `skill_schema` where applicable |
| `artifact_revision` | text or null | 12-hex content digest where applicable |
| `activation` | text | `ActivationState` |
| `installed_at` | text | |
| `last_verified_at` | text or null | |

**Invariants**

- Unique on `(kind, location)` — one row per physical resource (FR-146 steady state).
- `owner = manager` implies `content_hash` is null: Cairn did not write it and does not own
  its bytes; verification compares presence and effective configuration instead.
- A row with `owner = external` is never created by Cairn; external resources are reported
  from inspection and never recorded as owned.
- A row with zero bindings is deleted in the same transaction that removes its last binding.

**Privacy class**: `local`. `location` is a machine path and is one of the reasons this table
must never sync (FR-183).

### ResourceBinding

One agent's **dependency** on an installed resource. The reference count that makes shared
resources safe (D28, FR-243).

| Column | Type | Notes |
|---|---|---|
| `agent` | text | FK to `AgentIntegration` |
| `kind` | text | `ResourceKind` |
| `resource_id` | uuid | FK to `InstalledResource` |
| `bound_at` | text | |

**Invariants**

- Unique on `(agent, kind)` — an agent depends on exactly one resource per kind.
- Several bindings may point at one resource. Two do so in practice: the `AGENTS.md` managed
  block serves Codex and OpenCode, and Claude Code's per-user Skill can serve OpenCode, which
  scans `~/.claude/skills` (D32).
- Connect is "ensure this binding exists"; disconnect is "ensure it does not". Both are
  idempotent (FR-157).
- Disconnecting an agent removes its bindings. Each freed resource is removed **only if no
  binding remains** (FR-243). The last consumer's disconnect is what deletes the file, the
  block, or the entry.
- The `AgentIntegration` row survives while any binding for that agent remains — which is how
  a manager-owned resource keeps its ownership record alive after a native disconnect
  (FR-244, D28a).

**Privacy class**: `local`.

**Why two tables rather than a flag**: connect asks "does this resource already exist",
disconnect asks "is anyone else still using it", and doctor asks "who does this serve". A
`satisfied_by` string answered only the third, and only for Skills — and under it,
disconnecting Codex deleted the `AGENTS.md` block that OpenCode was still relying on.

### CapabilityEvidence

What Cairn has established about one capability on this installation. The record behind
`confidence` (FR-242, FR-245, D19a).

| Column | Type | Notes |
|---|---|---|
| `agent` | text | FK to `AgentIntegration` |
| `capability` | text | The capability name |
| `evidence` | text | `introspection` \| `observation` |
| `established_at` | text | |
| `agent_version` | text or null | The detected agent version when it was established |

**Invariants**

- Primary key `(agent, capability)`. A capability with no row is `expected`.
- `introspection` evidence is **version-independent**: it proves a fact about a resource Cairn
  wrote, so a version change re-derives it in place rather than discarding it.
- `observation` evidence is **version-bound**: when detection reports a version different from
  `agent_version`, the row is deleted. What a previous build did is not evidence about this one
  (FR-245).
- Rows are created as a byproduct of work that already happens — writing a resource, or
  receiving a canonical event. Cairn never synthesizes an event or calls an undocumented
  interface to create one.
- Deleted with the agent's last binding.

**Privacy class**: `local`.

### MigrationState

The explicit transition FR-228 requires. Present only while a migration is in flight.

| Column | Type | Notes |
|---|---|---|
| `id` | uuid, PK | |
| `agent` | text | |
| `kind` | text | `ResourceKind` |
| `source_owner` / `source_scope` / `source_location` | text | Where it is now |
| `target_owner` / `target_scope` / `target_location` | text | Where it is going |
| `phase` | text | See transitions below |
| `overlap_permitted` | bool | False when the two locations share one effective slot, so the change is a single atomic replacement (FR-148) |
| `started_at` | text | |
| `last_error` | text or null | Redacted; never carries file content |

**State transitions**

```
planned ──▶ target_installed ──▶ target_verified ──▶ source_removed ──▶ (row deleted)
   │               │                    │
   └───────────────┴────────────────────┴──▶ failed  (source intact or restored)
```

- `planned` → `target_installed`: the target resource now exists. If `overlap_permitted` is
  false this step is the atomic replacement and the next two are immediate.
- `target_verified`: the target was inspected in the agent's real configuration and is
  effective (FR-234). For a manager target this happens only after the user's confirmation
  step.
- `source_removed`: the previous resource is gone; exactly one owner remains.
- `failed` at any point leaves the previously working configuration intact or restored, and
  the row is retained so the developer can resume or reverse it (FR-228).

**Invariants**

- At most one row per `(agent, kind)`.
- While a row exists, doctor reports that resource as `migrating` and never as `duplicated`
  or `conflicting_owner`.
- No phase may exist in which neither source nor target is effective (SC-117).

**Privacy class**: `local`.

### RecoveryArtifact

Metadata for content preserved before a forced repair (D39, FR-222).

| Column | Type | Notes |
|---|---|---|
| `id` | uuid, PK | |
| `agent` | text | |
| `kind` | text | `ResourceKind` |
| `source_path` | text | The file the content came from |
| `artifact_path` | text | `$CAIRN_HOME/recovery/<agent>/<kind>/<ts>-<hash>.txt` |
| `content_hash` | text | Canonical hash of the preserved content |
| `created_at` | text | |

**Invariants**

- The artifact file contains **only** Cairn-owned prior content: the managed block body, the
  canonical serialization of the owned entry, or a whole file Cairn generated in full. Never
  the enclosing configuration file (FR-238).
- Retention: the ten most recent per `(agent, kind)`; older rows and their files are pruned
  when a new one is written.
- The artifact's *content* is never logged and never enters diagnostics; only its path is
  ever printed (FR-239).

**Privacy class**: `local`.

---

## Reports (`derived`, never persisted)

### IntegrationHealthReport

Produced by doctor; shape in [`contracts/integration-health.md`](./contracts/integration-health.md).

Core section (component version alignment, daemon reachability, project registration), one
`AgentHealth` per detected agent — detection, version, compatibility, level, per-capability
coverage present/absent, and one entry per resource with its `HealthCondition`, owner, scope,
and versions — and one `ManagerHealth`. Contains no credential and no content from user
configuration beyond what names the problem (FR-171).

### IntegrationChangePlan

Produced by every mutating operation before it mutates, and by preview alone.

| Field | Type | Notes |
|---|---|---|
| `changes` | list | Each: `action` (`add` \| `update` \| `remove` \| `unchanged` \| `conflict`), agent, resource kind, owner, scope, target path, and a human reason |
| `untouched` | list | Named categories the operation will not modify — unrelated MCP servers, unrelated handlers, unrelated instructions (FR-161) |
| `blocking` | list | Conflicts that stop the operation, each with the manual sequence |

**Invariant**: computing a plan performs zero writes, including no temporary files (SC-118).

### ManagerActionRequired

The structured outcome when a manager step cannot be automated (FR-233). Shape in
[`contracts/cc-switch.md`](./contracts/cc-switch.md).

Fields: `manager`, `resource_kind`, `applications`, `action` (`import` \| `remove`),
`method` (`deep_link` \| `manual_ui`), `uri` (present only for `import`, and only when it
carries no secret), `instructions`, `verify_with`, and `status`
(`awaiting_user` \| `verified` \| `not_performed`).

**Invariant**: the operation that returns this **has not completed**. Cairn never reports
success on the strength of having asked (FR-233).

---

## Additive extensions to Feature 001 entities

Two, both additive and both local.

### Observation — `vendor_tool` (new, nullable)

The raw vendor tool name kept as bounded provenance (FR-122, D36). Normalized to
`[A-Za-z0-9_.-]`, truncated to 64 characters, passed through redaction like every other
field. **Not** part of the outbox payload — Feature 001's session/memory/handoff payloads are
unchanged, and observations never sync at all (FR-055).

### Session — `handoff_pending`, `handoff_attempts`, `handoff_error` (new)

Set inside the seal transaction at session close and cleared when the handoff is written
(D22). `handoff_attempts` counts synthesis attempts; `handoff_error` holds the last redacted
failure reason.

**Progress is guaranteed while the daemon runs** (FR-240):

1. The synthesis task retries on failure with a bounded backoff.
2. The daemon's existing maintenance tick — already used for the idle reaper — sweeps any
   session whose `handoff_pending` has been set for more than a few seconds and synthesizes
   it. No new scheduler.
3. After a bounded number of attempts the session is reported as `handoff synthesis failed`
   with its redacted reason in `cairn status` and in doctor's core section, and retried at a
   slow cadence. A terminal session never sits silently owing a handoff.
4. Daemon-start reconciliation remains the backstop for the process dying between the seal
   and the synthesis — not the only retry path.

**Why these are not a new entity**: they are three facts about one session's own boundary,
read only by the daemon's synthesis and reconciliation paths. A separate table would be a join
for a boolean and a counter.

---

## What syncs

Nothing in this document.

| Entity | Syncs? |
|---|---|
| DesiredIntegrationState and everything under it | No — derived, in memory |
| AgentIntegration, ManagerIntegration | No |
| InstalledResource, ResourceBinding, CapabilityEvidence, MigrationState, RecoveryArtifact | No |
| CanonicalLifecycleEvent | No — a transport shape; the observation it carries follows Feature 001's rules |
| Observation `vendor_tool` | No — observations never sync |
| Session `handoff_pending`, `handoff_attempts`, `handoff_error` | No — not in the session provenance payload |

The server's allow-list (FR-055) is unchanged, and the agent identity it already holds on
session provenance is the only place agent identity appears off this machine (FR-184).

---

## Relationships

```
AgentIntegration 1 ──── * ResourceBinding ──── 1 InstalledResource
        │                                                  ▲
        ├── * CapabilityEvidence   (per capability; observation rows die on version change)
        │                     ┌────────────────────────────┘
        │                     │   several bindings may share one resource
        │                     │   (the AGENTS.md block; the per-user Skill)
        │                     ├── 0..1 MigrationState   (per agent+kind, transient)
        │                     └── 0..* RecoveryArtifact (per agent+kind, capped at 10)
        │
        └── 0..1 CapabilityProfile   (derived, not stored)

ManagerIntegration 1 ──── * InstalledResource  (those whose owner = manager)

CanonicalLifecycleEvent ──▶ Session (by agent_session_key)  ──▶ Observation | Handoff
                                    └── existing Feature 001 entities, unchanged
```

The arrow from the canonical event into Feature 001's entities is the whole point of the
adapter boundary: nothing to the left of it knows a vendor's name, and nothing to the right
of it knows a vendor exists.
