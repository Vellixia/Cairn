# Contract — Verification Summary

The server may know **that** a memory was verified. It may never be *told* that, and it never
holds **what was observed** while establishing it.

## 1. What stays local, forever

Raw evidence facts and verification runs are structurally excluded from the server: no outbox
entity type carries them, no server table holds them (FR-811a, FR-707). This is not a policy a
handler enforces — the existing synchronization boundary refuses `observations`,
`observed_value`, `source_locator`, `value_digest` and every other evidence-shaped field
recursively at any depth (`crates/cairn-server/src/sync.rs:27-78`), and Feature 005 adds no
second path around it. Nothing in this feature reopens that refusal for verification data; it
adds only what §2 defines, on a new table, with a closed field set.

## 2. What the server may hold

A **derived summary**, three columns added to `memories` (data-model.md §6):

```sql
ALTER TABLE memories
  ADD COLUMN verification_state     TEXT,   -- DERIVED, never client-set
  ADD COLUMN verification_authority TEXT,
  ADD COLUMN last_verified_at       TIMESTAMPTZ;
```

The reason is stated once and applies everywhere below: **a state the user cannot see
centrally is a state the web control plane cannot report** (FR-811a). Principle X forbids
reporting a state Cairn cannot establish; it does not forbid holding the state Cairn *did*
establish, one level removed from the evidence that established it.

## 3. `verification_reports` — a report of a run, never a claim of a state

```sql
CREATE TABLE verification_reports (
  report_id     UUID PRIMARY KEY,
  memory_id     UUID NOT NULL,
  project_id    UUID NOT NULL,
  account_id    UUID NOT NULL,
  verdict       TEXT NOT NULL CHECK (verdict IN ('passed','failed','inconclusive')),
  verifier_kind TEXT NOT NULL,
  authority     TEXT NOT NULL,
  run_at        TIMESTAMPTZ NOT NULL,
  received_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

**`verdict` is spelled `passed | failed | inconclusive`, deliberately not
`verified | drifted | inconclusive`.** The local `VerifyResult` enum
(`crates/cairn-core/src/domain.rs:505-510`) uses the latter; the server's column uses the
former on purpose, so that the one word the server must never accept as a claim —
`verified` — is not even in the accepted vocabulary. A client cannot submit `"verdict":
"verified"` and have it mean anything; the field simply does not parse. This is the same style
of guarantee `recall-composition.md` uses for "no type carries two domains' scores" — the
value the rule forbids is not merely rejected, it is not expressible.

`verifier_kind` reuses the existing closed vocabulary
(`crates/cairn-core/src/domain.rs:486-495`): `file_exists`, `file_digest`, `git_ref`,
`git_commit`, `configuration`, `schema_version`, `test_outcome`, `command_outcome`,
`runtime_state`. There is no `model_judgment` member and none is added — a model's opinion is
not a verifier (Principle IX, `verify.rs:9-16`'s stated purpose, carried into this table
unchanged). `authority` reuses `VerificationAuthority` exactly: `Cairn | Attested | RemoteCairn
| RemoteAttested` (`domain.rs:300-314`).

### 3.1 Submission endpoint

```
POST /api/verification/reports
Authorization: Bearer <token>

{
  "report_id":     "3f9a…",
  "memory_id":     "7c21…",
  "verdict":       "passed",
  "verifier_kind": "test_outcome",
  "authority":     "cairn",
  "run_at":        "2026-08-30T14:02:11Z"
}
```

`project_id` and `account_id` are absent from the payload — bound server-side (§6). Response:

```json
{
  "report_id": "3f9a…",
  "status": "accepted",
  "previous_state": "needs_recheck",
  "derived_state": "verified"
}
```

A refused report never partially updates `memories` — derivation (§4) and the report insert
happen in one transaction, so a rejected report leaves the prior `verification_state` exactly
as it was.

### 3.2 What a report may not contain — a rejected example

The columns above are the entire shape. A submission carrying anything else is refused before
derivation runs, by the identical recursive field check `safe-events.md` describes for
canonical events:

```json
{
  "report_id": "3f9a…",
  "memory_id": "7c21…",
  "verdict": "failed",
  "verifier_kind": "command_outcome",
  "authority": "cairn",
  "run_at": "2026-08-30T14:02:11Z",
  "observed_value": "exit code 1",
  "command_output": "FAIL test_retry_backoff\n  expected 3, got 1\n",
  "source_locator": "src/retry.rs:88"
}
```

→ refused: `field_not_permitted`, naming `observed_value`, `command_output`,
`source_locator`. **No observed values, no source locators, no digests of file content, no
command output, no local paths** — state, authority, a timestamp and counts only (FR-811c).
The refusal carries the field names it rejected, never the values attached to them, mirroring
`safe-events.md` §7's per-item outcome shape and `consolidation.md` §9's rule that a refusal
record never carries the content that caused it.

## 4. Derivation — the state machine

The server never stores what a client says the state is; it stores what the **local rules,
re-applied to accepted reports**, say the state must be (FR-811b). The rules mirror
`crates/cairn-core/src/verify.rs`'s `transition` function, adapted to the two facts that
differ server-side: there is no fingerprint to change (fingerprints are evidence, and evidence
never crosses, §1), and there is no separate "evidence attached/removed" action — a report
**is** the only trigger the server ever receives.

| Current state | `verdict = passed` | `verdict = failed` | `verdict = inconclusive` |
|---|---|---|---|
| `unverified` | → `verified` | → `drifted` | → `unverified` (unchanged) |
| `verified` | → `verified` (reinforced) | → `conflicted` | → `verified` (unchanged) |
| `needs_recheck` | → `verified` | → `drifted` | → `needs_recheck` (unchanged) |
| `drifted` | → `verified` | → `drifted` (unchanged) | → `drifted` (unchanged) |
| `conflicted` | → `needs_recheck` | → `conflicted` (unchanged) | → `conflicted` (unchanged) |

Rows sourced directly from the local table (`verify.rs:78-117`), unchanged in shape:
`(unverified, passed→verified)`, `(unverified, failed→drifted)`,
`(unverified, inconclusive→unverified)`, `(needs_recheck, passed→verified)`,
`(needs_recheck, failed→drifted)`, `(needs_recheck, inconclusive→needs_recheck)`,
`(drifted, passed→verified)`. The local table documents no transition at all for the
remaining combinations (`verified`+`passed`, `verified`+`inconclusive`, `drifted`+`failed`,
`drifted`+`inconclusive`, `conflicted`+`inconclusive`); this contract fills them with the
conservative default the rest of the table already establishes — **a passed run reinforces
where it can, an inconclusive run changes nothing, a further failure while already `drifted` or
`conflicted` leaves it exactly there.**

Two rows are new, because the server has an action the local machine does not — a plain
report is all it ever gets, so a report is what both establishes *and* clears a conflict:

- **`(verified, failed) → conflicted`.** A report that disagrees with an established claim is
  the server's only shape for "this memory's own evidence disagrees with itself" — the direct
  server-side counterpart of local's `(Verified, ContradictingEvidenceAttached) → Conflicted`.
- **`(conflicted, passed) → needs_recheck`, never → `verified`.** Exit from `conflicted` lands
  on `needs_recheck`; a single passed report is not enough to clear a state that was reached
  because two reports disagreed. A second, subsequent passed report is required to reach
  `verified` from `needs_recheck` — the ordinary `(needs_recheck, passed) → verified` row above
  — which is the concrete mechanism behind "a needs_recheck state must not be resurrected to
  verified without a new passed run."

**`verified` requires ≥1 passed run and no contradicting report, restated as the falsifiable
form this table gives it**: every path that reaches `verified` passes through a `passed`
verdict on its last step (row 1, 3, 4, and the reinforcement on row 2); no path reaches
`verified` from `conflicted` or `drifted` in one step on a `failed` or `inconclusive` verdict.
The aggregate English sentence and the table say the same thing; the table is what a test can
assert against.

## 5. Why a client cannot set state — three independent reasons

1. **The vocabulary excludes the word.** `verdict` has no `verified` member (§3). There is no
   payload shape that means "mark this verified."
2. **There is no state-setting endpoint.** `/api/verification/reports` accepts a report and
   returns the state §4 derives from it; nothing accepts `verification_state` as an input
   field, and the column is never present in any request schema this feature defines.
3. **A model must not influence verification at all** (constitution, carried into this table
   by `verifier_kind` having no model-judgment member, §3). Consolidation is explicitly
   forbidden from asserting verified (FR-811, `consolidation.md` §5 gate 9) — an extractor's
   confidence in a candidate is not a verifier_kind, is never submitted as one, and would be
   refused by the same closed-vocabulary check as any other malformed `verifier_kind` if it
   were attempted.

## 6. Where a summary cannot be established

A memory's `verification_state` reads `unverified` — never inherits `verified` from anywhere it
cannot justify — in both of these cases (FR-811d):

| Situation | Server state |
|---|---|
| Every run against this memory happened, and stayed, local-only (no report was ever sent) | `unverified` |
| A report was sent but refused (§3.2, or failed §6's authorization checks) | `unverified` (unchanged by the refused report) |

**Local and server states can disagree, and that is allowed.** A developer's own machine may
show `verified` from a run it performed and never reported — nothing requires every local run
to be reported, and FR-815's explicit-creation guarantee does not extend to auto-reporting every
verification. The server's state is not wrong in that case; it is answering a narrower
question — *what has the server been told, and does that support this state* — while the local
state answers *what has this machine actually checked*. The two are reconciled, not merged: a
later report brings the server's derived state into agreement with what a machine already knew,
via the ordinary transition in §4, never by a bulk "trust the client" import. A web view reading
`unverified` for a memory the developer knows is locally `verified` is reporting truthfully
that no report has reached the server yet — not that the memory is unverified in the world.

## 7. Authorization checks

Applied in order; the first failure refuses the report and none run after it.

| # | Check | Failure |
|---|---|---|
| 1 | Caller is authenticated | `401 unauthorized` |
| 2 | `memory_id` resolves to a memory whose project the caller is a member of | `forbidden`, "you are not a member of this project" (mirrors `auth.rs:536`, FR-894a's pattern) |
| 3 | `account_id` is bound from the credential, never read from the payload | payload does not carry the field at all — there is nothing to check, by construction |
| 4 | `project_id` is bound from the memory's own record, never from the payload | same — absent by construction |
| 5 | `verdict`, `verifier_kind`, `authority` are each in their closed vocabulary | `malformed_report` |
| 6 | No field outside §3's closed set is present | `field_not_permitted`, naming the fields (§3.2) |
| 7 | The referenced memory is not in the `personal` or `team` domain being used to certify **cross-project** guidance | `cross_project_verification_refused` |

Check 7 is `FR-826`'s existing constitutional rule carried into this table: cross-project
guidance is never the basis of a verification claim, so a report whose memory is a personal or
team record being read *as if* it verified something for a project it does not name is refused
rather than silently accepted and misfiled.

Every check reads state Cairn already holds — the memory's project, the domains it may
participate in, the caller's membership — never anything the report itself asserts about
identity. Identity is established from the credential and the existing record graph, never
taken on the report's word (Principle XI, carried from `safe-events.md` and `consolidation.md`
into this table unchanged).

## Invariants

1. Raw evidence and verification runs never reach the server, in this feature or any other
   path this feature adds (FR-811a, FR-707).
2. The server's only verification input is a report of a run; it derives state, and never
   accepts a state as a claim (FR-811b).
3. `verdict`'s vocabulary structurally excludes the word `verified` — the forbidden claim is
   not merely rejected, it is not expressible (§3).
4. A report carries state, authority, a timestamp and counts only; no observed value, source
   locator, content digest, command output or local path (FR-811c).
5. `verified` is reachable only through a `passed` verdict on the transition's last step, from
   `unverified`, `needs_recheck`, `drifted`, or (reinforcing) `verified` itself — never in one
   step from `conflicted` (§4).
6. Exit from `conflicted` lands on `needs_recheck`; reaching `verified` from there requires a
   second, subsequent `passed` report (§4).
7. Where a summary cannot be established — the run was local-only, or its report was refused —
   the server holds `unverified` rather than inheriting a state it cannot justify (FR-811d).
8. Consolidation never submits a report and never asserts verified; a model's output is never a
   valid `verifier_kind` (FR-811, Principle IX).
9. `account_id` and `project_id` come from the authenticated credential and the memory's own
   record, never from the report payload (Principle XI).
10. A report referencing cross-project personal or team guidance is refused as the basis of a
    verification claim (FR-826).

---

## 10. Corrections from the falsification pass (binding)

Three defects were found in §§3-5 and are corrected here. Where this section and an earlier
one differ, this section governs.

### 10.1 `authority` is server-assigned, never client-chosen

A report **must not** carry `authority`. Accepting it made `verified` one indirection from
client-asserted: any project member could submit `{"verdict":"passed","authority":"cairn"}`
and obtain the strongest authority, which shipped code reserves for *"a deterministic check
**this machine** ran … the only authority a task criterion or a cross-project promotion
accepts"* (`crates/cairn-core/src/domain.rs:483-487`).

The server assigns authority from the path the evidence arrived by:

| Path | Assigned authority |
|---|---|
| A deterministic check the **server** ran itself | `Cairn` |
| A run reported by an authenticated client | `RemoteCairn` |
| An agent attestation reported by a client | `RemoteAttested` |
| Anything else | refused |

`Cairn` is unreachable from the report endpoint by construction. A client-reported run can
reach `RemoteCairn` at most, which is exactly what it is: a run this server did not witness.

### 10.2 A report is bound to a distinct run

`report_id` must not be client-supplied. The server assigns it, and enforces
`UNIQUE (memory_id, verifier_kind, run_at)`. Without this, the §4 guard requiring "a second,
subsequent `passed` report" to leave `conflicted` is satisfied by submitting one run twice
under two ids — a decorative guarantee.

A duplicate report is answered `duplicate` and changes no state.

### 10.3 Personal and team knowledge are reportable

`verification_reports.project_id` is **nullable**, and exactly one of `project_id` or
`owner_user_id`/team scope is set:

| Domain | Binding | Authorization |
|---|---|---|
| project | `project_id` from the memory | caller is a member of that project |
| personal | `owner_user_id` from the record | caller **is** that owner |
| team | team scope | caller is a member of the server's team |

Personal and team knowledge are project-independent by construction (FR-822), so a
`NOT NULL project_id` made verification of those domains unrepresentable. This is distinct
from FR-826, which forbids a check performed against one project from transferring its
authority to a project-independent assertion — that rule is unchanged and still applies to the
*authority* assigned, not to whether a report may exist.
