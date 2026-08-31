# Implementation Plan: Server-Authoritative Autonomous Memory

**Feature Directory**: `specs/005-server-authoritative-autonomous-memory`
**Git Branch**: `feature-005-spec`
**Baseline**: `origin/main` @ `f76a9fec8a786a76dc7ffa1b0b0daf96aae08b15`
**Spec**: [spec.md](./spec.md) — 272 functional requirements, 63 success criteria
**Constitution**: v1.2.1
**Created**: 2026-08-30
**Status**: Draft — plan only. No tasks, no implementation.

## Summary

Move authority for durable knowledge from the client's SQLite store to the server's
PostgreSQL, capture agent activity richly and vendor-natively, transform it locally into
privacy-approved typed events, consolidate those events into knowledge on the server without
anyone asking, deliver knowledge back into agent sessions automatically, and report truthfully
what actually happened at every stage.

The plan adds no new process, service, datastore, broker or worker platform. Everything runs
in the two processes that already exist: `cairnd` on the developer's machine and `cairn-server`
centrally.

## Technical Context

**Language**: Rust (workspace: `cairn`, `cairnd`, `cairn-server`, `cairn-core`, `cairn-store`,
`cairn-integrate`, `cairn-git`, `cairn-sys`), TypeScript/Next.js 15 + React 19 for `web/`.
**Storage**: PostgreSQL (server, canonical), SQLite (edge spool, cache, machine state).
**Local schema**: v7 → v8. **Server schema**: v3 → v4.
**Transport**: HTTPS to the server; Unix socket / named pipe between hook and daemon.
**Testing**: the existing `tests/` harness, which spawns prebuilt binaries against real repos.

### Phase 0 decisions

Every open decision named in the planning brief is resolved here. Details live in the
contracts; this table is the index and the rationale.

| Decision | Resolution | Why |
|---|---|---|
| Event schema | Typed closed union, per-kind content, 21 kinds | [contracts/safe-events.md](./contracts/safe-events.md) |
| Field naming | No field name the sync boundary refuses; explicit map | FR-777a1; drift between two boundaries on one server is forbidden |
| Numeric bounds | Stated as numbers, enforced both sides | FR-773, SC-743 need a number to test |
| Event identity | Daemon-assigned per-session ordinal, then UUIDv5 | Hooks are separate short-lived processes and cannot share a counter; the daemon can, transactionally |
| Edge spool | New `event_spool` table reusing the outbox claim protocol | FR-783; the protocol is proven and already handles per-author claims |
| Consolidation | In-process Tokio task, claim/reclaim, bounded | FR-793a–d; the server had no background execution at all |
| Extractor | Trait; **deterministic rule-based baseline ships as default** | No third-party egress in v1; hosted extraction stays optional and gated |
| Key normalization | Extend the existing deterministic normalizer | FR-796a–c; the problem is syntactic |
| Verification | Client reports results as `remote_attested`; server derives state | FR-811b/FR-811h — bearer auth and an HTTP route do not prove which verifier executed |
| Retrieval | Session-open and prompt-time, with per-session dedup | FR-838c; both committed agents document both points |
| OpenCode | Capture only; delivery declined, reason recorded | FR-838b — the hooks exist but are beta |
| Migration | Explicit server authority mode + `upgrade_required` | FR-876; retirement must not wait on a dormant device |

### The extractor decision, stated plainly

**Feature 005 ships a deterministic, rule-based extractor as the default and only supported
baseline. No hosted model provider is selected, and none is required.**

This was not a cost decision. Constitution v1.2.1 Principle V makes a third-party extractor a
second recipient at a second boundary, requiring naming, disclosure and per-project scoping,
and forbids assuming a provider's retention behaviour. That obligation is satisfiable, but it
is not satisfiable *by assumption*, and nothing in the acceptance story requires a model:
the rules in [contracts/extraction.md](./contracts/extraction.md) produce the failure,
decision and procedure candidates the end-to-end scenario calls for, from event structure
alone.

A model extractor remains permitted behind the same trait. Before one is enabled, the gate in
`contracts/extraction.md` §5 must be satisfied against the actual provider, model and endpoint:
retention, training use, zero-retention eligibility, caching, project and account isolation,
required user disclosure, and behaviour when a compliant mode is unavailable. If compliance
cannot be established, the deterministic extractor is what runs. This keeps the privacy
contract intact rather than trading it for extraction quality.

## Constitution Check

### Pre-design gate (v1.2.1)

| Principle | Assessment |
|---|---|
| I. Usable MVP First | **PASS** — the feature is sliced agent → daemon → server → web and ends in a runnable end-to-end scenario on a real repository. |
| II. Simple Architecture | **PASS** — no new process, service, broker, datastore or worker platform. Consolidation is deferred work inside the existing server, bounded and restartable, which II explicitly permits. The deterministic extractor adds no dependency at all. |
| III. Fail-Soft, Server-Authoritative | **PASS** — PostgreSQL canonical, SQLite demoted to spool/cache/machine state; the agent never blocks on Cairn; deleting the local store loses nothing the server accepted. |
| IV. Explicitly Domained Memory | **PASS** — canonical patterns are personal-domain records of type `pattern`; `PatternRef` changes reference shape, not domain; generated reference identity includes the full domain for every `KnowledgeRef`. |
| V. Privacy by Default | **PASS** — raw material never leaves the machine; only gate-approved structure crosses; the baseline extractor introduces no second recipient. |
| VI. Deterministic Data Boundaries | **PASS** — event identity, ingest and consolidation are all idempotent and clock-independent. |
| VII. Testable Behavior | **PASS** — every success criterion names an observation point; adversarial corpora are required rather than described. |
| VIII. Project Truth Not Displaceable | **PASS** — budget reserve and domain separation untouched. |
| IX. Autonomy Under Governance | **PASS** — the extractor proposes; Cairn decides durability, domain, scope, verification and supersession deterministically. |
| X. Report What Is Established | **PASS** — every authenticated retrieval creates a `requested` trace before generation; generation and later hook transmission are separate state transitions; only an authenticated idempotent outcome report can record `transmitted` and write `delivered_context`; receipt remains `unavailable / no evidence`. |
| XI. Identity Established, Never Asserted | **PASS** — account and project come from the authenticated credential; the session an event names is verified server-side. |

### Post-design gate (v1.2.1)

Re-evaluated after the contracts and data model were written. All eleven still **PASS**. Three
required a design change to keep passing, recorded here rather than in a footnote:

- **Principle V** would have failed if extraction ran on raw material or forwarded to an
  unverified third party. Resolved by moving extraction behind the boundary and shipping a
  deterministic baseline.
- **Principle XI** would have failed if consolidation had attributed personal knowledge from
  event body content. Resolved by binding owner from the account recorded at ingest.
- **Principle VI** would have failed if a reclaimed consolidation batch re-derived a second
  corroboration record. Resolved by deriving candidate identity from the project, the
  session and the normalized keys — deliberately **not** from the event set, which is not
  stable across a reclaim — making re-execution an upsert.

A falsification pass after the contracts were written found three further gate assessments
unearned, and each was repaired rather than argued away:

- **Principle II** claimed PASS while no artifact stated a bound, refill or invalidation policy
  for the briefing cache the design requires. `contracts/retrieval-delivery.md` §12.3 now states
  all three.
- **Principle VI** claimed PASS on an event identity derived from a counter that reset when the
  spool drained, and on a candidate identity that changed when a reclaimed batch saw different
  evidence. Both derivations were replaced (`data-model.md` §1.4, `contracts/consolidation.md` §7).
- **Principle X** claimed PASS while any project member could assert `authority = cairn` on a
  verification report. Feature 005 now assigns every client-reported result
  `remote_attested`; `cairn` is server-executed only, and no route produces `remote_cairn`
  without a stronger evidence path (`contracts/verification-summary.md` §4).

### Principle IV and reusable patterns

IV ¶1 requires every durable record to name `project`, `personal` or `team`; the 1.1.0
amendment history says the same explicitly. Feature 005 satisfies that rule directly:
`shared_patterns` is a canonical **personal-domain** record of type `pattern`, with
`domain = personal` and `owner_user_id`. It is project-independent because personal knowledge
is project-independent, not because the record lacks a domain.

`PatternRef(pattern_id)` remains distinct from `KnowledgeRef(domain, id)` because a pattern has
its own table and lifecycle. In a polymorphic reference row, `ref_kind = pattern` therefore uses
a null *reference-domain slot*; the referenced `shared_patterns` row still carries
`domain = personal`. This encoding cannot be used to call the durable record domain-less.

Owner-only retrieval prevents ambient visibility. Widening the content is a separate
team-domain proposal followed by human-admin ratification; the pattern itself remains personal
and owner-only. This uses the three existing domains, satisfies both paragraphs of Principle
IV, and needs no constitutional amendment.

**Complexity Tracking**: no principle is violated, so the table is empty.

## Project Structure

### Documentation (this feature)

```text
specs/005-server-authoritative-autonomous-memory/
├── spec.md                  # approved contract (272 FR, 63 SC)
├── research.md              # current-main audit + vendor evidence (preserved, extended)
├── plan.md                  # this file
├── data-model.md            # entities, local schema v8, server schema v4
├── quickstart.md            # how the feature is demonstrated end to end
├── checklists/requirements.md
└── contracts/
    ├── safe-events.md       # event model, field names, bounds, ingest API, identity
    ├── knowledge-commands.md# post-cutover mutation model: commands, offline queue, drain
    ├── consolidation.md     # worker, claims, candidate governance, idempotency
    ├── extraction.md        # extractor trait, deterministic baseline, hosted gate
    ├── retrieval-delivery.md# per-agent delivery points, dedup, traces
    ├── verification-summary.md # run reports, server-side derivation
    ├── migration-cutover.md # authority mode, upgrade_required, migration phases
    └── web-control-plane.md # read APIs and screens
```

### Source code

```text
crates/cairn-core/src/
├── event.rs        # NEW SafeCanonicalEvent, kinds, per-kind content, bounds
├── eventid.rs      # NEW deterministic event identity
├── knowledge.rs    # EXTEND key normalization (separator folding)
├── lifecycle.rs    # EXTEND canonical events beyond seven
├── redact.rs       # unchanged
└── promotion.rs    # unchanged

crates/cairn-integrate/src/agents/
├── claude_code.rs  # EXTEND prompt-submit, subagent, richer tool payloads
├── codex.rs        # EXTEND same
├── opencode.rs     # EXTEND capture only; no beta delivery dependency
└── mod.rs          # REPLACE two-field allowlist with per-vendor field maps

crates/cairn-store/
├── migrations/0008_safe_events.sql   # NEW event_spool, session_event_seq, command_spool,
│                                     #     capture dispositions, authority_mode,
│                                     #     retained_local, legacy_pattern_claims,
│                                     #     migration_state
└── src/spool.rs                      # NEW claim protocol reusing outbox semantics

crates/cairn-server/
├── migrations/0004_autonomous_memory.sql  # NEW events, consolidation, traces, health
├── src/events.rs         # NEW ingest endpoint + server-side validation
├── src/consolidate.rs    # NEW in-process worker
├── src/extract.rs        # NEW extractor trait + deterministic baseline
├── src/retrieve.rs       # NEW retrieval + requested/generated/terminal trace transitions
├── src/verifysummary.rs  # NEW run report ingest, state derivation
├── src/commands.rs       # NEW knowledge command API (create/supersede/relate/…)
└── src/api.rs            # EXTEND command/read APIs + authenticated transmission outcome

crates/cairnd/src/
├── capture.rs      # EXTEND build events, assign identity, spool
├── deliver.rs      # NEW session-open/prompt delivery + later outcome reporting
├── sync.rs         # EXTEND shared event/command drain primitive
└── migrate005.rs   # NEW migration phases

web/app/(app)/      # dashboard, activity, memory detail, retrievals, agents, team, system
```

## Phase ordering

Vertical slices, each ending in something runnable, per Principle I.

1. **Event spine** — event model, identity, spool, ingest API, server persistence. Ends with:
   events from a real session visible in PostgreSQL.
2. **Rich capture** — per-vendor field maps, new event kinds, prompt-submit and subagent
   capture, dispositions. Ends with: the capture matrix is true.
3. **Consolidation** — worker, claims, deterministic extractor, candidate governance. Ends
   with: knowledge appears without anyone asking.
4. **Retrieval** — server-side selection, session-open and prompt-time delivery, traces, and the
   authenticated post-response transmission outcome. Ends with: a second session starts already
   knowing, failed retrievals remain visible, and only transmitted items enter delivery dedup.
5. **Verification and health** — run reports, derived summaries, evidence-based health. Ends
   with: status tells the truth.
6. **Migration and cutover** — authority mode, `upgrade_required`, explicit one-time legacy
   pattern ownership claims persisted before delivery, migration phases. Ends with: an existing
   installation upgrades safely without attributing owner-less patterns to active credentials.
7. **Web control plane** — dashboard, detail, traces, health, team and admin. Ends with: the
   whole lifecycle is visible.

Phases 1–2 and 5 may proceed in parallel with 7 once the read APIs are fixed. Phase 6 must
follow 1–5, because it verifies canonical possession of what those phases produce.

## Risks

| Risk | Mitigation |
|---|---|
| Richer local capture breaks the millisecond capture deadline | FR-749a requires it fit or the budget be restated; phase 2 measures before widening further. Deadline drops become observable (FR-749c) rather than silent. |
| Deterministic extraction produces thin knowledge | Accepted for v1. The rules target the acceptance story specifically; extraction quality is the one thing a later model extractor improves, behind an unchanged trait. |
| Consolidation starves request serving | Bounded concurrency, batch size and pool share, stated numerically in `contracts/consolidation.md` §6. |
| Cutover strands a legacy device | `upgrade_required` leaves local data intact and is reversible by upgrading; no data is destroyed by refusing. |
| Personal and team knowledge have no server-side text index today; FR-806 puts extending it in scope | Phase 3 adds a GIN index over `personal_knowledge.content` and `team_knowledge.content`, mirroring the existing `memories_search` index. Not assumed to exist. |
| Server-side retrieval adds latency to session open | Declared degradation levels (FR-836) and a stated deadline; a slow server degrades the briefing, never the agent. |
