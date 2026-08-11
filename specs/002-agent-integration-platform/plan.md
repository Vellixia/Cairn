# Implementation Plan: Agent Integration Platform

**Feature directory**: `specs/002-agent-integration-platform` | **Git branch**:
`claude/agent-integration-spec-r05mg4` | **Date**: 2026-08-11 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/002-agent-integration-platform/spec.md` at
`c992c63`

## Summary

Feature 001 proved persistent project memory with one agent. Everything below the agent —
sessions, observations, scoped memory, briefings, handoffs, tasks, the six MCP tools, sync —
is already agent-neutral. Everything above it is not.

Feature 002 inserts one boundary. A new `cairn-integrate` crate holds every adapter, the CC
Switch manager, the desired-state model, and the configuration-change engine. Vendor payloads
become seven canonical lifecycle events before the daemon sees them; vendor configuration
becomes a change plan before anything is written. The daemon gains three things and no new
responsibility: canonical event handling, a local integration record, and a **sealed session
close** that records termination durably before acknowledging, so Codex's one-second handler
budget does not cost the completion guarantee.

The honesty requirements are the design's spine. Capability profiles are static vendor facts,
not probes; the integration level is computed from them; and Cairn's inactivity reaper is
classified as recovery-from-silence that can never buy a FULL rating. Under currently
verified vendor behavior that yields Claude Code FULL, Codex FULL once its hooks are trusted
and its close budget is measured, OpenCode MCP_PLUS, and generic MCP MCP_ONLY — outputs of the
rule, not names in a table.

No new service, no new datastore, no seventh tool.

## Technical Context

**Language/Version**: Rust (workspace, pinned via `rust-toolchain.toml`); TypeScript for the
web UI, unchanged

**Primary Dependencies**: existing — Tokio, SQLx, serde, `clap`, `tracing`. Added:
**`toml_edit`** (document-preserving TOML for Codex), **`include_dir`** (embedding the Skill
tree), and `serde_json`'s **`preserve_order`** feature (stable key order in JSON edits)

**Storage**: the existing local SQLite database, extended by one additive migration
(`0004_integrations.sql`). PostgreSQL is **not** touched — nothing in this feature syncs

**Testing**: `cargo test` across five tiers (D40); fixture corpora in
`crates/cairn-integrate/tests/fixtures/` and `tests/integrations/`; Playwright moved into
hosted CI against a release-build server

**Target Platform**: macOS and Linux developer machines, matching Feature 001

**Project Type**: Multi-binary Rust workspace, plus a separate web application — unchanged

**Performance Goals**: Feature 001's capture bounds hold per adapter (≤10 ms median, ≤25 ms
p95, inside 250 ms); Codex session close completes inside its **1 s default / 3 s maximum**
handler budget in 100% of measured boundaries; detection and doctor complete without network

**Constraints**: fully offline; no credential ever persisted by Cairn; no conversation text
persisted; no integration state synced; configuration operations fail loudly while capture
stays fail-soft; no write to any manager's private storage

**Scale/Scope**: four adapters and one manager on one developer machine; four resource kinds
per agent; a ≥20-file configuration fixture corpus

## Constitution Check

*GATE: evaluated before Phase 0 and re-evaluated after design.*

| Principle | Assessment |
|---|---|
| I. Usable MVP First | PASS — ten vertically sliced phases, each ending runnable. Phase C alone (Claude migrated onto the adapter model with the contract and Skill) is a shippable improvement over Feature 001. No architecture-first phase: the domain work in Phase A is proved by Phase C, not by its own existence. |
| II. Simple Architecture | PASS with one judgment call. No new service, datastore, broker, or MCP tool. One new crate, justified in Complexity Tracking. Adapters are a trait with five operations over shared machinery, not a plugin framework (D19). Capability profiles are data. |
| III. Local-First Reliability | PASS — every integration operation is local and offline. Capture keeps the fail-soft contract unchanged (FR-193); the sealed close makes session completion *more* robust, not less. Configuration operations deliberately fail loudly, which is a different contract for a different job (FR-196). |
| IV. Project-Scoped Memory | PASS — agent identity is provenance only. No new scope, partition, or filter keyed to an agent exists anywhere in the data model (FR-189). One repository still resolves to one project. |
| V. Privacy by Default | PASS — an explicit per-field payload allow-list (D35) closes the side channel new vendor payloads would otherwise open, including Claude's `last_assistant_message` and `tool_calls`. Recovery artifacts hold only Cairn-owned content (D39). Nothing in the feature syncs. |
| VI. Deterministic Data Boundaries | PASS — ownership is a reserved name plus a recorded canonical hash, never a substring search. Desired state serializes deterministically. Connect, repair, and migrate are idempotent. Semantic comparison means formatting is not a change. |
| VII. Testable Behavior | PASS — every success criterion has a named test at a named tier (D40), including the negative ones: no adapter maps idle to close, no capability the profile denies produces an event, and the idle reaper cannot produce FULL. |

**Re-check after Phase 1 design**: PASS. The design added one crate and three dependencies
beyond Feature 001 and no component. Two judgment calls are recorded in Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/002-agent-integration-platform/
├── plan.md                      # This file
├── spec.md                      # Feature specification (139 FR, 35 SC)
├── research.md                  # Decisions D18–D42
├── data-model.md                # Integration entities, invariants, transitions
├── quickstart.md                # Acceptance walkthrough per user story
├── checklists/requirements.md   # Specification quality gate
├── contracts/
│   ├── integration-cli.md       # CLI surface, flags, envelopes, error codes
│   ├── lifecycle.md             # Seven canonical events and the three mappings
│   ├── scope-matrix.md          # Agent × resource: location, scope, owner, precedence
│   ├── integration-health.md    # Detection, health, change plan, desired state
│   ├── agent-contract.md        # Usage contract, managed block, MCP instructions, Skill
│   └── cc-switch.md             # Manager boundary, import flow, manager-action-required
└── tasks.md                     # Generated by /speckit-tasks — NOT created here
```

### Source Code (repository root)

```text
crates/
├── cairn-core/          # unchanged in shape: canonical lifecycle event type, new wire
│                        # requests, `vendor_tool` on ObservationInput. Still no I/O.
├── cairn-git/           # unchanged
├── cairn-store/         # + migrations/0004_integrations.sql
│                        # + integrations.rs (AgentIntegration, ManagerIntegration,
│                        #   ResourceState, MigrationState, RecoveryArtifact)
│                        # + sessions.handoff_pending, observations.vendor_tool
├── cairn-integrate/     # NEW — the whole integration layer
│   ├── src/
│   │   ├── adapter.rs       # the trait: detect, capabilities, plan, inspect, normalize
│   │   ├── capability.rs    # profiles as data; level and completion-guarantee derivation
│   │   ├── desired.rs       # DesiredIntegrationState, deterministic serialization
│   │   ├── plan.rs          # change-plan engine, shared by connect/repair/migrate
│   │   ├── apply.rs         # atomic write, verify, partial-failure reporting
│   │   ├── markers.rs       # managed-block parse/splice/validate, canonical hashing
│   │   ├── edit/{json.rs,toml.rs,markdown.rs}   # format-specific, structure-preserving
│   │   ├── scope.rs         # the scope matrix as data
│   │   ├── render.rs        # contract → block, contract → MCP instructions; Skill tree
│   │   ├── agents/{claude_code.rs,codex.rs,opencode.rs,generic_mcp.rs}
│   │   ├── managers/cc_switch.rs
│   │   └── assets/agent-contract.md
│   └── tests/fixtures/      # the ≥20-file configuration corpus
├── cairnd/              # + canonical event handlers, integration-record handlers,
│                        #   sealed session close, handoff_pending reconciliation
├── cairn/               # + agents/doctor/repair/integration commands; hook entry points
│                        #   call cairn-integrate::normalize; MCP `instructions`
└── cairn-server/        # UNCHANGED — nothing in this feature reaches the server

skills/cairn/            # NEW — canonical Skill source; embedded, and the path CC Switch fetches
├── SKILL.md
└── references/*.md

tests/                   # + us4_opencode, us6_cross_agent, us10_concurrency,
                         #   perf_capture, perf_session_close, recovery_injected,
                         #   privacy_integration, privacy_payloads
tests/integrations/      # NEW — recorded vendor payload fixtures per adapter
.github/workflows/ci.yml # + web-e2e job (release server, desktop + mobile)
```

**Structure Decision**: one new crate, `cairn-integrate`, sitting between `cairn-core` and
`cairn` (D18). The dependency direction decides it: `cairn-core` is documented as having no
I/O and `cairn-server` depends on it, so vendor parsing and TOML editing must not live there;
the daemon must never parse vendor configuration, so it must not live there either; and the
fixture corpus must be testable without a daemon, a socket, or a temporary Git repository,
which it can only be in a crate with no SQLite and no runtime requirement. Persistence stays
in `cairn-store` behind daemon requests, preserving Feature 001's single-writer rule.

`cairn-server` is untouched. That is the clearest statement that this is a local feature.

## Architectural decisions

The plan resolves every decision the brief required. Full reasoning is in
[research.md](./research.md); this is the index.

| # | Decision | Where |
|---|---|---|
| A | Adapter boundary: one trait with `detect`/`capabilities`/`plan`/`inspect`/`normalize`; everything else shared | D19, [agent-adapter section below](#adapter-shape) |
| B | Manager boundary: separate type, import-only public surface, never the private DB | D33, [contracts/cc-switch.md](./contracts/cc-switch.md) |
| C | Canonical lifecycle: seven events, per-adapter mapping with explicit non-emissions; sealed close; recovery is not completion | D20–D22a, [contracts/lifecycle.md](./contracts/lifecycle.md) |
| D | Desired state: one in-memory canonical model, versioned, deterministic, serializable, no file written | D26, [contracts/integration-health.md](./contracts/integration-health.md) |
| E | Local record: `cairn-store` tables behind daemon requests, never synced | [data-model.md](./data-model.md) |
| F | Mutation engine: inspect → plan → validate → apply → verify, atomic per file, format-specific editors | D37, D38 |
| G | Ownership and scope: reserved name + recorded canonical hash; scope chosen per resource kind | D25, D27, [contracts/scope-matrix.md](./contracts/scope-matrix.md) |
| H | Doctor/repair: one inspection engine, one plan engine, two entry points | [contracts/integration-health.md](./contracts/integration-health.md) |
| I | Contract and Skill: one canonical source each, embedded, two renderings | D29, [contracts/agent-contract.md](./contracts/agent-contract.md) |
| J | MCP: stay at `2025-06-18`, add `instructions` | D34 |
| K | CC Switch: deep-link import, user confirmation, post-hoc verification, `manager_action_required` for removal | D33 |
| L | Testing: five tiers, four required in CI | D40, D41 |
| M | Playwright: new `web-e2e` job against a release-build server | D42 |

### Adapter shape

```rust
trait AgentAdapter {
    fn id(&self) -> AgentId;
    fn detect(&self, env: &Env) -> Detection;                    // installed? version?
    fn capabilities(&self, d: &Detection) -> CapabilityProfile;  // static data, refined by detection
    fn plan(&self, desired: &DesiredAgent, observed: &[Observed]) -> Vec<PlannedChange>;
    fn inspect(&self, env: &Env, record: &[ResourceState]) -> Vec<Observed>;
    fn normalize(&self, event: &str, payload: &RawPayload) -> Option<CanonicalLifecycleEvent>;
}
```

Five operations, four implementors. Atomic writing, marker handling, canonical hashing,
change classification, semantic comparison, and verification are shared code the adapters
call — so a bug in atomicity is fixed once. `normalize` returning `None` is the normal way an
adapter declines an event it does not map (FR-115).

The manager is a different type with a different shape — `detect`, `inspect_bindings`,
`import_uri`, `verify` — because it has no lifecycle, no instructions, and no removal
(FR-101).

## Phasing

Each phase ends runnable. Checkpoints are demoable states; the feature is the whole table.

| Phase | Delivers | Runnable proof |
|---|---|---|
| **A. Domain** | Canonical event type, capability model and level derivation, desired-state model, marker parsing and canonical hashing, contract + Skill assets and both renderings | `cargo test -p cairn-integrate`; contract size and rendering-parity tests pass |
| **B. Engine** | Change-plan engine, format editors, atomic apply and verify, scope matrix, local record migration and daemon handlers | `cairn connect --dry-run` prints a plan for a fixture repository and writes nothing |
| **C. Claude Code** | Claude adapter, legacy Feature 001 adoption, `PostCompact`, `cairn agents`/`doctor` | A Feature 001 repository upgrades with zero duplicates; SC-102/SC-103 pass |
| **D. Sealed close** | `handoff_pending`, two-phase session close, reconciliation, injected-failure recovery | `perf_session_close` and `recovery_injected` pass |
| **E. Codex** | Codex adapter, TOML editing, failure classification, trust states, `installed_not_activated` | Codex connects; SC-128 measured; level FULL only after trust |
| **F. OpenCode** | OpenCode plugin, event-bus translation, quiescence, compaction, shared-resource detection | OpenCode connects; the idle-reaper negative test passes |
| **G. Repair & migrate** | `repair`, `--force`, recovery artifacts, `MigrationState` machine, resume/abort | Eight seeded defects detected and repaired; SC-114–SC-117 pass |
| **H. CC Switch** | Manager detection, deep-link import, binding verification, `manager_action_required` | Distribution verified with zero writes to the manager's storage |
| **I. Generic MCP & instructions** | MCP `instructions`, `integration export mcp`, generic profile | A plain MCP client initializes, sees six tools, and is reported MCP_ONLY |
| **J. Evidence** | Cross-agent continuity, concurrency, capture performance per adapter, privacy suites, Playwright CI | The quickstart runs end to end; `web-e2e` is a required check |

Dependencies: B needs A; C needs B; D is independent of C but must precede E; E and F need C
(the legacy bridge and the shared `AGENTS.md` block are proved there); G needs C–F; H needs G
(migration); I is independent after A; J needs everything.

## Risks and mitigations

| Risk | Severity | Mitigation |
|---|---|---|
| A per-user Codex hook and OpenCode plugin fire in repositories Cairn does not manage, adding cost to unrelated work | **HIGH** | The hook already returns immediately outside a Cairn project. SC-122 measures the unmanaged-repository cost explicitly; if it is material, `--shared` project installation becomes the recommended default for those two and D27's matrix is revised. This is the one default that could annoy a user who did nothing wrong. |
| Codex's 1 s session-end budget still cannot be met on a busy machine | **HIGH** | The sealed close (D22) removes the quiesce, the Git call, and the synthesis from the acknowledged path, leaving one small transaction. SC-128 measures ≥100 boundaries. If it still fails, Codex is reported below FULL — the design degrades honestly rather than lying. |
| Codex hook trust invalidation on every Cairn upgrade makes the integration feel broken | **MEDIUM** | Artifact versions are decoupled from the package version (D26), so a patch release that changes neither the contract nor the hook shape rewrites nothing and invalidates nothing. When it does happen, `installed_not_activated` names the exact command. |
| The OpenCode Skill's `satisfied_by` link breaks when Claude Code is disconnected | **MEDIUM** | Disconnect leaves OpenCode's Skill `missing` and `cairn doctor` says so with a one-command remedy. Chosen over symlinks, whose behavior is unverified on two of the three agents (D28). A fixture covers the disconnect. |
| `toml_edit` or a vendor schema change breaks Codex config editing | **MEDIUM** | Malformed or unexpected input fails closed and writes nothing (FR-137). The fixture corpus includes comment-heavy, nested, and truncated TOML. `toml_edit` is the crate Cargo itself uses for this problem. |
| Adding `toml_edit` and `include_dir` to the `cairn` binary slows hook startup | **MEDIUM** | Hooks never call into `cairn-integrate`. SC-122 measures capture latency per adapter against Feature 001's baseline; if it regresses, the hook entry point moves to a thin binary. Not pre-emptively solved. |
| A vendor renames or removes an event Cairn maps | **MEDIUM** | Adapters degrade by capability detection, not version matching (FR-188). A missing event lowers the level and the integration keeps working; fixtures record the payloads Cairn was built against so a change is visible in a diff. |
| `opencode.jsonc` shadows Cairn's `opencode.json` entry | **LOW** | Detected as `conflicting_owner` before writing (D38) rather than producing a silently inert configuration. |
| CC Switch adds a removal interface and the plan looks stale | **LOW** | FR-235 already permits the adapter to use one if it appears; nothing needs redesigning. |
| Playwright makes CI flaky and gets ignored | **LOW** | Release-build server (the documented Argon2 timing problem), Chromium only, traces on failure, 20-minute cap, separate job so a browser failure is distinguishable from a build failure. |

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| A seventh crate, `cairn-integrate` | `cairn-core` is documented as I/O-free and `cairn-server` depends on it, so vendor parsing and `toml_edit` cannot go there; the daemon must not parse vendor configuration; and the ≥20-file fixture corpus plus every recorded vendor payload must be testable with no daemon, no socket, and no Git repository | Putting it in `crates/cairn` makes the corpus testable only through a binary that also owns the CLI, the hook runtime, and the MCP server, and pulls integration code into every hook invocation's compilation unit. Splitting it across `cairn-core` and `cairn` gives one boundary two homes and breaks the core's no-I/O rule |
| Two new runtime dependencies (`toml_edit`, `include_dir`) | FR-152 promises comment and formatting preservation, and FR-153 forbids hand-written string substitution into structured configuration — for TOML that requires a document-preserving parser, and `toml` is not one. The Skill is a directory tree that must ship inside the binary *and* be fetchable by CC Switch from a repository path | Hand-rolled TOML editing is explicitly prohibited by FR-153. Generating the Skill from string literals leaves CC Switch nothing to clone and makes the Skill unreviewable in diffs |
