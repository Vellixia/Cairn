≥10 cases: the same scope kind with different scope keys — `branch:main` against
`branch:feature/x`, `task:T1` against `task:T2`.

Rule: the two are never simultaneously applicable, so they neither conflict nor interact at all. This
falls out of the scope key rather than out of a heuristic (D48).
