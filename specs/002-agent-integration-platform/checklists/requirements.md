# Specification Quality Checklist: Agent Integration Platform

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-08-11
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

## Feature 002 Specific Gates

- [x] No requirement reintroduces a pre-reset Cairn concept; Feature 001 (`001-cairn-mvp`) is the
      only predecessor referenced
- [x] Every Feature 001 contract this feature extends is named explicitly as an additive extension
- [x] Vendor-specific behavior is stated only where it was verified against a current official
      source, and is confined to Assumptions and adapter-facing requirements
- [x] No canonical lifecycle event is defined that no supported agent signals
- [x] No vendor signal is mapped to a Cairn boundary on name similarity alone
- [x] Configuration ownership is unambiguous in steady state, with the migration transition
      defined explicitly rather than forbidden by accident
- [x] No requirement depends on an undocumented third-party interface
- [x] Installation scope is defined per resource kind and recorded alongside ownership
- [x] Privacy boundary is explicit for every newly exposed vendor payload field
- [x] Idempotency, migration, concurrency, and failure behavior are each stated as requirements
- [x] Capability reporting is honest: no surface may claim full integration below FULL
- [x] Out-of-scope boundary is explicit and includes the manager's provider functionality

## Notes

- Items marked incomplete require spec updates before `/speckit-clarify` or `/speckit-plan`.
- Validation history is recorded below.

### Iteration 1 — 2026-08-11 (initial draft)

Result: **3 open items**, all `[NEEDS CLARIFICATION]` markers; every other item passed.

Open: FR-109 (definition of the FULL level), FR-177 (repair of hand-edited Cairn resources),
FR-201 (project-local desired-state manifest).

### Iteration 2 — 2026-08-11 (post-clarification)

All four clarification questions were answered by the product owner and encoded into the spec
(see `## Clarifications`, Session 2026-08-11). All three markers are resolved; every checklist
item passes.

| Item | Status | Note |
|---|---|---|
| No implementation details | Pass | Vendor mechanisms are described by behavior; no language, framework, module, trait, or type names appear. File formats are named only in Assumptions, where the verified surfaces are recorded |
| Focused on user value | Pass | 10 user stories, each with an independent test |
| Written for non-technical stakeholders | Pass | Vendor terminology is unavoidable for an integration feature and is explained in place |
| All mandatory sections completed | Pass | Scenarios, clarifications, requirements, entities, success criteria, assumptions, out of scope |
| No [NEEDS CLARIFICATION] markers | Pass | 0 remaining (was 3) |
| Requirements testable and unambiguous | Pass | 127 requirements at this iteration, FR-101–FR-227, no duplicates, no gaps |
| Success criteria measurable | Pass | 30 criteria at this iteration, SC-101–SC-130, each with a count, percentage, byte-identity, or demonstrated-by-test assertion |
| Success criteria technology-agnostic | Pass | No product, language, or file-format names in the criteria |
| Acceptance scenarios defined | Pass | Every story has Given/When/Then scenarios; numbering is contiguous per story |
| Edge cases identified | Pass | 23 edge cases |
| Scope clearly bounded | Pass | Out of Scope names the manager's provider features, non-adapter agents, the deferred manifest, the deferred adopt operation, and automatic scope relocation |
| Dependencies and assumptions identified | Pass | Verified integration surfaces (checked 2026-08-11) plus product assumptions |
| Feature 002 gates | Pass | See gate list above; each was checked against the requirement text |

**Consistency checks performed**

- Requirement identifiers: 127 unique, contiguous FR-101–FR-227, zero duplicates.
- Success criteria: 30 unique, contiguous SC-101–SC-130, zero duplicates.
- Every clarification cross-reference resolves to an existing requirement or criterion.
- Capability honesty is stated once and enforced in three places (FR-110, FR-207, FR-209) with no
  contradiction between them.
- The scope model (FR-210–FR-220) and the ownership model (FR-145–FR-150) were checked against
  each other: scope never implies ownership, and a manager-owned per-user resource cannot be
  shadowed by a Cairn direct per-user resource (FR-219 defers to FR-146 rather than resolving by
  relocation).
- The FULL definition (FR-109) and the completion guarantee (FR-207) were checked against the
  OpenCode and Codex acceptance scenarios: neither story claims a level the requirements would
  deny it.

**Deferred to planning** (recorded, not blocking)

- Exact ownership marker syntax and the contract/Skill version scheme.
- Exact command names and hierarchy for connect, preview, doctor, repair, migrate, and disconnect.
- Which specific configuration file each adapter writes for each resource kind and scope.
- Whether Cairn's MCP server advertises a newer protocol revision than Feature 001 negotiates.
- Fixture corpus contents for the 20-configuration preservation criterion (SC-104).
- Where the migrating state is persisted and how an interrupted migration is resumed or reversed.
- The exact wire name and shape of the `manager action required` outcome.
- Where recovery artifacts live on disk and how long they are retained.

### Iteration 3 — 2026-08-11 (reconciliation pass)

Seven findings were raised against iteration 2 and resolved without broadening the feature: no new
user stories, no new vendor research beyond confirming Cairn's own idle reaper in
`crates/cairnd/src/recover.rs`.

| Finding | Resolution |
|---|---|
| **H1** ownership migration was impossible as written | FR-146 now governs steady state only; FR-148 defines the transition (atomic where one effective slot exists, bounded overlap only where vendor precedence keeps the effective configuration unambiguous, target verified before source removal, failure preserves the working state); new FR-228 makes `migrating` an explicit, resumable, separately-reported condition; SC-117 restated as "never zero effective resources" rather than "never two" |
| **H2** the existing ~2 h idle reaper could have bought OpenCode a FULL rating | FR-207 now requires a mechanism that *positively establishes termination*; new FR-229 classifies the inactivity timeout and daemon-start reconciliation as recovery-from-silence that may backstop a boundary but never counts towards FULL; FR-116 aligned; SC-127 restated; new SC-131 asserts it for OpenCode specifically |
| **H3** removal assumed a CC Switch API that is not documented | New FR-232 (documented interfaces only, never private storage), FR-233 (`manager action required` outcome), FR-234 (verify against the applications' real configuration), FR-235 (use an official removal interface if one exists); FR-149/FR-181 aligned; FR-236/FR-237 split the two migration directions; new SC-132 |
| **H4** whole-file backups would have duplicated unrelated credentials | FR-156 rewritten around atomic replacement rather than copying; new FR-238 limits preserved content to the Cairn-owned block/entry/generated file and refuses rather than copying a whole file it cannot isolate; new FR-239 keeps artifacts local, unsynced, unlogged; FR-222 aligned; new SC-133 |
| **M1** `session.idle` → "turn completed" over-claimed | The canonical boundary is renamed **agent quiesced** — the strongest claim all three agents actually establish. New FR-230 states what it does *not* imply and preserves Feature 001 `Stop` behavior exactly; FR-231 forbids synthesizing an outcome from it; new SC-134 |
| **M2** SC-128 mixed nominal and failure logic | Split: SC-128 is nominal deadline evidence over ≥100 boundaries with a healthy daemon; SC-129 is independent injected timeout / crash / daemon-unavailable recovery |
| **M3** spec metadata was internally dishonest | Header now records the feature directory and the real git branch separately; status moved to *Clarified — ready for planning* |

Result: all checklist items pass. 139 requirements (FR-101–FR-239), 35 success criteria
(SC-101–SC-135), 10 user stories, 23 edge cases, 0 clarification markers.

**Contradiction tests run against this iteration**

- H1: no requirement forbids the sanctioned transient overlap; FR-146 is scoped to steady state and
  FR-228 gives the transition its own reported condition. SC-117 no longer asserts the impossible.
- H2: no requirement or entity definition lets recovery-from-silence satisfy FULL. Searched for and
  removed the last "finalized or recovered" phrasing in the Key Entities definition.
- H3: no requirement assumes an undocumented manager interface. The only deletion language left is
  the prohibition itself.
- H4: no requirement copies a pre-existing configuration file as normal behavior; preserved content
  is bounded to Cairn's own.
- M1: no "turn completed" or "turn checkpoint" phrasing remains; the Feature 001 `Stop` semantics
  are preserved by FR-230 rather than by the old name.

### Iteration 4 — 2026-08-11 (planning reconciliation)

Planning surfaced six contradictions that design alone could not resolve, because the
requirements were either unsatisfiable or mutually inconsistent. Per the constitution's
governance rule, each was resolved **in the spec** — five requirements and two criteria added,
`SC-110` restated, and no existing identifier changed or weakened. See `## Clarifications`,
Session 2026-08-11 (planning reconciliation).

| Finding | Spec change | Design change |
|---|---|---|
| A budgeted session close could leave a terminal session owing a handoff forever | **FR-240**, **SC-136** | Bounded retry plus a sweep on the daemon's existing maintenance tick; permanent failure reported (D22) |
| A capability could be neither present nor absent (OpenCode tool failure) | **FR-241**, SC-110 restated | Availability becomes `guaranteed`/`conditional`/`absent`/`pending_activation`; conditional never counts towards FULL (D19) |
| Static capability data could not degrade when a vendor changes | **FR-242** | Confidence `verified`/`expected`, sourced from local introspection or observed events; gates the completion guarantee (D19a) |
| Disconnecting one agent deleted a resource another still needed | **FR-243** | `InstalledResource` + `ResourceBinding` reference counting replaces `satisfied_by` (D28) |
| Disconnect destroyed the record needed to verify a manager withdrawal | **FR-244**, **SC-137** | The agent record survives while any binding remains, including manager-owned ones (D28a) |
| JSON editing could not preserve non-Cairn bytes | none — the requirement was right and the design was wrong | `jsonc-parser` CST replaces the `serde_json` round-trip (D37) |

Result: 144 requirements (FR-101–FR-244), 37 success criteria (SC-101–SC-137), no duplicates,
no gaps, zero dangling cross-references, zero clarification markers. All checklist items pass.

### Iteration 5 — 2026-08-11 (final planning reconciliation)

Three cross-artifact contradictions and three reference errors, found while re-reading the
reconciled plan. One required a spec addition; the rest were artifact or design corrections.

| Finding | Resolution |
|---|---|
| **H1** research D20 still said OpenCode's tool failure was "not emitted" / `absent`, contradicting the conditional model everywhere else | D20 rewritten: no *guaranteed* failure signal is a different claim from no failure event ever. Only `session_closed` is genuinely absent |
| **H2** confidence gated only the completion guarantee, so a vendor removing tool capture could still produce FULL | **FR-245** and **SC-138** added; `CapabilityEvidence` persists evidence kind and the agent version it was established against; observation evidence dies on a version change, introspection evidence is re-derived; FULL now requires every FULL-required capability to be established (D19a) |
| **H3** the Skill ref plan assumed CC Switch resolves a commit SHA or tag | Verified in its source that the downloader hardcodes `archive/refs/heads/{branch}.zip` and silently falls back to `main`. D29 rewritten around a published, never-moved `skill-release/<schema>-<revision>` branch, with refusal rather than a broken link |
| **M1** FR-240 conflated the recoverable and unrecoverable paths | Split into four numbered clauses; the completion guarantee is not claimed while a handoff is owed |
| **M2** FR-149 cited FR-229 (recovery-from-silence) for the manager fallback | Corrected to FR-233; nearby manager references verified by meaning, not by existence |
| **M3** plan provenance named only `c992c63` | Now records all three spec states and what each added |

Result: 145 requirements (FR-101–FR-245), 38 success criteria (SC-101–SC-138), no duplicates,
no gaps, zero dangling cross-references, zero clarification markers. All checklist items pass.

### Iteration 6 — 2026-08-11 (publication and evidence-trigger reconciliation)

Three findings, none requiring a new requirement — the spec already demanded these behaviors;
the plan had not said who produced them or exactly what established them.

| Finding | Resolution |
|---|---|
| **H1** two statements in D29 still described a released tag as the distribution source and claimed "the commit SHA already works", contradicting the same decision's own analysis | Both rewritten. D29 now contains exactly one ref strategy; the only remaining tag/SHA mentions are the proof that they do not work and the rejected list |
| **H2** `skill-release/<schema>-<revision>` was depended on but had no producer | New **D29a**: a `publish-skill` job in `release.yml` computes the revision with the embedded algorithm, creates the branch at the release commit when absent, treats an identical branch as unchanged, **fails the release** if it exists elsewhere, and verifies it through CC Switch's own `refs/heads` fetch before the release completes. The branch name reaches a binary only after that verification, so no build ever claims a ref that does not exist |
| **M1** `context_at_session_open` and `stable_session_identifier` are not canonical events and had no stated trigger | Both defined in D19a and propagated: context delivery is established by a payload actually emitted on the agent's supported surface (degraded counts and is recorded; nothing emitted does not); identifier stability requires two events of different kinds carrying a vendor-supplied key routed to one session (Cairn's synthesized fallback never counts). SC-138's test design extended |

Result: 145 requirements (FR-101–FR-245), 38 success criteria (SC-101–SC-138), no duplicates,
no gaps, zero dangling cross-references, zero clarification markers. All checklist items pass.

### Iteration 7 — 2026-08-11 (branch semantics and revision algorithm)

Three corrections to the publication design added in iteration 6. No new requirement — the
spec's Skill-versioning demands were already right; the plan had encoded them wrongly.

| Finding | Resolution |
|---|---|
| **H1** the write-once rule failed every later release carrying an unchanged Skill, because the branch naturally still pointed at the first release's commit | D29a rewritten around content identity: absent → create; present → **never move**, fetch it the way CC Switch does and recompute the revision from what it contains; match → success whatever commit it is on; mismatch → fail the release. The three release scenarios (A introduces R, B unchanged, C introduces S) are stated as the release-evidence tests |
| **H2** `metadata.cairn_skill_revision` lives inside `SKILL.md`, so hashing the file hashed the value being computed | New **D29b**: one canonical algorithm in `cairn-integrate::revision`. Sorted relative paths, normalized line endings, length-prefixed path/content framing, the self-field's *value* replaced with `<REVISION>` on the parsed frontmatter before hashing, `cairn_skill_schema` hashed normally, 12-hex output. The checked-in value must equal the computed one, asserted by a unit test, by the release job, and again against the fetched tree |
| **M1** the release graph left the build-input ordering implicit | Stated explicitly: `publish-skill` needs `verify` and outputs `skill_schema` / `skill_revision` / `skill_branch`; `binaries` needs `publish-skill` and embeds `skill_branch`; `images` does not; `assets` and `release` depend transitively, so a failed verification stops the pipeline before any user-facing artifact. Ordinary CI passes no branch and keeps `unpublished_skill_ref`. The workflow calls the canonical function through a `skillref` developer binary rather than reimplementing the hash |

Result: 145 requirements (FR-101–FR-245), 38 success criteria (SC-101–SC-138), no duplicates,
no gaps, zero dangling cross-references, zero clarification markers. All checklist items pass.
