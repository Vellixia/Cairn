# Specification Quality Checklist: Cairn Project Intelligence

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-08-14
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details (languages, frameworks, APIs)
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic (no implementation details)
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] No implementation details leak into specification

## Feature 003 Specific Gates

- [x] The core principle — no agent statement becomes project truth directly — is stated as a
      requirement, not only as narrative (FR-301)
- [x] No requirement permits a silent last-write-wins path on canonical knowledge (FR-303, FR-336,
      FR-411, FR-412)
- [x] Semantic conflict and concurrent write conflict are defined separately and handled by
      different mechanisms (FR-331)
- [x] Lifecycle state and verification state are separate enums with Feature 001's lifecycle names
      preserved (FR-362)
- [x] Structured subject identity is optional; free-form memories remain fully valid (FR-313)
- [x] No ontology, taxonomy or registry of topic keys is defined (FR-314)
- [x] Evidence is bounded, attributable, redacted, privacy-safe and can go stale (FR-351–FR-359)
- [x] Verification is deterministic; a model opinion is explicitly not verification (FR-361)
- [x] Drift never mutates a memory; it changes verification state only (FR-371, FR-372)
- [x] Temporal semantics are minimal and explicitly not bitemporal (FR-345)
- [x] Scope is evaluated before conflict, and the scope-exception case is specified (FR-332, FR-333)
- [x] Branch lifecycle behaviour is explicit for active, merged, deleted and rebased (FR-382–FR-384)
- [x] Cross-project reuse is a distinct entity, never a global memory scope (FR-391)
- [x] Promotion is staged, explicit, and refuses with a named reason (FR-394–FR-397)
- [x] Anti-poisoning accounts for independence rather than repetition (FR-402, FR-403, FR-406)
- [x] Counterexamples are recorded and never delete the pattern (FR-404, FR-405)
- [x] Continuity is derived work state, not a conversation summary (FR-421)
- [x] Checkpoint staleness is detected and never presented as a live instruction (FR-431–FR-434)
- [x] Minimum safe context has a reserved budget share and a documented internal order (FR-442,
      FR-446)
- [x] Feature 001's never-exceed-the-budget guarantee is preserved (FR-445)
- [x] Protected invariants are bounded, scope-respecting and refusable (FR-451–FR-457)
- [x] Selection is explainable without spending agent budget by default (FR-461–FR-463)
- [x] No Feature 003 work is added to the session-open or capture-hook paths (FR-471, FR-475)
- [x] The task capability is secondary and explicitly excludes project-management machinery
      (FR-491)
- [x] The MCP surface stays at exactly six tools and is extended backward-compatibly (FR-495–FR-497)
- [x] No mandatory language model, embedding service, vector database or graph database
      (FR-511, SC-321)
- [x] Privacy boundary is explicit for every new record type, including what may not leave
      (FR-501–FR-508)
- [x] Migration is additive, lossless, and fabricates nothing (FR-513–FR-515)
- [x] Every derived value is rebuildable from durable records (FR-302, FR-517, SC-324)
- [x] Feature 001 and Feature 002 compatibility is stated as a requirement (FR-492, FR-519, SC-323)
- [x] Out-of-scope boundary explicitly excludes code intelligence and retrieval-augmented
      source search

## Notes

- Items marked incomplete require spec updates before `/speckit-clarify` or `/speckit-plan`.
- Validation history is recorded below.

### Iteration 1 — 2026-08-14 (initial draft)

Result: **all items pass**, with three material decisions taken as documented assumptions rather
than as `[NEEDS CLARIFICATION]` markers, because each has a defensible conservative default that
the baseline architecture already implies:

1. **Free-form equivalence.** Cairn cannot deterministically decide that two differently-worded
   memories concern one subject without inference, which FR-511 forbids in the correctness path.
   Resolved conservatively: reconciliation is strongest with a topic key, exact-normalized-content
   duplication is the only subject-free automatic case, and the usage contract asks agents to
   supply topic keys (FR-316, FR-317, Assumptions).
2. **Evidence content and the server.** Extending the server allowlist to carry evidence values
   would widen Feature 001's privacy contract. Resolved conservatively: evidence content stays
   local and only verification state, instant, count and verifier kinds travel (FR-502).
3. **Reusable pattern sharing.** Resolved conservatively: patterns are local and never
   synchronize in this feature (FR-508).

These are recorded in Assumptions and are re-examined in `/speckit-clarify`, where any that a
reader would materially dispute is raised as a question rather than assumed.

### Iteration 2 — 2026-08-14 (clarification pass)

Result: **all items pass**. No question was escalated: every open decision resolved from current
main, the Feature 001/002 contracts, the feature brief, or the constitution's conservative bias.
Eleven resolutions are recorded in the spec's `## Clarifications` section.

Requirements added or tightened by this pass:

- FR-302, FR-307 — `canonical answer` is now the single term, and the one/several/none cases are
  stated explicitly. `canonical head` is retired throughout.
- FR-308 — `importance` is bounded to within-bucket ordering and can never act as a scope override.
- FR-334 — a conflicted subject returns every competing answer rather than none.
- FR-362, FR-369 — the two `conflicted` states are separated and each is given its own trigger.
- FR-373 — a drifted memory is explicitly still returned, and never counts as verified.
- FR-415 — an older server degrades to Feature 001 semantics and is reported, not failed.
- FR-442 — the Level 0 reserve is a cap on the lower levels, not a floor Level 0 must spend, and the
  default context budget is unchanged.
- FR-484 — a criterion reaches `verified` only on Cairn-collected evidence.
- FR-499, FR-500 — the command-line surface is enumerated, and every bound the feature relies on
  must have a documented, test-asserted, configurable default.
- SC-320, SC-326, SC-327, SC-328 — measurable outcomes added for the bounds, the degraded server,
  the scope audit and criterion verification.

### Iteration 3 — 2026-08-14 (post-plan reconciliation)

Re-validated after planning. Result: **all items pass**, unchanged. Planning added no requirement
and removed none; it assigned every requirement an owning design surface. See
[traceability.md](../traceability.md).
