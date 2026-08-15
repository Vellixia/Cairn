"""Generate the `budget/` corpus.

The cases are **parameters**, not materialized briefings. A case names a memory
population, a budget, and how much Level 0 content exists; the suite builds that
assembly and asserts the properties. Writing 5,000 memories into JSON would make
the corpus unreadable and would assert nothing the parameters do not already say.

Two groups:

  * `budget/` — populations 0 x 10 x 500 x 5,000 crossed with budgets
    200 ... 12,000. Every case asserts `estimated_tokens <= budget`; the cases at
    the documented minimum assert Tier 0a is complete; the population-0 cases
    with no Level 0 content additionally assert the no-regression property
    (SC-308, SC-309, FR-445).
  * `budget/oversized_task/` — tasks of 5, 40 and 200 criteria whose text alone
    exceeds the whole budget. Tier 0a stays complete because it is O(1) in the
    size of the task; criterion text is admitted in action order until the budget
    binds, and what is dropped is counted by kind with a retrieval path
    (FR-443, FR-448, D83).
"""
import json, pathlib, sys

ROOT = pathlib.Path(sys.argv[1]) / "budget"
OVERSIZED = ROOT / "oversized_task"
ROOT.mkdir(parents=True, exist_ok=True)
OVERSIZED.mkdir(parents=True, exist_ok=True)

# The documented minimum. Below it a briefing is still produced, truncated in
# Level 0's admission order, and never rejected for size.
MIN_BUDGET = 600

POPULATIONS = [0, 10, 500, 5000]
BUDGETS = [200, 600, 1200, 3000, 6000, 12000]


def write(directory, i, slug, description, inp, expect):
    (directory / f"{i:03d}_{slug}.json").write_text(
        json.dumps(
            {
                "description": description,
                "input": {"extra": inp},
                "expect": {"extra": expect},
            },
            indent=2,
        )
        + "\n"
    )


# ---------------------------------------------------------------------------
# The population x budget matrix.
# ---------------------------------------------------------------------------

n = 0
for population in POPULATIONS:
    for budget in BUDGETS:
        n += 1
        # Level 0 content is present everywhere except the bare cases, which are
        # the ones that carry the no-regression assertion.
        bare = population == 0
        inp = {
            "memories": population,
            "budget": budget,
            "criteria": 0 if bare else 3,
            "blockers": 0 if bare else 1,
            "warnings": 0 if bare else 2,
            "pins": 0 if bare else 1,
            "task": not bare,
        }
        expect = {
            "within_budget": True,
            # Tier 0a is O(1), so it is complete at every budget at or above the
            # documented minimum, whatever the population.
            "tier_0a_complete": budget >= MIN_BUDGET and not bare,
            # With no task, no warnings, no pins and no checkpoint, the briefing
            # must be exactly what Feature 001 produced.
            "no_regression": bare,
        }
        if bare:
            description = (
                f"No task, no warnings, no pins and no checkpoint at a budget of "
                f"{budget}. The reserve is withheld and then released unspent, so "
                f"the briefing is byte-identical to the pre-feature baseline - the "
                f"reserve is a cap on the lower levels, never a floor Level 0 must "
                f"spend (FR-442)."
            )
        elif budget < MIN_BUDGET:
            description = (
                f"{population} memories at {budget} tokens, below the documented "
                f"minimum of {MIN_BUDGET}. The briefing is still produced and "
                f"truncated in Level 0's admission order - never rejected for size."
            )
        else:
            description = (
                f"{population} memories at {budget} tokens. Level 1 cannot displace "
                f"Level 0, and the measured cost never exceeds the budget."
            )
        write(ROOT, n, f"pop{population}_budget{budget}", description, inp, expect)

# ---------------------------------------------------------------------------
# Oversized tasks - the case the bounded guarantee exists for.
# ---------------------------------------------------------------------------

m = 0
for criteria in [5, 40, 200]:
    for budget in [200, MIN_BUDGET, 1200]:
        m += 1
        write(
            OVERSIZED,
            m,
            f"criteria{criteria}_budget{budget}",
            (
                f"A task of {criteria} acceptance criteria at {budget} tokens, "
                f"where the criterion text alone exceeds the whole budget. Tier 0a "
                f"stays complete because it is O(1) in the size of the task: the "
                f"agent still knows what it is doing, how far along it is and what "
                f"is blocking it. The text that does not fit is counted by kind "
                f"with a retrieval path, never dropped silently (FR-443, FR-448)."
            ),
            {
                "memories": 50,
                "budget": budget,
                "criteria": criteria,
                # Long enough that the text alone cannot fit any of these budgets.
                "criterion_text_tokens": 40,
                "blockers": 2,
                "warnings": 1,
                "pins": 1,
                "task": True,
            },
            {
                "within_budget": True,
                "tier_0a_complete": budget >= MIN_BUDGET,
                "omissions_reported": True,
                "no_regression": False,
            },
        )

print(f"wrote {n} matrix cases and {m} oversized-task cases")
