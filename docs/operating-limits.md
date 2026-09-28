# Operating limits

These are current source defaults and safety bounds for `v0.1.0-alpha.9`.
They are not measured throughput, durability, backup, or capacity guarantees.

| Boundary | Current limit or default | Behavior |
| --- | --- | --- |
| Context | 3,000 tokens; minimum 600 | Configurable local budget. |
| Capture/context deadline | 250ms / 1,500ms | Configurable local deadlines. A missed deadline returns an honest fallback. |
| Outage cache | 200 sessions, 64KiB each, 300s TTL | Account-bound in-memory LRU; restart loses it. Auth denial invalidates applicable cache. |
| Event spool | 50,000 rows or 256MiB payloads | Whichever binds first. Oldest capture rows are dropped and counted; boundary rows are protected. When only protected rows remain, new work is refused and counted rather than silently dropped. This is not a total disk limit. |
| Delivery claim / HTTP request | 60s / 20s | Expired claims are reclaimable; retry still relies on server idempotency. |
| Retrieval traces | 90 days; sweeps of 500 | Trace expiry does not delete knowledge, evidence, or receipts. |
| Logical export | 32MiB serialized JSON | Larger exports are refused before a response; this is not a database-size limit. Import requires the same bounded request body. |
| Local snapshot copy/hash | 64KiB I/O chunks | Source SQLite bytes are copied and hashed without loading the whole file; sufficient free disk is still required for the private copy and backup. |

No automatic retention/deletion policy exists for accepted knowledge, evidence, backups,
or PostgreSQL volume growth in alpha.9. Operators must size, back up, and test restore
for their own retention requirement. Logical import/export is transfer on a healthy
deployment; it is not a physical-disaster-recovery substitute.

Release evidence must exercise zero/near/full/overflow spool states, cache expiry and
identity isolation, claim recovery, trace expiry, and representative transfer sizes
before these values can support a candidate claim. No arbitrary-size or peak-memory
guarantee follows from the I/O chunk size. Measured workload targets belong to the
later capacity release.
