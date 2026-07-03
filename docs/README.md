---
title: Documentation Library
type: index
status: living
updated: 2026-07-01
---

# Cairn Documentation Library

This is the catalog of every doc in this repo. If you're new, start with the root
[README.md](../README.md) (for users) or [AGENTS.md](../AGENTS.md) (for AI agents) — they
route you here for anything deeper.

Writing a new doc? Read [CONVENTIONS.md](CONVENTIONS.md) first — it explains where things go
and which template to use.

## Reference — how Cairn works

Living docs, edited in place as the system evolves.

| Doc | What |
|---|---|
| [architecture.md](reference/architecture.md) | Crate graph, MCP tool surface, API endpoints, Docker topology, config |
| [decisions.md](reference/decisions.md) | Architecture decision records (ADR log) |
| [vision.md](reference/vision.md) | Product vision, five pillars, core principles |

## Guides — how to do X with Cairn

| Doc | What |
|---|---|
| [admin.md](guides/admin.md) | Bootstrap and manage the admin account, dashboard surface |
| [upgrading.md](guides/upgrading.md) | Version-to-version upgrade notes |
| [web-auth.md](guides/web-auth.md) | Web dashboard auth model, session vs device tokens |
| [ide-integration.md](guides/ide-integration.md) | Connect Claude Code, Codex, and OpenCode to a Cairn server |

## Testing — how Cairn is verified

| Doc | What |
|---|---|
| [overview.md](testing/overview.md) | Rust integration tests + dashboard flow tests, how they fit together |
| [e2e.md](testing/e2e.md) | Scenario-based end-to-end test harness |
| [benchmarks.md](testing/benchmarks.md) | Token-savings benchmark methodology + measured numbers |
| [flows.md](testing/flows.md) | Dashboard flow checklists (chrome-devtools MCP) |
| [run-agent-tests.md](testing/run-agent-tests.md) | Meta-instructions for driving the flow checklists |
| [live-e2e/](testing/live-e2e/README.md) | Live walk-through docs, one per user-facing surface |
| [findings/](testing/findings/README.md) | Bug registry (`open/` / `resolved/`) + dated run archives |

## Planning — where Cairn is going

| Doc | What |
|---|---|
| [roadmap.md](planning/roadmap.md) | Status tracker: done, in progress, next, per phase |
| [plans/v0.6.0.md](planning/plans/v0.6.0.md) | Released — the cleanup sprint |
| [plans/v0.7.0.md](planning/plans/v0.7.0.md) | Released — engine intelligence + dashboard UX |
| [plans/v0.8.0.md](planning/plans/v0.8.0.md) | Draft — scoped memory, SurrealDB, LLM intelligence, RAG, autopilot |

## Audits — point-in-time reviews

Frozen at the date shown; findings are not rewritten after the fact. See each doc's
"Fix-status update" section for what has since been resolved.

| Doc | What |
|---|---|
| [report.md](audits/report.md) | Consolidated audit report (2026-06-15) with fix-status tracking |
| [build-runtime.md](audits/build-runtime.md) | Build, runtime, and config audit — raw detail behind `report.md` |
| [deps-ci.md](audits/deps-ci.md) | Dependency + CI/supply-chain audit — raw detail behind `report.md` |
| [security-arch.md](audits/security-arch.md) | Security + architecture deep audit — raw detail behind `report.md` |
| [v6.1.0.md](audits/v6.1.0.md) | Test & security audit (2026-06-24), 21 crates / 25.6k LOC |
| [recommendations.md](audits/recommendations.md) | Forward-looking backlog written after v6.1.0 |

## Archive — superseded / historical

Kept for history. Not part of active navigation.

| Doc | What |
|---|---|
| [p0-security-plan.md](archive/p0-security-plan.md) | P0 security & build fixes — completed, superseded by the roadmap |
| [plan-v0.5.0.md](archive/plan-v0.5.0.md) | The v0.5.0 "smart memory" plan |

## Meta

| Item | What |
|---|---|
| [CONVENTIONS.md](CONVENTIONS.md) | Doc authoring rules: folder map, template picker, naming, frontmatter |
| [_templates/](_templates/) | Seven starting-point templates — plan, reference, guide, ADR, audit, walk, finding |
