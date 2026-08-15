1 project × 10 sessions; 3 projects × 1 session; and a `cairn_suggested` application carrying no
deterministic local evidence.

Rule: ten same-project applications yield a distinct-project validation count of **1**, not 10 —
applications are unique on `(pattern, project, signal_digest)`, so one incident counts once (FR-402,
SC-314). Trust advances only on distinct **non-origin** projects, and never on Cairn's own suggestion
without evidence collected in the applying project (FR-403).
