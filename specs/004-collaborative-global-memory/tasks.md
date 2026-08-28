---

description: "Task list for Cairn Collaborative Global Memory (004-collaborative-global-memory)"
---

# Tasks: Cairn Collaborative Global Memory

**Input**: Design documents from `/specs/004-collaborative-global-memory/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [migration.md](./migration.md),
[compatibility.md](./compatibility.md), [contracts/](./contracts/),
[quickstart.md](./quickstart.md), [traceability.md](./traceability.md),
[security-prerequisite.md](./security-prerequisite.md)

**Generated from**: the design at branch `004-collaborative-global-memory`, based on `main`
@ `96178fc` (v0.1.0-alpha.5) — 160 FR, 70 SC, decisions D401–D458, constitution v1.1.0. This
revision incorporates the second `/speckit-analyze` repair addendum (D446–D456), on top of
the first (D433–D445): the shared content validator grows from seven classes to nine and
gains a `project_identities` input, with project-identity screening now built into the
validator itself rather than living only in the promotion gate; server-side synchronization
ingest becomes the fifth mandatory entry point, screening against the union of the pushing
user's project memberships and refusing permanently rather than trusting the client;
`writer_id`/`writer_seq` now cross the wire and gain server columns instead of being claimed
never-transmitted; the global budget is computed from the non-reserve pool only, at
`min(floor(total_budget * 0.15), remaining_non_reserve)`; per-item exclusion reasons are
explain-only, reachable solely through the diagnostic path and structurally absent from the
rendered briefing; a personal or team record carries no verification field of any kind, not
merely no authority above `attested`; an API token past its expiry is refused
indistinguishably from a revoked one; and the old-client-against-a-new-server direction is
now specified alongside the already-specified new-client-against-an-old-server one. The
first round's fixes remain in force: the shared `validate_global_content` gate, per-machine
origin digest, administrator password reset, atomic never-zero-admins, the capability
re-probe cycle, personal-vs-team server-instance binding, the `topic`-free applicability
vocabulary, and the temporary-password lifecycle. A subsequent semantic-completeness pass
over the whole requirement set (D457, recorded in `research.md`) added seventeen success
criteria (SC-453–SC-469) for requirements that had a task and an acceptance scenario but no
measurable criterion; eight requirements (FR-415, FR-428, FR-441, FR-461, FR-465, FR-520,
FR-542, FR-559) are a documented, accepted residual with no dedicated criterion, covered by
acceptance scenario and task alone.

**Hard prerequisite**: the hardening patch in
[security-prerequisite.md](./security-prerequisite.md) must have landed on `main` before
T004 begins. It is not part of this task list. Everything in Phase 4 assumes public
self-registration and open project self-join are already gone; building a membership model
on top of an endpoint that hands membership to anyone would be pointless work.

**Tests**: included, and not optional. A large share of this feature's requirements are
**negative** — no `MemoryScope::Global`, no global knowledge in the Level 0 reserve, no
relation across domains, no timestamp comparison, no agent-authored team policy, no
absolute path on the wire, no project-sync stall against an old server, no bypass of the
content validator at any of its five entry points, no unvalidated record surviving
server-side ingest, no verification field of any kind on a promoted record, no released
Level 0 reserve spent by a global section, no origin digest on the wire, no password reset
that re-enables a disabled account, no expired token distinguishable from a revoked one,
no two concurrent demotions both succeeding. A negative requirement that no test asserts is
not implemented at all; it is a sentence. The twenty-six highest-risk negatives are written
**before** the code they constrain, so each is seen to fail for the right reason first.
Feature 003 learned this the hard way: `scope_audit.rs` had been passing vacuously since it
was written, and protected nothing (T174).

## Format: `[ID] [P?] [Story] Description`

- **[P]**: safe to run in parallel — different files, no dependency on an incomplete task,
  no shared schema change, no shared module, no shared contract
- **[Story]**: US1–US6 from [spec.md](./spec.md). Setup, Foundational, Repairs and Polish
  carry no story label because they serve every story rather than one
- Every task names the exact file it changes

## Path conventions

Rust workspace at the repository root: `crates/cairn-core`, `crates/cairn-store`,
`crates/cairnd`, `crates/cairn`, `crates/cairn-server`, with the end-to-end harness in
`tests/tests/`. The web app under `web/` is **not modified** by this feature — admin UI is
deferred to Feature 005.

---

## Phase 1: Setup

**Purpose**: confirm the ground this feature stands on.

- [ ] T001 Verify the security prerequisite has landed on `main`: `POST /api/auth/register` and `POST /api/projects/{id}/join` return 404, `lookup` filters by membership, and `tombstone` carries a `project_id` predicate. Record the commit in the implementation log. **Blocks everything.**
- [ ] T002 [P] Add the `004` feature entry to `CHANGELOG.md` under Unreleased
- [ ] T003 [P] Confirm `cargo build` (not `cargo build --tests`) is in the local verification loop, since `tests/tests/*` spawns prebuilt binaries and `--tests` leaves them stale

**Checkpoint**: the prerequisite is proven landed, and the build loop will not silently test stale binaries.

---

## Phase 2: Foundational (blocking prerequisites)

**Purpose**: the types, schemas and pure functions every story needs. No story-specific behavior.

**⚠️ No user story work begins until this phase completes.**

### Negative tests written first

- [ ] T004 [P] Test `tests/tests/domain_isolation.rs`: `MemoryScope` has exactly four variants and `memories` was not rebuilt — assert the `CHECK` text, the stored representation and the exhaustive match are byte-identical before and after, so adding a fifth variant fails (FR-521, SC-459)
- [ ] T005 Test `tests/tests/domain_isolation.rs`: no relation table can link records belonging to two different domains — assert the constraint is structural, not merely unused (FR-517)
- [ ] T006 [P] Test `tests/tests/promotion_gate.rs`: an applicability value outside the closed vocabulary is **rejected** on promotion — assert the rejection, not merely that valid values are accepted (FR-514)

### Core types

- [ ] T007 Add `KnowledgeDomain`, `ServerRole`, `UserStatus`, `ApplicabilityKind` to `crates/cairn-core/src/domain.rs`, leaving `MemoryScope` and `resolve_scope` untouched (FR-431, FR-521)
- [ ] T008 [P] Add `PersonalKnowledge`, `TeamKnowledge`, `TeamState` to `crates/cairn-core/src/global.rs` (new module) (FR-431, FR-451)
- [ ] T009 [P] Add `ApplicabilityFact` and the match predicate to `crates/cairn-core/src/applicability.rs` (new module) — AND across kinds, OR within a kind, no facts means universal, and the closed vocabulary is exactly `language | tool` with `topic` removed (FR-434, FR-435, FR-436, FR-569)
- [ ] T010 [P] Add `ProjectTrait` and `SyncNamespace` to `crates/cairn-core/src/domain.rs` (FR-437, FR-486)
- [ ] T011 Add `WriterIdentity` to `crates/cairn-core/src/domain.rs` and document that it is not a device registry (FR-490, FR-491)

### The shared content validator (D433, grown to nine classes by D446)

- [ ] T012 Implement `validate_global_content(content, topic_key, value_key, applicability, project_identities: &[ProjectIdentity]) -> Result<(), GlobalContentRejection>` as a pure, total function in `crates/cairn-core/src/validate.rs` (new module): nine rejection classes — the original seven (`absolute_path`, `home_dir_ref`, `drive_letter_path`, `file_uri`, `credentialed_url`, `env_assignment`, `encoded_secret_shape`) plus `project_identifying` and `command_shaped`; every applicability fact's value is run through the same nine classes as `content` rather than left as an unchecked open string; `project_identifying` screens `content`, `topic_key`, `value_key` and every applicability value against `project_identities` and PASSES when `project_identities` is empty — the one documented exception to fail-closed, to be distinguished from a check that cannot be evaluated at all, which still fails closed (FR-544, FR-546, FR-549, FR-578, FR-580, FR-511)
- [ ] T013 [P] Unit-test `validate_global_content` in `crates/cairn-core/src/validate.rs`: one test per class — an absolute path, a home-directory reference, a drive-letter path, a `file://` reference, a credentialed URL, an environment-variable assignment, a secret-shaped run, a project-identifying token, and a shell command invocation — each independently rejected; a **separate** test asserts that an **empty** `project_identities` set PASSES the `project_identifying` check (FR-580); a **separate** test asserts that a check which genuinely cannot be evaluated for lack of information still fails closed (FR-549) — these are two distinct behaviors, asserted by two distinct tests, and MUST NOT be collapsed into one assertion (FR-546, FR-549, FR-580, SC-454)
- [ ] T014 [P] Unit-test `validate_global_content`: an applicability **value** that would be refused as free-text content — e.g. an internal product name that also reads as a project-identifying token — is refused as an applicability value by the same nine classes, distinct from T006's rejection of a `kind` outside the closed vocabulary (FR-578, SC-448)
- [ ] T015 Audit `crates/cairn-core/src/validate.rs` and the rest of the workspace: `validate_global_content` MUST be the only implementation of its nine classes; write the audit so it demonstrably **fails when a second implementation is introduced** — prove this by seeding a throwaway duplicate check (e.g. a second inline absolute-path or shell-command detector) in a test fixture and asserting the audit catches it, rather than merely inspecting today's code, which is exactly the failure mode that left `scope_audit.rs` (T174) passing vacuously (FR-579, SC-453)
- [ ] T016 [P] Unit-test `validate_global_content`: a `GlobalContentRejection` carries only the rejection class — assert the offending substring never appears in its `Debug` or `Display` output, across all nine classes (FR-547)

### The promotion gate (pure function, eight checks after D446)

- [ ] T017 Implement the gate in `crates/cairn-core/src/promotion.rs` (new module) as a pure function over content, topic key, value key, proposed applicability, project traits and project identity, returning `PromotionRejection`, calling `validate_global_content` — passing the source project's identity as `project_identities` — for its `shared_content_validation` check (check 1) rather than re-implementing it; the gate MUST NOT re-implement `project_identifying`, which is no longer one of its checks and is satisfied entirely by delegating to the validator (D446) (FR-506, FR-507, FR-508, FR-509, FR-510, FR-512, FR-513, FR-514, FR-515, FR-516, FR-517, FR-518, FR-519, FR-520, FR-545, FR-579)
- [ ] T018 Implement the remaining seven checks in fixed order after `shared_content_validation` — `source_not_active`, `no_subject`, `evidence_leak`, `verification_reset` (never refuses; documents the absence of any verification field rather than resetting one), `not_a_member`, `origin_computation` (never refuses), `evaluation_incomplete` — fail-closed, first failure reported by name, per `contracts/promotion-privacy.md` (FR-507, FR-508, FR-509, FR-510, FR-512, FR-513, FR-515, FR-520)
- [ ] T019 [P] Unit-test each of the eight checks independently in `crates/cairn-core/src/promotion.rs`, including that a check which cannot be evaluated rejects (FR-518)
- [ ] T020 [P] Implement the salted origin digest reusing the approach in `crates/cairn-core/src/paths.rs`; test that two promotions from the same project on one machine share a digest, and that the same two promotions made on a second machine share a *different* one — this divergence is correct and intentional (D434) (FR-516, SC-441)
- [ ] T021 [P] Test `tests/tests/privacy_payloads.rs`: the origin digest never appears in any type serialized to the wire or to an outbox payload — assert by construction that no such type has a digest field (FR-551, SC-441)

### Migrations

- [ ] T022 Write `crates/cairn-store/migrations/0007_collaborative_global_memory.sql` per `migration.md`: the personal and team tables with no column for a project identifier, evidence reference, observation identifier, file path, command, or elevated verification authority, both relations tables, both applicability tables, the two FTS5 tables with their triggers mirroring `0002_memory_fts.sql`, `project_traits`, `writer_identity`, `sync_cursor` (FR-459, FR-486, FR-517, FR-519)
- [ ] T023 Rebuild `outbox` inside `0007` to widen the `entity_type` CHECK to **twelve** names — the eight that existed plus `personal_knowledge`, `personal_knowledge_relation`, `team_knowledge`, `team_knowledge_relation` — and add the `namespace` column: create, copy, drop, rename, recreate indexes, release claims. The two relation types are not optional: both relations tables exist in server Postgres as well as locally (`data-model.md` §3.5), and a table on the server is reachable only through the outbox; a relation names two rows and belongs to neither, so unlike applicability it cannot travel inside a parent's payload. The project-less CHECK covers all four domain types, not only the two knowledge ones (FR-528, Complexity Tracking item 1)
- [ ] T024 Backfill `sync_meta` into `sync_cursor` as namespace `project:<project_id>`, preserving each `pull_cursor` verbatim (FR-486, FR-487)
- [ ] T025 Write `crates/cairn-server/migrations/0003_collaborative_global_memory.sql`: the `users` columns, `server_instance`, `project_members.added_by_user_id`, `api_tokens.expires_at`, and the personal and team tables with their relations and applicability tables, each carrying `writer_id TEXT NOT NULL` and `writer_seq BIGINT NOT NULL` under `UNIQUE (writer_id, writer_seq)` — the same constraint the local store carries, now enforced on both sides — with the same structural column absence as T022 (FR-415, FR-417, FR-419, FR-517, FR-582)
- [ ] T026 Implement the deterministic `users.role` backfill — environment-named account, else oldest by `created_at`, all others member, never zero admins (FR-414, FR-524)
- [ ] T027 Bump the server schema version to 3 and add `SCHEMA_3_CAPABILITIES` in `crates/cairn-server/src/version.rs` (FR-521, FR-529)
- [ ] T028 Test `tests/tests/migration_alpha5.rs`: rebuild the real prior schema through actual migrations 1–6, assert row and byte equality for untouched tables, assert an interrupted migration leaves the store on the old version, assert the `users.role` backfill across seeded configurations — environment-named account present, absent, one legacy account, several legacy accounts — always ends with an administrator and never zero (FR-523, FR-525, SC-405, SC-432)
- [ ] T029 Test `tests/tests/migration_alpha5.rs`: outbox rows in flight before the rebuild survive it and keep their original idempotency keys (FR-528, FR-530, Complexity Tracking item 4)

**Checkpoint**: both databases migrate cleanly and reversibly-safely, the core types exist, the shared content validator and the promotion gate are tested pure functions, and the origin digest is proven local-only, with no storage behind any of it yet.

---

## Phase 3: US1 — Administered accounts (Priority: P1) 🎯 MVP

**Goal**: an operator runs a server where accounts are created by an administrator and nobody self-registers, and no sequence of legal operations can lock administration out.

- [ ] T030 [US1] Enforce `role` and `status` in `crates/cairn-server/src/auth.rs`; a disabled account is refused authentication by any means, including an otherwise-valid password (FR-410)
- [ ] T031 [US1] Revoke all of a user's API tokens at the moment of disabling, in the same transaction as the status change, in the `PATCH /api/admin/users/{id}` handler in `crates/cairn-server/src/api.rs` that T033 also implements — one handler, one transaction; re-enabling MUST NOT clear `revoked_at` (FR-409, FR-590)
- [ ] T032 [US1] Implement `POST /api/admin/users` returning a temporary password exactly once, setting `must_change_password` (FR-401, FR-403, FR-404)
- [ ] T033 [US1] Implement `GET /api/admin/users` and `PATCH /api/admin/users/{id}` for role and status changes; the never-zero-admins guarantee MUST be enforced by a single atomic conditional statement — conditioned on another active administrator still existing — never by a separate count followed by an update, and every such operation MUST serialize against every other one on a single application-wide lock held for the duration of the transaction so the check and the write cannot interleave (FR-402, FR-408, FR-411, FR-412, FR-413, FR-574)
- [ ] T034 [US1] Implement `POST /api/auth/password`, ending the must-change state and invalidating the temporary credential immediately (FR-405, FR-572)
- [ ] T035 [US1] Gate every authenticated route except the password change with `password_change_required` while the flag is set; the temporary credential authenticates to nothing else (FR-407)
- [ ] T036 [US1] Refuse to mint an API token while `must_change_password` is set (FR-407)
- [ ] T037 [US1] Implement optional `expires_at` on API tokens; refuse an expired token with a refusal identical in status and body to a revoked token's, so expiry cannot be probed (FR-417, FR-585)
- [ ] T038 [US1] Implement `server_instance` single-row generation and expose the id on `GET /api/version` (FR-415, FR-416)
- [ ] T039 [US1] Extend `ensure_admin` in `crates/cairn-server/src/auth.rs` so its `ON CONFLICT` also restores `role='admin'` and `status='active'` (FR-539)
- [ ] T040 [US1] Exempt the environment-named account from `must_change_password` (FR-540)
- [ ] T041 [US1] Refuse to demote or disable the environment-named account, naming `CAIRN_ADMIN_EMAIL` in the error (FR-541, FR-542)
- [ ] T042 [US1] Implement `POST /api/admin/users/{id}/reset-password` in `crates/cairn-server/src/api.rs`: returns a new temporary password exactly once on the response itself, invalidates the previous password immediately, revokes every API token issued to the account, and places it back into the must-change-password state; refused outright for the environment-named account, naming the environment setting; when the target account is disabled the reset succeeds but the account MUST remain disabled and unable to authenticate (FR-553, FR-554, FR-555, FR-556, FR-557, FR-558, FR-559, FR-573)
- [ ] T043 [US1] Add the `ResetPassword` daemon request variant in `crates/cairn-core/src/wire.rs` and its handler in `crates/cairnd/src/handlers.rs` (FR-553)
- [ ] T044 [US1] Add `cairn user reset-password` to `crates/cairn/src/main.rs` (FR-553)
- [ ] T045 [US1] Test `tests/tests/admin_lifecycle.rs`: two concurrent demotions of the two remaining administrators, issued simultaneously under real concurrency against a real, live database — not by reasoning about isolation levels — result in exactly one success and one refusal (FR-413, FR-560, FR-574, SC-444)
- [ ] T046 [US1] Add `cairn user create|list|disable|enable|promote|demote` to `crates/cairn/src/main.rs` (FR-401, FR-402, FR-411)
- [ ] T047 [US1] Add the password-change path to `cairn auth` in `crates/cairn/src/main.rs` (FR-405)
- [ ] T048 [US1] Add the corresponding daemon request variants in `crates/cairn-core/src/wire.rs` and handlers in `crates/cairnd/src/handlers.rs` for admin user create, list and patch
- [ ] T049 [US1] Test `tests/tests/admin_lifecycle.rs`: temporary password issued once, token minting refused before change, succeeds after (SC-401, SC-402, SC-403)
- [ ] T050 [US1] Test `tests/tests/admin_lifecycle.rs`: a disabled account cannot authenticate with its still-valid password, and its previously issued tokens are refused — asserted as two separate assertions so a regression in either cannot hide behind the other (SC-404, SC-436)
- [ ] T051 [US1] Test `tests/tests/admin_lifecycle.rs`: demoting the last non-environment admin is refused at runtime, and the break-glass restart restores administration (SC-433, SC-434, SC-437)
- [ ] T052 [US1] Test `tests/tests/admin_lifecycle.rs`: an admin resets a member's password — the old password immediately fails, the new temporary password authenticates only to the password-change route, and every token the member held is refused (SC-442)
- [ ] T053 [US1] Test `tests/tests/admin_lifecycle.rs`: resetting a disabled account's password leaves it disabled — authentication with the new temporary password is refused (FR-558, SC-443)
- [ ] T054 [US1] Test `tests/tests/admin_lifecycle.rs`: a request bearing a token past its `expires_at` is refused, and the refusal is identical in status and body to a revoked token's, so an attacker cannot distinguish expiry from revocation by probing (FR-585, SC-452)
- [ ] T055 [US1] Test `tests/tests/admin_lifecycle.rs`: role backfill across seeded configurations — environment-named account present, environment-named account absent, one legacy account, several legacy accounts — always ends with the documented administrator and never zero admins (FR-414, SC-405)
- [ ] T056 [US1] Document the trust statement — whoever sets the environment and restarts can always obtain admin — in `README.md` and `SECURITY.md` (FR-543, SC-435)

**Checkpoint**: accounts are administered, self-registration is gone, resets and role changes are atomic and cannot lock an operator out.

---

## Phase 4: US2 — Explicit membership and safe auto-link (Priority: P2)

**Goal**: a teammate who clones a repository can link it only because someone granted them access.

- [ ] T057 [US2] Implement `POST` and `DELETE /api/projects/{id}/members`, recording `added_by_user_id`; no route allows a user to add themselves (FR-418, FR-419, FR-420)
- [ ] T058 [US2] Implement `GET /api/projects/{id}/members` (admin-facing full list) and `GET /api/projects` scoped to the caller's own memberships (FR-423, FR-427)
- [ ] T059 [US2] Audit every project-scoped route in `crates/cairn-server/src/api.rs` and `crates/cairn-server/src/sync.rs` for the `require_member` predicate, and add it where absent — the known gap is `lookup_projects` (`api.rs`), whose doc comment already claims a membership filter the SQL does not have. Scope is bounded to routes that exist at the time T001 records the prerequisite commit; a route added later is that task's responsibility, not this one's (FR-422)
- [ ] T060 [US2] Add a `project_id` predicate to every sync upsert and every tombstone in `crates/cairn-server/src/sync.rs`, so removal takes effect immediately — this is defense in depth over the prerequisite patch, not a duplicate of it (FR-421)
- [ ] T061 [US2] Implement safe auto-link in `crates/cairnd/src/sync.rs`: `cairn link` with no `--project` auto-selects only when lookup returns exactly one project the caller is already a member of, and continues to accept an explicit project argument regardless of whether auto-link would apply (FR-424, FR-425, FR-428)
- [ ] T062 [US2] Add the membership join to `lookup_projects` in `crates/cairn-server/src/api.rs` so discovery returns nothing to a non-member and no code path can treat discovery as a grant. If the prerequisite patch already landed this, verify it rather than reapplying it and record that in the implementation log — T001 is what establishes which (FR-426)
- [ ] T063 [US2] Add `cairn project member add|remove|list` to `crates/cairn/src/main.rs`
- [ ] T064 [US2] Test `tests/tests/authorization_audit.rs`: membership is the only route to project access — enumerate every project-scoped endpoint and assert a non-member is refused by each (SC-406)
- [ ] T065 [US2] Test `tests/tests/authorization_audit.rs`: discovery by a non-member returns an empty result, and auto-link declines rather than joining (SC-407)
- [ ] T066 [US2] Test `tests/tests/authorization_audit.rs`: a user removed from a project's membership loses read and sync access within the very next request (FR-421, SC-408)
- [ ] T067 [US2] Test `tests/tests/authorization_audit.rs`: no authenticated route adds the caller to a project's membership — enumerate every route the server exposes and assert the caller's own membership set never changes as a side effect of calling it, so a route added later that grants self-membership fails this test (FR-418, SC-465)

**Checkpoint**: a cloned repository links safely for an authorized teammate, and for nobody else.

---

## Phase 5: US3 — Personal global memory, local only (Priority: P3)

**Goal**: knowledge recorded in one project is recalled in another, on one machine, entirely offline, and the shared content validator guards both ways a personal record can be created.

- [ ] T068 [US3] Implement personal read and write in `crates/cairn-store/src/global.rs`, records immutable after creation, with no project identity of any kind (FR-431, FR-432, FR-517, FR-440)
- [ ] T069 [US3] Implement the tombstone as the only permitted content mutation — forgetting (FR-441)
- [ ] T070 [US3] Wire `personal_fts` maintenance through the same trigger pattern as `memory_fts` (FR-442, FR-444)
- [ ] T071 [US3] Implement `crates/cairn-store/src/traits.rs`: derive `project_traits` deterministically from manifests and lockfiles at link and refresh time, never synchronized, drawing only from the `language | tool` vocabulary (FR-437, FR-438, FR-439, FR-569)
- [ ] T072 [P] [US3] Test `tests/tests/privacy_payloads.rs`: a project's derived traits appear in no transmitted payload and no server table, inspected across a corpus of projects whose traits are all distinct, so a trait becoming synchronized later fails the test (FR-438, SC-469)
- [ ] T073 [US3] Apply the applicability predicate at query time in `crates/cairn-store/src/global.rs` (FR-434, FR-435, FR-436)
- [ ] T074 [US3] Wire `validate_global_content` into direct personal creation — the first of its five entry points, passing the identities of the project currently being worked in, if any, as `project_identities` — in `crates/cairn-store/src/global.rs`; a rejected creation persists no record and enqueues no outbox entry (FR-545, FR-548, FR-580)
- [ ] T075 [US3] Implement promotion orchestration in `crates/cairnd/src/promote.rs`, calling both the pure promotion gate and `validate_global_content` — the second entry point — persisting only when both pass, and reporting a refusal synchronously in the same response (FR-506, FR-520, FR-545, FR-548)
- [ ] T076 [US3] Confirm `crates/cairnd/src/promote.rs` writes no verification field of any kind onto a promoted record, regardless of the source's verification state — there is no field to reset; the gate's `verification_reset` check exists only to make that absence explicit at the point of promotion (FR-513)
- [ ] T077 [US3] Reuse `classify_proposal` at write time for the personal domain (FR-442)
- [ ] T078 [US3] Reuse `derive_subject` at read time for the personal domain, over that domain's relations only (FR-442, FR-493)
- [ ] T079 [US3] Extend `cairn_remember` `action: "create"` with `domain`, accepting `project` and `personal` only — still six tools (FR-431, FR-517, FR-527)
- [ ] T080 [US3] Extend `cairn_remember` `action: "promote"` with `target` and `applicability`, defaulting `target` to `pattern` so today's behavior is unchanged; an applicability value outside the closed vocabulary is refused rather than silently dropped (FR-434, FR-514)
- [ ] T081 [US3] Extend `cairn_remember` `action: "forget"` with `domain` (FR-441)
- [ ] T082 [US3] Add `cairn personal list|forget` and `cairn traits` to `crates/cairn/src/main.rs` (FR-439, FR-444)
- [ ] T083 [US3] Test `tests/tests/domain_isolation.rs`: a personal record has no column and no serialized field for a project identifier, an evidence reference, an observation identifier, a file path, a command, or verification of any kind — not an authority, not a state, not a timestamp (FR-517, SC-424)
- [ ] T084 [US3] Test `tests/tests/domain_isolation.rs`: a personal note recorded in project A is retrievable in project B for the same user, and invisible to a different user in either project — privacy holds across both the project axis and the user axis (SC-409)
- [ ] T085 [US3] Test `tests/tests/promotion_gate.rs`: content carrying an absolute path is refused on **direct personal creation**, not only on promotion — the validator cannot be bypassed by the non-promotion path (FR-546, FR-548, FR-545)
- [ ] T086 [US3] Test `tests/tests/promotion_gate.rs`: a promotion whose content contains an absolute path is refused, naming the failed check, and leaves no record, no partial record and no outbox entry behind (SC-440)
- [ ] T087 [US3] Test `tests/tests/promotion_gate.rs`: drive a corpus covering every class `validate_global_content` declares (absolute paths, home-directory references, drive-letter paths, `file://` references, credentialed URLs, environment assignments, secret-shaped runs, project-identifying tokens, shell command invocations) through promotion; each is refused, names the class it failed on, and creates no partial record; derive the corpus from the validator's declared class list so a class added later without a corpus entry leaves this test unable to pass (SC-421)
- [ ] T088 [US3] Test `tests/tests/promotion_gate.rs`: the stored and serialized forms of a promoted record, inspected field by field on the wire and in the local store, contain no verification field of any kind, regardless of the source's verification state — a test that adds one fails (SC-422)
- [ ] T089 [US3] Test `tests/tests/domain_isolation.rs`: applicability filtering is deterministic — the same record and traits produce the same verdict across runs and machines (SC-410)
- [ ] T090 [US3] Test `tests/tests/domain_isolation.rs`: an applicability value outside the closed vocabulary on **direct personal creation** is refused rather than stored with the value silently dropped — not only on promotion (FR-446)
- [ ] T091 [US3] Test `tests/tests/domain_isolation.rs`: a project from which no traits can be derived admits only universal personal records — any record restricted to `language` or `tool` never matches (FR-437, SC-429)

**Checkpoint**: personal knowledge crosses projects on one machine, offline, cannot name the project it came from, and neither of its two entry points can bypass the content validator.

---

## Phase 6: US4 — Personal global memory across devices (Priority: P4)

**Goal**: a second device receives it, an offline divergence resolves into a standing conflict rather than a silent winner, and a blocked namespace recovers on its own once the server catches up.

- [ ] T092 [US4] Implement `crates/cairn-store/src/cursor.rs` over `sync_cursor`, one cursor per namespace (FR-486, FR-487)
- [ ] T093 [US4] Replace the process-global backoff in `crates/cairnd/src/sync.rs` with per-namespace backoff and drain state, so a failure or capability block in one namespace never delays another (FR-488, FR-497, FR-563)
- [ ] T094 [US4] Fix the conditional pull at `crates/cairnd/src/sync.rs`: schedule a per-namespace pull independent of outbox pending, so a consume-only machine still receives. Add `PULL_INTERVAL_SECONDS = 30` as a named constant and gate the pull on it — `WORKER_TICK = 500ms` stays the *check* cadence, not the pull cadence; without a pull interval of its own, "the pull-due timer has elapsed" is true every tick and each namespace polls twice a second forever (FR-489, FR-589)
- [ ] T095 [US4] Generate and persist the writer identity once per store in `crates/cairn-store/src/repo.rs` (FR-490)
- [ ] T096 [US4] Mix the writer identity into the idempotency key input for newly enqueued rows only, leaving in-flight rows on the old scheme (FR-491, Complexity Tracking item 4)
- [ ] T097 [US4] Implement `writer_seq` as within-writer gap detection and dedup only, never a cross-writer tiebreak (FR-445, FR-492)
- [ ] T098 [US4] Assert `MemoryFacts` in `crates/cairn-core/src/knowledge.rs` — the sole input `derive_subject`'s reconciliation reads — has no `writer_seq` field, so a tiebreak or ordering rule that consulted it would not compile; this is a structural assertion, not a runtime one (FR-583, SC-455)
- [ ] T099 [US4] Test `crates/cairn-core/src/knowledge.rs` (or `tests/tests/multi_device_convergence.rs`): replay one corpus of personal-knowledge proposals under reordered, withheld and renumbered writer sequences and assert `derive_subject` produces identical canonical output every time, pairing the behavioral guarantee with T098's structural one (FR-583, SC-455)
- [ ] T100 [US4] Route personal-domain writes to the `personal:<server_instance>:<user>` namespace in `crates/cairn-store/src/outbox.rs` (FR-486, FR-568)
- [ ] T101 [US4] Implement personal ingest and read-back in `crates/cairn-server/src/global.rs`, carrying `writer_id` and `writer_seq` on the wire and in the Postgres row in both directions (FR-489, FR-582)
- [ ] T102 [US4] Wire `validate_global_content` into server-side personal-knowledge ingest — the fifth entry point — in `crates/cairn-server/src/global.rs`, screening `content`, `topic_key`, `value_key` and applicability values against the union of the pushing user's project memberships as `project_identities`; the refusal is permanent, reports a rejection class only, does not enter the `blocked` state, and does not throttle the namespace; represent it as a distinct refusal type/variant a client can match on, so it is distinguishable from a capability refusal without inspecting a message string (FR-545, FR-577, FR-581)
- [ ] T103 [US4] Add `personal_knowledge` to `ENTITY_CAPABILITIES` and to the capability advertisement, so an older server causes only this entity type to be held, never project sync (FR-498, FR-522)
- [ ] T104 [US4] Handle `sync_deferred` for the new entity types, holding a record whose dependency has not yet arrived and retrying it oldest-first (FR-499, FR-501)
- [ ] T105 [US4] Key the personal synchronization namespace by both the server instance and the owning account, so two identities of the same human never merge (FR-568)
- [ ] T106 [US4] Implement the capability re-probe cycle: while a namespace is blocked for want of a server capability, re-read the server's advertised capabilities on a bounded, backed-off schedule; the probe is a capability read, never a retry of the held items (FR-561)
- [ ] T107 [US4] On a re-probe observing the required capability, return the namespace to eligible and release the held entries for delivery preserving their original idempotency keys, so a partially delivered entry applies exactly once; the transition requires no local write, no user command and no daemon restart (FR-562, FR-563)
- [ ] T108 [US4] Release claimed-but-unfinished synchronization work per namespace at daemon start, independently across all three namespaces, so an interrupted namespace resumes without waiting on any other (FR-502, FR-562)
- [ ] T109 [US4] Extend `cairn sync status` to report per namespace in `crates/cairn/src/main.rs` (FR-487)
- [ ] T110 [US4] Test `tests/tests/multi_device_convergence.rs`: two devices, both offline, record contradicting personal knowledge; on reconnection the result is a standing conflict, not a silent winner, independent of sync order or clock skew (FR-493, SC-411)
- [ ] T111 [US4] Test `tests/tests/multi_device_convergence.rs`: two devices emitting a byte-identical payload both land, rather than one being discarded as a duplicate of the other's write (SC-427)
- [ ] T112 [US4] Test `tests/tests/namespace_sync.rs`: a device with nothing queued to push receives another device's synchronized personal knowledge within 60 seconds — twice the 30-second `PULL_INTERVAL_SECONDS` (FR-589), a number the test can actually assert against — without pushing anything first (FR-489, SC-412)
- [ ] T113 [US4] Test `tests/tests/namespace_sync.rs`: on daemon start, a namespace whose claim was interrupted releases and resumes without waiting on any other namespace (FR-502, FR-562)
- [ ] T114 [US4] Test `tests/tests/namespace_sync.rs`: a personal record created on one device pulls into a second device's store with its `writer_id` and `writer_seq` intact, and a deliberately withheld middle record in that writer's sequence is reported as a detected gap rather than silently ignored (FR-492, FR-582, SC-450)

**Checkpoint**: personal knowledge is a multi-device store with no clock in its correctness argument, and a blocked namespace recovers by itself once the peer can accept it.

---

## Phase 7: US5 — Team global memory (Priority: P5)

**Goal**: any member proposes; only an administrator makes it authoritative; the content validator — now screening for project-identifying and command-shaped content from the start rather than only at the old gate check — is wired at both team entry points; and, with team ingest validation added here, all five of the feature's content-creating entry points are complete.

- [ ] T115 [US5] Implement team read and write in `crates/cairn-store/src/global.rs`, with `team_fts` maintenance and visibility to every user regardless of project membership once authoritative (FR-451, FR-458, FR-459)
- [ ] T116 [US5] Rewrite in `crates/cairn-store/src/global.rs`: refuse team knowledge sourced from a different `server_instance_id`; personal knowledge MUST NOT be refused on that basis — retain and partition it by owning identity, so a local store can hold the personal knowledge of more than one identity (FR-495, FR-496, FR-567)
- [ ] T117 [US5] Test `tests/tests/namespace_sync.rs`: a local store linked in turn to two different server instances retains both identities' personal knowledge and returns only the currently linked identity's entries from search, context and listing (FR-567, SC-447)
- [ ] T118 [US5] Test `tests/tests/namespace_sync.rs`: a local store that has recorded one server instance's identity refuses to merge a second instance's *team* knowledge into itself, while a second identity's *personal* knowledge on the same store is retained and partitioned rather than refused (FR-496, FR-567, SC-428)
- [ ] T119 [US5] Implement the `proposed → authoritative → retired` lifecycle with compare-and-swap on `expected_state`, reusing the revision-guard pattern in `crates/cairn-store/src/criteria.rs`, recording who acted and when — **both** transitions: ratification already carried `ratified_by_user_id`/`ratified_at`, and retirement gains `retired_by_user_id` alongside `retired_at`, because a timestamp on its own does not record who acted and retirement is the transition most worth attributing (FR-453, FR-454, FR-457, FR-461)
- [ ] T120 [US5] Make `proposed` invisible to search, context and every recall path, including the proposer's own (FR-452)
- [ ] T121 [US5] Restrict ratification and retirement to administrators; a request whose expected state no longer matches the entry's actual current state MUST be refused, naming that state (FR-453, FR-454, FR-456)
- [ ] T122 [US5] Refuse re-ratification of a retired entry; guidance restored after retirement MUST be recorded as a new proposal (FR-465)
- [ ] T123 [US5] Wire `validate_global_content` into team proposal creation — the third of five entry points, passing the proposer's current project's identities as `project_identities` — in `crates/cairn-store/src/global.rs`; a rejected proposal persists no record and enqueues no outbox entry (FR-545, FR-548, FR-580)
- [ ] T124 [US5] Route team promotion through `cairn_remember` `promote` with `target: "team"` — the fourth of five entry points — calling both the promotion gate and `validate_global_content`, always landing `proposed`, never authoritative, and refusing unless the promoter is a member of the source project (FR-515, FR-520, FR-545, FR-548)
- [ ] T125 [US5] Refuse `domain: "team"` on `cairn_remember` `action: "create"`, so no MCP action can author team knowledge directly — still six tools (FR-455, FR-527)
- [ ] T126 [US5] Ensure no MCP action can ratify — ratification exists only in the CLI and the server API (FR-455)
- [ ] T127 [US5] Reuse `classify_proposal` and `derive_subject` for the team domain, with `Supersedes` written by the ratifying admin rather than inferred, so two disagreeing authoritative entries both remain visible (FR-462)
- [ ] T128 [US5] Apply the applicability predicate to team knowledge under the same closed vocabulary and matching rule as personal (FR-460)
- [ ] T129 [US5] Implement team ingest and read-back in `crates/cairn-server/src/global.rs`, carrying `writer_id` and `writer_seq` on the wire and in the Postgres row in both directions, and add `team_knowledge` to `ENTITY_CAPABILITIES` (FR-459, FR-498, FR-522, FR-582)
- [ ] T130 [US5] Wire `validate_global_content` into server-side team-knowledge ingest — completing the fifth entry point for both domains — in `crates/cairn-server/src/global.rs`, screening against the union of the pushing user's project memberships as `project_identities`; the refusal is permanent, reports a rejection class only, does not enter the `blocked` state, and does not throttle the namespace; represent it as the same distinct refusal type T102 introduced, so ingest refusal is uniform across domains (FR-545, FR-577, FR-581)
- [ ] T131 [US5] Ensure an authoritative team entry is visible to a user with zero project memberships, because it is a server-wide default rather than something scoped to membership (FR-463)
- [ ] T132 [US5] Implement role-filtered team listing: a member sees authoritative entries and their own proposals; an admin sees every state (FR-464)
- [ ] T133 [US5] Add `cairn team list|propose|ratify|retire` to `crates/cairn/src/main.rs` (FR-451, FR-453, FR-456, FR-464)
- [ ] T134 [US5] Test `tests/tests/domain_isolation.rs`: `cairn_remember create` refuses `domain: "team"`; enumerating every action the six tools expose from the schema (not a hardcoded list), none creates a team entry in the authoritative state and every one it can create is `proposed` — so a new action that could ratify fails this test rather than passing unnoticed (FR-455, SC-460)
- [ ] T135 [US5] Test `tests/tests/domain_isolation.rs`: a proposed entry is absent from search and context for every user including its proposer (SC-413)
- [ ] T136 [US5] Test `tests/tests/domain_isolation.rs`: ratification attempted by a non-admin is refused (SC-414)
- [ ] T137 [US5] Test `tests/tests/domain_isolation.rs`: two concurrent ratifications of the same proposal — the second is refused by compare-and-swap naming the entry's actual current state, rather than silently applied on top of it (SC-415)
- [ ] T138 [US5] Test `tests/tests/domain_isolation.rs`: two contradicting authoritative entries produce a standing conflict, never an automatic resolution by order of ratification — both are returned with the disagreement surfaced regardless of ratification order (FR-462, SC-466)
- [ ] T139 [US5] Extend `tests/tests/clock_swap_invariance.rs` to both new domains, now that the team lifecycle exists — no ordering anywhere compares a timestamp or write order (FR-492, FR-493)
- [ ] T140 [US5] Test `tests/tests/domain_isolation.rs`: an authoritative team entry is visible to every account on its server regardless of project membership, and a retired one is visible to none (SC-416)
- [ ] T141 [US5] Test `tests/tests/domain_isolation.rs`: a retired team record refuses re-ratification; restoring its guidance requires a brand-new proposal (FR-465)
- [ ] T142 [US5] Test `tests/tests/domain_isolation.rs`: role-filtered listing — a member sees authoritative entries and only their own proposals, an admin sees every state (FR-464)
- [ ] T143 [US5] Test `tests/tests/domain_isolation.rs`: team applicability — an entry restricted to one language does not apply to a project lacking that trait, and applies where the project carries it (FR-460)
- [ ] T144 [US5] Test `tests/tests/domain_isolation.rs`: a team record has no column and no serialized field for a project identifier, an evidence reference, an observation identifier, a file path, a command, or verification of any kind — not an authority, not a state, not a timestamp (FR-517, SC-424)
- [ ] T145 [US5] Test `tests/tests/global_content_validation.rs` (new file): content carrying an absolute path is refused identically by all five entry points — direct personal creation, personal promotion, team proposal, team promotion, and server-side synchronization ingest — exercised with the same input (FR-545, SC-438)
- [ ] T146 [US5] Test `tests/tests/global_content_validation.rs`: content naming a project, and separately content carrying a shell command invocation, are each refused identically by all five entry points, exercised with the same inputs; extend the check to a seeded adversarial corpus of project-identifying tokens, file paths and shell commands across every free-text field and every applicability value, so the criterion tests the validator rather than the schema (FR-545, FR-546, SC-438, SC-424a)
- [ ] T147 [US5] Test `tests/tests/global_content_validation.rs`: a client that bypasses its own local validation and pushes personal or team content containing a project-identifying token or a shell command is refused by the server at ingest; the record is absent from the server store; and it never reaches the user's other devices — verified end to end against a real server (FR-577, FR-581, SC-449)
- [ ] T148 [US5] Test `tests/tests/global_content_validation.rs`: an ingest refusal is distinguishable from a capability refusal by the client without inspecting a message string — by matching on T102/T130's distinct refusal type — the refused item is never reported as delivered, and the refused namespace remains eligible and unthrottled, verified by observing continued push throughput of subsequent items (FR-581, SC-456)
- [ ] T149 [US5] Test `tests/tests/global_content_validation.rs`: a rejection message, log line and API response contain no fragment of the rejected content, across all five entry points (FR-547, SC-439)
- [ ] T150 [US5] Test `tests/tests/global_content_validation.rs`: after a rejected creation or promotion at any of the five entry points, no record, no partial record and no outbox entry exists, inspecting all three (FR-548, SC-440)
- [ ] T151 [US5] Test `tests/tests/privacy_promotion.rs`: forgetting or deleting a project memory that was previously promoted to personal or team knowledge leaves the promoted record unaffected — no live reference, no cascading change (FR-519, SC-423)

**Checkpoint**: team guidance flows to everyone only after a human decided it should, and none of the feature's five content-creating entry points — all now wired, client and server alike — can smuggle a path, a project name, a secret or a command past the shared validator.

---

## Phase 8: US6 — Unified bounded recall (Priority: P6)

**Goal**: one answer that keeps domains separate and never lets global displace project truth — proven with real global records in the store, not an empty one.

- [ ] T152 [US6] Add `personal_notes` and `team_guidance` sections to `crates/cairn-core/src/context.rs`, last in the priority order after `patterns`, personal ahead of team (FR-476)
- [ ] T153 [US6] Ensure the global fetch is not called during reserve computation, so no arithmetic path can admit it into Level 0 (FR-473)
- [ ] T154 [US6] Test `tests/tests/global_non_displacement.rs`: where a personal and a team item compete for the same remaining space and only one fits, the personal item is the one included, across a randomized budget matrix (FR-476, SC-462)
- [ ] T155 [US6] Apply the global cap `min(floor(total_budget * 0.15), remaining_non_reserve)` to personal and team sections together in `crates/cairn-core/src/context.rs`, adding the `remaining_non_reserve` accessor to `crates/cairn-core/src/budget.rs` alongside `general_remaining()`. **Both files are required**: `general_remaining()` includes reserve that `release_reserve()` returned, so reusing it is exactly the defect D449 exists to prevent — global sections MUST NOT consume released reserve, and contribute nothing when project sections consume the entire non-reserve pool (FR-474, FR-475, FR-584)
- [ ] T156 [US6] Wire `depth` end to end — add the field to `Request::Context` in `crates/cairn-core/src/wire.rs`, read it in the dispatch at `crates/cairn/src/mcp.rs`, and honor it in `crates/cairnd/src/briefing.rs`. It has never been wired; the schema advertises it and the daemon has never received it (FR-477)
- [ ] T157 [US6] Exclude global sections entirely at `depth: "minimum"`, with no configuration able to override this (FR-477)
- [ ] T158 [US6] Record a selection reason for every included or excluded personal/team item on the diagnostics path only, reusing the mechanism project sections already use; the rendered-briefing type MUST carry no reason field of any kind, so a reason cannot reach the briefing by construction rather than by the renderer choosing to omit it (FR-478)
- [ ] T159 [P] [US6] Test `crates/cairn-core/src/context.rs`: a selection reason for a personal or team item is present in the diagnostic output and absent from the rendered briefing, inspected field by field on the rendered form — a test that adds a reason field to the rendered form fails to compile or fails the inspection (FR-478, SC-463)
- [ ] T160 [US6] Add `personal[]` and `team[]` as sibling arrays to `SearchPayload`, never merged into `results[]`, with `total` still counting project results only (FR-469, FR-470)
- [ ] T161 [US6] Add the `domains` filter to `cairn_search` in `crates/cairn/src/mcp.rs` (FR-472)
- [ ] T162 [US6] Rank each domain within itself by BM25 over its own FTS table, with no cross-domain comparator; give each domain's ranking input a distinct type carrying no other domain's score, so a cross-domain comparison could not compile (FR-471)
- [ ] T163 [US6] Assert in `crates/cairn-core/src/context.rs`, where T162 declares the per-domain ranking types: each domain's ranking type carries no other domain's score field — a structural, compile-time assertion, not a runtime test. Demonstrate it by adding a second domain's score to one type in a scratch build and confirming the build fails (FR-471, SC-468)
- [ ] T164 [US6] Ensure a caller with no personal or team knowledge of their own sees zero difference in project search or context relative to a caller who never touches either domain (FR-481)
- [ ] T165 [US6] Ensure an importance hint on a personal or team item does not change its section's precedence and does not admit it into reserved context (FR-482)
- [ ] T166 [US6] Test `tests/tests/global_non_displacement.rs`: an importance hint of every supported value on a personal or team item leaves the assembled context byte-identical across all hint values (FR-482, SC-464)
- [ ] T167 [US6] Ensure the estimated size of an assembled context never exceeds the requested budget, including when personal and team sections are present (FR-480)
- [ ] T168 [US6] Rewrite `tests/tests/global_non_displacement.rs`: seed real, highly-rankable personal and team records before filling a project's context to its full budget; assert the assembled briefing is still byte-identical to the project-only baseline — the previous version of this test could pass against an empty global store, so this replaces it (FR-475, SC-418)
- [ ] T169 [US6] Test `tests/tests/global_non_displacement.rs`: with headroom in the budget, personal and team sections appear after every project section, bounded to the documented `min(floor(total_budget * 0.15), remaining_non_reserve)` share, and `estimated_tokens <= budget` holds across the budget matrix, re-verified now that D449's non-reserve restriction and D450's cap constant are both in place (FR-474, FR-480, FR-584, SC-419)
- [ ] T170 [US6] Test `tests/tests/global_non_displacement.rs`: with a large unspent Level 0 reserve released to the general pool and global records available to fill it, the global sections consume none of the released reserve — assert global spend against the non-reserve pool alone, so the test fails if the implementation ever spends released reserve (FR-584, SC-451)
- [ ] T171 [US6] Test `tests/tests/global_non_displacement.rs`: a context request at `depth: "minimum"` contains zero personal or team content regardless of available budget — and observably differs from standard depth now that T156 wires the field (SC-420)
- [ ] T172 [US6] Test `tests/tests/domain_isolation.rs`: search returns project, personal and team results in three distinct arrays, and the project result count is identical whether or not personal or team results exist (FR-469, FR-470, SC-417)
- [ ] T173 [US6] Test `tests/tests/global_non_displacement.rs`: across a seeded workload exercising recall, search, context assembly and synchronization with no explicit promotion or creation request issued, the personal and team record counts are unchanged — no implicit path creates global content (FR-506, SC-461)

**Checkpoint**: three domains, one bounded briefing, project truth never displaced — proven against a global store that actually holds content.

---

## Phase 9: Repairs

**Purpose**: shipped defects this feature's guarantees depend on, plus the documentation corrections the repair addendum requires. None of these are new capability.

- [ ] T174 Repair `tests/tests/scope_audit.rs`: it splits on `"fn scope_bucket"`, which does not exist, so `unwrap_or_default()` yields `""` and all four assertions pass vacuously. Rewrite so a missing target **fails** (FR-533)
- [ ] T175 Make the wire privacy check recurse in `crates/cairn-server/src/sync.rs`, and correct its doc comment, which calls a top-level denylist "the allowlist enforced on the wire" (FR-532, FR-535)
- [ ] T176 Emit repository-relative paths only in `changed_files` and in the `completed_work` prose built from them, in `crates/cairn-core/src/handoff.rs` (FR-531, SC-431)
- [ ] T177 Test `tests/tests/privacy_payloads.rs`: a generated handoff payload carries zero absolute filesystem paths in `changed_files`, in the `completed_work` prose built from them, or anywhere else in the transmitted object — assert against a repository whose working tree contains files with no Git counterpart, which is the case the existing suffix-dedup does not collapse (FR-531, SC-431)
- [ ] T178 Drop the command string from `tests_executed`, keeping name and outcome, in `crates/cairn-core/src/handoff.rs` (FR-532)
- [ ] T179 Remove the stale `REJECTED_OBSERVATION_FIELDS` duplicate in `crates/cairn-core/src/wire.rs` — 7 names against the server's live 27, presented as the same boundary (FR-534)
- [ ] T180 Test `tests/tests/privacy_payloads.rs`: no absolute path, observation summary or command string crosses the wire, asserted against a crafted nested payload that the old top-level check would have passed (FR-535)
- [ ] T181 [P] Correct `specs/003-project-intelligence/contracts/privacy-sync.md`: the field is `capabilities` with five names, not `capability` with three; the wire check is a denylist not an allowlist; the forbidden-field and entity-type counts are wrong (FR-534)
- [ ] T182 [P] Correct `specs/003-project-intelligence/contracts/knowledge.md`: the `duplicates` direction is backwards, the claimed recursion does not exist, one-member gives `Reinforced` not `Settled`, a cycle gives `Conflicted` not `Historical`, and there are seven verification tiers not six
- [ ] T183 [P] Correct `specs/004-collaborative-global-memory/contracts/promotion-privacy.md` and `data-model.md`: for every privacy guarantee, state whether it holds because no column exists to carry the value or because free text is validated by `validate_global_content`; a free-text field MUST NOT be described as structurally incapable of carrying a path or a command (FR-550)
- [ ] T184 Add a documentation-lint test/script over `specs/004-collaborative-global-memory/contracts/promotion-privacy.md`, `data-model.md`, `compatibility.md` and `contracts/global-memory.md`: fails when "structurally incapable", "impossible by construction" or "no column exists" (or an equivalent claim) is applied to a free-text field — `content`, `topic_key`, `value_key` or an applicability value — so T183's correction cannot silently regress; additionally fails when a document calls an applicability fact a "topic" or conflates it with a record's own `topic_key` (FR-570), and when `README.md`/`SECURITY.md` do not contain the environment-account trust statement (FR-543). This is a lint, not a human review; it must be seen to fail against each phrase deliberately reinserted (FR-550, FR-570, FR-543, SC-467)
- [ ] T185 [P] Correct `specs/004-collaborative-global-memory/contracts/promotion-privacy.md` and `data-model.md` to state that origin-digest recognition is per-machine only, and that two devices of the same user will not correlate promotions from the same project — an accepted limitation of keeping the digest off the wire (FR-552)
- [ ] T186 [P] Correct `specs/004-collaborative-global-memory/data-model.md` and `contracts/global-memory.md` to distinguish a record's own `topic_key` from an applicability fact, since both were previously called "topic" (FR-570)

**Checkpoint**: the privacy boundary this feature documents is the one the code actually enforces, and every guarantee names the mechanism it actually rests on.

---

## Phase 10: Polish and release evidence

- [ ] T187 Test `tests/tests/namespace_sync.rs`: against a real schema-2 server, personal and team entries go `blocked` while project sync keeps draining at full speed, and the degradation is reported by name (FR-522, SC-425)
- [ ] T188 Extend `tests/tests/mcp_backward_compatibility.rs`: still exactly six tools, the forbidden seventh-tool names still absent, and every pre-004 field still behaves identically (FR-527, SC-430)
- [ ] T189 Extend `tests/tests/rebuild_equivalence.rs` to per-domain derivation (FR-442)
- [ ] T190 Test `tests/tests/capability_upgrade_e2e.rs` (new file) — the end-to-end capability-upgrade path: queue personal and team content locally against a server advertising only schema 2; connect and observe both namespaces held `blocked` while project sync continues at full speed; replace the peer, at the same configured endpoint, with a supporting schema-3 server; perform no new local write and no daemon restart; observe the client's capability re-probe automatically detect the upgrade and return both namespaces to eligible; confirm the held content on each namespace delivers exactly once (FR-500, FR-561, FR-562, FR-563, SC-426, SC-445)
- [ ] T191 Implement stable, documented refusal responses for the removed `POST /api/auth/register` and `POST /api/projects/{id}/join` routes in `crates/cairn-server/src/api.rs`, each naming its replacement (`POST /api/admin/users`, `POST /api/projects/{id}/members`) rather than a bare not-found (FR-587)
- [ ] T192 Test `tests/tests/compat_old_client.rs` (new file): build a real pre-004 client binary and run it through a full project synchronization cycle — push, pull, cursor advance — against a 004 server; assert no namespace blocked and no throughput loss (FR-586, SC-457)
- [ ] T193 Test `tests/tests/compat_old_client.rs`: `POST /api/auth/register` and `POST /api/projects/{id}/join` each return the documented status and a body naming their replacement, and the shipped release documentation states all three of — self-registration gone, self-join gone, accounts now administrator-created — plus the operator remedy (FR-587, FR-588, SC-458)
- [ ] T194 Execute `quickstart.md` end to end on a real repository, with two machines and a real server
- [ ] T195 Produce `specs/004-collaborative-global-memory/release-evidence.md` in the style of 003's
- [ ] T196 Update `skills/cairn/SKILL.md` and `references/` for the new actions and domains
- [ ] T197 Update `README.md` for administered accounts, membership and the two new domains, stating in operator-facing terms that self-registration and self-join are gone, that accounts are now administrator-created, and what an operator must do for users who relied on either (FR-588)
- [ ] T198 Regenerate `specs/004-collaborative-global-memory/traceability.md` from the final `spec.md` by enumeration (D443, D457), and resolve the open items in `checklists/requirements.md`
- [ ] T199 Test `tests/tests/admin_lifecycle.rs`: assert the shipped `README.md` and `SECURITY.md` state who can ultimately obtain administrator access and why — the environment-named account, restored to admin and active on every start, reachable by whoever controls the host — by asserting the statement's presence and that it names both the mechanism and the reason. SC-435 says a reader can answer the question "without reading the source"; a documentation task that merely writes the sentence cannot verify that, and this was the one success criterion in the feature with no verifying task at all (FR-543, SC-435, D457)
- [ ] T200 Test `tests/tests/admin_lifecycle.rs`: a token issued before an account was disabled is still refused after the account is re-enabled — asserted separately from the disable-time revocation of T050, so a regression that clears `revoked_at` alongside `status` cannot be masked by the disable test passing. This is a credential-lifetime rule that lived only in a contract paragraph until now (FR-590, SC-470)

**Checkpoint**: the feature is demonstrated on a real repository, not only asserted in CI.

---

## Completion status

**198 of 200 complete. T104 and T194 incomplete, both documented in
`release-evidence.md`.**

The checkboxes above are not the record — they were never maintained, and a
checkbox is exactly the kind of evidence an adversarial review is right to
discard. The record is `implementation-log.md` for what was built and why, and
`release-evidence.md` for what was verified.

### Tasks reopened by the adversarial review, and the production path each has now

A task whose requirement is runtime behaviour is complete only when a production
path reaches it. Five were credited to code nothing called, and were reopened and
repaired:

| Task | What it was credited to | Production path now |
|---|---|---|
| T071 | `refresh_traits`, zero callers | `Daemon::project_traits` (`cairnd/src/state.rs`), called by the briefing, by `cairn_search`, and by `cairn traits` |
| T082 | `cairn personal list` via `recall_personal(&[])`, which hides every restricted record | `cairn_store::global::list_personal`, no applicability predicate |
| T078 | `personal_subject`, zero callers | `cairn memory subject --domain personal` → `handlers::global_subject` |
| T127 | `team_subject`, zero callers | `cairn memory subject --domain team` → `handlers::global_subject` |
| T115, T128, T131 | `recall_team`, zero callers | `briefing::global_candidates`, mirroring `recall_personal` |

### Tasks whose evidence was strengthened rather than reopened

| Task | Change |
|---|---|
| T072 | the privacy assertion now runs against traits derived through the real lifecycle and asks a real server what it holds, rather than screening a hand-built corpus that held vacuously |
| T192 | the pre-004 client is extracted with `git archive` instead of `git worktree add`, so the harness no longer mutates the repository's shared `.git/worktrees/` |
| T119, T127 | ratification now writes `superseded_by_id` alongside the relation, so an administrator's `--supersedes` reaches every reader and every device |

### Still incomplete

- **T104** — global relation transport. Blockers and their real consequence are
  stated in `release-evidence.md`. Does not block release; the reasoning is
  specific to which decisions cross the wire and which are re-derived.
- **T194** — the physical two-machine quickstart walkthrough. Its software
  substance runs in `namespace_sync.rs` and `capability_upgrade_e2e.rs`. No human
  has executed it on two machines, and an automated multi-device test is not that.

## Dependencies

- **T001 blocks everything.** The prerequisite patch is not optional.
- **Phase 2 blocks Phases 3–8.** Types, migrations, the shared content validator and the promotion gate are used by every story.
- **Phase 3 blocks Phase 4** — membership needs roles and accounts to exist.
- **Phase 4 blocks T102 and T130** — server-side ingest validation screens against the union of the pushing user's project memberships, which does not exist before the membership model does.
- **Phase 5 blocks Phase 6** — there is nothing to synchronize until the local domain works.
- **Phase 6 blocks Phase 7** — team knowledge reuses the namespace machinery, the backoff and the capability re-probe cycle personal sync builds and proves. Team is second on purpose: its lifecycle is the more complex of the two, and it inherits a proven transport.
- **Phases 5, 6 and 7 block Phase 8** — unified recall needs all three domains present to be tested honestly.
- **T156 blocks T157 and T171** — `depth` must exist before it can be honored or asserted.
- **T074 and T075 (Phase 5), T102 (Phase 6), and T123, T124 and T130 (Phase 7) block T145–T150** — the cross-entry-point validator tests need all five entry points to exist, including server-side ingest.
- **T106–T108 (Phase 6) block T190** — the E2E capability-upgrade test exercises the re-probe cycle and the per-namespace claim release both must already work.
- **T012 (the nine-class validator) blocks T087, T146 and T015** — each drives its corpus or audit from the validator's declared class list, not a hardcoded one.
- **Phase 9 is independent of Phases 3–8** and may proceed at any time after Phase 1, except T180, which needs T175–T178, and T184, which needs T183.

## Parallel opportunities

- Phase 9 (Repairs) runs alongside Phases 3–8 entirely, and is a good place for a second implementer.
- Within Phase 2, T007–T011 (core types) and T022–T027 (migrations) are separate files and proceed together once the negative tests T004–T006 exist.
- Phases 3 and 4 are sequential with each other but both are independent of Phase 9.
- T181–T186 and T184 (documentation corrections and the doc-lint check) are independent of all code work and of each other.

## Task count

200 tasks: Setup 3, Foundational 26, US1 27, US2 11, US3 24, US4 23, US5 37, US6 22, Repairs 13, Polish 14.

Of these, 20 are marked `[P]`.

The `[P]` count is lower than it first looks, and deliberately so. `[P]` was withdrawn from
eleven tasks that had carried it while writing to a file another `[P]` task also writes:
`crates/cairn/src/main.rs` (T044, T046, T047, T063, T082, T109, T133 — every CLI surface this
feature adds lands in one file, which makes that file the feature's main serialization point),
`tests/tests/domain_isolation.rs` (T005), `crates/cairn-core/src/validate.rs` (T015),
`crates/cairn-core/src/context.rs` (T163), and the two design documents T183 and T184 both
edit. A `[P]` marker that two implementers act on simultaneously against one file produces a
merge conflict, not parallelism, so the marker was wrong rather than optimistic.

`crates/cairn/src/main.rs` deserves the explicit note: T044, T046 and T047 sit in the same
phase, and T063, T082, T109 and T133 are spread across Phases 4–7 while Phase 9 runs alongside
all of them. Whoever schedules this work should treat every `main.rs` task as sequential with
every other one regardless of phase.

## The twenty-six negative tests

| # | Negative requirement | Task |
|---|---|---|
| 1 | `MemoryScope` unchanged, `memories` not rebuilt | T004 |
| 2 | Global never in the Level 0 reserve, with real global records present | T168 |
| 3 | No relation links two domains | T005 |
| 4 | No ordering compares a timestamp | T139 |
| 5 | No MCP path authors authoritative team policy | T134 |
| 6 | No path, summary or command on the wire | T180 |
| 7 | Old server does not stall project sync | T187 |
| 8 | Exactly six MCP tools, no seventh | T188 |
| 9 | Applicability outside the vocabulary is rejected | T006 |
| 10 | `scope_audit.rs` can actually fail | T174 |
| 11 | Interrupted migration leaves the old version | T028 |
| 12 | Offline divergence yields a standing conflict | T110 |
| 13 | The content validator cannot be bypassed by any of the five entry points | T145 |
| 14 | The origin digest never appears on the wire | T021 |
| 15 | A password reset does not re-enable a disabled account | T053 |
| 16 | Two concurrent demotions cannot both succeed | T045 |
| 17 | A promoted record carries no verification field of any kind | T088 |
| 18 | A client that bypasses its own validation is still refused at server-side ingest | T147 |
| 19 | Released Level 0 reserve is never spent by a global section | T170 |
| 20 | An expired token's refusal is indistinguishable from a revoked token's | T054 |
| 21 | The content validator is the only implementation of its nine classes | T015 |
| 22 | No route lets a caller add themselves to a project's membership | T067 |
| 23 | No cross-domain relevance comparison compiles | T163 |
| 24 | No free-text field is documented as structurally incapable of holding a path | T184 |
| 25 | Permuting a writer's sequence changes nothing about derived output | T098 |
| 26 | An ingest refusal never blocks or throttles its namespace | T148 |

The last six were added by the semantic traceability pass (D457) rather than by the design, and
they share a property worth naming: each one is an assertion about whether a *guarantee can
still be broken*, not about whether the current code behaves. Numbers 21, 22, 23 and 24 in
particular are audits, and an audit is the easiest kind of test to write vacuously — Feature
003's `scope_audit.rs` split on a function name that did not exist, compared against `""`, and
passed for its entire life (number 10 above, T174). Each of these four must be seen to fail
against a deliberately introduced violation before it is trusted: a second implementation of a
rejection class, a route that self-grants membership, a type carrying two domains' scores, and
the forbidden phrase reinserted into a document.
