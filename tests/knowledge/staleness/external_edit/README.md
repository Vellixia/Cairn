Relevant paths changed with **no Cairn session involved**: a human editor, a formatter, `git apply`,
an IDE refactor — with the commit unmoved.

Rule: the change is still detected, because the checkpoint compares the bounded per-path fingerprint
it recorded rather than looking for another session's observation (FR-432, D79, SC-311 metric 15a).

Also seeded: paths that are privacy-excluded, unreadable, or larger than the payload cap. Those are
reported `not_fingerprintable` — never `unchanged`, because "I could not look" and "nothing moved"
are different answers (metric 15b).
