≥15 cases: same scope, same scope key, differing value key.

Rule: `reconciliation: conflicted`, every member stays `active`, no member is marked superseded, no
single canonical answer is emitted, and a `conflicts_with` relation is recorded with its endpoints
normalized `(min, max)` (FR-334, D78). Zero silent winners (SC-302).
