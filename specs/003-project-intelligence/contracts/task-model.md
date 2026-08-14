# Contract: Task Work State

**Feature**: `003-project-intelligence` — **secondary capability**

Tasks gain stable criterion identity, blockers, a revision counter and derived readiness. Nothing
else. No sprints, points, epics, boards, assignees, estimates, inter-task dependencies or reporting
(FR-491).

The reason this is in scope at all is a defect, not an ambition. Today `repo::update_task` writes
`title`, `goal`, `acceptance_criteria` and `status` in one statement, and `acceptance_criteria` is a
JSON array of plain strings with no identity. Two sessions editing different criteria lose one
another's work, on one machine and across sync (research B3).

```text
Task
├── stable identity                 (existing)
├── goal, title, status             (existing)
├── revision                        NEW — monotone, advances on any change
├── criteria[]                      NEW — stable ids, state ⊥ verification, evidence
├── blockers[]                      NEW — append-only, one cleared transition
├── change log                      NEW — append-only, local
├── progress                        DERIVED — counts, never a percentage
└── completion_readiness            DERIVED — never changes status
```

## Criteria

### Identity

`task_criteria.id` is a UUIDv7 and is **stable across every update** (FR-481). `label` is `AC-<n>`
derived from `ordinal` at creation and **not renumbered** when a criterion is added or removed —
renumbering would silently change what "AC-2" means in a handoff, a checkpoint or a session's memory.

```text
create task with 2 criteria   → AC-1 (ordinal 1), AC-2 (ordinal 2)
add a criterion               → AC-3 (ordinal 3)
remove AC-2                   → AC-1, AC-3 remain. AC-3 is not renamed.
```

### The retained projection

`tasks.acceptance_criteria` — Feature 001's JSON array of strings — is **retained** and rewritten in
the same transaction as any criterion change, as the ordinal-ordered list of `text` values (D68,
FR-492).

Five readers depend on it and none changes:

| Reader | Uses |
|---|---|
| `cairn-core/context.rs::admit_task` | `t.acceptance_criteria` for the briefing |
| `cairn task get/list` renderers | the array |
| `cairn-store/outbox.rs::task_payload` | the array in the sync payload |
| `cairn-server/sync.rs::upsert_task` | `acceptance_criteria JSONB` |
| `web/app/(app)/projects/[id]/tasks` | the array from the API |

Asserted by `rebuild_criteria_projection` equality (I11, SC-324). This is the feature's one retained
denormalization and it is recorded in the plan's Complexity Tracking.

### Two independent axes

| Axis | Values | Meaning |
|---|---|---|
| `state` | `pending \| satisfied \| blocked \| waived` | The **work** state a session asserts |
| `verification` | `unverified \| verified \| failed` | What **evidence** establishes |

They are never collapsed (FR-482). `satisfied` + `unverified` is a normal, separately reported
combination — it is the honest description of "the agent says it is done and nothing has checked"
(FR-483).

```text
state:        pending ⇄ satisfied,  pending ⇄ blocked,  any → waived
verification: unverified ⇄ verified,  unverified ⇄ failed
```

### Criterion verification requires Cairn-collected evidence

`verification = 'verified'` requires ≥1 `criterion_evidence` row whose evidence fact has
`collector = 'cairn'` (D69, FR-484). Attested evidence may be attached, is labelled, and leaves the
criterion `unverified`, refused with `attested_not_sufficient`.

This is not pedantry. Completion readiness is the one derived value with an incentive attached; if an
agent can attest its way to `verified`, readiness becomes self-certification. The path stays open
because Cairn *does* collect test and command outcomes itself through Feature 001's hooks:

```text
agent runs `cargo test --workspace`
  → hook records a test_run observation: command, outcome, exit code, commit
  → cairn_task action=update_criterion criterion_id=… state=satisfied \
        evidence_observation_id=<that observation>
  → evidence fact kind=test_outcome, collector=cairn
  → verifier test_outcome → verified
```

## Blockers

Append-only with one transition (FR-485):

```text
open → cleared     terminal. Reopening creates a new blocker.
```

Both ends are attributed: `opened_by_session`/`opened_at`, `cleared_by_session`/`cleared_at`. There is
no edit and no delete-and-recreate, so "who said this was blocked and who said it was not" is always
answerable.

## Revision and the change log

`tasks.revision` is a monotone integer advanced **in the same transaction** as any change to the
task, its criteria or its blockers (FR-488, I12).

Every such change also writes a `task_changes` row naming its author, the kind, and the prior and new
value. That is what makes FR-488's "no assertion is lost even when a later one replaces it" true, and
what supplies the *content* of a divergence report.

| `kind` | `subject_id` | `prior_value` → `new_value` |
|---|---|---|
| `goal_changed`, `title_changed`, `status_changed` | — | the values |
| `criterion_added`, `criterion_removed` | criterion id | the text |
| `criterion_text` | criterion id | the texts |
| `criterion_state` | criterion id | `pending` → `satisfied` |
| `criterion_verification` | criterion id | `unverified` → `verified` |
| `blocker_opened`, `blocker_cleared` | blocker id | the description |

The change log is **local** (FR-503). A peer receives the criteria and blockers themselves; the log is
provenance-rich diagnostics for the machine that produced it.

## Concurrency

### Different criteria — no interaction

Two sessions updating different criteria touch different rows. Both apply. `tasks.revision` advances
twice and both changes appear in the log (FR-490, SC-317). This is what B3's defect cost, removed by
construction.

### The same criterion — revision comparison

```text
cairn_task action=get                       → returns each criterion's `revision`
cairn_task action=update_criterion
    criterion_id=… state=satisfied
    expected_revision=<what you read>       → applies, or refuses `revision_conflict`
                                               naming the current state and revision
```

Supplying `expected_revision` is how a caller is protected (FR-337, FR-490). Omitting it applies the
write and records `blind_write = true` in the change log, which surfaces in `cairn task history` and
in diagnostics.

**Why this is not a silent overwrite.** Task work state legitimately has a current value — that is
what work state *is*. The invariant Feature 003 protects is that no assertion is lost and no
overwrite is invisible: the prior value, the author and the fact that no revision was supplied are all
recorded. A caller that reads before writing cannot be overwritten at all. Canonical project knowledge
gets the stronger guarantee — there, nothing is ever overwritten (FR-336), because knowledge does not
have a "current value" in the same sense.

The tool description and the usage contract both tell agents to pass `expected_revision`.

## Derived progress

Counts by state. There is no field in which to store a percentage, so an agent cannot write one
(FR-486, US11 #5):

```json
{
  "progress": {
    "verified": 3, "satisfied_unverified": 1, "blocked": 1, "pending": 2, "waived": 0,
    "total": 7
  },
  "open_blockers": 1,
  "completion_readiness": "not_ready"
}
```

Rendered:

```text
PROGRESS  3 verified · 1 satisfied but unverified · 1 blocked · 2 pending
BLOCKERS  1 open — "staging credentials expired"
READINESS not_ready
```

## Completion readiness

```text
ready             every non-waived criterion is `satisfied` AND `verified`,
                  AND no blocker is `open`
ready_unverified  every non-waived criterion is `satisfied`,
                  AND no blocker is `open`,
                  AND at least one is not `verified`
not_ready         otherwise
```

Derived on read, never stored as authority. **Cairn never changes `tasks.status`** on the basis of it
(FR-487). `status = 'done'` stays an explicit act through Feature 001's existing semantics, which are
unchanged.

A `drifted` supporting memory does not affect a criterion's verification directly, but a criterion
whose evidence fact drifted returns to `unverified` on the next pass, which moves readiness back —
which is correct.

## Task divergence for an older session

`sessions.task_revision_at_bind` records what a session bound at (FR-489). On context refresh:

```text
if session.task_revision_at_bind < tasks.revision:
    emit a Level 0 task-divergence warning with the change list from `task_changes`
```

Rendered:

```text
⚠ TASK UPDATED
  you started at revision 5 · current revision 6
  changes:
    • criterion added — AC-4 "production smoke passes"
    • blocker opened  — "staging credentials expired"
```

Cairn does not silently present the session as having worked against revision 6 (FR-489, SC-318). The
session's binding revision is not rewritten; only the report changes.

## Surfaces

### CLI

| Command | Notes |
|---|---|
| `cairn task get <id>` | Now includes `revision`, criteria with ids/labels/states/verification/evidence counts, blockers, progress, readiness |
| `cairn task criterion add <task-id> --text …` | |
| `cairn task criterion set <criterion-id> --state … [--expected-revision N]` | |
| `cairn task criterion verify <criterion-id> [--evidence <id>]` | |
| `cairn task criterion remove <criterion-id>` | Tombstone; ordinals are not renumbered |
| `cairn task blocker open <task-id> --description …` | |
| `cairn task blocker clear <blocker-id>` | |
| `cairn task readiness <id>` | Derived, with the counts |
| `cairn task history <id>` | The change log, including `blind_write` markers |

`cairn task update --acceptance-criteria …` — Feature 001's whole-list form — **still works**. It
diffs against the existing criteria by text, keeping ids for unchanged entries, adding for new ones
and tombstoning for removed ones, and logs each as its own change. A Feature 001 caller therefore
loses no work and gains identity for free.

### MCP

See [mcp-tools.md](./mcp-tools.md). `cairn_task` gains `add_criterion`, `update_criterion`, `blocker`
and `readiness`; `get` and `list` gain the new read-only fields.

## Error codes

| Code | Meaning |
|---|---|
| `revision_conflict` | `expected_revision` did not match; the current state and revision are named |
| `criterion_not_found` | |
| `blocker_not_found` | |
| `blocker_already_cleared` | The only transition has already happened |
| `attested_not_sufficient` | Attested evidence was offered for a criterion's verification (D69) |
| `criterion_waived` | A state or verification change was requested for a waived criterion |
