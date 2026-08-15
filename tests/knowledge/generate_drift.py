import json, pathlib, sys
ROOT = pathlib.Path(sys.argv[1]) / "drift"
ROOT.mkdir(parents=True, exist_ok=True)

def write(i, slug, description, frm, trigger, to):
    (ROOT / f"{i:03d}_{slug}.json").write_text(json.dumps({
        "description": description,
        "input": {"extra": {"from": frm, "trigger": trigger}},
        "expect": {"extra": {"to": to}},
    }, indent=2) + "\n")

cases = [
    ("fingerprint_change_from_verified",
     "The support moved under an established claim. It owes a recheck, and it "
     "is not yet drifted: nothing has looked at the new value.",
     "verified", "fingerprint_changed", "needs_recheck"),
    ("fingerprint_change_from_drifted",
     "The support moved again. A drifted claim goes back to owing a recheck "
     "rather than staying drifted against a value nobody compared.",
     "drifted", "fingerprint_changed", "needs_recheck"),
    ("fingerprint_change_from_conflicted",
     "The memory's own evidence disagreed with itself, and then the support "
     "moved. The disagreement is no longer about the same thing.",
     "conflicted", "fingerprint_changed", "needs_recheck"),
    ("recheck_finds_the_same_value",
     "The file changed and changed back, or the change was elsewhere in it. "
     "The claim is verified again.",
     "needs_recheck", "run_verified", "verified"),
    ("recheck_finds_a_different_value",
     "The remembered value is no longer what the evidence says. The memory is "
     "drifted - and it is not rewritten, superseded or hidden (FR-372).",
     "needs_recheck", "run_drifted", "drifted"),
    ("recheck_cannot_read_the_target",
     "The file is gone or the ref is unresolvable. The claim stays owing a "
     "recheck: neither verified nor drifted is honest (FR-366).",
     "needs_recheck", "run_inconclusive", "needs_recheck"),
    ("the_last_supporting_fact_is_deleted",
     "Deleting a fact tombstones it and clears its fingerprint. The supported "
     "memory becomes needs_recheck, never stays verified.",
     "verified", "last_supporting_evidence_deleted", "needs_recheck"),
    ("deleted_evidence_from_needs_recheck",
     "Already owing a recheck, and now with nothing left to check against. It "
     "stays where it is rather than reporting an outcome it does not have.",
     "needs_recheck", "last_supporting_evidence_deleted", "needs_recheck"),
    ("an_unverified_claim_is_not_disturbed",
     "There is nothing to recheck. Moving it would claim a verification it "
     "never had.",
     "unverified", "fingerprint_changed", None),
    ("drift_is_never_cleared_by_a_fingerprint_change",
     "Only a run clears drift. A file moving again does not make a stale claim "
     "true.",
     "drifted", "fingerprint_changed", "needs_recheck"),
    ("supersession_moves_no_verification_state",
     "A superseded memory keeps its last verification, which is what lets a "
     "historical query say what was verified then (D50).",
     "verified", "superseded", None),
    ("staleness_moves_no_verification_state",
     "Scope staleness and evidence are orthogonal.",
     "verified", "marked_stale", None),
    ("an_import_never_moves_the_local_state",
     "A peer's verification is reported as established elsewhere; it does not "
     "overwrite what this machine knows (FR-368).",
     "verified", "imported", None),
    ("a_contradiction_makes_a_claim_conflicted",
     "This memory's own evidence disagrees with itself: supporting and "
     "contradicting facts both attached (FR-369).",
     "verified", "contradicting_evidence_attached", "conflicted"),
    ("removing_the_contradiction_owes_a_recheck",
     "The disagreement is gone, and what remains has not been checked since.",
     "conflicted", "contradicting_evidence_removed", "needs_recheck"),
]
for i, (slug, description, frm, trigger, to) in enumerate(cases, start=1):
    write(i, slug, description, frm, trigger, to)
print(f"{len(cases)} drift cases written")
