# Contract — Verification

One contract, stated once. Raw evidence stays on the machine that produced it; the server holds
a derived summary it computes itself and that no client can assert.

## 1. What already exists — audited, not assumed

Server `memories` already carries five verification columns
(`crates/cairn-server/migrations/0002_project_intelligence.sql:31-35`):

| Column | Type | Note |
|---|---|---|
| `verification` | `TEXT` | the state. **There is no `verification_state` column.** |
| `verification_authority` | `TEXT` | |
| `last_verified_at` | `TIMESTAMPTZ` | |
| `verification_basis` | `JSONB NOT NULL DEFAULT '[]'` | array of verifier-kind names |
| `evidence_fact_count` | `INTEGER NOT NULL DEFAULT 0` | |

None has a CHECK constraint. Local SQLite has `verification`, `verification_authority` and
`last_verified_at` (`crates/cairn-store/migrations/0005_project_intelligence.sql:29-31`) and
deliberately **not** `verification_basis` or `evidence_fact_count`.

Vocabularies, from `crates/cairn-core/src/domain.rs`:

- `VerificationState`: `unverified`, `verified`, `needs_recheck`, `drifted`, `conflicted`
- `VerificationAuthority`: `cairn`, `attested`, `remote_cairn`, `remote_attested`
- `VerifierKind`: `file_exists`, `file_digest`, `git_ref`, `git_commit`, `configuration`,
  `schema_version`, `test_outcome`, `command_outcome`, `runtime_state`

**Feature 005 adds no new verification column and renames none.** `verification` is the
canonical state column. An earlier draft of this plan proposed `verification_state`,
`verification_authority` and `last_verified_at` as additions; all three were wrong — the first
is a second name for an existing concept and the other two already exist.

## 2. The bypass this closes

`/api/sync/batch` currently accepts all five values verbatim from the client payload
(`crates/cairn-server/src/sync.rs:718-731`): `verification` and `verification_authority` via
`opt_text`, `last_verified_at` parsed from a client string, `verification_basis` as a raw JSON
array, `evidence_fact_count` as a raw integer. There is no parsing into the enums and no
derivation. The comment at `sync.rs:621-626` says the receiving daemon maps it; nothing enforces
that.

Any project member can therefore write `verification = 'verified'`,
`verification_authority = 'cairn'` on any memory in the project. `cairn` is the strongest
authority, reserved in code for a deterministic check *this machine* ran, and the only authority
a task criterion or a cross-project promotion accepts (`domain.rs:483-487`).

After cutover, `memory` writes on `/api/sync/batch` are refused entirely
(`knowledge-commands.md` §2), which closes this bypass along with the rest.

**Before cutover, behaviour is unchanged.** The server continues to accept the fields as it does
today. It must: `verification_reports` starts empty, so a server that preferred "what it has
derived" would hold `unverified` for every record and would zero every client's verification on
the next sync — and a pre-cutover client has no report endpoint to compensate with. Tightening
the old path before the new one exists would break working installations to fix a bypass that
cutover closes anyway.

The bypass therefore closes **at cutover**, in one step, together with §9's re-derivation.

## 3. What crosses, and what never does

Evidence facts, verification runs and their observed values remain local-only, structurally:
`evidence_fact` and `verification_run` are on `FORBIDDEN_ENTITY_TYPES` and there is no server
table for either.

A **run report** crosses. It carries:

```json
{ "memory_ref":    { "domain": "project", "knowledge_id": "…" },
  "verdict":       "passed",
  "verifier_kind": "test_outcome",
  "run_at":        "2026-08-30T09:14:22Z" }
```

That is the whole payload. It carries **no** observed value, no source locator, no digest of
file content, no command output, no local path, and — deliberately — **no `authority` and no
`report_id`**. A rejected example:

```json
{ "verdict": "passed", "authority": "cairn",
  "observed_value": "sha256:ab…", "source_locator": "/Users/me/repo/x.rs" }
```

refused: `authority` is server-assigned (§4), `report_id` is server-assigned (§5), and
`observed_value` / `source_locator` are refused field names at any depth.

## 4. Authority is assigned by the server, from the path evidence arrived by

| How the server learned of the run | Assigned `verification_authority` |
|---|---|
| a deterministic check the **server itself** ran | `cairn` |
| a run reported by an authenticated client | `remote_cairn` |
| an agent attestation reported by a client | `remote_attested` |
| a client-asserted authority field | *refused* |

`cairn` is unreachable from the report endpoint by construction. A client-reported run can reach
`remote_cairn` at most, which is exactly what it is: a run this server did not witness. This is
the rule that makes §2's bypass unrepeatable rather than merely relocated.

`verification_basis` accumulates the `verifier_kind` of accepted reports, deduplicated, and is
server-maintained. `evidence_fact_count` counts accepted reports, never a client-supplied number.

## 5. Report identity and duplicate runs

`report_id` is **server-assigned**. The natural key is
`UNIQUE (domain, knowledge_id, verifier_kind, run_at)`.

Without it, the §6 rule requiring a *second, subsequent* passed run to leave `conflicted` is
satisfied by submitting one run twice, and the guarantee is decorative. A repeat of the same
natural key is answered `duplicate` and changes no state.

## 6. Derived state

The server derives `verification` from the accepted report history. It never stores an asserted
state.

| Current | `passed` | `failed` | `inconclusive` |
|---|---|---|---|
| `unverified` | `verified` | `unverified` | `unverified` |
| `verified` | `verified` | `conflicted` | `verified` |
| `needs_recheck` | `verified` | `needs_recheck` | `needs_recheck` |
| `drifted` | `verified` | `drifted` | `drifted` |
| `conflicted` | `needs_recheck` | `conflicted` | `conflicted` |

Two properties are deliberate and mirror the shipped local machine
(`crates/cairn-core/src/verify.rs`): a `failed` report against a `verified` record yields
`conflicted` rather than silently demoting to unverified; and exit from `conflicted` lands on
`needs_recheck`, never directly on `verified`, so a contradiction always costs one more
deliberate run.

`last_verified_at` is set from `run_at` of the most recent accepted `passed` report.

Where no summary can be established — the run was local-only, or the report was refused — the
server holds the record as `unverified` rather than inheriting a state it cannot justify. An
unsubstantiated `verified` is the overclaim Principle X forbids.

Consolidation never asserts verification (FR-811), and a model never influences it at all: the
report endpoint is not reachable from the extractor, and `verdict` is not a field any
`CandidateProposal` carries.

## 7. Domain bindings

`verification_reports` binds per domain, exactly one of the three set. `project_id` is
**nullable**, because personal and team knowledge are project-independent by construction
(FR-822) and a `NOT NULL project_id` would make verification of those domains unrepresentable.

| Domain | Bound from | Authorization |
|---|---|---|
| `project` | the memory's `project_id` | caller is a member of that project |
| `personal` | the record's `owner_user_id` | caller **is** that owner |
| `team` | the server's single team | caller is a member; `proposed` rows also require author-or-admin |
| `pattern` | the pattern's `account_id` | caller **is** the pattern's author |

The referenced record is resolved as a `KnowledgeRef` (`data-model.md` §6.1) — project,
personal and team knowledge are separate tables and a bare id names only the first.

FR-826 is unchanged and separate: a deterministic check performed against one project does not
transfer its authority to a project-independent assertion. That constrains the **authority
assigned** to a personal or team record, not whether a report may exist for one.

## 7.4 Where a personal, team or pattern summary lives

`memories` carries the five verification columns. `personal_knowledge`, `team_knowledge` and
`shared_patterns` do **not**, and Feature 005 does not add them — five columns on four tables is
four places for a derivation to drift.

Summaries for non-project domains live in one derived table, keyed by `KnowledgeRef`:

```sql
CREATE TABLE knowledge_verification (
  domain        TEXT NOT NULL CHECK (domain IN ('personal','team','pattern')),
  knowledge_id  UUID NOT NULL,
  verification  TEXT NOT NULL DEFAULT 'unverified',
  verification_authority TEXT,
  verification_basis     JSONB NOT NULL DEFAULT '[]',
  evidence_fact_count    INTEGER NOT NULL DEFAULT 0,
  last_verified_at       TIMESTAMPTZ,
  PRIMARY KEY (domain, knowledge_id)
);
```

The project domain keeps using `memories`' own columns, because they exist, are already
populated, and moving them would be a migration with no benefit. Readers resolve a summary by
domain: `project` reads `memories`; everything else reads `knowledge_verification`. Both are
derived by the same code path from the same `verification_reports` table, so there is one
derivation and two storage locations, not two derivations.

FR-826 still binds here: a deterministic check performed against one project does not transfer
its authority to a project-independent record. A personal, team or pattern summary may therefore
reach `remote_attested` or `remote_cairn`, but never `cairn`.

## 7.5 The changes feed stops carrying verification at cutover

`/api/sync/changes` currently emits `verification{state, authority, last_verified_at,
evidence_fact_count, basis}` on every memory row (`crates/cairn-server/src/sync.rs:904-921`).

At cutover the feed **stops emitting the verification object**. Without this, §9's
re-derivation would propagate to every client's local `verification` column on the next pull and
overwrite states those machines earned from runs they actually performed — losing knowledge,
against FR-866, and contradicting §8's own invariant.

After cutover: each machine keeps deriving its local state from its own runs, and the server
keeps deriving its summary from accepted reports. Neither writes the other's. The web control
plane reads the server's summary; `cairn memory show` reads the local one; both are labelled.

## 8. Local and server states may differ, without contradiction

They answer different questions. The local state is derived from runs this machine performed
against evidence it holds; the server state is derived from reports it accepted. A machine that
has run a check the server has not been told about is legitimately ahead.

Neither overwrites the other. The server never pushes its state into the local `verification`
column, and after cutover the client cannot push its state into the server's. Where a user sees
both, the web control plane labels which is which.

## 9. Migrating legacy verification values

Existing server rows carry verification values written under §2's bypass, with no evidence
behind them.

**When:** at **cutover**, not at any client's migration. Cutover is the single server-wide
admin action (`migration-cutover.md` §2), and this is a server-wide re-derivation, so it belongs
in the same transaction — together with §7.5's change to the changes feed, so no client can pull
a demoted state before the feed stops carrying verification at all. Client migration phases are
unaffected and gain no step.

At cutover, every project memory's `verification` is **re-derived from zero reports**, which
means: any row whose state cannot be substantiated by an accepted report becomes `unverified`,
with `verification_authority` cleared, `verification_basis` reset to `[]` and
`evidence_fact_count` to `0`.

`last_verified_at` is **cleared** on the record, not retained in place. A timestamp sitting in
`last_verified_at` beside `verification = 'unverified'` reads as "verified then, unverified
now", which is a claim the server cannot substantiate either — the original value was
assertable by any project member, so it never established that a run occurred at that time.

The old value is not destroyed silently: it moves to
`legacy_verification_audit (domain, knowledge_id, legacy_state, legacy_authority,
legacy_last_verified_at, demoted_at)`, explicitly labelled untrusted pre-cutover audit
metadata. It is visible in the memory detail view under that label, is never read by any
derivation, and can never be promoted back into a state.

This is deliberately lossy in one direction only, and only on the server. The alternative —
grandfathering states that were assertable by any project member — would carry the bypass
forward past the point where it was closed, and would leave `authority = cairn` values in place
that no server-side check ever produced.

**No local state is demoted.** The re-derivation touches the server's columns only, and §7.5
stops the changes feed carrying verification from the same moment, so the demotion cannot reach
a client. A machine that earned `verified` from its own runs keeps it. Clients re-report their
runs after upgrading and the server's summary refills within one verification pass; an unearned
`verified` does not come back.

Migration reports the count of demoted server rows so the change is visible rather than silent.

## 10. Authorization checks, in order

1. Authenticate; resolve account from the credential.
2. Resolve `memory_ref` to a record; refuse if absent.
3. Resolve the domain binding per §7; refuse if the caller fails that domain's check.
4. Validate `verdict` and `verifier_kind` against their closed vocabularies.
5. Refuse any payload carrying `authority`, `report_id`, or a refused field name.
6. Assign `report_id` and authority (§4, §5).
7. Insert; on natural-key conflict answer `duplicate` and stop.
8. Re-derive `verification`, `verification_basis`, `evidence_fact_count` and `last_verified_at`
   for the record (§6), in the same transaction.
