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
| `shared` | Satisfied by another agent's copy of the same resource (D28) |
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

Static per adapter, refined by detection. Every field is a tri-state:
`present` | `absent` | `present_pending_activation`.

| Capability | Claude Code | Codex | OpenCode | Generic MCP |
|---|---|---|---|---|
| `mcp_user_scope` | present | present | present | present |
| `mcp_project_scope` | present | present | present | absent |
| `instructions_project` | present | present | present | absent |
| `skill_user` | present | present | present | absent |
| `skill_project` | present | present | present | absent |
| `lifecycle_session_open` | present | present | present | absent |
| `lifecycle_tool_success` | present | present | present | absent |
| `lifecycle_tool_failure` | present | present | **absent** | absent |
| `lifecycle_quiesce` | present | present | present | absent |
| `lifecycle_pre_compaction` | present | present | present | absent |
| `lifecycle_post_compaction` | present | present | present | absent |
| `lifecycle_session_close` | present | present | **absent** | absent |
| `context_at_session_open` | present | present | present | absent |
| `stable_session_identifier` | present | present | present | absent |
| `handlers_require_trust` | no | **yes** | no | n/a |

`present_pending_activation` applies to Codex's lifecycle capabilities until the user trusts
the hooks.

### Level derivation

```
full      ⟸ mcp ∧ instructions ∧ (skill ∨ agent has no skills)
            ∧ stable_session_identifier ∧ context_at_session_open
            ∧ lifecycle_tool_success ∧ lifecycle_quiesce
            ∧ completion_guarantee = demonstrated
mcp_plus  ⟸ mcp ∧ (instructions ∨ skill ∨ any lifecycle capability)
mcp_only  ⟸ mcp only
unsupported ⟸ detected but no safe integration
```

`completion_guarantee` is a separate tri-state — `demonstrated` | `not_demonstrated` |
`pending_activation` — and is `demonstrated` only when the adapter has a mechanism that
positively establishes that a session terminated (FR-207). **Cairn's inactivity timeout and
daemon-start reconciliation never set it** (FR-229). Under D31/D32 this yields: Claude Code
`full`; Codex `full` once its hooks are trusted and SC-128 passes, `mcp_plus` before that;
OpenCode `mcp_plus`; generic MCP `mcp_only`. No level is hardcoded by agent name — these are
the outputs of the rule above given the profiles above.

---

## Persisted local entities

All of these live in the existing local SQLite database, added by one additive migration
(`0004_integrations.sql`). **None of them has an outbox entity type, and the outbox
enqueue path is never called for them** (FR-183).

### AgentIntegration

One row per connected agent on this machine.

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

**Privacy class**: `local`. **Invariant**: a row exists only while at least one resource for
that agent is Cairn-owned; disconnect removes the row last, after its resources (FR-178).

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

### ResourceState

The authority on what Cairn installed, where, and as what. This is the record that makes
ownership exact rather than fuzzy (D25).

| Column | Type | Notes |
|---|---|---|
| `id` | uuid, PK | |
| `agent` | text | FK to `AgentIntegration` |
| `kind` | text | `ResourceKind` |
| `owner` | text | `ResourceOwner` |
| `scope` | text | `InstallationScope` |
| `location` | text | Absolute path, or the manager's application identifier when `owner = manager`. **Machine-local; never serialized into desired state** |
| `content_hash` | text or null | Canonical hash of exactly what Cairn wrote |
| `artifact_schema` | integer or null | `contract_schema` / `skill_schema` where applicable |
| `artifact_revision` | text or null | 12-hex content digest where applicable |
| `activation` | text | `ActivationState` |
| `satisfied_by` | text or null | Another agent whose copy serves this one (D28) |
| `installed_at` | text | |
| `last_verified_at` | text or null | |

**Invariants**

- Unique on `(agent, kind)` — one row per logical resource per agent (FR-146 steady state).
- `owner = manager` implies `content_hash` is null: Cairn did not write it and does not own
  its bytes; verification compares presence and effective configuration instead.
- `satisfied_by` non-null implies `location` is null and `owner` is the *other* agent's
  owner. Only `kind = skill` may use it.
- A row with `owner = external` is never created by Cairn; external resources are reported
  from inspection and never recorded as owned.

**Privacy class**: `local`. `location` is a machine path and is one of the reasons this table
must never sync (FR-183).

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

### Session — `handoff_pending` (new, default 0)

Set inside the seal transaction at session close and cleared when the handoff is written
(D22). Daemon-start reconciliation synthesizes a handoff for any session with this flag set.

**Why it is not a new entity**: it is one bit about a session's own boundary, read only by
the daemon's reconciliation path. A separate table would be a join for a boolean.

---

## What syncs

Nothing in this document.

| Entity | Syncs? |
|---|---|
| DesiredIntegrationState and everything under it | No — derived, in memory |
| AgentIntegration, ManagerIntegration | No |
| ResourceState, MigrationState, RecoveryArtifact | No |
| CanonicalLifecycleEvent | No — a transport shape; the observation it carries follows Feature 001's rules |
| Observation `vendor_tool` | No — observations never sync |
| Session `handoff_pending` | No — not in the session provenance payload |

The server's allow-list (FR-055) is unchanged, and the agent identity it already holds on
session provenance is the only place agent identity appears off this machine (FR-184).

---

## Relationships

```
AgentIntegration 1 ──── * ResourceState
        │                     │
        │                     └── 0..1 MigrationState   (per agent+kind, transient)
        │                     └── 0..* RecoveryArtifact (per agent+kind, capped at 10)
        │
        └── 0..1 CapabilityProfile   (derived, not stored)

ManagerIntegration 1 ──── * ResourceState  (those whose owner = manager)

CanonicalLifecycleEvent ──▶ Session (by agent_session_key)  ──▶ Observation | Handoff
                                    └── existing Feature 001 entities, unchanged
```

The arrow from the canonical event into Feature 001's entities is the whole point of the
adapter boundary: nothing to the left of it knows a vendor's name, and nothing to the right
of it knows a vendor exists.
