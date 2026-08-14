# Traceability: Cairn Project Intelligence

**Feature**: `003-project-intelligence` | **Date**: 2026-08-14

Every functional requirement has an owning design surface and a phase. Every success criterion has a
named test. This is the pre-task gate: a requirement with no owner, or a criterion with no test, is a
planning defect.

**Spec**: 158 FR · 28 SC. **Coverage**: 158/158 FR owned · 28/28 SC tested.

## Legend

| Crate | Short |
|---|---|
| `cairn-core` | core |
| `cairn-store` | store |
| `cairn-git` | git |
| `cairnd` | daemon |
| `cairn` (CLI + MCP) | cli |
| `cairn-server` | server |

Phases are from [plan.md](./plan.md) §Phasing: **A** Domain, **B** Knowledge, **C** Evidence &
verification, **D** Drift, **E** Context & continuity, **F** Tasks, **G** Sync & multi-device,
**H** Patterns & evidence of the whole.

---

## Functional requirements

### Proposal boundary and canonical knowledge — FR-301 … FR-308

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-301 proposals are attributed, never direct truth | store `knowledge.rs`, daemon `handlers.rs` | [knowledge](./contracts/knowledge.md) | B |
| FR-302 canonical answer is a deterministic derivation, rebuildable | core `knowledge.rs::derive_subject` | [knowledge](./contracts/knowledge.md), [records](./contracts/records-and-rebuild.md) | A, B |
| FR-303 no clock or id arbitration | core `knowledge.rs` | [knowledge](./contracts/knowledge.md) §derive_subject | A |
| FR-304 decisions are durable, append-only, with basis and provenance | store `knowledge.rs`, `memory_relations` | [data-model](./data-model.md) §3.1 | B |
| FR-305 decisions are idempotent | `memory_relations` PK | [records](./contracts/records-and-rebuild.md) §Idempotency | B |
| FR-306 reconciliation never rewrites proposal content | store `knowledge.rs` (no such UPDATE) | [data-model](./data-model.md) §5 I5 | B |
| FR-307 subjects are reportable with their decisions | cli `cairn memory subject` | [knowledge](./contracts/knowledge.md) | B |
| FR-308 importance orders within a bucket only | core `context.rs`, `search.rs` | [knowledge](./contracts/knowledge.md) | A, E |

### Subject identity — FR-311 … FR-318

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-311 optional topic and value keys | core `knowledge.rs`, `memories` columns | [knowledge](./contracts/knowledge.md) §Normalization | A, B |
| FR-312 total normalization; failure stores free-form and reports | core `knowledge.rs::normalize_topic_key` | [knowledge](./contracts/knowledge.md) | A |
| FR-313 free-form memories fully valid | absence of any required-key path | [compatibility](./compatibility.md) | B |
| FR-314 no vocabulary, taxonomy or registry | absence of one | [research](./research.md) D45 | — |
| FR-315 subject = project + scope + scope key + topic key | core `knowledge.rs`, `memories_subject` index | [knowledge](./contracts/knowledge.md) | A, B |
| FR-316 automatic reconciliation limited to two decidable cases | core `knowledge.rs` | [knowledge](./contracts/knowledge.md) §Automatic reconciliation | A |
| FR-317 no similarity-based merging | absence of any similarity function | [research](./research.md) D46 | — |
| FR-318 agents may propose keys through the existing tools | cli `mcp.rs`, `cairn_remember` | [mcp-tools](./contracts/mcp-tools.md) | B |

### Reinforcement, duplication, supersession — FR-321 … FR-326

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-321 reinforcement instead of a second active answer | core + store `knowledge.rs` | [knowledge](./contracts/knowledge.md) | B |
| FR-322 reinforcements distinguished from distinct origins | `reinforcement_count`, `distinct_origin_count` | [data-model](./data-model.md) §2.1 | B |
| FR-323 supersession preserves the predecessor entirely | store `knowledge.rs` | [knowledge](./contracts/knowledge.md) | B |
| FR-324 Feature 001's supersession link stays accurate | store, same transaction as the relation | [compatibility](./compatibility.md) | B |
| FR-325 supersession is never automatic | absence of an automatic caller | [knowledge](./contracts/knowledge.md) | B |
| FR-326 duplicate detection is content-exact after normalization | core `knowledge.rs::content_norm` | [knowledge](./contracts/knowledge.md) | A |

### Conflict — FR-331 … FR-337

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-331 semantic vs concurrent-write conflict separated | core `knowledge.rs` / store `tx.rs` | [knowledge](./contracts/knowledge.md), [records](./contracts/records-and-rebuild.md) | A, B |
| FR-332 conflict needs same scope and scope key | core `knowledge.rs::scope_overlap` | [knowledge](./contracts/knowledge.md) §Scope overlap | A |
| FR-333 scope exception, not conflict | core `knowledge.rs` | [knowledge](./contracts/knowledge.md) | A |
| FR-334 a conflicted subject keeps every member and warns | core `knowledge.rs`, daemon `briefing.rs` | [knowledge](./contracts/knowledge.md) | B, E |
| FR-335 resolution is explicit, with a basis | cli `memory reconcile`, `cairn_remember action=reconcile` | [knowledge](./contracts/knowledge.md) §Conflict resolution | B |
| FR-336 no path replaces a proposal or decision | append-only rows + PK idempotency | [records](./contracts/records-and-rebuild.md) §Concurrency | B |
| FR-337 mutable work state uses revision comparison | store `criteria.rs` | [task-model](./contracts/task-model.md) §Concurrency | F |

### Temporal truth — FR-341 … FR-345

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-341 effective from, superseded at | `memories` columns | [data-model](./data-model.md) §2.1 | B |
| FR-342 current and as-of answered and distinguished | store `search.rs`, cli `--as-of` | [knowledge](./contracts/knowledge.md) §Temporal queries | B |
| FR-343 historical queries modify nothing | read-only query path | [knowledge](./contracts/knowledge.md) | B |
| FR-344 `last_verified_at` distinct from other instants | `memories` column | [data-model](./data-model.md) §2.1 | C |
| FR-345 not bitemporal | absence of a valid-time table | [research](./research.md) D50 | — |

### Evidence — FR-351 … FR-359

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-351 eight evidence kinds | core `domain.rs`, store `evidence.rs` | [evidence-verification](./contracts/evidence-verification.md) | A, C |
| FR-352 required evidence fields | `evidence_facts` | [data-model](./data-model.md) §3.2 | C |
| FR-353 locator repository-relative, never absolute | store `evidence.rs` validation | [evidence-verification](./contracts/evidence-verification.md) | C |
| FR-354 exclusion, redaction and bounding before storage | daemon, reusing `redact.rs` and `config.rs` | [evidence-verification](./contracts/evidence-verification.md) | C |
| FR-355 collector distinguished; attested never re-run | `collector` column, daemon `verify.rs` | [evidence-verification](./contracts/evidence-verification.md) | C |
| FR-356 Feature 001 observation evidence unchanged; zero evidence valid | `memory_evidence` untouched | [compatibility](./compatibility.md) | C |
| FR-357 evidence can go stale and be rechecked | `fingerprint`, daemon `drift.rs` | [evidence-verification](./contracts/evidence-verification.md) §Drift | D |
| FR-358 deleted evidence resolves as deleted | store `evidence.rs` tombstone | [privacy-sync](./contracts/privacy-sync.md) §Deletion | C |
| FR-359 supports / contradicts roles | `memory_evidence_facts.role` | [data-model](./data-model.md) §3.3 | C |

### Verification — FR-361 … FR-369

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-361 deterministic; a model opinion is not verification | daemon `verify.rs` | [evidence-verification](./contracts/evidence-verification.md) | C |
| FR-362 verification state separate from lifecycle | core `domain.rs`, `memories.verification` | [data-model](./data-model.md) §4 | A |
| FR-363 a run records what/against/when/where/result | `verification_runs` | [evidence-verification](./contracts/evidence-verification.md) §Recording | C |
| FR-364 runs are append-only | `verification_runs`, insert-only | [records](./contracts/records-and-rebuild.md) | C |
| FR-365 verifiers read the worktree and Git only | daemon `verify.rs`, git | [evidence-verification](./contracts/evidence-verification.md) §Refused | C |
| FR-366 unreadable target is inconclusive | daemon `verify.rs` | [evidence-verification](./contracts/evidence-verification.md) | C |
| FR-367 proposal, evidence and result distinguished | `cairn_remember` actions + `verification_runs` | [mcp-tools](./contracts/mcp-tools.md) | C |
| FR-368 imported verification labelled, not local | `verification_origin` | [privacy-sync](./contracts/privacy-sync.md) §Import | G |
| FR-369 `conflicted` verification defined and separate | core `verify.rs` | [evidence-verification](./contracts/evidence-verification.md) | A, C |

### Drift — FR-371 … FR-375

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-371 fingerprint change → `needs_recheck`, nothing else | daemon `drift.rs` | [evidence-verification](./contracts/evidence-verification.md) §Marking | D |
| FR-372 no rewrite on one changed source | absence of such a path | [evidence-verification](./contracts/evidence-verification.md) | D |
| FR-373 drifted memory intact, returned, warned | store `search.rs`, daemon `briefing.rs` | [evidence-verification](./contracts/evidence-verification.md) | D, E |
| FR-374 bounded indexed triggering with a per-event cap | daemon `drift.rs`, `evidence_facts` index | [evidence-verification](./contracts/evidence-verification.md) | D |
| FR-375 total documented state machine | core `verify.rs` | [evidence-verification](./contracts/evidence-verification.md) §State machine | A |

### Scope and branch lifecycle — FR-381 … FR-385

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-381 scope evaluated first; no new scope | core `knowledge.rs`, existing `MemoryScope` | [knowledge](./contracts/knowledge.md) | A, B |
| FR-382 merged branch produces a candidate, not truth | git ancestry + daemon maintenance | [knowledge](./contracts/knowledge.md), [research](./research.md) D48 | B |
| FR-383 branch deletion preserves history as stale | existing `mark_stale_scopes` | [compatibility](./compatibility.md) | B |
| FR-384 rebase marks commit-pinned evidence for recheck | daemon `drift.rs` | [evidence-verification](./contracts/evidence-verification.md) | D |
| FR-385 branch vs project disagreement selects by scope | core `knowledge.rs` | [knowledge](./contracts/knowledge.md) | A |

### Reusable patterns — FR-391 … FR-399

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-391 a distinct record, never a global memory scope | `reusable_patterns` (no `project_id`) | [patterns](./contracts/patterns.md) | H |
| FR-392 required pattern fields | `reusable_patterns` | [data-model](./data-model.md) §3.6 | H |
| FR-393 no project identity; opaque origin | `origin_ref` digest | [patterns](./contracts/patterns.md) | H |
| FR-394 staged trust ladder | store `patterns.rs::rebuild_pattern_trust` | [patterns](./contracts/patterns.md) §Trust | H |
| FR-395 promotion is explicit; suggestions change no trust | cli/MCP `promote`; absence of an automatic caller | [patterns](./contracts/patterns.md) | H |
| FR-396 ten enumerated refusal classes | store `patterns.rs::gate` | [patterns](./contracts/patterns.md) §Gate | H |
| FR-397 refusal names the class, echoes nothing, writes nothing | store `patterns.rs::gate` | [patterns](./contracts/patterns.md) | H |
| FR-398 offered only on signal match, labelled unverified here | daemon `briefing.rs` | [patterns](./contracts/patterns.md) §Suggestion | H |
| FR-399 deleted origin: pattern survives, reports deletion | store `patterns.rs` | [privacy-sync](./contracts/privacy-sync.md) §Deletion | H |

### Independence and counterexamples — FR-401 … FR-406

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-401 applications record project, session, outcome, independence | `pattern_applications` | [data-model](./data-model.md) §3.7 | H |
| FR-402 trust only from distinct non-origin projects | `rebuild_pattern_trust` + unique key | [patterns](./contracts/patterns.md) | H |
| FR-403 Cairn-suggested needs local evidence to count | `discovery` set by the daemon, not the agent | [patterns](./contracts/patterns.md) | H |
| FR-404 counterexample recorded, no success count, no deletion | store `patterns.rs` | [patterns](./contracts/patterns.md) §Counterexamples | H |
| FR-405 contested is offered with both sides | daemon `briefing.rs`, cli renderer | [patterns](./contracts/patterns.md) | H |
| FR-406 no count presented as independent verifications | cli renderers, MCP payload shape | [patterns](./contracts/patterns.md) | H |

### Cross-device reconciliation — FR-411 … FR-417

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-411 no timestamp arbitration in a merge | daemon `sync.rs` + core `knowledge.rs` | [privacy-sync](./contracts/privacy-sync.md) §Import | G |
| FR-412 every proposal preserved; no row overwrite | daemon `sync.rs::import_memory` | [privacy-sync](./contracts/privacy-sync.md) | G |
| FR-413 decisions synchronize and are applied as decisions | outbox `MemoryRelation`, `sync_changes.relations` | [privacy-sync](./contracts/privacy-sync.md) | G |
| FR-414 reuse the outbox, keys and applied-once claim | store `outbox.rs`, server `sync.rs` | [records](./contracts/records-and-rebuild.md) | G |
| FR-415 additive both ways; degrade and report | daemon `sync.rs`, `sync_meta` | [privacy-sync](./contracts/privacy-sync.md) §Degradation | G |
| FR-416 merge-introduced incompatibility surfaces on every device | core `knowledge.rs` (derived on read) | [knowledge](./contracts/knowledge.md) | G |
| FR-417 holds for sessions, agents, worktrees, machines, members | the whole sync design | [privacy-sync](./contracts/privacy-sync.md) | G |

### Continuity — FR-421 … FR-428

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-421 structured, derived, not a conversation summary | daemon `continuity.rs` | [continuity-context](./contracts/continuity-context.md) | E |
| FR-422 required checkpoint contents | `continuity_checkpoints` + the anchored handoff | [continuity-context](./contracts/continuity-context.md) | E |
| FR-423 anchored to the existing boundary record | `handoff_id` | [continuity-context](./contracts/continuity-context.md) | E |
| FR-424 records its assumptions | assumption columns | [data-model](./data-model.md) §3.5 | E |
| FR-425 written at pre-compaction and close; available on demand | daemon `continuity.rs`, `cairn_session action=checkpoint` | [continuity-context](./contracts/continuity-context.md) | E |
| FR-426 honest continuity mode; no claimed guarantee | daemon derived read over Feature 002 capabilities | [continuity-context](./contracts/continuity-context.md), [compatibility](./compatibility.md) | E |
| FR-427 no canonical event added; no capability inflated | absence of one | [compatibility](./compatibility.md) | E |
| FR-428 fields survive any number of cycles | derived per boundary, never copied forward | [continuity-context](./contracts/continuity-context.md) | E |

### Checkpoint staleness — FR-431 … FR-435

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-431 classify current / diverged / unresolvable | core `continuity.rs::classify_checkpoint` | [continuity-context](./contracts/continuity-context.md) | A, E |
| FR-432 four divergence classes | core `continuity.rs` | [continuity-context](./contracts/continuity-context.md) | A, E |
| FR-433 name the specific differences | daemon `continuity.rs`, cli renderer | [continuity-context](./contracts/continuity-context.md) | E |
| FR-434 stale next action is labelled, never instructed | `previous_next_action` field | [continuity-context](./contracts/continuity-context.md) | E |
| FR-435 unresolvable still delivers what it can | daemon `continuity.rs` | [continuity-context](./contracts/continuity-context.md) | E |

### Minimum safe context — FR-441 … FR-447

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-441 three levels | core `context.rs` | [continuity-context](./contracts/continuity-context.md) §Part 2 | E |
| FR-442 reserved share; a cap not a floor; budget unchanged | core `budget.rs::with_reserve` | [continuity-context](./contracts/continuity-context.md) §Reserve | A, E |
| FR-443 Level 0 contents | core `context.rs` | [continuity-context](./contracts/continuity-context.md) | E |
| FR-444 Level 2 never automatic | core `context.rs` (no admission path) | [continuity-context](./contracts/continuity-context.md) | E |
| FR-445 never exceed the budget; truncate not reject | core `budget.rs::try_spend` unchanged | [compatibility](./compatibility.md) | A |
| FR-446 documented deterministic Level 0 order | core `context.rs` | [continuity-context](./contracts/continuity-context.md) | A, E |
| FR-447 omissions reported; high-priority guarantee holds | core `context.rs` unchanged fields | [compatibility](./compatibility.md) | E |

### Protected invariants — FR-451 … FR-457

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-451 pinning exists | `memories.pinned` | [continuity-context](./contracts/continuity-context.md) §Part 3 | E |
| FR-452 pin records who, when, why | pin columns | [data-model](./data-model.md) §2.1 | E |
| FR-453 a pin never overrides scope | core `context.rs` applicability filter | [continuity-context](./contracts/continuity-context.md) | E |
| FR-454 bounded, refusable, nothing auto-unpinned | store `repo.rs` pin path | [continuity-context](./contracts/continuity-context.md) | E |
| FR-455 user and agent may pin | cli `memory pin`, `cairn_remember action=pin` | [mcp-tools](./contracts/mcp-tools.md) | E |
| FR-456 cleared on supersession; kept on drift | store, same transaction as the relation | [continuity-context](./contracts/continuity-context.md) | E |
| FR-457 `local_only` stays local; a pin adds no content | outbox payload excludes `pin_reason` | [privacy-sync](./contracts/privacy-sync.md) | E, G |

### Explainability — FR-461 … FR-464

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-461 closed set of selection reasons | core `domain.rs::SelectionReason` | [continuity-context](./contracts/continuity-context.md) §Reasons | A, E |
| FR-462 report selected, omitted, scope, verification, ranking | core `context.rs`, cli `--explain` | [continuity-context](./contracts/continuity-context.md) | E |
| FR-463 diagnostics off by default; no budget cost | `explain` parameter | [mcp-tools](./contracts/mcp-tools.md) | E |
| FR-464 warnings are Level 0 content, not diagnostics | core `context.rs` | [continuity-context](./contracts/continuity-context.md) | E |

### Bounded work, performance, failure isolation — FR-471 … FR-478

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-471 nothing verifies or scans at session open | daemon (no call site) | [evidence-verification](./contracts/evidence-verification.md) | C |
| FR-472 cached, background or on-demand, with caps | daemon `verify.rs` on the maintenance tick | [evidence-verification](./contracts/evidence-verification.md) §Verifying | C |
| FR-473 `unverified` / `needs_recheck` is a valid answer | core `verify.rs` | [evidence-verification](./contracts/evidence-verification.md) | C |
| FR-474 bounded reconciliation per write | store `knowledge.rs`, `reconcile_members_max` | [knowledge](./contracts/knowledge.md) | B |
| FR-475 capture keeps 250 ms, exit 0, fail soft | daemon `capture.rs`, `drift.rs` | [evidence-verification](./contracts/evidence-verification.md) §Marking | D |
| FR-476 Cairn failure never aborts the agent | existing fail-soft paths | [compatibility](./compatibility.md) | all |
| FR-477 fully offline | absence of any network call | [evaluation](./contracts/evaluation.md) §Tiers | all |
| FR-478 untrusted derived value is rebuilt or reported unavailable | store rebuild procedures | [records](./contracts/records-and-rebuild.md) §Fail-closed | B–H |

### Tasks — FR-481 … FR-492

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-481 stable criterion identity | `task_criteria.id` | [task-model](./contracts/task-model.md) | F |
| FR-482 work state distinct from verification | two columns | [task-model](./contracts/task-model.md) | F |
| FR-483 assertion is not verification | `satisfied` + `unverified` reported separately | [task-model](./contracts/task-model.md) | F |
| FR-484 criterion evidence; `verified` needs Cairn-collected | `criterion_evidence` + the collector check | [task-model](./contracts/task-model.md) | C, F |
| FR-485 append-only blockers with one cleared transition | `task_blockers` | [task-model](./contracts/task-model.md) | F |
| FR-486 derived counts; no stored percentage | store `criteria.rs::derive_progress` | [task-model](./contracts/task-model.md) §Progress | F |
| FR-487 readiness derived; status never changed by Cairn | store `criteria.rs` | [task-model](./contracts/task-model.md) §Readiness | F |
| FR-488 monotone revision + append-only change log | `tasks.revision`, `task_changes` | [task-model](./contracts/task-model.md) | F |
| FR-489 session records its bind revision; divergence reported | `sessions.task_revision_at_bind` | [task-model](./contracts/task-model.md) §Divergence | F |
| FR-490 different criteria both apply; supplied revision protects | store `criteria.rs` | [task-model](./contracts/task-model.md) §Concurrency | F |
| FR-491 no project-management machinery | absence of it | spec Out of Scope | — |
| FR-492 Feature 001 task fields keep working | the retained projection | [compatibility](./compatibility.md) | F |

### Agent surface — FR-495 … FR-500

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-495 exactly six MCP tools | cli `mcp.rs::TOOL_NAMES` | [mcp-tools](./contracts/mcp-tools.md) | B–H |
| FR-496 backward-compatible action and parameter extension | cli `mcp.rs` | [mcp-tools](./contracts/mcp-tools.md) | B–H |
| FR-497 Feature 001 calls behave as today | cli `mcp.rs` defaults | [mcp-tools](./contracts/mcp-tools.md) | B–H |
| FR-498 usage contract teaches the new obligations, in bound | `skills/cairn/`, `cairn-integrate/render.rs` | [mcp-tools](./contracts/mcp-tools.md) §Descriptions | H |
| FR-499 every capability on the command line with the JSON envelope | cli `main.rs` | [compatibility](./compatibility.md) §Surfaces | B–H |
| FR-500 every bound documented, configurable, asserted | core `config.rs` | [research](./research.md) D75 | A |

### Privacy — FR-501 … FR-508

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-501 raw observations stay local | absence of an entity type and a server table | [privacy-sync](./contracts/privacy-sync.md) | G |
| FR-502 evidence content never leaves; four-field verification only | outbox payload shape | [privacy-sync](./contracts/privacy-sync.md) §Evidence | C, G |
| FR-503 runs, checkpoints, patterns, applications, changes are local | absence of entity types and tables | [privacy-sync](./contracts/privacy-sync.md) | C–H |
| FR-504 `local_only` means never transmitted, transitively | existing outbox behaviour + gate check 4 | [privacy-sync](./contracts/privacy-sync.md) | G, H |
| FR-505 deleted-origin semantics for every new reference | store tombstones | [privacy-sync](./contracts/privacy-sync.md) §Deletion | C–H |
| FR-506 allowlist extended by enumerated fields, enforced on the wire | server `sync.rs` | [privacy-sync](./contracts/privacy-sync.md) §Delta | G |
| FR-507 promotion is the highest risk; deterministic, fail-closed, tested | store `patterns.rs::gate` | [patterns](./contracts/patterns.md) §Gate | H |
| FR-508 patterns never synchronize | absence of an entity type and a server table | [privacy-sync](./contracts/privacy-sync.md) | H |

### Determinism, migration, compatibility — FR-511 … FR-519

| FR | Owner | Contract | Phase |
|---|---|---|---|
| FR-511 no model, embedding, vector or graph dependency | manifests unchanged | [evaluation](./contracts/evaluation.md) | all |
| FR-512 assistance proposes; a deterministic gate decides | `basis = explicit_agent` + gates | [knowledge](./contracts/knowledge.md), [patterns](./contracts/patterns.md) | B, H |
| FR-513 additive schema; no migration edited; no row rewritten | `0005` local, `0002` server | [migration](./migration.md) | B |
| FR-514 lossless migration, no user action, explicit defaults | [migration](./migration.md) §Steps | [migration](./migration.md) | B |
| FR-515 nothing fabricated for a new field | deliberate NULLs | [migration](./migration.md) §Step 2 | B |
| FR-516 schema-version guard unchanged | `migrate.rs` | [migration](./migration.md) §Mixed-version | B |
| FR-517 every derived value rebuildable, exercised by test | store rebuild procedures | [records](./contracts/records-and-rebuild.md) §Rebuild | B–H |
| FR-518 corruption fails closed | store rebuild + `degraded` | [records](./contracts/records-and-rebuild.md) | B–H |
| FR-519 Feature 001/002 behaviour holds; suites pass unchanged | existing suites | [compatibility](./compatibility.md) | all |

---

## Success criteria

| SC | Test | Tier | Phase |
|---|---|---|---|
| SC-301 one canonical answer, zero false merges | `us1_reconciliation` + corpus `equivalent/`, `distinct/`, `free_form/` | 2, 3 | B |
| SC-302 zero silent winners, zero false conflicts | `us3_conflict` + corpus `conflict/*` | 2, 3 | B |
| SC-303 32 concurrent proposals, zero lost writes | `us3_conflict::concurrent_proposals` | 3 | B |
| SC-304 clock reversal yields an identical merged state | `clock_swap_invariance` | 3 | G |
| SC-305 supersession history byte-identical; `as_of` correct | `us2_temporal` | 2, 3 | B |
| SC-306 verification transitions exhaustive; undocumented unreachable | `us4_evidence::state_machine` | 2 | A, C |
| SC-307 drift sequence correct; memory unchanged | `us5_drift` | 2, 3 | D |
| SC-308 estimated tokens never exceed the budget | `us10_min_safe_context` (+ the existing 5,000-memory property test) | 2 | A, E |
| SC-309 Level 0 retention at the minimum budget | `us10_min_safe_context` | 2, 3 | E |
| SC-310 continuity fields present after each of ten cycles | `us6_continuity` | 3 | E |
| SC-311 every divergence class detected; no live stale action | `us6_continuity::staleness` | 2, 3 | E |
| SC-312 pattern labelled unverified in the receiving project | `us8_patterns` | 3 | H |
| SC-313 counterexample increases nothing; pattern retained | `us9_counterexamples` | 2, 3 | H |
| SC-314 ten same-project applications → distinct count 1 | `us9_counterexamples` | 2, 3 | H |
| SC-315 promotion refuses every seeded violation, echoes nothing | `privacy_promotion` | 3 | H |
| SC-316 no evidence, run, checkpoint, pattern or observation field accepted | `privacy_payloads` (extended) | 3, 4 | G |
| SC-317 concurrent criterion updates both persist | `us11_task_criteria` | 3 | F |
| SC-318 revision divergence reported with its change list | `us11_task_criteria` | 3 | F |
| SC-319 session-open and capture latency within Feature 001 budgets, loaded | `perf_intelligence` | 3 | H |
| SC-320 every bound asserted; zero verification at session open | `bounds`, `perf_intelligence` | 1, 3 | A, C |
| SC-321 no model, embedding, vector or graph dependency | `ci_hermeticity` (extended) | 1 | A |
| SC-322 alpha.4 migration: zero rows lost, zero rewritten | `migration_alpha4` | 3 | B |
| SC-323 Feature 001/002 suites pass; six tools; no observation entity type | existing suites, `mcp_backward_compatibility` | 1–4 | all |
| SC-324 every derived value equals its rebuild | `rebuild_equivalence`, `relation_order_invariance` | 2, 3 | B–H |
| SC-325 no release gate reads a model judgement | CI configuration review + `evals/` isolation | — | H |
| SC-326 degraded sync delivers everything accepted and says so | `sync_degradation` | 4 | G |
| SC-327 no scope added; no importance/pin/verification scope override | `scope_audit` (extended) | 1, 3 | A, E |
| SC-328 criterion never verified on attested evidence alone | `us11_task_criteria` | 2, 3 | F |

---

## Invariant coverage

The twenty invariants in [data-model.md](./data-model.md) §5 map to tests as follows.

| Invariants | Test |
|---|---|
| I1, I4, I5 | `us3_conflict`, plus a source audit asserting no `UPDATE memories SET content` outside the tombstone |
| I2 | `us1_reconciliation::idempotent_decision` |
| I3 | `clock_swap_invariance` |
| I6 | `us5_drift::marks_only_verification` |
| I7, I11, I12, I20 | `rebuild_equivalence` |
| I8, I9 | `privacy_payloads`, `privacy_promotion` |
| I10 | `us11_task_criteria::attested_is_not_enough` |
| I13 | `us9_counterexamples::one_incident_counts_once` |
| I14, I15 | `us10_min_safe_context::pins` |
| I16, I17, I18 | `us10_min_safe_context::budget` |
| I19 | `perf_intelligence::no_verification_at_session_open` |

## Gate status

| Gate | Status |
|---|---|
| Every FR has an owning design surface | ✅ 158/158 |
| Every SC has a named test at a named tier | ✅ 28/28 |
| Every invariant has a test | ✅ 20/20 |
| Privacy boundary explicit per record type | ✅ [privacy-sync](./contracts/privacy-sync.md) |
| Migration and rebuild explicit | ✅ [migration](./migration.md), [records](./contracts/records-and-rebuild.md) |
| Concurrency semantics explicit | ✅ [records](./contracts/records-and-rebuild.md) §Concurrency |
| No silent last-write-wins path remains | ✅ FR-303, FR-336, FR-411, FR-412; task work state is recorded and attributed |
| Feature 001 compatibility accounted for | ✅ [compatibility](./compatibility.md) |
| Feature 002 six-tool and agent compatibility accounted for | ✅ [compatibility](./compatibility.md) |
| Task capability remains secondary | ✅ 12 of 158 FR; phase F; FR-491 |
| No mandatory model or vector dependency in core correctness | ✅ FR-511, SC-321 |
| No code-intelligence or source-RAG scope creep | ✅ spec Out of Scope |
| Quickstart demonstrates the user-visible result | ✅ [quickstart](./quickstart.md) |
| CRITICAL findings open | **0** |
| HIGH findings open | **0** |
| `tasks.md` | **NOT GENERATED** — by instruction |
