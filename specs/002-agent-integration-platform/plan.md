# Implementation Plan: Agent Integration Platform

**Feature directory**: `specs/002-agent-integration-platform` | **Git branch**:
`claude/agent-integration-spec-r05mg4` | **Date**: 2026-08-11 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/002-agent-integration-platform/spec.md`.

| Provenance | Commit | What it carried |
|---|---|---|
| Initial clarified specification | `c992c63` | 139 FR / 35 SC — the spec this plan was first written against |
| Planning-reconciled specification | `bcf9032` | +FR-240–FR-244, +SC-136–SC-137, SC-110 restated |
| Final planning reconciliation | this artifact set | +FR-245, +SC-138 — capability evidence gates FULL |

The plan depends on all three; the requirements added at each step exist because planning found
the previous set unsatisfiable or contradictory, and the constitution requires such conflicts to
be resolved in the spec.

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
not probes, and they carry both what the vendor guarantees and what Cairn has actually
established here; the integration level is computed from them; and Cairn's inactivity reaper is
classified as recovery-from-silence that can never buy a FULL rating. Under currently
verified vendor behavior that yields Claude Code FULL, Codex FULL once its hooks are trusted
and its close budget is measured, OpenCode MCP_PLUS, and generic MCP MCP_ONLY — outputs of the
rule, not names in a table.

No new service, no new datastore, no seventh tool.

## Technical Context

**Language/Version**: Rust (workspace, pinned via `rust-toolchain.toml`); TypeScript for the
web UI, unchanged

**Primary Dependencies**: existing — Tokio, SQLx, serde, `clap`, `tracing`. Added:
**`jsonc-parser`** with its `cst` feature (source-preserving JSON/JSONC mutation),
**`toml_edit`** (document-preserving TOML for Codex), and **`include_dir`** (embedding the
Skill tree)

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

**Re-check after the planning reconciliation (2026-08-11)**: PASS. Six contradictions were
resolved; the resolutions added one dependency (`jsonc-parser`, replacing an unworkable
`serde_json` round-trip), one table (`ResourceBinding`, which removed a special case rather
than adding one), and one sweep on an existing maintenance tick. No new component, no new
service, no new tool.

**Re-check after the final reconciliation (2026-08-11)**: PASS. Three findings closed: research
D20 was still carrying the pre-conditional OpenCode failure model; capability confidence was
defined but gated only the completion guarantee, so a vendor removing tool capture could still
produce FULL; and the CC Switch Skill ref plan assumed a generic Git ref that its downloader
does not accept. The resolutions added one small local table (`CapabilityEvidence`) and changed
a ref-naming convention. Still no new component, service, datastore, or tool.

**Re-check after the publication reconciliation (2026-08-11)**: PASS. The `skill-release` branch
needed a producer, so `release.yml` gains one job that computes the revision, creates the
branch when absent, and verifies it through the same `refs/heads` fetch CC Switch performs. It
adds no runtime component and no dependency — it is release plumbing that makes an existing
planning invariant real, and it fails the release rather than moving a branch.

**Re-check after the branch-semantics reconciliation (2026-08-11)**: PASS. Three corrections,
none structural: the branch identifies Skill *content* rather than a release, so an unchanged
patch release reuses it instead of failing; the revision digest was circular and now has one
canonical algorithm with the self-field normalized before hashing (D29b); and the release job
graph is stated explicitly so the branch name is a build input rather than a prediction. One
developer-only binary (`skillref`) was added so the workflow calls the same function the
released binary embeds instead of reimplementing the hash in shell.

## Project Structure

### Documentation (this feature)

```text
specs/002-agent-integration-platform/
├── plan.md                      # This file
├── spec.md                      # Feature specification (145 FR, 38 SC)
├── research.md                  # Decisions D18–D42 (incl. D19a, D22a, D28a, D29a, D29b)
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
├── traceability.md              # FR/SC coverage map and privacy verification table
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
│                        #   InstalledResource, ResourceBinding, CapabilityEvidence,
│                        #   MigrationState, RecoveryArtifact)
│                        # + sessions.handoff_pending/attempts/error,
│                        #   observations.vendor_tool
├── cairn-integrate/     # NEW — the whole integration layer
│   ├── src/
│   │   ├── adapter.rs       # the trait: detect, capabilities, plan, inspect, normalize
│   │   ├── capability.rs    # availability × confidence; evidence rules; level derivation
│   │   ├── desired.rs       # DesiredIntegrationState, deterministic serialization
│   │   ├── plan.rs          # change-plan engine, shared by connect/repair/migrate
│   │   ├── apply.rs         # atomic write, verify, partial-failure reporting
│   │   ├── markers.rs       # managed-block parse/splice/validate, canonical hashing
│   │   ├── edit/{json.rs,toml.rs,markdown.rs}   # CST/document-preserving, byte-exact outside
│   │   │                                        # the owned node
│   │   ├── scope.rs         # the scope matrix as data
│   │   ├── render.rs        # contract → block, contract → MCP instructions; Skill tree
│   │   ├── revision.rs      # the one canonical skill_revision algorithm (D29b)
│   │   └── bin/skillref.rs  # prints schema/revision/branch for the release job
│   │   ├── agents/{claude_code.rs,codex.rs,opencode.rs,generic_mcp.rs}
│   │   ├── managers/cc_switch.rs
│   │   └── assets/agent-contract.md
│   └── tests/fixtures/      # the ≥20-file configuration corpus
├── cairnd/              # + canonical event handlers, integration-record handlers,
│                        #   sealed session close, pending-handoff sweep on the existing
│                        #   maintenance tick, handoff_pending reconciliation
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
.github/workflows/
├── ci.yml               # + web-e2e job (release server, desktop + mobile)
└── release.yml          # + publish-skill job (needs: verify; outputs skill_schema,
                         #   skill_revision, skill_branch). Creates the branch when absent,
                         #   never moves an existing one, verifies content through CC Switch's
                         #   own refs/heads fetch. `binaries` needs it and embeds skill_branch;
                         #   `images` does not (D29a)
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
| A | Adapter boundary: one trait with `detect`/`capabilities`/`plan`/`inspect`/`normalize`; everything else shared. Capabilities carry availability × confidence | D19, D19a, [agent-adapter section below](#adapter-shape) |
| B | Manager boundary: separate type, import-only public surface, never the private DB | D33, [contracts/cc-switch.md](./contracts/cc-switch.md) |
| C | Canonical lifecycle: seven events, per-adapter mapping with explicit non-emissions; sealed close with guaranteed progress; recovery is not completion | D20–D22a, [contracts/lifecycle.md](./contracts/lifecycle.md) |
| D | Desired state: one in-memory canonical model, versioned, deterministic, serializable, no file written | D26, [contracts/integration-health.md](./contracts/integration-health.md) |
| E | Local record: `cairn-store` tables behind daemon requests, never synced | [data-model.md](./data-model.md) |
| F | Mutation engine: inspect → plan → validate → apply → verify, atomic per file, CST/document-preserving editors so untouched bytes survive exactly | D37, D38 |
| G | Ownership and scope: reserved name + recorded canonical hash; scope per resource kind; shared resources reference counted by binding | D25, D27, D28, D28a, [contracts/scope-matrix.md](./contracts/scope-matrix.md) |
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
    fn inspect(&self, env: &Env, record: &[InstalledResource]) -> Vec<Observed>;
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
| **D. Sealed close** | `handoff_pending`, two-phase session close, retry plus maintenance-tick sweep, reconciliation, injected-failure recovery | `perf_session_close`, `handoff_lands_without_restart`, and `recovery_injected` pass |
| **E. Codex** | Codex adapter, TOML editing, failure classification, trust states, `installed_not_activated` | Codex connects; SC-128 measured; level FULL only after trust |
| **F. OpenCode** | OpenCode plugin, event-bus translation, quiescence, compaction, shared-resource detection | OpenCode connects; the idle-reaper negative test passes |
| **G. Repair & migrate** | `repair`, `--force`, recovery artifacts, `MigrationState` machine, resume/abort | Eight seeded defects detected and repaired; SC-114–SC-117 pass |
| **H. CC Switch** | Manager detection, deep-link import, binding verification, `manager_action_required`, the `publish-skill` release job and the `unpublished_skill_ref` refusal | Distribution verified with zero writes to the manager's storage; a development build refuses the Skill import; the release job creates and verifies the branch, and fails rather than moving an existing one |
| **I. Generic MCP & instructions** | MCP `instructions`, `integration export mcp`, generic profile | A plain MCP client initializes, sees six tools, and is reported MCP_ONLY |
| **J. Evidence** | Cross-agent continuity, concurrency, capture performance per adapter, privacy suites, Playwright CI, and the live Skill-distribution proof | The quickstart runs end to end; `web-e2e` is a required check; a published Skill revision imports through CC Switch and doctor reports the installed revision equal to the embedded one |

Dependencies: B needs A; C needs B; D is independent of C but must precede E; E and F need C
(the legacy bridge and the shared `AGENTS.md` block are proved there); G needs C–F; H needs G
(migration); I is independent after A; J needs everything.

## Risks and mitigations

| Risk | Severity | Mitigation |
|---|---|---|
| A per-user Codex hook and OpenCode plugin fire in repositories Cairn does not manage, adding cost to unrelated work | **HIGH** | The hook already returns immediately outside a Cairn project. SC-122 measures the unmanaged-repository cost explicitly; if it is material, `--shared` project installation becomes the recommended default for those two and D27's matrix is revised. This is the one default that could annoy a user who did nothing wrong. |
| Codex's 1 s session-end budget still cannot be met on a busy machine | **HIGH** | The sealed close (D22) removes the quiesce, the Git call, and the synthesis from the acknowledged path, leaving one small transaction. SC-128 measures ≥100 boundaries. If it still fails, Codex is reported below FULL — the design degrades honestly rather than lying. |
| Codex hook trust invalidation on every Cairn upgrade makes the integration feel broken | **MEDIUM** | Artifact versions are decoupled from the package version (D26), so a patch release that changes neither the contract nor the hook shape rewrites nothing and invalidates nothing. When it does happen, `installed_not_activated` names the exact command. |
| A shared resource — the `AGENTS.md` block, the per-user Skill — is deleted while another agent still needs it | **MEDIUM** | Resolved structurally: resources and bindings are separate records, and a resource is deleted only when its last binding goes (D28, FR-243). Doctor prints the full `serves` list before anyone disconnects. Fixtures cover disconnecting the first and the last consumer. Chosen over symlinks, whose behavior is unverified on two of the three agents. |
| A pending handoff never lands because the daemon keeps running and never restarts | **MEDIUM** | Resolved structurally: bounded retry plus a sweep on the maintenance tick that already runs, with permanent failure reported in `cairn status` and doctor (D22, FR-240). SC-136 measures that the handoff lands without a restart. |
| An agent upgrade drops the level to MCP_PLUS until a session has run, and reads as a regression | **MEDIUM** | It is the honest state: observation evidence from the previous build proves nothing about this one (FR-245). Doctor names exactly what is awaited rather than showing a bare level, and one ordinary session restores FULL. Accepted deliberately over silently carrying stale evidence, which is the failure H2 identified. |
| CC Switch silently installs `main` when handed a ref it cannot resolve | **MEDIUM** | Verified in its downloader: any miss falls back to `main`, then `master`. Cairn therefore emits only a published `skill-release/<schema>-<revision>` branch and refuses otherwise (D29); the release job verifies the branch through that same fetch before the release completes (D29a); doctor's revision comparison is the backstop. |
| The `skill-release` branch and the released binary disagree about the Skill | **MEDIUM** | The branch names content, not a release: it is created when absent, never moved, and every release re-verifies it by fetching it and recomputing the revision from what it contains. A content/name mismatch fails the release. The branch name reaches a binary only after that fetch passed, so a build can never claim a ref that does not exist (D29a, D29b). |
| The Skill revision digest drifts between the workflow and the binary | **LOW** | One function in `cairn-integrate` computes it, and the workflow calls that function through the `skillref` binary rather than reimplementing the hash. A unit test asserts the checked-in frontmatter equals the computed value, so an ordinary `cargo test` catches a stale field (D29b). |
| `toml_edit` or a vendor schema change breaks Codex config editing | **MEDIUM** | Malformed or unexpected input fails closed and writes nothing (FR-137). The fixture corpus includes comment-heavy, nested, and truncated TOML. `toml_edit` is the crate Cargo itself uses for this problem. |
| Adding `jsonc-parser`, `toml_edit`, and `include_dir` to the `cairn` binary slows hook startup | **MEDIUM** | The hook path calls only `cairn-integrate::normalize` — a pure function with no I/O — and never the editors, planner, or embedded assets. The cost is binary size, not work. SC-122 measures capture latency per adapter against Feature 001's baseline; if it regresses, the hook entry point moves to a thin binary linking only the normalize path. |
| A vendor renames or removes an event Cairn maps | **MEDIUM** | Adapters degrade by capability detection, not version matching (FR-188). A missing event lowers the level and the integration keeps working; fixtures record the payloads Cairn was built against so a change is visible in a diff. |
| `opencode.jsonc` shadows Cairn's `opencode.json` entry | **LOW** | Detected as `conflicting_owner` before writing (D38) rather than producing a silently inert configuration. |
| CC Switch adds a removal interface and the plan looks stale | **LOW** | FR-235 already permits the adapter to use one if it appears; nothing needs redesigning. |
| Playwright makes CI flaky and gets ignored | **LOW** | Release-build server (the documented Argon2 timing problem), Chromium only, traces on failure, 20-minute cap, separate job so a browser failure is distinguishable from a build failure. |

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| A seventh crate, `cairn-integrate` | `cairn-core` is documented as I/O-free and `cairn-server` depends on it, so vendor parsing and `toml_edit` cannot go there; the daemon must not parse vendor configuration; and the ≥20-file fixture corpus plus every recorded vendor payload must be testable with no daemon, no socket, and no Git repository | Putting it in `crates/cairn` makes the corpus testable only through a binary that also owns the CLI, the hook runtime, and the MCP server, and pulls integration code into every hook invocation's compilation unit. Splitting it across `cairn-core` and `cairn` gives one boundary two homes and breaks the core's no-I/O rule |
| Three new runtime dependencies (`jsonc-parser`, `toml_edit`, `include_dir`) | SC-103/SC-104 assert byte identity for non-Cairn content, which a parse-and-reserialize editor cannot deliver in either format: `serde_json` discards indentation, escaping, and layout; `toml` discards comments. Both need a tree that retains source spans. The Skill is a directory tree that must ship inside the binary *and* be fetchable by CC Switch from a repository path | `serde_json` with `preserve_order` preserves key order only — it fails SC-104 by construction, which is why this decision was revised. Hand-rolled splicing is prohibited by FR-153. Generating the Skill from string literals leaves CC Switch nothing to clone and makes the Skill unreviewable in diffs |
