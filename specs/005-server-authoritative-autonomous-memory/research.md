# Feature 005 — Research Inventory

**Baseline**: `origin/main` @ `f76a9fec8a786a76dc7ffa1b0b0daf96aae08b15`
**Date**: 2026-08-29
**Purpose**: Establish what current `main` actually does, so Feature 005's requirements
describe a real delta rather than a remembered one. Every claim below is cited to code on
that commit. Where a motivating assumption turned out to be **stale**, it is marked so and
corrected — a requirement written against a defect that no longer exists is a fake
requirement.

---

## 0. Method

Six independent inventories were taken across capture, local storage, server, memory
lifecycle, web, and specification conventions. Load-bearing findings — the tool-payload
allowlist, the server's forbidden-content policy, the canonical event contract, the
promotion gate, and the requirement-ID ranges — were then re-verified directly against
source before being used. Two subagent claims were found to cite non-existent paths while
reporting correct substance; both are corrected below.

---

## 1. Capture

### 1.1 The canonical lifecycle vocabulary is seven events

`crates/cairn-core/src/lifecycle.rs:25-40` — `SessionOpened`, `ToolSucceeded`,
`ToolFailed`, `AgentQuiesced`, `ContextCompacting`, `ContextCompacted`, `SessionClosed`.
A test asserts the count is exactly seven (`lifecycle.rs:174-182`).

### 1.2 Only two of the seven may carry content — enforced, not conventional

`CanonicalLifecycleEvent::is_well_formed` (`lifecycle.rs:159-169`):

```rust
ToolSucceeded | ToolFailed => self.observation.is_some(),
_                          => self.observation.is_none(),
```

Checked before dispatch at `crates/cairnd/src/integrations.rs:56-59`. So session opening,
closing, quiescence, and both compaction events are **content-free by construction**.
This is the structural form of Problem A.

### 1.3 The tool-payload allowlist is exactly two keys

`crates/cairn-integrate/src/agents/mod.rs:252-319`, `tool_observation`:

```rust
let path    = input.and_then(|v| v.get("file_path")).and_then(|v| v.as_str())...;
let command = input.and_then(|v| v.get("command")).and_then(|v| v.as_str())...;
```

No other `.get()` against vendor input exists in the function. The header comment states
the rule: *"Only allow-listed fields are read; everything else is used for routing and
discarded (FR-198, FR-199, D35)."* Adapters confirm the intent explicitly — Claude Code's
`last_assistant_message` and `tool_calls` are "read for nothing and never persisted"
(`claude_code.rs:226-227`); OpenCode's tool `output` and `metadata` are "never persisted"
(`opencode.rs:206-208`).

### 1.4 `ObservationInput` is the whole capture surface

`crates/cairn-core/src/wire.rs:374-393`: `kind`, `path`, `command`, `exit_code`,
`outcome`, `summary`, `details`, `vendor_tool`. Eight fields, of which `details` is
populated only from Cairn's own derived failure string, never from vendor JSON
(`mod.rs:316`).

### 1.5 `ObservationType` is richer than the classifier that feeds it

`crates/cairn-core/src/domain.rs:115-124` defines eight kinds: `file_read`,
`file_changed`, `command_run`, `test_run`, `error`, `decision`, `discovery`,
`user_instruction`.

`classify_tool` (`crates/cairn-core/src/tools.rs:10-19`) can only ever emit four of them
(`FileRead`, `FileChanged`, `CommandRun`, `Discovery`). `tool_observation` additionally
derives `TestRun` (via `is_test_command` on the captured command string) and `Error` (on
failure). **`Decision` and `UserInstruction` are unreachable from the hook capture path
entirely** — no vendor event produces them, because `UserPromptSubmit` is declined by
every adapter. Two of the eight documented observation kinds have no producer.

### 1.6 Per-adapter event coverage

| Vendor event | Claude Code | Codex | OpenCode |
|---|---|---|---|
| session open | `SessionStart` → `SessionOpened` | `SessionStart` → `SessionOpened` | `session.created` → `SessionOpened` |
| tool success | `PostToolUse` → `ToolSucceeded` | `PostToolUse` (branched) | `tool.execute.after` (branched) |
| tool failure | `PostToolUseFailure` → `ToolFailed` | same event, `classify_failure` | same event, `establishes_failure` |
| quiescence | `Stop` | `Stop` | `session.idle` |
| pre-compaction | `PreCompact` (+`trigger`) | `PreCompact` (+`trigger`) | `experimental.session.compacting` |
| post-compaction | `PostCompact` (+`trigger`) | `PostCompact` (**no `trigger`**) | `session.compacted` |
| session close | `SessionEnd` → `SessionClosed` | `SessionEnd` → `SessionClosed` | **none exists** |
| subagent | `SubagentStop` **declined** | `SubagentStart`/`Stop` **declined** | **no such event** |
| user prompt | `UserPromptSubmit` **declined** | `UserPromptSubmit` **declined** | — |

Sources: `claude_code.rs:183-244`, `codex.rs:245-288`, `opencode.rs:26-34,183-226`.
`generic_mcp.rs:41-43` normalizes nothing at all.

### 1.7 Corrections to stale motivating assumptions

Three assumptions carried into Feature 005's brief do **not** hold on current `main` and
must not become requirements:

- **"Codex file changes are classified as commands"** — *stale*. `apply_patch` maps to
  `FileChanged` (`tools.rs:13`), asserted by test (`tools.rs:107`).
- **"Missing failure semantics"** — *stale*. All three adapters establish failure
  deliberately and refuse to infer it: `classify_failure` (`codex.rs:126-153`),
  `establishes_failure` (`opencode.rs:39-68`), and Claude Code's separate
  `PostToolUseFailure` hook. Each documents that an ambiguous payload yields no
  fabricated failure.
- **"Integration health is config-file matching"** — *partly stale*. `established()`
  (`capability.rs:355-361`) already requires **both** vendor availability **and**
  observed confidence. The real defect is precise: `apply_evidence` (`capability.rs:508-514`)
  raises confidence to verified for **any** evidence without reading its kind, so configuration
  read-back and observed runtime behaviour collapse to one reported confidence even though the
  distinction survives in the stored evidence. A requirement written against the stale framing
  would pass vacuously against today's code.

Two assumptions **do** hold and are load-bearing:

- **Missing Codex file paths** — *confirmed by construction*. Codex passes
  `payload.value("tool_input")` (`codex.rs:268`) into a normalizer that reads only
  `file_path`. Codex's file-editing tool is `apply_patch`, whose input carries a patch,
  not a `file_path` key. The canonical event therefore carries no path for a Codex edit.
  No test asserts otherwise.
- **Weak subagent capture** — *confirmed absent for every vendor* (§1.6).

### 1.8 Vendor provenance is nearly absent, and what exists is dropped

`CanonicalLifecycleEvent` (`lifecycle.rs:95-114`) carries `agent` and
`agent_session_key` but **no raw vendor event name and no vendor event id**.

`vendor_tool` is captured into `ObservationInput` (`mod.rs:317`) and a `vendor_tool`
column was added to `observations` by `crates/cairn-store/migrations/0004_integrations.sql:157`
— but `NewObservation` (`crates/cairn-store/src/repo.rs:635-647`) has no such field and
the INSERT has no such column. **The column exists and nothing on this path writes it.**

### 1.9 Redaction exists and its ordering is fixed

`crates/cairn-core/src/redact.rs` — static regex set (PEM keys, JWTs, `sk-`/`pk-`/`rk-`,
`ghp_`/`github_pat_`, GitLab, Slack `xox*`, AWS `AKIA`/`ASIA`, Google `AIza`, `Bearer`,
credentials in connection URLs) plus a keyed pattern for `secret|token|password|
credential|private_key`-shaped assignments. `crates/cairnd/src/capture.rs:1-9` fixes the
order: **exclusion → redaction → structured extraction → bounding → write**, so nothing
sensitive is persisted even briefly. Self-described as "a mechanism, not a guarantee"
(`redact.rs:5-6`).

### 1.10 The hook/daemon boundary

Vendor JSON never reaches the daemon. The hook process runs the adapter and sends
`Request::CanonicalEvent` over a Unix socket / named pipe as newline-delimited JSON
(`crates/cairn/src/hook.rs:95-99,141-159`; `crates/cairn/src/client.rs:292-320`).
Boundary-class events await a reply; capture-class events are fire-and-forget
(`lifecycle.rs:69-76`, `hook.rs:150-157`).

### 1.11 Post-compaction delivers nothing back

`ContextCompacted` is capture-class, and its daemon arm is a literal no-op:
`CanonicalEvent::ContextCompacted => Ok(serde_json::json!({}))`
(`crates/cairnd/src/integrations.rs:250`). `delivers_context` is true only for
`SessionOpened` (`hook.rs:138`). Restoration happens at the next session open, or via an
explicit `cairn_context(reason=post_compaction)` call.

---

## 2. Local storage (SQLite)

### 2.1 Schema is at v7

`crates/cairn-store/migrations/0001..0007`. Roughly 40 tables.

### 2.2 What is durable knowledge, and what is machine state

**Canonical durable knowledge held locally**: `memories`, `memory_relations`,
`personal_knowledge` (+relations), `team_knowledge` (+relations), `handoffs`,
`reusable_patterns`.

**Local-only by design, no outbox entity type, no server table**: `observations`,
`evidence_facts`, `memory_evidence_facts`, `verification_runs`,
`continuity_checkpoints`, `pattern_applications`, `task_changes`, `criterion_evidence`,
`project_traits`, `writer_identity`, `sync_cursor`, `sync_meta`, `sync_deferred`, and
every `0004_integrations.sql` table (`agent_integrations`, `manager_integrations`,
`installed_resources`, `resource_bindings`, `capability_evidence`, `migration_states`,
`recovery_artifacts`).

The exclusion is structural, not procedural: `0005_project_intelligence.sql:126-129`
states evidence has "no outbox entity type… schema rather than a promise", and
`0004_integrations.sql:3-6` says the same of every integration table.

### 2.3 What is permanently lost if the local database is deleted today

Everything in the local-only list above, plus:

- any `memories` row with `local_only = 1` (`repo.rs:1506`),
- any row whose outbox delivery has not completed (`pending`/`in_flight`/`blocked`),
- **`reusable_patterns` and `pattern_applications` in full** — these are durable
  knowledge with **no synchronization path of any kind**; the outbox entity-type list
  (`0007…sql:398-402`) omits them and `patterns.rs` never enqueues.

This is the concrete gap Feature 005's durability invariant must close. Feature 004's own
Out of Scope already commits pattern synchronization to Feature 005.

### 2.4 Outbox

States: `pending`, `in_flight`, `delivered`, `failed`, `blocked`
(`0005_project_intelligence.sql:465`). Claim is an atomic
`UPDATE … WHERE id IN (SELECT …) RETURNING *`, oldest first, with a 60-second stale-claim
reclaim (`crates/cairn-store/src/outbox.rs:217-260`). `blocked` means the server refused
on capability grounds (`unknown_entity_type`, `unknown_field`, `schema_older`) and is
released when the server catches up (`outbox.rs:621-641`).

Idempotency keys are content-derived:
`digest("{entity_type}:{entity_id}:{operation}:{digest(body)}")` for project rows
(`outbox.rs:73-76`), and the same prefixed with `writer_id` for global rows
(`outbox.rs:116-133`) so two devices proposing identical content stay distinguishable for
reconciliation rather than colliding in transport.

Namespaces: `project:<id>`, `personal:<instance>:<user>`, `team:<instance>`
(`0007…sql:416-420`). Claiming a global row additionally filters on
`authored_by_user_id`, so a proposal authored under one account is never delivered while
another is signed in (`outbox.rs:513-532`).

### 2.5 `writer_identity` / `writer_seq` are provenance only — already

This matters, because it means Feature 005 does not need to remove them. The code says so
directly (`crates/cairn-store/src/global.rs:2679-2685`):

> Nothing in recall, reconciliation or ordering reads this — `MemoryFacts` has no
> `writer_seq` field at all… What it is for is: *did something this writer sent never
> arrive?*

Restated at `repo.rs:1702-1703`: "diagnostic only… nothing here or downstream ever
compares one writer's sequence against another's to decide anything." Consumed solely by
`personal_writer_gaps` / `team_writer_gaps` (`global.rs:2698-2739`).

### 2.6 Server → local merge rules differ per entity

| Entity | Rule | Cite |
|---|---|---|
| `memories` | `INSERT OR IGNORE` — content never overwritten | `repo.rs:897-956` |
| `memory_relations` | `INSERT OR IGNORE` on PK `(from,to,kind)` | `knowledge.rs:78-95` |
| `tasks`, `task_criteria`, `task_blockers` | `ON CONFLICT DO UPDATE` — server wins | `criteria.rs:1347-1495` |
| `personal_knowledge` | insert-once; only lifecycle columns advance, content never rewritten | `global.rs:2586-2671` |
| `team_knowledge` | insert-once; state only **advances** `proposed→authoritative→retired`, guarded so out-of-order delivery cannot walk it backwards | `global.rs:2418-2529` |

There is no `import_project` or `import_session`; projects and sessions are push-only.

### 2.7 Three independent FTS corpora

`memory_fts`, `personal_fts`, `team_fts` — FTS5 external-content over each table's
`content` column (`0002`, `0007…sql:122-145,240-263`). Ranked independently by BM25, and
the migration states why no cross-domain comparator exists: *"BM25 scores from different
corpora are not comparable (D425)"*.

---

## 3. Server (PostgreSQL)

### 3.1 Schema is at v3; 21 tables

`crates/cairn-server/migrations/0001..0003`. Canonical server-owned: `users`,
`web_sessions`, `api_tokens`, `project_members`, `sync_state`, `server_instance`, and the
lifecycle columns of `team_knowledge`. Everything else mirrors client-authored rows.

### 3.2 The forbidden-content policy — verified verbatim

`crates/cairn-server/src/sync.rs:27-78`. Refused entity types:

```
observation, observation_ref, evidence_fact, verification_run,
continuity_checkpoint, reusable_pattern, pattern_application,
task_change, criterion_evidence
```

Refused field names, **recursive at any depth** (`contains_key_recursive`,
`sync.rs:387-395`):

```
summary, path, command, details, exit_code, observations, observed_value,
source_locator, value_digest, fingerprint, relevant_paths, criteria_snapshot,
sanitization_report, origin_ref, alternative_cause, signal_digest, pin_reason,
rationale, basis_evidence_id, path_fingerprints, task_snapshot_at_bind, detail,
prior_value, new_value, content_norm_digest, local_revision
```

Plus `outcome` at top level only (`sync.rs:258`), deliberately non-recursive to avoid
colliding with a legitimate `TestRunRecord.outcome`. Session-local fields
`worktree_path`, `agent_session_key`, `daemon_run_id`, `last_event_at`,
`last_turn_ended_at` are stripped (`sync.rs:81-87`). Enforcement runs first in
`apply_item`, before the idempotency claim (`sync.rs:162`).

The comment states the intent: *"This list **is** the privacy boundary, stated once."*

**Consequence for Feature 005**: `/api/sync/batch` cannot carry a semantic event. `path`,
`command`, `summary`, `exit_code` and `details` are refused at any depth, and even
`path_fingerprints` is refused. A separate, separately-governed ingest boundary is not a
stylistic preference — it is the only way to add event ingest without dismantling the one
place the existing privacy boundary is stated.

**Correction — what the boundary actually refuses.** An earlier reading of this list, that
repository-relative paths cannot reach the server at all, is wrong, and the error mattered
enough to state plainly. `handoff` is a synchronized entity type, and the server stores
`changed_files` — a list of repository-relative paths — binding it on ingest and serving it to
the web client (`crates/cairn-server/migrations/0001_init.sql:127`,
`crates/cairn-server/src/sync.rs:754,772`, `crates/cairn-server/src/api.rs:1212`).
`changed_files` is on neither refusal list. What the boundary refuses is a set of **field
names**, not path data as a category. The open question in the specification is therefore
narrower than it first appeared: not *whether* repository-relative file identity may cross, but
under what explicitly defined field name it may, given that reusing a refused name would make
two boundaries on one server disagree about the same name.

**Consequence for reusable patterns.** `reusable_pattern` is on the refused entity-type list,
and a pattern row additionally carries `signal_digest`, `origin_ref` and `sanitization_report`
(`crates/cairn-store/migrations/0005_project_intelligence.sql:239-262`), all three refused
recursively at any depth. Making patterns durable server-side — Feature 004's deferral to this
feature — is therefore not a matter of routing them somewhere; it requires an explicitly named
exception and a redefined pattern representation.

### 3.3 Idempotency

`sync_state.idempotency_key` is the primary key; applied via
`INSERT … ON CONFLICT DO NOTHING` inside the item's own transaction
(`sync.rs:176-191`). `rows_affected() == 0` ⇒ `duplicate`. A later validation failure
rolls back the claim with it, so the item stays retryable (`sync.rs:206-213`).

### 3.4 Authorization is sound and must not be reopened

Identity for authorization **never** comes from the request body. `owner_user_id` and
`proposed_by_user_id` are bound from the authenticated extractor (`global.rs:307-312`);
project routes go through `auth::require_member` (`auth.rs:538-550`); ratify and retire
require `AdminUser` plus a compare-and-swap on the current state
(`global.rs:1063-1150`); tombstones are scoped `WHERE id=$1 AND project_id=$2`
(`sync.rs:802-841`); the last-admin guarantee uses an advisory transaction lock
(`api.rs:527-561`). Server instance identity is compared exactly, with no fallback to
endpoint match, closing a same-URL-different-server attack
(`crates/cairnd/src/sync.rs:1007-1019`).

Feature 004 hardened this over six review rounds. Feature 005 inherits it and must not
introduce a path that re-derives identity from a payload.

### 3.5 What the server does not have

- **No background job, scheduler, worker or timer of any kind.** The server is a single
  request-serving axum process (`main.rs:266-274`); the only refreshed state is a lazy
  in-memory release cache. Consolidation has no execution home today.
- **No event stream, no telemetry table.**
- **No request body limit, no batch-size cap, no rate limiting** — `SyncBatchBody.items`
  is unbounded (`sync.rs:98-102`). The only enforced size is the read-back page size
  `PAGE = 500`.
- **Full-text search on `memories.content` only** — a GIN index over
  `to_tsvector('english', content)` (`0001_init.sql:113-114`), used by the web memory
  route (`api.rs:1259-1268`). Neither `personal_knowledge` nor `team_knowledge` is
  indexed server-side.

---

## 4. Memory creation, reconciliation, retrieval

### 4.1 No automatic observation → memory pipeline exists

Every durable-memory creation path traces to an explicit act: `cairn_remember`
(create / supersede / promote), `cairn memory add`, `cairn team propose`,
`cairn pattern promote`, or a sync pull replaying another writer's already-explicit
creation. Confirmed by exhaustive search of `crates/cairnd/src/handlers.rs` and
`crates/cairn-store/src/repo.rs`.

Two derived pipelines exist and neither produces memory:

- **Handoffs** (`crates/cairnd/src/handoffs.rs:12-121`) — every field derived from
  observations, pre-existing decision memories and git state; an agent narrative may only
  be attached afterwards through `annotate`. Writes a `handoffs` row.
- **Continuity checkpoints** (`crates/cairnd/src/continuity.rs:122`) — structured
  snapshot, explicitly "not a summary of conversation", local, never synced.

Drift marking is deliberately constrained further: its module doc enumerates what it must
never do — *"rewrite the memory / create a superseding memory / mark the memory stale or
superseded"* (`crates/cairnd/src/drift.rs:16-21`).

### 4.2 Relations, and the automation rule Feature 005 collides with

`RelationKind` (`domain.rs:392-417`): `Reinforces`, `Duplicates`, `Supersedes`,
`ConflictsWith`, `Narrows`, `NotApplicableTo`.

`RelationBasis` (`0005_project_intelligence.sql:112-113`): `deterministic_rule`,
`evidence`, `explicit_agent`, `explicit_user`.

**`is_automatic()` (`domain.rs:433-435`) permits only `Duplicates` and `ConflictsWith` to
be recorded without an explicit instruction. `Reinforces` and `Supersedes` are documented
as never automatic** (`domain.rs:397-398,407`).

This is a direct, live constraint on "automatic consolidation" and is resolved explicitly
in the specification rather than waived.

### 4.3 Duplicate detection is exact, by construction

`classify_proposal` (`crates/cairn-core/src/knowledge.rs:356-360`) compares
`content_norm_digest` — SHA-256 over NFC → lowercase → whitespace-collapse → stripped
trailing punctuation (`knowledge.rs:121-138`). The documentation is emphatic:
*"Two digests are equal or they are unrelated; there is no third answer"*
(`knowledge.rs:117-121`). BM25 is used for search ranking only, never for dedup.

**Consequence**: model-authored candidates will essentially never collide on this digest.
Consolidation cannot rely on existing dedup and needs its own deterministic identity.

### 4.4 Conflicts are detected automatically and never auto-resolved

Same `topic_key`, overlapping scope, differing `value_key` ⇒ `ConflictsWith` with basis
`deterministic_rule` (`knowledge.rs:396-425`), endpoints normalized `(min,max)` so two
offline machines converge on one row. `memory_reconcile` refuses to act on a conflicted
pair (`handlers.rs:2298-2303`, code `NOT_CONFLICTED`).

### 4.5 Reinforcement is recomputed, never incremented

`rebuild_reinforcement` (`crates/cairn-store/src/knowledge.rs:359-398`) recomputes
`reinforcement_count` and `distinct_origin_count` from the relation graph on every
change. This is already replay-safe.

### 4.6 Verification

`VerificationState`: `Unverified`, `Verified`, `NeedsRecheck`, `Drifted`, `Conflicted`
(`domain.rs:290-309`). `VerificationAuthority`: `Cairn`, `Attested`, `RemoteCairn`,
`RemoteAttested` (`domain.rs:319-360`). A run appends to `verification_runs`
(`evidence.rs:402`) and the memory's cached state is **derived** from run history plus
any attached `contradicts` evidence (`evidence.rs:508-747`) — never asserted
independently. Exit from `Conflicted` always lands on `NeedsRecheck`, never directly on
`Verified`.

Evidence is local-only structurally: `EvidenceKind` and `VerifierKind` have no
`OutboxEntityType` variant at all (`domain.rs:216-238`).

### 4.7 The promotion gate is the model for deterministic governance

`crates/cairn-core/src/promotion.rs` — eight checks, fixed order, fail-closed, and a
**pure function**: no database handle, no clock, no network, every input by value. Its
rejection type has no `String` field at all, so a caller "cannot log the rejected content
through it even carelessly". `validate_global_content` runs at all five entry points so
there is exactly one implementation of every rejection class (FR-579).

Feature 005 reuses this shape rather than inventing a second gate.

### 4.8 Retrieval

**Automatic**: one path only — `SessionOpened` builds a briefing and pushes it onto the
agent's context surface (`hook.rs:112-181,191,259`).
**Manual**: `cairn_context`, `cairn_search`, subject reads, and
`cairn_context(reason=post_compaction)`.

Budget is structural, not statistical. `crates/cairn-core/src/context.rs:140-151` and
`crates/cairn-core/src/budget.rs:16-28`:

| Constant | Value |
|---|---|
| `CHARS_PER_TOKEN` | 3.5 |
| `GLOBAL_SHARE_MAX` (personal + team ceiling) | 0.15 |
| `reserve_fraction` (Level-0 reserve) | 0.40 |
| `goal_max_tokens` | 60 |
| `warnings_in_context_max` | 5 |
| `pins_in_context_max` | 4 |
| `MEMORY_PER_SCOPE` / `GLOBAL_PER_BRIEFING` | 12 / 12 |

Selection is section-ordered, not score-ordered: `SECTION_ORDER` (`context.rs:40-56`) is
task, repository, previous_handoff, known_failures, decisions, task_memory,
branch_memory, project_memory, patterns, personal_notes, team_guidance — admitted in that
order until the budget binds. Scope precedence is `Task=0, Branch=1, Project=2,
Session=3` (`domain.rs:150-158`), applied as `ORDER BY scope_bucket ASC, relevance DESC,
created_at DESC` (`crates/cairn-store/src/search.rs:118`). Personal is recency-ordered
and team is authoritative-only, explicitly "not a search" (`briefing.rs:170-186`).

### 4.9 Retrieval selection is not persisted

**Not present.** No table or column records which memories were considered or selected for
any session or context call. The `explain` flag returns diagnostics in the response only
and is never written.

### 4.10 Delivery telemetry exists, but is a capability bit, not a log

`deliver_context` distinguishes a genuinely delivered briefing (possibly empty or
degraded) from an absent one, and reports
`capability: "context_at_session_open", evidence: "observation", degraded: <bool>`
(`hook.rs:226-241`). This lands in `capability_evidence`, whose primary key is
`(agent, capability)` (`0004_integrations.sql:96-106`) — **one row per agent per
capability, overwritten**. It proves the channel carried a payload once. It is not a
per-retrieval record and cannot answer "what was delivered to this session".

---

## 5. Web

### 5.1 Ten screens

`/`, `/projects/[id]`, `/tasks`, `/sessions`, `/sessions/[sessionId]`, `/memory`,
`/sync`, `/tokens`, `/login`, plus a 404. Next.js 15 App Router, React 19, Tailwind v4,
all data fetched client-side through `@tanstack/react-query`; every page is an
authenticated client component. Browser auth is the `cairn_session` HttpOnly cookie.

### 5.2 The memory view is thin

Displayed today: content, type badge, scope badge, non-active state, truncated origin
session id, and an `evidence_count` number beside a static note that evidence content is
local. **Not displayed at all**: relations, verification state, reinforcement counts,
retrieval usage. `superseded_by_id` is carried in the client type but never rendered.

### 5.3 Whole subsystems have no web surface

Memory relations; verification; reinforcement counts; personal knowledge; team knowledge
and the entire propose/ratify/retire lifecycle; admin user management; project creation
and membership; integration health — the last having no HTTP route at all, so no screen
could exist. There is no activity feed, no retrieval trace, no system-health view, and no
relation graph.

Feature 004's Out of Scope already commits web administration and team-curation screens
to Feature 005.

### 5.4 The precise health weakness

Health is a conjunction of vendor availability and observed confidence
(`capability.rs:355-361,827-891`), which is stronger than "config file matched". The
actual weakness is that **`EvidenceKind::Introspection` (a configuration file read back
and found to match) and `EvidenceKind::Observation` (runtime behaviour actually captured)
both raise `Confidence` to `Verified` identically** (`capability.rs:508-514`). The
distinction survives in the stored evidence kind but is erased in the derived status a
user reads.

---

## 6. Feature 004 machinery — disposition input

| Mechanism | Exists because | Disposition |
|---|---|---|
| `writer_identity` | provenance + delivery-gap detection | **KEEP** — already never used for conflict resolution (§2.5) |
| `writer_seq` | per-writer gap detection | **KEEP** — same |
| `sync_cursor` | per-namespace pull position | **KEEP** — still needed to pull canonical knowledge into cache |
| `sync_meta` (0001) | legacy project cursor, superseded by `sync_cursor` but still has callers | **REMOVE** after migration |
| `visibility_context` | team feed differs per caller's authorization | **KEEP** — orthogonal to authority |
| outbox + namespaces | transactional send queue | **REPURPOSE** — becomes the event spool and residual knowledge drain |
| per-namespace backoff | politeness under server failure | **KEEP** |
| server instance binding | closes same-URL-different-server attack | **KEEP** — security-critical |
| local personal/team replicas | offline authority | **REPURPOSE** — bounded non-authoritative cache |
| server→local merge (lifecycle-advance-only) | both sides could author | **SIMPLIFY** after migration completes; **required during** it |
| offline multi-writer convergence | authority was duplicated | **REMOVE** — the reason ceases to exist |
| `sync_deferred` | pulled child arrived before parent | **KEEP** |

---

## 7. Specification conventions

Requirement IDs use `FR-` and `SC-` only. Occupied ranges: 001 uses FR-001–064 /
SC-001–012; 002 uses FR-001–245 / SC-101–138; 003 uses FR-301–519 / SC-301–331; 004 uses
FR-401–608 / SC-401–470. The highest identifier appearing anywhere under `specs/` is
FR-610 and SC-470; decision ids reach D458.

**003 and 004 overlap in the FR-401–FR-519 band** — a pre-existing collision in the
corpus. Feature 005 therefore does not follow the "leading digit equals feature number"
habit, which would collide again. It uses **FR-701+ and SC-701+**, the nearest clean band,
and records the reason so a later reader does not mistake it for a numbering error.

`## Clarifications` and `## Out of Scope` are house conventions beyond the template.
`## Constitution Check` appears only in `plan.md`, never in `spec.md`.
`.specify/scripts/bash/create-new-feature.sh` creates no git branch; it derives the
number by scanning `specs/`, copies the spec template, and rewrites `.specify/feature.json`.

---

## 8. Open questions carried into the specification

1. Where consolidation executes — the server has no scheduler of any kind today (§3.5).
2. What model, if any, performs semantic extraction, and where it runs.
3. Whether a repository-relative path may cross the new event boundary, given that
   `path` and even `path_fingerprints` are refused on the existing one (§3.2).

These are recorded as `[NEEDS CLARIFICATION]` in `spec.md` rather than silently defaulted.
