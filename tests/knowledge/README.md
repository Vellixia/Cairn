# The Feature 003 deterministic corpus

JSON fixtures, versioned in the repository, loaded by pure functions with **no database and no
daemon** (`contracts/evaluation.md` §The corpus). This is tier 2 of the five test tiers, and it is
where most of Feature 003's correctness lives, because the reconciliation derivation, the
verification state machine and the staleness comparison are pure functions.

## The paired-corpus rule

> Every case in `reconciliation/equivalent/` has a sibling in `reconciliation/distinct/` differing
> in **exactly the way that matters**.

"Zero false merges" is only measurable against cases that *look* mergeable and must not be. A corpus
of cases that obviously differ proves nothing: it measures whether the code can tell apart things
nobody would confuse. The pairing is what makes SC-301 a measurement rather than an aspiration.

The same discipline applies elsewhere in the tree:

| Positive | Its paired negative | What the pair measures |
|---|---|---|
| `reconciliation/equivalent/` | `reconciliation/distinct/` | zero false merges (SC-301) |
| `reconciliation/duplicate_content/` | `reconciliation/coarse_value_key/` | a shared value key is not a shared claim (FR-327) |
| `conflict/real/` | `conflict/scope_exception/`, `conflict/disjoint/` | zero false conflicts (SC-302) |
| `patterns/promote/` | `patterns/refuse/`, `privacy/` | the gate fails closed (SC-315) |
| `verification/` documented transitions | the same directory's undocumented transitions | the state machine is total (SC-306) |

## Naming

One case per file, `NNN_short_slug.json`. The number orders the case within its directory and is not
otherwise meaningful. A fixture failure names the file, so a red run points at a case rather than at
a line number.

## What a case may not contain

No absolute path, no real credential, no personal identifier — except inside `privacy/`, where the
seeded values are deliberately synthetic and exist to be refused. Nothing here is read by product
code at run time.
