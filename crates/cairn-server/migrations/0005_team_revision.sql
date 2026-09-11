-- Server schema v5 — `team_knowledge` rows carry a **monotonic revision**, and
-- the pull feed pages on it.
--
-- **`changed_at` was never a row version.** Both team readers computed
-- `GREATEST(created_at, ratified_at, retired_at, superseded_at) AS changed_at`
-- and paged on `(changed_at, id)`, and the mirror stored that value as
-- `team_knowledge.server_changed_at` (local schema 11) to order pulled pages
-- against its own writes. Every one of those columns is stamped with
-- PostgreSQL `now()`, which is **transaction start time**, not statement time —
-- so two lifecycle writes whose transactions open in one order and commit in
-- the other record timestamps in the wrong order. Measured against this
-- repository's own PostgreSQL, with no lock contention needed:
--
--     S2 opens its transaction   14:41:43.252792  -> writes retired_at  = 43.252792
--     S1 ratifies (own tx)       14:41:44.048009  -> writes ratified_at = 44.048009
--     final row: state = 'retired', retired_at < ratified_at = true,
--                GREATEST(...) = 44.048009 — the ratification's value
--
-- The row went proposed -> authoritative -> retired and `changed_at` after the
-- retirement is byte-for-byte `changed_at` after the ratification. Two
-- consequences, both live:
--
-- 1. **Equality cannot order two states.** A page fetched after the
--    ratification compared equal to the retired row's recorded version and was
--    admitted by `merge_synced_team`, un-retiring it on that device.
-- 2. **The feed skips the retirement outright.** A cursor already at that
--    value never sees a row whose key did not advance, so *no other device ever
--    learns of the retirement*. Silent divergence, unbounded in time, because
--    nothing later contradicts it — the feed is keyed by the value that failed
--    to move.
--
-- So the ordering key stops being a clock reading and becomes a sequence.
--
-- **Why a sequence and not a clock.** `nextval` is evaluated at *statement*
-- time and is monotonic across concurrent transactions by construction — it is
-- non-transactional, which is exactly the property wanted here: two writes
-- get two revisions in the order the writes actually happened, whatever their
-- transactions' start times were. In the trace above the revisions are
-- 1 (insert), 2 (ratify), 3 (retire): correct where the timestamps are not.
-- Gaps are expected and carry no meaning — a rolled-back write, or the
-- `ON CONFLICT DO NOTHING` in the proposal ingest, consumes a value and keeps
-- nothing. Only the *order* is load-bearing.
--
-- `changed_at` is not removed and still travels on the wire. It remains
-- meaningful provenance ("when did this last change"), it is what the human
-- listing sorts by, and a store upgraded before its server still has nothing
-- else to compare against.
CREATE SEQUENCE IF NOT EXISTS team_knowledge_revision_seq AS BIGINT MINVALUE 1;

-- **Backfilled in `changed_at` order, deliberately, and not by the default.**
-- `ALTER TABLE ... ADD COLUMN DEFAULT nextval(...)` would assign a revision to
-- every existing row, but in heap order — which bears no relation to the order
-- those rows changed, so an existing corpus would be handed to every device in
-- an arbitrary sequence. Assigning `row_number()` over the old ordering key
-- instead means the first revision-keyed page of an upgraded server walks the
-- corpus in the same order the timestamp-keyed feed did.
--
-- Written as `SET revision = o.rn` rather than `SET revision = nextval(...)`:
-- the order in which an `UPDATE` evaluates a volatile default across rows is
-- not specified, so `nextval` here would produce distinct values in an
-- unspecified order — which is the thing this statement exists to avoid.
ALTER TABLE team_knowledge ADD COLUMN IF NOT EXISTS revision BIGINT;

UPDATE team_knowledge t
   SET revision = o.rn
  FROM (SELECT id,
               row_number() OVER (
                   ORDER BY GREATEST(created_at, ratified_at, retired_at,
                                     superseded_at), id) AS rn
          FROM team_knowledge) o
 WHERE t.id = o.id;

-- The sequence starts after the backfill, so the first row written by this
-- deployment sorts after every row it already held.
SELECT setval('team_knowledge_revision_seq',
              COALESCE((SELECT MAX(revision) FROM team_knowledge), 0) + 1, false);

-- **One assignor, and it cannot be forgotten.** A `BEFORE INSERT OR UPDATE`
-- trigger rather than a `revision = nextval(...)` clause in each statement,
-- because the invariant is "no write to this row leaves the revision where it
-- was" and a clause per statement is an invariant maintained by remembering.
-- There are four writers today — the proposal ingest, `ratify_team`,
-- `retire_team` and the supersession pointer in `record_supersedes`, and one of
-- them (`POST /api/team`, in `commands.rs`) enumerates its columns and would
-- have needed a fifth edit — and a fifth writer added later would silently
-- produce a change the pull feed can never deliver. The trigger makes that
-- unrepresentable: the revision advances on *every* row write, whether or not
-- the write touched a lifecycle column.
--
-- Over-bumping is the safe direction. A revision that advanced when nothing
-- semantically changed re-delivers the row once, and every importer is
-- idempotent by id; a revision that failed to advance loses the change
-- permanently.
CREATE OR REPLACE FUNCTION team_knowledge_bump_revision() RETURNS trigger AS $$
BEGIN
    NEW.revision := nextval('team_knowledge_revision_seq');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS team_knowledge_revision_bump ON team_knowledge;
CREATE TRIGGER team_knowledge_revision_bump
    BEFORE INSERT OR UPDATE ON team_knowledge
    FOR EACH ROW EXECUTE FUNCTION team_knowledge_bump_revision();

-- `NOT NULL` is set *after* the trigger exists, and there is deliberately no
-- column default: the trigger fires before the constraint is checked, so an
-- `INSERT` that never names `revision` still satisfies it, and a second
-- mechanism assigning the same column would only make it ambiguous which one
-- did.
ALTER TABLE team_knowledge ALTER COLUMN revision SET NOT NULL;

-- `UNIQUE`, because it is true and because it is the invariant the feed rests
-- on: a keyset scan ordered on `revision` alone needs no id tie-break only if
-- no two rows can share a revision. The index is also what makes
-- `WHERE revision > $1 ORDER BY revision LIMIT n` a range scan rather than a
-- sort of the table.
CREATE UNIQUE INDEX IF NOT EXISTS team_knowledge_revision
    ON team_knowledge (revision);
