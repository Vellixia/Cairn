History preservation and the temporal predicate.

Rule: a superseded proposal is byte-identical before and after — content, provenance and evidence
references — and an `as_of` query at an instant before the supersession returns the historical answer
(SC-305). Chains ≥3 deep are included, because a two-member chain hides ordering defects.

At least one case has `stale_at IS NULL`, asserting the historical answer reports
`applicability: unknown` rather than a bounded interval. Unknown is the honest answer, and NULL never
means "not stale" (FR-341, FR-342, D82).
