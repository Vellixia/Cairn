"""Generate the `tasks/` corpus.

Three families in one flat directory, matching the `drift/` convention:

  * the criterion **state x verification** matrix — which bucket of derived
    progress each of the twelve combinations lands in, and in particular that
    `satisfied` + `unverified` is counted separately from `verified` (FR-482,
    FR-483);
  * derived **readiness** — including the cases where a waived criterion is
    excluded, an open blocker overrides everything, and a `failed` check reads
    as "not verified" rather than as its own readiness state (FR-486, FR-487);
  * the **action order** Level 0 consumes (FR-482, `contracts/continuity-context.md`).

A case names its criteria by ordinal, so ties never fall through to the id and
the expected order is fully determined by the contract.
"""
import json, pathlib, sys

ROOT = pathlib.Path(sys.argv[1]) / "tasks"
ROOT.mkdir(parents=True, exist_ok=True)

_n = 0

def crit(ordinal, state, verification, deleted=False):
    return {
        "ordinal": ordinal,
        "label": f"AC-{ordinal}",
        "text": f"criterion {ordinal}",
        "state": state,
        "verification": verification,
        "deleted": deleted,
    }

def blocker(state, deleted=False):
    return {"state": state, "deleted": deleted}

def write(slug, description, criteria, blockers, expect):
    global _n
    _n += 1
    (ROOT / f"{_n:03d}_{slug}.json").write_text(json.dumps({
        "description": description,
        "input": {"extra": {"criteria": criteria, "blockers": blockers}},
        "expect": {"extra": expect},
    }, indent=2) + "\n")

def progress(verified=0, satisfied_unverified=0, blocked=0, pending=0, waived=0):
    return {
        "verified": verified,
        "satisfied_unverified": satisfied_unverified,
        "blocked": blocked,
        "pending": pending,
        "waived": waived,
        "total": verified + satisfied_unverified + blocked + pending + waived,
    }

# ---------------------------------------------------------------------------
# The state x verification matrix. One criterion per case, so the bucket it
# lands in is unambiguous.
# ---------------------------------------------------------------------------

MATRIX = [
    ("pending", "unverified", "pending",
     "Nothing asserted and nothing checked. The ordinary starting state."),
    ("pending", "verified", "pending",
     "Evidence establishes something, but no session has asserted the work is "
     "done. The axes are independent, so this counts as pending work - "
     "verification does not assert it (FR-482)."),
    ("pending", "failed", "pending",
     "A check failed while the work state is still pending. The failure is "
     "carried on the verification axis and does not invent a work state."),
    ("satisfied", "unverified", "satisfied_unverified",
     "The agent says it is done and nothing has checked. Counted separately "
     "from verified rather than folded into it - this is the combination the "
     "separate bucket exists for (FR-483)."),
    ("satisfied", "verified", "verified",
     "Asserted done and established by a Cairn-collected check. The only "
     "combination that counts as verified."),
    ("satisfied", "failed", "satisfied_unverified",
     "The agent says it is done and the check disagreed. It is not verified, "
     "so it does not count as verified; the disagreement is visible on the "
     "verification axis and is never silently promoted."),
    ("blocked", "unverified", "blocked",
     "Work is blocked. Blocked is reported on its own, never as a kind of "
     "pending."),
    ("blocked", "verified", "blocked",
     "Evidence established the claim while the work state says blocked. The "
     "work state is what progress counts, so it reports blocked."),
    ("blocked", "failed", "blocked",
     "Blocked with a failed check. Still one blocked criterion, not two "
     "signals."),
    ("waived", "unverified", "waived",
     "Waived work is counted on its own and excluded from readiness."),
    ("waived", "verified", "waived",
     "A waived criterion that happens to carry a verification stays waived. "
     "Waiving is terminal."),
    ("waived", "failed", "waived",
     "A waived criterion with a failed check stays waived and never "
     "reappears as work."),
]

for state, verification, bucket, description in MATRIX:
    write(
        f"matrix_{state}_{verification}",
        description,
        [crit(1, state, verification)],
        [],
        {"progress": progress(**{bucket: 1}), "readiness": None, "action_order": None},
    )

# ---------------------------------------------------------------------------
# Derived readiness.
# ---------------------------------------------------------------------------

write(
    "readiness_every_criterion_verified",
    "Every non-waived criterion is satisfied and verified and no blocker is "
    "open. This is the only route to `ready` - and reaching it still changes "
    "no task status (FR-487).",
    [crit(1, "satisfied", "verified"), crit(2, "satisfied", "verified")],
    [],
    {"progress": progress(verified=2), "readiness": "ready", "action_order": None},
)

write(
    "readiness_one_criterion_unverified",
    "All the work is asserted done and one criterion has nothing that checked "
    "it. Reported as ready_unverified rather than ready: the difference "
    "between an assertion and a check is the whole point (FR-483).",
    [crit(1, "satisfied", "verified"), crit(2, "satisfied", "unverified")],
    [],
    {
        "progress": progress(verified=1, satisfied_unverified=1),
        "readiness": "ready_unverified",
        "action_order": None,
    },
)

write(
    "readiness_a_failed_check_is_not_verified",
    "A satisfied criterion whose check failed is not verified, so readiness is "
    "ready_unverified and never ready. The failure does not move the work "
    "axis - the two axes stay independent (FR-482).",
    [crit(1, "satisfied", "verified"), crit(2, "satisfied", "failed")],
    [],
    {
        "progress": progress(verified=1, satisfied_unverified=1),
        "readiness": "ready_unverified",
        "action_order": None,
    },
)

write(
    "readiness_a_pending_criterion_blocks_it",
    "One criterion nobody has asserted is enough for not_ready, whatever the "
    "others say.",
    [crit(1, "satisfied", "verified"), crit(2, "pending", "unverified")],
    [],
    {
        "progress": progress(verified=1, pending=1),
        "readiness": "not_ready",
        "action_order": None,
    },
)

write(
    "readiness_a_blocked_criterion_blocks_it",
    "A blocked criterion is not satisfied, so the task is not ready.",
    [crit(1, "satisfied", "verified"), crit(2, "blocked", "unverified")],
    [],
    {
        "progress": progress(verified=1, blocked=1),
        "readiness": "not_ready",
        "action_order": None,
    },
)

write(
    "readiness_an_open_blocker_overrides_everything",
    "Every criterion is satisfied and verified and a blocker is still open. "
    "An open blocker is decisive on its own: readiness is a claim about "
    "whether the task can be completed, and an open blocker says it cannot.",
    [crit(1, "satisfied", "verified")],
    [blocker("open")],
    {"progress": progress(verified=1), "readiness": "not_ready", "action_order": None},
)

write(
    "readiness_a_cleared_blocker_does_not",
    "The same task once the blocker is cleared. A cleared blocker is retained "
    "for attribution and stops holding readiness down (FR-485).",
    [crit(1, "satisfied", "verified")],
    [blocker("cleared")],
    {"progress": progress(verified=1), "readiness": "ready", "action_order": None},
)

write(
    "readiness_waived_criteria_are_excluded",
    "A waived criterion is excluded from readiness rather than counted as "
    "unsatisfied. Waiving is how work is taken out of scope without "
    "pretending it was done.",
    [crit(1, "satisfied", "verified"), crit(2, "waived", "unverified")],
    [],
    {
        "progress": progress(verified=1, waived=1),
        "readiness": "ready",
        "action_order": None,
    },
)

write(
    "readiness_all_waived",
    "Every criterion waived leaves nothing to consider, and no blocker is "
    "open, so the task is ready. Vacuous, and deliberately so: the alternative "
    "would leave a task permanently unable to be reported ready after its "
    "scope was cut.",
    [crit(1, "waived", "unverified"), crit(2, "waived", "unverified")],
    [],
    {"progress": progress(waived=2), "readiness": "ready", "action_order": None},
)

write(
    "readiness_no_criteria_at_all",
    "A task with no criteria has nothing unsatisfied and no open blocker. "
    "Ready is the honest answer; it still changes no status.",
    [],
    [],
    {"progress": progress(), "readiness": "ready", "action_order": None},
)

write(
    "readiness_a_removed_criterion_does_not_count",
    "A tombstoned criterion is excluded from both progress and readiness. "
    "Removal tombstones rather than deletes, so the id and its history "
    "survive - but the criterion is no longer work (FR-481).",
    [crit(1, "satisfied", "verified"), crit(2, "pending", "unverified", deleted=True)],
    [],
    {"progress": progress(verified=1), "readiness": "ready", "action_order": ["AC-1"]},
)

write(
    "readiness_a_removed_blocker_does_not_count",
    "A tombstoned blocker holds nothing down.",
    [crit(1, "satisfied", "verified")],
    [blocker("open", deleted=True)],
    {"progress": progress(verified=1), "readiness": "ready", "action_order": None},
)

# ---------------------------------------------------------------------------
# Action order - what Level 0 admits first.
# ---------------------------------------------------------------------------

write(
    "action_order_full_precedence",
    "blocked, then satisfied-but-unverified, then pending, then verified, then "
    "waived. The ones an agent must act on arrive first: blocked is what stops "
    "progress and satisfied-but-unverified is what needs a check, while "
    "verified and waived are what an agent least needs to re-read.",
    [
        crit(1, "waived", "unverified"),
        crit(2, "satisfied", "verified"),
        crit(3, "pending", "unverified"),
        crit(4, "satisfied", "unverified"),
        crit(5, "blocked", "unverified"),
    ],
    [],
    {
        "progress": progress(verified=1, satisfied_unverified=1, blocked=1, pending=1, waived=1),
        "readiness": "not_ready",
        "action_order": ["AC-5", "AC-4", "AC-3", "AC-2", "AC-1"],
    },
)

write(
    "action_order_ties_break_by_ordinal",
    "Criteria at the same rank keep their ordinal order, so the sequence is "
    "stable across reads and a label never moves for a reason the reader "
    "cannot see.",
    [
        crit(1, "pending", "unverified"),
        crit(2, "pending", "unverified"),
        crit(3, "pending", "unverified"),
    ],
    [],
    {
        "progress": progress(pending=3),
        "readiness": "not_ready",
        "action_order": ["AC-1", "AC-2", "AC-3"],
    },
)

write(
    "action_order_a_failed_check_ranks_as_unverified",
    "A satisfied criterion whose check failed ranks with satisfied-but-"
    "unverified, ahead of a verified one - it is exactly the criterion an "
    "agent needs to look at.",
    [crit(1, "satisfied", "verified"), crit(2, "satisfied", "failed")],
    [],
    {
        "progress": progress(verified=1, satisfied_unverified=1),
        "readiness": "ready_unverified",
        "action_order": ["AC-2", "AC-1"],
    },
)

write(
    "action_order_removed_criteria_are_absent",
    "A tombstoned criterion is never admitted, whatever its rank would have "
    "been.",
    [
        crit(1, "pending", "unverified"),
        crit(2, "blocked", "unverified", deleted=True),
        crit(3, "satisfied", "unverified"),
    ],
    [],
    {
        "progress": progress(satisfied_unverified=1, pending=1),
        "readiness": "not_ready",
        "action_order": ["AC-3", "AC-1"],
    },
)

print(f"wrote {_n} cases to {ROOT}")
