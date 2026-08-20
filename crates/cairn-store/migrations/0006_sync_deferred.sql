-- Durable retry for a pulled record whose parent has not arrived (#44).
--
-- The pull cursor is a timestamp over `updated_at`, and the server only offers
-- a record again when the record itself changes. So a relation or criterion
-- declined because the memory or task it names had not arrived yet was not
-- "carried by the next pull" as the code claimed: the cursor advanced past it
-- and it was never offered again. One page of 500 memories is enough to reach
-- it — the cursor pins to that page's newest row, and a relation older than
-- that row whose memory falls in the *next* page is lost permanently.
--
-- Holding the payload locally keeps the wire and the cursor exactly as they
-- were: nothing here is transmitted, and a held record is replayed after every
-- later pull until the parent lands.
CREATE TABLE IF NOT EXISTS sync_deferred (
    project_id      TEXT    NOT NULL REFERENCES projects(id),
    -- Which importer to hand the payload back to.
    kind            TEXT    NOT NULL CHECK (kind IN ('relation', 'criterion', 'blocker')),
    -- The record's own identity, so a re-sent record replaces its held copy
    -- rather than accumulating one row per pull. Criteria and blockers use
    -- their id; a relation has none on the wire and uses from:to:kind, which is
    -- the key `record_relation` already treats as the relation's identity.
    record_key      TEXT    NOT NULL,
    payload         TEXT    NOT NULL,
    -- The parent that was missing, so a project waiting on one record can say
    -- which record it is waiting for.
    waiting_on      TEXT    NOT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    first_seen_at   TEXT    NOT NULL,
    last_attempt_at TEXT    NOT NULL,
    PRIMARY KEY (project_id, kind, record_key)
);

CREATE INDEX IF NOT EXISTS sync_deferred_project ON sync_deferred (project_id);
