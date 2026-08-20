# Contract: Evidence, Verification and Drift

**Feature**: `003-project-intelligence`

Deterministic first. A model's opinion is never verification (FR-361). What Cairn can read itself, it
reads; what it cannot, an agent may attest — labelled, and capped in what it can establish.

```text
claim (a memory)  ─┬─▶ evidence fact (bounded, redacted, attributable, local)
                   │        │
                   │        └─▶ fingerprint
                   │                │
                   │       fingerprint changed
                   │                ▼
                   └─▶ verification run ──▶ verified | drifted | inconclusive
                            (deterministic verifier, recorded with repo state)
```

Three things stay distinct at every step: the **claim**, the **evidence**, and the **verified
result** (FR-367).

## Evidence facts

### What may be stored

| Field | Bound | Rule |
|---|---|---|
| `subject` | ≤128 bytes | Redacted. A label — "database backend", "API port" |
| `observed_value` | ≤256 bytes | Redacted **before** the bound is applied (FR-354) |
| `value_digest` | 64 hex | SHA-256 of the normalized observed value |
| `source_locator` | ≤256 bytes | Repository-relative path, or a Git ref name. **Never absolute** |
| `fingerprint` | 64 hex | What change detection compares |

`source_locator` is refused if it starts with `/` or `\`, matches `^[A-Za-z]:[\\/]`, contains `..`
after normalization, or resolves outside the worktree. Asserted by test (FR-353, I9).

### The bad and good cases, made structural

```text
REFUSED   DATABASE_URL=postgres://user:password@host/db
```

There is nowhere to put it. Redaction rewrites the credential to `[REDACTED]` before the write
(Feature 001's pipeline, FR-354), and 256 bytes after redaction is not a configuration file.

```text
STORED    kind:            configuration
          subject:         database backend
          observed_value:  postgresql
          value_digest:    9f2e…
          source_locator:  config/database.yml
          fingerprint:     9f2e…
          collector:       cairn
          repo_commit:     abc123
```

### Excluded paths

Evidence is never created for a path matching `excluded_paths`, or from a command matching
`excluded_commands`. The memory stays `unverified` and the reason is reported as
`evidence_excluded` — distinguishable from `no_evidence`, so a developer can tell "I told Cairn not
to look" from "nobody attached anything" (spec Edge Cases).

### Deletion

Deleting an evidence fact tombstones it: identity, kind, timestamps and provenance survive;
`observed_value`, `value_digest`, `source_locator` and `fingerprint` are cleared. The link row
survives, so the reference resolves to **evidence deleted** rather than disappearing — Feature 001's
semantics for observation evidence, extended (FR-358, FR-505). The supported memory's verification
becomes `needs_recheck`, never `verified`.

## The verifier catalog

### Cairn-collected (`collector = cairn`)

Cairn reads these itself, inside the worktree subject to exclusions, or through `cairn-git`. It
executes nothing (FR-365).

| Verifier | Reads | Fingerprint | `inconclusive` when |
|---|---|---|---|
| `file_exists` | path presence | `exists:<0\|1>:<size>` | path is excluded, or the worktree is unreadable |
| `file_digest` | file bytes | content SHA-256 | file unreadable, excluded, or larger than the payload cap |
| `git_ref` | `cairn-git` ref resolution | resolved object id | ref unresolvable |
| `git_commit` | commit presence, ancestry | commit id | commit not present in this clone |
| `configuration` | one key in a repository configuration file | value SHA-256 | file unparseable, key absent, or path excluded |
| `schema_version` | a declared version field | value SHA-256 | as above |
| `test_outcome` | a **captured** `test_run` observation's recorded outcome and exit code | `<outcome>:<exit>:<commit>` | no captured observation matches at or after the claimed commit |
| `command_outcome` | a **captured** `command_run` observation's recorded exit code | `<exit>:<commit>` | as above |

`test_outcome` and `command_outcome` are Cairn-collected because Feature 001's hooks already record
the command, the exit code and the commit — Cairn observed the result without running anything. This
is what resolves the brief's apparent tension between "known test result at a commit is valid
verification" and "no autonomous command execution" (D52).

### Agent-attested (`collector = agent`)

| Verifier | Source | May establish |
|---|---|---|
| `runtime_state` | the agent submits an observed value and its digest | a memory's `verified`, authority `attested` |
| `test_outcome` / `command_outcome` with no captured observation | the agent submits outcome and exit code | as above |

Attested evidence:

- is stored with `collector = 'agent'`;
- can move a **memory** to `verified` — with authority `attested`, which travels with the state
  everywhere, including across a sync boundary (FR-355, FR-370);
- can **never** move a **task criterion** to `verified` (D69, FR-484, SC-328);
- can **never** make a memory eligible for cross-project promotion (FR-396, D62 check 2);
- is never re-executed or re-collected by Cairn — a recheck of attested evidence yields
  `needs_recheck`, not `verified`, until the agent attests again.

That last rule is what stops attested evidence from decaying into a permanent unfalsifiable claim.

## Verification authority

The state says *what* was established. The authority says *what established it*. Collapsing them would
let an agent's own assertion wear the same badge as a check Cairn performed — the one thing this
contract exists to prevent (FR-370).

Authority is a separate dimension, exactly as verification is separate from lifecycle. It is **derived**,
not asserted:

```text
authority(memory) =
    cairn            ≥1 verification run with result `verified` consulted evidence
                     whose collector is `cairn`
    attested         the memory is `verified`, and every run that established it
                     consulted only `collector = agent` evidence
    remote_cairn     the state was imported, and the peer's authority was `cairn`
    remote_attested  the state was imported, and the peer's authority was `attested`
    —                the memory is not `verified` (authority is meaningless)
```

The strongest basis wins: a memory supported by both an attested fact and a Cairn-read file digest,
verified by the digest, has authority `cairn`. The attested fact stays attached and stays labelled. A
deterministic check did establish the claim, so saying otherwise would understate what Cairn knows.

This replaces the earlier `verification_origin ∈ {local, remote}`, which distinguished *which machine*
but not *what kind of check* — so an attested `test_outcome` synced as
`{state: verified, basis: ["test_outcome"]}` and arrived indistinguishable from a peer that had really
run the tests. `basis` cannot close that gap, because `test_outcome` and `command_outcome` are each
reachable both ways (D76).

### What each consumer accepts

| Consumer | Accepts | Refuses |
|---|---|---|
| Memory `verified` state | `cairn`, `attested` | — |
| Subject representative ranking | both; `cairn` outranks `attested` | — |
| Task criterion `verified` | `cairn` only | `attested`, and **every** imported authority (FR-484) |
| Cross-project promotion | `cairn` only | `attested`, `remote_*` (FR-396) |
| Local completion readiness | `cairn` only | `attested`, `remote_*` |
| Context and search output | both, always labelled | — |

Criterion verification and promotion are the two places with an incentive attached, so both take the
strict setting. Everywhere else attested evidence is useful and is simply told apart.

### How it is reported

Never bare. Every surface that shows a verification state shows its authority:

```text
✓ verified                      configuration @ config/app.yml · 2026-08-14   (authority: cairn)
✓ verified (attested)           runtime_state · attested by session 0192f4…   (authority: attested)
✓ verified elsewhere            deterministic check on another machine        (authority: remote_cairn)
✓ verified elsewhere (attested) attested on another machine                   (authority: remote_attested)
```

The two `remote_*` renderings are what a teammate sees, and the distinction is exactly the information
FR-368 requires them to have.

### Refused

| Requested | Why |
|---|---|
| A path outside the worktree | FR-365 |
| A path matching an exclusion | FR-354, privacy |
| Running a build, test or shell command | FR-365, spec Out of Scope |
| A network or API call by Cairn | FR-365; the agent may attest the response instead |
| "The model believes this" | FR-361 — recorded as a proposal, never as evidence |

## The verification state machine

Total. Every transition names its trigger; nothing else moves the state (FR-375, SC-306).

| From | Trigger | To |
|---|---|---|
| `unverified` | run → `verified` | `verified` |
| `unverified` | run → `drifted` | `drifted` |
| `unverified` | run → `inconclusive` | `unverified` |
| `verified` | evidence fingerprint changed | `needs_recheck` |
| `verified` | contradicting evidence attached | `conflicted` |
| `needs_recheck` | run → `verified` | `verified` |
| `needs_recheck` | run → `drifted` | `drifted` |
| `needs_recheck` | run → `inconclusive` | `needs_recheck` |
| `needs_recheck` | last supporting evidence deleted | `needs_recheck` |
| `drifted` | evidence fingerprint changed | `needs_recheck` |
| `drifted` | run → `verified` | `verified` |
| `conflicted` | evidence fingerprint changed | `needs_recheck` |
| `conflicted` | the contradicting evidence removed | `needs_recheck` |
| any | imported from a peer | unchanged locally; authority becomes `remote_cairn` or `remote_attested` per the peer's value |

**Not transitions** — asserted unreachable:

- supersession → any verification change. A superseded memory keeps its last verification, which is
  what makes a historical query able to say what was verified then (D50).
- `stale` → any verification change. Scope staleness and verification are orthogonal.
- `drifted` → `verified` without a run. Drift is only cleared by evidence.
- any → any on a model's assertion.

`conflicted` here means **this memory's own evidence disagrees with itself**: supporting and
contradicting facts both attached, or two runs of the same verifier disagreeing at the same
`repo_commit`. It is not subject-level disagreement, which is a `SubjectView` state (FR-369).

**Authority under transition.** Authority is recomputed with the state, from the runs that established
it. A memory `verified` by attestation that later passes a Cairn-collected verifier becomes
`verified` with authority `cairn` — an upgrade, and the honest one. The reverse cannot happen: a
Cairn-established verification does not degrade to `attested` because someone attaches an attested
fact, since the deterministic run still stands. A memory leaving `verified` has no authority at all
until it is verified again.

## Drift

Two mechanisms, deliberately separate (D54).

### Marking — on the capture path, cheap

```text
file_changed observation stored
    ▼
SELECT id FROM evidence_facts
 WHERE project_id = ? AND source_locator = ?      -- indexed
 LIMIT evidence_lookups_per_event_max              -- 8
    ▼
for each: recompute fingerprint → if changed, set supported memories' verification
          to needs_recheck  (and nothing else)
```

Branch or commit change on a session marks commit-pinned facts for that branch the same way.

Constraints:

- exact locator equality only — no globbing, no prefix scan;
- at most 8 lookups per event; exceeding defers to the background pass and is **not** an error;
- writes exactly `verification` and `last_verified_at` on the memory — never content, type, scope,
  provenance or lifecycle state (FR-371, I6);
- inside Feature 001's 250 ms capture deadline, with its always-exit-0 fail-soft rule unchanged
  (FR-475).

### Verifying — on the existing maintenance tick, bounded

The 15-minute tick that already reaps idle sessions, sweeps owed handoffs and marks stale scopes
gains one pass:

| Cap | Default |
|---|---|
| evidence facts examined | `verify_pass_evidence_max` 200 |
| verifier runs | `verify_pass_runs_max` 50 |
| wall clock | `verify_pass_wall_ms` 2000 |
| concurrency | 1 |

Order: `needs_recheck` memories first, oldest `last_verified_at` first; within that, pinned before
unpinned, then `project` scope before narrower. Exceeding any cap yields; remaining work is picked up
next tick. Never blocks a request (FR-472).

On-demand paths: `cairn verify [--memory <id> | --task <id> | --all]` and
`cairn_remember action=verify`. Same caps, same verifiers, reported synchronously.

### What drift does not do

```text
evidence changed ──✗──▶ rewrite the memory
evidence changed ──✗──▶ create a superseding memory
evidence changed ──✗──▶ mark the memory stale or superseded
evidence changed ──✓──▶ verification = needs_recheck
```

A `drifted` memory stays lifecycle-`active`, stays returned by default retrieval, carries its warning
into Level 0 context, and never counts as verified for any derived readiness (FR-373). Creating the
replacement is an explicit act (FR-372).

Worked example — the brief's case:

```text
memory   topic_key: service.api_port   value_key: 8080   verification: verified
config/app.yml changes
  → fingerprint differs → verification: needs_recheck
background pass runs `configuration` verifier
  → observed 9000, expected 8080 → verification: drifted
context   ⚠ drift: service.api_port — remembered 8080, evidence at config/app.yml now 9000
agent     cairn_remember action=supersede memory_id=… topic_key=service.api_port value_key=9000
  → old: state superseded, verification drifted (kept, historically true)
  → new: state active,     verification unverified → verified on next pass
```

## Recording a verification

Every run records enough to answer FR-363's four questions:

| Question | Field |
|---|---|
| What was checked? | `verifier`, and the memory or criterion |
| Against which evidence? | `evidence_id`, `expected_digest`, `observed_digest` |
| When? | `checked_at` |
| At what project state? | `repo_branch`, `repo_commit` |
| What was the result? | `result`, `detail` |

Append-only (FR-364). `cairn verify --explain <memory-id>` prints the run history; only the memory's
cached `verification` and `last_verified_at` reflect the latest.

## Error codes

| Code | Meaning |
|---|---|
| `evidence_excluded` | The locator matches a privacy exclusion; nothing was stored |
| `evidence_outside_worktree` | The locator resolves outside the project worktree |
| `evidence_too_large` | The value exceeded its bound after redaction |
| `absolute_locator` | The locator was absolute |
| `verifier_unavailable` | No verifier exists for that evidence kind |
| `verification_inconclusive` | The check ran and could not establish either outcome |
| `attested_not_sufficient` | Attested evidence was offered where a deterministic check is required — a criterion's verification, or promotion |
| `imported_not_sufficient` | An imported verification was offered for a criterion's verification; readiness is a local claim |
| `verify_pass_yielded` | The bounded pass hit a cap; remaining work is queued |

`verification_inconclusive` and `verify_pass_yielded` are `ok: true` with a note — they are outcomes,
not failures (FR-366, FR-473).
