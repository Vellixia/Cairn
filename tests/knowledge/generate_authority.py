#!/usr/bin/env python3
"""Emit the verification-authority corpus (T044).

One case per authority value, and one per strict-consumer refusal. The point of
the directory is a single property: **an agent's attestation is never
indistinguishable from a check Cairn performed** (FR-370). Everything here
either demonstrates an authority or demonstrates a refusal that rests on it.

    python3 generate_authority.py <corpus-root>
"""
import json
import pathlib
import sys

ROOT = pathlib.Path(sys.argv[1]) / "verification" / "authority"
ROOT.mkdir(parents=True, exist_ok=True)


def write(index, slug, body):
    (ROOT / f"{index:03d}_{slug}.json").write_text(
        json.dumps(body, indent=2, ensure_ascii=False) + "\n"
    )


def case(index, slug, description, runs, expect):
    write(index, slug, {
        "description": description,
        "input": {"extra": {"runs": runs}},
        "expect": {"extra": expect},
    })


# --- the four authority values -------------------------------------------
case(1, "cairn_deterministic_check",
     "A deterministic check this machine ran over evidence Cairn collected. "
     "The only authority a task criterion or a cross-project promotion accepts.",
     [{"result": "verified", "collector": "cairn", "verifier": "configuration"}],
     {"authority": "cairn", "satisfies_deterministic_requirement": True,
      "criterion_may_verify": True, "promotion_eligible": True})

case(2, "attested_by_an_agent",
     "The memory is verified, and every run that established it consulted only "
     "agent-attested evidence. Useful, labelled everywhere, and visibly weaker.",
     [{"result": "verified", "collector": "agent", "verifier": "runtime_state"}],
     {"authority": "attested", "satisfies_deterministic_requirement": False,
      "criterion_may_verify": False, "promotion_eligible": False,
      "refusal": "attested_not_sufficient"})

case(3, "imported_from_a_peer_that_checked_it",
     "A peer established this deterministically. It is reported as verified "
     "elsewhere, never as verified here, and it does not count toward local "
     "readiness (FR-368).",
     [],
     {"imported_from": "cairn", "authority": "remote_cairn",
      "satisfies_deterministic_requirement": False,
      "criterion_may_verify": False, "promotion_eligible": False,
      "refusal": "imported_not_sufficient"})

case(4, "imported_from_a_peer_that_attested_it",
     "A peer established this by attestation. Both facts travel: that it was "
     "established elsewhere, and that an attestation is what established it.",
     [],
     {"imported_from": "attested", "authority": "remote_attested",
      "satisfies_deterministic_requirement": False,
      "criterion_may_verify": False, "promotion_eligible": False,
      "refusal": "imported_not_sufficient"})

# --- the strongest basis wins ---------------------------------------------
case(5, "both_bases_the_deterministic_one_establishes_it",
     "A memory supported by both an attested fact and a Cairn-read digest, "
     "verified by the digest, has authority `cairn`: a deterministic check did "
     "establish the claim, and saying otherwise would understate what Cairn "
     "knows. The attested fact stays attached and stays labelled (metric 25c).",
     [{"result": "verified", "collector": "agent", "verifier": "runtime_state"},
      {"result": "verified", "collector": "cairn", "verifier": "file_digest"}],
     {"authority": "cairn", "satisfies_deterministic_requirement": True})

case(6, "order_does_not_change_which_basis_wins",
     "The same two bases in the other order. Which run was recorded first "
     "cannot decide what established the claim.",
     [{"result": "verified", "collector": "cairn", "verifier": "file_digest"},
      {"result": "verified", "collector": "agent", "verifier": "runtime_state"}],
     {"authority": "cairn", "satisfies_deterministic_requirement": True})

# --- authority is meaningless unless verified ------------------------------
for i, state in enumerate(["unverified", "needs_recheck", "drifted", "conflicted"], start=7):
    case(i, f"no_authority_when_{state}",
         f"A memory that is {state} carries no authority at all. Authority "
         "says what established a verification, and nothing established one.",
         [{"result": "verified", "collector": "cairn", "verifier": "file_digest"}],
         {"state": state, "authority": None})

# --- failing and inconclusive runs establish nothing -----------------------
case(11, "a_drifted_run_establishes_no_authority",
     "A run that found the claim no longer matches its evidence establishes "
     "nothing to have an authority for.",
     [{"result": "drifted", "collector": "cairn", "verifier": "configuration"}],
     {"authority": None})

case(12, "an_inconclusive_run_establishes_no_authority",
     "The check ran and could establish neither outcome (FR-366).",
     [{"result": "inconclusive", "collector": "cairn", "verifier": "file_digest"}],
     {"authority": None})

case(13, "a_run_that_consulted_no_evidence_establishes_nothing",
     "A successful run with no evidence fact behind it is an inconsistent "
     "cache, and the honest answer is none rather than a guess (FR-478).",
     [{"result": "verified", "collector": None, "verifier": "file_digest"}],
     {"authority": None})

case(14, "verified_with_no_run_at_all_fails_closed",
     "A memory reported verified with no successful run has a cache that "
     "disagrees with its records. The durable records win (FR-518).",
     [],
     {"authority": None})

# --- what a peer may be told ----------------------------------------------
case(15, "the_wire_carries_only_the_two_local_values",
     "A peer learns what kind of check stands behind the state, not that this "
     "machine imported it. `remote_cairn` is sent as `cairn`, `remote_attested` "
     "as `attested` — and the receiver maps them back to `remote_*` on import, "
     "so neither side ever sees the other's provenance as its own.",
     [],
     {"on_the_wire": {"cairn": "cairn", "attested": "attested",
                      "remote_cairn": "cairn", "remote_attested": "attested"},
      "on_import": {"cairn": "remote_cairn", "attested": "remote_attested",
                    "remote_cairn": "remote_cairn",
                    "remote_attested": "remote_attested"}})

print(f"{len(list(ROOT.glob('*.json')))} cases written to {ROOT}")
