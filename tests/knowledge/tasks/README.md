Concurrent criterion updates, revision divergence and derived readiness.

Rule: two sessions updating **different** criteria both persist and neither resets the other
(SC-317); a caller supplying an `expected_revision` that has advanced is refused with
`revision_conflict` (FR-490); a criterion reaches `verified` only on a local `cairn`-authority
verification (FR-484); readiness is derived and never changes `tasks.status` (FR-487); and no field
exists anywhere in which an agent could store a completion percentage (FR-486).
