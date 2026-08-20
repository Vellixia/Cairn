"""Generate the `patterns/` corpus (T114, T115).

Four directories, each with one rule:

  * `promote/`         — candidates the ten-check gate must pass;
  * `refuse/`          — **one case per refusal class**, in gate order, plus the
                         two authority refusals that are the ways an agent could
                         otherwise launder its own claim into cross-project
                         knowledge (SC-328);
  * `independence/`    — what does and does not count towards trust (SC-314);
  * `counterexample/`  — a negative outcome that contests a pattern without
                         deleting it or decreasing anything (SC-313).

The privacy half of the refusal set lives in `patterns/../privacy/`, seeded by
T005: thirty adversarial cases, one per shape the redactor knows. `refuse/` here
carries **one** representative secret and one representative identifier so the
gate's ordering is testable in isolation; `privacy_promotion.rs` runs the full
thirty.
"""
import json, pathlib, sys

ROOT = pathlib.Path(sys.argv[1]) / "patterns"
DIRS = {name: ROOT / name for name in ("promote", "refuse", "independence", "counterexample")}
for d in DIRS.values():
    d.mkdir(parents=True, exist_ok=True)

_n = {name: 0 for name in DIRS}


def write(where, slug, description, given, expect):
    _n[where] += 1
    (DIRS[where] / f"{_n[where]:03d}_{slug}.json").write_text(
        json.dumps(
            {
                "description": description,
                "input": {"extra": given},
                "expect": {"extra": expect},
            },
            indent=2,
        )
        + "\n"
    )


# A source memory that satisfies every check. Cases below vary exactly one thing
# from it, which is what makes each one a test of a single gate check rather
# than of a whole scenario.
def source(**overrides):
    base = {
        "state": "active",
        "verification": "verified",
        "verification_authority": "cairn",
        "evidence_facts": 1,
        "local_only": False,
        "subject_reconciliation": "settled",
        "type": "procedure",
    }
    base.update(overrides)
    return base


def candidate(**overrides):
    base = {
        "title": "Docker cannot allocate a non-overlapping bridge network",
        "problem": (
            "Container creation fails because Docker cannot allocate a bridge "
            "subnet that does not overlap an existing one."
        ),
        "signals": [
            "could not find an available non-overlapping ipv4 address pool",
            "docker bridge network create failure",
        ],
        "applicability": [
            "Docker bridge networking in use",
            "the error names address-pool allocation",
            "the configured default address pools are actually exhausted",
        ],
        "root_cause": "The daemon's default-address-pools are fully allocated.",
        "approach": (
            "Expand default-address-pools in the daemon configuration and restart."
        ),
        "constraints": [
            "existing networks are not migrated to the new pool",
            "verify the ranges are genuinely exhausted before changing daemon config",
        ],
    }
    base.update(overrides)
    return base


# ---------------------------------------------------------------------------
# promote/ — what the gate must let through
# ---------------------------------------------------------------------------

write(
    "promote", "a_verified_procedure_promotes",
    "The brief's own case. A procedure, verified by a deterministic Cairn check, "
    "evidence-backed, active, unconflicted, with two specific signals and no "
    "project identifier anywhere in it. Every check passes and the pattern is "
    "written with an opaque origin reference (FR-395).",
    {"source": source(), "candidate": candidate()},
    {
        "promoted": True,
        "trust": "sanitized",
        "origin_ref_opaque": True,
        "project_id_column_exists": False,
        "sanitization_report_names_values": False,
    },
)

write(
    "promote", "a_failure_promotes",
    "A `failure` memory is transferable for the same reason a `procedure` is: it "
    "describes something going wrong and how it was resolved, not what this "
    "project happens to be configured with.",
    {
        "source": source(type="failure"),
        "candidate": candidate(
            title="A test suite deadlocks on a pool query inside an open transaction",
            problem="A query taken from the pool while a transaction is held blocks until the pool times out.",
            signals=["pool timed out while waiting for connection", "test hangs then fails far from the cause"],
            root_cause="The code path takes a second connection while holding the first.",
            approach="Pass the open connection down instead of reaching for the pool.",
        ),
    },
    {"promoted": True, "trust": "sanitized"},
)

write(
    "promote", "a_convention_without_a_configuration_topic_promotes",
    "A `convention` is transferable only when it is not bound to project "
    "configuration — the gate reads its topic key to decide, rather than the "
    "type alone (check 6).",
    {
        "source": source(type="convention", topic_key=None),
        "candidate": candidate(
            title="Name a migration after what it does, not when it ran",
            problem="Timestamp-named migrations make review order meaningless.",
            signals=["migration named only by timestamp", "reviewer cannot tell what a migration does"],
            root_cause="The generator's default name carries no intent.",
            approach="Rename migrations to state their effect and keep the ordinal prefix.",
        ),
    },
    {"promoted": True, "trust": "sanitized"},
)

# ---------------------------------------------------------------------------
# refuse/ — one per class, in gate order
# ---------------------------------------------------------------------------

REFUSALS = [
    (
        "a_superseded_source_is_not_promotable",
        "source_not_active",
        "Promoting a memory this project has already replaced would export a "
        "conclusion its own project no longer holds.",
        {"source": source(state="superseded"), "candidate": candidate()},
    ),
    (
        "an_unverified_source_is_not_promotable",
        "source_unverified",
        "Nothing checked it. Cross-project knowledge is the furthest-travelling "
        "thing Cairn produces and it starts from a check, not from a claim.",
        {"source": source(verification="unverified", verification_authority=None), "candidate": candidate()},
    ),
    (
        "an_attested_source_is_not_promotable",
        "attested_not_sufficient",
        "The source **is** verified — by the agent's own attestation. This is one "
        "of the two ways an agent could launder its own claim into shared "
        "knowledge: attest a fact, watch it reach `verified`, promote it. "
        "Promotion requires a deterministic check Cairn ran itself (FR-370, "
        "SC-328).",
        {"source": source(verification_authority="attested"), "candidate": candidate()},
    ),
    (
        "an_imported_verification_is_not_promotable",
        "imported_not_sufficient",
        "The other way. `remote_cairn` means another machine checked it against "
        "evidence this machine cannot see, so this machine cannot vouch for it "
        "and must not export it as though it could (FR-368, SC-328).",
        {"source": source(verification_authority="remote_cairn"), "candidate": candidate()},
    ),
    (
        "a_source_with_no_evidence_fact_is_not_promotable",
        "no_evidence",
        "A verification with nothing behind it is a state without a reason.",
        {"source": source(evidence_facts=0), "candidate": candidate()},
    ),
    (
        "a_local_only_source_never_leaves",
        "local_only_memory",
        "The memory was marked never to travel. A pattern derived from it would "
        "be that memory travelling under another name.",
        {"source": source(local_only=True), "candidate": candidate()},
    ),
    (
        "a_conflicted_subject_is_not_promotable",
        "source_conflicted",
        "The project itself does not agree on this. Exporting one side of an "
        "unresolved disagreement would state as settled what is not.",
        {"source": source(subject_reconciliation="conflicted"), "candidate": candidate()},
    ),
    (
        "a_fact_is_never_transferable",
        "not_transferable",
        "The check that earns its place. \"The production database is "
        "PostgreSQL\" is true, verified, evidence-backed — and says nothing any "
        "other project can use. A pattern describes a problem and its "
        "resolution, never a project's configuration.",
        {
            "source": source(type="fact"),
            "candidate": candidate(
                title="The production database is PostgreSQL",
                problem="Knowing which database production runs.",
                signals=["which database does production use", "database backend question"],
                root_cause="The project chose PostgreSQL.",
                approach="Use PostgreSQL.",
            ),
        },
    ),
    (
        "a_surviving_secret_refuses_without_echoing_it",
        "possible_secret",
        "One representative case; the full thirty adversarial shapes live in "
        "`privacy/` and are run by `privacy_promotion.rs`. What matters here is "
        "the ordering: the secret scan runs before the identifier scan, and the "
        "refusal never repeats the value it found (FR-397).",
        {
            "source": source(),
            "candidate": candidate(
                approach="Expand default-address-pools and restart. token=sk-CORPUSFIXTURE-not-a-real-key"
            ),
        },
    ),
    (
        "a_project_identifier_refuses",
        "project_identifying",
        "An absolute path names a machine and a person. The pattern would carry "
        "both into every other project that ever saw it.",
        {
            "source": source(),
            "candidate": candidate(
                approach="Edit /Users/dev/src/helios-ledger/daemon.json and restart."
            ),
        },
    ),
    (
        "one_signal_is_not_specific_enough",
        "insufficient_specificity",
        "A pattern that matches on one token matches too much. Two is the "
        "minimum a signal set may normalize to (`pattern_signals_min`).",
        {
            "source": source(),
            "candidate": candidate(signals=["docker network failure"]),
        },
    ),
    (
        "a_duplicate_pattern_refuses",
        "duplicate_pattern",
        "The same signals and the same root cause is the same pattern. The "
        "unique index on `(signal_digest, root_cause_digest)` makes this "
        "structural, so a race cannot produce what the check refuses.",
        {
            "source": source(),
            "candidate": candidate(),
            "already_promoted": candidate(),
        },
    ),
]

for slug, refusal, why, given in REFUSALS:
    write(
        "refuse", slug, why, given,
        {
            "promoted": False,
            "refusal": refusal,
            "partial_pattern_written": False,
            "echoes_offending_value": False,
        },
    )

# ---------------------------------------------------------------------------
# independence/ — what counts, and what only looks like it counts
# ---------------------------------------------------------------------------

write(
    "independence", "one_project_ten_sessions_counts_once",
    "Ten sessions in one project describing one incident. The unique key "
    "`(pattern_id, project_id, signal_digest)` is the whole anti-poisoning "
    "mechanism: they are one row, the distinct-project count is 1, and "
    "repetition has bought nothing (FR-402, SC-314).",
    {
        "applications": [
            {"project": "A", "sessions": 10, "outcome": "resolved",
             "discovery": "independent", "evidence": True, "is_origin": False}
        ]
    },
    {
        "rows": 1,
        "applications": 1,
        "distinct_projects_applied": 1,
        "qualifying_successes": 1,
        "trust": "validated",
    },
)

write(
    "independence", "three_projects_one_session_each",
    "The same total number of sessions, spread across three projects. This is "
    "what independent corroboration looks like, and the counters say so.",
    {
        "applications": [
            {"project": p, "sessions": 1, "outcome": "resolved",
             "discovery": "independent", "evidence": True, "is_origin": False}
            for p in ("A", "B", "C")
        ]
    },
    {
        "rows": 3,
        "distinct_projects_applied": 3,
        "qualifying_successes": 3,
        "trust": "validated",
    },
)

write(
    "independence", "the_origin_project_cannot_validate_its_own_pattern",
    "An application in the project the pattern came from is history, not "
    "evidence. `is_origin` is excluded from trust, so a pattern cannot walk "
    "itself up the ladder at home (FR-402).",
    {
        "applications": [
            {"project": "origin", "sessions": 3, "outcome": "resolved",
             "discovery": "independent", "evidence": True, "is_origin": True}
        ]
    },
    {
        "rows": 1,
        "distinct_projects_applied": 1,
        "qualifying_successes": 0,
        "trust": "sanitized",
    },
)

write(
    "independence", "cairn_suggested_without_evidence_does_not_validate",
    "An agent read Cairn's own suggestion and agreed with it. That is Cairn "
    "confirming Cairn. It counts as an application and not as a validation; "
    "with deterministic local evidence it would count as both (FR-403).",
    {
        "applications": [
            {"project": "B", "sessions": 1, "outcome": "resolved",
             "discovery": "cairn_suggested", "evidence": False, "is_origin": False}
        ]
    },
    {
        "rows": 1,
        "applications": 1,
        "distinct_projects_applied": 1,
        "qualifying_successes": 0,
        "trust": "sanitized",
    },
)

write(
    "independence", "cairn_suggested_with_local_evidence_does_validate",
    "The sibling of the case above, differing in exactly the way that matters. "
    "The agent was shown the pattern **and** collected deterministic evidence in "
    "its own project, which is confirmation rather than agreement.",
    {
        "applications": [
            {"project": "B", "sessions": 1, "outcome": "resolved",
             "discovery": "cairn_suggested", "evidence": True, "is_origin": False}
        ]
    },
    {"rows": 1, "qualifying_successes": 1, "trust": "validated"},
)

# ---------------------------------------------------------------------------
# counterexample/ — a negative outcome contests, never deletes
# ---------------------------------------------------------------------------

write(
    "counterexample", "a_not_applicable_outcome_contests_the_pattern",
    "Project B saw the same symptom from a different cause. The pattern is "
    "retained, nothing is decreased, and its trust becomes `contested` — which "
    "is evaluated **before** `validated`, so a pattern carrying both a success "
    "and a counterexample reports both sides (FR-404, FR-405, D64).",
    {
        "applications": [
            {"project": "B", "sessions": 1, "outcome": "resolved",
             "discovery": "independent", "evidence": True, "is_origin": False},
            {"project": "C", "sessions": 1, "outcome": "not_applicable",
             "discovery": "independent", "evidence": True, "is_origin": False,
             "alternative_cause": "A VPN route collision produced the same symptom."},
        ]
    },
    {
        "trust": "contested",
        "qualifying_successes": 1,
        "counterexamples": 1,
        "deleted": False,
        "successes_decreased": False,
        "suggestion_carries_alternative_cause": True,
        "suggestion_carries_check_this_first": True,
    },
)

write(
    "counterexample", "a_failed_outcome_also_contests",
    "`failed` and `not_applicable` are both counterexamples: the approach was "
    "tried and did not resolve it, or it did not apply. Either way the pattern "
    "must stop presenting itself as settled.",
    {
        "applications": [
            {"project": "C", "sessions": 1, "outcome": "failed",
             "discovery": "independent", "evidence": True, "is_origin": False}
        ]
    },
    {"trust": "contested", "counterexamples": 1, "deleted": False},
)

write(
    "counterexample", "a_counterexample_never_reports_a_verification_count",
    "No surface presents any of these numbers as independent verifications. The "
    "reported shape is fixed, and says applications, distinct projects, "
    "independently validated in, and counterexamples (FR-406).",
    {
        "applications": [
            {"project": p, "sessions": 4, "outcome": "resolved",
             "discovery": "independent", "evidence": True, "is_origin": False}
            for p in ("A", "B", "C")
        ]
        + [
            {"project": "D", "sessions": 1, "outcome": "not_applicable",
             "discovery": "independent", "evidence": True, "is_origin": False},
            {"project": "E", "sessions": 1, "outcome": "failed",
             "discovery": "independent", "evidence": True, "is_origin": False},
        ]
    },
    {
        "trust": "contested",
        "applications": 5,
        "distinct_projects_applied": 5,
        "qualifying_successes": 3,
        "counterexamples": 2,
        "rendered": "applications 5 · distinct projects 5 · independently validated in 3 · counterexamples 2",
        "renders_verification_count": False,
    },
)

print(
    "wrote "
    + ", ".join(f"{n} {name}" for name, n in _n.items())
    + " pattern cases"
)
