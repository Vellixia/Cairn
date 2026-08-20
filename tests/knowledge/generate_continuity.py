"""Generate the `continuity/` corpus.

Ten consecutive compaction cycles, one case per cycle, each stating which
FR-422 fields must be delivered after it.

The rule is per **cycle**, not per trace: a checkpoint that survives one
compaction and quietly loses its relevant paths on the sixth is the failure
this corpus exists to catch, and a single end-to-end assertion after cycle ten
would not see it. Each case therefore names the whole field set, so any cycle
that drops one is attributable to that cycle.

What is deliberately absent from every case: any field that could carry a
provider's summary of the conversation. The checkpoint is derived from recorded
project state, and a paraphrase is the one thing it must never contain
(SC-310).
"""
import json, pathlib, sys

ROOT = pathlib.Path(sys.argv[1]) / "continuity"
ROOT.mkdir(parents=True, exist_ok=True)

# The FR-422 field set a restored checkpoint must carry.
REQUIRED = [
    "task_id",
    "task_state_digest",
    "branch",
    "commit",
    "relevant_paths",
    "next_action",
    "restore_count",
]

# Fields whose presence would mean a conversation had been summarized.
FORBIDDEN = [
    "summary",
    "transcript",
    "assistant_message",
    "user_prompt",
    "conversation",
]

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


for cycle in range(1, 11):
    write(
        f"cycle_{cycle:02d}_delivers_every_recorded_field",
        f"Compaction cycle {cycle} of ten. Every FR-422 field that exists in "
        f"recorded state is delivered again, and the restore count has reached "
        f"{cycle}. Nothing accumulates and nothing is dropped: the checkpoint is "
        "rebuilt from the same recorded state each time, so cycle ten costs what "
        "cycle one cost and says the same things.",
        {
            "cycle": cycle,
            "recorded": {
                "task_bound": True,
                "commit": "abc123",
                "branch": "feature/retry",
                "relevant_paths": ["src/retry.rs"],
                "next_action": "finish the retry backoff in src/retry.rs",
            },
        },
        {
            "required_fields": REQUIRED,
            "forbidden_fields": FORBIDDEN,
            "restore_count": cycle,
            "checkpoint_state": "current",
            "next_action_is_live": True,
        },
    )

# The two cases that are about what a cycle must *not* do.

write(
    "a_cycle_with_no_task_bound_omits_the_task_fields",
    "A session with no task bound has no task state to record, so the task "
    "fields are absent rather than empty. An empty string for a task digest "
    "would compare unequal to itself on the next cycle and report a divergence "
    "that never happened.",
    {
        "cycle": 1,
        "recorded": {
            "task_bound": False,
            "commit": "abc123",
            "branch": "main",
            "relevant_paths": ["src/main.rs"],
            "next_action": "read the failing test",
        },
    },
    {
        "required_fields": ["branch", "commit", "relevant_paths", "next_action"],
        "absent_fields": ["task_id", "task_state_digest"],
        "forbidden_fields": FORBIDDEN,
        "checkpoint_state": "current",
    },
)

write(
    "a_cycle_after_the_world_moved_reports_divergence_and_no_live_action",
    "The tenth cycle, taken after the commit moved and a relevant path changed. "
    "Every field is still delivered, the checkpoint is `diverged`, and the "
    "recorded next action is reported as a **previous** one — presenting it as "
    "the action to take is how an agent resumes work that no longer applies "
    "(FR-434).",
    {
        "cycle": 10,
        "recorded": {
            "task_bound": True,
            "commit": "abc123",
            "branch": "feature/retry",
            "relevant_paths": ["src/retry.rs"],
            "next_action": "finish the retry backoff in src/retry.rs",
        },
        "moved": {"commit": "def456", "relevant_paths": ["src/retry.rs"]},
    },
    {
        "required_fields": REQUIRED,
        "forbidden_fields": FORBIDDEN,
        "checkpoint_state": "diverged",
        "divergence_kinds": ["commit", "files"],
        "next_action_is_live": False,
        "previous_next_action_present": True,
    },
)

print(f"wrote {_n} continuity cases")
