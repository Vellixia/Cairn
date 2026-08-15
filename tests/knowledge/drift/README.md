The full drift transition set.

Rule: a changed evidence fingerprint moves the claim it supports to `needs_recheck` and **nothing
else** changes — not content, not type, not scope, not provenance, not lifecycle state, and no memory
is created (FR-371, FR-372). Re-verification then yields `verified` (same value), `drifted`
(different value) or `inconclusive` (unreadable target), and `inconclusive` leaves the memory in
`needs_recheck` (FR-366, SC-307).
