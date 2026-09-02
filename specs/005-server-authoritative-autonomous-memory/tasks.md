# Tasks: Server-Authoritative Autonomous Memory

**Input**: Approved artifacts in `specs/005-server-authoritative-autonomous-memory/`
**Prerequisites**: `spec.md`, `plan.md`, `research.md`, `data-model.md`, `quickstart.md`, all eight files in `contracts/`, Constitution v1.2.1, and `checklists/requirements.md`

**Tests are mandatory.** Each story begins with tests that must fail before its implementation tasks and pass at its checkpoint. Task order is dependency order unless `[P]` marks disjoint files.

## Format

`- [ ] T### [P?] [US#?] Action with exact file path`

---

## Phase 1: Setup

- [X] T001 Record the pre-Feature-005 workspace dependency manifest used by SC-737 in `tests/feature005/dependency-baseline.toml`
- [X] T002 Add shared Feature 005 PostgreSQL/SQLite fixtures, identical-UUID seed helpers, authenticated multi-account helpers, and restart injection controls in `tests/src/feature005.rs`
- [X] T003 Export the Feature 005 harness from `tests/src/lib.rs`
- [X] T004 Add only the test dependencies required by the approved Rust/PostgreSQL/Next.js stack in `tests/Cargo.toml`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Establish schemas, authority, typed boundaries, spools, complete references, authenticated command handling, and restart-safe claim primitives before story work.

- [X] T005 Implement SQLite schema v8 exactly as `data-model.md` §5—including `event_spool`, durable event/command sequences, `command_spool`, disposition counts, `authority_mode`, `retained_local`, `migration_state`, and immutable `legacy_pattern_claims`—in `crates/cairn-store/migrations/0008_safe_events.sql`
- [X] T006 Test v7→v8 migration, interrupted migration rollback, existing-row preservation, retained-local discriminator constraints, and same-owner/different-owner legacy-pattern claim uniqueness in `tests/tests/feature005_local_schema.rs` (depends on T005)
- [X] T007 Implement the complete PostgreSQL schema v4 exactly as `data-model.md` §6—including final personal/team FTS indexes and triggers, authority mode, safe events, consolidation leases/work/runs/candidates, provenance, requested/generated/transmitted/failed traces, trace-item source timestamps, delivery dedup, reports/summaries, personal-domain patterns, health, and dispositions—in `crates/cairn-server/migrations/0004_autonomous_memory.sql`
- [X] T008 Test v3→v4 migration, every foreign key/index/trigger/CHECK, final personal/team FTS ownership, trace lifecycle constraints, canonical `reference_key`, nullable project bindings, owner-only pattern columns, and server-authority initialization in `tests/tests/feature005_server_schema.rs` (depends on T007; SC-766)
- [X] T009 [P] Add server schema v4 and the settled capability advertisement without weakening older capability behavior in `crates/cairn-server/src/version.rs` (depends on T007)
- [X] T010 Define `KnowledgeRef(domain,id)`, `PatternRef(pattern_id)`, the polymorphic reference enum, canonical `reference_key`, domain/type validation, and per-domain resolution contracts in `crates/cairn-core/src/domain.rs` (FR-708c–e, FR-817–FR-826, FR-819a)
- [X] T011 [P] Define the closed, versioned 21-kind `SafeCanonicalEvent` union, typed per-kind content, provenance, dispositions, refusal vocabularies, and all numeric bounds in `crates/cairn-core/src/event.rs` (FR-734–FR-745)
- [X] T012 [P] Implement daemon-assigned UUIDv5 event, command, candidate, refusal, corroboration, and pattern identities from their approved inputs in `crates/cairn-core/src/eventid.rs` (FR-708f, FR-738, FR-796–FR-798c)
- [X] T013 Export the new domain, event, and identity APIs without changing the existing public tool surface in `crates/cairn-core/src/lib.rs` (depends on T010–T012)
- [X] T014 Test the 21-kind closed union, old seven-event compatibility, per-kind content legality, bounds, clock-independent repeated-event identity, retry stability, and full reference encoding in `crates/cairn-core/tests/feature005_events.rs` (depends on T010–T013)
- [X] T015 Extend the one canonical topic/value-key normalizer with approved separator folding while preserving dot-segment semantics and rejecting—not repairing—invalid keys in `crates/cairn-core/src/knowledge.rs` (FR-796a–d, FR-824). **Clarified during implementation:** T015 activates folding immediately while T142 rewrites existing rows only during the explicit US7 migration, so T015 must also supply the pre-cutover comparison compatibility that stops a legacy value key and its own canonical form reading as two values in the interval between them. T142 remains the permanent normalization and collision step (FR-867a); nothing here rewrites a stored key.
- [X] T016 Test at least fifty key-variant groups, invalid-key refusal, and unchanged topic dot segmentation in `crates/cairn-core/tests/feature005_keys.rs` (depends on T015; SC-745)
- [X] T017 Extend the existing single privacy validator for safe-event text, `repo_file`, patterns, and consolidation candidates while keeping it pure, deterministic, fail-closed, and structurally text-free on refusal in `crates/cairn-core/src/validate.rs` (FR-749–FR-764, FR-777–FR-777g)
- [X] T018 Test every privacy rejection class, missing-input refusal, POSIX/Windows path attacks, 1024-byte/64-segment limits, and refusal Debug/Display non-leakage in `crates/cairn-core/tests/feature005_privacy.rs` (depends on T017; SC-704, SC-705, SC-743)
- [X] T019 Extend canonical lifecycle translation so all existing seven lifecycle events remain expressible through `SafeCanonicalEvent` in `crates/cairn-core/src/lifecycle.rs` (depends on T011)
- [X] T020 Implement durable ordinal allocation, transactional identity assignment, exact-account claims, stale-claim reclaim, bounded backoff, permanent refusal, and capacity policy shared by event and command spools in `crates/cairn-store/src/spool.rs` (depends on T005, T012). **Settled during implementation:** (a) `failed` is the *retrying* state here and `refused` the terminal one — the opposite of `outbox.rs`, where `failed` is terminal, because schema v8 gives the spool a `refused` the outbox never had; (b) FR-784's "retry MUST be bounded" is read as a bound on attempts and not only on backoff, so a row that spends a fixed attempt budget becomes terminal-and-visible rather than retrying forever. The budget is counted in attempts, not elapsed time: a device that was switched off has not used any of it, because being off is not the server refusing (FR-783). It is also enforced on the **claim**, not only where a failure is recorded — a drainer that crashes after claiming never reports a failure, and every claim increments `attempts`; (c) `command_spool` is bounded by the same stated capacity as `event_spool`, and its terminal behaviour is to refuse new commands visibly, never to shed queued ones — the event spool's shedding policy is inexpressible for commands, which carry no `boundary_class` because no explicit command is droppable (`contracts/knowledge-commands.md` §4).
- [X] T021 Export spool repositories and replace insert-once personal/team replica merging with server-wins cache refresh semantics in `crates/cairn-store/src/lib.rs` (depends on T020; FR-712a)
- [X] T022 Test concurrent ordinal allocation, drain-independent identity, exact-account claims, stale reclaim, retry bounds, boundary-row protection, saturation/drop counters, and command ordering in `crates/cairn-store/tests/feature005_spool.rs` (depends on T020–T021)
- [X] T023 Add authenticated per-domain reference resolution and ownership/membership guards—including owner-only `PatternRef` resolution and withheld-not-opaque behavior—to `crates/cairn-server/src/auth.rs` (depends on T007, T010; FR-768–FR-769a, FR-834, FR-846a)
- [X] T024 Write failing generic command-boundary contract tests for intent-only payloads, credential-derived attribution, cross-member overwrite refusal, idempotent command retries, personal ownership, pattern command shape/owner-guard delegation, and atomic team transitions in `tests/tests/feature005_commands.rs` (depends on T002, T007, T023)
- [X] T025 Implement the generic post-cutover authoritative command boundary and DTO/dispatch contracts—including project, personal and team commands plus the safe pattern promote/forget command shape, but excluding the US3 pattern repository/lifecycle implementation—and reject all client-writable derived state in `crates/cairn-server/src/commands.rs` (depends on T024; FR-701, FR-712, FR-815–FR-816)
- [X] T026 Wire project, personal and team command endpoints and reuse the existing compare-and-swap ratify/retire handlers; pattern routes remain interface-only until US3 supplies their lifecycle repository in `crates/cairn-server/src/api.rs` (depends on T025; FR-825, FR-889a)
- [ ] T027 Route explicit non-pattern daemon/CLI knowledge mutations through online commands or the account-bound command spool, preserving sessionless store-scoped identity, in `crates/cairnd/src/handlers.rs` (depends on T020, T025)
- [X] T028 Write failing safe-event ingest contract tests for strict fields, refused names, bounds, UUID re-derivation, ordered vocabulary validation, session/account/project authorization, duplicate success, permanent versus transient failure, and atomic enqueue in `tests/tests/feature005_ingest.rs` (depends on T002, T007, T011, T023)
- [X] T029 Implement `POST /api/events/batch` validation and atomic `safe_events` plus `consolidation_work` persistence in `crates/cairn-server/src/events.rs` (depends on T028; FR-765–FR-780)
- [X] T030 Wire strict body limits, authentication, routing, and per-event outcomes for event ingest in `crates/cairn-server/src/main.rs` (depends on T029)
- [ ] T031 Write failing consolidation-claim tests covering concurrent workers, session ordering, done-session reopening, partial-batch immediate re-election, lease reclaim, and the exact edge cases—failures 1–4 retry, attempt-5 success is `done`, attempt-5 failure is `failed`, attempt 6 never runs, crash after start consumes the attempt, and failed work never strands its session—in `tests/tests/feature005_consolidation_claims.rs` (depends on T002, T007)
- [ ] T032 Implement one-session lease election, attempt increment at claim start, heartbeat/reclaim, success-before-fifth-failure close ordering, and lease release/reopen/close in `crates/cairn-server/src/consolidate.rs` (depends on T031; FR-793a–d, FR-808)
- [ ] T033 Start exactly one bounded in-process consolidation task with batch 200, lease five minutes, 100 ms yield, max attempts five, and pool share `min(2,floor(max_connections/5))` in `crates/cairn-server/src/main.rs` (depends on T032; FR-793a1)
- [ ] T034 Test deliberately identical `project:id-X`, `personal:id-X`, `team:id-X`, and `pattern:id-X` across candidate results, traces, delivery, summaries, and reports; test two reporters and every invalid discriminator/domain combination in `tests/tests/feature005_reference_identity.rs` (depends on T007, T010, T023; SC-766, SC-767)
- [ ] T035 Define the complete per-agent/per-event capability matrix vocabulary, evidence kind, no-evidence state and per-machine attribution in `crates/cairn-integrate/src/capability.rs`, and implement the shared authenticated bounded health/disposition report-validation and seeded-row read API primitive used by US5 and US6 in `crates/cairn-server/src/api.rs` (FR-728–FR-729, FR-838a–f, FR-851–FR-860)
- [ ] T036 Add pre-registered capture, decision/instruction, paraphrase, privacy, path, restart, and migration corpus metadata in `tests/feature005/corpora/manifest.json`
- [ ] T037 Add compile-time and schema lint tests that fail if a safe-event field reuses any sync-refused name or any new raw-transcript/raw-vendor-json storage field appears in `tests/tests/feature005_boundary_audit.rs` (depends on T007, T011; SC-730, SC-731, SC-751)
- [ ] T038 Add an authorization route inventory that fails when a new project-scoped Feature 005 endpoint lacks `require_member` or accepts identity from its body in `tests/tests/feature005_authorization_audit.rs` (depends on T023; FR-769, FR-894a)
- [ ] T039 Implement and register the reusable bounded event/command drain primitive—ordered claims, per-item outcomes, supported-version deferral, account/server binding hooks, backoff and stale-claim controls—in `crates/cairnd/src/sync.rs` and `crates/cairnd/src/main.rs`, while registering all foundation modules without changing the two-process architecture in `crates/cairn-server/src/main.rs` (depends on T009–T038)

**Checkpoint**: Both migrations apply; typed events and complete references exist; no bare UUID is cross-domain identity; event/command spools and command API are authoritative-safe; consolidation claims are restartable and fifth-attempt-correct.

---

## Phase 3: User Story 1 — Work becomes knowledge without anyone asking (Priority: P1) 🎯 MVP

**Goal**: A real no-tool coding session produces accurate, governed, provenance-bearing knowledge.

**Independent Test**: Start with zero memories, drive one supported agent through investigate/change/test-pass without Cairn tools, and resolve the resulting knowledge back to its session and safe events.

### Tests for User Story 1

- [ ] T040 [P] [US1] Write failing vendor fixtures for every declared event cell, file identity disposition, provenance field, unknown tool, subagent attribution, Claude/Codex semantic source, and OpenCode structural-only decline in `tests/tests/feature005_capture_matrix.rs` (depends on T035; SC-706, SC-707, SC-744)
- [ ] T041 [P] [US1] Write failing twenty-session decision/instruction scenarios for Claude Code and Codex CLI, including vocabulary-only token assertions and zero ungrounded prompt/assistant words, in `tests/tests/feature005_semantic_signals.rs` (depends on T036; SC-701a, SC-701b)
- [ ] T042 [P] [US1] Write failing extraction rule tests for R1–R8, no-worthy-knowledge cases, identical vendor-provenance invariance, all five kinds, source verification, and deterministic keys in `tests/tests/feature005_extraction.rs` (depends on T015, T036; FR-724, FR-795)
- [ ] T043 [P] [US1] Write failing adversarial extractor tests that attempt foreign domain/scope/owner, asserted durability/verification/supersession, invalid keys, foreign sources, and forbidden content in `tests/tests/feature005_governance.rs` (depends on T017, T036; SC-704, SC-734, SC-735, SC-741, SC-742, SC-749)
- [ ] T044 [P] [US1] Write failing idempotency/restart tests for unchanged reruns, fifty paraphrase pairs, additive evidence, stable corroboration identity, and twenty pre-registered crash points in `tests/tests/feature005_consolidation_restart.rs` (depends on T031, T036; SC-703, SC-736, SC-739)
- [ ] T045 [P] [US1] Write failing backlog isolation benchmark with 10,000 pending events and ten trials in `tests/tests/feature005_consolidation_backlog.rs` (depends on T031; SC-740, SC-748)
- [ ] T046 [US1] Replace the shared two-field payload allowlist with per-vendor typed field maps, local path relativization, redaction-before-vocabulary, deterministic signal mapping, and raw-payload disposal in `crates/cairn-integrate/src/agents/mod.rs` (depends on T040–T041)
- [ ] T047 [P] [US1] Implement Claude Code session, tool, file, test, prompt, settled assistant, and subagent capture from the approved fields while excluding `StopFailure.last_assistant_message` and streaming deltas in `crates/cairn-integrate/src/agents/claude_code.rs` (depends on T046)
- [ ] T048 [P] [US1] Implement Codex CLI session, tool, patch/file, test, prompt, nullable settled assistant, and subagent capture from the approved fields while excluding `StopFailure` and streaming display text in `crates/cairn-integrate/src/agents/codex.rs` (depends on T046)
- [ ] T049 [P] [US1] Implement OpenCode structural capture and explicit `declined_by_cairn` semantic-signal status without reading beta prompt/context surfaces in `crates/cairn-integrate/src/agents/opencode.rs` (depends on T046)
- [ ] T050 [US1] Convert approved canonical events into daemon-assigned ordinal/UUID identities, spool them transactionally, and record content-free deadline/drop/refusal dispositions in `crates/cairnd/src/capture.rs` (depends on T019–T022, T046–T049)
- [ ] T051 [US1] Preserve boundary-class replies and capture-class fail-soft deadlines across the richer event path in `crates/cairnd/src/integrations.rs` (depends on T050; FR-749a–d)
- [ ] T052 [US1] Define the replaceable extractor trait, bounded `ExtractionInput`, deterministic R1–R8 baseline, project/account scoping, no hosted-provider default, and a fail-closed disclosure/retention/training/caching/isolation compliance gate for any future hosted configuration in `crates/cairn-server/src/extract.rs` (depends on T042; FR-763a–b, FR-805a–f)
- [ ] T053 [US1] Implement fixed-order candidate governance: source/key/privacy/domain/scope/ownership/key-evidence/dedup/reinforcement/conflict/no-supersession/verification/team-proposal gates in `crates/cairn-server/src/consolidate.rs` (depends on T032, T043, T052; FR-794–FR-812)
- [ ] T054 [US1] Persist deterministic candidates, additive source events, corroboration endpoints excluded from recall/counts, fixed relation bases, run counters, refusals without text, and provenance atomically in `crates/cairn-server/src/consolidate.rs` (depends on T044, T053; FR-797–FR-808)
- [ ] T055 [US1] Implement consolidation's project/personal/team exact normalized-key and text query paths using the final schema-v4 FTS indexes from T007, without changing migration schema or adding embeddings, in `crates/cairn-server/src/consolidate.rs` (depends on T007, T015, T053; FR-806)
- [ ] T056 [US1] Record consolidation throughput, backlog depth/oldest/failures, run outcomes, and ingest-independent failure metrics in `crates/cairn-server/src/consolidate.rs` (depends on T045, T054; FR-793c, FR-807, FR-813–FR-814)
- [ ] T057 [US1] Persist vendor provenance and server-bound project/account/session attribution without consulting provenance in extraction or ranking in `crates/cairn-server/src/events.rs` (depends on T029, T042; FR-723–FR-725, FR-779)
- [ ] T058 [US1] Configure and use the T039 shared drain primitive for normal capture, preserving session-sequence order and accepted/duplicate/refused/version-deferred handling, in `crates/cairnd/src/sync.rs` (depends on T039, T050; FR-770–FR-775)
- [ ] T059 [US1] Surface capture/consolidation dispositions and fixed refusal reasons through daemon status data in `crates/cairnd/src/handlers.rs` (depends on T050, T056)
- [ ] T060 [US1] Make all T040 capture-matrix assertions pass without narrowing any declared agent/event cell in `tests/tests/feature005_capture_matrix.rs` (depends on T046–T051, T057–T059)
- [ ] T061 [US1] Make all T041 semantic-signal scenarios pass with 14/20 matching decision/convention outcomes and no prose leakage in `tests/tests/feature005_semantic_signals.rs` (depends on T046–T053)
- [ ] T062 [US1] Make all T043 governance and privacy adversarial assertions pass with identical outcomes for hostile versus empty extractor output in `tests/tests/feature005_governance.rs` (depends on T053–T054)
- [ ] T063 [US1] Make all T044 restart/idempotency/paraphrase assertions pass with zero duplicate effects in `tests/tests/feature005_consolidation_restart.rs` (depends on T054–T056)
- [ ] T064 [US1] Implement the story-level no-tool real-repository acceptance test for Claude Code, Codex CLI, and OpenCode in `tests/tests/feature005_us1_autonomous_learning.rs` (depends on T060–T063; SC-701, SC-702)

**Checkpoint**: User Story 1 passes independently; accepted safe activity becomes governed durable knowledge without a tool call.

---

## Phase 4: User Story 2 — A second session starts already knowing (Priority: P2)

**Goal**: Claude Code and Codex CLI receive deterministic bounded context automatically, with complete traces and truthful delivery status.

**Independent Test**: Seed authorized records directly through commands, open a related session, and verify delivery without depending on Story 1's capture path.

### Tests for User Story 2

- [ ] T065 [P] [US2] Write failing selection tests for domain-separated ordering, project reserve/non-displacement, stable explanations, full versus 25% incremental budgets, unchanged-item dedup, changed-item re-entry, and identical-UUID coexistence in `tests/tests/feature005_retrieval.rs` (depends on T025, T034; SC-709–SC-711, SC-767)
- [ ] T066 [P] [US2] Write failing trace/privacy tests proving every authenticated retrieval creates a `requested` trace, generation failure becomes `failed` without `delivered_context`, and complete considered/selected refs, 90-day retention, no briefing text, owner withholding, dense post-filter ranks, scoped budgets, and membership refusal hold in `tests/tests/feature005_retrieval_traces.rs` (depends on T023, T034; SC-729, SC-761)
- [ ] T067 [P] [US2] Write failing delivery tests for generation failure, generated-plus-transmission failure, generated-plus-transmission success, duplicate outcome idempotency, foreign account/session/trace refusal, Claude/Codex session-open and prompt-time delivery, compact-triggered reopen, no post-compact return, OpenCode decline, and acknowledgement remaining `unavailable / no evidence` in `tests/tests/feature005_delivery.rs` (depends on T023, T035; SC-708, SC-712, SC-729)
- [ ] T068 [P] [US2] Write failing cache tests for 64 KiB/200-session bounds, refill, stale labels, account partition/invalidation, and fresh-unavailable behavior in `tests/tests/feature005_briefing_cache.rs` (depends on T022; SC-718)
- [ ] T069 [US2] Implement server-side authorized per-domain retrieval, section-order ranking, project reserve, pattern general-pool treatment, full/incremental budgets, complete-reference dedup, and four deterministic degradation levels in `crates/cairn-server/src/retrieve.rs` (depends on T065; FR-817–FR-838)
- [ ] T070 [US2] Persist a trace before each authenticated retrieval generation, transition it to `generated` or generation-stage `failed`, and implement idempotent transactional transmission outcomes where only `generated → transmitted` upserts selected `reference_key` rows into `delivered_context`, while `generated → failed` writes no delivery rows; include bounded retention sweeps in `crates/cairn-server/src/retrieve.rs` (depends on T066–T069; FR-839–FR-850)
- [ ] T071 [US2] Add authenticated `POST /api/retrieve` plus `POST /api/retrieval-traces/{trace_id}/transmission`; bind retrieve identity from the session, bind outcome account/project/session from the stored trace, accept only bounded outcome/reason fields, refuse foreign/conflicting reports, and expose no caller-selectable authority or acknowledgement in `crates/cairn-server/src/api.rs` (depends on T069–T070)
- [ ] T072 [US2] Implement daemon delivery orchestration, deadline degradation, and retry-safe reporting of the actual post-response hook transmission outcome by server-issued `trace_id`, plus the account-bound bounded outage cache, in `crates/cairnd/src/deliver.rs`, then register it in `crates/cairnd/src/main.rs` (depends on T068, T071)
- [ ] T073 [US2] Return automatic context at Claude Code session-open and prompt-submit hooks, including session-open `compact` restoration, in `crates/cairn/src/hook.rs` (depends on T072)
- [ ] T074 [US2] Return automatic context at Codex CLI session-open and prompt-submit hooks, including session-open `compact` restoration, in `crates/cairn-integrate/src/agents/codex.rs` (depends on T072)
- [ ] T075 [US2] Keep OpenCode automatic delivery absent while exposing manual MCP context/search and explicit `declined_by_cairn` capability in `crates/cairn-integrate/src/agents/opencode.rs` (depends on T035, T072)
- [ ] T076 [US2] Preserve manual `cairn_context`, `cairn_search`, and `cairn_remember` overrides against the server path in `crates/cairnd/src/handlers.rs` (depends on T071–T072; FR-831)
- [ ] T077 [US2] Make T065 selection/budget/dedup tests pass, including personal `id-X` not suppressing team `id-X`, in `tests/tests/feature005_retrieval.rs` (depends on T069–T071)
- [ ] T078 [US2] Make T066 trace/privacy tests pass, including generation failure leaving one failed trace and zero delivery rows, without opaque references, rank gaps, cross-account patterns, or persisted briefing text in `tests/tests/feature005_retrieval_traces.rs` (depends on T070–T071)
- [ ] T079 [US2] Make T067 transmission-outcome and delivery-point tests pass with one durable effect per duplicate report, no delivery row on either failure path, refusal of foreign trace use, and acknowledgement always `unavailable / no evidence` in `tests/tests/feature005_delivery.rs` (depends on T071–T075)
- [ ] T080 [US2] Make T068 outage-cache account-isolation tests pass in `tests/tests/feature005_briefing_cache.rs` (depends on T072)
- [ ] T081 [US2] Add deterministic retrieval latency/degradation measurements for session-open and prompt-time deadlines in `tests/tests/feature005_retrieval_performance.rs` (depends on T069–T075; FR-835–FR-836)
- [ ] T082 [US2] Implement the story-level second-session no-tool acceptance test for Claude Code and Codex CLI in `tests/tests/feature005_us2_automatic_recall.rs` (depends on T077–T081)

**Checkpoint**: User Story 2 passes independently from command-seeded knowledge; context is bounded, deduplicated, traced, authorized, and delivered only on settled vendor surfaces.

---

## Phase 5: User Story 3 — Losing a machine does not lose knowledge (Priority: P3)

**Goal**: Server-accepted project, personal, team, and pattern records survive local-store loss; losses are named honestly.

**Independent Test**: Seed and accept all durable domains plus a pattern, delete/recreate SQLite, and verify server restoration plus an exact loss inventory.

- [ ] T083 [P] [US3] Write failing local-loss tests for project/personal/team/pattern restoration from directly seeded server records, cache-empty/stale reporting, and exact lost categories in `tests/tests/feature005_local_loss.rs` (depends on T034; SC-713, SC-714, SC-738)
- [ ] T084 [P] [US3] Write failing owner-only pattern lifecycle tests that compile against only T025's command shape—not an implemented repository—for safe promotion, deterministic duplicate identity, trust narrowing, list/forget tombstone behavior, retrieval/traces/API isolation, and team widening only through a separate proposal in `tests/tests/feature005_patterns.rs` (depends on T025, T034; SC-760–SC-762)
- [ ] T085 [US3] Implement and wire the actual safe personal-domain pattern promote/list/forget repository lifecycle behind T025's generic boundary, with credential-bound owner, deterministic content identity, `sanitized` server trust, tombstones, and no local-only fields, in `crates/cairn-server/src/commands.rs` and `crates/cairn-server/src/api.rs` (depends on T084)
- [ ] T086 [US3] Implement server-wins cache refill and invalidation for project/personal/team knowledge and owner-only patterns in `crates/cairn-store/src/global.rs` (depends on T021, T085; FR-703–FR-710a)
- [ ] T087 [US3] Keep pattern applications, evidence, verification runs, observations, checkpoints, and diagnostics local-only while exposing their durability class in `crates/cairn-store/src/diag.rs` (depends on T005; FR-705–FR-708)
- [ ] T088 [US3] Route pattern promotion/forget and explicit personal/team/project reads through the server-authoritative command/read boundary in `crates/cairnd/src/patterns.rs` (depends on T085–T087)
- [ ] T089 [US3] Implement local-store loss inventory, cache refill status, and the local-only durability warning at the point that choice is offered in `crates/cairnd/src/handlers.rs` (depends on T086–T087)
- [ ] T090 [US3] Make T084 pattern lifecycle/idempotency/privacy tests pass, including another account sharing the project, in `tests/tests/feature005_patterns.rs` (depends on T085, T088)
- [ ] T091 [US3] Make T083 local-loss tests pass and prove only server-unaccepted/local-machine categories disappear in `tests/tests/feature005_local_loss.rs` (depends on T086–T089)
- [ ] T092 [US3] Implement the story-level destroy/recreate/restore acceptance test in `tests/tests/feature005_us3_durability.rs` (depends on T090–T091)

**Checkpoint**: User Story 3 passes independently; server-accepted knowledge survives local deletion and exclusions are explicit.

---

## Phase 6: User Story 4 — The server goes away and the agent keeps working (Priority: P4)

**Goal**: Capture and explicit commands fail soft, queue safely, replay once, and never create local authority.

**Independent Test**: Seed a session, remove server connectivity, continue capture/commands/retrieval, restore connectivity, and inspect spools plus canonical rows.

- [ ] T093 [P] [US4] Write failing outage tests for agent deadlines, event spooling/replay, duplicate responses as success, zero local durable knowledge, and backlog visibility in `tests/tests/feature005_outage.rs` (depends on T022, T029; SC-715–SC-717)
- [ ] T094 [P] [US4] Write failing credential/server-switch tests for event and command account binding and exact server-instance refusal in `tests/tests/feature005_identity_outage.rs` (depends on T022; FR-790–FR-791)
- [ ] T095 [P] [US4] Write failing capacity tests for oldest capture-class shedding, boundary-row preservation, fully-boundary saturation, content-free drop records, and automatic recovery in `tests/tests/feature005_spool_capacity.rs` (depends on T022; FR-785)
- [ ] T096 [US4] Extend the T039 shared event/command drain primitive with bounded outage loops, exponential backoff, reconnect scheduling and stale-claim recovery in `crates/cairnd/src/sync.rs` (depends on T039, T093)
- [ ] T097 [US4] Queue explicit remember/supersede/relate/pattern/verification commands as accepted-for-delivery—not durable—when unreachable in `crates/cairnd/src/handlers.rs` (depends on T027, T096; FR-815a)
- [ ] T098 [US4] Enforce spool capacity, boundary protection, saturation, and disposition accounting atomically in `crates/cairn-store/src/spool.rs` (depends on T095)
- [ ] T099 [US4] Preserve exact credential and server-instance bindings across drain, sign-out, and re-authentication in `crates/cairnd/src/sync.rs` (depends on T094, T096)
- [ ] T100 [US4] Surface event/command depth, oldest entry, retry blocker, saturation, permanent refusals, and fresh-knowledge-unavailable state in `crates/cairnd/src/handlers.rs` (depends on T097–T099; FR-788–FR-792)
- [ ] T101 [US4] Make T093 outage/replay/no-local-authority tests pass in `tests/tests/feature005_outage.rs` (depends on T096–T100)
- [ ] T102 [US4] Make T094 credential/cache/server-instance isolation tests pass in `tests/tests/feature005_identity_outage.rs` (depends on T099–T100)
- [ ] T103 [US4] Make T095 capacity and content-free drop tests pass in `tests/tests/feature005_spool_capacity.rs` (depends on T098–T100; SC-752, SC-753)
- [ ] T104 [US4] Add repeated response-loss replay trials proving one canonical event, one consolidation input, and one durable effect in `tests/tests/feature005_replay_idempotency.rs` (depends on T096, T101; SC-716)
- [ ] T105 [US4] Implement the story-level mid-session outage/recovery acceptance test in `tests/tests/feature005_us4_fail_soft.rs` (depends on T101–T104)

**Checkpoint**: User Story 4 passes independently; outage changes freshness and queue state, never agent usability or knowledge authority.

---

## Phase 7: User Story 5 — A user can see what Cairn learned, and why (Priority: P5)

**Goal**: Authenticated web/API views reconstruct the lifecycle without exposing local-only or cross-account material.

**Independent Test**: Seed canonical events, a run, candidate, knowledge, relations, verification, and retrieval directly, then reconstruct the path using only web APIs/UI.

- [ ] T106 [P] [US5] Write failing API tests for funnel stages/zero-vs-null, activity default/full sets, memory details, runs, traces, health, pagination, and every project membership/admin refusal in `tests/tests/feature005_control_plane_api.rs` (depends on T038; SC-727, SC-728)
- [ ] T107 [P] [US5] Write failing browser tests for the complete session→event→run→candidate→knowledge→retrieval path, local-only notices, domain separation, pattern owner privacy, and team compare-and-swap actions in `web/e2e/feature005-control-plane.spec.ts`, with canonical seeded fixtures in `web/e2e/seed.ts` (depends on T106)
- [ ] T108 [US5] Implement bounded membership-guarded funnel, activity, consolidation-run, retrieval-trace and system-health read handlers, consuming T035's shared integration-health read API rather than owning it, in `crates/cairn-server/src/api.rs` (depends on T035, T106; FR-879–FR-882, FR-886–FR-887, FR-891, FR-894–FR-895)
- [ ] T109 [US5] Extend memory list/detail APIs with origin, provenance, evidence summary without content, verification, relations, reinforcement, and retrieval usage in `crates/cairn-server/src/api.rs` (depends on T106; FR-883–FR-885)
- [ ] T110 [US5] Implement owner-scoped personal/pattern feeds and team visibility rules without cross-account enumeration in `crates/cairn-server/src/global.rs` (depends on T023, T106; FR-888, FR-892–FR-893)
- [ ] T111 [US5] Extend typed API clients for all Feature 005 control-plane shapes, nullable counts, complete references, and withheld fields in `web/lib/api.ts` (depends on T108–T110)
- [ ] T112 [P] [US5] Extend the project dashboard with the twelve-stage memory funnel and zero/unavailable distinction in `web/app/(app)/projects/[id]/page.tsx` (depends on T111)
- [ ] T113 [P] [US5] Implement the semantic activity feed with declared default kinds and explicit show-all control in `web/app/(app)/projects/[id]/activity/page.tsx` (depends on T111)
- [ ] T114 [P] [US5] Implement the bounded memory explorer in `web/app/(app)/projects/[id]/memory/page.tsx` (depends on T111)
- [ ] T115 [P] [US5] Implement memory detail with provenance, evidence-local notice, verification, relations, reinforcement, origin, and retrieval usage in `web/app/(app)/projects/[id]/memory/[memoryId]/page.tsx` (depends on T111)
- [ ] T116 [P] [US5] Implement retrieval trace list and filtering in `web/app/(app)/projects/[id]/retrievals/page.tsx` (depends on T111)
- [ ] T117 [P] [US5] Implement retrieval detail without briefing text, preserving complete refs and scoped budgets in `web/app/(app)/projects/[id]/retrievals/[traceId]/page.tsx` (depends on T111)
- [ ] T118 [P] [US5] Implement per-agent/per-machine integration health with evidence-kind, staleness, decline, failure, and no-evidence distinctions in `web/app/(app)/projects/[id]/agents/page.tsx` (depends on T111)
- [ ] T119 [P] [US5] Implement visibly separate project/personal/pattern/team panels with owner-only patterns in `web/app/(app)/projects/[id]/domains/page.tsx` (depends on T111)
- [ ] T120 [P] [US5] Implement admin-only team proposal review using only existing atomic ratify/retire routes in `web/app/(app)/team/page.tsx` (depends on T111)
- [ ] T121 [P] [US5] Implement admin-only ingest/consolidation/retrieval system health in `web/app/(app)/system/page.tsx` (depends on T111)
- [ ] T122 [P] [US5] Implement bounded admin user management using existing endpoints in `web/app/(app)/admin/users/page.tsx` (depends on T111)
- [ ] T123 [US5] Add role/feature-aware navigation for activity, memory, retrievals, agents, domains, team, system, and admin users in `web/components/app-sidebar.tsx` (depends on T112–T122)
- [ ] T124 [US5] Make API and browser acceptance paths pass without database/log access in `web/e2e/feature005-control-plane.spec.ts` (depends on T108–T123; SC-727, SC-728)

**Checkpoint**: User Story 5 passes independently from seeded server data; the web tells the whole authorized story and never becomes an authority boundary.

---

## Phase 8: User Story 6 — Status tells the truth, including when it does not know (Priority: P6)

**Goal**: Verification and integration health report only evidence actually established.

**Independent Test**: Submit reports through both authenticated routes, exercise/no-op integration capabilities, and verify derived authority/state and no-evidence distinctions.

- [ ] T125 [P] [US6] Write failing verification tests for both HTTP routes always assigning `remote_attested`, authority/report-id/refused-field rejection, server-only `cairn`, no baseline `remote_cairn`, derivation transitions, and duplicate identity including account in `tests/tests/feature005_verification_authority.rs` (depends on T034; SC-765, SC-767)
- [ ] T126 [P] [US6] Write failing summary tests for project columns versus non-project `knowledge_verification`, same-UUID personal/team/pattern coexistence, raw-evidence absence, and authorization in `tests/tests/feature005_verification_summaries.rs` (depends on T034; SC-766, SC-767)
- [ ] T127 [P] [US6] Write failing health tests for configured-vs-runtime, every matrix cell, failure stage, stale evidence, per-machine attribution, OpenCode declines, and receipt no-evidence in `tests/tests/feature005_health.rs` (depends on T035; SC-724–SC-726)
- [ ] T128 [US6] Implement authenticated run/attestation report ingestion, server-assigned report IDs/authority, full-reference/account natural-key idempotency, and same-transaction state derivation in `crates/cairn-server/src/verifysummary.rs` (depends on T125–T126; FR-811a–d, FR-811h–i)
- [ ] T129 [US6] Wire `/api/verification/runs` and `/api/verification/attestations` without any caller-selectable authority in `crates/cairn-server/src/api.rs` (depends on T128)
- [ ] T130 [US6] Queue privacy-safe verification reports through the foundational T020/T025 command-spool boundary while keeping raw evidence facts/runs local in `crates/cairnd/src/verify.rs` (depends on T020, T025, T129; FR-707, FR-811c)
- [ ] T131 [US6] Derive per-agent/per-capability/per-machine health from recorded introspection/observation evidence and explicit failure dispositions in `crates/cairn-integrate/src/capability.rs` (depends on T127)
- [ ] T132 [US6] Persist authenticated health/disposition reports and implement stale/no-evidence derivation behind T035's shared server API boundary in `crates/cairn-server/src/api.rs` (depends on T035, T131)
- [ ] T133 [US6] Make T125 authority/idempotency tests pass, including two accounts reporting one project/team ref without collision in `tests/tests/feature005_verification_authority.rs` (depends on T128–T130)
- [ ] T134 [US6] Make T126 full-reference summary tests pass, including PatternRef insertion with null reference-domain slot and personal-domain owner resolution in `tests/tests/feature005_verification_summaries.rs` (depends on T128–T129)
- [ ] T135 [US6] Make T127 health truthfulness tests pass without converting no evidence into vendor absence or success in `tests/tests/feature005_health.rs` (depends on T131–T132)
- [ ] T136 [US6] Add an explicit hostile-route-choice regression proving the stronger-looking URL cannot create `remote_cairn` or `cairn` in `tests/tests/feature005_verification_authority.rs` (depends on T133; SC-765)
- [ ] T137 [US6] Implement the story-level configure-then-exercise status acceptance test in `tests/tests/feature005_us6_truthful_status.rs` (depends on T133–T136)

**Checkpoint**: User Story 6 passes independently; route names and caller payloads cannot manufacture provenance or health.

---

## Phase 9: User Story 7 — An existing installation migrates without losing anything (Priority: P7)

**Goal**: A populated Feature 004 store migrates resumably only after canonical possession, with explicit legacy-pattern ownership and safe cutover.

**Independent Test**: Build a real schema-v7 store with every record/outbox/verification/pattern case, interrupt every phase, switch credentials after a pattern claim, and compare every record after retry.

- [ ] T138 [P] [US7] Write failing populated-v7 migration tests for inspect/drain/possession/switch/demote ordering, every drained reference shape, blocked rows, retained-local records, and record-level preservation in `tests/tests/feature005_migration.rs` (depends on T006, T008; SC-719–SC-723)
- [ ] T139 [P] [US7] Write failing legacy-pattern ownership tests for explicit claim, persisted owner/content-key/pattern-id before delivery, unclaimed retention, credential switch, other-owner refusal, and repeated same-owner idempotency in `tests/tests/feature005_pattern_migration.rs` (depends on T006, T084; SC-764)
- [ ] T140 [P] [US7] Write failing migration restart/key tests for interruption at every phase, rechecked possession at demotion, normalized legacy keys, surfaced collisions, and zero duplicate rows in `tests/tests/feature005_migration_restart.rs` (depends on T016, T138; SC-721, SC-750)
- [ ] T141 [US7] Implement the exact per-author legacy-row eligibility and persisted-claimant pattern eligibility primitives used by migration drain in `crates/cairnd/src/sync.rs` (depends on T138–T139; FR-864a, FR-867b)
- [ ] T142 [US7] Implement legacy topic/value-key normalization through the shared normalizer and collision recording through existing conflict machinery in `crates/cairnd/src/migrate005.rs` (depends on T015, T140; FR-867a)
- [ ] T143 [US7] Orchestrate the completed T141 eligibility and T142 normalization primitives through a resumable migration state-machine core with injected retained-store and remote-operation interfaces—inspect, explicit pattern claim, drain, possession, authority-switch request, possession recheck, demotion and retained retry—without claiming retained-store, remote-endpoint or cutover-transaction implementation, in `crates/cairnd/src/migrate005.rs`, then register it in `crates/cairnd/src/main.rs` (depends on T138–T142; FR-861–FR-878)
- [ ] T144 [US7] Implement retained-local and immutable legacy-pattern-claim repositories with canonical dedupe keys in `crates/cairn-store/src/migrate.rs` (depends on T006, T139)
- [ ] T145 [US7] Implement migration registration/token-scoped legacy drain and bounded three-result possession endpoints for KnowledgeRef, PatternRef, and RelationRef in `crates/cairn-server/src/api.rs` (depends on T023, T025, T138)
- [ ] T146 [P] [US7] Write failing cutover compatibility tests for pre-cutover migration, post-cutover permanent `upgrade_required`, untouched legacy local data, ordinary local operation, upgraded-client retry stop, and server-instance binding in `tests/tests/feature005_cutover.rs` (depends on T009, T138; SC-746, SC-747)
- [ ] T147 [P] [US7] Write failing cutover verification tests for server demotion/audit counts, cleared unsupported values, untouched client states, and changes-feed omission in `tests/tests/feature005_verification_cutover.rs` (depends on T126; SC-763)
- [ ] T148 [US7] Implement admin compare-and-swap cutover, authority advertisement, and server-wide unsubstantiated verification demotion/audit in one transaction in `crates/cairn-server/src/api.rs` (depends on T145, T147; FR-811e–g, FR-876)
- [ ] T149 [US7] Refuse post-cutover knowledge-bearing sync shapes with permanent `upgrade_required` while preserving non-knowledge sync and all read feeds in `crates/cairn-server/src/sync.rs` (depends on T146, T148; FR-876a–e, FR-877)
- [ ] T150 [US7] Stop emitting server verification objects on the post-cutover changes feed so local run-derived state is never demoted remotely in `crates/cairn-server/src/sync.rs` (depends on T147, T148)
- [ ] T151 [US7] Add `cairn migrate --inspect|--claim-patterns|--run|--status|--retry-retained` and explicit upgrade-required rendering in `crates/cairn/src/main.rs` (depends on T143–T150)
- [ ] T152 [US7] Make populated-v7 migration and individual blocked/retained reporting tests pass in `tests/tests/feature005_migration.rs` (depends on T143–T151)
- [ ] T153 [US7] Make credential-switch pattern ownership and repeated-claim tests pass with no second owner or pattern row in `tests/tests/feature005_pattern_migration.rs` (depends on T143–T151)
- [ ] T154 [US7] Make restart, key normalization, cutover, legacy-client, and verification-demotion tests pass in `tests/tests/feature005_migration_restart.rs` (depends on T143–T151)
- [ ] T155 [US7] Implement the story-level populated Feature 004 migration acceptance test in `tests/tests/feature005_us7_migration.rs` (depends on T152–T154; FR-878)

**Checkpoint**: User Story 7 passes independently on a populated v7 store; migration is explicit, resumable, owner-safe, possession-gated, and cutover-safe.

---

## Phase 10: Polish & Cross-Cutting Validation

- [ ] T156 [P] Run the complete five-phase real-repository acceptance scenario with no Cairn tool in phases 1–2 and machine/server loss in phases 4–5 in `tests/tests/feature005_end_to_end.rs` (depends on T064, T082, T092, T105, T124, T137, T155)
- [ ] T157 [P] Execute and record at least ten real-repository trials per supported agent/capability and the human accuracy rubric results in `tests/feature005/acceptance-results.md` (depends on T156; SC-701, SC-708, SC-715)
- [ ] T158 [P] Inspect every centrally persisted table after the adversarial corpus and assert zero transcripts, raw tool output, credentials, absolute paths, arbitrary vendor JSON, refused candidate text, or raw evidence in `tests/tests/feature005_central_privacy.rs` (depends on T062, T133; SC-730, SC-741)
- [ ] T159 [P] Compare the final dependency manifest to the Feature 005 baseline and fail on a datastore, broker, worker-platform, graph-database, or mandatory-embedding addition in `tests/tests/feature005_architecture.rs` (depends on T001; SC-737)
- [ ] T160 [P] Re-run capture deadline, session-open/prompt retrieval deadline, and 10,000-event backlog benchmarks with stored measurements in `tests/tests/feature005_performance.rs` (depends on T045, T081, T103; SC-715, SC-740, SC-752)
- [ ] T161 [P] Verify all new project reads refuse non-members, all owner reads exclude other accounts, all admin reads refuse members, and no API relies on web filtering in `tests/tests/feature005_authorization_audit.rs` (depends on T038, T108–T110, T145)
- [ ] T162 [P] Verify every duplicated schema/key/reference definition matches `data-model.md`, including complete `reference_key` identity and structural discriminator checks, in `tests/tests/feature005_contract_consistency.rs` (depends on T034, T134)
- [ ] T163 [P] Verify all 272 FR and 63 SC identifiers are unique and represented in the traceability index of this file in `tests/tests/feature005_traceability.rs`
- [ ] T164 Re-run the install/link/capture/consolidate/retrieve/outage/local-loss/migrate commands and expected observations from `specs/005-server-authoritative-autonomous-memory/quickstart.md` (depends on T156)
- [ ] T165 Update operator/user documentation for server authority, queued-not-durable commands, exact local-loss categories, OpenCode capture-only status, migration, privacy, and truthful verification in `README.md` (depends on T156–T164)

---

## Dependencies & Execution Order

### Phase dependencies

- Setup (T001–T004) has no feature dependency.
- Foundations (T005–T039) depends on Setup and blocks every user story.
- US1 (T040–T064) depends only on Foundations.
- US2 (T065–T082) depends only on Foundations; its independent test seeds knowledge through T025 rather than requiring US1.
- US3 (T083–T092) depends only on Foundations; its independent test seeds canonical server records directly and implements its own cache-refill path.
- US4 (T093–T105) depends only on Foundations; T039 supplies the shared drain primitive, and US4 extends it for outage/recovery without requiring US1's normal-capture configuration. Its independent outage path exercises fresh-unavailable behavior, while cached briefing behavior remains independently covered in US2.
- US5 (T106–T124) depends on Foundations and fixed read shapes; seeded server data makes it independent of capture/consolidation execution.
- US6 (T125–T137) depends only on Foundations and uses T035's shared health API with server-seeded records; US5 consumes that boundary, while US6 does not depend on US5 or web UI.
- US7 (T138–T155) follows US1–US6 because its possession/cutover path must migrate and verify every canonical surface those phases establish, matching `plan.md` phase ordering.
- Polish (T156–T165) depends on all seven story checkpoints.

### Within each story

1. Author the listed failing tests and fixed corpora.
2. Implement only the settled contract needed to make them pass.
3. Re-run the story tests and its independent acceptance test.
4. Do not begin US7 cutover work until canonical event, command, retrieval, pattern, verification, and health surfaces are present.

### Parallel opportunities

- `[P]` tasks touch disjoint files and may run concurrently only after their stated dependencies.
- Within US1, vendor adapters T047–T049 are parallel after T046; test corpora T040–T045 are parallel after Foundations.
- Within US5, pages T112–T122 are parallel after the typed API client T111.
- Test implementation and production implementation are intentionally not parallel when the production task depends on the failing test.

---

## Traceability Index

| Requirement block | Primary tasks |
|---|---|
| FR-701–FR-712a storage authority, durability, personal-domain patterns, commands/cache | T005–T027, T083–T092, T138–T155 |
| FR-717–FR-730 vendor-native capture and provenance | T035, T040–T051, T057, T060–T064 |
| FR-734–FR-745 safe canonical event model | T011–T014, T019, T028–T030, T037 |
| FR-749–FR-780 privacy and safe-event ingest | T017–T018, T028–T030, T037, T041, T043, T050–T058, T158 |
| FR-781–FR-792 edge spool/outage behavior | T020–T022, T068, T093–T105 |
| FR-793–FR-816 consolidation, extraction, verification restraint, explicit commands | T012, T015–T016, T025–T033, T042–T064, T125–T136 |
| FR-817–FR-826 domain preservation and project precedence | T010, T023–T026, T034, T053, T065–T082, T084–T090 |
| FR-827–FR-850 and FR-838a–FR-838f retrieval, agent delivery, traces | T035, T065–T082, T106–T124 |
| FR-851–FR-860 integration health | T035, T059, T106, T118, T125–T137 |
| FR-861–FR-878 migration and cutover | T005–T009, T138–T155 |
| FR-879–FR-895 web control plane | T038, T106–T124, T161 |
| FR-901–FR-905 optional bounded graph | No graph task selected: FR-901 is MAY and explicitly makes FR-902–FR-905 inert when no graph is built; required relation visibility is delivered by T109/T115 over existing relations. |
| SC-701–SC-707 autonomous capture/knowledge | T040–T064, T156–T158 |
| SC-708–SC-712 automatic delivery | T065–T082, T157 |
| SC-713–SC-718 durability/outage | T068, T083–T105, T156 |
| SC-719–SC-723 migration safety | T138–T155 |
| SC-724–SC-729 health/web/traces | T066–T067, T106–T137 |
| SC-730–SC-737 privacy/governance/architecture | T037, T042–T063, T158–T159 |
| SC-738, SC-760–SC-767 patterns, migration ownership, verification authority, polymorphic identity | T008, T034, T083–T090, T125–T155, T162 |
| SC-739–SC-753 restart, backlog, adversarial extraction/path/key/cutover/deadline | T016, T018, T031–T045, T093–T105, T138–T160 |

<!-- Mechanical coverage inventory. Keep exact identifiers so T163 can compare sets. -->
<!-- FR: FR-701 FR-702 FR-703 FR-704 FR-705 FR-706 FR-707 FR-708 FR-708a FR-708b FR-708c FR-708d FR-708e FR-708f FR-708g FR-709 FR-710 FR-710a FR-711 FR-712 FR-712a FR-717 FR-718 FR-719 FR-720 FR-721 FR-722 FR-723 FR-724 FR-725 FR-726 FR-727 FR-727a FR-727b FR-727c FR-727d FR-727e FR-728 FR-729 FR-730 FR-734 FR-735 FR-736 FR-737 FR-738 FR-739 FR-740 FR-741 FR-742 FR-743 FR-744 FR-745 FR-749 FR-749a FR-749b FR-749c FR-749d FR-750 FR-751 FR-752 FR-753 FR-754 FR-755 FR-756 FR-757 FR-758 FR-759 FR-760 FR-761 FR-762 FR-763 FR-763a FR-763b FR-764 FR-765 FR-766 FR-767 FR-768 FR-769 FR-769a FR-770 FR-771 FR-772 FR-773 FR-774 FR-775 FR-776 FR-777 FR-777a FR-777a1 FR-777b FR-777c FR-777d FR-777e FR-777f FR-777g FR-778 FR-779 FR-780 FR-781 FR-782 FR-783 FR-784 FR-785 FR-786 FR-787 FR-788 FR-789 FR-790 FR-790a FR-791 FR-792 FR-793 FR-793a FR-793a1 FR-793b FR-793c FR-793d FR-794 FR-795 FR-796 FR-796d FR-796a FR-796b FR-796c FR-797 FR-798 FR-798a FR-798b FR-798c FR-799 FR-800 FR-801 FR-801a FR-802 FR-803 FR-804 FR-804a FR-805 FR-805a FR-805a1 FR-805b FR-805c FR-805d FR-805e FR-805f FR-806 FR-807 FR-808 FR-809 FR-809a FR-810 FR-810a FR-811 FR-811a FR-811b FR-811c FR-811d FR-811e FR-811f FR-811g FR-811h FR-811i FR-812 FR-813 FR-814 FR-815 FR-815a FR-816 FR-817 FR-818 FR-819 FR-819a FR-820 FR-821 FR-822 FR-823 FR-824 FR-825 FR-826 FR-838a FR-838b FR-838c FR-838d FR-838e FR-838f FR-827 FR-828 FR-829 FR-830 FR-831 FR-832 FR-833 FR-834 FR-835 FR-836 FR-837 FR-838 FR-839 FR-840 FR-841 FR-842 FR-843 FR-844 FR-845 FR-846 FR-846a FR-847 FR-848 FR-849 FR-850 FR-851 FR-852 FR-853 FR-854 FR-855 FR-856 FR-857 FR-858 FR-859 FR-860 FR-861 FR-862 FR-863 FR-864 FR-864a FR-865 FR-866 FR-867 FR-867a FR-867b FR-868 FR-869 FR-870 FR-871 FR-872 FR-873 FR-874 FR-875 FR-876 FR-876a FR-876b FR-876b1 FR-876c FR-876d FR-876e FR-877 FR-878 FR-879 FR-880 FR-881 FR-882 FR-883 FR-884 FR-885 FR-886 FR-887 FR-888 FR-889 FR-889a FR-890 FR-891 FR-892 FR-893 FR-894 FR-894a FR-895 FR-901 FR-902 FR-903 FR-904 FR-905 -->
<!-- SC: SC-701 SC-701a SC-701b SC-702 SC-703 SC-704 SC-705 SC-706 SC-707 SC-708 SC-709 SC-710 SC-711 SC-712 SC-713 SC-714 SC-715 SC-716 SC-717 SC-718 SC-719 SC-720 SC-721 SC-722 SC-723 SC-724 SC-725 SC-726 SC-727 SC-728 SC-729 SC-730 SC-731 SC-732 SC-733 SC-734 SC-735 SC-736 SC-737 SC-738 SC-760 SC-761 SC-762 SC-763 SC-764 SC-765 SC-766 SC-767 SC-739 SC-740 SC-741 SC-742 SC-743 SC-744 SC-745 SC-746 SC-747 SC-748 SC-749 SC-750 SC-751 SC-752 SC-753 -->

---

## Implementation Strategy

### MVP first

1. Complete Setup and Foundations.
2. Complete US1 and prove autonomous knowledge creation.
3. Stop and validate US1 independently before adding recall.

### Incremental delivery

1. US1 creates governed knowledge.
2. US2 delivers it automatically.
3. US3 proves server durability.
4. US4 proves fail-soft outage behavior.
5. US5 makes the lifecycle auditable.
6. US6 makes status and verification truthful.
7. US7 safely migrates existing installations only after all canonical surfaces exist.

### Non-negotiable guards

- PostgreSQL is canonical after cutover; SQLite contains queues, cache, retained exceptions, and machine-local evidence only.
- No client payload chooses account/project authority, verification authority, derived state, domain ownership, or supersession.
- No raw transcript, raw vendor payload, raw tool output, secret, absolute local path, or local evidence crosses the approved boundary.
- `KnowledgeRef` identity always includes domain; `PatternRef` resolves to an owner-only personal-domain pattern; `reference_key` is used wherever the union participates in identity.
- OpenCode is automatic capture only in baseline 005; automatic delivery and semantic signals remain `declined_by_cairn` for the documented beta-boundary reason.
- Attempt 5 runs; success becomes `done` before remaining fifth failures become `failed`; attempt 6 never runs; every lease closes or reopens.
