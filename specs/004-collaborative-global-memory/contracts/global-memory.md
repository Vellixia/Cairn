# Contract: Personal and Team Global Memory

**Feature**: `004-collaborative-global-memory`

This contract guarantees that personal and team knowledge are first-class knowledge types
distinct from project memory's scope system, that neither can ever leak a project identity
because neither has anywhere to put one, that a personal or team record is exactly as
immutable and exactly as deterministically reconciled as project memory already is, that
applicability is a closed, auditable predicate rather than a heuristic, and that team
knowledge cannot become visible to anyone until an administrator explicitly ratifies it.

## 1. `KnowledgeDomain` — orthogonal to `MemoryScope`

`KnowledgeDomain` is a new `cairn-core` enum, three variants: `Project | Personal | Team`.
It answers **whose** knowledge a record is. `MemoryScope` — `Project | Branch | Task |
Session` (`crates/cairn-core/src/domain.rs:112-120`) — answers **how narrow inside a
project** a piece of project knowledge is. The two questions are independent:

| | Answers | Variants | Column | Table |
|---|---|---|---|---|
| `KnowledgeDomain` | Whose knowledge is this | 3: project, personal, team | *(not stored on `memories` — expressed by which table a row lives in, see §2)* | — |
| `MemoryScope` | How narrow, inside a project | 4: project, branch, task, session | `memories.scope`, `CHECK` | `memories` only |

**`MemoryScope` keeps exactly four variants. This feature adds no fifth.** The prior-feature
inspection established why a fifth variant is structurally expensive: SQLite cannot widen a
`CHECK` in place (`0001_init.sql:105-106`; the migration that *could* have done it explicitly
declined, `0005_project_intelligence.sql:10-15`), the scope-bucket ranking is duplicated as
three hand-written SQL `CASE … ELSE 3` expressions (`crates/cairn-store/src/search.rs:37-39,
251-253`; `crates/cairn-server/src/api.rs:628-629`), and `derive_subject` partitions by
`(project_id, scope, scope_key, topic_key)` — a partition key that has no meaning for a
record with no project at all. Personal and team knowledge is therefore not "a fifth scope";
it is a different *domain*, living in its own tables, that never touches the `memories.scope`
column or any of the code that matches on it.

## 2. Two tables, not one column

Personal and team knowledge live in **two separate tables** —
`personal_knowledge` and `team_knowledge` — not one shared table with a `domain`
discriminator column. This is a deliberate rejection of the more "normalized" design:

> A forgotten `WHERE domain = ?` in one shared table is a privacy breach waiting to happen.
> Two tables make that mistake **unwritable** — there is no query that can accidentally read
> another user's personal row while intending to read the caller's own, because there is no
> single table where both rows coexist.

This is the same discipline `reusable_patterns` already applies at the column level ("a
pattern that cannot name a project cannot leak one," `0005_project_intelligence.sql:236-238`)
extended to the table level: a personal row and a team row are not merely filtered apart,
they are *stored* apart.

### `personal_knowledge`

| Column | Notes |
|---|---|
| `id` | PK |
| `owner_user_id` | The one user this record belongs to, on the server that minted that user's account. Never null, never reassigned. Local rows are partitioned by the owning identity — a local store may hold rows for more than one `(server_instance_id, owner_user_id)` pair; recall surfaces only the currently linked identity's rows (D438, `sync-namespaces.md` §10). |
| `knowledge_type` | Same vocabulary as `memories.type` (`fact\|decision\|convention\|failure\|procedure`) |
| `content` | Free text |
| `topic_key`, `value_key` | Same normalization as project memory (§6) |
| `content_norm_digest` | Local-only index, never transmitted — same rule as `memories.content_norm_digest` |
| `origin_digest` | Present only when this row was created by promotion (`promotion-privacy.md`); `NULL` for a directly authored entry |
| `writer_id`, `writer_seq` | See `sync-namespaces.md` §3 |
| `created_at` | |
| `superseded_by_id` | Set only by an explicit relation, never by an in-place update |
| `forgotten_at` | Tombstone timestamp; content is cleared when set (§3) |

### `personal_knowledge_applicability`

`(personal_id, kind, value)`, primary key on the triple, `kind CHECK ('language','tool')`
(D439 — the vocabulary closes at two kinds; see §4). Zero rows for a given `personal_id`
means universal (§4).

### `personal_knowledge_relations`

`(from_id, to_id, kind)`, the same six relation kinds as `memory_relations`
(`crates/cairn-core/src/domain.rs:357-375`), scoped to personal knowledge only — a personal
relation never points at a project memory or a team entry.

### `team_knowledge`

| Column | Notes |
|---|---|
| `id` | PK |
| `knowledge_type`, `content`, `topic_key`, `value_key`, `content_norm_digest`, `origin_digest` | Same meaning as the personal table |
| `state` | `CHECK ('proposed','authoritative','retired')` — §5 |
| `proposed_by_user_id` | Traceable reference, never project-identifying content (FR-459) |
| `ratified_by_user_id`, `ratified_at` | Set only by the `proposed → authoritative` transition |
| `writer_id`, `writer_seq` | Same meaning as personal |
| `retired_by_user_id`, `retired_at` | Set only by the `authoritative → retired` transition (FR-457) |
| `created_at`, `superseded_by_id` | |

`team_knowledge_applicability` mirrors the personal table, keyed `(team_id, kind, value)`.

`team_knowledge_relations` — **D431**, correcting an omission in the original design brief,
which named `personal_knowledge_relations` but omitted a team equivalent. Without it, a
superseded team entry would have no link to its replacement, and two contradicting
authoritative entries would both read as equally authoritative with no conflict marker at
all — exactly the gap D406's "reconciliation reuse in both domains" exists to close. Same
shape as `personal_knowledge_relations`: `(from_id, to_id, kind)` primary key, in the local
SQLite store **and** in server Postgres (team knowledge is server-wide and therefore
synchronized, unlike `personal_knowledge_relations`, which syncs only within one identity's
own `personal:<server_instance_id>:<user_uuid>` namespace — see `sync-namespaces.md` §1, §6
and §12). The same six
relation kinds as `memory_relations` (`crates/cairn-core/src/domain.rs:357-375`).

Neither table has a `project_id` column, an evidence-fact reference, an observation
identifier, or a verification field **of any kind** — not an authority, not a state, not a
timestamp (FR-513, FR-517, D452). The earlier wording here said "no verification authority above
`attested`", which still admitted a column holding `attested`, and a column holding `attested` is
a place for one project's deterministic check to become a project-independent claim one migration
later. There is nothing to reset because there is nowhere to hold a value; `SC-422` and `SC-424`
assert this field by field against the **stored and serialized** forms in both stores, so adding
such a column fails a test rather than passing review.

A file path and a command are a **different** guarantee and must not be listed alongside those.
They are not absent columns: they are values that free text can hold. `content`, `topic_key`,
`value_key` and every applicability *value* are free text, kept clean by
`validate_global_content` at all five entry points, not by column absence (FR-550). See
`promotion-privacy.md` §2b for the two-layer split stated in full; it applies to every row in
these tables regardless of whether it arrived by direct creation, by promotion, or by
synchronization from another device.

## 3. Immutability, and the one permitted mutation

**FR-440, FR-461**: personal and team records are never updated after creation — the same
rule Feature 003 already enforces for `memories` ("the only `UPDATE ... SET content`
statements are tombstones," `crates/cairn-store/src/repo.rs:1501,1601`;
`crates/cairn-server/src/sync.rs:628`; `crates/cairn-server/src/api.rs:713`). A change is
recorded as forgetting the old entry and, where applicable, creating a new one — never as
rewriting the old row's content in place.

The **one** permitted `UPDATE` beyond a content-clearing tombstone is the team state
transition (`proposed → authoritative`, `authoritative → retired`), and even that never
touches `content`:

```sql
UPDATE team_knowledge
   SET state = 'authoritative', ratified_by_user_id = $1, ratified_at = now()
 WHERE id = $2 AND state = 'proposed'   -- compare-and-swap on expected_state
```

This reuses, verbatim in spirit, the `expected_revision`/`blind_write` compare-and-swap
already proven for task criteria:

```rust
// crates/cairn-store/src/criteria.rs:572-585
fn check_revision(c: &Criterion, expected: Option<i64>) -> Result<bool> {
    match expected {
        None => Ok(true),
        Some(r) if r == c.revision => Ok(false),
        Some(r) => Err(refused(codes::REVISION_CONFLICT, format!(
            "criterion {} is at revision {} ({}, {}), not {r}",
            c.label, c.revision, c.state.as_str(), c.verification.as_str()))),
    }
}
```

The team-state analog compares `expected_state` against the row's actual `state` instead of
an integer revision, but the shape is identical: the caller states what it expects, the
statement's `WHERE` clause enforces it atomically, and a mismatch is refused by name
(`state_conflict`) rather than silently applied — `criteria.rs`'s guard exists precisely so a
concurrent change is *visible* rather than clobbered, and FR-454 asks for exactly that
property on team transitions. This is a deterministic compare-and-swap, not last-write-wins:
two concurrent ratification attempts against the same `proposed` row race on the `WHERE
state = 'proposed'` predicate, and SQL guarantees exactly one `UPDATE` affects a row — the
loser's statement affects zero rows and is reported `state_conflict`, naming the entry's
actual current state (FR-454, SC-415).

## 4. The applicability model

**FR-434, FR-435, FR-436, FR-446, FR-569, FR-570.** A record's applicability is a set of zero
or more `(kind, value)` facts. This is a fact about a *project* ("does this project use Rust,
does it use Docker") — it is unrelated to, and MUST NOT be confused with, the record's own
`topic_key` (§ below).

### Closed vocabulary — `language | tool`, and why `topic` is not a third member (D439)

**`ApplicabilityKind` has exactly two variants: `language | tool`.** An earlier draft of this
feature carried a third, `topic`, and this contract has removed it. The reason is structural,
not stylistic: `project_traits` (§5) is derived **only** from files present in a working
tree — manifest and lockfile presence, checked by `Path::exists`, never file content, never a
language model (FR-437). `language` and `tool` are exactly the kind of fact that derivation can
produce (`Cargo.toml` on disk ⇒ `language=rust`). **There is no file whose mere presence tells
Cairn a project's "topic."** A `topic` applicability kind therefore could never be populated by
§5's derivation, on any project, ever — it is not merely under-supported today, it is a
vocabulary member that structurally cannot match anything until an entirely different,
out-of-scope mechanism (reading semantic content) exists to populate it.

**A vocabulary member that can never match is worse than no member at all (FR-569).** A closed
vocabulary exists so that every kind a caller can name is one the system can actually resolve
against real project state. A kind that can never resolve does not fail loudly — it fails
silently, by making every record that names only that kind inapplicable to every project,
forever, with no error at creation time to say so. `ApplicabilityKind` is therefore restricted
to kinds §5 can actually derive: `language | tool`. `topic` is deferred to Feature 005 along
with the rest of the applicability work — any richer applicability that would require reading
file content, not merely checking presence, belongs there, not here.

`ApplicabilityKind` is a new `text_enum!` following the same pattern as `MemoryScope`. There is
no third kind and no free-form kind name; a value with an unrecognized kind is rejected at the
database `CHECK` before it can be considered at the application layer (FR-446, and
structurally, D414 — a value that cannot be an arbitrary string cannot carry a path, a
hostname, or a project name).

### A record's `topic_key` is not an applicability fact (FR-570)

Every personal or team record already carries its own `topic_key`/`value_key` (§2, the same
subject-key discipline Feature 003 established) — the knowledge's own subject, "what this
record is about" ("retry.backoff", "ci.provider"). **This is a completely different question
from applicability**, which asks "which *projects* does this record apply to," answered
exclusively by `(language | tool)` facts checked against `project_traits`. Both were once
loosely called "a topic," inviting exactly the conflation FR-570 forbids: a record's `topic_key`
is never consulted by the match predicate below, and an applicability `(kind, value)` pair is
never consulted by `derive_subject`'s reconciliation (§6). Wherever this document or
`recall-composition.md` discusses one, it names it precisely — "subject key" or `topic_key` for
the record's own key, "applicability" or "project trait" for the match predicate's input —
rather than "topic" doing double duty for both.

### Value normalization

Each value is normalized with the existing `normalize_value_key`
(`crates/cairn-core/src/knowledge.rs:99-111`: NFC → lower-case → collapse whitespace → reject
empty or `> 64` chars) and then additionally constrained to match `[a-z0-9_]{1,64}` — tighter
than `normalize_value_key` alone, because a value here is not free text like a memory's
`value_key` might loosely be; it names a discrete fact ("rust", "cargo", "graphql") and
nothing else is representable. A value that fails either step causes the *creation* of the
personal or team entry to be refused (`invalid_applicability_value`) — never silently
dropped, silently truncated, or silently stored with a null kind (FR-446).

### No facts means universal (FR-435)

A record with zero rows in its applicability table applies to every project. This is the
default, and it is what keeps "I just want to remember this for myself, everywhere" the
simple case: creating a personal entry with no `applicability` argument at all produces a
row visible from every project.

### The match predicate (FR-436, D412)

```text
applies(record, project) :=
  for every distinct kind K present on record.applicability:
    exists at least one value V such that (K, V) ∈ record.applicability
                                       AND (K, V) ∈ project.derived_traits
```

In words: **AND across kinds, OR within a kind.** A record naming both `language=rust` and
`tool=docker` applies only to a project that is *both* Rust and Docker. A record naming
`language=rust` and `language=python` applies to a project that is *either* — the two
`language` facts are alternatives, not a requirement for both.

### Worked truth table — two kinds, `language | tool`

Project traits: `{language: rust, tool: cargo}` (a project with `Cargo.toml`, no
`package.json`, no `Dockerfile`).

| # | Record's applicability facts | Applies? | Why |
|---|---|---|---|
| 1 | *(none)* | yes | No facts means universal (FR-435) |
| 2 | `language=rust` | yes | The one `language` fact matches |
| 3 | `language=python` | no | The one `language` fact does not match; no other `language` fact to OR against |
| 4 | `language=rust`, `language=python` | yes | OR within `language`: `rust` matches even though `python` does not |
| 5 | `language=rust`, `tool=docker` | no | AND across kinds: `language` matches but `tool` does not, and both kinds must be satisfied |
| 6 | `tool=cargo` | yes | The one `tool` fact matches |
| 7 | `language=rust`, `tool=cargo` | yes | Both kinds satisfied |
| 8 | `tool=docker` | no | The one `tool` fact does not match this project's derived traits; zero matching values for a present kind is the same as one non-matching value |

There is no ninth row for a third kind: `ApplicabilityKind` closes at `language | tool`
(above), so every record's applicability facts are drawn from exactly these two kinds, and
every row this table could ever need is one of the eight shapes above (some facts present or
absent, per kind, matching or not).

## 5. `project_traits` — derivation, not inference

**FR-437, FR-438, FR-439.** New **local-only** SQLite table,
`project_traits(project_id, kind, value)`, populated at `cairn link` time and refreshed on
demand (`cairn traits`, and internally whenever the daemon resolves a project and the
manifest set on disk has changed since the last derivation — a cheap stat-based check, not a
background scan).

Derivation is a fixed table lookup over files already present in the working tree — never
file *content*, never a language model, never a guess:

| File present at repo root (or a documented subdirectory) | Traits added |
|---|---|
| `Cargo.toml` | `language=rust`, `tool=cargo` |
| `package.json` | `language=node` |
| `pnpm-lock.yaml` | `tool=pnpm` |
| `package-lock.json` | `tool=npm` |
| `yarn.lock` | `tool=yarn` |
| `go.mod` | `language=go`, `tool=go` |
| `pyproject.toml` or `requirements.txt` | `language=python` |
| `Gemfile` | `language=ruby`, `tool=bundler` |
| `Dockerfile` or `docker-compose.yml` | `tool=docker` |
| `.github/workflows/*.yml` (any file present) | `tool=github_actions` |

This is FR-437's whole content: "derived deterministically from files already present in its
working tree, and MUST NOT be guessed, inferred from file content, or asked of a language
model." The check is existence-only (`Path::exists`), matching Constitution VI's discipline
that a derived fact must be recomputable identically by anyone holding the same repository
state, the same discipline `cairn-git`'s `discover()` already applies to Git facts
(`crates/cairn-git/src/lib.rs:97-124`) — no heuristic scoring, no partial credit, no
confidence value.

**Never synchronized (FR-438).** `project_traits` carries no `OutboxEntityType` variant and
no server table — the same structural mechanism that keeps `reusable_patterns` local
(`0005_project_intelligence.sql:236-238`) applied here for a different reason: traits are a
statement about *this machine's checkout*, and two machines' checkouts of the same project
could in principle diverge (a submodule present on one, absent on another) — synchronizing
traits would imply a false claim of one canonical trait set per project.

`cairn traits` (FR-439) prints the derived set for the current working directory:

```text
$ cairn traits
  language  rust
  tool      cargo
  tool      docker
```

## 6. Reconciliation — the same machinery, applied per domain

**FR-442, FR-493.** Personal knowledge reconciles among one user's own entries using
`classify_proposal` (`crates/cairn-core/src/knowledge.rs:329-427`) unchanged; team knowledge
reconciles among all `authoritative` entries the same way. The subject key changes shape —
there is no `(project_id, scope, scope_key)` to partition by — but the algorithm itself is
identical:

| Project memory subject | Personal/team subject |
|---|---|
| `(project_id, scope, scope_key, topic_key)` | `(owner_user_id, topic_key)` for personal; `(topic_key)` alone for team (there is exactly one team per server, so no further partition key exists) |

Applied at write time exactly as today:

1. Identical `content_norm_digest` within the subject ⇒ `Duplicate`, one `duplicates`
   relation, `basis = deterministic_rule`.
2. Same `value_key`, differing content ⇒ `Corroborating`, both retained, no relation.
3. Differing `value_key` ⇒ `ConflictDetected`, one `conflicts_with` per disagreeing member.
4. Otherwise `Created`.

Read time reuses `derive_subject` (`crates/cairn-core/src/knowledge.rs:580-872`) unchanged,
including its representative selection
(`verification_rank`/`representative_key`, `knowledge.rs:481-503`) and, critically, its
**no-timestamp** discipline: `MemoryFacts` and `Relation` carry no timestamp field at all
(`knowledge.rs:437-454`, `:210-216`), and this feature adds no timestamp field to
`personal_knowledge` or `team_knowledge` structs passed into `derive_subject` either. **No
comparison of `created_at`, `writer_seq`, or wall-clock time ever decides which of two
disagreeing personal or team entries prevails** (FR-493, FR-583) — exactly the same guarantee project
memory already has, applied to a domain where two devices reconciling offline makes the
guarantee matter even more, because there is no server-side arbitration step at all for
personal knowledge (it never even passes through `require_member`, since there is no project
to be a member of).

**Two disagreeing authoritative entries both stay visible** (FR-462, SC-466). Ratification is
not a resolution step: an admin ratifying a second entry that contradicts an existing
authoritative one produces a surfaced disagreement, not a replacement. Nothing about the order
in which the two were ratified changes which are returned — `derive_subject` never sees a
ratification time, for the same reason it never sees a `created_at`. `SC-466` asserts both
halves, because an implementation that returned only the most recently ratified entry would
satisfy "both remain stored" while breaking the guarantee that matters at recall.

### Relation kinds on team knowledge, and who may write each (D431)

`team_knowledge_relations` (§2) carries the same six kinds `memory_relations` does
(`crates/cairn-core/src/domain.rs:357-375`), but not all six arrive the same way:

| Kind | Automatic or explicit | Who / what writes it |
|---|---|---|
| `duplicates` | **Automatic** — `is_automatic` (`domain.rs:393-395`) | `classify_proposal`, exactly as for project memory and personal knowledge |
| `conflicts_with` | **Automatic**, and the only **symmetric** kind (`domain.rs:388-390`) | `classify_proposal`; endpoints normalized `(min(id_a,id_b), max(id_a,id_b))` exactly as `normalize_relation_endpoints` already does (`crates/cairn-store/src/knowledge.rs:248-254`) |
| `supersedes` | **Explicit only, and specifically an admin's act** | Written only by the admin performing a ratification that is meant to replace an existing authoritative entry — never inferred by `classify_proposal`, and never available to an ordinary member's proposal |
| `reinforces`, `narrows`, `not_applicable_to` | Explicit only, same as project memory | Recorded by whatever surface 004 exposes for them (out of scope beyond the CLI's general `cairn team` surface; no new discriminator is invented for these three) |

**Why `supersedes` on team knowledge is an admin act, not an inference**: superseding shared,
server-wide policy is a curation decision with consequences for every account on the server —
unlike a project-memory `supersedes`, which one session can decide unilaterally about its own
project's knowledge, an admin superseding team guidance is already the person in the loop at
the moment that matters (ratification), so requiring the decision to be explicit costs nothing
and prevents a member's ordinary proposal from silently retiring existing guidance it merely
happens to duplicate closely.

**A standing conflict between two authoritative team entries is a signal for an admin, never
auto-resolved.** 003's rule applies unchanged here: "a conflict may stand indefinitely;
standing is a reported state, not an error" — extended to team knowledge with the added
weight that the conflict is now visible to every account on the server, which is precisely
why it must be *resolved by a person* (an admin recording an explicit `supersedes` or
retiring one side) rather than by any deterministic tiebreak Cairn could invent.

**`derive_subject` operates per domain, over that domain's own relations, with no cross-
domain edges — this is an invariant, not a remark.** A relation may never link a record in
one domain to a record in another: no `personal_knowledge_relations` row ever names a
`team_knowledge` or `memories` id, and no `team_knowledge_relations` row ever names a
`personal_knowledge` or `memories` id. This is stated as firmly as it is because it is the
**one** edge shape that would be capable of leaking a private personal note into team-visible
derivation — if a personal entry could `supersedes` or `duplicates` a team entry, reading the
team subject's relations could expose the existence (and, depending on rendering, the
content) of a note that was never meant to leave its owner's account. Enforced structurally:
each relations table's foreign-key-shaped columns (`from_id`, `to_id`) are typed against, and
only ever populated from, that same table's own domain — `personal_knowledge_relations.
from_id`/`to_id` reference `personal_knowledge.id` exclusively, `team_knowledge_relations.
from_id`/`to_id` reference `team_knowledge.id` exclusively, and `memory_relations` continues
to reference `memories.id` exclusively as it always has. There is no column, on any of the
three tables, wide enough to name an id from a different domain's table in the first place.

## 5b. Team lifecycle: `proposed → authoritative → retired`

**FR-451 through FR-465.**

| Transition | Who | Effect |
|---|---|---|
| *(create)* → `proposed` | Any member of at least one project (checked against `project_members`, not against team membership — there is no team membership, only server accounts) | Row created, invisible to recall |
| `proposed` → `authoritative` | Admin only (`ratify`) | Visible to every account on the server from this instant |
| `authoritative` → `retired` | Admin only (`retire`) | Removed from recall; content unchanged |

**No action on the agent tool surface produces an `authoritative` row** (FR-455, FR-515). The
six tools can create a `proposed` row and nothing else; ratification is reachable only through
the CLI or server administration, by an admin. This is the constitutional line — *an agent may
propose; only a human administrator may make team-wide guidance authoritative* — and it is the
one guarantee in this contract that a single new tool action could quietly undo.

`SC-460` therefore verifies it by exercising **every** action the six tools expose and asserting
the resulting state, enumerated from the tool schema rather than from a list written today. A
test against a hardcoded list of actions passes unchanged on the day a seventh action is added,
which is precisely when it needed to fail.

**A `proposed` entry is invisible to *all* recall** (FR-452) — not merely filtered from the
default search, but absent from every code path that reads `team_knowledge` for display:
`cairn_search`, `cairn_context`, `cairn team list` for anyone who is not either the proposer
(who may see their own pending proposals, so they know what they proposed) or an admin (who
sees every state, because ratifying requires seeing what is waiting). This is the CHECK-level
predicate every read query carries:

```sql
WHERE state = 'authoritative'
   OR (state = 'proposed' AND (proposed_by_user_id = $caller OR $caller_is_admin))
```

**Ratify/retire is not an agent action (FR-455).** `cairn_remember` has no action that reaches
`ratify` — the MCP surface's `promote` action can create a `proposed` team entry (via
promotion, see `promotion-privacy.md`), but nothing in the six-tool agent surface can move a
`proposed` row to `authoritative`. Only `cairn team ratify` (CLI, backed by an admin-token
API call) or the server administration path can. This is deliberate: an agent, even holding
an admin's token by accident of environment, still cannot make policy authoritative through
its *tool* surface, because that surface simply has no action shaped like ratification. A
human must run the CLI command themselves.

**Retirement is not reversible by re-ratifying (FR-465).** `retire` sets `state = 'retired'`,
`retired_at = now()`; there is no transition back to `proposed` or `authoritative` from
`retired` — the state `CHECK` constraint plus the compare-and-swap predicate (§3) together
make "un-retire" simply not a statement the system can execute. Restoring retired guidance is
recorded as a **new** proposal, which itself must be ratified — so every piece of currently
authoritative guidance has exactly one ratification event in its own history, never a
resurrection.

**Visibility is server-wide, not membership-scoped (FR-458, FR-463).** An `authoritative`
team entry is visible to every account on the server, including one with zero project
memberships — team knowledge is a server-wide default, and `project_members` plays no role
in gating it. This is the one place in this feature where authorization is *not* mediated by
project membership, and it is deliberate: team guidance ("we use Conventional Commits," "CI
runs on GitHub Actions") is meant to reach every account precisely because it does not depend
on which projects that account happens to be added to.

**Disagreement stays visible (FR-462).** Two `authoritative` entries that conflict (differing
`value_key` on the same subject) both remain in `derive_subject`'s `Conflicted` output —
exactly the project-memory behavior — never silently resolved by "whichever was ratified
more recently." There is no code path that orders two `authoritative` rows by `ratified_at`
to pick a winner; the only way one stops competing with the other is an admin recording an
explicit `supersedes` (see above) or retiring one of them outright.

### `cairn team` CLI

```text
cairn team list [--all]      # member: authoritative + own proposals; admin: --all shows every state
cairn team propose <content> [--topic-key] [--value-key] [--applicability k=v]...
cairn team ratify <id>       # admin only
cairn team retire <id>       # admin only
```

`ratify`/`retire` are never available through `cairn_remember` — see `promotion-privacy.md`
and `mcp-tools.md` (003) for the "one discriminator, no sub-operation" precedent this
deliberately does not extend.

## Invariants

1. `MemoryScope` has exactly four variants after this feature ships, identical to before it
   (`project`, `branch`, `task`, `session`); no code path introduces a fifth (D401, FR-521).
   `SC-459` asserts the variant list and the `memories` scope `CHECK` text, so a fifth variant
   fails a test — this is the feature's central constraint and it had no criterion through three
   review passes (D457).
2. `KnowledgeDomain` and `MemoryScope` are never compared, combined, or partitioned
   together — a personal or team record has no `scope` and no `scope_key`.
3. No query joins `personal_knowledge` and `team_knowledge` in a way that could return one
   user's personal row while a caller believes they are reading team or another user's
   personal data; the two tables are read by entirely separate code paths (D402).
4. Neither `personal_knowledge` nor `team_knowledge` has a `project_id` column, in schema,
   permanently (D403, FR-517, FR-459).
5. No `UPDATE` statement targets `personal_knowledge.content` or `team_knowledge.content`
   after insertion; the only permitted `UPDATE` clears content (tombstone) or transitions
   `team_knowledge.state` via compare-and-swap on `expected_state` (D405, D409, FR-440,
   FR-461).
6. A personal or team applicability value that is not `[a-z0-9_]{1,64}` after
   `normalize_value_key`, or whose kind is not `language | tool`, causes creation to be
   refused; it is never stored, truncated, or silently dropped (FR-446, D414, D439).
7. A record with zero applicability facts matches every project; a record with one or more
   facts matches a project iff every present kind has at least one matching value (D411,
   D412).
7a. No personal or team row carries a verification field of any kind — not an authority, not a
   state, not a timestamp — in either store or in the serialized form (FR-513, FR-517, D452,
   SC-422, SC-424).
7b. No action on the six-tool agent surface creates a `team_knowledge` row in the
   `authoritative` state (FR-455, FR-515, SC-460).
7c. Two disagreeing `authoritative` entries on one subject are both returned by recall, and
   ratification order changes nothing about which are returned (FR-462, SC-466).
8. `project_traits` carries no `OutboxEntityType` variant and appears in no outbox payload,
   ever (FR-438, D413).
9. No comparison of `created_at`, `writer_seq`, or arrival order ever selects a winner
   between two disagreeing personal or team entries on the same subject; the same
   `classify_proposal`/`derive_subject` pair project memory uses decides it, unchanged
   (FR-442, FR-493, D406).
10. A `team_knowledge` row in state `proposed` is returned by no search, context, or list
    surface except to its proposer or to an admin (FR-452).
11. No MCP action transitions a `team_knowledge` row's state; ratification and retirement
    are reachable only through the CLI or direct server administration (FR-455).
12. A `retired` team entry never returns to `proposed` or `authoritative`; restoring its
    guidance requires a new proposal and a new ratification (FR-465).
13. `team_knowledge_relations` exists, in both the local store and server Postgres, with the
    same six kinds and the same PK shape as `personal_knowledge_relations`; `duplicates` and
    `conflicts_with` remain automatic and `conflicts_with` remains the only symmetric kind;
    `supersedes` on team knowledge is written only by the ratifying admin, never inferred
    (D431).
14. No relation of any kind ever links records in two different domains: a
    `personal_knowledge_relations`, `team_knowledge_relations`, or `memory_relations` row
    names only ids from that same table's own domain, with no exception (D431).
15. `ApplicabilityKind` has exactly two variants, `language | tool`; no code path, CHECK
    constraint, or documentation names a third (FR-569, D439).
16. A record's `topic_key` is never read by the applicability match predicate (§4), and an
    applicability `(kind, value)` fact is never read by `derive_subject`'s reconciliation
    (§6); the two are documented, and implemented, as unrelated (FR-570).
17. `content` and every `applicability` value on a personal or team record has passed
    `validate_global_content` (`promotion-privacy.md` §2a) before the row exists, whether
    the record arrived by direct creation or by promotion — direct creation is not exempt
    (D433, FR-544, FR-545).
18. A local store's `personal_knowledge` rows are partitioned by the owning
    `(server_instance_id, user_uuid)` pair; recall surfaces only the currently linked
    identity's rows, and a second identity's rows already present are neither deleted nor
    merged (D438, FR-567, FR-568 — see `sync-namespaces.md` §10 for the full argument).
