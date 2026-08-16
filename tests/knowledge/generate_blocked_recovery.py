"""Generate the `merge/blocked_recovery/` corpus (T106, SC-331).

Work an older server refuses for lack of capability is **retained**: not
retried, not marked failed, not reported delivered, and delivered exactly once
after the server is upgraded — with no manual repair of stored data.

These cases carry no clock-reversed twin, and deliberately so. The pairing rule
in `merge/README.md` exists because a merge must not consult a timestamp to
choose a winner; nothing here chooses between two writes. What is under test is
a **transition** — refused, retained, released, delivered — and reversing two
machines' clocks says nothing about it.

Each case names the rejection the server gives, the state the row must reach,
and the two things that must **not** happen. The negatives are the point: a
`blocked` state that quietly retried, or that was reported as `failed`, would
satisfy a test that only asserted the eventual delivery.
"""
import json, pathlib, sys

ROOT = pathlib.Path(sys.argv[1]) / "merge" / "blocked_recovery"
ROOT.mkdir(parents=True, exist_ok=True)

_n = 0


def write(slug, description, given, expect):
    global _n
    _n += 1
    (ROOT / f"{_n:03d}_{slug}.json").write_text(
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


# ---------------------------------------------------------------------------
# A capability the server does not have
# ---------------------------------------------------------------------------

write(
    "a_relation_a_schema_1_server_cannot_hold",
    "A `memory_relation` is queued against a server whose schema stops at 1. The "
    "server has no table to put it in and says so by entity type. The row becomes "
    "`blocked` with the refusal class recorded, and is retried zero times: there "
    "is nothing to retry against a server that is missing the capability, and "
    "retrying would spend the drain cycle on work that cannot succeed (FR-418).",
    {
        "server": {"schema_version": 1, "capabilities": []},
        "queued": [{"entity_type": "memory_relation", "operation": "upsert"}],
        "rejection": {"code": "unknown_entity_type", "class": "capability"},
    },
    {
        "state": "blocked",
        "blocked_reason": "unknown_entity_type",
        "blocked_at_capability": "schema_version=1",
        "retries": 0,
        "never_failed": True,
        "never_reported_delivered": True,
    },
)

write(
    "a_field_the_server_does_not_know",
    "The entity type is one the server has, and a **field** is not. That is the "
    "same class of answer - the server cannot hold this work - and it is retained "
    "the same way. Treating it as a content rejection would strand a memory's "
    "subject identity permanently on the machine that wrote it.",
    {
        "server": {"schema_version": 1, "capabilities": []},
        "queued": [{"entity_type": "memory", "operation": "upsert"}],
        "rejection": {"code": "unknown_field", "class": "capability"},
    },
    {
        "state": "blocked",
        "blocked_reason": "unknown_field",
        "retries": 0,
        "never_failed": True,
        "never_reported_delivered": True,
    },
)

write(
    "a_server_that_says_it_is_older",
    "The server answers `schema_older` outright rather than naming a type or a "
    "field. The class is the same and so is the outcome, which is what makes the "
    "three codes one behaviour rather than three.",
    {
        "server": {"schema_version": 1, "capabilities": []},
        "queued": [{"entity_type": "task_criterion", "operation": "upsert"}],
        "rejection": {"code": "schema_older", "class": "capability"},
    },
    {
        "state": "blocked",
        "blocked_reason": "schema_older",
        "retries": 0,
        "never_failed": True,
        "never_reported_delivered": True,
    },
)

# ---------------------------------------------------------------------------
# What must keep working while some work is blocked
# ---------------------------------------------------------------------------

write(
    "feature_001_keeps_syncing_throughout",
    "A blocked relation must not block the queue. Everything a schema-1 server "
    "can hold - memories, sessions, handoffs, tasks - is delivered in full while "
    "the refused work waits. Degradation that stopped ordinary sync would be an "
    "outage dressed as a compatibility feature (SC-326).",
    {
        "server": {"schema_version": 1, "capabilities": []},
        "queued": [
            {"entity_type": "memory_relation", "operation": "upsert"},
            {"entity_type": "memory", "operation": "upsert"},
            {"entity_type": "session", "operation": "upsert"},
            {"entity_type": "handoff", "operation": "upsert"},
        ],
        "rejection": {"code": "unknown_entity_type", "class": "capability"},
    },
    {
        "blocked": 1,
        "delivered": 3,
        "feature_001_delivery_share": 1.0,
        "never_failed": True,
    },
)

# ---------------------------------------------------------------------------
# The distinction the whole state rests on
# ---------------------------------------------------------------------------

write(
    "a_content_rejection_still_fails_permanently",
    "The negative that gives `blocked` its meaning. A payload the server refuses "
    "on its **content** - an observation identifier where none may go - is a "
    "permanent failure exactly as it is today, and no upgrade will ever make it "
    "acceptable. Retaining it would turn a privacy refusal into a pending "
    "delivery (FR-418, D81).",
    {
        "server": {"schema_version": 2, "capabilities": ["memory_relations"]},
        "queued": [{"entity_type": "memory", "operation": "upsert"}],
        "rejection": {"code": "forbidden_field", "class": "content"},
    },
    {
        "state": "failed",
        "never_blocked": True,
        "released_by_upgrade": False,
    },
)

# ---------------------------------------------------------------------------
# The upgrade
# ---------------------------------------------------------------------------

write(
    "the_upgrade_releases_the_retained_work",
    "The server gains the capability. The blocked rows return to `pending` with "
    "their **original** idempotency key and payload, and the ordinary drain "
    "delivers them - exactly once, under the server's unchanged claim mechanism. "
    "A new key would make the retained work a second delivery rather than the "
    "one that was waiting (FR-418, SC-331).",
    {
        "server_before": {"schema_version": 1, "capabilities": []},
        "server_after": {
            "schema_version": 2,
            "capabilities": ["memory_relations", "task_criteria", "task_blockers"],
        },
        "queued": [
            {"entity_type": "memory_relation", "operation": "upsert"},
            {"entity_type": "task_criterion", "operation": "upsert"},
        ],
        "rejection": {"code": "unknown_entity_type", "class": "capability"},
    },
    {
        "blocked_before_upgrade": 2,
        "released_on_upgrade": 2,
        "delivered_after_upgrade": 2,
        "delivered_exactly_once": True,
        "idempotency_key_preserved": True,
        "payload_preserved": True,
        "manual_repair_required": False,
        "peers_converged": True,
    },
)

write(
    "an_upgrade_that_adds_only_part_of_what_was_refused",
    "The server gains one capability and not the other. Only the rows whose "
    "class the new capability supports are released; the rest stay blocked and "
    "stay recoverable. Releasing everything on any change would put work back in "
    "front of a server that still cannot hold it, and the futile retry the state "
    "exists to prevent would happen anyway.",
    {
        "server_before": {"schema_version": 1, "capabilities": []},
        "server_after": {"schema_version": 2, "capabilities": ["memory_relations"]},
        "queued": [
            {"entity_type": "memory_relation", "operation": "upsert"},
            {"entity_type": "task_blocker", "operation": "upsert"},
        ],
        "rejection": {"code": "unknown_entity_type", "class": "capability"},
    },
    {
        "released_on_upgrade": 1,
        "still_blocked": 1,
        "retries_of_still_blocked": 0,
        "never_failed": True,
    },
)

print(f"wrote {_n} blocked-recovery cases")
