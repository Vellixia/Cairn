"""Generate the `staleness/` corpus.

Two groups:

  * `staleness/` — every divergence class (branch, commit, task, files) alone
    and in combination, each naming the classification it must produce and
    whether the recorded next action may be presented as the action to take. It
    may not, ever, unless the checkpoint is `current` (FR-434, SC-311).
  * `staleness/external_edit/` — relevant paths changed with **no Cairn session
    involved**, and paths that cannot be fingerprinted at all. The first group
    is what makes detection observation-independent (D79); the second is what
    keeps "I could not look" from reading as "nothing moved" (FR-432).

A case names the assumption set and the current state as the pure classifier
consumes them, so the corpus asserts the contract rather than an implementation.
"""
import json, pathlib, sys

ROOT = pathlib.Path(sys.argv[1]) / "staleness"
EXTERNAL = ROOT / "external_edit"
ROOT.mkdir(parents=True, exist_ok=True)
EXTERNAL.mkdir(parents=True, exist_ok=True)

BRANCH = "main"
COMMIT = "abc123def456abc123def456abc123def456abcd"
MOVED = "def456abc123def456abc123def456abc123defa"
TASK = "01a00000-0000-7000-8000-000000000001"
DIGEST_A = "1111111111111111111111111111111111111111111111111111111111111111"
DIGEST_B = "2222222222222222222222222222222222222222222222222222222222222222"


def fingerprint(path, cls="digest", value=DIGEST_A, exists=True):
    fp = {"path": path, "class": cls, "exists": exists}
    if value is not None:
        fp["value"] = value
    return fp


def case(directory, i, slug, description, assumed, current, expect):
    (directory / f"{i:03d}_{slug}.json").write_text(
        json.dumps(
            {
                "description": description,
                "input": {"extra": {"assumed": assumed, "current": current}},
                "expect": {"extra": expect},
            },
            indent=2,
        )
        + "\n"
    )


def assumed(branch=BRANCH, commit=COMMIT, task=TASK, digest="d0", paths=None):
    return {
        "branch": branch,
        "commit": commit,
        "task_id": task,
        "task_state_digest": digest,
        "path_fingerprints": paths if paths is not None else [fingerprint("src/retry.rs")],
    }


def current(
    branch=BRANCH,
    commit=COMMIT,
    digest="d0",
    paths=None,
    task_exists=True,
    worktree_exists=True,
):
    return {
        "branch": branch,
        "commit": commit,
        "task_exists": task_exists,
        "worktree_exists": worktree_exists,
        "task_state_digest": digest,
        "path_fingerprints": paths if paths is not None else [fingerprint("src/retry.rs")],
    }


# ---------------------------------------------------------------------------
# Every divergence class, alone.
# ---------------------------------------------------------------------------

n = 0

n += 1
case(
    ROOT, n, "nothing_moved",
    "Branch, commit, task state and every relevant path are as recorded. This is "
    "the only case in which the recorded next action may be presented as the "
    "action to take.",
    assumed(), current(),
    {"state": "current", "divergences": [], "next_action_is_live": True},
)

n += 1
case(
    ROOT, n, "branch_alone",
    "The session resumed on a different branch. What to do next on `main` is not "
    "what to do next on a feature branch.",
    assumed(), current(branch="feature/retry"),
    {"state": "diverged", "divergences": ["branch"], "next_action_is_live": False},
)

n += 1
case(
    ROOT, n, "commit_alone",
    "The head moved. Someone committed - possibly another session, possibly the "
    "developer - and the recorded action may already be done.",
    assumed(), current(commit=MOVED),
    {"state": "diverged", "divergences": ["commit"], "next_action_is_live": False},
)

n += 1
case(
    ROOT, n, "task_alone",
    "The task state digest differs, so the task advanced. Decided by the derived "
    "digest and never by a counter, so it means the same thing whichever machine "
    "moved it (D80).",
    assumed(), current(digest="d1"),
    {"state": "diverged", "divergences": ["task"], "next_action_is_live": False},
)

n += 1
case(
    ROOT, n, "files_alone",
    "A relevant path's content differs from the fingerprint recorded for it, with "
    "the commit unmoved.",
    assumed(), current(paths=[fingerprint("src/retry.rs", value=DIGEST_B)]),
    {"state": "diverged", "divergences": ["files"], "next_action_is_live": False},
)

# ---------------------------------------------------------------------------
# In combination.
# ---------------------------------------------------------------------------

n += 1
case(
    ROOT, n, "commit_and_files",
    "The head moved and a relevant path changed with it. Both are reported: "
    "collapsing them would hide which one the agent needs to look at.",
    assumed(),
    current(commit=MOVED, paths=[fingerprint("src/retry.rs", value=DIGEST_B)]),
    {"state": "diverged", "divergences": ["commit", "files"], "next_action_is_live": False},
)

n += 1
case(
    ROOT, n, "branch_and_task",
    "A different branch and an advanced task.",
    assumed(), current(branch="feature/retry", digest="d1"),
    {"state": "diverged", "divergences": ["branch", "task"], "next_action_is_live": False},
)

n += 1
case(
    ROOT, n, "every_class_at_once",
    "Everything moved. All four classes are reported, and the recorded action is "
    "still delivered - labelled - because throwing it away loses information "
    "while presenting it as the instruction is the failure mode (US6 #2).",
    assumed(),
    current(
        branch="feature/retry",
        commit=MOVED,
        digest="d1",
        paths=[fingerprint("src/retry.rs", value=DIGEST_B)],
    ),
    {
        "state": "diverged",
        "divergences": ["branch", "commit", "task", "files"],
        "next_action_is_live": False,
    },
)

# ---------------------------------------------------------------------------
# Unresolvable — partial continuity is a result, not a failure.
# ---------------------------------------------------------------------------

n += 1
case(
    ROOT, n, "the_task_no_longer_exists",
    "The assumed task is gone. Every continuity field that does not depend on it "
    "is still delivered; the checkpoint is unresolvable, not an error (FR-435).",
    assumed(), current(task_exists=False),
    {"state": "unresolvable", "next_action_is_live": False},
)

n += 1
case(
    ROOT, n, "the_worktree_no_longer_exists",
    "The worktree the checkpoint was taken in is gone.",
    assumed(), current(worktree_exists=False),
    {"state": "unresolvable", "next_action_is_live": False},
)

# ---------------------------------------------------------------------------
# external_edit — the change nobody told Cairn about.
# ---------------------------------------------------------------------------

m = 0
for slug, who in [
    ("a_human_editor", "a developer editing in an editor"),
    ("a_formatter", "a formatter rewriting the file on save"),
    ("git_apply", "`git apply` landing a patch"),
    ("an_ide_refactor", "an IDE refactor touching the file"),
]:
    m += 1
    case(
        EXTERNAL, m, slug,
        f"A relevant path changed by {who}, with the commit unmoved and **no "
        f"Cairn session involved**. The change is still detected, because the "
        f"checkpoint compares the fingerprint it recorded rather than looking for "
        f"another session's observation. The earlier design missed every one of "
        f"these (D79).",
        assumed(),
        current(paths=[fingerprint("src/retry.rs", value=DIGEST_B)]),
        {
            "state": "diverged",
            "divergences": ["files"],
            "path_outcomes": {"src/retry.rs": "changed"},
            "next_action_is_live": False,
        },
    )

m += 1
case(
    EXTERNAL, m, "a_privacy_excluded_path",
    "The path matches a privacy exclusion, so Cairn never read it. The file is "
    "there - `exists` is true - but nothing comparable was recorded, so it is "
    "reported `not_fingerprintable` and never `unchanged`. 'I was told not to "
    "look' is not 'nothing moved'.",
    assumed(paths=[fingerprint("secrets/prod.env", cls="unknown", value=None)]),
    current(paths=[fingerprint("secrets/prod.env", cls="unknown", value=None)]),
    {"path_outcomes": {"secrets/prod.env": "not_fingerprintable"}},
)

m += 1
case(
    EXTERNAL, m, "an_unreadable_path",
    "The file is present and could not be read at restoration. `exists` stays "
    "true and the class is `unknown`, which is what keeps this distinct from a "
    "file that was deleted.",
    assumed(),
    current(paths=[fingerprint("src/retry.rs", cls="unknown", value=None)]),
    {"path_outcomes": {"src/retry.rs": "not_fingerprintable"}},
)

m += 1
case(
    EXTERNAL, m, "over_the_payload_cap",
    "The file exceeds the payload cap, so it was fingerprinted by size on both "
    "sides. A size match is weaker than a digest match - a same-length edit reads "
    "as unchanged - and the class is recorded so the weaker comparison is visible "
    "rather than implied.",
    assumed(paths=[fingerprint("vendor/large.bin", cls="size", value="10485760")]),
    current(paths=[fingerprint("vendor/large.bin", cls="size", value="10485760")]),
    {"path_outcomes": {"vendor/large.bin": "unchanged"}},
)

m += 1
case(
    EXTERNAL, m, "over_the_cap_and_a_different_size",
    "The same oversized file at a different length. Detected, by the weaker "
    "comparison, and still reported as a change.",
    assumed(paths=[fingerprint("vendor/large.bin", cls="size", value="10485760")]),
    current(paths=[fingerprint("vendor/large.bin", cls="size", value="10485761")]),
    {
        "state": "diverged",
        "divergences": ["files"],
        "path_outcomes": {"vendor/large.bin": "changed"},
    },
)

m += 1
case(
    EXTERNAL, m, "a_path_that_appeared",
    "A relevant path recorded as absent now exists.",
    assumed(paths=[fingerprint("src/new.rs", cls="unknown", value=None, exists=False)]),
    current(paths=[fingerprint("src/new.rs")]),
    {
        "state": "diverged",
        "divergences": ["files"],
        "path_outcomes": {"src/new.rs": "added"},
    },
)

m += 1
case(
    EXTERNAL, m, "a_path_that_was_removed",
    "A relevant path that existed at checkpoint time is gone.",
    assumed(),
    current(paths=[fingerprint("src/retry.rs", cls="unknown", value=None, exists=False)]),
    {"path_outcomes": {"src/retry.rs": "removed"}},
)

print(f"wrote {n} divergence cases and {m} external-edit cases")
