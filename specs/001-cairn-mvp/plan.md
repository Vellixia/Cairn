# Implementation Plan: Cairn MVP

**Branch**: `001-cairn-mvp` | **Date**: 2026-08-07 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/001-cairn-mvp/spec.md`

## Summary

Deliver a Cairn that a developer installs, connects to Claude Code, and immediately
benefits from: sessions start automatically against real Git state, work is captured as
structured observations, session boundaries produce derived handoffs, and the next
session opens with a bounded briefing assembled from memory and the previous handoff.
Memory is explicitly scoped (project / branch / task / session) and recalled with exact
filters plus lexical search ranked scope-first. Sharing with teammates is opt-in per
project and rides a transactional outbox to a small Axum + PostgreSQL server with a
Next.js UI for inspecting and curating what Cairn knows.

The build is sliced vertically. Every phase ends with something runnable: US1 gives a
working handoff, US2 makes the next session informed, US3 makes knowledge durable, US4
sharpens scope, US5 makes storage controllable, US6 adds sharing, US7 makes it visible.
Those are demoable checkpoints, not release gates — **Feature 001 is the Cairn MVP, and it
is complete only when all seven stories ship.**

## Technical Context

**Language/Version**: Rust (workspace, pinned via `rust-toolchain.toml`); TypeScript for
the web UI

**Primary Dependencies**: Tokio, SQLx, Axum + Tower, serde, `rmcp` (MCP), `tracing`,
`clap`, argon2; Next.js App Router, Tailwind, shadcn/ui, TanStack Query

**Storage**: SQLite (local, WAL, FTS5) via SQLx; PostgreSQL (server) via SQLx

**Testing**: `cargo test` / `cargo nextest` with real temporary Git repositories and real
SQLite files; PostgreSQL in Docker for server tests; Playwright for the UI acceptance
walkthrough

**Target Platform**: macOS and Linux developer machines for `cairn`/`cairnd`; Linux
container for `cairn-server`; modern browsers for the UI

**Project Type**: Multi-binary Rust workspace (CLI + daemon + server) plus a separate web
application

**Performance Goals**: Capture hooks ≤10 ms median and ≤25 ms p95, inside a 250 ms
deadline; `SessionStart`
context path ≤1,500 ms deadline with a clean reduced-context fallback; briefing assembly
under 200 ms warm

**Constraints**: Fully offline for all local paths; briefing never exceeds its configured
Cairn-estimated-token budget (default target 2,000–4,000 estimated tokens); observation
payloads bounded (default 4 KB);
raw observations never leave the machine; Cairn failures never abort an agent session

**Scale/Scope**: One developer per local store; tens of projects, thousands of sessions,
low hundreds of thousands of observations locally; small teams per shared project

## Constitution Check

*GATE: evaluated before Phase 0 and re-evaluated after design.*

| Principle | Assessment |
|---|---|
| I. Usable MVP First | PASS — seven vertically sliced stories, each ending runnable; US1 is the first usable slice, and Feature 001 is the MVP only once all seven are delivered. No horizontal "build all the layers" phase. |
| II. Simple Architecture | PASS — three binaries, two datastores, one queue-free sync mechanism. No broker, cache tier, or extra service. Six crates, each with a distinct boundary (see Structure Decision). |
| III. Local-First Reliability | PASS — every capture, recall, briefing, handoff, and search path is local-only. Sync is additive. Hooks fail soft by contract. |
| IV. Project-Scoped Memory | PASS — `scope` + `scope_key` on every memory, provenance to session and observations, scope-first ranking. |
| V. Privacy by Default | PASS — no transcripts, bounded payloads, redaction before write, exclusions, `local_only`, scoped deletion with tombstone propagation, opt-in sharing, and raw observations that never leave the machine even as evidence. |
| VI. Deterministic Data Boundaries | PASS — repository state derived from Git CLI, local repository instance separated from server-assigned shared project identity, UUIDv7 record identity, idempotent sync by key, deterministic budget compliance. |
| VII. Testable Behavior | PASS — each story's Independent Test has a matching end-to-end test in the quickstart; bounds, redaction, deletion scope, and estimator error are asserted rather than assumed. |

**Re-check after Phase 1 design**: PASS. The design added no component beyond those
listed. The one judgment call — six crates rather than one — is recorded in Complexity
Tracking below.

## Project Structure

### Documentation (this feature)

```text
specs/001-cairn-mvp/
├── plan.md              # This file
├── spec.md              # Feature specification
├── research.md          # Decisions D1–D13
├── data-model.md        # Entities, relationships, what syncs
├── quickstart.md        # Acceptance walkthrough per user story
├── contracts/
│   ├── mcp-tools.md     # The six MCP tools
│   ├── agent-integration.md  # CLI surface and Claude Code hook contract
│   └── server-api.md    # HTTP API: auth, sync, read
└── tasks.md             # Generated by /speckit-tasks
```

### Source Code (repository root)

```text
crates/
├── cairn-core/          # Domain types and enums, wire types (serde), redaction,
│                        # context budgeting, handoff synthesis. No I/O.
├── cairn-git/           # Git CLI adapter: repository identity, branch, commit,
│                        # worktree, working-tree status
├── cairn-store/         # SQLite schema + migrations, repositories, FTS5 search,
│                        # transactional outbox
├── cairnd/              # Daemon binary: IPC server, session lifecycle, capture
│                        # pipeline, staleness reconciliation, sync worker
├── cairn/               # CLI binary: commands, `cairn hook <event>`, `cairn mcp`
└── cairn-server/        # Axum binary: auth, sync ingest, read API, PostgreSQL
                         # migrations

web/                     # Next.js App Router UI
├── app/                 # projects, project overview, tasks, sessions, memory, sync
├── components/
└── lib/                 # API client, TanStack Query hooks

tests/                   # Workspace end-to-end tests, one module per user story
.github/workflows/       # CI: fmt, clippy, tests on macOS and Linux
Cargo.toml               # Workspace manifest
rust-toolchain.toml      # Pinned toolchain
rustfmt.toml
docker-compose.yml       # PostgreSQL for local server development
```

**Structure Decision**: A single Rust workspace with six crates, plus a separate
`web/` application. `cairn-core` holds the domain and wire types so the daemon and the
server cannot drift; `cairn-git` and `cairn-store` isolate the two I/O surfaces that need
independent testing; `cairnd`, `cairn`, and `cairn-server` are the three shipped binaries.
The MCP server is a subcommand of `cairn`, not a fourth binary, so connecting an agent
never means installing a second runtime (D1, D5).

## Phasing

Each phase is a vertical slice that ends runnable.

Checkpoints below are demoable states, not shipping points. The MVP is the whole table.

| Phase | Delivers | Runnable proof |
|---|---|---|
| Setup | Workspace, toolchain, CI skeleton | `cargo build --workspace` |
| Foundational | Domain types, Git adapter, SQLite schema, daemon + CLI skeleton, IPC | `cairn init`, `cairn status` |
| US1 (P1) | Session lifecycle, capture pipeline, hooks, handoff synthesis | `cairn handoff show` after a real session |
| US2 (P1) | Briefing assembly, budgeting, `SessionStart` injection, `cairn_context` | Next session opens informed |
| US3 (P2) | Memory model, FTS5 search, scope ranking, `cairn_remember`/`cairn_search` | Cross-session recall |
| US4 (P2) | Tasks, session binding, task-scoped context and memory | Task-led briefing |
| US5 (P2) | Exclusions, redaction hardening, `local_only`, deletion | Seeded-secret fixture passes |
| US6 (P3) | Server, auth, outbox sync, membership | Two members share memory |
| US7 (P3) | Web UI screens | Teammate curates memory in a browser |
| Polish | Docs, install path, performance verification | Quickstart runs end to end |

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Capture-hook overhead shows up in the agent's latency | 250 ms deadline, parse-and-forward only, all work in the daemon; SC-007 bounds hook latency absolutely (median ≤10 ms, p95 ≤25 ms), measured in Polish |
| `SessionStart` cannot assemble a briefing in time on a cold daemon or a large repository | Separate 1,500 ms deadline, and a clean fallback that starts the session with reduced context rather than blocking (D15); the value is validated against real repositories in Polish |
| A shared memory leaks the file paths and commands behind it | Evidence travels as identifiers, a count, and a digest; the outbox has no observation entity type and the server rejects observation fields (D9, FR-055) |
| Two clones of one repository become two shared projects | Shared identity is server-assigned and established by an explicit `cairn link`, never derived from a path (D14) |
| Derived handoffs read as thin or generic | Handoff synthesis is built against the US1 acceptance scenario first, with the failing-test case as the shape to satisfy |
| Briefing budget estimate drifts from real tokenization | The budget is denominated in Cairn-estimated tokens and compliance is guaranteed only against that estimator; the estimator is conservative and its error against a real tokenizer is measured and recorded (D8) |
| Cairn claims to know a session died when the integration cannot tell it | No liveness detection: sessions leave `active` only on `SessionEnd`, an explicit end, or daemon start, and resume if an event arrives afterwards (D16) |
| Scope-first ranking hides a relevant project-wide fact | Ranking is bucketed, not filtered — every scope is represented before truncation; `cairn_search` exposes explicit scope override |
| Redaction misses a secret shape | Redaction runs before write, patterns are a documented extensible set, and a seeded fixture asserts the mechanism (not exhaustiveness) |
| SQLite contention across worktrees | WAL, busy timeout, per-project transaction serialization (D12) |

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| Six crates rather than a single crate | `cairn-core` must be depended on by both the daemon and the server without dragging SQLite into the server or PostgreSQL into the daemon; `cairn-git` and `cairn-store` are the two I/O surfaces that need to be tested independently of the daemon | A single crate would force the server to compile SQLite and would make it impossible to test Git and storage adapters without booting the daemon; two crates (`core` + `bin`) would put three binaries' dependency trees into one compilation unit |
| A separate web application alongside the Rust workspace | Six product screens with search and detail views; the constitution's "usable MVP" bar includes a teammate curating memory without a terminal | Server-rendered templates in Axum would make the memory search and filter interactions materially worse for no reduction in moving parts |
