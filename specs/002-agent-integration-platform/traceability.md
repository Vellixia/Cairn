# Traceability: Requirements and Criteria to Design

**Feature**: `002-agent-integration-platform` | **Date**: 2026-08-11

The planning gate. Every requirement maps to a planning component; every success criterion
maps to a verification strategy at a named tier. Written because Feature 002 carries 145
requirements across four adapters and a manager — a scale where "it is all covered" is not a
claim anyone can check by reading.

Test tiers are from [research.md](./research.md) D40:
**T1** unit (`cairn-integrate`) · **T2** configuration fixtures · **T3** lifecycle fixtures ·
**T4** daemon integration (`tests/`) · **T5** live agent, manual release evidence only.

## Requirements → components

| Requirements | Section | Primary component | Artifact |
|---|---|---|---|
| FR-101–FR-106 | Adapters, managers, detection | `cairn-integrate::adapter`, `::agents/*`, `::managers/cc_switch` | plan §Adapter shape, [cc-switch.md](./contracts/cc-switch.md) |
| FR-107–FR-111, FR-207–FR-209, FR-229, FR-241, FR-242, FR-245 | Capability model, evidence, and level | `cairn-integrate::capability` — availability × confidence; `CapabilityEvidence` persists what established each capability and against which version; every FULL-required capability must be established; the idle reaper never counts (D22a, D19a) | [data-model.md](./data-model.md) §CapabilityProfile and §CapabilityEvidence, [integration-health.md](./contracts/integration-health.md) |
| FR-112–FR-119, FR-230, FR-231, FR-240 | Canonical lifecycle and sealed close | `AgentAdapter::normalize`, `cairn-core` event type, daemon handlers, pending-handoff sweep on the existing maintenance tick | [lifecycle.md](./contracts/lifecycle.md), D20–D22a |
| FR-120–FR-122 | Tool categories | Feature 001 `classify_tool` reused; `vendor_tool` column | [lifecycle.md](./contracts/lifecycle.md) §Tool normalization, D36 |
| FR-123–FR-127 | Usage contract | `cairn-integrate::render` from one embedded asset | [agent-contract.md](./contracts/agent-contract.md), D29 |
| FR-128–FR-131 | MCP surface | `cairn/src/mcp.rs` — `instructions` added, six tools unchanged, revision held | D34 |
| FR-132–FR-139 | Managed instructions | `cairn-integrate::markers`, `::edit::markdown` | [agent-contract.md](./contracts/agent-contract.md), D25 |
| FR-140–FR-144 | Skill | `skills/cairn/` embedded; shared by binding; manager distribution only from a published `skill-release/<schema>-<revision>` branch, else refused | D28, D29 |
| FR-145–FR-150, FR-228, FR-232–FR-235, FR-243, FR-244 | Ownership and sharing | `InstalledResource` + `ResourceBinding` (reference counting), `MigrationState`, manager boundary | [data-model.md](./data-model.md), [cc-switch.md](./contracts/cc-switch.md), D28, D28a |
| FR-210–FR-220 | Installation scope | `cairn-integrate::scope` — the matrix as data | [scope-matrix.md](./contracts/scope-matrix.md), D27 |
| FR-151–FR-158, FR-238, FR-239 | Safe mutation | `::plan`, `::apply`, `::edit/*`, `RecoveryArtifact` | D37, D39 |
| FR-159–FR-162 | Change preview | `::plan` with no apply; `--dry-run` on every mutating command | [integration-health.md](./contracts/integration-health.md) §Change plan |
| FR-163–FR-165 | Onboarding | `cairn connect --auto`; proposal from `DesiredIntegrationState` | [integration-cli.md](./contracts/integration-cli.md) |
| FR-166–FR-171 | Diagnostics | `::inspect` + health report; one engine shared with repair | [integration-health.md](./contracts/integration-health.md) |
| FR-172–FR-177, FR-221–FR-225 | Repair | `cairn repair`, `--force`, recovery artifacts, semantic comparison | [integration-cli.md](./contracts/integration-cli.md), D39 |
| FR-178–FR-181, FR-236, FR-237 | Disconnect | `cairn disconnect [--only <kind>]`; removal by binding; manager-owned resources reported, never removed | [integration-cli.md](./contracts/integration-cli.md), [cc-switch.md](./contracts/cc-switch.md) |
| FR-182–FR-184 | Local record | `cairn-store` migration `0004`; daemon handlers; no outbox entity type | [data-model.md](./data-model.md) |
| FR-185–FR-188 | Version compatibility | `Detection` + `CompatibilityClassification`; capability-driven degradation | [integration-health.md](./contracts/integration-health.md) |
| FR-189–FR-192 | Cross-agent memory | **No change to Feature 001's memory model** — the requirement is that nothing is added | [data-model.md](./data-model.md) §What syncs |
| FR-193–FR-196 | Non-blocking | Hook path unchanged; configuration operations fail loudly | [lifecycle.md](./contracts/lifecycle.md) §Failure behavior |
| FR-197–FR-200 | Privacy | Per-field payload allow-list; recovery artifact scoping; nothing syncs | D35, D39 |
| FR-201, FR-202, FR-226, FR-227 | Desired state | `cairn-integrate::desired`, single model, no file written | [integration-health.md](./contracts/integration-health.md) §Desired state |
| FR-203–FR-206 | Verification | Five tiers; Playwright job | D40, D42 |

Coverage: **145 of 145**. FR-189–FR-192 are satisfied by an absence — the design adds no
agent-keyed scope anywhere — and that absence is asserted by test (SC-108).

## Criteria → verification

| Criterion | Tier | Where |
|---|---|---|
| SC-101 zero-to-capturing in <5 min, three agents | T5 | quickstart US1/US3/US4, timed |
| SC-102 connect twice, zero changes | T2 | `fixtures::idempotent_reconnect` per adapter |
| SC-103 Feature 001 upgrade, one entry each | T2 | `fixtures::legacy_f001_adoption` |
| SC-104 ≥20 configs, connect→disconnect byte-identical | T2 | `fixtures::preservation` over the corpus |
| SC-105 contract ≤ size bound | T1 | `render::contract_within_bound` |
| SC-106 exactly six MCP tools | T1 | existing `mcp::tool_count` extended |
| SC-107 generic client: init, contract, six tools, MCP_ONLY | T3 | `tests/integrations/generic-mcp/` |
| SC-108 cross-agent continuity, one project | T4 | `us6_cross_agent` (T5 mirror) |
| SC-109 two agents, two sessions, correct provenance | T4 | `us10_concurrency` |
| SC-110 guaranteed proved, absent silent, conditional both ways | T3 | `tests/integrations/*` — three-way per capability |
| SC-111 no idle→close mapping, any adapter | T3 | `lifecycle::idle_never_closes` |
| SC-112 manager distribution, one entry per app | T2 + T5 | binding fixtures; live confirmation |
| SC-113 provider switch leaves Cairn healthy | T2 | post-switch fixture |
| SC-114 eight seeded defects detected exactly | T2 | `fixtures::defects` |
| SC-115 repair fixes owned defects only, idempotent | T2 | `fixtures::repair` |
| SC-116 disconnect preserves everything else | T2 + T4 | `fixtures::disconnect`; memory-survival in T4 |
| SC-117 migration never zero-effective; one owner after | T2 | `fixtures::migration` incl. interrupted resume/abort |
| SC-118 preview writes nothing | T1 | `dry_run_is_inert` — checksums before/after |
| SC-119 no secrets in preview/diagnostics/logs | T2 | seeded-credential corpus |
| SC-120 no integration state in sync payloads or server DB | T4 | `privacy_integration` inspects outbox and PostgreSQL |
| SC-121 no conversation text stored | T3 + T4 | `privacy_payloads` with seeded `last_assistant_message`, `tool_calls`, prompts |
| SC-122 capture latency per adapter, release build | T4 | `perf_capture`, ≥200 invocations each |
| SC-123 unknown version integrates; only known-bad rejected | T1 | `capability::compatibility` |
| SC-124 fixture tests hermetic in CI | CI | no vendor binary, no credentials, no network |
| SC-125 hosted CI: lint, typecheck, build, Playwright both viewports | CI | `web-e2e` job, D42 |
| SC-126 default scopes write no committed files | T2 | `fixtures::scope_defaults` |
| SC-127 FULL requires a demonstrated completion mechanism | T1 + T4 | `capability::full_requires_completion`; Codex gated on SC-128 |
| SC-128 ≥100 nominal boundaries inside the vendor budget | T4 | `perf_session_close` |
| SC-129 injected timeout/crash/daemon-unavailable recovery | T4 | `recovery_injected` |
| SC-130 formatting-only difference is healthy; semantic edit is modified | T1 | `markers::semantic_equivalence` |
| SC-131 idle reaper never yields FULL | T4 | `us4_opencode::idle_reaper_never_grants_full` |
| SC-132 zero writes to manager storage; verified after user action | T2 | checksum `~/.cc-switch/` around every manager operation |
| SC-133 no foreign credential in artifacts, logs, or state | T2 | seeded-credential corpus across all four operations |
| SC-134 quiescence after an error synthesizes nothing | T3 | `lifecycle::quiesce_after_error` |
| SC-135 desired state deterministic, secret-free, single source | T1 | `desired::determinism`, `desired::single_consumer` |
| SC-136 sealed close lands without a restart; permanent failure reported | T4 | `handoff_lands_without_restart` |
| SC-137 shared resource survives one disconnect; manager state survives | T2 + T4 | `fixtures::shared_binding`, `fixtures::manager_state_survives` |
| SC-138 evidence gates FULL; version change re-opens it; establishing restores FULL | T1 + T4 | `capability::evidence_gates_full`, `capability::version_change_invalidates`, `capability_evidence` (T4) |

Coverage: **38 of 38**, with 34 in required CI and 4 (SC-101, and the live halves of SC-108,
SC-112, SC-122's end-to-end context) additionally carried as release evidence.

## Privacy verification map

Required by the brief's threat review. For each new surface: what Cairn reads, retains, logs,
and syncs.

| Surface | Reads | Retains | Logs | Syncs |
|---|---|---|---|---|
| Claude hook payloads | routing fields, tool name/input, error | per D35 allow-list | drop reasons only | no |
| Codex hook payloads | same, plus `turn_id` for routing | same allow-list | drop reasons only | no |
| OpenCode plugin events | `sessionID`, tool name/args, derived outcome | same allow-list | drop reasons only | no |
| Agent config files | whole file, to plan and verify (CST retains spans in memory only) | only Cairn's own entries and their hashes | file paths, never content | no |
| CC Switch | installation presence and version | binding results only | the deep-link URI (secret-free by construction) | no |
| Skills and instruction files | whole file, to splice | only the Cairn block's hash | paths only | no |
| Preview and doctor output | inspected files | nothing | nothing | no |
| Recovery artifacts | Cairn-owned prior content only | that content, ≤10 per resource | the path, never the content | no |
| Desired-state serialization | the model | nothing (in memory) | nothing | no |

Defaults confirmed: configuration content stays local; credentials are never persisted;
conversation text is never persisted; raw tool output is never persisted; integration state
never syncs.
