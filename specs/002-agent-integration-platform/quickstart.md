# Quickstart: Agent Integration Platform

**Feature**: `002-agent-integration-platform`

The acceptance walkthrough, one section per user story. Each section is runnable and states
what must be true afterwards. Commands follow
[`contracts/integration-cli.md`](./contracts/integration-cli.md); locations follow
[`contracts/scope-matrix.md`](./contracts/scope-matrix.md).

Sections marked **hermetic** run in CI with no vendor binary, no authentication, and no
network. Sections marked **live** are manual release evidence (FR-205, D40).

## Prerequisites

```bash
cargo build --workspace --release
export PATH="$PWD/target/release:$PATH"
export REPO=$PWD                        # remember the checkout, for fixtures below
export CAIRN_HOME=$(mktemp -d)          # isolate this walkthrough
cd $(mktemp -d) && git init && git commit --allow-empty -m init
cairn init
```

For the live sections: Claude Code, Codex, and OpenCode installed and signed in; CC Switch
installed for its section.

---

## US1 — The agent understands Cairn *(hermetic + live)*

```bash
cairn connect claude-code
```

Then:

```bash
grep -c 'cairn:managed:begin id=agent-contract' CLAUDE.md    # → 1
cairn doctor claude-code --json | jq '.data.agents[0].resources[]
  | select(.kind=="instructions") | .condition'              # → "healthy"
ls ~/.claude/skills/cairn/SKILL.md                            # exists
```

**Must be true**

- Exactly one managed block in `CLAUDE.md`; every pre-existing line byte-identical.
- The rendered block body is ≤1,200 characters (SC-105).
- The Skill is installed at user scope with `metadata.cairn_skill_schema` present.
- Running `cairn connect claude-code` again reports `unchanged` and writes nothing (SC-102).
- `cairn integration export mcp` and the MCP `initialize` response state the same numbered
  rules as the block (SC-135's rendering half).

**Live**: start a Claude Code session and ask what it should do before investigating an
unfamiliar area. The answer reflects the contract without further prompting.

---

## US2 — A Feature 001 user upgrades without damage *(hermetic)*

Reconstruct a Feature 001 installation, then upgrade:

```bash
FIX=$REPO/crates/cairn-integrate/tests/fixtures/claude-code/legacy-f001
mkdir -p .claude && cp $FIX/.claude/settings.json .claude/settings.json
cp $FIX/.mcp.json .mcp.json
sha256sum .claude/settings.json .mcp.json > /tmp/before

cairn connect claude-code
```

**Must be true**

- Exactly one Cairn entry per registered hook event and exactly one Cairn MCP entry
  (SC-103).
- Every unrelated hook, MCP server, and setting is byte-identical to `/tmp/before`.
- Both resources are recorded at `project_shared` — **adopted in place, not relocated**
  (FR-217).
- `cairn connect claude-code` again reports `unchanged`.
- A user hook whose command merely contains the words `cairn hook` inside a longer script is
  **not** adopted (the legacy bridge matches exact shapes only).

---

## US3 — Codex participates natively *(hermetic + live)*

```bash
cairn connect codex --dry-run     # inspect the plan first
cairn connect codex --yes
```

**Must be true**

- `[mcp_servers.cairn]` added to `~/.codex/config.toml`; every comment, key order, and
  unrelated table preserved (SC-104).
- Cairn hooks written to `~/.codex/hooks.json`.
- One managed block in `AGENTS.md`, recorded as shared with OpenCode.
- Output states plainly that Codex will not run the hooks until trusted, with the exact
  command, and `cairn doctor codex` reports `installed_not_activated` with level below FULL
  (FR-209).

**Live**, after trusting the hooks in Codex and running one ordinary session:

```bash
cairn doctor codex --json | jq '{level: .data.agents[0].level,
                                awaited: .data.agents[0].awaited_behaviors}'
```

Before that session the level is `mcp_plus` with `awaited_behaviors` naming the observations
still needed; after it — and once SC-128 passes — it is `full` (FR-245).

Run a short Codex session that edits a file and runs a failing command, then:

```bash
cairn handoff show
```

The handoff names the changed file and the failure. Observations carry canonical categories,
and a failure is recorded only where the payload established it (FR-117).

---

## US4 — OpenCode participates natively *(hermetic + live)*

```bash
cairn connect opencode --yes
```

**Must be true**

- Plugin file at `~/.config/opencode/plugin/cairn.js` — a file drop, no config edit
  (D32).
- `mcp.cairn` added to `~/.config/opencode/opencode.json`; unrelated servers untouched.
- `AGENTS.md` block **bound** to the existing Codex block, not written twice; doctor reports
  it `shared` serving `codex, opencode`.
- Skill reported `shared` at `~/.claude/skills/cairn/` serving `claude-code, opencode`, with no
  second copy (D28).
- `cairn doctor opencode` reports `MCP_PLUS`, `lifecycle_coverage.absent` containing
  `session_closed`, `lifecycle_coverage.conditional` containing `tool_failed`, a
  `missing_behaviors` entry naming automatic session completion (SC-131), and a
  `conditional_behaviors` entry explaining when failures are captured.
- A fixture payload whose OpenCode tool output establishes a failure produces `tool_failed`;
  an ambiguous one produces nothing (SC-110, conditional half).

**The release-blocking negative check** (hermetic):

```bash
# drive session_opened, tool events and session.idle through the adapter,
# then advance the clock past the inactivity timeout
cargo test -p cairn-e2e --test us4_opencode idle_reaper_never_grants_full
```

The session is closed by inactivity with a `recovered` handoff, and the computed level is
**still** `MCP_PLUS` — recovery from silence never counts (FR-229, SC-131).

---

## US5 — CC Switch distributes Cairn *(hermetic + live)*

```bash
cairn agents                       # cc-switch listed as a manager, not an agent
cairn integration distribute --via cc-switch --resource mcp --apps claude,codex,opencode
```

**Must be true**

- The command returns `manager_action_required` with `status: awaiting_user` and exits `1` —
  it has not completed (FR-233).
- A checksum of `~/.cc-switch/` is identical before and after (SC-132).

**Live**: confirm the import in CC Switch, then:

```bash
cairn doctor --json | jq '.data.manager'
```

Exactly one Cairn MCP entry per selected application, ownership recorded as `manager`, zero
`conflicting_owner` findings, and unrelated CC Switch configuration unchanged (SC-112).

Then switch provider inside CC Switch and re-run `cairn doctor`: every Cairn resource still
healthy, zero duplicates (SC-113).

**Skill distribution from a development build** (hermetic):

```bash
cairn integration distribute --via cc-switch --resource skill --apps claude
```

Must fail with `unpublished_skill_ref` and exit `1`. CC Switch's downloader only accepts
`refs/heads` and silently falls back to `main` on a miss, so emitting an unpublished ref would
install the wrong Skill with no error. The MCP resource still distributes from the same build.

**Skill distribution from a released build** *(live, release evidence — D29a)*:

```bash
# 1. the release published the branch (it may point at an earlier release's commit —
#    the branch names content, not a release)
git ls-remote https://github.com/Vellixia/Cairn "refs/heads/skill-release/*"

# 2. it fetches the way CC Switch fetches
curl -fsSL -o /tmp/s.zip \
  "https://github.com/Vellixia/Cairn/archive/refs/heads/skill-release/1-c07d4419b2ae.zip"
unzip -p /tmp/s.zip '*/skills/cairn/SKILL.md' | grep cairn_skill_revision

# 3. the same algorithm, from the workspace, agrees with both
cargo run -q -p cairn-integrate --bin skillref -- --json

# 4. Cairn emits that branch, not a SHA or a tag
cairn integration distribute --via cc-switch --resource skill --apps claude --dry-run
```

**Must be true**

- The branch exists. When the Skill revision did not change since the previous release,
  `git ls-remote` shows the **same commit as before** — it was reused, not moved.
- The archive contains `skills/cairn/SKILL.md` whose `metadata.cairn_skill_revision` equals the
  revision in the branch name **and** the revision embedded in the running binary.
- The emitted deep link's `branch=` is that branch — never a commit SHA, never a tag, never
  `main`.

Then complete the import in CC Switch and verify:

```bash
grep cairn_skill_revision ~/.claude/skills/cairn/SKILL.md
cairn doctor claude-code --json | jq '.data.agents[0].resources[]
  | select(.kind=="skill") | .condition'          # → "healthy"
```

The installed revision equals the embedded revision, and doctor reports the Skill `healthy`
rather than `outdated` — which is the check that would have caught a silent `main` fallback.

---

## US6 — Work continues in a different agent *(hermetic + live)*

The release-blocking continuity scenario. Hermetically, the three agents are three
`agent_session_key`s driven through their adapters with recorded payloads (D41):

```bash
cargo test -p cairn-e2e --test us6_cross_agent
```

The scenario:

1. A Claude-shaped session records decision **D**, failure **F**, and produces handoff **H**.
2. A Codex-shaped session opens the same repository → resolves the **same project and task**,
   and its briefing contains D, F, and H.
3. It records procedure **P**.
4. An OpenCode-shaped session opens → receives D, F, P, and the latest handoff.

**Must be true**

- Exactly one project for the repository (SC-108).
- Every retrieved item names the agent and session that produced it.
- Zero export, import, or copy steps.
- Scope and ranking use only project / branch / task / session — filtering by producing agent
  is impossible because nothing stores it as a scope (FR-189).

**Live**: the same sequence with real Claude Code, Codex, and OpenCode on one repository.

---

## US7 — Diagnose and repair *(hermetic)*

Break one thing per agent, then:

```bash
cairn doctor --json > /tmp/health.json
```

| Break | Expected condition |
|---|---|
| Delete a Cairn hook entry | `missing` |
| Edit text inside the managed block | `modified` |
| Delete the block's `end` marker | `damaged_markers` |
| Set `schema=0` in the marker | `outdated` |
| Add a second Cairn MCP entry at another scope | `duplicated` |
| Point a resource's record at the other owner | `conflicting_owner` |
| Truncate `~/.codex/config.toml` mid-table | `malformed_config` |
| Reset Codex hook trust | `installed_not_activated` |

Each names the exact resource (SC-114). Then:

```bash
cairn repair --dry-run     # plan only, zero writes
cairn repair
```

**Must be true**

- `missing`, `outdated`, and `duplicated` are fixed; nothing else is touched (SC-115).
- `modified`, `damaged_markers`, `conflicting_owner`, and `malformed_config` are explained
  and unchanged.
- A second `cairn repair` reports nothing to do.
- Reformatting the managed block without changing its meaning is reported `healthy`, not
  `modified` (SC-130).

Forced repair:

```bash
cairn repair claude-code --force
```

A recovery artifact is written containing **only** the previous managed block body; the
enclosing `CLAUDE.md` is not copied, and no credential from any inspected file appears
anywhere in `$CAIRN_HOME` (SC-133).

---

## US8 — Any MCP-compatible agent *(hermetic)*

```bash
cairn integration export mcp > /tmp/mcp.json
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
  | cairn mcp | jq '{v: .result.protocolVersion, has_instructions: (.result.instructions != null)}'
```

**Must be true**

- `protocolVersion` is `2025-06-18` (D34) and `instructions` is present.
- `tools/list` returns exactly six tools; a seventh fails the build (SC-106).
- All six work end to end against the daemon.
- The reported level is `MCP_ONLY`, with automatic lifecycle and automatic capture named as
  unavailable — never described as full (SC-107).

---

## US9 — Removal and ownership change *(hermetic)*

```bash
sha256sum ~/.codex/config.toml AGENTS.md > /tmp/before
cairn disconnect codex
```

**Must be true**

- Cairn's Codex lifecycle, Skill, and directly-owned MCP entry are gone.
- **The `AGENTS.md` block is still there**, because OpenCode is still bound to it — and
  `cairn doctor opencode` still reports its instructions `healthy` (SC-137). Only Codex's
  binding was removed.
- Every unrelated entry in every touched file is byte-identical to `/tmp/before` (SC-116).
- Every project, task, session, observation, memory, and handoff still exists.
- Claude Code and OpenCode are unaffected.

Then disconnect the last consumer and confirm the resource is finally removed:

```bash
cairn disconnect opencode
grep -c 'cairn:managed:begin' AGENTS.md    # → 0, and the developer's content remains
```

Manager-owned state survives a native disconnect:

```bash
cairn disconnect codex --only mcp   # with the MCP entry owned by CC Switch
cairn doctor codex --json | jq '.data.agents[] | select(.agent=="codex") | .resources'
```

Codex's record is still present with the manager-owned resource and its pending action, so the
withdrawal stays verifiable (SC-137, FR-244).

Ownership migration:

```bash
cairn integration migrate codex skill --to cc-switch --dry-run
cairn integration migrate codex skill --to cc-switch
```

At every step exactly one resource is effective; the state is reported `migrating` with its
source and target; killing the process mid-migration leaves a resumable row, and
`--resume`/`--abort` both end with exactly one owner (SC-117).

---

## US10 — Several agents at once *(hermetic + live)*

```bash
cargo test -p cairn-e2e --test us10_concurrency
```

Covers: Claude + Codex in one worktree; Codex + OpenCode in one worktree; two worktrees of
one repository with a different agent in each; an unattributable request; simultaneous memory
writes.

**Must be true**

- Two distinct active sessions per pair, keyed by each agent's own session identifier.
- 100% of observations and memories carry the provenance of the session that produced them;
  zero events routed to the wrong session (SC-109).
- An unattributable request returns `ambiguous_session` naming the candidates — never a guess
  and never a fabricated session.
- One project across all of it.
- No duplicate integration-record rows under concurrent connects.

**Live**: Claude Code and Codex running simultaneously in one checkout.

---

## Measured checks

| Check | Command | Bound |
|---|---|---|
| Capture latency per adapter | `cargo test --release -p cairn-e2e --test perf_capture` | Feature 001's SC-007 bounds hold for all three adapters, ≥200 invocations each (SC-122) |
| Codex session-close budget | `cargo test --release -p cairn-e2e --test perf_session_close` | ≥100 boundaries, 100% inside Codex's own budget, zero overruns (SC-128) |
| Injected-failure recovery | `cargo test -p cairn-e2e --test recovery_injected` | Handler timeout, handler crash, daemon unavailable — every session ends with a durable handoff, zero agent sessions disrupted (SC-129) |
| Sealed close actually lands | `cargo test -p cairn-e2e --test handoff_lands_without_restart` | ≥100 sealed closes, 100% with a durable handoff inside the bound and **no daemon restart**; forced permanent failure is reported, not silent (SC-136) |
| Shared resource survives one disconnect | `cargo test -p cairn-integrate --test fixtures shared_binding` | Disconnecting one of two bound agents keeps the resource; the last one removes it; manager-owned state survives (SC-137) |
| Capability evidence gates FULL | `cargo test -p cairn-e2e --test capability_evidence` | An unobserved FULL-required runtime capability blocks FULL and is named as awaited; a detected version change discards observation evidence but keeps introspection evidence; observing everything restores FULL; a session start that delivered no context does not establish context delivery while a degraded one does; a synthesized or single-event identifier does not establish stability (SC-138) |
| Skill branch identifies content, and is never moved | `.github/workflows/release.yml` `publish-skill` job, plus `cargo test -p cairn-integrate revision` | Release **A** introduces revision `R` → branch created at A. Release **B** changes no Skill file → same `R`, branch still at A, **not moved**, content re-verified, release succeeds. Release **C** introduces `S` → a new branch, `…-R` untouched. A branch whose fetched content does not match its name → the release **fails**. No force update in any case (D29a) |
| Skill revision is not circular | `cargo test -p cairn-integrate revision` | The checked-in `metadata.cairn_skill_revision` equals the value computed by the canonical algorithm; the self-field is normalized to `<REVISION>` before hashing; the same function is what `skillref`, doctor, and the release job call (D29b) |
| Configuration preservation | `cargo test -p cairn-integrate --test fixtures` | ≥20 fixtures, connect→disconnect returns 100% of non-Cairn bytes (SC-104) |
| Preview writes nothing | `cargo test -p cairn-integrate --test dry_run_is_inert` | Checksums of every candidate file identical before and after (SC-118) |
| Secrets stay out | `cargo test -p cairn-e2e --test privacy_integration` | Seeded credentials appear in zero artifacts, logs, diagnostics, or sync payloads (SC-133, SC-120) |
| Conversation text stays out | `cargo test -p cairn-e2e --test privacy_payloads` | Seeded assistant text and prompts appear in zero stored records (SC-121) |
| Browser regression | CI `web-e2e` job | Desktop and mobile against a release-build server (SC-125) |

## Measurements on record

Filled in during implementation, as Feature 001 did:

| Measurement | Value |
|---|---|
| Capture latency, Claude adapter (median / p95) | — |
| Capture latency, Codex adapter (median / p95) | — |
| Capture latency, OpenCode adapter (median / p95) | — |
| Codex seal phase duration (median / p95 / max) | — |
| Handoff-after-seal latency (p50 / p99) | — |
| Rendered contract size (characters) | — |
| Per-user hook cost in an unmanaged repository | — |
