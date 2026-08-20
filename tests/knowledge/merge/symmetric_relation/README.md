The same conflict detected independently on both stores while offline.

Rule: after both directions sync, exactly **one** durable `conflicts_with` relation exists, not two
facing opposite ways — the primary key absorbs the second machine because symmetric endpoints are
normalized before the write (FR-305, D78, SC-324 metric 34d).
