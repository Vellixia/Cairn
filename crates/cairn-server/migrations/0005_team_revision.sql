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
-- So the ordering key stops being a clock reading and becomes an allocated
-- revision. In the trace above the revisions are 1 (insert), 2 (ratify),
-- 3 (retire): correct where the timestamps are not.
--
-- **What the feed actually needs is not monotonicity.** It is that a client
-- cannot advance its cursor past a change that commits later — and monotonic
-- allocation does not give that, because allocation happens at statement time
-- and visibility at commit. The allocator below is chosen for the stronger
-- property; the paragraph on it says why, and what it costs.
--
-- `changed_at` is not removed and still travels on the wire. It remains
-- meaningful provenance ("when did this last change"), it is what the human
-- listing sorts by, and a store upgraded before its server still has nothing
-- else to compare against.
-- **The allocator is a row, not a sequence, and that is the whole point.**
--
-- `nextval` hands out numbers at statement time and says nothing about commit
-- order, so two writers interleave like this:
--
--     tx A takes revision 100 and stays open
--     tx B takes revision 101 and commits
--     a pull sees 101, advances its cursor to 101
--     tx A commits; revision 100 is now behind the cursor, forever
--
-- Reproduced against this database: the reader saw only the second row, and the
-- first became invisible to every later pull. A sequence is monotonic, which is
-- not the property the feed needs. The property the feed needs is that a client
-- cannot advance its cursor past a committed change that becomes visible later.
--
-- A counter row gives exactly that, because the allocation is the row lock. A
-- writer that has taken a revision holds that lock until it commits or rolls
-- back, so the next writer cannot take a number until the previous one is
-- visible. Allocation order therefore *is* commit order, and a reader holding
-- revision R has necessarily seen every revision below it.
--
-- Two further properties fall out. A rollback returns the number rather than
-- burning it, so the feed has no gaps to reason about — unlike a sequence,
-- where a rolled-back allocation leaves a hole that looks identical to a row
-- that has not committed yet. And the counter cannot drift from the table,
-- because nothing else writes it.
--
-- The cost is that team writes serialize on one row. Team knowledge is
-- proposed, ratified and retired by administrators, not by capture traffic, so
-- the contention is measured in operations per hour; losing a retirement
-- permanently is not.
CREATE TABLE IF NOT EXISTS team_revision_counter (
    -- One row, enforced by the type system rather than by convention.
    only_row      BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (only_row),
    next_revision BIGINT NOT NULL
);

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
INSERT INTO team_revision_counter (only_row, next_revision)
VALUES (TRUE, COALESCE((SELECT MAX(revision) FROM team_knowledge), 0) + 1)
ON CONFLICT (only_row) DO UPDATE
    SET next_revision = GREATEST(team_revision_counter.next_revision,
                                 EXCLUDED.next_revision);

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
-- `UPDATE … RETURNING` rather than a read then a write: the update takes the
-- row lock, and the value returned is the one this transaction owns until it
-- ends. A `SELECT` followed by an `UPDATE` would let two writers read the same
-- number.
CREATE OR REPLACE FUNCTION next_team_revision() RETURNS BIGINT AS $$
DECLARE
    allocated BIGINT;
BEGIN
    UPDATE team_revision_counter
       SET next_revision = next_revision + 1
     RETURNING next_revision - 1 INTO allocated;
    RETURN allocated;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION team_knowledge_bump_revision() RETURNS trigger AS $$
BEGIN
    NEW.revision := next_team_revision();
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
