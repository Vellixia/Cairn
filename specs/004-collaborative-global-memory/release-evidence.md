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
