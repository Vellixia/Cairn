"""Generate the `merge/` corpus.

Every scenario has a **clock-reversed twin**: the same offline writes, with the
two machines' clocks swapped. The pair is the assertion. If any merge step
consulted a timestamp to choose a winner, the twin would produce a different
answer — so a byte-identical result across the pair is what proves no clock
decides anything (FR-411, SC-304).

Three groups:

  * `merge/` — two-store offline scenarios over canonical knowledge;
  * `merge/symmetric_relation/` — the same conflict detected independently on
    both stores, which must converge to exactly **one** durable relation (D78);
  * `merge/task_divergence/` — different criteria changed offline on each, which
    must both survive and leave the two machines computing an identical
    `task_state_digest` (SC-330).
"""
import json, pathlib, sys

ROOT = pathlib.Path(sys.argv[1]) / "merge"
SYMMETRIC = ROOT / "symmetric_relation"
TASKS = ROOT / "task_divergence"
for d in (ROOT, SYMMETRIC, TASKS):
    d.mkdir(parents=True, exist_ok=True)

EARLY = "2026-01-01T00:00:00Z"
LATE = "2026-06-01T00:00:00Z"

_n = {"merge": 0, "sym": 0, "task": 0}


def write(directory, key, slug, description, extra, expect):
    _n[key] += 1
    (directory / f"{_n[key]:03d}_{slug}.json").write_text(
        json.dumps(
            {
                "description": description,
                "input": {"extra": extra},
                "expect": {"extra": expect},
            },
            indent=2,
        )
        + "\n"
    )


def twin(directory, key, slug, description, extra, expect):
    """A scenario and its clock-reversed twin.

    The twin swaps which store holds the earlier clock and nothing else. Both
    must produce the same merged state.
    """
    write(directory, key, slug, description, {**extra, "clocks": [EARLY, LATE]}, expect)
    write(
        directory,
        key,
        f"{slug}_clock_reversed",
        description
        + " The same scenario with the two machines' clocks swapped: identical "
        "merged state, because nothing in the merge consults a clock.",
        {**extra, "clocks": [LATE, EARLY]},
        expect,
    )


# ---------------------------------------------------------------------------
# Canonical knowledge merged from two offline stores.
# ---------------------------------------------------------------------------

twin(
    ROOT, "merge", "disjoint_proposals_both_survive",
    "Two machines each propose a different value for the same subject while "
    "offline. Both proposals survive with their provenance; neither is "
    "overwritten, because there is no canonical row to overwrite (FR-336).",
    {
        "store_a": {"proposals": [{"topic": "deploy.queue_backend", "value": "sqs"}]},
        "store_b": {"proposals": [{"topic": "deploy.queue_backend", "value": "rabbitmq"}]},
    },
    {"memories": 2, "reconciliation": "conflicted", "relations": 0},
)

twin(
    ROOT, "merge", "the_same_value_from_both_corroborates",
    "Two machines independently record the same value, in their own words. That "
    "is corroboration, not a conflict, and it is reported as a count rather than "
    "a warning - a warning here would train people to ignore warnings. Whether "
    "two members are one claim or several is decided by content and never by the "
    "value key (FR-327, D77), which is what separates this case from its "
    "identical-content sibling below.",
    {
        "store_a": {"proposals": [{
            "topic": "infra.db", "value": "postgresql",
            "content": "The primary datastore is PostgreSQL",
        }]},
        "store_b": {"proposals": [{
            "topic": "infra.db", "value": "postgresql",
            "content": "We run Postgres for everything durable",
        }]},
    },
    {"memories": 2, "reconciliation": "corroborated", "relations": 0},
)

twin(
    ROOT, "merge", "the_same_statement_from_both_reinforces",
    "The sibling of the case above, differing in exactly the way that matters: "
    "the two machines wrote the *same statement*, not merely the same value. "
    "Identical content after normalization is the one merging case Cairn can "
    "decide without inference (D46), so this is one reinforced claim rather than "
    "several corroborating ones.",
    {
        "store_a": {"proposals": [{
            "topic": "infra.db", "value": "postgresql",
            "content": "The primary datastore is PostgreSQL",
        }]},
        "store_b": {"proposals": [{
            "topic": "infra.db", "value": "postgresql",
            "content": "the primary datastore is postgresql.",
        }]},
    },
    {"memories": 2, "reconciliation": "reinforced", "relations": 0},
)

twin(
    ROOT, "merge", "a_supersession_decided_elsewhere_lands",
    "Machine A records a supersession while B holds both memories unsuperseded. "
    "The decision travels and B re-derives from it. This is the defect research "
    "B2 found: importing only the memory row left the decision stranded, so a "
    "supersession decided on another machine never landed (D67, R5).",
    {
        "store_a": {
            "proposals": [
                {"topic": "api.port", "value": "8080"},
                {"topic": "api.port", "value": "9000"},
            ],
            "relations": [{"kind": "supersedes", "from": 1, "to": 0}],
        },
        "store_b": {"proposals": []},
    },
    # Two relations, not one. Proposing 9000 for a subject that already held
    # 8080 is a conflict, and Cairn detects that itself the moment the second
    # proposal is written. The supersession is the *answer* to that conflict,
    # recorded afterwards - and because relations are append-only, both survive:
    # the record of what disagreed, and the record of how it was resolved.
    {"memories": 2, "reconciliation": "settled", "relations": 2, "superseded": 1},
)

twin(
    ROOT, "merge", "a_local_only_memory_never_travels",
    "A memory marked local_only stays put, whatever else merges. The boundary "
    "is the memory's own flag, checked where the payload is built.",
    {
        "store_a": {"proposals": [{"topic": "infra.db", "value": "postgresql", "local_only": True}]},
        "store_b": {"proposals": []},
    },
    {"memories_on_b": 0},
)

# ---------------------------------------------------------------------------
# The same conflict, detected independently on both stores.
# ---------------------------------------------------------------------------

twin(
    SYMMETRIC, "sym", "one_conflict_detected_twice",
    "Both machines notice the same disagreement while offline and each records "
    "a `conflicts_with` relation. The endpoints are normalized before the key is "
    "written, so the two decisions are the same primary key and the merge "
    "absorbs the second exactly as it absorbs a local duplicate - converging to "
    "exactly one durable relation, not two mirror images (D78, SC-304).",
    {
        "store_a": {
            "proposals": [
                {"topic": "deploy.queue_backend", "value": "sqs"},
                {"topic": "deploy.queue_backend", "value": "rabbitmq"},
            ],
            "relations": [{"kind": "conflicts_with", "from": 0, "to": 1}],
        },
        "store_b": {
            "relations": [{"kind": "conflicts_with", "from": 1, "to": 0}],
        },
    },
    {"relations": 1, "reconciliation": "conflicted"},
)

# ---------------------------------------------------------------------------
# Different criteria changed offline on each machine.
# ---------------------------------------------------------------------------

twin(
    TASKS, "task", "different_criteria_both_land",
    "A holds AC-1 and moves it to satisfied; B holds AC-2 and moves it to "
    "satisfied. Both machines end with both changes, because criteria upsert by "
    "stable id and different criteria are different rows. The two local "
    "counters differ and are never compared; both machines compute the same "
    "task_state_digest (FR-490, FR-493, SC-330).",
    {
        "task": {"criteria": ["one", "two"]},
        "store_a": {"criterion_changes": [{"label": "AC-1", "state": "satisfied"}]},
        "store_b": {"criterion_changes": [{"label": "AC-2", "state": "satisfied"}]},
    },
    {
        "criteria_satisfied": ["AC-1", "AC-2"],
        "digests_match": True,
        "counters_compared": False,
    },
)

twin(
    TASKS, "task", "a_blocker_opened_on_one_machine",
    "B opens a blocker offline while A satisfies a criterion. Both arrive, and "
    "readiness on both machines reflects both - an open blocker is decisive "
    "wherever it was opened.",
    {
        "task": {"criteria": ["one"]},
        "store_a": {"criterion_changes": [{"label": "AC-1", "state": "satisfied"}]},
        "store_b": {"blockers": ["staging credentials expired"]},
    },
    {"digests_match": True, "readiness": "not_ready"},
)

twin(
    TASKS, "task", "the_same_criterion_changed_on_both",
    "Both machines move the same criterion. One value ends up current - that is "
    "what work state is - and neither assertion is lost: both are in each "
    "machine's local change log, and the digests agree once the rows converge.",
    {
        "task": {"criteria": ["one"]},
        "store_a": {"criterion_changes": [{"label": "AC-1", "state": "satisfied"}]},
        "store_b": {"criterion_changes": [{"label": "AC-1", "state": "blocked"}]},
    },
    {"digests_match": True, "assertions_lost": 0},
)

print(
    f"wrote {_n['merge']} merge cases, {_n['sym']} symmetric-relation cases, "
    f"{_n['task']} task-divergence cases"
)
