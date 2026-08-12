---

description: "Task list for the Agent Integration Platform (002-agent-integration-platform)"
---

# Tasks: Agent Integration Platform

**Input**: Design documents from `/specs/002-agent-integration-platform/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/](./contracts/), [quickstart.md](./quickstart.md)

**Tests**: Included, and not optional here. The constitution requires user-observable behavior
to be verified, [quickstart.md](./quickstart.md) defines the acceptance walkthrough, and
FR-203–FR-205 require fixture-based adapter tests that run hermetically. Several of this
feature's requirements are *negative* — no adapter maps idle to close, no capability the
profile denies produces an event, the idle reaper never yields FULL — and a negative
requirement that is not asserted by test is not implemented at all.

## Format: `[ID] [P?] [MANUAL?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[MANUAL]**: Requires a live, authenticated agent or manager. Release evidence only —
  never a required CI check (FR-205, D40 tier 5)
- **[Story]**: Which user story the task belongs to (US1–US10)
- Setup, Foundational, and Polish tasks carry no story label, and neither do the
  cross-cutting evidence tasks in Phase 11 (performance, privacy, compatibility, CI), which
  assert properties spanning every story rather than serving one

## Path Conventions

Rust workspace at repository root: `crates/{cairn-core,cairn-git,cairn-store,cairn-integrate,cairnd,cairn,cairn-server}`.
Canonical Skill source at `skills/cairn/`. Workspace end-to-end tests in the `cairn-e2e`
package at `tests/` (test files in `tests/tests/`). Recorded vendor payload fixtures in
`tests/integrations/`. Web UI in `web/`. Workflows in `.github/workflows/`.

## Phase map

The plan's ten delivery phases (plan.md §Phasing) map onto the phases below. Each ends
runnable; the feature is the whole table.

| Plan phase | Phases here | Stories |
|---|---|---|
| **A. Domain** | 1–2 | — (foundational) |
| **B. Engine** | 3 | — (foundational) |
| **C. Claude Code** | 4 | US1, US2 |
| **D. Sealed close** | 5 | US3 |
| **E. Codex** | 6 | US3 |
| **F. OpenCode** | 7 | US4 |
| **G. Repair & migrate** | 8 | US7, US9 |
| **H. CC Switch** | 9 | US5 |
| **I. Generic MCP & instructions** | 10 | US8 |
| **J. Evidence** | 11 | US6, US10 |
| — | 12 | Polish |

---

## Phase 1: Setup

**Purpose**: The new crate, the two canonical asset sources, and the fixture directories —
nothing wired to anything yet.

- [x] T001 Add `crates/cairn-integrate` to the workspace: add the member to the root `Cargo.toml` and create `crates/cairn-integrate/Cargo.toml` depending on `cairn-core`, `serde`, `sha2`, `jsonc-parser` with its `cst` feature, `toml_edit`, and `include_dir`, with **no** `sqlx` and no required async runtime so the fixture corpus is testable without a daemon, a socket, or a Git repository, and declare `[[bin]] name = "skillref"` (plan.md §Structure Decision, §Complexity Tracking, D18)
- [x] T002 [P] Create the canonical Skill source tree at `skills/cairn/SKILL.md` and `skills/cairn/references/{resuming-work,searching-first,recording-knowledge,choosing-scope,sessions-and-tasks,diagnosing-cairn}.md`, with `SKILL.md` frontmatter carrying only `name`, `description`, `metadata.cairn_skill_schema: 1`, and `metadata.cairn_skill_revision`; the entry document stays short and reaches depth through the reference files (FR-140, FR-141, FR-142, `contracts/agent-contract.md` §The Cairn Skill, D29)
- [x] T003 [P] Create the single canonical usage-contract source at `crates/cairn-integrate/assets/agent-contract.md` carrying the nine behavioral rules and nothing else — no documentation, no workflow guidance, no tool reference (FR-123, FR-124, FR-127, `contracts/agent-contract.md` §The always-on contract)
- [x] T004 [P] Create the configuration fixture corpus directory `crates/cairn-integrate/tests/fixtures/` with a `README.md` stating the rule every fixture follows: each file declares its expected ownership outcome, and the corpus spans tab and four-space indentation, CRLF, minified single-line objects, unusual key order, unicode escapes, and comment-bearing TOML and JSONC (D37, D40 tier 2)
- [x] T005 [P] Create the recorded vendor payload directories `tests/integrations/{claude-code,codex,opencode,cc-switch,generic-mcp}/` with a `README.md` naming, per payload, the official vendor source and date it was recorded from, so a vendor change is visible in a diff (FR-203, D40 tier 3, plan.md risk table)

**Checkpoint**: `cargo build --workspace` succeeds on a clean checkout with the seventh crate present.

---

## Phase 2: Foundational — Domain (plan Phase A)

**Purpose**: The canonical vocabulary, the capability model, the desired-state model,
ownership markers, the Skill revision algorithm, both contract renderings, and the scope
matrix. Pure logic, no I/O.

**⚠️ CRITICAL**: No user story work can begin until Phases 2 and 3 are complete.

- [x] T006 [P] Define `CanonicalLifecycleEvent` in `crates/cairn-core/src/domain.rs` — exactly the seven events `session_opened`, `tool_succeeded`, `tool_failed`, `agent_quiesced`, `context_compacting`, `context_compacted`, `session_closed`, each carrying `agent`, `agent_session_key`, `cwd`, and the optional `source`/`trigger`/`reason`/`observation` fields, with the Feature 001 meanings preserved exactly and no eighth event (FR-112, FR-113, FR-114, FR-118, FR-119, FR-230, `contracts/lifecycle.md` §The events, `data-model.md` §CanonicalLifecycleEvent)
- [x] T007 [P] Add `vendor_tool` to `ObservationInput` in `crates/cairn-core/src/domain.rs`, normalized to `[A-Za-z0-9_.-]` and truncated to 64 characters, and extend Feature 001's `classify_tool`/`is_test_command` with the Codex and OpenCode vendor names so canonical categories stay the only thing memory, handoff, and context consult (FR-120, FR-121, FR-122, D36, `contracts/lifecycle.md` §Tool normalization)
- [x] T008 [P] Define the two integration shapes in `crates/cairn-integrate/src/adapter.rs` — `AgentAdapter` with `id`/`detect`/`capabilities`/`plan`/`inspect`/`normalize`, where `normalize` returning `None` is the normal way an adapter declines an event, and a separate `IntegrationManager` with `detect`/`inspect_bindings`/`import_uri`/`verify` and no lifecycle, no instructions, and no removal (FR-101, FR-102, FR-115, plan.md §Adapter shape, D19)
- [x] T009 [P] Implement the capability model in `crates/cairn-integrate/src/capability.rs` — the capability set of FR-107, availability (`guaranteed`/`conditional`/`absent`/`pending_activation`) × confidence (`verified`/`expected`), the four static per-adapter profiles as data, `established(c)`, the `completion_guarantee` tri-state, and level derivation to FULL/MCP_PLUS/MCP_ONLY/UNSUPPORTED with FULL withheld while any FULL-required capability is merely expected (FR-107, FR-108, FR-109, FR-207, FR-241, FR-242, FR-245, `data-model.md` §CapabilityProfile, §Level derivation, D19, D19a)
- [x] T010 Unit tests for level derivation in `crates/cairn-integrate/src/capability.rs` — a `conditional` capability never counts towards FULL; an `expected` FULL-required capability withholds FULL and never yields `unsupported`; the inactivity timeout and daemon-start reconciliation contribute nothing to `completion_guarantee`; a capability whose availability is `absent` names the specific behavior the developer will not get rather than a score (FR-110, FR-111, FR-229, SC-127, SC-131)
- [x] T011 [P] Implement `DesiredIntegrationState`, `DesiredAgent`, `DesiredResource`, and `DesiredManager` in `crates/cairn-integrate/src/desired.rs` with fixed key order, lists sorted by a stable key, no field capable of holding a credential, and no absolute path — locations are named by scope and resolved from the matrix at apply time (FR-201, FR-202, FR-226, `data-model.md` §Desired-state entities, `contracts/integration-health.md` §Desired state, D26)
- [x] T012 Tests `desired::determinism` and `desired::single_consumer` in `crates/cairn-integrate/src/desired.rs` — identical inputs serialize byte-identically across runs; serialization from a configuration seeded with recognizable secrets contains none of them; exactly one entry per `(agent, kind)`; and no Feature 002 code path derives intent from anywhere but this model (FR-227, SC-135)
- [x] T013 [P] Implement ownership markers in `crates/cairn-integrate/src/markers.rs` — locate by the full literal prefix `<!-- cairn:managed:begin id=` and never by searching for the word `cairn`, validate exactly one balanced begin/end pair with matching `id`, splice only the bytes between the markers, and compare by canonical digest of the normalized body so reflow is not an edit (FR-133, FR-134, FR-135, FR-136, FR-138, FR-139, FR-223, `contracts/agent-contract.md` §The managed instruction block, D25)
- [x] T014 Tests for `crates/cairn-integrate/src/markers.rs` — missing, unbalanced, duplicated, and mismatched-`id` markers all yield `damaged_markers` and change nothing; a formatting-only difference is `healthy`; a semantic edit is `modified`; removal leaves surrounding content and the file itself in place (FR-137, SC-130)
- [x] T015 [P] **Skill revision task A** — implement the one canonical `skill_revision` algorithm in `crates/cairn-integrate/src/revision.rs`: enumerate every file under `skills/cairn/`, sort by relative path as raw bytes, normalize CRLF → LF with exactly one trailing newline, replace the *value* of the parsed frontmatter field `metadata.cairn_skill_revision` with the literal `<REVISION>` while hashing `cairn_skill_schema` normally, feed each entry as `path` + `0x00` + eight big-endian length bytes + content, and take the first 12 hex characters of the SHA-256 (FR-141, D29b)
- [x] T016 **Skill revision task B** — self-validation tests in `crates/cairn-integrate/src/revision.rs`: the checked-in `metadata.cairn_skill_revision` in `skills/cairn/SKILL.md` equals the computed value so an ordinary `cargo test` catches a Skill edit that forgot the field; the self-field is normalized before hashing and a body mention of the field name is untouched; length-prefix framing makes `a/b`+`c` and `a`+`/bc` distinct; a CRLF copy of the tree hashes identically (D29b)
- [x] T017 **Skill revision task C** — implement `crates/cairn-integrate/src/bin/skillref.rs` printing `{"skill_schema","skill_revision","skill_branch"}` as JSON, a thin wrapper over T015's function so CI never reimplements the digest in shell or YAML (D29b, D29a)
- [x] T018 [P] Implement both contract renderings in `crates/cairn-integrate/src/render.rs` from the single `assets/agent-contract.md` source — the managed-block body and the compact MCP `instructions` string — plus `contract_schema` and the `contract_revision` 12-hex digest of the normalized rendered body (FR-123, FR-126, `contracts/agent-contract.md` §The always-on contract, D26)
- [x] T019 Tests `render::contract_within_bound` and `render::renderings_agree` in `crates/cairn-integrate/src/render.rs` — the rendered always-on body is ≤1,200 characters, asserted automatically rather than assumed, and both renderings state the same numbered rules because they come from one function over one source (FR-125, SC-105)
- [x] T020 [P] Implement the scope matrix as data in `crates/cairn-integrate/src/scope.rs` — per agent × resource kind: official scopes, Cairn default, location, whether the location is committed, whether the resource is manager-ownable, and the agent's own precedence rule, with a `--shared` variant per row (FR-210, FR-211, FR-212, FR-213, FR-214, FR-215, FR-219, `contracts/scope-matrix.md`, D27)
- [x] T021 Tests `scope::matrix_rows` and `fixtures::scope_defaults` in `crates/cairn-integrate/src/scope.rs` — every row of `contracts/scope-matrix.md` has an assertion; default scopes produce zero committed-file changes for lifecycle handlers; `--shared` produces exactly the committed changes it described and no others; and where an agent offers no developer-local location the matrix requires explicit agreement rather than a silent committed fallback (FR-218, FR-220, SC-126)

**Checkpoint**: `cargo test -p cairn-integrate` passes. The contract size bound, the rendering
parity, and the checked-in Skill revision are all asserted.

---

## Phase 3: Foundational — Engine (plan Phase B)

**Purpose**: The change-plan engine, the three source-preserving editors, atomic apply and
verify, the local record migration, and the daemon handlers behind it.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [x] T022 [P] Implement JSON/JSONC editing in `crates/cairn-integrate/src/edit/json.rs` on the `jsonc-parser` CST, mutating only the owned node so indentation, escaping, key order, comments, and layout outside it survive byte-exactly; malformed input fails closed and writes nothing (FR-152, FR-153, D37)
- [x] T023 [P] Implement TOML editing in `crates/cairn-integrate/src/edit/toml.rs` on `toml_edit`, preserving comments, ordering, and formatting outside the owned table; malformed or unexpected input fails closed and writes nothing (FR-152, FR-153, FR-137, D37)
- [x] T024 [P] Implement Markdown managed-block splicing in `crates/cairn-integrate/src/edit/markdown.rs` over T013's markers — the only bytes that change are between the markers, and a file is never replaced wholesale (FR-133, FR-153)
- [x] T025 Build the ≥20-file preservation corpus in `crates/cairn-integrate/tests/fixtures/` and the `fixtures::preservation` test that runs connect → disconnect over every file and asserts 100% of non-Cairn bytes are byte-identical to the original, across all three formats and every formatting dimension a re-serializer would destroy (FR-152, SC-104, D37, D40 tier 2)
- [x] T026 Implement the change-plan engine in `crates/cairn-integrate/src/plan.rs` — one engine shared by connect, repair, migrate, and disconnect, classifying each change `add`/`update`/`remove`/`unchanged`/`conflict` with resource, owner, scope and target file named, plus the mandatory `untouched` blast-radius list and a `blocking` list carrying each conflict's manual sequence (FR-151, FR-158, FR-159, FR-160, FR-161, `contracts/integration-health.md` §Change plan)
- [x] T027 Implement apply and verify in `crates/cairn-integrate/src/apply.rs` — per-file atomic replacement so an interrupted write leaves the prior state intact, re-inspection after writing, and `partial_apply` on multi-file partial failure naming exactly which changes landed and which did not; no whole-file copy of a pre-existing configuration is ever made as normal behavior (FR-154, FR-155, FR-156, FR-196, FR-238, `contracts/integration-health.md` §Apply results)
- [x] T028 Test `dry_run_is_inert` in `crates/cairn-integrate/tests/dry_run_is_inert.rs` — computing a plan for every supported operation performs zero filesystem modifications including no temporary files, verified by checksumming every candidate file before and after, and a dry run against a broken configuration reports the conflict and still writes nothing (FR-159, SC-118)
- [ ] T029 Add the additive local-record migration `crates/cairn-store/migrations/0004_integrations.sql` and register it in `crates/cairn-store/src/migrate.rs` — `agent_integrations`, `manager_integrations`, `installed_resources` (unique on `(kind, location)`), `resource_bindings` (unique on `(agent, kind)`), `capability_evidence` (PK `(agent, capability)`), `migration_states` (at most one per `(agent, kind)`), `recovery_artifacts`, plus `sessions.handoff_pending`/`handoff_attempts`/`handoff_error` and `observations.vendor_tool` (FR-182, `data-model.md` §Persisted local entities, §Additive extensions)
- [ ] T030 Implement the local-record repositories in `crates/cairn-store/src/integrations.rs` — reference-counted `InstalledResource` + `ResourceBinding` where connect is "ensure this binding exists", disconnect is "ensure it does not", and a resource row is deleted in the same transaction that removes its last binding; owner is exactly one of `direct`/`manager`/`external`, `external` is never recorded as owned, and `owner = manager` rows carry no content hash (FR-145, FR-146, FR-147, FR-150, FR-220, FR-243, `data-model.md` §InstalledResource, §ResourceBinding, D28)
- [ ] T031 Add the integration wire requests to `crates/cairn-core/src/wire.rs` and their handlers to `crates/cairnd/src/handlers.rs` — record read/write, canonical-event ingestion, and evidence recording, all local; **no outbox entity type is added and the outbox enqueue path is never called for any of them** (FR-182, FR-183, FR-184, `data-model.md` §What syncs)
- [ ] T032 [P] Add Feature 002's error codes to the CLI's existing `codes` module in `crates/cairn/src/` as a closed set — `agent_not_detected`, `agent_unsupported`, `malformed_config`, `permission_denied`, `damaged_markers`, `resource_modified`, `duplicate_resource`, `conflicting_owner`, `installed_not_activated`, `migration_in_progress`, `migration_unsafe`, `manager_action_required`, `verification_failed`, `confirmation_required`, `unpublished_skill_ref`, `partial_apply` — each at exit 1, with `daemon_unavailable`/`storage_unavailable` unchanged at exit 2 (FR-167, `contracts/integration-cli.md` §Error codes)
- [ ] T033 Implement `cairn connect [<agent>] [--dry-run] [--yes] [--shared] [--scope <kind>=<scope>]` in `crates/cairn/src/connect.rs` against the plan engine — printing the change plan and exiting without writing under `--dry-run`, and exiting 1 with `confirmation_required` for a non-interactive run without `--yes` (FR-157, FR-164, FR-196, `contracts/integration-cli.md` §`cairn connect`)
- [ ] T034 Test `privacy::no_integration_outbox` in `tests/tests/privacy_integration.rs` asserting that writing every Feature 002 record type produces zero outbox rows and that no integration table has an outbox entity type at all (FR-183, SC-120)

**Checkpoint**: `cairn connect --dry-run` prints a change plan for a fixture repository and
writes nothing at all.

---

## Phase 4: User Story 2 + User Story 1 — Claude Code on the adapter model (Priority: P1) 🎯 First usable slice (plan Phase C)

**Goal**: Claude Code moves onto the adapter boundary with the usage contract and the Skill
installed, and a Feature 001 repository upgrades with zero duplicates and zero collateral
damage.

**Independent Test (US2)**: Take a repository configured by Feature 001 — six hook entries,
the project MCP entry, plus unrelated user hooks, unrelated MCP servers, and unrelated
settings. Run the Feature 002 connect. Assert exactly one Cairn entry per registered event,
exactly one Cairn MCP entry, every unrelated entry byte-identical, and captured behavior
identical to before.

**Independent Test (US1)**: Connect Claude Code to a scratch repository and assert one managed
instruction block bounded to its size, a versioned Skill installed, and the deeper workflow
material absent from the always-on block.

- [ ] T035 [US2] Implement Claude Code detection and capabilities in `crates/cairn-integrate/src/agents/claude_code.rs` — installation presence, version where obtainable without authentication, and the static profile of `data-model.md`, performing no mutation and requiring no network (FR-104, FR-105, D30)
- [ ] T036 [US2] Implement `plan` and `inspect` for Claude's four resource kinds in `crates/cairn-integrate/src/agents/claude_code.rs` at the matrix scopes — `mcp` at `~/.claude.json` (user), `lifecycle` at `.claude/settings.local.json` (project_local), `instructions` at `./CLAUDE.md` (project_shared), `skill` at `~/.claude/skills/cairn/` (user), with `--shared` moving `mcp` → `.mcp.json` and `lifecycle` → `.claude/settings.json` (FR-213, `contracts/scope-matrix.md` §Claude Code)
- [ ] T037 [US2] Replace Feature 001's fuzzy `entry.to_string().contains("cairn hook")` ownership test in `crates/cairn/src/connect.rs` with the legacy bridge in `crates/cairn-integrate/src/agents/claude_code.rs`: a closed set of exact shapes — a hook entry whose sole command is exactly `cairn hook <Event>` for one of the six Feature 001 events with the Feature 001 structure, and an MCP entry exactly `{"command":"cairn","args":["mcp"]}` — matched once, adopted in place at its actual scope, recorded, and never matched by shape again (FR-139, FR-217, `contracts/scope-matrix.md` §Ownership identity)
- [ ] T038 [US1] Install and update the managed instruction block in `./CLAUDE.md` (or `./.claude/CLAUDE.md` where the project keeps it) through the marker splicer, carrying `schema=` and `content=` in the marker and leaving every pre-existing instruction byte-for-byte unchanged (FR-132, FR-134, FR-136, US1 #1, US1 #5)
- [ ] T039 [US1] Install the Cairn Skill at `~/.claude/skills/cairn/` from the `include_dir`-embedded `skills/cairn/` tree, recording an `InstalledResource` plus a `claude-code` binding, never overwriting a Skill named `cairn` that Cairn does not own — that is `conflicting_owner` — and writing the computed `skill_schema`/`skill_revision` onto the record (FR-140, FR-143, `contracts/agent-contract.md` §Installation)
- [ ] T040 [US2] Implement `normalize` for Claude Code in `crates/cairn-integrate/src/agents/claude_code.rs` — `SessionStart`→`session_opened`, `PostToolUse`→`tool_succeeded`, `PostToolUseFailure`→`tool_failed` built from the payload's own error, `Stop`→`agent_quiesced`, `PreCompact`→`context_compacting`, `PostCompact`→`context_compacted`, `SessionEnd`→`session_closed`; every other Claude event is left unregistered and reported as unused rather than claimed (FR-115, FR-119, US2 #6, `contracts/lifecycle.md` §Claude Code)
- [ ] T041 [US2] Enforce the payload allow-list in `crates/cairn-integrate/src/agents/claude_code.rs` — retain only the session identifier, `cwd`, `source`, `trigger`, `reason`, `tool_name` as bounded provenance, and the redacted, bounded `tool_input.file_path`/`tool_input.command` and failure summary; `transcript_path`, `last_assistant_message`, `tool_calls`, prompts, `model`, `permission_mode`, `turn_id`, `agent_id`, and `agent_type` are read for routing and discarded (FR-197, FR-198, FR-199, `contracts/lifecycle.md` §Payload allow-list, D35)
- [ ] T042 [US2] Refactor `crates/cairn/src/hook.rs` to call `cairn-integrate::normalize` and register `PostCompact` alongside the six Feature 001 events, keeping the two deadline classes (250 ms capture, 1,500 ms boundary/context), the always-exit-0 rule, the fail-soft drop on an unreachable daemon, and the bounded context fallback at session open (FR-193, FR-194, FR-195, `contracts/lifecycle.md` §Failure behavior)
- [ ] T043 [US1] Implement `cairn agents` in `crates/cairn/src/` — detected agents and managers with version, compatibility classification, and integration level, where a level below FULL is never printed as a bare word and the parenthetical naming the missing behavior is part of the output, plus `--json` emitting the detection block of `contracts/integration-health.md` (FR-104, FR-110, FR-111, `contracts/integration-cli.md` §`cairn agents`)
- [ ] T044 [US7] Implement the first cut of `cairn doctor [<agent>]` in `crates/cairn/src/` over the shared inspection engine — core section (component version alignment, daemon reachability, project registration, local schema version), per-agent detection/version/compatibility/level/capability coverage, and per-resource condition, owner, scope and versions; read-only, exit 0 when every condition is `healthy`/`shared`/`unknown` (FR-166, FR-168, FR-170, `contracts/integration-health.md` §Health report)
- [ ] T045 [US1] **Skill revision task D** — implement doctor's Skill revision comparison in `crates/cairn-integrate/src/agents/` and the health report: read the installed `SKILL.md`'s `metadata.cairn_skill_revision`, recompute the embedded revision through T015's function, and report `outdated` on a mismatch with the remedy naming the correct source (FR-141, D29b, `contracts/cc-switch.md` §Skill Git ref)
- [ ] T046 [US1] Record `introspection` capability evidence in `crates/cairn-store/src/integrations.rs` as a byproduct of writing each configuration resource — reading back a resource Cairn itself wrote — marked version-independent so a detected version change re-derives it in place rather than discarding it (FR-242, FR-245, `data-model.md` §CapabilityEvidence, D19a)
- [ ] T047 [US2] Fixture tests `fixtures::idempotent_reconnect` and `fixtures::legacy_f001_adoption` in `crates/cairn-integrate/tests/fixtures.rs` — a second connect produces zero changes and reports `unchanged` for every resource kind; a Feature 001 repository upgrades to exactly one Cairn lifecycle entry per registered event and exactly one Cairn MCP entry with every pre-existing non-Cairn entry byte-identical; and Feature 001's project-scoped resources are adopted at their actual scope, never relocated (FR-135, FR-157, FR-216, FR-217, SC-102, SC-103, US2 #3)
- [ ] T048 [US2] Lifecycle fixture tests in `tests/integrations/claude-code/` asserting, per capability, exactly what the profile states: each `guaranteed` capability produces its canonical event from a realistic recorded payload, and every capability the profile does not claim produces nothing from any payload (FR-203, FR-204, SC-110, SC-124, D40 tier 3)
- [ ] T049 [US2] Behavior-equivalence test in `tests/tests/us1_capture_handoff.rs` (extended) — a Claude session that reads files, edits files, runs a failing test, quiesces, compacts, and ends produces the same stored observations, checkpoint, compaction handoff, and session-end handoff Feature 001 produced for the same actions; the canonical rename changes no stored behavior (FR-114, US2 #5)

**Checkpoint**: A Feature 001 repository upgrades with zero duplicates; SC-102, SC-103 and
SC-105 pass. Phase 4 alone is a shippable improvement over Feature 001.

---

## Phase 5: Sealed session close (Priority: P1) (plan Phase D)

**Goal**: A session boundary that acknowledges in one small transaction and still guarantees a
durable handoff while the daemon runs, so a vendor's short handler budget never costs the
completion guarantee.

**Independent Test**: With the daemon running throughout and no restart, 100 sealed closes all
have a durable handoff inside the documented bound; with synthesis forced to fail permanently,
all 100 surface as a named, retryable condition rather than silence.

**Dependency note**: independent of Phase 4, and it must precede Phase 6.

- [ ] T050 [US3] Implement the seal phase in `crates/cairnd/src/handlers.rs` — one transaction sets terminal status, end reason, `ended_at`, and `handoff_pending`, with no Git call, no capture quiesce, and no synthesis before the reply; the daemon answers there (FR-240 clause 1, `contracts/lifecycle.md` §The sealed close, D22)
- [ ] T051 [US3] Implement the synthesis phase in `crates/cairnd/src/handlers.rs` — quiesce in-flight captures, build the handoff, write it, clear `handoff_pending`, and retry on failure with bounded backoff incrementing `handoff_attempts` and recording a redacted `handoff_error` (FR-240 clause 2, `data-model.md` §Session)
- [ ] T052 [US3] Add the pending-handoff sweep to the daemon's existing maintenance tick in `crates/cairnd/src/recover.rs` — the same tick that reaps idle sessions also synthesizes any session whose `handoff_pending` has been set for more than a few seconds, so progress is guaranteed without a restart and without a new scheduler (FR-240 clause 2, D22)
- [ ] T053 [US3] Extend daemon-start reconciliation in `crates/cairnd/src/recover.rs` to pick up sessions sealed but not synthesized by a previous run, keeping it a backstop rather than the only retry path, and keep its handoffs marked `recovered` (FR-009 extension, FR-229)
- [ ] T054 [US3] Report the owed and failed states — `sessions_awaiting_handoff` and `handoff_synthesis_failures` in doctor's core section and in `cairn status` — with a redacted reason after a bounded number of attempts, retried at a slow cadence, and never treated as a terminal outcome that closes the matter (FR-240 clause 3, `contracts/integration-health.md` §Health report)
- [ ] T055 [US3] Gate the completion guarantee on the owed state in `crates/cairn-integrate/src/capability.rs` — a boundary that acknowledged but has not yet produced its handoff is reported as owed, not complete, and `completion_guarantee` is not `demonstrated` while any boundary is owed (FR-240 clause 4)
- [ ] T056 [US3] Add `wait_for_handoff` to the session-end request in `crates/cairn-core/src/wire.rs` — true for `cairn session end` from the command line, which keeps Feature 001's synchronous behavior, and false for hook-driven boundaries (`contracts/lifecycle.md` §The sealed close)
- [ ] T057 [US3] Test `tests/tests/handoff_lands_without_restart.rs` — ≥100 sealed closes with the daemon running throughout, 100% with a durable handoff inside the documented bound and no daemon restart; with synthesis forced to fail permanently, 100% reported as a named condition rather than left silently owed (SC-136)
- [ ] T058 [US3] Test `tests/tests/recovery_injected.rs` — handler timeout, handler crash, and daemon unavailable at the boundary each end with the session reconciled and a durable handoff, zero sessions left permanently without one, and zero agent sessions aborted or visibly disrupted (SC-129)

**Checkpoint**: `perf_session_close`'s dependencies exist; `handoff_lands_without_restart` and
`recovery_injected` pass.

---

## Phase 6: User Story 3 — Codex participates natively (Priority: P1) (plan Phase E)

**Goal**: Codex sessions are Cairn sessions through Codex's own configuration and lifecycle
surfaces, with hook trust as a first-class state and the completion guarantee earned by
measurement rather than asserted.

**Independent Test**: In a scratch repository with Codex installed, connect Codex, complete
the trust step Codex requires, run a short session that edits a file and runs a failing
command, and assert a Cairn session with Codex as its agent, observations in the correct
canonical categories, and a handoff at the boundary Codex actually signals.

- [ ] T059 [US3] Implement Codex detection and capabilities in `crates/cairn-integrate/src/agents/codex.rs`, including `handlers_require_trust: yes` and `pending_activation` availability for every lifecycle capability until the user trusts the hooks (FR-104, FR-209, D31, D24)
- [ ] T060 [US3] Implement `plan`/`inspect` for Codex in `crates/cairn-integrate/src/agents/codex.rs` — `mcp` as `[mcp_servers.cairn]` in `~/.codex/config.toml` via `toml_edit`, `lifecycle` in `~/.codex/hooks.json` via the JSON CST, `instructions` in `./AGENTS.md`, `skill` at `~/.codex/skills/cairn/`, with `--shared` moving `mcp` and `lifecycle` into `.codex/config.toml`; state at connect time that per-user is the only developer-local option Codex offers rather than falling back to a committed file silently (FR-212, FR-218, `contracts/scope-matrix.md` §Codex)
- [ ] T061 [US3] Implement `normalize` for Codex's six registrations in `crates/cairn-integrate/src/agents/codex.rs` — `SessionStart`, `PostToolUse` (one registration, two canonical outcomes), `Stop`, `PreCompact`, `PostCompact`, `SessionEnd` — leaving `PreToolUse`, `PermissionRequest`, `UserPromptSubmit`, `SubagentStart`, and `SubagentStop` unregistered (FR-115, `contracts/lifecycle.md` §Codex)
- [ ] T062 [US3] Implement Codex failure classification in `crates/cairn-integrate/src/agents/codex.rs` in the order explicit non-zero `exit_code` → explicit `success: false` or `error` → otherwise success, where an uninterpretable response yields the success-shaped observation and never a fabricated error (FR-117, D23)
- [ ] T063 [US3] Implement the trust states in `crates/cairn-integrate/src/agents/codex.rs` — a newly written hook is untrusted and an edited trusted hook is reset, both reported as `installed_not_activated` with the exact command to run and a `post_apply_actions` entry, with the level reflecting what actually works until they are active (FR-209, US3 #3, D24)
- [ ] T064 [US3] Lifecycle fixture tests in `tests/integrations/codex/` — every `guaranteed` capability produces its canonical event; the ambiguous `tool_response` produces the success-shaped observation and never an asserted failure; and no payload produces an event the profile does not claim (SC-110, SC-124, US3 #4, US3 #5)
- [ ] T065 [US3] Test `tests/tests/perf_session_close.rs` — ≥100 session-end boundaries with release builds and a healthy daemon, Cairn's session-end work inside Codex's own 1 s default / 3 s maximum budget in 100% of runs and exceeding the deadline in none (FR-208, SC-128)
- [ ] T066 [US3] Wire the Codex level gate in `crates/cairn-integrate/src/capability.rs` — FULL only once hook trust is active, SC-128 is demonstrated, and every FULL-required capability is established; otherwise Codex is reported below FULL with automatic session completion named as the missing behavior (FR-207, SC-127, US3 #7)

**Checkpoint**: Codex connects, SC-128 is measured, and the level is FULL only after trust.

---

## Phase 7: User Story 4 — OpenCode participates natively (Priority: P1) (plan Phase F)

**Goal**: OpenCode captures and receives context through its own plugin surface, with the
parts of the lifecycle it does not signal reported as absent rather than faked.

**Independent Test**: Connect OpenCode in a scratch repository, run a short session, and assert
observations are captured, that going idle produces a quiescence checkpoint leaving the Cairn
session `active`, and that the capability report claims no session-completion signal.

- [ ] T067 [US4] Create the Cairn plugin asset and install it as a file drop at `~/.config/opencode/plugin/cairn.js` (`--shared`: `.opencode/plugin/cairn.js`) — OpenCode auto-discovers `{plugin,plugins}/*.{ts,js}` in every config directory, so the lifecycle path needs no mutation of `opencode.json` at all; Cairn generates the whole file and owns it by path (FR-212, `contracts/scope-matrix.md` §OpenCode, D32)
- [ ] T068 [US4] Implement OpenCode detection and capabilities in `crates/cairn-integrate/src/agents/opencode.rs` — `lifecycle_tool_failure` as **conditional** with what it depends on named, `lifecycle_pre_compaction` conditional on `experimental.session.compacting` being exposed, and `lifecycle_session_close` as **absent** (FR-107, FR-108, FR-241, `data-model.md` §CapabilityProfile)
- [ ] T069 [US4] Implement `normalize` for OpenCode in `crates/cairn-integrate/src/agents/opencode.rs` — `session.created` or first activity for an unseen `sessionID` → `session_opened` with context delivered at the first `chat.message`, `tool.execute.after` → `tool_succeeded`, `session.idle` → `agent_quiesced` and **never** `session_closed`, `experimental.session.compacting` → `context_compacting`, `session.compacted` → `context_compacted`, with `session.deleted`, `session.updated`, `session.status`, `session.error`, `message.*`, `file.*`, `permission.*`, `todo.*`, `lsp.*`, `installation.*`, and `server.*` unmapped (FR-115, FR-116, `contracts/lifecycle.md` §OpenCode)
- [ ] T070 [US4] Implement the conditional tool-failure rule in `crates/cairn-integrate/src/agents/opencode.rs` — emit `tool_failed` only where the output unambiguously establishes a failure, emit nothing where it does not, and never synthesize an outcome from `session.idle`; OpenCode's tool `output` text and `metadata` are never persisted (FR-117, FR-199, FR-231)
- [ ] T071 [US4] Implement OpenCode's `mcp` entry at `~/.config/opencode/opencode.json` `mcp.cairn` via the JSON CST, and detect a `mcp.cairn` already declared in `opencode.jsonc` — which merges after the `.json` and would shadow Cairn's entry — as `conflicting_owner`, reported and never edited (FR-150, `contracts/scope-matrix.md` §OpenCode, D37, D38)
- [ ] T072 [US4] Implement shared-resource binding in `crates/cairn-integrate/src/agents/opencode.rs` — bind OpenCode to the **existing** `AGENTS.md` managed block Codex uses rather than writing a second one, and bind to Claude Code's `~/.claude/skills/cairn/` where it exists rather than writing a second Skill that would collide on skill name, falling back to `~/.config/opencode/skills/cairn/` otherwise (FR-144, FR-243, D28)
- [ ] T073 [US4] Make plugin registration idempotent in `crates/cairn-integrate/src/agents/opencode.rs` — an existing Cairn plugin registration is updated in place, never appended a second time, and unrelated plugin registrations are untouched (FR-157, FR-180, US4 #9)
- [ ] T074 [US4] Lifecycle fixture tests in `tests/integrations/opencode/` — `lifecycle::idle_never_closes` asserts no adapter maps an idle, quiet, or inactive signal to `session_closed`, for every adapter; `lifecycle::quiesce_after_error` asserts a quiescence signal following an error produces exactly one checkpoint and zero synthesized success or failure observations; and the conditional failure capability is tested both ways — one payload that establishes failure produces `tool_failed`, one ambiguous payload produces nothing (FR-116, FR-230, SC-110, SC-111, SC-134)
- [ ] T075 [US4] Test `tests/tests/us4_opencode.rs::idle_reaper_never_grants_full` — with the inactivity timeout and daemon-start reconciliation as the only routes to a terminal session state, the computed level is below FULL in 100% of cases and the report states that sessions are closed by inactivity rather than completed (FR-229, SC-131, US4 #6, US4 #7)
- [ ] T076 [US4] Assert the extension path in `crates/cairn-integrate/src/capability.rs` — a future mechanism that establishes actual termination promotes OpenCode to FULL on the strength of the demonstrated capability alone, with no vendor event name special-cased anywhere in the derivation (FR-207, US4 #8)

**Checkpoint**: OpenCode connects; the idle-reaper negative test passes; the level is
MCP_PLUS and says why.

---

## Phase 8: User Story 7 + User Story 9 — Repair, ownership, and removal (Priority: P1/P2) (plan Phase G)

**Goal**: Cairn diagnoses exactly what is wrong, repairs only what it owns, and makes removal
and ownership migration safe enough that connecting is a low-risk decision.

**Independent Test (US7)**: Connect every supported agent, introduce eight distinct defects,
run diagnostics and assert each is identified by exact resource and correct condition; run
repair and assert each Cairn-owned defect is fixed, nothing else changed, and a second repair
reports nothing to do.

**Independent Test (US9)**: With several agents connected and unrelated configuration present,
disconnect one agent and assert its Cairn resources are gone, every other agent still works,
every unrelated setting is intact, and every project, task, session, memory, and handoff still
exists. Then migrate one resource between owners and assert no window exists in which it is
absent or doubled.

- [ ] T077 [US7] Complete the health report in `crates/cairn/src/` over the shared inspection engine — the full closed `HealthCondition` set, mandatory `lifecycle_coverage` split guaranteed/conditional/absent, mandatory `conditional_behaviors` beside any conditional entry, mandatory `serves` on any multi-binding resource, mandatory `missing_behaviors` below FULL, mandatory `unverified_behaviors` and `awaited_behaviors`, plus `evidence` naming what established each capability and against which version (FR-167, FR-168, FR-243, `contracts/integration-health.md` §Health report)
- [ ] T078 [US7] Enforce diagnostic redaction in the report renderer — `detail` names the problem without quoting user configuration beyond what identifies it, no content of user instructions, user Skills, or unrelated configuration appears, and no credential, token, or key value is ever printed in human or machine-readable form (FR-162, FR-171, US7 #8)
- [ ] T079 [US7] Fixture `fixtures::defects` in `crates/cairn-integrate/tests/fixtures.rs` seeding the eight defects — deleted lifecycle entry, damaged managed block, outdated contract version, outdated Skill version, duplicated MCP entry, resource under the wrong owner, malformed configuration file, and an inactive lifecycle handler — and asserting the exact resource and correct condition in 100% of cases (FR-166, SC-114)
- [ ] T080 [US7] Implement `cairn repair [<agent>] [--dry-run]` in `crates/cairn/src/` — restore `missing`, upgrade `outdated`, remove `duplicated` Cairn-owned entries, run for one agent or all of them, share connect's preview, and change nothing else (FR-172, FR-173, FR-175, FR-176, `contracts/integration-cli.md` §`cairn repair`)
- [ ] T081 [US7] Implement the report-only conditions in `crates/cairn/src/` — `modified`, `damaged_markers`, `conflicting_owner`, and `malformed_config` are explained with their options and changed by no default repair, and a malformed file is never replaced with a valid file of Cairn's own construction (FR-174, FR-177, US7 #3, US7 #6)
- [ ] T082 [US7] Implement `cairn repair --force` and `RecoveryArtifact` in `crates/cairn-integrate/src/apply.rs` and `crates/cairn-store/src/integrations.rs` — restore `modified` resources strictly inside Cairn's ownership boundary after preserving only the Cairn-owned block, entry, or wholly Cairn-generated file to `$CAIRN_HOME/recovery/<agent>/<kind>/<ts>-<hash>.txt`, ten most recent per `(agent, kind)`, path printed but content never logged, never synced, never containing a foreign credential; `--force` still refuses `damaged_markers`, `conflicting_owner`, and `malformed_config` (FR-221, FR-222, FR-238, FR-239, D39)
- [ ] T083 [US7] Fixture `fixtures::repair` and `markers::semantic_equivalence` — repair fixes 100% of Cairn-owned unambiguous defects, changes zero non-Cairn settings, reports nothing to do on a second run; a formatting-only difference is healthy and a semantic edit is modified in 100% of seeded cases with default repair changing neither; no recovery path requires disconnect-and-reconnect; and repair never adopts a developer's edit as Cairn-managed content (FR-223, FR-224, FR-225, SC-115, SC-130)
- [ ] T084 [US9] Implement `cairn disconnect <agent> [--only <kind>]... [--dry-run]` in `crates/cairn/src/connect.rs` — removal is by binding, not by file: drop this agent's binding to each resource and delete the resource only when no other binding remains; remove the agent's local record last and only once its last binding is gone (FR-178, FR-243, FR-244, `contracts/integration-cli.md` §`cairn disconnect`)
- [ ] T085 [US9] Assert disconnect's blast radius in `fixtures::disconnect` and `tests/tests/us9_removal.rs` — no project, task, session, observation, memory, or handoff is deleted; no unrelated MCP server, hook, plugin, instruction, credential, or setting is modified; no other connected agent's configuration changes; every unrelated setting in the touched files is byte-identical; and an instruction file survives with the developer's content when only Cairn's block is removed (FR-179, FR-180, SC-116, US9 #8)
- [ ] T086 [US9] Fixtures `fixtures::shared_binding` and `fixtures::manager_state_survives` — disconnecting one of two agents bound to the shared `AGENTS.md` block or the shared Skill leaves the resource in place and the remaining agent healthy in 100% of cases, disconnecting the last one removes it, disconnect stays idempotent, and a manager-owned resource plus its pending manager action survive a native disconnect and remain verifiable (FR-243, FR-244, SC-137, D28a)
- [ ] T087 [US9] Implement the `MigrationState` machine in `crates/cairn-integrate/src/plan.rs` and `crates/cairn-store/src/integrations.rs` — `planned → target_installed → target_verified → source_removed`, with `overlap_permitted` false collapsing the first step into a single atomic replacement, the target verified in the agent's real configuration before the source is removed, exactly one owner remaining on success, and the previously working configuration preserved or restored on failure at any step (FR-148, FR-228)
- [ ] T088 [US9] Implement `cairn integration migrate <agent> <kind> --to direct|cc-switch [--dry-run] [--resume] [--abort]` in `crates/cairn/src/` — an interrupted migration is reported as `migrating` with its source and target, distinctly from `duplicated` and `conflicting_owner`, and is resumable or reversible without disconnecting the agent first (FR-228, US9 #7, `contracts/integration-cli.md` §`cairn integration migrate`)
- [ ] T089 [US9] Implement the `migration_unsafe` refusal in `crates/cairn-integrate/src/plan.rs` — where two owners write one effective slot and overlap would make the effective configuration ambiguous, refuse to migrate automatically and print the manual sequence, which is the expected outcome for Claude Code's `mcp` at user scope (FR-148, D38)
- [ ] T090 [US9] Fixture `fixtures::migration` — inspecting configuration after every step shows the developer is never left with no effective resource: zero intermediate states where both owners share one slot, exactly one unambiguous effective resource recorded as `migrating` where they do not, exactly one owner on completion, and the previously working configuration intact under induced failure at each step, including interrupted resume and abort (SC-117, US9 #5, US9 #6)
- [ ] T091 [US7] Fixture `fixtures::malformed_and_readonly` covering the environment edge cases — an absent configuration directory (Cairn creates only what it owns at the documented location and reports it), a malformed file (fails loudly with the file named and the parse problem stated, nothing rewritten, no partial file left), a read-only file or full filesystem (fails with the reason, original untouched), and an unreachable daemon during connect (fails clearly, writes no configuration it cannot verify) (FR-137, FR-196, spec.md §Edge Cases)

**Checkpoint**: Eight seeded defects are detected and repaired; SC-114–SC-117 pass.

---

## Phase 9: User Story 5 — CC Switch distributes Cairn across agents (Priority: P2) (plan Phase H)

**Goal**: Cairn's MCP server and Skill reach CC Switch-managed applications through CC Switch's
own documented import surface, with zero writes to its storage and no fabricated removal path.

**Independent Test**: With CC Switch installed, distribute the Cairn MCP server and the Cairn
Skill to a chosen set of applications, then run diagnostics and assert each target application
has exactly one Cairn MCP entry, that Cairn records CC Switch as owner, and that no unrelated
CC Switch configuration changed.

- [ ] T092 [US5] Implement CC Switch detection in `crates/cairn-integrate/src/managers/cc_switch.rs` — installation presence and version where obtainable without authentication, reported as an integration manager and never in the agent list, reading nothing from `~/.cc-switch/cc-switch.db` or any other private file, not even for detection (FR-101, FR-103, FR-104, FR-232, `contracts/cc-switch.md` §Detection)
- [ ] T093 [US5] Implement the `ccswitch://v1/import` deep-link builder in `crates/cairn-integrate/src/managers/cc_switch.rs` for `resource=mcp` and `resource=skill`, carrying only the secret-free configuration block `cairn integration export mcp` emits, and never a credential (FR-200, `contracts/cc-switch.md` §Distribution)
- [ ] T094 [US5] Implement `cairn integration distribute --via cc-switch --resource mcp|skill --apps <list> [--dry-run]` in `crates/cairn/src/` — open or print the link, then stop and return `ManagerActionRequired` with status `awaiting_user` at exit 1; the operation has not completed and Cairn never reports success on the strength of having asked (FR-165, FR-233, `contracts/integration-cli.md` §`cairn integration distribute`)
- [ ] T095 [US5] **Skill revision task E** — implement the published-branch rule in `crates/cairn-integrate/src/managers/cc_switch.rs`: the Skill import emits `branch=skill-release/<schema>-<revision>` only where the running build was given that verified branch name as a build input, and otherwise fails with `unpublished_skill_ref` stating why and giving the manual path — never a SHA, never a tag, never floating `main`, because CC Switch's downloader silently falls back to `main` then `master` on any miss; an MCP distribution from the same development build still succeeds (FR-149, `contracts/cc-switch.md` §Skill Git ref, D29, D29a)
- [ ] T096 [US5] Add the `publish-skill` job to `.github/workflows/release.yml` with `needs: verify`, outputs `skill_schema`/`skill_revision`/`skill_branch`, and ref-write permission scoped to it alone: compute both values via `cargo run -q -p cairn-integrate --bin skillref -- --json`, create `skill-release/<schema>-<revision>` at the release commit when absent, **never move it when present**, then verify in both cases by downloading `https://github.com/Vellixia/Cairn/archive/refs/heads/<branch>.zip` the way CC Switch does and recomputing the revision from the fetched tree; a content/name mismatch fails the release. Rewire `binaries` to `needs: [verify, publish-skill]` consuming `skill_branch` as a build input, leave `images` on `needs: verify`, and add no `publish-skill` to `ci.yml` so every development build keeps `unpublished_skill_ref` behavior (D29a)
- [ ] T097 [US5] **Skill revision task F** — release-branch tests in `crates/cairn-integrate/tests/release_branch.rs` covering the three cases and the failure: release **A** introduces revision `R` and the branch is created at A; release **B** changes no Skill file, the branch still points at A, is **not moved**, its content is re-verified, and the release succeeds; release **C** introduces revision `S`, creating a new branch and leaving `…-R` untouched; and a branch whose fetched content does not match its name fails the release. No force update in any case (D29a, D29b)
- [ ] T098 [US5] Implement binding verification in `crates/cairn-integrate/src/managers/cc_switch.rs` — after the developer confirms, doctor inspects each target application's **own** configuration (`~/.claude.json`, `~/.codex/config.toml`, `~/.config/opencode/opencode.json`) and each application's Skill directory, and records ownership from what it finds rather than from what was requested, updating `status` to `verified` or `not_performed` (FR-169, FR-234, `contracts/cc-switch.md`)
- [ ] T099 [US5] Implement the removal outcome in `crates/cairn-integrate/src/managers/cc_switch.rs` — CC Switch documents no automated removal, so withdrawal returns `manager_action_required` naming the resource, the target applications, the supported CC Switch path, and `verify_with`, with `uri: null` rather than a fabricated link, the record kept at `owner = manager` until verification says otherwise, and the withdrawal available as an operation separate from disconnecting a native adapter; where CC Switch later publishes a documented removal interface the adapter may use it under the same verification (FR-149, FR-181, FR-233, FR-235, FR-237, `contracts/cc-switch.md` §Removal)
- [ ] T100 [US5] Implement the direct → manager migration path in `crates/cairn-integrate/src/plan.rs` — automated up to CC Switch's confirmation boundary, verified in the agent's real configuration, and only then removing what Cairn owns directly; the reverse direction installs and verifies the direct resource, then reports the manager-side withdrawal as manager action required and stays `migrating` until exactly one owner is confirmed (FR-236, FR-237, `contracts/cc-switch.md` §Migration between owners)
- [ ] T101 [US5] Fixture `fixtures::manager_zero_writes` in `crates/cairn-integrate/tests/fixtures.rs` — checksum every file under `~/.cc-switch/` before and after connect, distribute, migrate, repair, and disconnect and assert zero changes in 100% of cases, and assert that ownership is updated only from verification against the target applications (FR-232, SC-132)
- [ ] T102 [US5] Fixture `fixtures::manager_bindings` — distributing to a chosen set of applications yields exactly one Cairn MCP entry per selected application, zero changes to unrelated manager-held configuration, and zero conflicting-ownership findings; and where Cairn already owns `mcp` directly for that agent, connect returns `conflicting_owner` naming both rather than silently creating a second copy (FR-146, FR-219, SC-112, US5 #4)
- [ ] T103 [US5] Fixture `fixtures::post_provider_switch` — after a provider or configuration switch inside CC Switch, every Cairn resource is reported healthy with zero duplicates and every other CC Switch-managed provider, MCP server, prompt, and Skill is untouched (FR-200, SC-113, US5 #8)
- [ ] T104 [US5] Assert the adapter boundary in `crates/cairn-integrate/src/managers/cc_switch.rs` tests — CC Switch produces no sessions, no observations, and no lifecycle events, and no native lifecycle adapter is added for any application it happens to support; those reach Cairn through the generic MCP path only (FR-101, FR-106)

**Checkpoint**: Distribution is verified with zero writes to the manager's storage; a
development build refuses the Skill import; the release job creates and verifies the branch,
and fails rather than moving an existing one.

---

## Phase 10: User Story 8 — Any MCP-compatible agent gets useful memory (Priority: P2) (plan Phase I)

**Goal**: A plain MCP client initializes, receives the usage contract where the protocol
carries it, exercises the six tools, and is told plainly what automatic behavior it is not
getting.

**Independent Test**: Drive Cairn's MCP server with a plain MCP client. Assert initialization
succeeds, the usage contract arrives, exactly six tools are offered, each works, and the
reported level states that automatic lifecycle and capture are unavailable.

- [ ] T105 [US8] Add the `instructions` field to the `initialize` response in `crates/cairn/src/mcp.rs`, rendered from the same contract source, keeping `PROTOCOL_VERSION` at `2025-06-18` and answering only with a version Cairn actually implements — never echoing one it does not support (FR-129, FR-130, D34)
- [ ] T106 [US8] Extend the existing `mcp::tool_count` test in `crates/cairn/src/mcp.rs` so a seventh tool fails the build, and assert the six Feature 001 tool names are unchanged — diagnostics, repair, connection, and export stay developer operations, not agent tools (FR-128, SC-106)
- [ ] T107 [US8] Implement the generic MCP adapter in `crates/cairn-integrate/src/agents/generic_mcp.rs` — every lifecycle capability absent, level `MCP_ONLY`, and a report that never treats the contract as delivered because a client may not surface server instructions (FR-129, `contracts/lifecycle.md` §Generic MCP)
- [ ] T108 [US8] Implement `cairn integration export mcp [--agent <agent>] [--format json|toml]` in `crates/cairn/src/` — deterministic, secret-free, writing nothing, emitting the named agent's format or a generic `mcpServers` object (FR-131, `contracts/integration-cli.md` §`cairn integration export mcp`)
- [ ] T109 [US8] Test `tests/integrations/generic-mcp/` — a plain client completes initialization, receives the usage contract where the protocol carries it, exercises all six tools with Feature 001 behavior, and the reported level is `MCP_ONLY` naming automatic lifecycle and automatic capture as unavailable and never describing the integration as full (FR-110, SC-107, US8 #2, US8 #3)

**Checkpoint**: A plain MCP client initializes, sees six tools, and is reported MCP_ONLY.

---

## Phase 11: User Story 6 + User Story 10 — Evidence (Priority: P1) (plan Phase J)

**Goal**: The product claim proved end to end — knowledge crosses agents, concurrent agents
never steal each other's sessions, capture stays inside its budget per adapter, privacy holds,
and the browser suite runs in hosted CI.

**Independent Test (US6)**: Run the cross-agent continuity scenario end to end on one
repository and assert the second and third agents retrieve the first agent's durable knowledge
and latest handoff, with provenance naming the agent and session that produced each item.

**Independent Test (US10)**: Run two agents concurrently in one worktree and, separately, in
two worktrees of one repository, and assert two distinct sessions, correct provenance, one
project, and an actionable ambiguous-session error instead of a guess.

- [ ] T110 [US6] Test `tests/tests/us6_cross_agent.rs` — against a real daemon and a real temporary Git repository, drive three adapters' `normalize` with recorded payloads under three distinct `agent_session_key`s: knowledge recorded in the first agent is retrieved by the second and third in the session in which they open the repository, with zero export, import, or copy steps, and exactly one project exists for the repository (FR-191, FR-192, SC-108, D41)
- [ ] T111 [US6] Assert the memory invariant in `tests/tests/us6_cross_agent.rs` — no memory scope, partition, ownership domain, or retrieval filter keyed to the producing agent exists anywhere; scoping and ranking use only project, branch, task, and session; agent identity appears as provenance only (FR-189, FR-190, US6 #3, US6 #4)
- [ ] T112 [US10] Test `tests/tests/us10_concurrency.rs` — two supported agents in one worktree yield exactly two active sessions bound to one project, 100% of observations and memories carry the provenance of the session that produced them, zero events route to the wrong session, every unattributable request returns an ambiguous-session error naming the candidates rather than a guess, and two worktrees of one repository give one project, two sessions, and per-worktree repository state (FR-118, SC-109, US10 #5)
- [ ] T113 [US10] Assert the identifier-absence path in `tests/tests/us10_concurrency.rs` — an adapter that cannot supply a stable session identifier reports `stable_session_identifier: absent` in its capability profile rather than sharing one session between agents (US10 #6)
- [ ] T114 [P] Test `tests/tests/perf_capture.rs` — ≥200 capture-class lifecycle invocations per adapter with release builds, staying within Feature 001's SC-007 latency budget and within each agent's own handler deadline, with zero Cairn failures aborting or visibly disrupting an agent session, and reporting the per-user hook and plugin cost measured in a repository Cairn does not manage (FR-194, SC-007, SC-122, plan.md risk table)
- [ ] T115 [P] Test `tests/tests/privacy_integration.rs` — across every connect, repair, migrate, and disconnect on configuration seeded with recognizable credentials, zero credentials appear in recovery artifacts, local state, logs, diagnostics, or sync payloads; zero whole-file copies of pre-existing configuration are created; and zero agent configuration content, absolute path, or integration health detail appears in any outbound sync payload or in the shared server's PostgreSQL database, inspecting both (FR-183, FR-197, FR-200, SC-119, SC-120, SC-133)
- [ ] T116 [P] Test `tests/tests/privacy_payloads.rs` — with vendor payloads seeded with recognizable assistant text, user prompts, transcripts, and raw tool output, zero conversation content is present in stored observations, memories, or handoffs, and every adapter-captured field passed through Feature 001's exclusion → redaction → bounding pipeline with no path around it (FR-198, FR-199, SC-121)
- [ ] T117 [P] Test `tests/tests/capability_evidence.rs` — a FULL-required runtime capability that has never been observed keeps the level below FULL and is named as awaited; a detected agent-version change deletes `observation` evidence and only that, keeping `introspection` evidence; once every FULL-required capability is established FULL is granted; a session start that delivered no context does not establish `context_at_session_open` while a degraded delivery does and records `degraded: true`; and a synthesized identifier, or a single event carrying one, does not establish `stable_session_identifier` (FR-242, FR-245, SC-138, D19a)
- [ ] T118 [P] Test `capability::compatibility` in `crates/cairn-integrate/src/capability.rs` — a version newer than any Cairn has verified is `compatible_unverified` and integrates successfully; only a positively known-incompatible version is `unsupported`, and the report states what is incompatible; and adapter behavior degrades by capability detection rather than version-string matching, so a removed vendor event lowers the level and the integration keeps working (FR-185, FR-186, FR-187, FR-188, SC-123, US7 #7)
- [ ] T119 Add the `web-e2e` job to `.github/workflows/ci.yml` — PostgreSQL service container, `cargo build --release -p cairn-server` started on `127.0.0.1:8080`, `npm ci && npm run build && npm start` on `127.0.0.1:3100` with `NEXT_PUBLIC_CAIRN_API` pointed at the server, then `npx playwright test` over both existing desktop and mobile projects with Chromium only, traces uploaded on failure, a 20-minute job timeout, and the job added to required checks alongside the existing lint/typecheck/build job (FR-206, SC-125, D42)
- [ ] T120 Assert CI hermeticity — the adapter fixture tests for every native adapter, the integration manager, and the generic MCP path pass in required CI with no agent installed, no credentials, and no network, and no task in this feature requires a secret or an authenticated service to run in CI (FR-204, FR-205, SC-124)
- [ ] T121 [MANUAL] [US6] Live release evidence for the cross-agent scenario — run the `quickstart.md` US6 walkthrough with real, authenticated Claude Code, Codex, and OpenCode on one repository and confirm the recorded payload fixtures still match what the agents actually send (FR-205, SC-108 live half, D40 tier 5)
- [ ] T122 [MANUAL] Live release evidence for onboarding — a developer with each of Claude Code, Codex, and OpenCode installed goes from `cairn init` to a connected, capturing session in under 5 minutes using only documented steps (SC-101)
- [ ] T123 [MANUAL] [US5] Live release evidence for Skill distribution — with CC Switch installed, a published `skill-release/<schema>-<revision>` branch fetches through CC Switch's own `refs/heads` path, the installed `metadata.cairn_skill_revision` equals the embedded one, and an unchanged Skill across two releases reuses the branch without moving it (SC-112 live half, D29a, D29b)

**Checkpoint**: The quickstart runs end to end; `web-e2e` is a required check; every success
criterion has a passing named test.

---

## Phase 12: Polish & Cross-Cutting Concerns

- [ ] T124 Implement `cairn connect --auto` guided onboarding in `crates/cairn/src/connect.rs` — detect everything installed, propose a complete plan from `DesiredIntegrationState` including the proposed owner for each resource and the manager-owned/directly-owned split, present it for confirmation before anything changes, and refuse to proceed where a conflict needs a human decision even with `--yes` (FR-163, FR-164, FR-165)
- [ ] T125 Record the measurements in `specs/002-agent-integration-platform/quickstart.md` §Measurements on record — per-adapter capture latency median/p95, Codex seal duration median/p95/max, handoff-after-seal p50/p99, rendered contract size in characters, and the per-user hook cost in an unmanaged repository; if that last number is material, revise D27's matrix to recommend `--shared` for Codex and OpenCode (SC-122, plan.md open questions)
- [ ] T126 [P] Write the connect, doctor, repair, disconnect, and distribute documentation in `README.md` and `docs/`, covering the per-resource scope defaults, what `--shared` changes, and the fact that cloning a repository installs and activates nothing (FR-216)
- [ ] T127 [P] Audit and record: exactly six MCP tools (FR-128); zero outbox entity types and zero server schema changes for any Feature 002 entity (FR-183, FR-184); `cairn-server` untouched; no manifest drift handling, merge semantics, or automatic application of intent on clone crept in (FR-227); and every item in `spec.md` §Out of Scope is still out
- [ ] T128 Run the full `quickstart.md` walkthrough on macOS and Linux, hermetic sections in CI and live sections by hand, and fix what it surfaces

---

## The Skill revision chain

The six tasks D29a/D29b require, in dependency order. Each calls the same function; none
reimplements the digest.

| # | Task | Delivers |
|---|---|---|
| A | T015 | The one canonical `skill_revision` algorithm in `cairn-integrate::revision` |
| B | T016 | Self-validation: the checked-in frontmatter equals the computed value; the self-field is normalized before hashing |
| C | T017 | `skillref` — the developer binary the workflow calls instead of shelling out a hash |
| D | T045 | Doctor recomputes the embedded revision and compares it with the installed `SKILL.md` |
| E | T096 | The `publish-skill` release job: create when absent, never move, verify through CC Switch's own `refs/heads` fetch |
| F | T097 | Release A/B/C branch tests plus the content/name mismatch failure |

---

## Dependencies & Execution Order

### Phase dependencies

- **Setup (Phase 1)**: no dependencies
- **Foundational Domain (Phase 2)**: depends on Setup — blocks every user story
- **Foundational Engine (Phase 3)**: depends on Phase 2 — blocks every user story
- **Claude Code (Phase 4)**: depends on Phase 3
- **Sealed close (Phase 5)**: depends on Phase 3; independent of Phase 4; **must precede Phase 6**
- **Codex (Phase 6)**: depends on Phases 4 and 5 — the legacy bridge and the shared `AGENTS.md` block are proved in Phase 4, and the completion guarantee needs Phase 5
- **OpenCode (Phase 7)**: depends on Phase 4 for the shared-resource machinery; independent of Phase 6
- **Repair & migrate (Phase 8)**: depends on Phases 4–7 — it repairs and removes what those install
- **CC Switch (Phase 9)**: depends on Phase 8 for the migration machine
- **Generic MCP (Phase 10)**: depends on Phase 2 only; can run any time after it
- **Evidence (Phase 11)**: depends on everything
- **Polish (Phase 12)**: depends on everything intended to ship

### Within a story

- The negative assertions are part of the implementation, not a follow-up: an adapter is not
  done until the "produces nothing" half of its capability profile is asserted
- Domain and store changes before daemon logic; daemon logic before CLI surface
- A fixture exists before the behavior it pins is relied on elsewhere
- Story complete and demonstrable before moving to the next priority

### Parallel opportunities

- T002–T005 in Setup
- T006–T009, T011, T013, T015, T018, T020 in Phase 2 — distinct modules, no shared state
- T022–T024 in Phase 3 — three independent editors
- Phase 7 (OpenCode) alongside Phase 6 (Codex) once Phase 4 lands, with separate people
- Phase 10 (Generic MCP) alongside anything after Phase 2
- T114–T118 in Phase 11 — distinct test binaries
- T126–T127 in Polish

---

## Parallel Example: Phase 2

```bash
# The domain modules have no dependencies on each other:
Task: "Define CanonicalLifecycleEvent in crates/cairn-core/src/domain.rs"
Task: "Implement the capability model in crates/cairn-integrate/src/capability.rs"
Task: "Implement DesiredIntegrationState in crates/cairn-integrate/src/desired.rs"
Task: "Implement ownership markers in crates/cairn-integrate/src/markers.rs"
Task: "Implement the skill_revision algorithm in crates/cairn-integrate/src/revision.rs"
Task: "Implement both contract renderings in crates/cairn-integrate/src/render.rs"
Task: "Implement the scope matrix in crates/cairn-integrate/src/scope.rs"
```

---

## Implementation Strategy

### First usable slice

1. Phase 1 → Phase 2 → Phase 3. At the end of Phase 3, `cairn connect --dry-run` prints a
   change plan and writes nothing — the safety property everything else rests on.
2. Phase 4. Claude Code is on the adapter boundary with the usage contract and the Skill, and
   a Feature 001 repository upgrades with zero duplicates. **Stop and validate**: SC-102,
   SC-103, SC-104 and SC-105 all pass here, and this state is shippable on its own.

### Incremental delivery

Each phase after that adds one honest capability and can be demoed:

- Phase 5 → session completion is durable at a budgeted boundary
- Phase 6 → Codex, FULL only once trust and measurement earn it
- Phase 7 → OpenCode, MCP_PLUS and saying exactly why
- Phase 8 → the integration can diagnose and fix itself, and be removed safely
- Phase 9 → distribution through CC Switch, with zero writes to its storage
- Phase 10 → any MCP client gets useful memory
- Phase 11 → the product claim proved: knowledge crosses agents

**Stopping early is a scope decision, not a completed feature.** Feature 002 is complete only
when every user story is delivered; a P2 story is later in sequence, not optional.

### Parallel team strategy

1. Everyone on Phases 1–3.
2. Then: developer A on Phase 4 → Phase 6; developer B on Phase 5, then Phase 7 once Phase 4
   lands; developer C on Phase 10 immediately, then the fixture corpus and Phase 11's privacy
   suites.
3. Phases 8 and 9 rejoin once the adapters exist.

---

## Notes

- 128 tasks across 12 phases; every phase ends with something runnable
- Every task names a concrete path from `plan.md`'s structure decision
- FR and SC references trace each task back to `spec.md`; the coverage map is in
  [traceability.md](./traceability.md)
- `[MANUAL]` tasks are release evidence and are never required for CI to pass (FR-205)
- No task requires a credential, an authenticated service, a network fetch, or an installed
  vendor binary to run in required CI (FR-204, SC-124)
- CC Switch is an integration manager and never an agent adapter; no task writes to
  `~/.cc-switch/cc-switch.db` or any other private manager file (FR-232)
- Out-of-scope items from `spec.md` are not tasks and must not appear as "while we're here"
  work
