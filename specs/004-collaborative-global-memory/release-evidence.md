# Feature 004 — release evidence

Verified on `feat/004-collaborative-global-memory`, base `main @ 214154f`.

## The suite

```
cargo fmt --all -- --check          clean
cargo clippy --workspace --all-targets -- -D warnings    clean
cargo test --workspace --all-targets  1405 passed, 0 failed
```

Re-run after the adversarial-review repairs; the compatibility suite was run from a cold cache,
so the pre-004 client was genuinely rebuilt rather than reused, and `git worktree list` is
unchanged afterwards.

PostgreSQL 16 at `postgres://cairn:cairn@localhost:5433/cairn`. The server suites report a
named skip rather than passing vacuously when `CAIRN_TEST_DATABASE_URL` is absent, and
`ci_hermeticity` asserts that this is the **only** environment variable any test reads — which
is why the pre-004 compatibility suite builds its own client from repository history rather than
being handed one.

## What the tests found

Nine defects, each found by a test written for a requirement rather than by inspection. They
are listed because the pattern matters more than the individual bugs: in every case a mechanism
existed, was unit-tested, and was reachable from nowhere.

| Defect | How it presented | Found by |
|---|---|---|
| `outbox::enqueue_global` was called from no write path | personal knowledge was recorded locally and could never leave the machine; `outbox::known_namespaces` returned nothing for either global lane, so the whole lane was inert rather than slow | T100 audit |
| `NamespaceClock::last_pull` / `last_probe` were never advanced | `pull_due` stayed true from the first tick, so every lane pulled and re-probed on every 500 ms tick — the unbounded poll `PULL_INTERVAL_SECONDS` exists to prevent, with backoff unable to help because the requests succeed | `marking_a_pull_or_a_probe_is_what_advances_its_clock` |
| `reject_beyond_capability` returned early for any `schema_version >= 2` | `personal_knowledge`/`team_knowledge` pushed at a schema-2 server bypassed the capability gate and reached a table that does not exist, surfacing as an internal error — which a daemon cannot distinguish from "this server is broken", so the lane failed instead of blocking | T187 |
| A schema-2 server reports no `server_instance_id`, so no lane key could be formed | a personal write against such a server was **never queued at all**: not held, not reported, invisible. §11a says queued content is held and released on upgrade; content that was never queued is neither | T187 |
| `owner_user_id` on personal knowledge was the **local** user id | two server identities on one store shared one partition, so relinking merged two accounts' personal knowledge with no way to separate them afterwards (FR-567, FR-568) | T117 |
| `team_propose` screened against an empty identity set | the team-proposal entry point was the one of five at which naming a project was permitted, because `validate_global_content` passes `project_identifying` when it has nothing to compare against | T146 |
| Promotion screened against the project's **name** alone | a project called `internal-tooling` behind `git@host:acme/widgets.git` is named by "acme" and "widgets" too, so promotion was weaker than direct creation for the same content | T146 |
| `upsert_personal` / `upsert_team` never wrote applicability facts | every fact was dropped at push and read back empty, so a record arriving on a second device became **universal** — silently widening its audience | T101 review |
| `namespace_sync_status` read only `outbox` | a lane whose only job is pulling has no outbox row, so the lane it was actively pulling on was absent from its own status — and so was the gap report attached to it | T114 |
| `cairn sync now` drained only the project lane | "sync now" was true of one third of what the command is named after | T114 |

## Found during integration

| Defect | Consequence | Origin |
|---|---|---|
| `retired_by_user_id` absent from the record type and the wire | "who retired this guidance" was answerable only on the server; two doc comments asserting the shapes matched field for field were false (FR-457) | 004 |
| four `wal_checkpoint(TRUNCATE)` calls on non-deletion paths, two of them per pulled row | the background pull repeatedly took an exclusive lock while foreground commands wrote | 004 |
| `integrations::{bind,unbind,remove_agent_if_unbound}` used a deferred transaction and then upgraded the lock | `cairn connect` failed with "database is locked" in roughly half of runs; SQLite refuses a lock upgrade immediately and a busy timeout does not apply | 002, exposed by 004's background writers |

All three repaired. `privacy_integration` now passes eight consecutive runs where it
had been failing about one in two.

## The five entry points

`tests/tests/global_content_validation.rs` exercises the same input against all five
(direct personal creation, personal promotion, team proposal, team promotion, server-side
ingest) across an adversarial corpus, and asserts: identical refusal by class, no fragment of the
content in any refusal, no record and no outbox entry left behind, and an ingest refusal
distinguishable from a capability refusal by response code rather than by message text.

One case is deliberately **not** "refused everywhere":
`no_entry_point_lets_a_credential_reach_storage`. The four local entry points redact before they
validate, so by the time the validator sees a credentialed URL there is no credential left to
refuse; server ingest validates as pushed and refuses it. Both protect the user, and the
assertion is the stronger claim — that no entry point lets a credential reach storage, whether
by refusing the write or by removing the credential from it.

## Capability upgrade, end to end

`tests/tests/capability_upgrade_e2e.rs` runs SC-445 as one scenario: content queued against a
schema-2 peer is held while project sync drains at full speed; the peer is replaced **at the
same address** by one that applies every migration; with no `cairn` command run against the
sandbox afterwards and no daemon restart, the held content delivers itself, and the idempotency
keys before and after are byte-identical.

The same-address replacement is why `Server::upgraded_in_place` exists. A replacement on a new
port would be testing a re-link, and a re-link is a local write the scenario forbids.

## Old client, new server

`tests/tests/compat_old_client.rs` builds the real pre-004 `cairn`/`cairnd` pair from commit
`214154f` and runs it through init, authentication, link, write, push and pull against a 004
server: the push lands, a record written by a current client arrives, nothing is degraded,
nothing failed. SC-457 asks for a real binary specifically because a hand-rolled request omits
whatever fields its author knew had changed — which is exactly the set a shipped binary still
sends.

The two removed routes answer `410 Gone` with a body naming both the replacement route and the
CLI verb, and the shipped README states all three facts plus the operator's remedy.

## The adversarial review, and what it changed

The first version of this document reported a green suite and 198/200. An
independent spec-to-diff review returned `IMPLEMENTATION-READY: NO`, and the
central finding was that a green suite is not evidence of a reachable
implementation. Seven findings, four more discovered while repairing them, and every
one an instance of the same class the table above lists nine of: a mechanism exists,
is unit-tested, and no production path reaches it.

The repairs are recorded in `implementation-log.md`. What belongs here is the
correction to this document's own claims:

| Previously claimed | Actually true before the repair |
|---|---|
| T071 complete — traits derived from manifests | `refresh_traits` had no caller; `project_traits` was empty forever, so every applicability-restricted record was invisible in every project and `cairn traits` reported nothing |
| T082 complete — `cairn personal list` | enumerated through `recall_personal` with an empty trait slice, which hides every record carrying a condition |
| T078, T127 complete — `derive_subject` reused at read time | `personal_subject`/`team_subject` had no caller; reconciliation was recorded on every write and read by nothing |
| T115/T128/T131 complete — team recall | `recall_team` had no caller; the briefing used `search_team`, and neither consulted supersession |
| SC-457 verified against a real pre-004 client | true, but the harness registered a git worktree in the repository's shared `.git`, mutating state outside the worktree under review |

Four further defects surfaced during the repair, all runtime-reachable:

| Defect | Consequence |
|---|---|
| `upsert_team` accepted `superseded_by_id`/`retired_at` on conflict | any authenticated account, with no project membership, could remove any authoritative team guidance from every reader on the server — an escalation the supersession fix turned from inert into exploitable |
| `record_supersedes` decoded `SELECT 1` as `i64` | `--supersedes` returned an internal error for every caller; no test had reached the line |
| a relink established a second `team:*` lane | the instance binding became ambiguous and resolved to whichever row the query returned first |
| a lane pulled whichever peer the token pointed at | after a relink, the surviving `team:<A>` lane merged server B's ratified guidance into a corpus bound to A, labelled as A's, where the mismatch check could not see it |

## Deliberately not implemented

**T104 — global relation transport, and therefore `sync_deferred` for its two kinds.**
Still incomplete, and the blockers are unchanged: nothing enqueues
`personal_knowledge_relation` or `team_knowledge_relation`, `apply_item` has no arm for either,
there is no relation read-back, and `sync_deferred.project_id` is `NOT NULL REFERENCES
projects(id)` so holding a project-less record needs a table rebuild rather than a widened
`CHECK`.

**Its real consequence, stated accurately.** An earlier version of this document presented T104
as a self-contained deferral with no user-visible effect. That was wrong: relation rows do not
synchronize, so a reconciliation decision recorded on one device is not visible on another. What
the repair establishes is that this does not affect the decisions that matter across devices,
and the reason is specific rather than reassuring:

- **Supersession does cross the wire**, because it is carried by
  `team_knowledge.superseded_by_id` and its `superseded_at` timestamp, not by the relation. It is
  also decided *only* on the server, by an admin-gated route, so every device learns it from the
  same authoritative source on its next pull. The relation row is provenance — "an administrator
  decided this" — and stays local to wherever the decision was applied.
- **Retirement likewise** travels as `retired_at`.
- **`duplicates` and `conflicts_with` are derived, not transported.** Both are recorded by
  `classify_proposal` at write time from the rows a store holds, so a device that has pulled the
  same records re-derives the same relations independently. Two devices holding one corpus agree
  because the derivation is deterministic over the corpus, which is the same reason the merge
  corpus is clock-independent — not because the decisions were shipped.
- **What is genuinely absent** is an *explicit* cross-device decision for the two automatic kinds.
  There is no surface that records one: `narrows` and `not_applicable_to` are project-memory
  reconciliation actions and have no personal or team equivalent in this feature. So there is
  currently no decision a user can make in these domains that fails to reach another device.

That is why T104 does not block release. It becomes blocking the moment either domain gains an
explicit user-recorded relation, and completing it then means the whole transport — enqueue,
capability and deferred behaviour, server ingest, read-back, multi-device convergence, upgrade
compatibility — never the `sync_deferred` fragment alone, which would reproduce exactly the
defect class this feature has now found thirteen instances of.

**T194 — the literal two-machine quickstart walkthrough.** Its substance runs automatically:
`namespace_sync.rs` exercises two devices of one account against a real server, including a
consume-only device that receives knowledge it never wrote, and `capability_upgrade_e2e.rs`
exercises the staged-rollout path against a real schema-2 server upgraded in place. What is not
done is a human executing `quickstart.md` on two physical machines and recording the transcript.

## Post-integration review, round 3

Three defects, and all three are the same shape as the ones before them: a value learned once
and trusted afterwards, in a place where the thing it described had moved on.

**A raw failed test command reached transmitted handoff fields.** `derive_tests` sanitized the
command it recorded and said so in its own comment — "the runner's name, never the invocation" —
while `derive_failures`, ten lines away, formatted the *raw* command into `Test failed: {…}`.
That string is not confined to `failures`: `derive_remaining`, `derive_progress` and
`derive_next_step` all build prose from it, so one unsanitized format reached several
transmitted fields at once. Capture-time redaction does not close it, because `redact::redact`
matches secret *shapes*, and neither `/Users/dev/work/repo` nor `--db-password=hunter2` is one.
Both paths now go through `test_runner_name`, the single implementation. Six unit tests over
strings cover POSIX, Windows, UNC, a secret-bearing argument, the summary fallback, and the
whole `synthesize` output serialized and searched for every fragment.

**A token switch could keep the previous account's identity (FR-591).** `account_id` is
persisted deliberately, so a daemon restarting offline still knows whose personal partition it
holds; falling back to the machine's local id would silently reassign every existing row. But
`learn_account_identity` is best-effort — offline, it reports success whenever an id is
*already* recorded — and `establish_global_namespaces` skipped relearning for exactly the same
reason, because the field was `Some`. Set a second account's token while offline and account B
read and wrote inside account A's partition, with no failure reported anywhere. A credential
change now invalidates the identity in memory and on disk and the daemon fails closed: no
`personal:*` lane can be keyed and nothing is attributed to the previous account until a live
`GET /api/auth/me` answers. Re-setting the *same* credential is not a change and keeps the
identity, which is the common offline-friendly case. `logout` was the sibling — it dropped the
token and left the id sitting there looking authoritative — and now drops both.

A second sibling was fixed without a test that can fail today: `merge_pulled_personal` stamped
`owner_user_id` from `owner_identity()` rather than from the lane key that delivered the row.
The two agree in the ordinary case and are not the same thing, and the lane key is the authority
on whose rows it carries. Driving them apart requires an identity change mid-pull, which no
deterministic surface exposes; recorded here as hardening rather than as covered behaviour.

**A `team:*` cursor ignored whose feed it had walked (FR-592).** A `personal:*` key carries its
owner, so two identities get two lanes and two cursors. A `team:*` key carries only the server
instance — deliberately, since a store binds to one server's team corpus — while the team feed
is *not* the same feed for every caller: a pending proposal reaches its author and any admin and
nobody else. A member promoted to admin therefore inherited a cursor that had already walked
past proposals it could not see then and can see now, and a monotonic cursor never asks again.
An admin's store was permanently missing the proposals that admin exists to ratify.

`GET /api/sync/changes/team` now returns a `visibility` fingerprint derived from `SettledUser`
— the authenticated actor, never payload — and the client stores it in
`sync_cursor.visibility_context`. A reported value that differs from the stored one discards the
cursor instead of advancing it, and the lane re-reads from the beginning; every team merge is
idempotent by id, so the cost is one request. A lane with no cursor yet advances normally rather
than restarting for nothing, and a server that reports no `visibility` behaves exactly as
before. The client could not compute this itself: role is half of what decides the filter, and
role is the server's to state.

Each of the five new e2e tests and all six new unit tests were confirmed to fail against the
unfixed code, and to fail only against their own fix.

Gate after round 3: **1436 passed, 0 failed**; `cargo fmt --all --check` and
`cargo clippy --workspace --all-targets -- -D warnings` clean.

T104 and T194 are unchanged by this round and stay as recorded above.

## Post-integration review, round 4

Round 3 established that a credential change must invalidate the identity learned from it.
Round 4 is what that repair did not finish: three places that still routed global sync by
something other than the authenticated account, and one that invalidated the identity only in
memory.

**The routing guard was at one of two entry points (FR-593).** `cairn sync now` skipped a
`personal:*` lane whose owner is not the current account. `run_worker` builds its own target
list — from the outbox, for what needs pushing, and from `sync_cursor`, for what needs pulling
— and applied no such filter. So the guarantee held exactly as long as a user synchronized by
hand: on the worker's next tick, thirty seconds later and unprompted, account A's lane was
drained and pulled under account B's credentials. Both entry points now route through one
`may_sync_lane`, because a rule enforced at one of two call sites is not enforced.

The pull direction is the serious half, and the one the lane filter alone closes. A
`GET /api/sync/changes/personal` sent on A's lane while holding B's token returns **B's** rows
— the server filters that feed by the authenticated caller, correctly — and
`merge_pulled_personal` files them under the lane's owner, which is A. One account's knowledge
written into another's partition, on a timer.

**A queued team proposal could change author by being late (FR-594).** The `team:*` lane is
shared by every account on a server, by design, and the server refuses to trust payload
identity — also by design, and a round-2 repair. Together those two correct decisions meant an
undelivered proposal authored as A, pushed after B logged in, was recorded with B as its
proposer. Global outbox rows now carry `authored_by_user_id` — the *account*, distinct from
`writer_id`, which names the device — and the claim filters on it. Filtering in the claim
rather than after it is what keeps this from being a spin: a row claimed and then declined has
already spent an attempt and moved to `in_flight`, so it would eventually read as a failing
delivery rather than a waiting one. Unclaimed, it stays `pending`, reports as pending, and goes
out unchanged when its author returns.

**The batch's authorization project was a fact about the machine's past (FR-595).** A global
batch carries no project, because personal and team rows belong to none, but
`POST /api/sync/batch` authorizes by project membership and so needs one. The client offered
the first *locally linked* project. A store linked as A and authenticated as B named A's
project, the route refused a caller who is not a member of it, and every global push failed —
personal and team both, silently, for as long as B stayed logged in. Nothing local can
distinguish the two cases, because membership is not local state, so the server is asked:
`GET /api/projects` returns exactly the caller's memberships, intersected with what this
machine has linked so an established project is preferred. Resolved lazily, on the first
non-empty batch, so a lane holding only another account's held rows costs no request at all.

**Invalidation was fail-closed for one process (FR-596).** `forget_account_identity` logged a
config-save failure at debug and returned. The in-memory clear is real, and lasts until the
next daemon start — which reads the previous account back off disk and pairs it with the *new*
token, which is FR-591 again, reconstituted by a restart. The save is now propagated, and the
invalidation moved ahead of every write: when it fails, the old token is still on disk beside
the old account id and the two still agree. There is no state in which a stored credential and
a stored identity name different accounts. `logout` follows the same order, and reports a
failure rather than removing a credential while leaving the identity that outlived it.

Round-4 coverage: two store-level unit tests for the author-scoped claim, and five e2e tests
for the background pull, team attribution across a switch, membership-based authorization,
restart persistence, and a credential change whose invalidation cannot reach the disk. Each was
confirmed to fail against the unfixed code and against no other fix.

Two of them needed the arrangement corrected before they could fail at all, which is worth
recording because both first drafts were green against the defect:

- The background-pull test passed until the daemon was restarted after the account switch. A
  restart puts every lane's clock back to due; without it, B's brand-new lane is due at once
  while A's waits a full pull interval, B's row lands first, and the misfile then fails on the
  primary key instead of happening.
- The push-direction version of that test passed because the author filter (FR-594) closes
  pushing independently. Only pulling isolates FR-593.

Gate after round 4: **1443 passed, 0 failed**; `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and `git diff --check`
clean.

T104 and T194 are unchanged by this round.
