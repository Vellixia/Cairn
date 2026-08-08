-- Concurrency-safe outbox claiming (FR-056, D9).
--
-- The background worker and `cairn sync now` drain the same queue. Without a
-- claim they select the same `pending` rows and deliver them at the same time,
-- which turns a benign redelivery into a race the server has to absorb.
--
-- A drainer now moves rows to `in_flight` before sending. `claimed_at` records
-- when, which is what makes an abandoned claim recoverable rather than
-- permanent: a drainer that died mid-send leaves a row whose claim has simply
-- gone stale, and the next claim takes it back.
ALTER TABLE outbox ADD COLUMN claimed_at TEXT;

-- Rows left `in_flight` by a build without claim timestamps have no owner that
-- could still be alive: this migration runs at open, before anything drains.
UPDATE outbox SET state = 'pending' WHERE state = 'in_flight';

-- The claim reads by project and state, oldest first.
CREATE INDEX IF NOT EXISTS outbox_claimable
    ON outbox (project_id, state, created_at);
