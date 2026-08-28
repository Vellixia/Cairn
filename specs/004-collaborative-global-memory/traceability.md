# Traceability: Cairn Collaborative Global Memory

**Feature**: `004-collaborative-global-memory` · **Regenerated**: after the independent audit
(D446–D457) · **Anchor**: `spec.md` at **160 functional requirements, 70 success criteria** ·
**Task list**: `tasks.md` at **200 tasks** across ten phases

---

## 1. What this pass verified, and what the last one did not

The previous version of this file certified the design by **citation coverage**: every
requirement named a task, every task named a requirement, and the two sets matched in both
directions. It reported a clean result. An independent audit then found three CRITICAL and eight
HIGH defects in the design it had certified.

Citation coverage cannot fail on the defect that mattered most. `FR-546` declared the content
validator's rejection classes; `SC-438` demanded that content naming a project and content
carrying a shell command be refused *at every entry point*; the validator's class list held
seven classes and neither of those two was among them. Both requirements cited the same task,
that task existed, and every cross-reference resolved. Three artifacts agreed with each other
and all three were wrong together.

So this pass asks four questions of every requirement, and a requirement is covered only when
all four have an answer:

1. **Mechanism** — what concrete construct enforces this? A named file and, where the guarantee
   is structural, a named type or constraint. Not "the validator handles it" but *which*
   function, in *which* module, and whether the guarantee holds because a column is absent or
   because a value is checked.
2. **Call sites** — every place the mechanism must be reached from. This is the question the
   validator defect hid in: the mechanism existed and two of its call sites did not call it.
3. **Task** — the implementation task that builds it.
4. **A falsifiable test** — an assertion that fails if the mechanism is removed. A test that
   passes both before and after the guarantee is deleted protects nothing; Feature 003's
   `scope_audit.rs` split on a function name that did not exist, compared against `""`, and
   passed for its entire life.

Running this properly found **seventeen requirements with no success criterion at all**, closed
as `SC-453`–`SC-469`, and **one success criterion with no verifying task** (`SC-435`), closed as
`T199`. A subsequent adversarial pass over the four checklist items this file had recorded as
"open, non-blocking" found two more defects behind them (D458, §4e), closed as `FR-589`/`FR-590`
and `SC-470` with `SC-412` given a number it had never had. Six of the seventeen were sentences the audit round had itself just written — a repair
round is exactly where a requirement is least likely to have a criterion yet. The other eleven
had survived three passes, and the worst of them is `FR-521`: *"do not add
`MemoryScope::Global`"* is the constraint `plan.md`'s Summary calls the one decision everything
turns on, it had a task, and nothing asserted it. **A task is not a criterion.** A task can be
reworded, deferred, or marked done against a different assertion than the one intended.

### How to read the matrix

| Column | Meaning |
|---|---|
| **Mechanism** | The files the citing tasks name. `tasks.md`'s own discipline is that every task names the exact file it changes, so an em dash here means the task named none — see §5. |
| **Design** | Which contract or design artifact states the requirement. `ident`, `proj-auth`, `global`, `recall`, `sync`, `privacy` are the six contracts; `data`, `migr`, `compat`, `qs`, `rsrch`, `plan` the design documents. |
| **Builds** | Implementation tasks. An em dash is correct for a pure invariant — nothing is *built* to satisfy "no origin digest appears on the wire"; it is a property of code built for other reasons. |
| **Falsifiable test** | The verifying task. `via SC-nnn → Tnnn` means the requirement has no test of its own and is verified through the criterion that asserts its observable consequence — a legitimate chain, listed explicitly so it can be disputed rather than assumed. `accepted residual` means no dedicated criterion, deliberately (§4). |

---

## 2. Functional requirements

| Requirement | Mechanism | Design | Builds | Falsifiable test |
|---|---|---|---|---|
| `FR-401` | `main.rs` | ident, qs, rsrch | T032, T046 | via `SC-401` → T049 |
| `FR-402` | `main.rs` | ident, data, qs | T033, T046 | via `SC-401` → T049 |
| `FR-403` | — | ident, qs, rsrch | T032 | via `SC-402` → T049 |
| `FR-404` | — | ident | T032 | via `SC-403` → T049 |
| `FR-405` | `main.rs` | ident, data, qs | T034, T047 | via `SC-403` → T049 |
| `FR-407` | — | ident, data, qs, rsrch | T035, T036 | via `SC-403` → T049 |
| `FR-408` | — | ident, data | T033 | via `SC-436` → T050 |
| `FR-409` | `api.rs` | ident | T031 | via `SC-404` → T050 |
| `FR-410` | `auth.rs` | ident, data | T030 | via `SC-436` → T050 |
| `FR-411` | `main.rs` | ident | T033, T046 | via `SC-401` → T049 |
| `FR-412` | — | ident | T033 | via `SC-437` → T051 |
| `FR-413` | `admin_lifecycle.rs` | ident, migr, qs, rsrch | T033 | T045 |
| `FR-414` | `admin_lifecycle.rs` | ident, migr, qs | T026 | T055 |
| `FR-415` | `0003_collaborative_global_memory.sql` | ident, data, qs, rsrch | T025, T038 | **accepted residual** (D457) |
| `FR-416` | — | ident, data, compat | T038 | **accepted residual** (D457) |
| `FR-417` | `0003_collaborative_global_memory.sql` | ident, data | T025, T037 | via `SC-452` → T054 |
| `FR-418` | `authorization_audit.rs` | proj-auth, rsrch | T057 | T067 |
| `FR-419` | `0003_collaborative_global_memory.sql` | proj-auth, data, qs, rsrch | T025, T057 | **accepted residual** (D457) |
| `FR-420` | — | — | T057 | via `SC-408` → T066 |
| `FR-421` | `sync.rs`, `authorization_audit.rs` | proj-auth, qs | T060 | T066 |
| `FR-422` | `api.rs`, `sync.rs` | proj-auth, qs | — | T059 |
| `FR-423` | — | proj-auth | T058 | **accepted residual** (D457) |
| `FR-424` | `sync.rs` | proj-auth, qs | T061 | via `SC-407` → T065 |
| `FR-425` | `sync.rs` | proj-auth, qs | T061 | via `SC-407` → T065 |
| `FR-426` | `api.rs` | proj-auth, qs | — | T062 |
| `FR-427` | — | proj-auth | T058 | **accepted residual** (D457) |
| `FR-428` | `sync.rs` | proj-auth, qs, rsrch | T061 | **accepted residual** (D457) |
| `FR-431` | `domain.rs`, `global.rs` | data, qs | T007, T008, T068, T079 | via `SC-409` → T084 |
| `FR-432` | `global.rs` | data, qs | T068 | via `SC-409` → T084 |
| `FR-434` | `applicability.rs`, `global.rs` | global, data | T009, T073, T080 | via `SC-429` → T091 |
| `FR-435` | `applicability.rs`, `global.rs` | global, data, rsrch | T009, T073 | via `SC-429` → T091 |
| `FR-436` | `applicability.rs`, `global.rs` | global, qs | T009, T073 | via `SC-410` → T089 |
| `FR-437` | `domain.rs`, `traits.rs`, `domain_isolation.rs` | global, data, migr, qs | T010, T071 | T091 |
| `FR-438` | `traits.rs`, `privacy_payloads.rs` | global, data, rsrch | T071 | T072 |
| `FR-439` | `traits.rs`, `main.rs` | global, qs | T071, T082 | **accepted residual** (D457) |
| `FR-440` | `global.rs` | global, data | T068 | **accepted residual** (D457) |
| `FR-441` | — | data, qs, rsrch | T069, T081 | **accepted residual** (D457) |
| `FR-442` | `rebuild_equivalence.rs` | global | T070, T077, T078 | T189 |
| `FR-444` | `main.rs` | qs | T070, T082 | via `SC-409` → T084 |
| `FR-445` | — | data | T097 | via `SC-450` → T114 |
| `FR-446` | `domain_isolation.rs` | global, data, qs | — | T090 |
| `FR-451` | `global.rs`, `main.rs` | global, data, qs | T008, T115, T133 | via `SC-413` → T135 |
| `FR-452` | — | global, data, qs | T120 | via `SC-413` → T135 |
| `FR-453` | `criteria.rs`, `main.rs` | data, qs | T119, T121, T133 | via `SC-414` → T136 |
| `FR-454` | `criteria.rs` | global, sync, data, qs | T119, T121 | via `SC-415` → T137 |
| `FR-455` | `domain_isolation.rs` | global, qs, rsrch | T125, T126 | T134 |
| `FR-456` | `main.rs` | data | T121, T133 | via `SC-416` → T140 |
| `FR-457` | `criteria.rs` | — | T119 | **accepted residual** (D457) |
| `FR-458` | `global.rs` | global, qs | T115 | via `SC-416` → T140 |
| `FR-459` | `0007_collaborative_global_memory.sql`, `migration.md`, `0002_memory_fts.sql`, `global.rs` | global, data, rsrch | T115, T129 | T022 |
| `FR-460` | `domain_isolation.rs` | data, rsrch | T128 | T143 |
| `FR-461` | `criteria.rs` | global, rsrch | T119 | **accepted residual** (D457) |
| `FR-462` | `domain_isolation.rs` | global, data, rsrch | T127 | T138 |
| `FR-463` | — | global, qs | T131 | via `SC-416` → T140 |
| `FR-464` | `main.rs`, `domain_isolation.rs` | — | T132, T133 | T142 |
| `FR-465` | `domain_isolation.rs` | global, data, qs, rsrch | T122 | T141 |
| `FR-469` | `domain_isolation.rs` | recall, qs | T160 | T172 |
| `FR-470` | `domain_isolation.rs` | recall, qs | T160 | T172 |
| `FR-471` | `context.rs` | recall, rsrch | T162 | T163 |
| `FR-472` | `mcp.rs` | recall, qs | T161 | via `SC-417` → T172 |
| `FR-473` | — | recall, qs | T153 | via `SC-418` → T168 |
| `FR-474` | `context.rs`, `budget.rs`, `global_non_displacement.rs` | recall, qs, rsrch | T155 | T169 |
| `FR-475` | `context.rs`, `budget.rs`, `global_non_displacement.rs` | recall, qs | T155 | T168 |
| `FR-476` | `context.rs`, `global_non_displacement.rs` | recall, qs, rsrch | T152 | T154 |
| `FR-477` | `wire.rs`, `mcp.rs`, `briefing.rs` | recall, qs, plan | T156, T157 | via `SC-420` → T171 |
| `FR-478` | `context.rs` | recall, qs, rsrch | T158 | T159 |
| `FR-480` | `global_non_displacement.rs` | recall, qs | T167 | T169 |
| `FR-481` | — | recall, compat, qs | T164 | via `SC-417` → T172 |
| `FR-482` | `global_non_displacement.rs` | recall, rsrch | T165 | T166 |
| `FR-486` | `domain.rs`, `0007_collaborative_global_memory.sql`, `migration.md`, `0002_memory_fts.sql`, `cursor.rs`, `outbox.rs` | sync, data, qs | T010, T024, T092, T100 | T022 |
| `FR-487` | `cursor.rs`, `main.rs` | sync, data, qs | T024, T092, T109 | via `SC-425` → T187 |
| `FR-488` | `sync.rs` | sync, compat, qs | T093 | via `SC-425` → T187 |
| `FR-489` | `sync.rs`, `global.rs`, `namespace_sync.rs` | sync, compat, qs | T094, T101 | T112 |
| `FR-490` | `domain.rs`, `repo.rs` | sync, data, qs | T011, T095 | via `SC-427` → T111 |
| `FR-491` | `domain.rs` | sync, data, qs | T011, T096 | via `SC-427` → T111 |
| `FR-492` | `namespace_sync.rs`, `clock_swap_invariance.rs` | sync, data, rsrch, plan | T097 | T114, T139 |
| `FR-493` | `multi_device_convergence.rs`, `clock_swap_invariance.rs` | global, sync, data, qs | T078 | T110, T139 |
| `FR-495` | `global.rs` | sync, data, migr, compat | T116 | via `SC-428` → T118 |
| `FR-496` | `global.rs`, `namespace_sync.rs` | sync, data, migr, compat, qs, rsrch | T116 | T118 |
| `FR-497` | `sync.rs` | sync, data, compat, qs | T093 | via `SC-425` → T187 |
| `FR-498` | `global.rs` | sync, data, qs | T103, T129 | via `SC-425` → T187 |
| `FR-499` | — | sync, compat, qs, rsrch | T104 | via `SC-445` → T190 |
| `FR-500` | `capability_upgrade_e2e.rs` | sync, compat, rsrch | — | T190 |
| `FR-501` | — | sync | T104 | via `SC-426` → T190 |
| `FR-502` | `namespace_sync.rs` | sync, compat | T108 | T113 |
| `FR-506` | `promotion.rs`, `promote.rs`, `global_non_displacement.rs` | proj-auth, privacy, data, qs, rsrch | T017, T075 | T173 |
| `FR-507` | `promotion.rs`, `promotion-privacy.md` | privacy, data, qs | T017 | T018 |
| `FR-508` | `promotion.rs`, `promotion-privacy.md` | — | T017 | T018 |
| `FR-509` | `promotion.rs`, `promotion-privacy.md` | — | T017 | T018 |
| `FR-510` | `promotion.rs`, `promotion-privacy.md` | privacy, data | T017 | T018 |
| `FR-511` | `validate.rs` | privacy | T012 | via `SC-438` → T145, T146 |
| `FR-512` | `promotion.rs`, `promotion-privacy.md` | privacy | T017 | T018 |
| `FR-513` | `promotion.rs`, `promotion-privacy.md`, `promote.rs` | global, privacy, data, migr, qs, rsrch, plan | T017 | T018, T076 |
| `FR-514` | `promotion_gate.rs`, `promotion.rs` | privacy, rsrch | T017, T080 | T006 |
| `FR-515` | `promotion.rs`, `promotion-privacy.md` | global, privacy, rsrch | T017, T124 | T018 |
| `FR-516` | `promotion.rs`, `paths.rs` | privacy, data, qs, rsrch | T017 | T020 |
| `FR-517` | `domain_isolation.rs`, `promotion.rs`, `0007_collaborative_global_memory.sql`, `migration.md`, `0002_memory_fts.sql`, `0003_collaborative_global_memory.sql`, `global.rs` | global, privacy, data, qs, rsrch, plan | T017, T025, T068, T079 | T005, T022, T083, T144 |
| `FR-518` | `promotion.rs` | privacy | T017 | T019 |
| `FR-519` | `promotion.rs`, `0007_collaborative_global_memory.sql`, `migration.md`, `0002_memory_fts.sql`, `privacy_promotion.rs` | privacy, data, qs | T017 | T022, T151 |
| `FR-520` | `promotion.rs`, `promotion-privacy.md`, `promote.rs` | privacy, data, rsrch | T017, T075, T124 | T018 |
| `FR-521` | `domain_isolation.rs`, `domain.rs`, `version.rs` | global, data, compat, qs, rsrch, plan | T007, T027 | T004 |
| `FR-522` | `global.rs`, `namespace_sync.rs` | compat, qs | T103, T129 | T187 |
| `FR-523` | `migration_alpha5.rs` | sync, data, migr | — | T028 |
| `FR-524` | — | migr | T026 | via `SC-405` → T028, T055 |
| `FR-525` | `migration_alpha5.rs` | migr | — | T028 |
| `FR-527` | `mcp_backward_compatibility.rs` | compat | T079, T125 | T188 |
| `FR-528` | `migration_alpha5.rs` | sync, data, migr, qs | T023 | T029 |
| `FR-529` | `version.rs` | compat, qs | T027 | via `SC-425` → T187 |
| `FR-530` | `migration_alpha5.rs` | sync, data, migr | — | T029 |
| `FR-531` | `handoff.rs`, `privacy_payloads.rs` | privacy, rsrch | T176 | T177 |
| `FR-532` | `sync.rs`, `handoff.rs` | compat | T175, T178 | via `SC-431` → T177 |
| `FR-533` | `scope_audit.rs` | — | — | T174 |
| `FR-534` | `wire.rs`, `privacy-sync.md` | privacy | T179, T181 | via `SC-467` → T184 |
| `FR-535` | `sync.rs`, `privacy_payloads.rs` | privacy, compat, rsrch | T175 | T180 |
| `FR-539` | `auth.rs` | rsrch | T039 | via `SC-433` → T051 |
| `FR-540` | — | — | T040 | via `SC-433` → T051 |
| `FR-541` | — | — | T041 | via `SC-434` → T051 |
| `FR-542` | — | rsrch | T041 | **accepted residual** (D457) |
| `FR-543` | `README.md`, `SECURITY.md`, `promotion-privacy.md`, `data-model.md`, `compatibility.md`, `global-memory.md`, `admin_lifecycle.rs` | rsrch | T056 | T184, T199 |
| `FR-544` | `validate.rs` | global, privacy, data, qs, rsrch | T012 | via `SC-438` → T145, T146 |
| `FR-545` | `promotion.rs`, `global.rs`, `promote.rs`, `promotion_gate.rs`, `global_content_validation.rs` | global, privacy, data, qs, rsrch | T017, T074, T075, T102, T123, T124, T130 | T085, T145, T146 |
| `FR-546` | `validate.rs`, `promotion_gate.rs`, `global_content_validation.rs` | privacy, data, qs, rsrch | T012 | T013, T085, T146 |
| `FR-547` | `global_content_validation.rs` | sync, privacy, data, compat, qs | — | T016, T149 |
| `FR-548` | `global.rs`, `promote.rs`, `promotion_gate.rs`, `global_content_validation.rs` | privacy, data, qs | T074, T075, T123, T124 | T085, T150 |
| `FR-549` | `validate.rs` | privacy, data, rsrch | T012 | T013 |
| `FR-550` | `promotion-privacy.md`, `data-model.md`, `compatibility.md`, `global-memory.md`, `README.md`, `SECURITY.md` | global, privacy, data, compat, rsrch, plan | T183 | T184 |
| `FR-551` | `privacy_payloads.rs` | privacy, data, rsrch | — | T021 |
| `FR-552` | `promotion-privacy.md`, `data-model.md` | privacy, data, rsrch | T185 | via `SC-441` → T020, T021 |
| `FR-553` | `api.rs`, `wire.rs`, `handlers.rs`, `main.rs` | ident, data, qs, rsrch | T042, T043, T044 | via `SC-442` → T052 |
| `FR-554` | `api.rs` | ident, qs | T042 | via `SC-442` → T052 |
| `FR-555` | `api.rs` | ident, data, qs | T042 | via `SC-442` → T052 |
| `FR-556` | `api.rs` | ident, data, qs | T042 | via `SC-442` → T052 |
| `FR-557` | `api.rs` | ident, data, qs | T042 | via `SC-442` → T052 |
| `FR-558` | `api.rs`, `admin_lifecycle.rs` | ident, data, qs, rsrch | T042 | T053 |
| `FR-559` | `api.rs` | ident, data, qs, rsrch | T042 | **accepted residual** (D457) |
| `FR-560` | `admin_lifecycle.rs` | ident, migr, rsrch | — | T045 |
| `FR-561` | `capability_upgrade_e2e.rs` | sync, migr, compat, qs, rsrch | T106 | T190 |
| `FR-562` | `namespace_sync.rs`, `capability_upgrade_e2e.rs` | sync, migr, compat, qs | T107, T108 | T113, T190 |
| `FR-563` | `sync.rs`, `capability_upgrade_e2e.rs` | sync, migr, compat, qs, rsrch | T093, T107 | T190 |
| `FR-567` | `global.rs`, `namespace_sync.rs` | global, sync, data, qs, rsrch | T116 | T117, T118 |
| `FR-568` | `outbox.rs` | global, sync, data, qs, rsrch | T100, T105 | via `SC-447` → T117 |
| `FR-569` | `applicability.rs`, `traits.rs` | global, data, qs, rsrch | T009, T071 | via `SC-429` → T091 |
| `FR-570` | `promotion-privacy.md`, `data-model.md`, `compatibility.md`, `global-memory.md`, `README.md`, `SECURITY.md` | global, recall, data, qs, rsrch | T186 | T184 |
| `FR-572` | — | ident, qs, rsrch | T034 | via `SC-442` → T052 |
| `FR-573` | `api.rs` | ident, rsrch | T042 | via `SC-442` → T052 |
| `FR-574` | `admin_lifecycle.rs` | ident, migr, rsrch | T033 | T045 |
| `FR-577` | `global.rs`, `global_content_validation.rs` | sync, privacy, data, compat, qs, rsrch, plan | T102, T130 | T147 |
| `FR-578` | `validate.rs` | privacy, data, qs, rsrch, plan | T012 | T014 |
| `FR-579` | `validate.rs`, `scope_audit.rs`, `promotion.rs` | privacy, data, rsrch, plan | T017 | T015 |
| `FR-580` | `validate.rs`, `global.rs` | privacy, data, rsrch, plan | T012, T074, T123 | T013 |
| `FR-581` | `global.rs`, `global_content_validation.rs` | sync, privacy, data, compat, qs, rsrch | T102, T130 | T147, T148 |
| `FR-582` | `0003_collaborative_global_memory.sql`, `global.rs`, `namespace_sync.rs` | sync, data, migr, rsrch, plan | T025, T101, T129 | T114 |
| `FR-583` | `knowledge.rs`, `multi_device_convergence.rs` | global, sync, data, rsrch | — | T098, T099 |
| `FR-584` | `context.rs`, `budget.rs`, `global_non_displacement.rs` | recall, qs, rsrch | T155 | T169, T170 |
| `FR-585` | `admin_lifecycle.rs` | ident, data, migr, qs, rsrch | T037 | T054 |
| `FR-586` | `compat_old_client.rs` | compat, rsrch, plan | — | T192 |
| `FR-587` | `api.rs`, `compat_old_client.rs` | compat, qs, rsrch | T191 | T193 |
| `FR-588` | `compat_old_client.rs`, `README.md` | compat, rsrch | T197 | T193 |
| `FR-589` | `sync.rs`, `namespace_sync.rs` | sync, rsrch | T094 | T112 |
| `FR-590` | `api.rs`, `admin_lifecycle.rs` | ident, rsrch | T031 | T200 |
---

## 3. Success criteria

| Criterion | Mechanism | Design | Builds | Falsifiable test |
|---|---|---|---|---|
| `SC-401` | `admin_lifecycle.rs` | qs, rsrch | — | T049 |
| `SC-402` | `admin_lifecycle.rs` | qs | — | T049 |
| `SC-403` | `admin_lifecycle.rs` | qs | — | T049 |
| `SC-404` | `admin_lifecycle.rs` | ident, rsrch | — | T050 |
| `SC-405` | `migration_alpha5.rs`, `admin_lifecycle.rs` | migr, qs | — | T028, T055 |
| `SC-406` | `authorization_audit.rs` | qs | — | T064 |
| `SC-407` | `authorization_audit.rs` | qs | — | T065 |
| `SC-408` | `authorization_audit.rs` | qs | — | T066 |
| `SC-409` | `domain_isolation.rs` | qs | — | T084 |
| `SC-410` | `domain_isolation.rs` | qs | — | T089 |
| `SC-411` | `multi_device_convergence.rs` | qs, rsrch | — | T110 |
| `SC-412` | `namespace_sync.rs` | sync, qs, rsrch | — | T112 |
| `SC-413` | `domain_isolation.rs` | qs | — | T135 |
| `SC-414` | `domain_isolation.rs` | qs, rsrch | — | T136 |
| `SC-415` | `domain_isolation.rs` | global, qs | — | T137 |
| `SC-416` | `domain_isolation.rs` | qs | — | T140 |
| `SC-417` | `domain_isolation.rs` | qs | — | T172 |
| `SC-418` | `global_non_displacement.rs` | recall, qs | — | T168 |
| `SC-419` | `global_non_displacement.rs` | recall, qs, plan | — | T169 |
| `SC-420` | `global_non_displacement.rs` | qs | — | T171 |
| `SC-421` | `promotion_gate.rs` | privacy, data, qs, rsrch | — | T087 |
| `SC-422` | `promotion_gate.rs` | global, privacy, data, qs, rsrch | — | T088 |
| `SC-423` | `privacy_promotion.rs` | qs | — | T151 |
| `SC-424` | `domain_isolation.rs` | global, privacy, data, qs, rsrch | — | T083, T144 |
| `SC-424a` | `global_content_validation.rs` | privacy, qs | — | T146 |
| `SC-425` | `namespace_sync.rs` | compat, qs | — | T187 |
| `SC-426` | `capability_upgrade_e2e.rs` | compat, qs | — | T190 |
| `SC-427` | `multi_device_convergence.rs` | qs | — | T111 |
| `SC-428` | `namespace_sync.rs` | compat, qs | — | T118 |
| `SC-429` | `domain_isolation.rs` | — | — | T091 |
| `SC-430` | `mcp_backward_compatibility.rs` | compat | — | T188 |
| `SC-431` | `handoff.rs`, `privacy_payloads.rs` | qs | T176 | T177 |
| `SC-432` | `migration_alpha5.rs` | — | — | T028 |
| `SC-433` | `admin_lifecycle.rs` | migr, rsrch | — | T051 |
| `SC-434` | `admin_lifecycle.rs` | — | — | T051 |
| `SC-435` | `README.md`, `SECURITY.md`, `admin_lifecycle.rs` | — | T056 | T199 |
| `SC-436` | `admin_lifecycle.rs` | rsrch | — | T050 |
| `SC-437` | `admin_lifecycle.rs` | — | — | T051 |
| `SC-438` | `global_content_validation.rs` | privacy, qs, rsrch, plan | — | T145, T146 |
| `SC-439` | `global_content_validation.rs` | sync, privacy, compat, qs | — | T149 |
| `SC-440` | `promotion_gate.rs`, `global_content_validation.rs` | privacy, qs | — | T086, T150 |
| `SC-441` | `paths.rs`, `privacy_payloads.rs` | privacy | — | T020, T021 |
| `SC-442` | `admin_lifecycle.rs` | ident, qs, rsrch | — | T052 |
| `SC-443` | `admin_lifecycle.rs` | ident, qs, rsrch | — | T053 |
| `SC-444` | `admin_lifecycle.rs` | ident, migr, rsrch | — | T045 |
| `SC-445` | `capability_upgrade_e2e.rs` | sync, compat, qs, rsrch | — | T190 |
| `SC-447` | `namespace_sync.rs` | — | — | T117 |
| `SC-448` | — | privacy, data, qs, plan | — | T014 |
| `SC-449` | `global_content_validation.rs` | privacy, data, compat, qs, rsrch | — | T147 |
| `SC-450` | `namespace_sync.rs` | sync | — | T114 |
| `SC-451` | `global_non_displacement.rs` | recall, qs, rsrch, plan | — | T170 |
| `SC-452` | `admin_lifecycle.rs` | ident, data, migr, qs, rsrch, plan | — | T054 |
| `SC-453` | `validate.rs`, `scope_audit.rs` | privacy, data, rsrch | — | T015 |
| `SC-454` | `validate.rs` | privacy, data, rsrch | — | T013 |
| `SC-455` | `knowledge.rs`, `multi_device_convergence.rs` | sync, data, rsrch | — | T098, T099 |
| `SC-456` | `global_content_validation.rs` | sync, privacy, data, compat, rsrch | — | T148 |
| `SC-457` | `compat_old_client.rs` | compat, rsrch | — | T192 |
| `SC-458` | `compat_old_client.rs` | compat, qs, rsrch | — | T193 |
| `SC-459` | `domain_isolation.rs` | global, data, rsrch | — | T004 |
| `SC-460` | `domain_isolation.rs` | global, qs, rsrch | — | T134 |
| `SC-461` | `global_non_displacement.rs` | privacy, rsrch | — | T173 |
| `SC-462` | `global_non_displacement.rs` | recall, qs, rsrch | — | T154 |
| `SC-463` | `context.rs` | recall, qs, rsrch | — | T159 |
| `SC-464` | `global_non_displacement.rs` | recall, qs, rsrch | — | T166 |
| `SC-465` | `authorization_audit.rs` | proj-auth, qs, rsrch | — | T067 |
| `SC-466` | `domain_isolation.rs` | global, qs, rsrch | — | T138 |
| `SC-467` | `promotion-privacy.md`, `data-model.md`, `compatibility.md`, `global-memory.md`, `README.md`, `SECURITY.md` | privacy, data, compat, rsrch | — | T184 |
| `SC-468` | `context.rs` | recall, qs, rsrch | — | T163 |
| `SC-469` | `privacy_payloads.rs` | data, qs, rsrch | — | T072 |
| `SC-470` | `admin_lifecycle.rs` | ident, rsrch | — | T200 |
---

## 4. What the pass found

### 4a. Seventeen requirements had no success criterion (D457)

Closed by `SC-453`–`SC-469`. Six were written by this audit round; eleven predated it.

| Requirement | The obligation nothing tested | Closed by |
|---|---|---|
| `FR-579` | the validator is the only implementation of its classes | `SC-453` — an audit that fails *when a second implementation appears*, not one that inspects the code as it stands |
| `FR-580` | an empty project-identity set passes rather than refuses | `SC-454` — the vacuous case and the unevaluable case asserted **separately** |
| `FR-583` | `writer_seq` is never an ordering key or tiebreak | `SC-455` — permuted, withheld and renumbered sequences produce identical derived output |
| `FR-581` | an ingest refusal is distinguishable from a capability refusal | `SC-456` — and distinguishable *without inspecting a message string* |
| `FR-586` | a pre-004 client keeps syncing projects | `SC-457` — against a **real** pre-004 binary |
| `FR-587`, `FR-588` | the removed-route response and the operator documentation | `SC-458` |
| `FR-521` | **`MemoryScope` is unchanged** — the feature's central constraint | `SC-459` — the variant list and the `CHECK` text, so a fifth variant fails |
| `FR-455`, `FR-515` | no tool action creates an authoritative team entry | `SC-460` — every action enumerated from the schema, not a hardcoded list |
| `FR-506` | Cairn never promotes automatically | `SC-461` — global record count unchanged across a seeded workload |
| `FR-476` | personal is considered ahead of team | `SC-462` — the case where only one of the two fits |
| `FR-478` | reasons are diagnostic-only | `SC-463` — the rendered form inspected field by field |
| `FR-482` | an importance hint changes nothing | `SC-464` — byte-identical context across every hint value |
| `FR-418` | no route adds the caller to a project | `SC-465` — every live route exercised, not one named route asserted absent |
| `FR-462` | two disagreeing authoritative entries both survive | `SC-466` — including that ratification order is irrelevant |
| `FR-550` | documentation names the mechanism behind each guarantee | `SC-467` — a lint that **fails on the forbidden phrasing**, since this is the requirement that exists *because* the documentation lied once |
| `FR-471` | no cross-domain relevance comparison | `SC-468` — by construction: distinct ranking types, so it would not compile |
| `FR-438` | project traits stay local | `SC-469` — the wire inspected across projects with all-distinct traits |

Four of the seventeen are verified **by construction** rather than by assertion — `SC-455`'s
absent sequence field, `SC-459`'s variant list, `SC-468`'s distinct ranking types, and
`SC-424`'s absent columns. That is the preferred shape wherever it is reachable, for the reason
Principle V gives: a comparison that would not compile cannot be reintroduced by someone who
never read the requirement.

### 4b. One success criterion had no verifying task

`SC-435` — *"a reader of the shipped documentation can state, without reading the source, who is
ultimately able to obtain administrator access and why"* — was cited only by `T056`, which
**writes** the trust statement. A task that authors a sentence cannot verify that a reader can
answer a question from it. Closed by `T199`, which asserts the statement's presence in
`README.md` and `SECURITY.md` and that it names both the mechanism (the environment-named
account, restored to admin and active on every start) and the reason (whoever controls the host
controls the environment).

### 4c. Fifteen requirements deliberately carry no dedicated criterion

Recorded rather than quietly counted as covered: `FR-415`, `FR-416`, `FR-419`, `FR-423`,
`FR-427`, `FR-428`, `FR-439`, `FR-440`, `FR-441`, `FR-457`, `FR-461`, `FR-465`, `FR-520`,
`FR-542`, `FR-559`.

Each has an implementation task and an acceptance scenario, and each is a **positive capability**
("an administrator MUST be able to view a project's full membership list") or a local
housekeeping property, not a privacy or authorization boundary. The judgment is that a criterion
per capability would inflate the criteria set without changing what any test does — the
walkthrough in `quickstart.md` exercises all fifteen. The judgment is recorded here so that
"every requirement has a criterion" is **not** claimed, because it is not true.

Two of the fifteen are the ones to revisit first if this call is reconsidered: `FR-440` and
`FR-461` state that personal and team records are immutable after creation, which is closer to
an invariant than to a capability, and both currently rest on an implementation task plus the
`no UPDATE ... SET content` rule stated in `global-memory.md`'s invariants.

### 4d. Twelve requirements have a verifying task and no implementation task

`FR-422`, `FR-446`, `FR-500`, `FR-523`, `FR-525`, `FR-530`, `FR-533`, `FR-547`, `FR-551`,
`FR-560`, `FR-583`, `FR-586`. This is **correct, not a gap**: each is a property of code built
for other reasons rather than a thing to build. Nothing is implemented to satisfy "no origin
digest appears in any transmitted payload" or "an interrupted migration leaves the store on its
prior version" — the work is the assertion. Listed so a future reader does not mistake the em
dash in the *Builds* column for an omission.

### 4e. Two "open, non-blocking" checklist items were defects (D458)

This file previously listed CHK007, CHK009, CHK018 and CHK027 as open items that could not
invalidate implementation. Inspecting them individually rather than accepting that judgment:

| Item | Verdict | Why |
|---|---|---|
| CHK007 — a project's membership falling to zero | **not a defect** | The mechanism is already required. A zero-admin server is unrecoverable through any supported API, hence `FR-413`'s atomic floor; a zero-member project is recoverable, because `FR-419` says "an existing member **or an admin**" and `project-authorization.md` §2 states the server-admin bypass exists precisely to bootstrap a project with none left. |
| CHK027 — the universal-applicability default at promotion | **not a defect** | `FR-435`/`FR-460` are stated about the *record*, not the creation path, so a promoted entry with no applicability facts is covered without restatement. Restating it would create a second normative sentence about one obligation, which D455 exists to prevent. |
| CHK009 — "the documented background interval" | **DEFECT** | `SC-412`'s bound had no referent, and `sync-namespaces.md` §5 had argued *against* naming one. Followed literally, the pull-due timer elapses every 500 ms tick, so three namespaces poll six times a second per machine indefinitely; backoff does not contain it, because backoff engages on failure and these succeed. Closed by `FR-589`, `PULL_INTERVAL_SECONDS = 30`, and `SC-412` asserting 60 seconds. |
| CHK018 — re-enabling and revoked tokens | **DEFECT** | Decided correctly in one contract paragraph and one contract invariant; no requirement, no criterion, no test. A credential-lifetime rule holding by the author's intention. An implementer clearing `revoked_at` alongside `status` resurrects every token the account held before it was disabled, and every existing test still passes, because they all assert the disable side. Closed by `FR-590`, `SC-470`, and `T200` asserting the re-enable case separately. |

Both defects had carried a parenthetical reading "confirm this asymmetry is deliberate" or
"confirm this is meant to carry through" across three review passes. An open item with a
reassuring parenthetical is indistinguishable, to a later reader, from a confirmed one — which is
the same failure shape as certification by citation, applied to a checklist instead of a matrix.

---

## 5. Where the file discipline is not met

`tasks.md` states that every task names the exact file it changes. **Fifty-one** tasks name none,
down from fifty-seven after the adversarial pass audited all of them one by one:

| Class | Count | Disposition |
|---|---|---|
| Legitimately cross-cutting, command-only, or verification-only | 3 | `T001` (the prerequisite gate), `T003` (the build loop), `T126` (ratification's absence from the tool surface, which is a property verified by `T134`, not an edit) |
| Target unambiguous from the phase's other tasks, the contract, or `plan.md`'s structure map | 48 | Server endpoint work (`T032`–`T041`, `T057`, `T058`), store and core work (`T069`–`T081`, `T096`, `T097`), sync work (`T103`–`T108`), team work (`T120`–`T132`), context assembly (`T153`, `T157`–`T167`), migrations (`T023`, `T024`, `T026`) |
| **Ambiguous — repaired** | 6 | listed below |

The six repaired, and why each could have gone wrong:

- **`T021`** named no test file for "the origin digest never appears on the wire". Now
  `tests/tests/privacy_payloads.rs`.
- **`T031`** ("revoke all tokens at the moment of disabling") named no file and silently shared an
  endpoint with `T033`, which implements the same `PATCH` handler. Two tasks, one handler, no file
  named: the revocation could have landed in a second transaction, which is exactly what
  `SC-404`'s "in the same transaction" wording forbids. Now names the handler and the file, and
  carries `FR-590`.
- **`T059`** said "add the missing ones found by audit" — unbounded scope with no file. Now names
  `api.rs` and `sync.rs`, names the known gap (`lookup_projects`, whose doc comment already claims
  a filter its SQL lacks), and bounds the scope to routes existing when `T001` records the
  prerequisite commit.
- **`T062`** ("ensure discovery returns nothing to a non-member") overlapped both `T059` and the
  prerequisite patch, so it could have been done twice or not at all. Now names the function and
  says explicitly to verify rather than reapply if the prerequisite already landed it.
- **`T155`** — the load-bearing one. It specified the global cap formula including
  `remaining_non_reserve` and named no file. `remaining_non_reserve` does not exist; the obvious
  reading is to use `general_remaining()`, which includes reserve that `release_reserve()`
  returned — precisely the defect D449 exists to prevent. Now names **both**
  `cairn-core/src/context.rs` and `cairn-core/src/budget.rs` and states that both are required.
- **`T185`** said "correct the same documents", a back-reference that stopped resolving when
  `T184`'s document list grew. Now names its own two files.

One further task was repaired for a related reason without being in the no-file class: **`T163`**
read "`context.rs` (**or wherever** ranking inputs are typed)". A compile-time structural
assertion whose location is hedged is not a structural assertion. It now names where `T162`
declares the types, and requires the failure to be demonstrated in a scratch build.

The remaining 51 are not blocking: each cites a requirement, each requirement resolves to a
contract section naming the module, and §2's *Design* column shows which. They should be
tightened as each phase lands rather than by speculative edit now.

---

## 6. The four re-opened certifications (D456), re-derived

Each of these was previously certified by the citation pass. Each is re-derived here against the
mechanism, which is the point.

**F6 — the never-transmitted field set.** Previously **CLEAN**; it was **wrong**.
`data-model.md` listed `personal_knowledge.writer_id`/`writer_seq` and their `team_knowledge`
counterparts as never transmitted, while the local schema declared both `NOT NULL` under
`UNIQUE (writer_id, writer_seq)` — an invariant no pulled record could satisfy. The certification
had been checked against the prose describing the serialized forms rather than against the forms.
Re-derived field by field: both now cross the wire and carry server columns under the same unique
constraint (D448, `FR-582`), the `writer_identity` **table** stays local-only, and the distinction
is stated as *the stamp travels, the registry that minted it does not*. **Now CLEAN, on a
different basis.**

**F11 — the Layer A / Layer B split.** Previously **CLEAN**; it claimed Layer A for fields that
are Layer B. "Structurally impossible" is now reserved for columns that do not exist —
`project_id`, evidence references, observation identifiers, and any verification field at all.
`content`, `topic_key`, `value_key` and every applicability *value* are Layer B: free text, kept
clean by a validator, not by absence. `SC-467` makes this self-enforcing with a lint that fails on
the forbidden phrasing. Running that lint as a dry run over the whole artifact set found exactly
one live violation — `compatibility.md`'s drift table, since corrected. **Now CLEAN, and now
testable.**

**F12 — "all entry points".** Previously **CLEAN** at four. The count is **five** (D447), the
fifth being server-side synchronization ingest, and the four that existed were all client-side —
so the privacy boundary held only while the client cooperated. Every occurrence was re-counted
rather than pattern-replaced, because a count is exactly the kind of claim that survives a
citation check unexamined. **Now CLEAN at five.**

**E3 — the budget invariant `estimated_tokens <= budget`.** Previously **CLEAN**, and it still
holds — but it had to be re-derived, because D449 and D450 both changed what the global sections
may spend. Global allowance is now `min(floor(total_budget * 0.15), remaining_non_reserve)`, a
floor of two terms that bind in different situations. Both are bounded above by the budget, so the
invariant is preserved; `SC-419` asserts it and `SC-451` asserts the non-reserve term separately.
**Still CLEAN, under a changed rule.**

---

## 7. Cross-feature citations

`FR-057` (Feature 001), `FR-391`, `FR-393`, `FR-395`, `FR-397` and `SC-331` (Feature 003) appear
in `research.md`, `plan.md` and `contracts/sync-namespaces.md`. Every one is another feature's
numbering, cited as precedent or as the amended constitutional constraint (`FR-391`), never as a
004 id. Two were qualified in place during this pass — an unlabelled `FR-057` in a 004 document
reads as a 004 id to anyone scanning it.

## 8. Unallocated id ranges

`FR-429`, `FR-430`, `FR-447`–`FR-450`, `FR-466`–`FR-468`, `FR-479`, `FR-483`–`FR-485`,
`FR-503`–`FR-505`, `FR-526`, `FR-536`–`FR-538`, `FR-575`, `FR-576` were never allocated. They are
headroom inside a subject block, not missing requirements.

Separately, nine ids were **deleted** by D455 as duplicates — `FR-406`, `FR-433`, `FR-443`,
`FR-494`, `FR-564`, `FR-565`, `FR-566`, `FR-571`, `SC-446`. Their numbers are never reused and
the surviving ids are never renumbered, so an external reference to a retired id resolves to
"retired" rather than to a different requirement. Every citation of a deleted id across every
artifact was repointed to the surviving id; a sweep of all sixteen documents finds no dangling
citation except inside D455's own record, which names them deliberately.

---

## 8a. Corrections folded back from Phase 2 implementation

Six contradictions surfaced while Phase 2 implemented these artifacts. Five were wording or
dialect defects; one was a **missing invariant**, and it is worth separating from the others.

| # | Contradiction | Resolution |
|---|---|---|
| 1 | `promotion-privacy.md` named four classes differently from `data-model.md` and `tasks.md` (`home_dir_reference`, `file_url`, `env_var_assignment`, `secret_shaped`) | canonical names are `home_dir_ref`, `file_uri`, `env_assignment`, `encoded_secret_shape` — the two artifacts that agreed, one of them the task list an implementer reads. No aliases kept. |
| 2 | `invalid_applicability` read as a tenth content class | **nine** content classes; `invalid_applicability` is the applicability *format* refusal, travelling through the same type but absent from `CONTENT_CLASSES` and from SC-453's audit list |
| 3 | `PromotionRejection` had `check` + `class` in one artifact and `check` alone in another | both, with distinct semantics: `check` is always set and names the gate check; `class` is set only for check 1 and carries what the validator returned. Neither is a `String`, so the type still cannot hold offending text. |
| 4 | `sync-namespaces.md` §6 listed **12** outbox entity types; `data-model.md` said "exactly two variants… ten" | **twelve** — §6 was right. See below. |
| 5 | `sync-namespaces.md` §3's `writer_identity` lacked the singleton `id` column `data-model.md` §2.10 requires | added, with the reason it exists |
| 6 | `migration.md`'s `users.role` backfill SQL was invalid PostgreSQL (a CTE referenced outside its statement) and relied on unspecified `UNION ALL` ordering | replaced with the implemented algorithm: CTE repeated per statement, explicit `priority` and `ORDER BY` |

**Contradiction 4 was a missing invariant, not a numbering slip.** Both relations tables exist
in server Postgres as well as in the local store (`data-model.md` §3.5, `global-memory.md` §2,
server migration `0003`), and a table on the server is reachable only through the outbox. A
relation also has nowhere else to travel: unlike an applicability fact, which rides inside its
knowledge row's payload, a relation names *two* rows and belongs to neither. Without
`personal_knowledge_relation` and `team_knowledge_relation` as entity types, those tables could
never be populated and FR-493's "disagreement is expressed as relations" would hold locally and
nowhere else. The `outbox.entity_type` CHECK and `tasks.md` T023 now carry twelve names, and the
project-less CHECK covers all four domain types rather than only the two knowledge ones.

Two behaviour rules were also tightened, in the code first and then written down:

- **`command_shaped`** was implemented as "a known command name followed by a flag or a path",
  which admitted `cargo test`, `rm target`, `sudo reboot`, `npm install` and `git status`. The
  rule is now grammatical position — prose names a tool as a noun, an invocation puts it in
  imperative head position — and `promotion-privacy.md` §2a states it.
- **`project_identifying`** folds separators on both sides, because an applicability value must
  be `[a-z0-9_]` after normalization and a hyphenated project name therefore cannot appear
  there in its own spelling at all. The over-refusal this causes across a sentence boundary is
  recorded as the accepted direction of error rather than left to be rediscovered.

---

## 9. Verdict

**Every one of the 158 functional requirements and 69 success criteria resolves to a mechanism, a
design artifact that states it, a task, and a falsifiable test** — directly, through a named
success criterion, or, for the fifteen positive-capability requirements in §4c, through an
acceptance scenario and the quickstart walkthrough with that limitation recorded rather than
hidden.

Open items, carried forward rather than closed:

1. **Fifty-one tasks name no file** (§5), all three classes dispositioned, the six ambiguous ones
   repaired. A weakness of the task list; the rest to be tightened as each phase lands.
2. **Fifteen requirements have no dedicated criterion** (§4c), two of which — `FR-440` and
   `FR-461` — are closer to invariants than to capabilities and are the first to revisit.
3. **The four audits cannot be proven from the artifacts alone** — `SC-453`, `SC-465`, `SC-467`,
   `SC-468`. Each names its mutation and each requires that mutation to be demonstrated, but
   whether the audit actually fails under it is only observable after implementation. This is a
   correctly-specified obligation on the implementer, not an unresolved design question — and it
   is the one item on this list that no amount of further specification work can close.
4. **`T001` gates everything on the security prerequisite, which has not landed on `main`.**
   Verified directly: `POST /api/auth/register` and `POST /api/projects/{id}/join` are still
   routed, and `lookup_projects` still has no membership filter while its doc comment claims one.

Items 1–3 do not block correct implementation. Item 4 does block *starting*: it is the difference
between specification-readiness and execution-readiness, and it is not a defect in these
artifacts — it is a dependency they correctly declare.
