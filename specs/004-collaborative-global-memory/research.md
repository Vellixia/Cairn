# Research: Cairn Collaborative Global Memory

**Feature**: `004-collaborative-global-memory` | **Date**: 2026-08-21
**Baseline**: `main` @ `96178fc` (v0.1.0-alpha.5)

Feature 001 recorded D1–D17; Feature 002, D18–D42; Feature 003, D43–D83. This feature's
decisions are **block-numbered D401–D445**, matching the FR-401..FR-574 allocation this
feature assigns itself — a namespace choice, not a claim that D84–D400 exist elsewhere.
D401–D432 are the original design; D433–D445 are the repairs recorded in §11 after
`/speckit-analyze`.

Sections 1–8 are the investigations that produced the architecture in the brief's §2; they
are argued here because they are *why* the decisions read the way they do. Section 9
records the four decisions the user made before this investigation started. Section 10 is
the decision log, D401–D428: question, decision (the brief's intent, unchanged), rationale
grounded in the repository, alternatives rejected.

---

## 1. Why not `MemoryScope::Global`

**Question**: the obvious way to add project-independent knowledge is a fifth
`MemoryScope` variant. Why does 004 reject that for an orthogonal `KnowledgeDomain`?

**The schema wall.** `MemoryScope` has exactly four variants
(`crates/cairn-core/src/domain.rs:112-120`, `Project|Branch|Task|Session`), persisted with
`CHECK (scope IN ('project','branch','task','session'))`
(`crates/cairn-store/migrations/0001_init.sql:105-106`). SQLite cannot `ALTER ... CHECK` in
place; widening it means rebuilding `memories`, which `0005_project_intelligence.sql:10-15`
already refused to do, and which `0005:441-442` states as a general rule when it *did*
rebuild the outbox for a related reason: "SQLite cannot alter a CHECK constraint in place,
so the table is rebuilt." The server's `memories.scope` is plain `TEXT` with no CHECK
(`cairn-server/migrations/0001_init.sql`) — a new value would round-trip through the server
silently while the client that enforces the vocabulary refused it, the exact
client-accepts/server-silent asymmetry §7 documents elsewhere.

**One exhaustive match, many silent ones.** Only `cairnd/src/handlers.rs:2262-2277`
(`resolve_scope`) is an exhaustive match with no wildcard, so it alone would fail to compile
on a fifth variant — the right outcome, and the *only* place that gets it. Everywhere else
compiles clean and misbehaves: three duplicated SQL `CASE`s each `ELSE 3`, tying a new
variant with `Session` (`cairn-store/src/search.rs:37-39`, `:251-253`,
`cairn-server/src/api.rs:628-629`); `repo.rs:908-911`'s `_ =>` arm keeps a sender's raw scope
key for an unknown variant; `repo.rs:1308-1320`'s `mark_stale_scopes` has `_ => false`, so
the new scope never goes stale; `handlers.rs:1881-1890` is the one wildcard site that fails
loudly, and only without an explicit scope key.

**The code already flagged this.** `cairn-core/src/knowledge.rs:181-186` (`scope_overlap`)
carries a comment for this exact moment: "adding a scope that shares a rank has to decide
this deliberately." `derive_subject` partitions by
`(project_id, scope, scope_key, topic_key)` (`knowledge.rs:252-258`) — a project-independent
scope has no natural `project_id` or `scope_key`, forcing a sentinel value Constitution VI
("Identity, scope, and sync are keyed by stable identifiers") does not sanction.

**Two tests exist to catch it.** `tests/tests/scope_audit.rs:360-370` hard-asserts
`MemoryScope::ALL == ["branch","project","session","task"]` ("the scope vocabulary changed;
every addition to it is a retrieval decision"); `domain.rs:962-966` asserts bucket order.

**The precedent already chosen.** Feature 003 already solved "knowledge not about one
project" once, for patterns, without a scope: `reusable_patterns`
(`0005_project_intelligence.sql:239-260`) has **no `project_id` column**, and the migration
says why (`0005:236-238`): "a pattern that cannot name a project cannot leak one." D401–D403
generalize that same answer rather than inventing a scope variant.

---

## 2. Whether the existing privacy boundary already holds

**Question**: before adding two more categories of knowledge that leave a project's
boundary on purpose, does the boundary that already exists actually hold? **It does not.**

**The doc comment contradicts the body.** `cairn-server/src/sync.rs:296`:

```rust
/// The allowlist enforced on the wire.
fn reject_forbidden_fields(item: &SyncItem) -> Result<(), ApiError> {
```

(`sync.rs:296-322`.) The function rejects named fields/types and accepts everything else —
a denylist, not an allowlist. `specs/003-project-intelligence/contracts/privacy-sync.md:5-8,
313` inherited the same wrong description.

**It does not recurse, and that is exploited today.** The check inspects
`item.payload.as_object()` top-level keys only (`sync.rs:303-313`). Live leaks through
`handoffs`, all crossing the wire today:
- `changed_files` — absolute local paths, copied unrelativized from the vendor tool
  (`crates/cairn-integrate/src/agents/mod.rs:259-262`), collected by `derive_changed_files`
  (`cairn-core/src/handoff.rs:82-105`), whose own comment admits it: "Observations carry
  absolute paths" (`handoff.rs:91-92`). The dedup only collapses an absolute path against a
  Git-relative *suffix match*; anything with no Git counterpart (untracked/ignored files)
  survives. Sent verbatim (`cairn-store/src/outbox.rs:468`), stored with no check
  (`cairn-server/src/sync.rs:601`). The denylist blocks a key named `"path"`, not the array.
- `completed_work` — the same paths again, reformatted into prose by `derive_completed`
  (`handoff.rs:152-171`), now unrecoverable by any future path-shaped filter.
- `failures`/`decisions` — raw observation summaries, pushed verbatim by `derive_failures`
  (`handoff.rs:120-134`) and `derive_decisions` (`handoff.rs:136-151`). The denylist blocks a
  top-level `"summary"` key; these are array elements.
- `tests_executed[].command` — raw shell commands via `TestRunRecord`
  (`domain.rs:913-918`, built at `handoff.rs:109-118`). The denylist blocks top-level
  `"command"`/`"outcome"`; nested, they are invisible to it.

**The client's own mirror is stale.** `cairn-core/src/wire.rs:1700-1708`
(`REJECTED_OBSERVATION_FIELDS`) lists 7 names; the server's live `FORBIDDEN_OBSERVATION_
FIELDS` (`cairn-server/src/sync.rs:21-56`) has 27. No production code reads the client
constant, but it is a trap presented as the boundary.

**What this means for 004**: requirement "raw observations and local paths must never cross
the wire" is a **repair of shipped behavior**, not a new capability — `FORBIDDEN_
OBSERVATION_FIELDS` already claims this guarantee and does not deliver it for the one
entity type most likely to carry exactly the local, path-shaped, project-identifying
content personal/team knowledge must also never leak. This is why the brief puts the
handoff repair **in scope** for 004 (D-U4), not deferred.

---

## 3. How multi-device concurrency can be safe without clocks

**Question**: personal-global crosses one person's devices; team crosses people and
projects. Neither has a reliable clock relationship. What makes this safe without vector
clocks, HLC, or LWW?

**The building blocks are already scope-agnostic.** `MemoryFacts`
(`cairn-core/src/knowledge.rs:437-454`) and `Relation` (`:210-216`) carry **no timestamp
field at all** — stated as the enforcement mechanism in the module header
(`knowledge.rs:10-17`): "`derive_subject` never reads `created_at`, `updated_at`,
`effective_from`, a relation's `decided_at`, or the timestamp embedded in a UUIDv7 to choose
between competing proposals ... The types here carry no timestamp at all, so the rule is
enforced by what the function can see rather than by review." Three facts compound:

1. **No canonical row.** `0005:86-88`: "there is no canonical row, so the project's answer
   is derived from proposals and decisions and there is nothing for a later writer to
   overwrite (D44, D47)." `knowledge.rs:525`: "Nothing stores this (D44)."
2. **Content is never updated after creation.** The only `UPDATE ... SET content`
   statements anywhere are tombstones (`cairn-store/src/repo.rs:1501,1601`,
   `cairn-server/src/sync.rs:628`, `api.rs:713`). Two devices proposing the same fact write
   two rows with two UUIDv7 ids; import is `INSERT OR IGNORE` (`repo.rs:927`), so a peer's
   row never overwrites a local one.
3. **Disagreement is relations, resolved at read time.** `classify_proposal`
   (`knowledge.rs:329-427`) deterministically classifies a new proposal against existing
   members in its `(scope, scope_key, topic_key)` bucket: identical `content_norm_digest` ⇒
   `Duplicate`+`Duplicates`; same `value_key`, different content ⇒ `Corroborating`, no
   relation; different `value_key` ⇒ `ConflictDetected`+`ConflictsWith`. `derive_subject`
   (`knowledge.rs:580-872`) then computes the answer at read time, never picking a winner
   when several remain (`:842-843`: "Nothing here picks one, which is why there is no
   branch that could"). Determinism is structural: every collection is a `BTreeMap`/
   `BTreeSet`/sorted `Vec` (`:581,741,852-863`); "Pure. No I/O, no clock, no randomness, no
   database" (`:1-8`); `tests/tests/clock_swap_invariance.rs` runs the merge corpus with
   clocks reversed.

**The one genuine gap.** The outbox idempotency key is
`SHA-256("{entity_type}:{entity_id}:{operation}:{sha256(payload)}")` (`outbox.rs:60-63`),
claimed via `INSERT ... ON CONFLICT (idempotency_key) DO NOTHING`
(`cairn-server/src/sync.rs:171-186`) against `sync_state`, whose primary key is the
idempotency key **alone** (`cairn-server/migrations/0001_init.sql:144-150`) — no writer
dimension. Two devices under one identity that independently produce a byte-identical
payload (a deterministic default applied on both) collide as `"duplicate"`. Not a
correctness bug today (payloads are identical), but 004 multiplies the devices writing into
one identity's stream, so D407's writer identity closes it by salting the key input.

**Ruled out, explicitly.** Vector clocks and HLC: absent anywhere in `crates/` (grep
confirms), and unjustified — immutability already solves content convergence.
Last-write-wins-on-timestamp: absent from the merge/derivation path, but four
**last-arrival** (not clock-compared) overwrites exist and are the anti-pattern 004 must not
extend to a fifth: `cairn-server/src/sync.rs:509-523` (`upsert_memory`), `criteria.rs:239-
242` (`import_criterion`), `criteria.rs:342-344` (`import_task`), `cairnd/src/sync.rs:1167-
1182` (`import_verification`, guarded only by "does this machine hold local runs",
`:1115-1122`, not a timestamp). D409's team-state CAS deliberately does not join this list.

---

## 4. Whether an older server can keep syncing projects

**Question**: 004 adds entity types and a server schema bump. Must not break project sync
against an unmigrated server. Does today's mechanism guarantee that?

**Today's mechanism.** One-way, additive advertisement, not a handshake —
`cairnd/src/sync.rs:786-790`: "A server that answers without `capabilities` is a server from
before the field existed, and its silence is the answer ... That is why there is no probe
endpoint and no negotiation." `GET /api/version` → `VersionPayload{schema_version,
capabilities}` (`cairn-server/src/version.rs:54-72`), `capabilities_for` (`:46-52`) →
`SCHEMA_2_CAPABILITIES` (`:32-38`) once `schema_version >= 2`. Inbound,
`reject_beyond_capability` (`cairn-server/src/sync.rs:257-293`) 409s with
`unknown_entity_type`/`unknown_field`, gated by `carries_meaning` (`:221-238`) so a
default-valued field never trips it:

```rust
fn carries_meaning(field: &str, value: &Value) -> bool {
    match field {
        "topic_key" | "value_key" => value.is_string(),
        "importance" => value.as_str().is_some_and(|s| s != "normal"),
        ...
        _ => !value.is_null(),
    }
}
```

Client-side, `codes::CAPABILITY_REFUSALS` (`wire.rs:178`) maps such codes to `blocked`
(retry later), everything else to `failed` (`cairnd/src/sync.rs:744-759`), and
`refresh_capability` (`:791-856`) requires **all** capabilities an entity type needs
(`ENTITY_CAPABILITIES`, `:871-879`) before releasing blocked work.

**The unstated assumption this rests on.** `run_worker` (`cairnd/src/sync.rs:39-119`) loops
every linked project in one process sharing a **single** `backoff`/`hit_transient`:

```rust
const BACKOFF_MIN: Duration = Duration::from_millis(500);   // sync.rs:20
const BACKOFF_MAX: Duration = Duration::from_millis(30_000); // sync.rs:21
...
if hit_transient { tokio::time::sleep(backoff).await; backoff = (backoff * 2).min(BACKOFF_MAX); }
else { backoff = BACKOFF_MIN; }
```

(`sync.rs:18-21,56-118`.) Under 003 this is correct because every project in one daemon
talks to the *same single server*, so one backoff variable truthfully describes "is the
server reachable." **004 breaks that precondition**: three sync namespaces (D426) can have
independently determined capability states against the same server — the ordinary case of a
server migrated for project sync but not yet for team knowledge. Without per-namespace
state, a capability-blocked namespace does not itself trigger `hit_transient` today only
because there has never been more than one relationship in the loop to observe the
difference. This is the load-bearing finding behind D427: the fix is not optional polish, it
is what makes "an older/partially-upgraded server must not break normal project sync" true
rather than aspirational once more than one namespace exists.

**Conclusion**: yes, but only with D427's per-namespace backoff/drain state — today's
backoff genuinely is process-global (`sync.rs:56-118`), and 004 is the first feature to put
more than one independently-failable relationship inside that loop.

---

## 5. Why background pull must change

**Question**: personal/team knowledge exists to reach quiet machines that are not pushing.
Does the background worker already pull in that case? **No.**

`cairnd/src/sync.rs:64-86`:

```rust
let (pending, _) = match outbox::counts(&daemon.store, project.id).await { Ok(c) => c, Err(_) => continue };
if pending == 0 {
    let blocked = outbox::blocked_count(&daemon.store, project.id).await.unwrap_or(0);
    if blocked == 0 || !probe_due { continue; }
    probed = true;
}
match drain(&daemon, project.id, server_project_id).await {
    Ok(...) => { ...; let _ = pull(&daemon, project.id, server_project_id).await; }
    ...
}
```

If nothing is pending and nothing is blocked, the loop `continue`s before `drain` or `pull`.
`pull` is reached only here (gated on a non-empty outbox having just drained) and from
`sync_now` (`sync.rs:623`, explicit `cairn sync now`). A linked project with an empty
outbox and nothing blocked — the steady state between a developer's own edits — **never
pulls in the background**.

Under 003 this is a minor lag (a quiet machine's project search lags until its own next
write or a manual `sync now`). Under 004 it is a correctness failure: team guidance exists
specifically for machines that did *not* author it. If personal/team namespaces inherit this
same consume-only trigger, a machine that never writes into a namespace can never receive
anything in it either — not slowly, but never, absent every participant manually running
`sync now` on a schedule nobody would reliably keep. **Conclusion**: 004 must schedule
`pull` on its own per-namespace interval, decoupled from "did this namespace's drain have
something to send."

---

## 6. How applicability can be deterministic without embeddings

**Question**: a record should surface in some projects, not others, without similarity
scoring (Constitution II rules out speculative infrastructure absent demonstrated need).

**The design.** Each record carries zero or more `(kind, value)` applicability facts;
`kind` is CHECK-constrained to `language | tool | topic` (D410 — **narrowed by D439 in §11:
`topic` is removed, because it cannot be derived from filenames and manifests without
reading semantic content, and the shipped vocabulary is `language | tool`**); `value` reuses the existing
`normalize_value_key` (`cairn-core/src/knowledge.rs:99-111`: NFC → lower → collapse
whitespace → ≤64 chars), further restricted to `[a-z0-9_]{1,64}`. No facts ⇒ universal, the
default (D411). Match rule (D412), no numeric component: applies iff, for every `kind`
present, at least one of its values is among the project's traits — AND across kinds, OR
within a kind. Traits (D413) come from a new local-only `project_traits` table, derived
deterministically from manifest/lockfile presence at link/refresh time (`Cargo.toml` ⇒
`rust`+`cargo`, `package.json` ⇒ `node`, etc.) — the same "derived from the repository, not
guessed" discipline Constitution VI already requires of Git state, extended one layer up.

**Why the closed vocabulary IS the privacy guard (D414).** A value validated against
`[a-z0-9_]{1,64}` cannot hold a path (needs `/`), a hostname, or a project name — the
restriction on what can be *represented* is the entire protection, with no separate
sanitization step to get wrong. This mirrors 003's own `topic_key`/`value_key` doctrine:
"unrepresentable input returns `None`, never an error" (`knowledge.rs:41-44`) — the
representable space being smaller than the input space is a safety property, not just
canonicalization.

**Why scoring/embeddings are rejected.** Constitution II lists "embeddings, graphs, decay
models, confidence engines" as out of scope "until real usage demonstrates need"; 004 is the
first feature needing applicability at all, so there is no accumulated evidence the
deterministic rule is insufficient. A set-membership test is also fully explainable by
inspection — consistent with `derive_subject` and `cairn verify` already being fully
explainable — where a similarity score would be Cairn's first unexplainable retrieval
signal.

---

## 7. Where the 003 contracts disagree with the code

**Question**: 004 extends 003's reconciliation/sync/privacy machinery directly. Do the
existing contracts describe it accurately? **No — eleven places disagree**, and 004 must
correct them (FR-531..FR-535) rather than extend an inaccurate description.

`contracts/privacy-sync.md`:

| # | Contract | Code |
|---|---|---|
| 1 | `:223-229` — `capability` block, 3 values | Field is **`capabilities`** (plural), **five** differently-spelled values (`cairn-server/src/version.rs:32-38`), string-matched exactly (`cairnd/src/sync.rs:871-879`). Contract's names match nothing. |
| 2 | `:140-142` — 6 forbidden entity types | **9** (`sync.rs:63-73`): the 6 plus `observation`, `observation_ref`, `criterion_evidence`. |
| 3 | `:131-138`/`:314` — 19 or "16" forbidden fields | **27** live (`sync.rs:21-56`), including `local_revision`, omitted from the contract's own code block. |
| 4 | `:78-85` omits `stale_at` | `memory_payload_for` sends it (`cairn-store/src/outbox.rs:544-547`). |
| 5 | `:109-111` — server gains `effective_from`, `superseded_at` | Columns exist (`cairn-server/migrations/0002:21-23`) but **nobody reads or writes them** — `upsert_memory` binds neither (`sync.rs:500-563`), `sync_changes` returns neither (`:680-725`); client sends all three, server drops them. |
| 6 | `:152-155` read-back = `{memories,relations,criteria,blockers,cursor}` | Also carries **`tasks`** (`sync.rs:742-754`); cites a "`§What crosses`" section that **does not exist**. |
| 7 | `:94-97` relation payload has `decided_at` | Push carries it (`outbox.rs:594-610`); **read-back omits it** (`sync.rs:839-847`) — asymmetric wire, harmless (never read back). |
| 8 | `:33-36` — local-only test list is complete | Test (`domain.rs:975-1012`) omits `memory_evidence`, `installed_resources`, `resource_bindings`, `capability_evidence`, `migration_states`, `recovery_artifacts`, `manager_integrations`, `agent_integrations`, `sync_deferred`, `sync_meta`, `users`. |
| 9 | `:5-8,313` — "an explicit field allowlist" | A **denylist over top-level keys**, non-recursive (`sync.rs:296-322`, §2). Allowlist is real only client-side. |
| 10 | `outbox.rs:3-5` — every change enqueues in the same transaction | Four sites enqueue **separately** after the mutation commits: session start (`repo.rs:329`→`:364-380`), a re-queue path (`repo.rs:842-855`), `supersede_memory` (`repo.rs:1297`→`:1300-1313`), link-time backfill (`cairnd/src/sync.rs:406-423,521-537`). |

`contracts/knowledge.md`:

| # | Contract | Code |
|---|---|---|
| 11 | `:166` — drops what `duplicates` points *to* | Drops the **`from`** (`knowledge.rs:665-667`: "the newer proposal points at the member it duplicates, so `from` is the one that drops out"). Direction is backwards. |
| 12 | `:171` — one member ⇒ Settled | `Reinforced` if it absorbed duplicates (`:782-786`). |
| 13 | `:167` — none remain ⇒ Historical | A supersession cycle emptying `remaining` ⇒ **Conflicted** with every active member as an answer (`:715-737`). |
| 14 | `:176-179` — several partitions recurse from step 1 | **No recursion**; `supersedes` applies once before partitioning, several partitions ⇒ `Conflicted` directly (`:841-871`). |
| 15 | `:190` — 6 verification tiers | **7**, `verified(remote_cairn)` at rank 1 (`:481-491`). |
| 16 | `:163-182` — the algorithm | Omits transitive duplicate-history propagation, self-relation filtering, lowest-id duplicate-cycle resolution, and the Kosaraju SCC machinery (`:625-997`). |
| 17 | `:156-157` — reads `pinned`, `importance` | `derive_subject` reads neither (grep of `:580-872`); both are read only by ranking elsewhere. |

And a **dead test**: `tests/tests/scope_audit.rs:376-398` splits `search.rs`'s source on the
literal `"fn scope_bucket"` — a function that does not exist (the bucket is an inline SQL
`CASE`, `search.rs:37-39`). The split returns `None`, `.unwrap_or_default()` (`:387`) yields
`""`, and all four `assert!(!bucket.contains(...))` checks pass against the empty string,
permanently, regardless of what `search.rs` actually ranks by. It would keep passing if
scope precedence were rewired to rank by `importance` tomorrow — indistinguishable in CI
from a test protecting something.

004 corrects all eleven contract disagreements and repairs the vacuous test as part of its
own work, not deferred: its own new mechanisms (sync namespaces, a new capability list,
reconciliation over two more record types) directly extend the exact machinery these items
misdescribe.

---

## 8. The constitution amendment

**Question**: Principle IV says "Memory is never global or ambient." 004's entire purpose
is memory that is not scoped to one project. Complexity Tracking exception, or does the
principle change?

Principle IV, verbatim (`.specify/memory/constitution.md:35-40`, v1.0.0, 2026-08-07):

> All durable knowledge carries explicit scope — project, branch, task, or session — and
> explicit provenance back to the session and observations that produced it. Memory is never
> global or ambient. Retrieval respects scope precedence, and any recalled item can be
> traced to where it came from and why it applies.

003's FR-391, verbatim (`specs/003-project-intelligence/spec.md:821-823`):

> Cairn MUST represent transferable knowledge as a **reusable pattern**, a record distinct
> from a memory. A project memory MUST NOT become a global memory, and no memory scope
> crossing projects may be introduced.

These directly conflict with D401–D428: "Memory is never global" and "a `Personal`/`Team`
record exists, has provenance, and is retrievable outside its originating project" cannot
both be true of the same system. Per the constitution's own Governance clause
(`constitution.md:89-96`): "When a requirement and a principle conflict, the conflict is
resolved in the spec — not silently in the implementation." Reinterpreting "global" narrowly
enough to evade the word, or leaving FR-391 standing while building what it forbids, are
both silent workarounds; the honest move is an amendment on the record.

**The v1.1.0 resolution (D-U1)**: refine Principle IV from "never global or ambient" to
"never *ambient*." **Ambient** — knowledge a retrieval surface admits without the record
itself declaring who it is for or where it came from — is the property actually worth
forbidding, and what "global" stood in for imprecisely. A **domain** (`Project|Personal|
Team`, D401) is not ambient: `Personal` carries an owner, a writer identity, and (once
promoted) a salted origin (D418); `Team` additionally carries proposer and ratifier
(D409/D-U3). Every field Principle IV already required of project memory has a direct
counterpart, narrowed by design where the narrowing itself is the privacy requirement
(session/observation identifiers become a writer id and an origin digest — raw identifiers
are exactly what D416's gate exists to strip).

**Why a domain is not a scope.** `MemoryScope` answers "how narrow within a project" — all
four values presuppose a project. `KnowledgeDomain` answers "whose knowledge is this, and
does a project even apply" — orthogonal, and a `Personal`/`Team` record has no `MemoryScope`
at all, no `project_id` column to omit, no scope column to misuse. This is why D402/D403
insist on separate tables rather than a `domain` column on `memories`: a column invites the
"just add a value" thinking §1 refutes at length; a separate table with no `project_id`
column makes the mistake unwritable, following `reusable_patterns`' own precedent.

A new principle — non-displacement — accompanies the amendment: personal/team knowledge
must never crowd out project knowledge in retrieval. Not a new invention: D420/D421's
Level-0 exclusion and 15%-of-remainder cap, elevated from implementation detail to
constitutional constraint so a later feature cannot erode it the way an unamended FR-391
would have been silently eroded.

FR-391's literal text is superseded in scope by this amendment note; every subsequent
decision that gives personal/team knowledge provenance, a closed vocabulary, and hard budget
limits is what "never ambient" means in practice.

---

## 9. User-level decisions (D-U1–D-U4)

Made before the investigation above began; recorded as given.

### D-U1 — Constitution amended to v1.1.0
**Decision**: Principle IV refined to "never ambient"; every record carries an explicit
domain; a new non-displacement principle; 003 FR-391 superseded in scope.
**Rationale**: §8 in full — FR-391 directly forbids what 004 builds; governance requires the
conflict resolved in the spec, not worked around in the implementation.
**Alternatives**: leave Principle IV unchanged, record 004 as a Complexity Tracking
exception — rejected; that mechanism is for a bounded exception to an otherwise-sound rule,
not a rule the feature's premise contradicts. Redefine "global" narrowly to exclude
personal/team — rejected as evasive.

### D-U2 — Security prerequisite ships separately, before 004
**Decision**: the four live authorization/data-loss holes ship as a hardening patch against
`main` before 004; 004 declares it a hard prerequisite and documents it in
`security-prerequisite.md`.
**Rationale**: these are live defects in shipped `v0.1.0-alpha.5`, independent of 004 — any
authenticated user can self-register, discover a project UUID via its public git remote,
self-join, and read/write everything (chain detailed in `security-prerequisite.md`). Building
knowledge domains meant to be trusted *more* broadly than project memory on top of an
authorization layer that cannot keep an uninvited user out at all would have the new,
wider-reaching knowledge inherit an existing hole. The fix is also nearly independent
engineering (identity/role/session lifecycle) from 004's knowledge model.
**Alternatives**: fix as part of 004's own tasks — rejected, conflates an urgent independent
fix with 004's review timeline. Ship 004 unpatched — rejected outright.

### D-U3 — Team Global Memory: member proposes, admin ratifies
**Decision**: a member's team entry lands `proposed`, invisible to recall; only an admin
ratification makes it authoritative.
**Rationale**: team knowledge is retrieved by every project every member works on — a larger
blast radius than any project today. `project_members` is deliberately flat with no role
("a user is a member or is not", Feature 001 FR-057, `0001_init.sql:48`) — there has never
been a finer-grained authority concept to reuse, so 004 introduces the minimum one. Mirrors
patterns' existing "explicit, never autonomous" promotion discipline (Feature 003 FR-395),
extended from one axis (trust state) to two (trust state and authorial reach).
**Alternatives**: any member's entry immediately authoritative — rejected, unilateral
server-wide policy power with no review. A voting/quorum model — rejected as speculative
infrastructure (Constitution II) with no demonstrated need.

### D-U4 — Deferrals and non-deferrals
**Decision**: deferred to Feature 005 — team/shared pattern sync; all Web UI administration
screens (004 ships CLI + server endpoints only). **Not** deferred — personal-global
cross-device sync; the handoff payload privacy repair.
**Rationale**: patterns already have a full, separate lifecycle; syncing them is a
project-sized effort D401–D428 never touches. Web UI is pure surface work with no
data-model dependency 004 needs, and building it before the API is exercised risks
designing around a shape that changes. Personal-global sync is *not* deferred because a
single-device personal record would be indistinguishable from existing `local_only` memory
— cross-device is the entire point of the domain. The handoff repair is not deferred because
§2 shows it is a live hole in data that crosses the wire *today*, unrelated to whether 004
ships.
**Alternatives**: ship only local `Personal` records — rejected, makes the domain a rename
of `local_only`. Defer the handoff repair alongside the UI work — rejected, unrelated
urgency.

---

## 10. Core architectural decisions (D401–D428)

### D401 — `KnowledgeDomain` is orthogonal to `MemoryScope`
**Decision**: `KnowledgeDomain: Project | Personal | Team` in `cairn-core`, orthogonal to
`MemoryScope` (untouched: four variants, CHECK unchanged, no rebuild, `resolve_scope`
untouched). Domain answers "whose knowledge"; scope answers "how narrow within a project."
**Rationale**: §1 in full. Keeping the axes orthogonal means every scope-precedence site
keeps compiling and behaving unchanged, and `scope_audit.rs:360-370` keeps passing
unmodified.
**Alternatives**: `MemoryScope::Global`/`::Personal`/`::Team` variants — rejected, §1.

### D402 — Two separate tables, not one table with a domain column
**Decision**: `personal_knowledge` and `team_knowledge` are separate tables, not one shared
table with a `domain` discriminator.
**Rationale**: a forgotten `WHERE domain = ?` on a shared table is a privacy breach; separate
tables make the mistake unwritable — the same habit behind `project_members`'s role-less
flatness and `reusable_patterns`' missing `project_id`. A cross-domain query against the
wrong table is a compile-time type error instead of a runtime information disclosure.
**Alternatives**: shared table + app-level checks — rejected per above. Postgres row-level
security — rejected as infrastructure with no present need (Constitution II); the two-table
split gets the same guarantee from the schema alone.

### D403 — Neither table has a `project_id` column
**Decision**: no `project_id` on either table, following `reusable_patterns` verbatim.
**Rationale**: `0005:236-238`'s own words restated for a new table: "a pattern that cannot
name a project cannot leak one." A column that is usually null is still joinable by
mistake; a column that does not exist is not. D418's salted origin digest gives what a
`project_id` would have given ("same project?") without what it must never give ("which").
**Alternatives**: nullable `project_id`, cleared on promotion — rejected, reintroduces the
forgettable-predicate problem D402 avoids, one column instead of one table.

### D404 — `reusable_patterns` stays a distinct type, untouched
**Decision**: patterns are not absorbed into the domain model; team pattern sync is
Feature 005 (D-U4).
**Rationale**: patterns already have a complete lifecycle (candidate→sanitized→validated→
contested) unrelated to domain; forcing them into `KnowledgeDomain` migrates a working
system for taxonomy tidiness alone.
**Alternatives**: unify under `KnowledgeDomain::Team` — rejected; patterns are keyed and
reasoned about by a shape (signal/root-cause digests, trust state, counterexamples) nothing
else in 004 shares.

### D405 — Personal/team records are immutable after creation
**Decision**: immutable like `memories`; the only `UPDATE` is a content-clearing tombstone.
**Rationale**: §3 — memory content is never updated after creation anywhere today (only
tombstones: `repo.rs:1501,1601`, `sync.rs:628`, `api.rs:713`), and that immutability is what
makes `derive_subject` clock-free. Personal/team inherit it for the same reason.
**Alternatives**: mutable content + `updated_at` LWW — rejected by §3's finding that nothing
like it exists outside four flagged exceptions the brief says not to extend.

### D406 — Divergence is relations + read-time derivation, reusing 003 unchanged
**Decision**: `classify_proposal` at write time, `derive_subject` at read time,
`representative_key` for a representative — no timestamp compared, no VC/HLC/LWW.
**Rationale**: §3 — this machinery was never scope-specific; `MemoryFacts`/`Relation` carry
no timestamp regardless of what table they describe. Duplicating it would be the "second
competing transactional model" 003's D43 already rejected event sourcing for.
**Alternatives**: a bespoke comparator for personal/team — rejected as unjustified
duplication of working, tested logic.

### D407 — Writer identity closes the idempotency-key collision, not a device registry
**Decision**: one opaque UUID per local store (`writer_identity`, one row, local-only),
mixed into the outbox idempotency key. Not a device registry — no server registration, no
device table, no lifecycle, no name. API tokens remain the per-device credential.
**Rationale**: §3 — `sync_state`'s PK is the idempotency key alone
(`0001_init.sql:144-150`), whose formula has no writer input; salting closes exactly that
gap without the device subsystem brief §7 excludes.
**Alternatives**: server-side device registry — rejected by brief §7. Salt with the API
token's hash — rejected; a token rotation would silently change every subsequent key and
defeat dedup across the rotation.

### D408 — `writer_seq`: gap detection only, never a tiebreaker
**Decision**: a per-writer monotonic counter, used only for gap detection/dedup within one
writer's stream; never compared across writers.
**Rationale**: mirrors `tasks.local_revision`'s established discipline exactly — "a monotone
counter for THIS store only" (`criteria.rs:19-24`), imports refuse to move it
(`:213-215`). Using it cross-writer would be LWW wearing a different name.
**Alternatives**: use it to break `Conflicted` ties — rejected, reintroduces last-write-wins
in spirit.

### D409 — Team state transitions use compare-and-swap
**Decision**: ratify/retire uses CAS on `expected_state`, reusing the
`expected_revision`/`blind_write` pattern from `crates/cairn-store/src/criteria.rs:174-189`.
A mismatch is rejected and reported, never silently applied.
**Rationale**: `check_revision` already gives exactly this guarantee for task criteria; team
ratification needs the same — an admin ratifying `proposed` must not silently clobber a
concurrent retirement.
**Alternatives**: unconditional transitions (last admin action wins) — rejected, it is the
last-arrival-wins anti-pattern §3 flags, applied to the one field 004 actually mutates.

### D410 — Applicability: closed-vocabulary set membership, not a score
**Decision**: `(kind, value)` facts; `kind` CHECK `language|tool|topic`; `value` via
`normalize_value_key` + `[a-z0-9_]{1,64}`.
**Rationale**: §6 in full.
**Alternatives**: free-text tags — rejected, reopens the arbitrary-string privacy surface
and produces non-deterministic matching. An extensible per-server vocabulary — rejected,
unjustified complexity for three kinds.

> Narrowed by D439 (§11): `topic` is removed from the vocabulary — the CHECK becomes
> `language|tool` only.

### D411 — No applicability facts means universal
**Decision**: a record with none applies everywhere; the default.
**Rationale**: most personal notes and team guidance are not tool-specific; requiring
enumeration would make the common case need the most input.
**Alternatives**: explicit `"all"` sentinel — rejected as ceremony; empty already means it.

### D412 — Match rule: AND across kinds, OR within a kind, no partial credit
**Decision**: applies iff every present `kind` has ≥1 matching value; match/no-match only.
**Rationale**: the natural reading of "Rust or Python, AND Docker"; a boolean output keeps
the rule inspectable, consistent with rejecting scoring in §6.
**Alternatives**: OR across kinds — rejected, would apply a `rust`+`docker` record to a
Python+Docker project, contrary to the author's evident intent.

### D413 — Project traits: derived from the working tree, local-only, never synced
**Decision**: new local-only `project_traits`, derived from manifest/lockfile presence at
link/refresh time.
**Rationale**: extends Constitution VI's "derived from Git, not guessed" one layer up; two
clones of the same repo derive identical traits with no coordination.
**Alternatives**: sync traits for server-side matching — rejected; matching happens locally
at recall time, and syncing traits adds a new entity type and privacy surface for no gain.

### D414 — The closed vocabulary IS the privacy guard
**Decision**: no separate redaction step for applicability; out-of-vocabulary values are
rejected at the gate.
**Rationale**: §6 — rejection at the gate is strictly stronger than sanitizing accepted
free text, since there is no "did the sanitizer miss a shape" question left to ask.
**Alternatives**: run values through `redact::redact` — rejected; redaction targets secret
*shapes*, not project-identifying shapes (names, paths), the actual risk here.

### D415 — Promotion is explicit-only, through a pure, fixed-order, fail-closed gate
**Decision**: modeled on the existing pattern-promotion gate (`cairn-store/src/
patterns.rs`); pure function over (content, topic_key, value_key, proposed applicability,
project traits, project identity) — no I/O, unit-testable.
**Rationale**: `patterns.rs:127`'s promote function already checks transferability, runs
redaction plus `contains_secret` on raw and redacted text (`:225-247`), and checks
`project_identifier` (`:360-380`), refusing rather than sanitizing on any hit
(`:249-259`). Reusing this shape keeps 004's promotion held to the same "safety is a fact
about what the function can see" standard as `derive_subject`.
**Alternatives**: confidence-threshold auto-promotion — rejected by Constitution II and by
003's FR-395 ("An agent or a developer proposes it; Cairn MUST NOT promote autonomously").
A trigger/background job — rejected, not independently unit-testable like a pure function.

### D416 — The gate's ten checks, fail-closed, first failure wins

> **Narrowed by D446 (§12).** Check 4 `project_identifying` moved out of the gate into the
> shared content validator, and the gate now fixes **eight** checks. The positional numbering
> rule recorded here is unchanged and is why check 6 keeps its slot despite refusing nothing
> (D452).
**Decision**: (1) source active; (2) has `topic_key`; (3) no absolute path/`~/`/drive
letter/`file://`/credentialed URL/env-assignment/long base64-or-hex; (4) does not name the
project (name, `git_common_dir` component, remote host/org/repo); (5) no evidence/
observation id travels (counts only); (6) verification resets to `unverified`; (7) every
applicability fact is in-vocabulary; (8) team promotion requires source-project membership
and lands `proposed`; (9) origin recorded only as a salted digest; (10) any unevaluable
check rejects.
**Rationale**: (3)/(4) directly extend `patterns.rs`'s existing `project_identifier` and
`redact`/`contains_secret` checks (`:225-259,360-380`) to a second target, same fixed-order,
first-failure-named structure. (10) is the fail-closed backstop — an ambiguous input is
never treated as passed, matching `patterns.rs`'s "refuse rather than fix" posture toward
secret-shaped content.
**Alternatives**: none of the ten were independently optional — drawn directly from
FR-393/FR-397's existing bans rather than invented; (6)/(8)/(9) below record the debated
ones.

> Narrowed by D433 (§11): this gate runs at promotion only, and two of the four entry points
> that can create global content never call it — D433 adds a shared validator that runs at
> all four.

### D417 — Verification resets to `unverified` on promotion
**Decision**: a promoted record's verification state is not carried forward.
**Rationale**: a deterministic check ran against *a specific project*; its authority does
not transfer to a project-independent claim. Extends 003's "a model's opinion is never
verification" one step further.
**Alternatives**: preserve state, tag with source project — rejected, requires exactly the
project-identifying detail D416 check (4) exists to strip.

### D418 — Origin is a salted digest, reusing the pattern's salting approach
**Decision**: origin = a digest of the source project identity salted per-machine, reusing
`crates/cairn-core/src/paths.rs:97-108`'s `origin_ref` approach — correlates same-project
promotions without naming the project.
**Rationale**: `paths.rs:97-108`'s own comment states the property verbatim: "a digest of
the source project salted with this value, which answers 'did these two patterns come from
the same project?' without answering 'which project?'" Per-machine salt, 0600, never
transmitted, so two machines never produce the same digest for the same project (FR-516).
**Alternatives**: no origin field — rejected, loses a genuinely useful, privacy-free signal
("this guidance has now been proposed from three projects").

> Finalized by D434 (§11): resolves the FR-516/T015 contradiction explicitly and supplies
> FR-516's replacement text; the local-only, per-machine-salted design recorded here is
> unchanged, now stated as the reason the contradiction does not apply.

### D419 — Structural privacy: no columns for what must never travel

> **Corrected by D448 (§12).** This record classified `writer_id`/`writer_seq` as
> never-transmitted by analogy to `tasks.local_revision`. The analogy was wrong — a writer
> sequence is useful only to a peer — and the classification was unsatisfiable against the
> `NOT NULL` local columns. Both fields now cross the wire and gain server columns.
> **Further narrowed by D452 (§12):** "no verification authority above `attested`" became
> "no verification field of any kind".
**Decision**: `personal_knowledge`/`team_knowledge` have no columns for project id,
evidence, observation ids, file paths, commands, or verification authority above
`attested`.
**Rationale**: restates 003's structural doctrine (`cairn-server/migrations/0002_
project_intelligence.sql:1-12`: "A record with nowhere to go cannot be sent by mistake") at
table-design time. §2's finding is that `handoffs` leaks *because its schema has columns
shaped to receive* observation-derived content, and the wire denylist doesn't recurse into
them — the lesson taken here is "give the new tables no such column," not "add a recursive
denylist."
**Alternatives**: include the columns, leave them null/empty by convention — rejected, a
column that exists is a column a future migration or careless write can populate; one that
does not exist cannot.

> Narrowed by D433 (§11): this record is the structural layer only (Layer A). The original
> FR-517 wording also claimed the free-text `content` column was incapable of carrying a
> path or command, which is false — D433 adds a validated layer (Layer B) for `content`.

### D420 — Global knowledge never enters the Level 0 reserve
**Decision**: enforced structurally — the global fetch is never called during reserve
computation. Level 0 stays project-only.
**Rationale**: `cairn-core/src/context.rs:8-53`'s Level 0/1/2 model exists to guarantee task/
repository/handoff continuity survives a tight budget (`HIGH_PRIORITY_SECTIONS`,
`context.rs:56`); a missing team convention is an inconvenience, missing task state is
disorientation. Structural exclusion (not in the function's input) cannot lose a budget
fight it was never entered into.
**Alternatives**: a very low priority number in the existing flat list — rejected; 003's own
model has no reserved floor below Level 0 (baseline B4), so low priority is merely *usually*
excluded, not guaranteed.

### D421 — Global sections capped at 15% of the post-reserve remainder
**Decision**: `global_share_max = 0.15` of total budget, applied to the Level 1 remainder;
if project sections fill it, global contributes nothing and the briefing is unchanged.
**Rationale**: extends D420's structural exclusion into the space global *is* allowed to
compete for, preventing it from crowding out `task_memory`/`branch_memory`/`project_memory`
— the non-displacement principle §8 adds.
**Alternatives**: no cap, priority order alone — rejected; a light-memory project with heavy
global knowledge could let global sections dominate Level 1.

### D422 — Section priority: personal before team
**Decision**: append `personal_notes > team_guidance` to the existing order (`task >
repository > previous_handoff > known_failures > decisions > task_memory > branch_memory >
project_memory > patterns`).
**Rationale**: preserves every existing relationship; personal-before-team keeps the
specificity gradient (task→branch→project→personal→team) monotone — a developer's own
relevant note typically beats general team convention for "what matters right now."
**Alternatives**: team before personal (reviewed > unreviewed) — rejected; review status is
about trustworthiness, not specificity, and the order ranks by the latter.

### D423 — `depth: "minimum"` excludes global sections entirely

> **Extended by D447 (§12).** The capability-refusal path recorded here (`409
> unknown_entity_type`, the `blocked` state, namespace backoff) is now one of *two* refusal
> paths. A server-side ingest refusal is permanent, must not enter `blocked`, and must not
> apply the namespace backoff, because retrying the same bytes can never succeed.
**Decision**: no global content at `depth: "minimum"`.
**Rationale**: `mcp.rs:116-138` already documents "minimum is Level 0 only," and D420
already places global outside Level 0 — this restates the interaction explicitly rather than
leaving it implied.
**Alternatives**: a small capped amount even at minimum — rejected, blurs the one guarantee
`minimum` currently makes for latency/size-sensitive callers.

### D424 — Domains stay separated: new sibling arrays, never merged
**Decision**: `cairn_search` gains sibling `personal[]`/`team[]`; `results[]` (project) and
`total` (project count) unchanged.
**Rationale**: exact precedent from `include_patterns`'s existing `patterns[]` — spliced in
separately (`cairnd/src/handlers.rs:2280-2338`) because "a pattern is not this project's
knowledge" (`wire.rs:1425-1430`). Reusing the shape means existing callers reading
`results[]`/`total` are unaffected.
**Alternatives**: a single merged, re-ranked array — rejected by D425 (scores aren't
comparable across corpora) and because it breaks existing callers' assumptions.

### D425 — Each domain gets its own FTS5 table, ranked within itself
**Decision**: `personal_fts`, `team_fts`, each ranked by the same BM25 expression, no
cross-domain comparator.
**Rationale**: mirrors `memory_fts` (`0002_memory_fts.sql:6-11`); BM25 scores are a function
of term statistics within one corpus and are not comparable across a 50-row and a
50,000-row corpus, so interleaving by score would be meaningless, not merely imprecise.
**Alternatives**: one shared FTS table with a `domain` column — rejected by D402 for the
same reason a shared knowledge table is rejected.

### D426 — Three sync namespaces with independent cursors
**Decision**: `project:<uuid>`, `personal:<user_uuid>`, `team:<server_instance_id>`; new
`sync_cursor(namespace TEXT PRIMARY KEY, pull_cursor TEXT)` replaces `sync_meta`, backfilled
to `project:<id>`.
**Rationale**: `sync_meta`'s `project_id TEXT PRIMARY KEY` (`0001_init.sql:180-184`) cannot
key personal/team cursors, which have no project (D403) — not a widening, a different key
space. The backfill is a pure rename, not a semantic change.
**Alternatives**: two more single-row tables alongside `sync_meta` — rejected, three tables
doing the identical job with three key shapes instead of one uniform one.

> Narrowed by D438 (§11): the `personal:` namespace key is (server instance, account), not
> user alone — `personal:<user_uuid>` above is imprecise; see FR-568.

### D427 — Backoff and drain state become per-namespace
**Decision**: replaces the process-global backoff at `cairnd/src/sync.rs:56-118`; a
capability-blocked namespace no longer throttles a healthy one.
**Rationale**: §4 in full — the load-bearing finding. Today's shared backoff is correct only
because one daemon has only ever had one server relationship; 004 puts three
independently-failable namespaces in the same loop.
**Alternatives**: keep global backoff, rely on `blocked`'s existing exclusion from the retry
path — rejected; that only covers the capability-block case, not one namespace hitting an
ordinary *transient* error while another is healthy.

### D428 — Compatibility reuses the one-way advertisement verbatim
**Decision**: two new capability names (`personal_knowledge`, `team_knowledge`) in
`SCHEMA_3_CAPABILITIES`; `ENTITY_CAPABILITIES` gains the two entity types; an old server's
409 maps to `blocked` exactly as today; `refresh_capability` releases it on upgrade.
**Rationale**: §4 — the additive-advertisement model already generalized from schema 1 to 2
without a handshake; the only genuinely new machinery §4 identifies is D427's per-namespace
backoff, not the advertisement mechanism itself.
**Alternatives**: a versioned handshake — rejected, the "negotiation" `sync.rs:786-790`
explicitly says was never built, for a problem additive advertisement already solves.

> Extended by D437 (§11): specifies the state machine and re-probe schedule that releases a
> blocked namespace — the missing transition between D428's "409 maps to `blocked`" and its
> "releases it on upgrade."

---

## 11. Repairs from design analysis (D433–D445)

Sections 1–10 above are the original research pass and are unchanged by what follows. A
`/speckit-analyze` pass against the finished design brief found twelve findings against
`spec.md`, `plan.md`, `tasks.md` and `traceability.md`; the repair addendum resolving them is
authoritative and is decision-recorded here as D433–D444, block-numbered to continue directly
from D428. Where one of these narrows or supersedes an earlier record in §10, a one-line
forward pointer is added at the original record; the original text is left standing.

### D429 — The environment-named account is the break-glass path

**Question**: `ensure_admin` re-applies the environment account on every process start
(`crates/cairn-server/src/auth.rs:131-176`, and its own doc comment at `:128-130` says this is
deliberate: "The environment is the source of truth, so this re-applies the password on every
start"). Once `role` and `status` exist, what does that upsert do with them?

**Decision**: its `ON CONFLICT` also sets `role = 'admin'` and `status = 'active'`. The account
is exempt from `must_change_password`. Demote and disable refuse to target it, naming
`CAIRN_ADMIN_EMAIL`. The resulting trust statement is documented rather than implied
(FR-539–FR-543).

**Rationale**: it is already the documented recovery path — "Naming the account in the
environment closes both gaps" (`auth.rs:125-126`). Without restoring authority as well as the
password, an operator who demotes or disables the last admin bricks administration with no
recovery. The exemption exists because the environment re-establishes the password every start,
so a forced change would be reverted on the next restart — an unbreakable loop. Refusing the
disable rather than warning about it is the honest choice: a change silently undone by a
restart is worse than a rejection. And the trust boundary must be written down, because
whoever can set the environment and restart the process can always obtain admin. That is
correct for a self-hosted server — they already control the host and the database — but it is
the outer boundary of the entire role model, not a footnote.

**Alternatives considered**: leaving `role`/`status` untouched by the upsert — rejected, it is
exactly the lockout case. Allowing the disable and warning — rejected, see above. Treating the
environment account as self-registration and forbidding it under FR-401 — rejected: it is
operator-configured through the server's own environment, not claimed by whoever reaches the
server first, which is the behaviour FR-401 actually targets.

**Relationship to D436/D445**: this guarantees never-zero-admins is *recoverable* at runtime.
D445 guarantees it is never *violated* by concurrent legal operations. Both are needed; neither
substitutes for the other.

### D431 — `team_knowledge_relations` exists

**Question**: the original table list named `personal_knowledge_relations` with no team
equivalent, while D406 requires reconciliation reuse in both domains. Was that deliberate?

**Decision**: no, it was an omission. `team_knowledge_relations` exists in both the local store
and the server, PK `(from_id, to_id, kind)`, the same six relation kinds.

**Rationale**: without it a superseded team entry has no link to its replacement, and two
contradicting authoritative entries are both presented as equally authoritative with no
conflict marker. `Duplicates` and `ConflictsWith` stay automatic per
`crates/cairn-core/src/domain.rs:393-395`; `Supersedes` on team knowledge is written by the
ratifying admin as a deliberate act rather than inferred, because superseding shared policy is
a curation decision and the admin is already in the loop at ratification. A standing conflict
between two authoritative entries is a signal *for* an admin and is never auto-resolved —
003's rule that a conflict may stand indefinitely applies unchanged.

**Invariant this forced into the design**: a relation may never link records in two different
domains. Cross-domain edges are the one construct capable of leaking a private personal note
into team-visible derivation, so the constraint is structural — each relation table references
its own domain's table only, and there is no table in which such an edge could be stored.

### D432 — No `project_members.added_at`

**Question**: the original entity list added both `added_by_user_id` and `added_at` to
`project_members`. Is the timestamp new information?

**Decision**: no. `project_members` already has
`created_at TIMESTAMPTZ NOT NULL DEFAULT now()` (`crates/cairn-server/migrations/0001_init.sql:52`).
Only `added_by_user_id` is added, nullable.

**Rationale**: a second timestamp on the same row answering the same question is a wart a
reviewer will rightly flag. Pre-existing rows backfill `added_by_user_id` to NULL rather than
to a guess, because who granted those memberships was never recorded — the same rule that left
003's `topic_key` and `stale_at` NULL rather than fabricating unrecorded state.

### D433 — Free-text content is validated, not structurally absent (Finding 1)

> **Extended by D446 and D447 (§12).** The validator's class list grew from seven to nine
> (`project_identifying`, `command_shaped`), its signature gained `project_identities`,
> applicability *values* are now validated as well as their kinds, and it runs at **five**
> entry points, not four — server-side synchronization ingest being the fifth. Layer A as
> stated here also over-claimed: see D452.
**Decision**: split the FR-517 privacy guarantee into two honestly distinct layers. Layer A
(structural): no column exists for a project identifier, an evidence reference, an
observation identifier, a file path, a command, or a verification authority above `attested`
(D419, restated as FR-517's replacement). Layer B (validated): the free-text `content`
column — which plainly exists and can hold any string, including a path or a command — is
checked by one shared pure validator, `validate_global_content(content, topic_key, value_key,
applicability) -> Result<(), GlobalContentRejection>`, run at all four entry points capable of
creating global content: direct personal creation, personal promotion, team proposal, team
promotion (FR-544–FR-550; FR-517 replacement).
**Rationale**: the original claim — that a personal/team record has "no field capable of
holding" a path or a command — was simply false. `content` is free text: D419 below (this
file, "no columns for project id, evidence, observation ids, file paths, commands, or
verification authority above attested") lists only what the table structurally lacks; it
never claimed the same of `content`, because it could not — a free-text column is exactly
what the record's substance lives in. The bypass this produced was real, not hypothetical:
D416's ten-check gate below runs only at promotion, and two of the four ways to create global
content — direct personal creation and team proposal — never call it, so content typed
straight into those two paths went unchecked by anything. A single shared pure validator, run
at all four entry points, closes exactly that gap, the same way D415's promotion gate is pure
and unit-testable rather than duplicated ad hoc at each call site.
**Alternatives**: route every creation through the existing promotion gate — rejected. The
promotion-only checks in D416 (source active, subject present, source project identity absent
from content, origin digest, evidence metadata stripped, verification reset — checks 1, 4, 5,
6, 9) need a project-memory source to check against; direct personal creation and team
proposal have no such source, so the gate cannot run unmodified on a path that was never
sourced from a project.

### D434 — Origin digest stays local-only; FR-516/T015 resolved, not chosen between (Finding 2)
**Decision**: the origin digest never leaves the machine that computes it. It is stored in a
local-only column, keyed by the existing machine-local salt already used for `origin_ref`
(D418, `paths.rs:97-108`), and is never transmitted (FR-516 replacement, FR-551, FR-552).
**Rationale**: the contradiction was real, not a documentation slip: FR-516 required two
promotions made from the same project to be recognizable as such, while T015 required two
machines to produce different digests for the same project. Both cannot hold of a value that
travels the wire and is compared centrally. Scoping FR-516's recognition to one machine
resolves the contradiction rather than picking a side, and is sufficient for the digest's
actual purpose (D418: "has this guidance now been proposed from three projects"). The
strongest argument for keeping it local is the enumeration one: the server already knows
every project identity linked to it, so a transmitted salted digest is exactly the kind of
value that party can brute-force — hash each known project identity with the observed salt
and compare. Keeping the digest off the wire entirely removes that attack rather than
mitigating it. The accepted cost is recorded, not hidden: two devices of the same user, each
with its own local salt, will never compute the same digest for the same source project, so
cross-device correlation does not happen (FR-552).

| Question | Answer |
|---|---|
| Two devices of the same user recognize the same source project? | No — accepted limitation, FR-552. |
| Team promotions from different users recognize the same source project? | No — different machines, different salts. |
| Local-only or server-visible? | Local-only; never transmitted (FR-551). |
| Where does the salt come from? | The existing machine-local salt already used for pattern `origin_ref` (D418, `paths.rs:97-108`) — no new key material. |
| Durable across machines? | No, deliberately. |
| Reversible or usable to enumerate project identities? | No — the server never receives it, so the party that knows every project id never holds a digest to test against; had it traveled, that same party could brute-force it over its own project list. |

**Alternatives**: transmit the digest so the server can widen recognition across a user's
devices — rejected, per the enumeration argument above. A per-user (rather than per-machine)
salt distributed out-of-band across a user's devices — rejected as new key-distribution
machinery of exactly the kind D407 already rejected a device registry for.

### D435 — Administrator password reset (Finding 3)
**Decision**: add an explicit admin-initiated password reset: it returns a new temporary
password exactly once, invalidates the old password and every API token immediately, and
places the account back into must-change-password state — but it MUST NOT re-enable a
disabled account (FR-553–FR-559, SC-442, SC-443).
**Rationale**: the spec referred to an administrator "resetting" a confined account, but no
such operation existed anywhere in the design. Revoking every token on reset is required
because a leaked or reused old credential must not survive the reset meant to end it.
Refusing to re-enable a disabled account on reset is the sharper point: reset and
enable/disable are two independent administrative decisions, and folding them together would
let a password operation silently restore access an administrator disabled for an unrelated
reason — the account owner performing a plausible, innocuous action (asking for their
password back) would have a side effect nobody who disabled the account authorized. FR-559's
carve-out for the environment-named bootstrap account exists for the same reason D436's
SC-433 replacement treats that account specially: its credential is re-established from the
environment on every daemon start, so a reset through the ordinary path would be silently
undone at the next restart.
**Alternatives**: allow reset to also re-enable — rejected, conflates two administrative
decisions per above. Fail the reset outright on a disabled account rather than succeeding-but-
staying-disabled — rejected; FR-558's design lets an administrator pre-stage a password for an
account they are about to re-enable separately, without a second round trip.

### D436 — Never-zero-admins enforced atomically, not by count-then-update (Finding 4)
**Decision**: the never-zero-admins guarantee is enforced within the single statement that
performs a demotion or disable, conditioned on another active administrator still existing —
never as a separate read-count followed by a write (FR-413 replacement, FR-560).
**Rationale**: a read-then-write implementation races. Concretely: administrators A and B are
the only two active admins remaining. An operator concurrently issues "demote A" and "demote
B". Each request independently counts active admins, sees two, and concludes its own
demotion is safe because one other admin will remain — each observes the *other* request's
target as still active, because neither has committed yet. Both proceed, and the server ends
with zero active administrators, the exact outcome the guarantee exists to prevent. Only a
single atomic statement — condition and mutation evaluated together, with no window between
them a concurrent request can slip through — closes this: whichever demotion's statement
commits first makes the other's precondition false, so it is refused, not silently applied
(FR-560).
**Alternatives**: a database-level advisory lock held across a count-then-update — rejected as
unjustified extra machinery when the same guarantee is available as a plain conditional
single-statement update, the same shape D409's CAS pattern (`criteria.rs:174-189`) already
uses for team state transitions elsewhere in this feature. Serialize in the application layer
(a process-local mutex) — rejected; it does not protect against two requests handled by
different connections/processes on the server.

### D437 — Capability re-probe closes the blocked → eligible gap (Finding 5)
**Decision**: while a namespace is blocked for want of a server capability, the client
re-probes the server's advertised capabilities (`GET /api/version`) on a bounded, backed-off
schedule; the probe is a capability read, never a retry of the held items, so held items are
still never sent to a server that has already refused them. When a probe observes the
capability, the namespace returns to eligible and the held entries release for delivery
preserving their original idempotency keys (FR-561–FR-563, SC-445).
**Rationale**: FR-499 said held items must never be retried against a server that cannot
accept them; FR-500 said they deliver automatically after an upgrade. Nothing in the original
design said what causes the transition between those two states. The fix distinguishes two
different things "retrying" could mean: retrying the *items themselves* (forbidden by FR-499,
still forbidden) versus probing the server's *capability list* (a cheap, side-effect-free
read FR-499 was never about). D428 below already has `refresh_capability` read `capabilities`
on every drain cycle for the ordinary upgrade case; D437 makes explicit that this same read is
what closes the blocked-namespace gap, and specifies the missing state machine: eligible →
blocked on a 409 `unknown_entity_type`; blocked → blocked while a probe still shows the
capability absent (bounded backoff, using D427's per-namespace state below); blocked →
eligible when a probe observes it, releasing held entries with their original idempotency
keys — the same preservation `compatibility.md` §2 step 6 already describes for the ordinary
case. See `compatibility.md` for the full state-machine specification added for this decision.
**Alternatives**: retry each held item on the same schedule as the probe, treating a failed
retry as the signal — rejected; this is exactly the FR-499 violation the finding exists to
prevent. Add a dedicated negotiation/probe endpoint instead of reusing `GET /api/version` —
rejected; D428 already established there is no probe endpoint and no negotiation, and
`GET /api/version` already carries everything needed.

### D438 — Personal knowledge partitions by (server, account); team stays server-bound (Finding 6)
**Decision**: team knowledge stays bound to the server instance that ratified it, refused
with a reported error when it arrives from a different instance. Personal knowledge is never
refused on server-instance grounds; the personal sync namespace is keyed by both the server
instance and the owning account, so a local store can hold more than one identity's personal
knowledge, partitioned, and recall surfaces only the currently-linked identity's own
(FR-496, FR-567, FR-568).
**Rationale**: T080 applied the same `server_instance_id` mismatch rejection to both domains,
but the spec only ever defined that boundary for team knowledge (`compatibility.md` §6; D426
below's `team:<server_instance_id>` namespace). Team knowledge is a server-wide artifact —
ratified by one server's admins, for that server's projects — so binding it to the ratifying
server is correct. Personal knowledge belongs to the user, not the server, so rejecting it on
server-instance grounds would directly contradict "personal knowledge follows the user"
(D-U4's stated reason personal sync is not deferred). The subtlety the original design missed:
user identity is per-server. The same human, authenticated to two different server instances,
is two different accounts — there is no cross-server user identity to merge them under.
Partitioning, not rejecting and not merging, is what makes "personal knowledge follows the
user" true without conflating two identities that only happen to belong to the same person;
FR-568's namespace key of (server instance, account) is what keeps them apart.
**Alternatives**: reject personal knowledge on server-instance mismatch, identically to team —
rejected, this is exactly T080's bug, and makes "personal follows the user" false the moment a
user's daemon talks to a second server. Merge personal knowledge across server instances for
the same human — rejected; there is no reliable cross-server identity to merge under, and
merging on a coincidental username match would leak one server's personal notes into a context
the user never wrote them in.

### D439 — `topic` removed from the applicability vocabulary (Finding 8)
**Decision**: the closed applicability vocabulary narrows from `language | tool | topic` to
`language | tool`. `ApplicabilityKind::Topic` is removed entirely (FR-569, FR-570).
**Rationale**: D413 below already commits project traits to deterministic derivation from
manifest and lockfile presence at link/refresh time — no semantic content is read, no model
runs. `language` and `tool` are derivable exactly that way (`Cargo.toml` ⇒ `rust`+`cargo`, and
so on). `topic` is not: no filename or manifest deterministically announces a project's
*subject matter* the way a lockfile announces its toolchain. Keeping a vocabulary member that
can never be populated by the derivation the design actually commits to is worse than not
having it, because D412's match rule (AND across kinds present) means a `topic`-tagged record
would silently fail to match *any* project — a record whose author added a topic fact meaning
to narrow its applicability would instead make it inapplicable everywhere, with no error and
no visible symptom short of the record simply never surfacing. This does not touch
`topic_key` on the record itself (D415's promotion-gate input, `knowledge.rs:252-258`'s
partition key) — that is the record's own subject key inherited from Feature 003 and is
unrelated to project-trait applicability; FR-570 requires the documents stop conflating the
two, which the shared name invited.
**Alternatives**: keep `topic` and populate it later once richer applicability (reading file
content) exists — rejected for 004's scope; deferred to Feature 005 with the rest of the
applicability work, rather than shipping a permanently-empty vocabulary member now. Derive
`topic` heuristically from directory names or README headings — rejected, contradicts D413's
"derived from the working tree, not guessed" commitment and Constitution VI.

### D440 — Temporary-password wording removes "retrieved later" (Finding 9)
**Decision**: account creation reveals the temporary password exactly once, in the creation
response itself; it is never stored in retrievable form and is never obtainable afterward by
any route, including by the creating administrator. The only way to obtain a new one is an
explicit admin password reset (D435) (FR-403 replacement, FR-407, FR-572–FR-573).
**Rationale**: the prior wording left room to imply a temporary password could be looked up
again later. FR-407/FR-572 make the credential's lifecycle explicit end-to-end — it
authenticates only to the password-change route while the account requires a change, and a
successful change invalidates it immediately — and FR-573 closes the loop D435 opened: reset
is now the only other way to get a new temporary credential, so the two decisions are
consistent with each other by construction.
**Alternatives**: allow a one-time re-display within a short grace window (e.g. the same admin
session) — rejected; "exactly once, in the creation response itself" is simpler to reason
about and to test than a time-boxed exception, and D435's reset path already exists for the
case this would have been trying to solve.

### D441 — `plan.md`'s phase narrative is regenerated from `tasks.md`, not the reverse (Finding 10)
**Decision**: `plan.md`'s eight-phase narrative is rewritten to match `tasks.md`'s ten-phase
structure, which is correct as-is.
**Rationale**: bookkeeping, but the direction matters. `tasks.md` reflects the dependency
ordering actually needed (prerequisite work first; shared schema, types and gates before the
stories that use them; accounts and roles before membership authorization; local personal
domain before personal sync; namespace sync before team sync reuses it; all three domains
complete before unified recall; independent repairs parallel where safe). Treating `plan.md`
as authoritative and back-porting its stale eight phases into `tasks.md` would discard
correct, more granular sequencing to preserve a narrative that was never updated.
**Alternatives**: renumber `tasks.md` to fit `plan.md`'s eight phases — rejected, loses the
finer-grained dependency ordering `tasks.md` already gets right, for no benefit beyond
matching the document that is actually the stale one.

### D442 — Requirement-to-test coverage audit (Finding 7)
**Decision**: every FR gets an implementing task and every SC gets a test task, with negative
requirements' test tasks written before or alongside their implementing task; the audit
specifically covers the weak cases the analysis named — invalid applicability on direct
personal creation (not only promotion), team applicability, zero-membership authoritative
team recall, role-filtered team listing, retired-record re-ratification refusal,
per-namespace unfinished-claim release at daemon start, immediate access loss for a removed
member, personal privacy across both users and projects, the idle consume-only background
pull, non-admin ratification refusal, authoritative-versus-retired visibility, the search
sibling-array invariance of the project result count, a forgotten or deleted source leaving
its promoted record untouched, post-upgrade exactly-once release, server-instance mismatch
for team versus partitioning for personal, a project with no derivable traits, full-budget
non-displacement with global records actually present, and `depth: "minimum"` observably
excluding global sections.
**Rationale**: the analysis's Finding 7 was that requirement and test coverage existed in
aggregate but several specific, easy-to-miss cases were not independently owned by any task —
several are exactly the cases where a passing test could hide a bug that only manifests on
the negative or edge path. D420/D421's non-displacement test is the clearest example: a test
that never populates global records cannot fail no matter how badly displacement is broken,
because an empty global store trivially never crowds out anything.
**Alternatives**: rely on the general "every FR needs a task" convention already in force,
without naming the weak cases explicitly — rejected; that convention already existed and is
exactly what let these particular gaps through unnoticed.

### D443 — `traceability.md` is regenerated, not hand-patched (Finding 11)
**Decision**: after every semantic change lands, `traceability.md` is regenerated from the
final `spec.md` by enumeration; a mismatch between them is build-blocking. Counts in
`tasks.md`, `plan.md`, `traceability.md` and the checklist are recomputed from the files
themselves, not carried forward by hand.
**Rationale**: bookkeeping, but load-bearing for every other repair in this section — D433
through D440 add and replace FR/SC ids across the spec. A `traceability.md` maintained by
hand-editing individual rows is exactly the kind of drift §7 above already found eleven
instances of between `contracts/` and the code; the fix for stale documentation is
regeneration from source, not another manual pass that will drift again the next time an id
changes.
**Alternatives**: patch only the rows affected by D433–D440 by hand — rejected; this is the
same manual-editing failure mode §7 already catalogs eleven instances of, applied to a
different document.

### D444 — Numbering conventions for this addendum (bookkeeping)
**Decision**: new FRs occupy FR-544–FR-573; new SCs occupy SC-438–SC-445. FR-403, FR-413,
FR-516, FR-517 and SC-433 are replaced in place, keeping their ids — no existing FR is
renumbered. These additions are appended to `spec.md`'s Requirements section as a "Repairs
from design analysis" block, grouped by the finding that produced them rather than by
subject, each cross-referenced back to the subject section it amends.
**Rationale**: bookkeeping. Preserving existing ids under replacement, rather than
renumbering, keeps every other document's existing FR/SC citations — including the ones
throughout §10 above, e.g. FR-516 at D418 — valid without a synchronized rename across every
file in the feature.
**Alternatives**: renumber replaced FRs to a fresh id and mark the old one deprecated —
rejected; strictly more churn across every citing document for the same outcome, and D443
already commits to regenerating derived documents rather than tracking a deprecation list.

---

### D445 — Never-zero-admins serializes on an advisory lock, not on EvalPlanQual (amends D436)

**Question**: D436 required the never-zero-admins guarantee to be atomic. Two atomic shapes
were proposed in different documents — a `FOR UPDATE`-locked CTE, and a conditional `UPDATE`
run at `SERIALIZABLE` isolation. Which is specified?

**Decision**: neither. Any transaction that could reduce the number of active administrators
takes `pg_advisory_xact_lock` on one fixed application-wide key **before** evaluating the
guard, and then applies the conditional `UPDATE ... WHERE EXISTS (another active admin)`.
The lock is transaction-scoped and released on commit or rollback (FR-574, SC-444).

**Rationale**: both rejected shapes are probably correct, and that "probably" is the problem.
The `FOR UPDATE` CTE relies on Postgres `EvalPlanQual` re-evaluating the locked set after the
blocking transaction commits — real behaviour under `READ COMMITTED`, but subtle,
version-sensitive, and reached through a CTE and an aggregate. The `SERIALIZABLE` shape relies
on serializable snapshot isolation detecting a read-write dependency between the two
transactions and aborting one, which also requires the API layer to catch a serialization
failure and translate it, and requires that endpoint to run at a different isolation level
from everything around it. Both make a safety invariant depend on an isolation-level argument
that a passing test could satisfy for the wrong reason.

The advisory lock removes the argument entirely: the second transaction blocks before it
reads anything, so the check and the write of two concurrent operations cannot interleave.
There is no isolation-level reasoning left to get wrong, and the failure mode is a plain
zero-row result rather than an exception the caller must classify.

**Alternatives considered**:
- *`FOR UPDATE`-locked CTE* — rejected above; retained in `migration.md` only as a documented
  alternative, never as the specified mechanism.
- *`SERIALIZABLE` isolation with retry* — rejected above; also the only option of the three
  that changes how callers must handle errors.
- *A partial unique index or constraint trigger enforcing "at least one admin"* — rejected
  because SQL has no clean way to express a minimum-cardinality constraint over a filtered
  set, and a trigger would run on every user write rather than only the rare ones that matter.
- *Accepting the race* — rejected: locking every administrator out of a running server is not
  a recoverable state through any supported API, which is precisely why the break-glass path
  in D429 exists and why it should stay a repair path rather than an expected one.

**Cost**: admin demotions and disables serialize globally. They are rare administrative acts,
not a hot path, so the contention is irrelevant.


## 12. Repairs from independent audit (D446–D458)

Sections 1–11 are unchanged by what follows. An independent audit of the repaired artifacts
found three CRITICAL and eight HIGH findings, and — more importantly than any individual
finding — established *why* the first repair round's traceability sweep certified a design that
still contained them. That sweep verified **citation coverage**: every FR named a task, every
task named an FR. Citation coverage cannot see that a validator's class list omits the very
check the requirement depends on. FR-546 cited the validator task; the validator task existed;
the validator's seven classes did not include the project-name screen that SC-438 demanded at
every entry point. Three artifacts agreed with each other and all three were wrong together.

The lesson is recorded here rather than only in the finding it produced: a requirement is
covered when a named mechanism can be pointed at, every call site reached, and a test written
that fails if the mechanism is removed. Anything less is a bibliography.

These decisions are block-numbered D446–D456, continuing from D445.

### D446 — The content validator needs nine classes and a project-identity input

**Question**: `validate_global_content` (D433) was specified with seven rejection classes:
`absolute_path`, `home_dir_ref`, `drive_letter_path`, `file_uri`, `credentialed_url`,
`env_assignment`, `encoded_secret_shape`. SC-438 requires that content naming a project, and
content carrying a shell command, be refused *identically at every entry point*. Neither class
is in the list. Where were those two checks actually performed?

**Decision**: the class list becomes **nine**, adding `project_identifying` and
`command_shaped`, and the signature gains a fifth parameter:

```rust
pub fn validate_global_content(
    content: &str,
    topic_key: Option<&str>,
    value_key: Option<&str>,
    applicability: &[ApplicabilityFact],
    project_identities: &[ProjectIdentity],
) -> Result<(), GlobalContentRejection>;
```

`project_identities` is the source project's identity tokens at a promotion, the current
project's tokens at a direct personal creation or a team proposal, and the union over the
pushing user's memberships at server-side ingest (D447). The promotion gate's check 4 is
**removed from the gate** and satisfied by delegation, exactly as check 1 already delegates,
dropping the gate from nine checks to eight. The validator becomes the only implementation of
these classes (FR-579). Applicability **values** are validated against the same nine classes
(FR-578) — the closed `language | tool` vocabulary constrains a fact's *kind*, and its *value*
was an unchecked open string.

**Rationale**: the project-name screen existed only as promotion-gate check 4, and the gate
runs at exactly one of the entry points. Direct personal creation and team proposal — the two
paths an agent is most likely to take, because they need no project memory to exist first —
had no project-name screen and no command screen at all. The design's own framing for the
shared validator was "no entry point can bypass the gate"; two of them bypassed two of its
most important checks, because those checks were never in it. Moving them in is not a new
mechanism, it is the mechanism the design already claimed to have.

Validating applicability values follows from the same reasoning one level down. An
applicability fact was treated as safe because its *kind* comes from a closed vocabulary, but
`tool = "acme-internal-deploy"` names a project as surely as any sentence does, and it travels
in a field nothing checked.

**The one deliberate hole, named rather than hidden**: an **empty** `project_identities` set
**passes** the `project_identifying` check (FR-580). This is the single documented exception to
the fail-closed rule of FR-549, and it is stated as an exception everywhere fail-closed is
stated. The reasoning is that a check with nothing to match is *vacuous*, not *unevaluable*,
and those are different things. Implementing it fail-closed refuses every global creation made
outside a linked project — which is the normal case for cross-project personal knowledge, and
would make the feature unusable in exactly the situation it exists for. A check that genuinely
cannot be evaluated, because a required input is structurally absent rather than merely empty,
still fails closed. The distinction is load-bearing and easy to get wrong in code, which is
why it is a named requirement with its own test rather than a comment.

**Alternatives considered**:
- *Leave the project-name check in the promotion gate and add a second copy to the other entry
  points* — rejected. Two implementations of one privacy rule drift, and the audit's own root
  cause was three artifacts agreeing with each other while all being wrong. FR-579 exists to
  forbid precisely this.
- *Constrain applicability values to a closed vocabulary too* — rejected. The set of language
  and tool names is open by nature; a closed list would be wrong within a release.
- *Fail closed on an empty identity set* — rejected above. Recorded because it is the choice a
  reviewer will reach for, and the reason against it is not obvious from the requirement text.

### D447 — Server-side ingest is the fifth entry point, because the client cannot be trusted

**Question**: the validator ran at four entry points, all client-side. What validates content
that arrives at the server from a client that did not run it?

**Decision**: server-side synchronization ingest is the **fifth** mandatory entry point
(FR-545, FR-577). It screens against the union of the pushing user's project memberships. A
refused item is not persisted, not partially persisted, and not acknowledged as delivered
(FR-581). The refusal carries a class only, never the offending content, and is **permanent** —
distinguishable by the client from a capability refusal, which is not.

**Rationale**: the previous design accepted client trust for a privacy boundary. A modified
client, an old client, or a client with a bug wrote unvalidated content straight into the
server store, from which it propagated to every other device the user owns. A privacy
guarantee that holds only when the client cooperates is not a guarantee; it is a convention.

The server cannot know which project the client was working in, which initially looked like a
reason it could not perform this check. It is not: the server knows every project that user
*could* have been in, and screening against the union is strictly stronger than screening
against one project. It also catches the case the client check structurally cannot — content
naming project X, written while working in project Y — because the client only ever holds the
identity of the project in front of it. The two checks layer; neither replaces the other.

Separating the refusals matters because they have opposite retry semantics. A capability
refusal (D423, `409 unknown_entity_type`, the `blocked` state) becomes retryable after a server
upgrade, so the design deliberately keeps the item queued and re-probes. An ingest refusal can
never succeed on retry of the same bytes; queueing it produces a namespace that retries forever
and throttles itself. So an ingest refusal must not enter `blocked` and must not apply the
namespace backoff.

**Alternatives considered**:
- *Trust the client and audit server-side asynchronously* — rejected. By the time an audit
  runs, the content has already reached the user's other devices, which is the harm.
- *Screen against the single project the client declares* — rejected. It requires trusting a
  client-supplied claim to decide what to check a client-supplied payload against.
- *Reject the whole push on one bad item* — rejected. One malformed record would stall an
  entire namespace, reintroducing the throttling problem from the other direction.

### D448 — Writer identity crosses the wire, because a local-only invariant was unsatisfiable

**Question**: `data-model.md` listed `personal_knowledge.writer_id`/`writer_seq` and their
`team_knowledge` counterparts as never-transmitted, while the local schema declares both
`NOT NULL` under `UNIQUE (writer_id, writer_seq)`. What does a store insert for two `NOT NULL`
columns when it pulls a record it did not write?

**Decision**: both fields **cross the wire** and gain columns in the Postgres schema, each with
the same `UNIQUE (writer_id, writer_seq)` constraint the local store carries (FR-582). The
`writer_identity` table stays never-transmitted. `writer_seq` is **diagnostic only** (FR-583):
no importer may consult it as an ordering key, a tiebreak, or a conflict-resolution input, and
this is asserted by its **absence from the reconciliation input type**, so a tiebreak
consulting it would not compile.

**Rationale**: the two statements were not both satisfiable, and the reflex repair — make the
local columns nullable — destroys the thing they exist for. FR-492 says a peer can detect a gap
in a writer's stream; a peer that cannot see the sequence cannot detect anything. The fields
were classified local-only by analogy to `tasks.local_revision`, and the analogy is wrong:
`local_revision` is meaningful only to its own store because nothing else acts on it, whereas
a writer sequence is *only* useful to someone else. Its whole purpose is to let a second party
notice that record 7 arrived and record 6 never did.

Enforcing the uniqueness constraint on the server as well as locally is the part worth stating.
The invariant could have been asserted client-side and hoped for on the server, which is the
same mistake as D447 in a different costume. Two enforcement points, one invariant.

The diagnostic-only rule reuses the discipline that keeps timestamps out of `MemoryFacts`
(D-U2, §10): the value is visible, and the function permitted to decide anything is not
allowed to see it. Determinism is enforced by what the function *can* see, not by a comment
asking it not to look. That is why the assertion is structural — a field the reconciliation
input does not have cannot be consulted by an importer that compiles.

**Alternatives considered**:
- *Nullable local columns* — rejected, destroys gap detection.
- *A separate transmitted "provenance" record alongside the untransmitted stamp* — rejected: a
  second record that must arrive with the first, for two integers already on the row.
- *Server columns without the unique constraint* — rejected. The constraint is the invariant;
  a column without it is a place to store a violation.

### D449 — Released Level 0 reserve is not available to global sections

**Question**: Feature 003 releases an unspent Level 0 reserve back into the general pool.
FR-474 caps global sections at a fraction of the budget "independent of how much space project
sections left unused". May global sections spend released reserve?

**Decision**: no (FR-584). Global allowance is computed from the **non-reserve pool only**.
SC-451 asserts global spend against the non-reserve pool alone, so the test fails if released
reserve is ever spent.

**Rationale**: a project with little critical state releases most of its 40% reserve. If global
sections may spend it, that project hands a large share of its briefing to project-independent
guidance — which is Principle VIII's failure mode ("Project Truth Is Not Displaceable") wearing
a budget's clothes rather than a scope's. The reserve exists to guarantee room for critical
project state; space it did not need is not thereby space that global defaults have earned. It
returns to the general pool for *project* ranked content, which is what a released reserve was
always for.

**Alternatives considered**: allowing it, capped — rejected; a cap on a growing base still
grows. Not releasing the reserve at all — rejected, that changes 003 behavior for callers who
have nothing to do with 004.

### D450 — The cap is `min(floor(total_budget * 0.15), remaining_non_reserve)`

**Question**: FR-474 says "a fixed fraction of the total context budget" and names no number.
Two implementations could both claim conformance, and no test could be written.

**Decision**: the fraction is **0.15**, taken against the **total** budget as FR-474 states,
and then bounded by D449's non-reserve pool. The two are a floor-of-two, not alternatives:
global spend is `min(floor(total_budget * 0.15), remaining_non_reserve)`.

**Rationale**: an unnamed constant in a requirement is an unimplementable requirement, and
`reserve_fraction = 0.40` and `CHARS_PER_TOKEN = 3.5` are already named constants in this
codebase — the precedent is to state the number. 0.15 against the default 3000-token budget is
450 tokens, enough for a handful of durable cross-project facts and not enough to crowd a
briefing. The interaction with D449 has to be stated as a minimum of two quantities rather than
as a single rule, because they bind in different situations: the fraction binds when the
non-reserve pool is large, and the pool binds when project content has nearly filled it.
Describing only one leaves the other's behavior undefined at exactly the boundary that matters.

### D451 — Exclusion reasons are explain-only, enforced by construction

**Question**: FR-478 requires per-item include/exclude reasons using 003's selection reason
vocabulary, and simultaneously requires that they never appear in the rendered briefing. Where
does "reported" mean?

**Decision**: reasons are produced solely on the diagnostic path and returned in the structured
explain output. The **rendered-briefing type carries no reason field** — enforced by
construction, not by a renderer that chooses to omit them. A test that adds a reason to the
rendered form fails.

**Rationale**: "produce it but do not render it" is a rule a renderer can forget. "The type has
nowhere to put it" is a rule that cannot be forgotten, and it is the same Layer A reasoning the
privacy boundary uses: a record with no column for a secret cannot carry one. As written, the
requirement was two obligations that could only be reconciled by a convention; as decided, it
is one type declaration.

### D452 — No verification field at all, not merely no authority above `attested`

**Question**: FR-513 and FR-517 said a promoted record carries no verification *authority above
`attested`*. That admits a field holding `attested`. Gate check 6 was named
`verification_reset`, implying a field to reset. Which is it?

**Decision**: a personal or team record has **no verification field of any kind** — not an
authority, not a state, not a timestamp. There is nothing to reset because there is nowhere to
hold a value. Check 6 keeps its position and its name for order stability (D416 numbers
positionally) and is documented as never refusing and resetting nothing. SC-422 asserts the
**stored and serialized** forms, field by field, in **both** stores, and a test that adds a
verification field fails.

**Rationale**: this is the Layer A/Layer B distinction applied to the field the audit found on
the wrong side of it. Principle VIII's supporting sentence is that "a deterministic check
performed against one project does not transfer its authority to a project-independent
assertion". A field holding `attested` is a place for exactly that transfer to happen, one
schema migration later. Keeping check 6 in the sequence while documenting that it refuses
nothing is deliberate: it marks the absence at the moment promotion occurs, where a reader of
the gate would otherwise wonder what happened to the source's verification state.

**Alternatives considered**: keeping a field pinned to `attested` — rejected above. Removing
check 6 from the sequence entirely — rejected; it renumbers the remaining checks for no gain
and removes the only place the absence is visible.

### D453 — An expired token is refused indistinguishably from a revoked one

**Question**: `expires_at` was added to `api_tokens` with no statement of what happens past it,
nor whether expiry is distinguishable from revocation.

**Decision**: refused, with a refusal **identical in status and body** to a revoked token's
(FR-585, SC-452).

**Rationale**: distinguishable refusals let a holder of a stale token learn that it was once
valid for this server, which is an oracle about the server's history and about the account.
Identical refusals cost nothing — no legitimate caller needs to know which of the two happened,
because the remedy is the same. Recorded again for the record, since it is the kind of thing a
later reviewer will flag: `api_tokens.token_hash` using the fast `cairn_core::digest` rather
than Argon2 is **correct** for a 32-byte CSPRNG token and must not be changed. Fast hashing
suits high-entropy secrets; Argon2 is for low-entropy passwords.

### D454 — The old-client / new-server direction

**Question**: `compatibility.md` covered a new client against an old server. The security
prerequisite removes `POST /api/auth/register` and `POST /api/projects/{id}/join`. What does a
pre-004 client see against a 004 server?

**Decision** (FR-586, FR-587, FR-588): project synchronization continues **unchanged** — only
the removed account and self-join routes stop functioning, and project sync must not degrade,
stall, or enter `blocked` because two unrelated routes are gone. A removed route answers with a
stable, documented status and a message naming its replacement rather than a bare not-found.
The release documents, in operator-facing terms, that self-registration and self-join are gone,
that accounts are administrator-created, and what an operator must do for users who relied on
either. The ingest-refusal case (D447) is documented alongside the capability-refusal case,
with the retry semantics that distinguish them.

**Rationale**: compatibility was analyzed in one direction because that is the direction 004
adds capabilities in. But the security prerequisite *removes* endpoints, and removal is a
compatibility event for every client that predates it. The bare-404 default is the specific
failure worth avoiding: an operator debugging a client that suddenly cannot create accounts
gets a status with no information, and the actual cause is a deliberate design decision
documented somewhere they are not looking.

### D455 — Nine duplicate requirement ids merged by deletion, never by alias

**Question**: nine ids restated a normative obligation already stated by another id. Merge how?

**Decision**: deleted outright — `FR-406`, `FR-433`, `FR-443`, `FR-494`, `FR-564`, `FR-565`,
`FR-566`, `FR-571`, `SC-446`. `FR-433` was deleted entirely, because nothing distinct remained
once its duplicated clause was stripped. `FR-459` was **kept**: it looked duplicative and
carried a distinct obligation. Every citation of a deleted id, in every artifact, is repointed
to the surviving id. Numbers are **not** reused and the remaining ids are **not** renumbered.

**Rationale**: an alias or a "see also" leaves two normative sentences in the document, and two
normative sentences about one obligation drift — one gets amended and the other does not, and
the next audit finds a contradiction that was manufactured by the repair. Not reusing numbers
follows 003's rule: an external reference to a retired id must resolve to "retired", never to a
different requirement, which is a far worse failure than a gap in a sequence.

### D456 — Four certifications re-opened, because they rested on citations

**Decision**: F6 (the never-transmitted table), F11 (the Layer A/B split), F12 ("all entry
points"), and E3 (the `estimated_tokens <= budget` invariant) are re-opened and re-derived
against mechanisms rather than re-asserted.

**Rationale**: each was certified by the sweep this section opens by criticizing. F6 was wrong
about `writer_id`/`writer_seq` (D448) — it was checked against the prose describing the
serialized forms rather than against the forms. F11 claimed Layer A for fields that are Layer B.
F12's count changed from four to five (D447), and a count is exactly the kind of claim that
survives a citation check unexamined. E3 must be re-verified because D449 and D450 both change
what the global sections may spend, and an invariant proven under one budget rule is not proven
under another. The point is general: a re-derivation must go to the mechanism, because the
artifacts already agree with each other.


### D457 — Seventeen requirements had no success criterion, found by the semantic pass itself

**Question**: the semantic traceability pass this section demands asks, for every
requirement, what test fails if the mechanism is removed. Run against the repaired spec, what
did it find?

**Decision**: seventeen requirements had no criterion at all, and `spec.md` gained
`SC-453`–`SC-469` rather than being certified around them. Two existing criteria were also
amended, because they under-specified what they verified. The pass ran in two waves — the first
over the requirements this audit round added, the second over the whole set — and the second
wave found more than the first, which is the finding that matters most.

**Wave one — requirements this round wrote:**

| Requirement | Was | Now |
|---|---|---|
| FR-579 — the validator is the only implementation of its classes | no criterion | `SC-453`, and specifically an audit that **fails when a second implementation is introduced** rather than one that inspects the code as it stands |
| FR-580 — an empty identity set passes | no criterion | `SC-454`, asserting the vacuous case and the unevaluable case **separately** |
| FR-583 — `writer_seq` is diagnostic only | no criterion | `SC-455`, permuting sequences over one corpus and asserting identical derived output |
| FR-581 — an ingest refusal is distinguishable from a capability refusal | only the storage half, via `SC-449` | `SC-456`, adding the distinguishability half, and requiring it without inspecting a message string |
| FR-586 — a pre-004 client keeps syncing projects | no criterion | `SC-457`, against a **real** pre-004 binary |
| FR-587, FR-588 — removed-route response and operator documentation | no criterion | `SC-458` |
| `SC-421` | a corpus of six classes | all **nine**, and a class added to the validator without a corpus entry leaves the criterion unmet |
| `SC-424` | "or a verification claim" | "verification of any kind — not an authority, not a state, not a timestamp" |

**Wave two — the pre-existing requirement set, swept the same way:**

| Requirement | The obligation with no test behind it | Now |
|---|---|---|
| FR-521 — no change to `MemoryScope` or its stored representation | *the feature's central constraint*, and nothing verified it | `SC-459`, asserting the four-variant list and the `CHECK` text so a fifth variant fails |
| FR-455 — the agent surface cannot create a team entry directly; FR-515 — promotion always lands proposed | `SC-414` covers a non-admin's *ratification* being refused, which is a different act | `SC-460`, exercising every action the six tools expose and asserting the resulting state |
| FR-506 — Cairn must never promote automatically | no criterion | `SC-461`, asserting the global record count is unchanged across a seeded workload |
| FR-476 — personal considered ahead of team | no criterion | `SC-462`, the case where only one of the two fits |
| FR-478 — reasons reported on the diagnostic path, absent from the briefing | no criterion | `SC-463`, inspecting the rendered form field by field |
| FR-482 — an importance hint changes neither precedence nor reserve admission | no criterion | `SC-464`, byte-identical context across every hint value |
| FR-418 — no route through which a user adds themselves to a project | `SC-401` covers the *account* analogue, not this one | `SC-465`, exercising every route and asserting membership is unchanged |
| FR-462 — two disagreeing authoritative team entries both stay visible | `SC-411` covers the personal case only | `SC-466`, including that ratification order changes nothing |
| FR-550 — documentation names the mechanism behind every privacy guarantee | no criterion, and this is the requirement that exists *because* the documentation lied once | `SC-467`, an audit that fails on the forbidden phrasing rather than a review that might not |
| FR-471 — a score from one domain's index is never compared against another's | no criterion | `SC-468`, by construction: distinct ranking input types, so the comparison would not compile |
| FR-438 — project traits stay local and are never synchronized | no criterion, though it is a privacy requirement | `SC-469`, inspecting the wire across projects with all-distinct traits |

**Rationale**: every one of these seventeen requirements had a task. Six are `MUST`/`MUST NOT`
sentences added by this very audit round, which is the uncomfortable part — a repair round can
introduce the same defect it was convened to fix, because a newly written requirement is the one
least likely to have a criterion yet. `SC-421` is the clearest illustration of why citation
coverage misses this: it named six classes while `FR-546` declared nine, and both were cited by
the same task, so every cross-reference resolved. Only reading the two texts against each other
shows that three classes had no corpus entry.

The eleven from wave two are worse, because they had survived three prior passes. `FR-521` is
the one to dwell on: *"Do not add `MemoryScope::Global`"* is the constraint the entire feature
was designed around, the plan's Summary calls it "the one decision" everything turns on, and no
success criterion asserted it. A task tested it (`T004`), and a task is not a criterion — a task
can be reworded, deferred, or marked done by someone who ran a different assertion than the one
intended. `FR-550` is the second: it was written *because* the documentation had claimed a
free-text field was structurally incapable of holding a path, and it was given no test, so the
requirement created to stop a false guarantee could itself have been quietly unmet.

Three of the eleven are verified **by construction** rather than by assertion — `SC-459`'s
variant list, `SC-468`'s distinct ranking types, and `SC-455`'s absent sequence field. That is
the preferred shape wherever it is reachable, for the reason Principle V gives: a comparison
that would not compile cannot be reintroduced by someone who did not read the requirement.

Two of the new criteria are worth their wording rather than their existence.

`SC-453` could have been written as "verify no other component implements these classes",
which would pass today and pass again the day after a duplicate is added — Feature 003's
`scope_audit.rs` failed in exactly that way, splitting on a string that did not exist and
asserting against `""`. The criterion instead requires an audit that fails when a second
implementation is introduced, which is a statement about the test's own falsifiability rather
than about the code.

`SC-456` requires the two refusal kinds to be distinguishable *without inspecting a message
string*, which forces the distinction into a typed field. A client that classifies a refusal by
substring-matching an error message works until the message is reworded, and the failure is
silent: a permanent refusal starts being retried forever, or a retryable one is discarded.

**Deliberately left without a dedicated criterion.** Eight negative requirements are covered by
an acceptance scenario and a task but by no success criterion of their own, and the honest
record is that this was a judgment call rather than a clean result: FR-415 (a server instance
identity is never reassigned), FR-428 (an explicit project argument is never overridden by
auto-link), FR-441 (a forgotten personal entry stops appearing), FR-461 (team knowledge is
immutable and retiring does not alter content), FR-465 (retirement is not reversible by
re-ratifying the same record), FR-520 (a promotion refusal is synchronous), FR-559 (a password
reset is refused for the environment-named account), and FR-542 (the environment account is not
self-registration). Each is a real obligation; each would be caught by the acceptance scenario
that covers it; none is a privacy or authorization boundary. They are listed here so the next
reviewer inherits the list rather than rediscovering it, and so that "every requirement has a
criterion" is not claimed when it is not true.

**Alternatives considered**: folding each into an adjacent existing criterion — rejected for
most, because the adjacent criterion verified a different obligation and widening it would have
made a passing test ambiguous about which requirement it protected. `FR-587` and `FR-588` *were*
folded together into `SC-458`, because they describe one operator-visible event: a removed route
and the documentation that explains it, and `FR-455` and `FR-515` into `SC-460` for the same
reason. Renumbering the criteria into a contiguous block was rejected on D455's rule: numbers
are never reused and never shuffled.


### D458 — Two of the four "open, non-blocking" checklist items were defects

**Question**: an adversarial readiness pass was asked to inspect CHK007, CHK009, CHK018 and
CHK027 individually rather than accept them as recorded open questions. Were any of them a
design decision, a security property, a protocol invariant, or an acceptance criterion in
disguise?

**Decision**: two were, and both are now closed.

**CHK009 — an acceptance criterion with no referent, and an unbounded poll behind it.**
`SC-412` read "within the documented background interval" and no document stated an interval.
This was recorded twice as a probable spec/plan split, alongside CHK010; CHK010 turned out to be
an unimplementable requirement for exactly the same reason and was closed by pinning `0.15`
(D450). CHK009 is worse than its twin, because the missing number was not merely unstated — it
was *argued against*. `sync-namespaces.md` §5 said the pull-due timer needed no new constant,
since `WORKER_TICK = 500ms` already provides a cadence and the pull call only needs moving
outside the `pending == 0` short-circuit. Follow that literally: with no interval of its own,
"the pull-due timer has elapsed" is true on every tick, so each namespace pulls twice a second,
three namespaces per machine, indefinitely, whether or not anything changed. Backoff does not
contain it, because backoff engages on failure and these requests succeed. `FR-589` now requires
a stated bound, `PULL_INTERVAL_SECONDS = 30` is that bound, `WORKER_TICK` is demoted to the check
cadence, and `SC-412` asserts 60 seconds — twice the interval, so a passing test does not depend
on landing inside one window.

**CHK018 — a credential-lifetime rule that existed only as a contract paragraph.** Whether
re-enabling a disabled account restores its revoked tokens was decided correctly in
`identity-administration.md` §6 ("`revoked_at` is not cleared"), stated in that contract's
invariant 4, and covered by no requirement and no criterion. The property therefore held by the
intention of whoever wrote the paragraph. That is not enough for a rule about credential
lifetime: an implementer clearing `revoked_at` alongside `status` is making a plausible-looking
change that resurrects every token an account held before it was disabled, and every existing
test still passes, because they all assert the *disable* side. `FR-590` and `SC-470` close it,
and `SC-470` asserts the re-enable side **separately** from `SC-404`'s disable side for the same
reason `SC-436` is separate from `SC-404`: a regression in either must not be maskable by the
other.

**CHK007 and CHK027 were not defects**, and the reason is worth recording so the next pass does
not re-litigate them.

CHK007 asked whether a project's membership falling to zero needs the treatment a zero-admin
server gets. It does not, because the two states differ in recoverability rather than in
severity. A zero-admin server is unrecoverable through any supported API, which is why `FR-413`
enforces a floor atomically and why the break-glass path of D429 exists. A zero-member project
is recoverable by design: `FR-419` says "an existing member **or an admin**" may add a user, and
`project-authorization.md` §2 states that a server admin bypasses the membership check on that
route specifically so membership can be bootstrapped on a project with none left. The asymmetry
is deliberate and the mechanism that makes it safe is already required.

CHK027 asked whether the universal-default rule carries through the promotion path or is only
stated for direct creation. It carries, because `FR-435` and `FR-460` are stated about the
*record* — "a personal knowledge entry with no applicability facts MUST apply to every project"
— not about the path that created it. A promoted entry with no applicability facts is an entry
with no applicability facts. `FR-514` adds a promotion-specific obligation (validate the
proposed vocabulary) without displacing the record-level default. No restatement is needed, and
adding one would create a second normative sentence about one obligation, which D455 exists to
prevent.

**Rationale for recording all four rather than only the two that changed**: an item marked open
with a parenthetical reading "confirm this asymmetry is deliberate" is indistinguishable, to a
later reader, from an item that was actually confirmed. Two of these four had carried such a
parenthetical through three review passes while concealing a defect. The disposition, not the
open mark, is what the next reviewer needs.


## Appendix — facts carried forward from 003, relied on throughout

- The four-scope enum is pinned by a CHECK that cannot be widened without a `memories`
  rebuild, three `ELSE 3` SQL CASEs, one exhaustive match, and two assertions
  (`scope_audit.rs:360-370`, `domain.rs:962-966`). (§1)
- The precedent for project-independent knowledge is `reusable_patterns` — no `project_id`
  column at all (`0005:236-238`) — not a scope variant. (§1, D401–D403)
- The privacy boundary's strongest guarantee is `OutboxEntityType` plus the `outbox.
  entity_type` CHECK; its weakest link is the server's top-level-only denylist and the
  handoff payload, which carries observation summaries, commands, and absolute paths under
  names the denylist never sees. (§2, D419)
- No version, vector clock, HLC, per-writer sequence, or device identity exists anywhere
  today. Convergence rests on immutable content, UUID identity, normalized relation PKs, and
  a clock-free derivation. (§3, D405–D408)
- Compatibility is a one-way, additive advertisement, never a handshake; today's single
  process-global backoff is correct only because there has only ever been one server
  relationship per daemon — until 004. (§4, D427–D428)
- The background worker only pulls after a drain that had something to push; a quiet,
  linked project never pulls in the background today. (§5)
- Applicability is deterministic because it is closed-vocabulary set membership over
  deterministically-derived traits — no score, no ranking contribution — and the same
  vocabulary is the privacy guard. (§6, D410–D414)
- Eleven contract-vs-code disagreements exist plus one vacuous test; 004 corrects all of
  them as FR-531..FR-535. (§7)
- The original FR-517 stated a privacy guarantee that was false — free-text `content` can
  hold anything, and two of the four creation paths never ran the promotion gate at all.
  §11's D433 splits the guarantee into what no column can hold and what a shared validator
  checks. (§11, D433)
- Principle IV is amended to v1.1.0 — "never ambient," not "never global" — because 003's
  FR-391 directly conflicts with 004, and governance requires that conflict resolved in the
  spec. A domain is not a scope: scope is "how narrow within a project," domain is "whose
  knowledge, and does a project even apply." (§8, D-U1)
