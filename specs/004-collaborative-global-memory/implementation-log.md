# Implementation log — Feature 004

Kept because `T001` requires a commit to be recorded, and because a phase
boundary is easier to audit against a written record than against a git log.

**Branch**: `feat/004-collaborative-global-memory` · **Base**: `main` @ `214154f`

---

## Phase 1 — Setup

### T001 — PASS

**Prerequisite commit: `214154f`** — *"fix(server): close five authorization holes
in the sync and link paths"*, fast-forwarded onto `main` from
`fix/authz-prerequisite`.

Verified against the resulting `main`, not inferred from the patch:

| Check | Result |
|---|---|
| `POST /api/auth/register` returns 404 | **404**, unauthenticated request against a live server |
| `POST /api/projects/{id}/join` returns 404 | **404**, authenticated member requesting its own project |
| `lookup` filters by membership | `JOIN project_members m ON m.project_id = p.id … AND m.user_id = $2` |
| `tombstone` carries a `project_id` predicate | present on all five arms; the `project` arm scoped by identity, since a project row has no `project_id` column |

The hotfix additionally covered two defects `T001` does not name but the
prerequisite required: every `ON CONFLICT (id) DO UPDATE` in sync ingest now
carries `WHERE <table>.project_id = $n` and refuses on zero rows, and relations,
criteria and blockers check their referenced rows' project directly, because
`DO NOTHING` and an insert against a foreign key cannot be caught by a row
count.

Each fix was mutation-tested: reverting it makes exactly its own test in
`tests/tests/authorization_prerequisite.rs` fail, and the suite returns green
once restored. That property, rather than the tests' existence, is what makes
them worth having.

### T002 — done

`CHANGELOG.md`, under Unreleased: the hotfix under **Fixed**, and Feature 004's
landed foundation under **Added**.

### T003 — done, already satisfied in CI

`.github/workflows/ci.yml:57` runs `cargo build --workspace` immediately before
`cargo test --workspace --all-targets`, with a comment stating why: `cargo test`
does not build another package's binaries, and the end-to-end suite drives the
real ones.

**For the local loop**, the same rule applies and is easy to get wrong: run
`cargo build` (not `cargo build --tests`) before `cargo test -p cairn-e2e …`.
`--tests` leaves `cairn`, `cairnd` and `cairn-server` stale, and the harness then
exercises yesterday's binaries while reporting on today's source — a green run
that means nothing.

**Baseline before any Feature 004 code**, on `214154f` with
`CAIRN_TEST_DATABASE_URL` set: **1111 passed, 0 failed**, `cargo fmt --check`
clean, `cargo clippy --workspace --all-targets -- -D warnings` clean. Any later
failure is attributable to this feature.

---

## Phase 2 — Foundational

**T004–T029, complete.** 60 tests added; full workspace **1173 passed / 0 failed** at the phase
boundary, fmt and clippy clean.

Three defects were found by the suite rather than by review, and each was invisible from its
result:

1. **`namespace TEXT NOT NULL` with no default** and no enqueue site setting it — every outbox
   insert failed. Surfaced as a convergence test where one machine received nothing.
2. **`current_setting('cairn.admin_email')` was never set.** Migration 3's environment-named
   branch was unreachable, so every migrating deployment fell through to "oldest by
   `created_at`" — a legitimate outcome that also yields exactly one admin, which is why nothing
   noticed. `db::connect` now issues `set_config` inside each migration's transaction.
3. **`command_shaped` refused this contract's own passing example**, and the narrowing that fixed
   it introduced a worse bypass (below).

## Phase 2 — artifact-sync pass

Six divergences between the artifacts and what Phase 2 implemented, folded back. Recorded in
full in `traceability.md` §8a; the one that changed the schema is repeated here:

**The outbox needs twelve entity types, not ten.** `sync-namespaces.md` §6 said twelve,
`data-model.md` said ten, and the two relation types were the difference. Both relations tables
exist in server Postgres as well as locally, a server table is reachable only through the
outbox, and a relation belongs to neither of the two rows it names — so unlike an applicability
fact it cannot ride inside a parent's payload. Without those two types the server's relations
tables could never be populated, and FR-493's "disagreement is resolved by reconciliation and
relations" would have held on one machine and nowhere else. Migration `0007`'s CHECK and T023
now carry twelve, and the project-less CHECK covers all four domain types.

**`command_shaped` was a heuristic standing in for a rule.** "A known command name followed by a
flag or a path" admitted `cargo test`, `rm target`, `sudo reboot`, `npm install` and
`git status`. Replaced with grammatical position, and the contract now states the rule and names
the five commands the old one let through, so the next reader sees why the obvious rule was
rejected rather than only what replaced it.

**Deferred, with the reason:** `OutboxEntityType` still has eight variants. Phase 2 owns the
migration's CHECK (T023); the enum gains its four variants when an enqueue path for the new
types exists, in Phase 5/6. Nothing in Phase 2 enqueues a project-less row, and the table's own
CHECK requires `project_id IS NULL` for exactly those four types, so a premature enum variant
would be a name with no writer.

---

## Phase 3 — US1, administered accounts

Server core, tests and documentation. **T030–T042, T045, T049–T056** complete;
**T043, T044, T046–T048** (daemon request variants and the `cairn user` CLI) delegated and
landing separately.

**`cairn-server users add` and `POST /api/admin/users` share one mechanism.** Both call
`auth::create_user`; neither reimplements validation or hashing. They differ in exactly one
argument, and the reason is the operator's intent rather than an accident:

| | `users add` | `POST /api/admin/users` |
|---|---|---|
| Who runs it | an operator with shell access to the host | an administrator with a token |
| Where the password comes from | the operator types it | the server generates it |
| `must_change_password` | off by default, `--must-change-password` opts in | always on |

The subcommand's caller chose the password, which is how scripted provisioning works and
forcing a change would break it. The route generates a password the new account has never
seen, and there a forced change is the entire point.

**Authorization is a type, not a check.** Three extractors, and which one a route names *is*
its policy:

- `CurrentUser` — establishes identity, refuses a disabled account, does **not** enforce the
  password-change gate. Exactly one route takes it: the password change itself.
- `SettledUser` — identity plus nothing outstanding. What an ordinary route should take.
- `AdminUser` — that, plus `role = admin`.

The gate lives on the extractor rather than in each handler so a route added later inherits it
by writing a type name, and gets it wrong only by explicitly asking for the ungated one.
`create_token` is the route where escaping the gate would undo it entirely — a temporary
credential that can mint a bearer token has bought itself unrestricted access — so it is
called out in a comment there.

**The last-admin guarantee** is one statement under one advisory lock, per D436/D445: the
`EXISTS` subquery and the `UPDATE` are evaluated together, and
`pg_advisory_xact_lock(4770040001)` is taken before either. Write skew is the anomaly — the two
transactions touch different rows, so no row lock makes either wait.

Its test is split in two, deliberately. The concurrent test asserts **one success, one
refusal, one administrator remaining** and does *not* pin the loser's status code: the caller
has to be one of the two administrators, so two legitimate interleavings produce two different
refusals (`403` if the caller's own demotion committed first and it is a member by the time its
second request is authorized; `409 last_admin` if the other committed first). A sequential
companion pins `409` by name, where there is only one path.

**Phase 3 tests:** 12 in `tests/tests/admin_lifecycle.rs`, all passing. Two bugs in my own
tests were worth the finding: the error body is nested under `error`, and the concurrency test
originally ran on the shared database and promoted accounts other tests had created. It now
takes a database of its own — a test that rewrites every account's role cannot share one.

## Phase 4 — US2, membership and safe auto-link

**T057–T062, T064–T067** complete. **T063** (`cairn project member` CLI) delegated.

`POST`/`DELETE`/`GET /api/projects/{id}/members`, with `added_by_user_id` recorded. Two
separate rules, and only the second closes the hole: *you must already be a member to grant*,
and *a grant names somebody else*. An existing member adding itself again is harmless, but a
route shaped to allow it is one refactor away from allowing a non-member to.

Grants are addressed by user id, not email — an email-addressed grant route is an
email-enumeration oracle, since a caller learns which addresses have accounts by watching which
grants return `404`.

**T059/T060/T062 were verification, not new work.** The prerequisite hotfix (`214154f`) already
landed the `lookup_projects` membership join and the `project_id` predicates on every sync
upsert and tombstone. Audited rather than reapplied: all 13 `Path`-taking project-scoped
handlers in `api.rs` carry `require_member`, and both `sync.rs` handlers do. Two were still on
`CurrentUser` and are now `SettledUser` — syncing is an authenticated action and FR-407 admits
exactly one exception.

**T067's sweep reads the live router.** `SC-465` is about the routes that *exist*, including
ones added later, so the test enumerates `api::routes` from source and calls every one with
every verb, asserting the caller's own membership set is unchanged. A test naming the deleted
join route would pass unchanged the day a different route grants self-membership.

## Phase 6 — the security-critical pieces, ahead of the rest

**T098 and T102** done early because they are the feature's two hardest privacy claims.

`MemoryFacts` carries no `writer_seq` and no timestamp, asserted against its serialized field
set so that *adding* one fails. This is what makes "the sequence is diagnostic only" a property
of the type: reconciliation cannot consult what its only input does not carry.

**Server-side ingest is live** — the fifth validator entry point. It calls the identical
`validate_global_content`, never a re-implementation, with `project_identities` set to the union
of every project the pushing user belongs to. That union is deliberately broader than any
client-side check and catches the one case a client-side check structurally cannot: content
naming project X pushed by a client working in project Y. A test asserts exactly that
asymmetry.

The refusal is a distinct type (`IngestRefusal`) surfacing as `422 content_rejected`, separate
from the capability path's `409 unknown_entity_type`, because their remedies are opposite and a
client must branch on shape rather than parse prose. Screening happens **before** any insert
and rolls back the `sync_state` claim, so a corrected payload gets its own attempt rather than
being reported a duplicate.

One finding while writing it: a git remote's structural parts (`git`, `ssh`, `www`, `com`) must
be dropped from the identity set. A project whose identities contained `git` would refuse any
content mentioning version control — over-refusal on a scale that makes the screen useless
rather than merely strict.

---

## Phase 5 — US3, personal memory (store layer)

**T068–T071, T073, T074, T077, T078** complete. `crates/cairn-store/src/global.rs` and
`traits.rs`. The MCP/CLI surface (T079–T082) and the Phase 5 tests (T072, T083–T091) remain.

`validate_global_content` is called as `create_personal`'s **first statement**, before the
transaction opens — so a refused creation has nothing to roll back rather than something rolled
back. Trait derivation is `Path::exists` only, never file content, never a model: `Cargo.toml`
→ `language=rust` + `tool=cargo`, and so on, deduped, with a full delete-and-reinsert on refresh
so a vanished manifest's trait vanishes with it.

## Phase 6 — partial

**T098, T102, T103** complete, plus the four `OutboxEntityType` variants (twelve names).
Server-side ingest is live for both domains and screens against the union of the pushing user's
memberships.

## Phase 7 — partial

**T129, T130** complete: team ingest, completing the fifth entry point for both domains. `state`
is deliberately absent from the ingest upsert's `DO UPDATE SET` list — an ingested item may
create a proposal and carry a tombstone, and may **not** decide its own authority (FR-455,
FR-515). The rest of Phase 7 (the store-side lifecycle, T115–T128, T131–T151) remains.

## Phase 8 — partial

**T152, T153, T155, T158, T163's types, T165, T167** complete in `context.rs` and `budget.rs`.

`remaining_non_reserve()` is `general_remaining().min(limit - reserve_initial)`. One subtlety
the worker found and fixed: querying it fresh per candidate is not enough, because while a large
released reserve keeps `general_remaining()` above the fixed non-reserve pool size, the live
query does not shrink as global's *own* prior admissions consume it — so a second item walks
past the true ceiling. `admit_global` snapshots it once and tracks cumulative global spend
against that snapshot.

`admit_global` is not yet wired into `assemble()`: that needs `Briefing.personal_notes` and
`Briefing.team_guidance` on the wire type, which was under concurrent edit.

## Phase 9 — complete

**T174–T186.** `scope_audit.rs` no longer passes vacuously (proved by renaming its target and
watching it panic). The server's wire check recurses. Handoffs emit repository-relative paths
only. Two Feature 003 contracts corrected against the code rather than the prose. The doc-lint
(T184) is falsifiable three ways — each check has a seeded-violation test.

### One cross-file defect the recursion exposed

`reject_forbidden_fields` now recurses, and `TestRunRecord` still serialized a field literally
named `command` inside `tests_executed`. **Every handoff carrying a completed test run would
have been refused by the server** — the value had been sanitized to a runner name, but a
field-name denylist does not read values. Fixed by renaming the field to `runner`: the key had
to disappear, not merely its contents. No test caught this because none both synced a handoff
through a real server and populated `tests_executed`; it was found by the worker reasoning about
the interaction, and it is the kind of defect that ships.

### Two defects of mine, found by workers

`cairn-server` failed to start when pinned below schema 3 — `server_instance` is read
unconditionally and the table arrives with migration 3. A held-back deployment is a supported
configuration, so the read is now conditional and `server_instance_id` is `Option`.

`create_user` inserted `must_change_password` against schema-2 databases that have no such
column. Now checks the column rather than the schema version, because that is the fact the
statement depends on.

Bare `cairn link` with no server configured reached for the network instead of answering from
local state — my auto-link change. It now falls back to `link_status` when no credential exists.

---

## Phase 5 — completed

**T072, T075, T076, T083–T091** added to the store layer already logged above.

`crates/cairnd/src/promote.rs` composes the eight-check gate and the shared validator and adds
nothing to either. What it contributes is the I/O the pure functions deliberately refuse: reading
the source memory, reading the machine salt, writing the result. The salt's absence is a
**refusal**, not a default — a promotion whose origin digest could not be computed is one whose
check 7 could not run (FR-518).

T076 is the absence rather than an action: the origin digest is the only thing promotion adds to
a record. There is no verification state to carry over in either direction, because the row has
nowhere to hold one.

**T085 is the bypass test.** Direct personal creation refuses an absolute path *and* the identical
content is refused identically at promotion — one shared validator, one answer. Before D433 that
first assertion would have failed: the content check lived inside the promotion gate, and direct
creation never called it, which is the path an agent reaches for first because it needs no project
memory to exist.

**T072** is asserted structurally before it is asserted by value: `OutboxEntityType` has no
`project_traits` variant, so an outbox row for one cannot be *spelled*, let alone written.

## admit_global wired into assemble()

`Briefing` gained `personal_notes` and `team_guidance` as `Vec<String>` — the same shape as
`decisions` and `known_failures`, deliberately. There is **no field for a selection reason**, and
that absence is the enforcement (FR-478, D451): reasons are produced on the diagnostic path and
the rendered type has nowhere to put one, so a renderer cannot leak them by forgetting to omit
them.

Both sections are admitted together under `"personal_notes"` in the `SECTION_ORDER` loop, because
their cap is a property of the pair rather than of either one. Both are `skip_serializing_if =
"Vec::is_empty"`, so a caller with no global knowledge gets byte-identical output to one that
never touched either domain (FR-481).

---

## Phase 7 — store layer complete

**T115–T128, T131, T132.** `propose_team`, `ratify_team`, `retire_team`, `recall_team`,
`list_team`, `team_subject`, `merge_synced_team`, with 22 tests.

The CAS needs no caller-supplied `expected_state`: the required source state **is** the expected
state for each transition. That one choice makes re-ratifying a retired row refuse naming
`retired`, and a losing concurrent ratifier refuse naming `authoritative`, with no extra
branching.

**The server-instance asymmetry** (FR-496 vs FR-567) is where this could most easily have gone
wrong, and the worker got it right. `merge_synced_team` binds to the first instance it sees and
**refuses** a second, naming both. `create_personal`/`recall_personal` take no server-instance
argument at all — personal knowledge is *partitioned* by owning identity, never refused, so one
store holds two identities' personal memory side by side. Reversing these two silently destroys a
user's personal memory the first time they link a second server.

`propose_team` deliberately does not consult the instance binding: a freshly authored proposal is
not "sourced from" any server yet, and gating it would refuse a legitimate proposal made after a
deliberate relink.

### FR-457 was not satisfied, and now is

`team_knowledge` recorded `ratified_by_user_id` **and** `ratified_at` for ratification, but only
`retired_at` for retirement. "Every state transition MUST be recorded with who acted and when" is
not satisfied by a timestamp — and retirement is the transition most worth attributing, because it
removes guidance from every user on the server.

`retired_by_user_id` added to both schemas, `retire_team` takes the actor, and a test asserts both
halves are recorded *and* that the ratifier is still recorded separately — conflating the two
actors would lose the earlier one. `data-model.md`, `contracts/global-memory.md` and T119's text
are synced to match.

The worker found this by reading FR-457 against the schema rather than against the task list,
and flagged it instead of quietly recording half of it.

---

## Build repair after two workers were cut off mid-edit

Both the Phase 6 (sync machinery) and MCP/CLI workers hit a session limit partway through, leaving
`wire.rs` carrying five new optional fields that ~15 call sites across five files had not been
taught about. Repaired here rather than left for a later session, because a red build blocks every
kind of verification.

Every new field is `Option` with `#[serde(default)]`, so the repair at an untaught call site is the
"as before" value — `None`, or `Vec::new()` for `applicability_facts`. That is not a shortcut: a
caller that has not learned about `depth` should behave exactly as it did before `depth` existed,
which is what `None` means here.

The mechanical pass over-applied twice and both mistakes are worth recording, because the shape
recurs: a regex that matches `Name {` cannot tell an **initializer** from a **struct definition**
or a **match pattern**, so it injected `depth: None,` into a clap command definition, into a
`SearchContext` declaration, and after a `..Default::default()`. Redone by driving from the
compiler's own error locations instead — `E0063` names the file, the line and the missing fields,
and nothing else can be mistaken for an initializer.

### Two duplicates removed rather than left

`cairn sync status` had gained **two** per-namespace implementations — `sync::namespace_status` and
`handlers::namespace_sync_status` — from the two workers approaching T109 from opposite ends. Only
the `handlers.rs` one was wired. The unused one is deleted: two implementations of one query is
worse than a gap, because the next person to change the behaviour will change one of them.

A raw `sqlx` query in the surviving one had `.map_err(storage_err)`, which takes `StoreError`.
`storage_err` exists to map the store's *named refusals* onto their contract codes, and a bare
query produces none — so it now maps to `storage_unavailable` directly.

### A correction I made and had to unmake

I removed `reason` from `handlers::context`'s signature on the grounds that nothing read it, to get
under the argument-count lint. It **is** read — it decides the post-compaction path. Restored, and
the lint is allowed with the actual reason: all eight arguments are load-bearing, and bundling them
into a struct would only give `Request::Context`'s fields a second name, since the caller
destructures them straight out of that variant.

---

## A whole class of defect: schema-3 columns read unconditionally

`sync_degradation`'s two upgrade tests failed four times in a row, each time on a *different*
statement, and each failure was the same mistake in a new place: Feature 004 adds columns in
migration 3, and a server pinned below it — a supported, deliberate configuration for a staged
rollout — has none of them.

Found and fixed, in the order the tests surfaced them:

| Statement | Column | What broke |
|---|---|---|
| `login` | `users.status` | every login on a schema-2 server |
| `standing_of` | `users.{role,status,must_change_password}` | every request, at the extractor, before any route ran |
| `create_token` | `api_tokens.expires_at` | every token mint |
| `user_for_api_token` | `api_tokens.expires_at` | every bearer authentication |

At that point I stopped fixing what the tests reported and swept every schema-3 column reference in
`cairn-server` instead. That found two more the tests had not reached:

- **`ensure_admin`** wrote `role`, `status` and `must_change_password` unconditionally, so a
  held-back deployment given `CAIRN_ADMIN_EMAIL` **failed to start**. That is the single
  configuration that exists to prevent lockout, and it was the one that could not boot.
- The **membership routes** name `added_by_user_id`. They now refuse with `schema_too_old` rather
  than surfacing a column error — and refusing is right rather than pedantic: FR-419 requires a
  grant to record who made it, and a grant with the attribution silently missing is exactly what
  this route replaced.

Two judgment calls worth stating. Where the column's absence makes the guarantee **vacuous**, the
old behaviour is correct and is what the code now does: below schema 3 no token can carry an expiry,
so the expiry predicate has nothing to check, and no account can be disabled, so every account is
active. Where the column's absence makes the guarantee **unimplementable**, the request is refused:
a caller asking for a token expiry on a server that cannot record one gets an error, because
silently issuing a token that never expires downgrades a security control the caller asked for.

The lesson is not "remember the schema check". It is that four tests reported four symptoms of one
cause, and reading the fourth as a fourth bug would have left the two that no test covered.

---

## Full workspace green

**1314 passed, 0 failed.** `cargo fmt --all -- --check` clean. `cargo clippy --workspace
--all-targets -- -D warnings` clean.

The last two failures were the `TestRunRecord.command` → `runner` rename reaching two Feature 001
tests. Updated to read `runner`, and each gained a second assertion that a `command` key has **not**
come back — because if it does, the recursive wire denylist refuses every handoff carrying a
completed test run, silently, and the only symptom is sync failing for the sessions that did the
most work.

---

## The lane that was never wired

The previous section ended with a green workspace, and it was green for a reason that had nothing
to do with personal or team knowledge working: nothing exercised the path end to end. An
inventory pass over all 200 tasks — checking each against the code rather than against a
citation — found the shape of the problem immediately. `outbox::enqueue_global` existed, carried
a careful doc comment about the writer-mixed idempotency key, had its own unit tests, and was
called from **nowhere**. `create_personal`'s doc comment said so explicitly and framed it as
Phase 5 being single-machine by design.

That framing was wrong by the time it was written. Without the enqueue there is no outbox row,
without an outbox row `outbox::known_namespaces` returns nothing, and without a namespace the
worker forms no target — so the lane was not slow or incomplete, it was inert. A personal note
recorded on one machine stayed on that machine forever, and every test that could have noticed
was a unit test on one side of the gap.

Wiring it up surfaced five more of the same shape in a row, and they are worth listing together
because the pattern is the finding:

- **`NamespaceClock` never advanced its own clocks.** `record` folded in an outcome and touched
  only the backoff. Nothing in production ever moved `last_pull` or `last_probe`, so both
  predicates were true from the first tick onward and `WORKER_TICK` became the pull frequency:
  three lanes, six requests a second, forever, against a server that answers all of them
  successfully so backoff never engages. `PULL_INTERVAL_SECONDS` existed and described nothing.
- **`reject_beyond_capability` returned `Ok` for any `schema_version >= 2`.** The two new entity
  types were introduced at schema 3 and the gate had never been generalised, so a personal item
  pushed at a schema-2 server sailed past it into a missing table. The distinction that matters
  is not that it failed but *how*: an internal error is not a held item, and a daemon cannot tell
  "retry after the upgrade" from "this server is broken".
- **A schema-2 server reports no instance id, so no lane key could be formed at all.** This one
  needed a design decision rather than a fix. §11a says content queued against a server that
  cannot accept it is held and released automatically once the peer supports it; content that was
  never queued is not held, it is invisible. The lane now opens under a provisional id derived
  from the configured endpoint and re-keys itself to the real id the moment the server reports
  one — the endpoint being the right thing to derive from because it is exactly what §11a's
  scenario holds fixed. Re-keying moves the cursor, the backoff, the capability fingerprint and
  every queued row, and touches no idempotency key, so an entry in flight across the re-key still
  applies exactly once.
- **`namespace_sync_status` read only `outbox`.** That table answers "what has work to push",
  which is the wrong question for a lane whose entire job is pulling. A consume-only device was
  actively pulling on a lane that did not appear in its own status — and neither did the gap
  report attached to it, which is the one place a missing record is ever mentioned.
- **`cairn sync now` drained only the project lane.** A user who runs a command called "sync now"
  and then checks the other machine would reasonably conclude that sync is broken.

## The identity that was not per-server

`T117` failed with both identities' rows carrying the same `owner_user_id`, and the value was
`repo::ensure_local_user`'s — this machine's local id, minted once and never per-server.

`sync-namespaces.md` §10 is precise about why that cannot stand: a user account is per-server, so
the same human on two servers is two accounts with two different ids, and their personal knowledge
must sit in two **disjoint sets of rows**. Keying on the local identity merged them, and there is
no unmerging afterwards — the rows have no other field that distinguishes them.

`Daemon::owner_identity()` now returns the linked server's account id when there is one and the
local id otherwise, learned from `GET /api/auth/me` at authentication and persisted in config
rather than re-fetched, because a daemon restarting offline that fell back to the local id would
silently reassign every existing row. Team knowledge uses the same identity for a related reason:
the server records `proposed_by_user_id` as *its* account id, so a locally recorded proposal keyed
on the local identity would stop being the caller's own proposal the moment the row came back from
a pull, and the role-filtered listing that shows a member their own pending proposals would stop
showing it.

Notes written before any link stay owned by the local id. Reassigning them on link would attribute
work to an identity that did not do it, and would push it to a server the user had not chosen to
send it to when they wrote it.

## Two entry points were weaker than the other three

`T146` asks whether content naming a project is refused *identically* by all five entry points.
Two were not.

`team_propose` screened against an empty identity slice, because `Request::TeamPropose` carried no
`cwd` and the wire module's own comment explained that team knowledge is server-wide so none of
the four team requests needed one. True of authorization, false of the privacy screen:
`validate_global_content` passes `project_identifying` when it has nothing to compare against
(FR-580, the one documented fail-open), so a proposal made with no `cwd` was the one surface of
five at which naming a project was allowed. `TeamPropose` now carries a `cwd`, used as an input to
the screen and never to a permission decision.

Promotion passed `ProjectIdentity(project.name)` — the name alone, where direct creation had always
passed the name *and* every token the git remote contributes. A project called `internal-tooling`
behind `git@host:acme/widgets.git` is named by "acme" and by "widgets" just as surely.
`evaluate_promotion` now takes a slice, and an empty one is `evaluation_incomplete` rather than
vacuous: a project always has an identity, so a caller that supplied none did not establish that
there is none — it failed to look.

## Two invariants pushed back, and were right to

Writing the new surface tripped two pre-existing tests, and in both cases the test was correct and
the new code was not.

`applicability_facts` was drafted as an array of `{kind, value}` objects, which read well in a JSON
schema. `mcp_backward_compatibility` refused it: an action's parameters are flat here by rule
(D70), because a nested object is how a tool grows sub-operations. The wire type had always been
`Vec<String>`, so the flat `kind=value` form removed a conversion rather than adding one.

`scope_audit` refused the word "retroactive" in a doc comment about not reassigning pre-link notes.
The audit screens for valid-time and bitemporal vocabulary, and it does not care that this
particular use was a plain English adverb — which is the right trade for a term whose accidental
adoption is exactly what the audit exists to prevent.

## What is not done, and why

**T104** — `sync_deferred` for the two new relation kinds. The mechanism cannot be reached:
nothing enqueues either relation type, the server's `apply_item` has no arm for them, and there is
no relation read-back, so no such record can arrive to be deferred. `sync_deferred.project_id` is
also `NOT NULL REFERENCES projects(id)`, and a global relation has no project, so holding one needs
a table rebuild rather than a widened `CHECK`. Adding the deferral now would produce a second
mechanism with no callers — which, this session, is a defect class rather than a hypothetical.
Global relation transport is the work this needs.

**T194** — the literal two-machine quickstart walkthrough. Its substance runs automatically in
`namespace_sync.rs` and `capability_upgrade_e2e.rs`; what is missing is a human executing
`quickstart.md` on two physical machines.

Both are recorded in `release-evidence.md` as well, so a reader who never opens this log still
finds them.

## Final state

`cargo test --workspace --all-targets`: **1396 passed, 0 failed**. `cargo fmt --all -- --check`
clean. `cargo clippy --workspace --all-targets -- -D warnings` clean.

---

# Repair pass, after an independent adversarial review

The previous section ended with a green workspace and a claim of 198/200. An
independent spec-to-diff review returned `IMPLEMENTATION-READY: NO` with seven
findings, and it was right. The suite was green while a personal record could be
written, acknowledged with an id, and be unreachable from every read path.

The review's finding was not really seven defects. It was one, seven times: **a
mechanism existed, was unit-tested, and no production path reached it** — the same
class this log already recorded nine instances of. The nine had been found by
writing tests for requirements. These seven were found by asking a different
question: *who calls this?*

That question is now the one this feature is judged by, and it found four more
defects during the repair itself.

## F1 — traits were never derived

`refresh_traits` was the only writer of `project_traits` and had no caller.
`traits_for_project` therefore always returned `[]`, and `applies(&facts, &[])` is
`false` for any record carrying a fact. So every personal or team record with an
applicability condition was invisible in search, in the briefing, and in listing —
in the very project whose trait it had been scoped to. `cairn traits` reported
nothing, in a repository with `Cargo.toml` at its root.

The repair is `Daemon::project_traits`, one accessor that derives when stale and is
now the only thing any read path calls. It is an accessor rather than a step inside
`resolve` because `resolve` runs on every request and most requests do not care
about traits; deriving there would pay for a filesystem scan and a write
transaction on every session event. Bounded by `TRAIT_REFRESH_INTERVAL` (60s), and
invalidated by `forget_repo` — which is `cairn init`, the daemon's one "this
checkout is not what I thought it was" signal.

Two things about this are worth recording. The first is that the fix had to be
found twice: the initial version did not invalidate on `init`, so a manifest added
mid-session waited out the interval, and the test that caught it looked like a
passing test for ten minutes because the e2e suite spawns *prebuilt* binaries and
`cargo build --tests` does not rebuild them. The second is that no test in the
workspace had ever called `refresh_traits` — every trait-using test hand-built
`ProjectTrait` values, which is exactly why a correct function could sit unreached
through three review passes.

## F3 — enumeration is not recall

`cairn personal list` called `recall_personal` with an empty trait slice. That is
not "unfiltered": `applies` rejects every record carrying a fact when the trait set
is empty, so the surface that promises "everything I hold" hid precisely the
records a user had bothered to scope. The doc comment asserted the opposite of what
the code did.

`list_personal` is a separate function with no applicability predicate at all,
because the two answer different questions and the difference is not a parameter.
"All records" is no longer spelled as a fake empty trait set anywhere.

## F2 — reconciliation was write-only

Every global write recorded reconciliation — `duplicates`, `conflicts_with`, and an
administrator's explicit `supersedes` — and no reader consumed any of it. The only
consumer was `personal_subject`/`team_subject`, which nothing called. Three
canonical reads each spelled `state = 'authoritative'` independently, and none
consulted supersession.

The user-visible consequence: `cairn team ratify <new> --supersedes <old>` succeeded
and the replaced guidance kept being served, to everyone, indefinitely.

Two repairs, and the split matters.

**One definition of "current".** `team_active_predicate` is now the single place
`state = 'authoritative' AND superseded_by_id IS NULL` is written, used by
`search_team`, `recall_team`, `team_subject` and `team_members_tx`. Three spellings
of one predicate is three places for it to drift, and it had already drifted.

**The pointer, not only the relation.** Ratification now sets
`team_knowledge.superseded_by_id` in the same transaction as the state change, on
both sides. The relation records *that an administrator decided*; the pointer is
what readers consult and — decisively — what already crosses the wire, so a second
device learns of the supersession without the relation tables needing to
synchronize. That is what lets F2 be correct while T104 stays incomplete.

**And a timestamp, for the cursor.** `superseded_at` was added to the server's
`team_knowledge` for one reason: `GET /api/sync/changes/team` orders on `GREATEST`
of a row's own timestamps, so a change with no timestamp cannot move a device past
it. Setting only `superseded_by_id` would have fixed the ratifying device and left
every other device serving replaced guidance — the same defect one layer out. This
was caught by reasoning about the fix, not by a test, which is worth admitting.

**A reachable subject read.** `cairn memory subject --domain personal|team` is the
production caller `personal_subject` and `team_subject` never had. FR-442 requires
"the same deterministic reconciliation already used for project memory", and that
is only a real property if the same function answers a user's question in all three
domains. `--scope` is refused for a domain that has none rather than accepted and
ignored.

## F4 — a test mutated shared git metadata

`compat_old_client.rs` ran `git worktree add`, which writes into the repository's
*shared* `.git/worktrees/` — state outside the worktree under review — and never
deregistered it, so `cargo clean` orphaned the entry. It is now `git archive | tar
-x`, which reads history and writes only the files asked for, and the test asserts
that no `pre004-src` registration exists afterwards. Verified from a cold cache: a
genuine 61-second build, no registration.

## F5, F6, F7

`personal_subject` and `team_subject` gained the subject-read caller above.
`recall_team` now backs the briefing's team fetch, mirroring `recall_personal` on
the personal side — and the choice is not cosmetic: `search_team` ranks by BM25
over a query, a briefing has no query, so every row would score zero and the order
would fall to an arbitrary tiebreak. Recency is a defensible answer to "what should
an agent see first". Both consult the one active-entry predicate, so they cannot
disagree about what is current, only about what to show first.

`cursor::backoff_until` and `set_backoff_until` are removed. Per-namespace backoff
lives in memory per worker task, which is what the contract asks for; a public API
implying durable state that nothing wrote was misleading. The column stays —
`data-model.md` §7 names it — annotated as reserved and unwritten, because a reader
who found it populated would reasonably assume it governed something.
`failures_namespace` and `total_namespace` are removed for the same reason: `sync
status` computes every count it reports in one aggregate query, so a public
accessor per count was a second way to ask a question nothing asked.

The privacy claim for traits is now exercised against traits that exist: derive
through the lifecycle, link to a real server, synchronize, then ask the server what
it has. The previous test screened a hand-built corpus and held for the wrong
reason.

## Four more defects, found while repairing

**Sync ingest could hide team guidance.** `upsert_team`'s `ON CONFLICT DO UPDATE`
accepted `superseded_by_id` and `retired_at` from a pushed payload. Both are
administrator acts behind an `AdminUser`-gated route, and once canonical reads began
consulting `superseded_by_id` this became a privilege escalation: any authenticated
account, with no project membership, could name an arbitrary successor and remove
any authoritative guidance from every reader on the server — or push a null and
resurrect guidance an administrator had replaced. The conflict clause is now `DO
NOTHING`. The lifecycle travels server-to-device only, which is the only direction
an administrator's decision can legitimately move.

Worth stating plainly: the F2 repair *created* the exploitability of a field that
had been inert. A fix that makes a dormant field load-bearing has to be followed by
asking who else can write it.

**`--supersedes` returned a 500 for everyone.** `record_supersedes`'s existence check
was `SELECT 1` decoded into `i64`; Postgres types the literal as `INT4`, so it
failed at runtime with a type mismatch. No test had reached the line, because
nothing consumed supersession. Now `SELECT id`.

**A relink opened a second team lane.** `bind_team_server_instance_tx` answers "which
instance is this store's team corpus bound to?" by reading the recorded `team:*`
lane. Establishing a second one made the question ambiguous and the answer became
whichever row the query returned first. A store may hold several `personal:*` lanes
and exactly one `team:*` lane — the asymmetry is D438 — so the second is no longer
opened.

**A lane pulled the wrong peer.** That fix alone was not enough, and the reason is
the more interesting defect: the lane key carries an instance id, but the HTTP
client points wherever the current token points. After a relink, the surviving
`team:<A>` lane pulled server B and merged B's ratified guidance into a corpus bound
to A — labelled as A's, so `merge_synced_team` could not catch it: it was handed the
lane's own instance, which matched by construction. `pull_global` now confirms the
peer reports the instance the lane names before pulling anything. This is the
strongest argument in this log for the review's central standard: three of these four
were invisible to every test until a mechanism became reachable.

## What the green suite was worth

The suite was green before this pass and green after it, and the two greens mean
different things. The first meant "every test passes". The second means "every test
passes, and the mechanisms the tests describe are reached by the paths users take".
Only the second is evidence.

---

# Integration pass

Feature 004 committed as `c644d92` on `feat/004-collaborative-global-memory`, and
integrated onto `main` (`214154f`) in an isolated worktree. Integration itself was
a fast-forward: the branch was based on main's tip and main had not moved, so there
were no conflicts to resolve. The integrated tree ran green on the first attempt.

Three things surfaced afterwards that were worth fixing rather than shipping.

## `retired_by_user_id` never left the server

FR-457 asks that every team state transition be recorded with who acted **and**
when. Ratification satisfied both halves everywhere; retirement satisfied them only
on the server and on the machine that performed it.

The field was not on `cairn_core::global::TeamKnowledge` at all, so the record type
could not carry the answer even where the database held it — `cairn team list` had
no way to say who removed a piece of guidance, and `SyncedTeamKnowledge` had
nothing to copy. `SyncedTeamKnowledge`'s own doc comment claimed it carried "every
field `team_knowledge` stores except `origin_digest`", and the server's wire row
claimed to match it "field for field". Both statements were false, and the field
they were false about is the one an operator asks for first.

Now carried end to end: record type, wire row, `team_changes` projection, mirror,
merge. Both doc comments were rewritten to be true, including about `superseded_at`,
which is genuinely server-local (it exists so a supersession can move the pull
cursor, and a device has no use for it).

## Four checkpoints that were not deletions

`Store::checkpoint` runs `PRAGMA wal_checkpoint(TRUNCATE)`, takes an exclusive
lock, and exists for one purpose: so deleted content leaves the write-ahead log
rather than lingering in an old frame (FR-052). Feature 004 added five calls. One —
`forget_personal` — is a real content removal and belongs. The other four —
`ratify_team`, `retire_team`, and both merge paths — change lifecycle columns and
remove nothing.

The two merge paths were the expensive mistake: they run once per pulled row in the
background worker, so a machine catching up took an exclusive checkpoint lock
repeatedly while foreground commands were writing. All four are gone.

## The deferred transaction underneath it

Removing the checkpoints cut `cairn connect` failing with "database is locked" from
about half of runs to about one in six. It did not remove it, and the residual had
a different cause that predates this feature.

`integrations::bind`, `unbind` and `remove_agent_if_unbound` opened a **deferred**
transaction with `pool().begin()`, read, and then wrote. SQLite refuses that lock
upgrade with `SQLITE_BUSY` *immediately* when another connection has written since
the read began — a busy timeout does not apply to an upgrade, only to acquisition.
So under any concurrent writer these failed outright rather than waiting. This is
precisely what `crates/cairn-store/src/tx.rs` exists to prevent, and its own doc
comment says so; these three call sites simply never used it.

The defect is Feature 002's. Feature 004 is what made it show: three synchronization
lanes and a background pull put a second writer on the store where there had usually
been none. Switched to `tx::begin` (BEGIN IMMEDIATE, with the contention retries the
rest of the store already gets): eight consecutive runs green, where the same suite
had been failing roughly one run in two.

Worth stating as the general lesson, because it is the same one this log keeps
recording: none of these three were found by a failing assertion about the thing
that was wrong. They were found by asking, of a green suite, *what does this
actually reach, and who else is writing at the same time?*
