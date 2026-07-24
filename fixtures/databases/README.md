# Cairn database fixtures

`feature-001-v1.sqlite3` is a populated migration fixture produced by the clean,
detached Feature 001 checkout at commit
`4a06c4125715bb4b78b54e49c81eccd82100a7b7` using that checkout's `cairn` and
`cairnd` binaries.

The producer created four independent Git repositories and exercised normal daemon
IPC to leave one session in each Feature 001 lifecycle state: `active`, `recovering`,
`stopped`, and `interrupted`. The fixture contains repositories, worktrees, snapshots,
leases, resume-token hashes, and 18 ordered append-only events. No raw resume token is
stored in the fixture or manifest.

The accompanying `feature-001-v1.manifest.json` records the exact producer SHA,
migration checksum, database hash, schema object names, table counts, session fields,
and canonical ordered event-row hashes. Fixture verification must fail if Feature 002
tables or columns are present, if the maximum SQLx migration is not 1, or if any
recorded hash or count changes.

Regeneration is evidence-sensitive: build and run only the exact producer commit from
a clean detached checkout, verify its status both before and after generation, and
replace the database and manifest together. Never regenerate this fixture using the
Feature 002 migration runner.
