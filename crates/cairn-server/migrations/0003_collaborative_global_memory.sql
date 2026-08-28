-- Feature 004 — collaborative global memory. Additive only (FR-523):
-- every statement below either creates a new table or adds a nullable /
-- defaulted column to an existing one. The one exception, run in this same
-- script immediately after the column exists, is the `users.role` backfill
-- (step 2) — a data change, not a schema rebuild, and required so the
-- "never zero admins" guarantee (FR-413, FR-524) holds the instant the
-- column becomes meaningful rather than only at the end of this migration.
--
-- What is absent from this file *is* the privacy boundary, the same
-- discipline `0002_project_intelligence.sql` states for its own tables:
-- no `content_norm_digest`, no `origin_digest`, and — checked field by
-- field — no verification field of any kind on `personal_knowledge` or
-- `team_knowledge` (FR-513, FR-517, D433 Layer A).

-- ---------------------------------------------------------------------------
-- Step 1 — `users` additive columns
-- ---------------------------------------------------------------------------

ALTER TABLE users ADD COLUMN IF NOT EXISTS role
    TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('admin', 'member'));
ALTER TABLE users ADD COLUMN IF NOT EXISTS status
    TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled'));
ALTER TABLE users ADD COLUMN IF NOT EXISTS must_change_password
    BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE users ADD COLUMN IF NOT EXISTS password_changed_at TIMESTAMPTZ;

-- ---------------------------------------------------------------------------
-- Step 2 — the `users.role` backfill (FR-414, FR-524)
-- ---------------------------------------------------------------------------
--
-- Algorithm (migration.md §5):
--   1. If CAIRN_ADMIN_EMAIL is set and a user row has that email, that user
--      becomes admin.
--   2. Otherwise, the single oldest account by created_at becomes admin.
--   3. Every other existing account becomes member.
--   4. If the users table is empty, there is nothing to backfill, and the
--      environment-seeded admin account will be created — already admin,
--      by the same rule applied at the moment it is upserted — on the
--      server's next start.
--
-- `current_setting('cairn.admin_email', true)` reads the same configured
-- identity `auth::ensure_admin` keys off of (CAIRN_ADMIN_EMAIL), so the
-- backfill and the runtime seed never disagree about which email is
-- authoritative. `true` (missing_ok) makes an unset setting read as NULL
-- rather than raising an error, so this migration runs unchanged whether or
-- not the caller has set `cairn.admin_email` for this session.
--
-- Corrected from migration.md's literal text in two ways, same algorithm:
--   - migration.md's second UPDATE references `admin_candidate` outside the
--     WITH clause that defines it, which PostgreSQL does not scope that way;
--     each UPDATE below carries its own copy of the CTE instead.
--   - migration.md's UNION ALL relies on an unspecified row order between
--     its two arms to make the email match win over the oldest-account
--     fallback. An explicit `priority` column plus `ORDER BY priority,
--     created_at` makes that precedence a property of the query the planner
--     must honor, not an accident of execution order.
--
-- Determinism and the "never zero admins" guarantee: the configured-email
-- arm can select at most one row (`email` is UNIQUE), and the fallback arm
-- selects every row there is with priority 1, so the CTE is not empty
-- whenever `users` has at least one row. `ORDER BY priority, created_at
-- LIMIT 1` therefore always resolves to exactly one id — the email match
-- when one exists, otherwise the globally oldest account — and the first
-- UPDATE sets exactly that one row to `admin`. When `users` is empty, the
-- CTE is empty, both UPDATEs' subqueries return NULL, `id = NULL` and
-- `id <> NULL` are both unknown for every row, and neither UPDATE touches
-- anything (case 4 above): there is nothing to leave non-admin.

WITH admin_candidate AS (
    SELECT id, 0 AS priority, created_at FROM users
     WHERE email = current_setting('cairn.admin_email', true)
    UNION ALL
    SELECT id, 1 AS priority, created_at FROM users
)
UPDATE users SET role = 'admin'
 WHERE id = (SELECT id FROM admin_candidate ORDER BY priority, created_at ASC LIMIT 1);

-- Step 3 of the algorithm: everyone else, explicitly, so the DEFAULT
-- 'member' from the ALTER TABLE above is restated here as an assertion
-- rather than relied on silently.
WITH admin_candidate AS (
    SELECT id, 0 AS priority, created_at FROM users
     WHERE email = current_setting('cairn.admin_email', true)
    UNION ALL
    SELECT id, 1 AS priority, created_at FROM users
)
UPDATE users SET role = 'member'
 WHERE id <> (SELECT id FROM admin_candidate ORDER BY priority, created_at ASC LIMIT 1);

-- ---------------------------------------------------------------------------
-- Step 3 — `server_instance`: one row, ever
-- ---------------------------------------------------------------------------
--
-- Exposed unauthenticated at GET /api/version, the same posture as
-- schema_version, so any client — including one that has never linked
-- anything — can discover which server instance it is talking to (FR-416).
-- Immutable: server_instance_id is what a local store pins its team
-- knowledge to (FR-495/FR-496), so this migration inserts exactly one row
-- and no UPDATE anywhere in this feature ever targets `id`.
CREATE TABLE IF NOT EXISTS server_instance (
    id         UUID PRIMARY KEY CHECK (id = id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO server_instance (id, created_at)
SELECT gen_random_uuid(), now()
 WHERE NOT EXISTS (SELECT 1 FROM server_instance);

-- ---------------------------------------------------------------------------
-- Step 4 — `project_members.added_by_user_id` (FR-419)
-- ---------------------------------------------------------------------------
--
-- Nullable, no DEFAULT: a pre-existing membership row backfills to NULL
-- because who added it was never recorded, and fabricating an answer would
-- violate the additive-only policy's rule against inventing unrecorded
-- state — the same rule that leaves `topic_key` and `stale_at` NULL rather
-- than guessed. `created_at` (0001_init.sql) already answers "when" for
-- every row, existing and new, so no second timestamp column is added.
ALTER TABLE project_members ADD COLUMN IF NOT EXISTS added_by_user_id
    UUID REFERENCES users(id);

-- ---------------------------------------------------------------------------
-- Step 5 — `api_tokens.expires_at` (FR-417)
-- ---------------------------------------------------------------------------
--
-- Optional: tokens MAY still be issued with none. NULL means what it means
-- today for every existing token: no expiry.
ALTER TABLE api_tokens ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ;

-- ---------------------------------------------------------------------------
-- Step 6 — the six domain-knowledge tables (data-model.md §3.5-3.10)
-- ---------------------------------------------------------------------------
--
-- Mirror their local counterparts with the columns local-only by design
-- removed: no `content_norm_digest` (a local exact-duplicate diagnostic that
-- never needs to leave the machine that computed it), and no `origin_digest`
-- (D434, FR-551: machine-salted and local-only by construction, exactly
-- like `content_norm_digest` — transmitting it would let the server brute
-- force it against every project identity it already knows).
--
-- `writer_id`/`writer_seq` are the opposite case (D448, FR-582): they DO
-- cross the wire and are declared here under the same
-- `UNIQUE (writer_id, writer_seq)` index the local table already carries, so
-- the invariant is enforced on both sides rather than asserted on one.
-- `writer_seq` stays diagnostic only (FR-583) — nothing on the read or
-- reconciliation path may consult it as an ordering key or a tiebreak.
--
-- No verification column exists on either knowledge table at all — not an
-- authority, not a state, not a timestamp (FR-513, FR-517): there is
-- nowhere on this row for such a value to be stored.

CREATE TABLE IF NOT EXISTS personal_knowledge (
    id               UUID PRIMARY KEY,
    owner_user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    knowledge_type   TEXT NOT NULL CHECK (knowledge_type IN (
        'fact', 'decision', 'convention', 'failure', 'procedure')),
    content          TEXT NOT NULL,
    topic_key        TEXT,
    value_key        TEXT,
    writer_id        TEXT NOT NULL,
    writer_seq       BIGINT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- No REFERENCES: matches the existing deliberate choice for
    -- memory_relations' endpoints (0002_project_intelligence.sql) — a hard
    -- foreign key would refuse an insert that arrives before the row it
    -- names has synced, dropping it silently instead of holding it for
    -- replay.
    superseded_by_id UUID,
    forgotten_at     TIMESTAMPTZ,
    CHECK (value_key IS NULL OR topic_key IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS personal_knowledge_owner
    ON personal_knowledge (owner_user_id, forgotten_at);
CREATE UNIQUE INDEX IF NOT EXISTS personal_knowledge_writer_seq
    ON personal_knowledge (writer_id, writer_seq);

CREATE TABLE IF NOT EXISTS personal_knowledge_applicability (
    personal_id UUID NOT NULL REFERENCES personal_knowledge(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('language', 'tool')),
    value       TEXT NOT NULL,
    PRIMARY KEY (personal_id, kind, value)
);

CREATE TABLE IF NOT EXISTS personal_knowledge_relations (
    from_id    UUID NOT NULL,
    to_id      UUID NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN (
        'reinforces', 'duplicates', 'supersedes',
        'conflicts_with', 'narrows', 'not_applicable_to')),
    basis      TEXT NOT NULL,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (from_id, to_id, kind)
);

CREATE TABLE IF NOT EXISTS team_knowledge (
    id                  UUID PRIMARY KEY,
    knowledge_type      TEXT NOT NULL CHECK (knowledge_type IN (
        'fact', 'decision', 'convention', 'failure', 'procedure')),
    content             TEXT NOT NULL,
    topic_key           TEXT,
    value_key           TEXT,
    state               TEXT NOT NULL DEFAULT 'proposed'
        CHECK (state IN ('proposed', 'authoritative', 'retired')),
    proposed_by_user_id UUID NOT NULL REFERENCES users(id),
    ratified_by_user_id UUID REFERENCES users(id),
    ratified_at         TIMESTAMPTZ,
    writer_id           TEXT NOT NULL,
    writer_seq          BIGINT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    superseded_by_id    UUID,
    -- When the supersession happened, and it exists for one reason: the pull
    -- cursor. `GET /api/sync/changes/team` orders on GREATEST of this row's own
    -- timestamps, so a change with no timestamp of its own cannot move a device
    -- past it — a second device whose cursor had already passed the old entry's
    -- creation would never learn it had been replaced, and would keep serving
    -- guidance an administrator retired. Setting only `superseded_by_id` is
    -- exactly that bug (FR-462).
    superseded_at       TIMESTAMPTZ,
    -- Who retired it, alongside `retired_at` (FR-457). See the local schema's
    -- note: a timestamp alone does not record who acted.
    retired_by_user_id  UUID REFERENCES users(id),
    retired_at          TIMESTAMPTZ,
    CHECK (value_key IS NULL OR topic_key IS NOT NULL),
    CHECK (state = 'proposed'
           OR (ratified_by_user_id IS NOT NULL AND ratified_at IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS team_knowledge_state ON team_knowledge (state);
CREATE UNIQUE INDEX IF NOT EXISTS team_knowledge_writer_seq
    ON team_knowledge (writer_id, writer_seq);

CREATE TABLE IF NOT EXISTS team_knowledge_applicability (
    team_id UUID NOT NULL REFERENCES team_knowledge(id) ON DELETE CASCADE,
    kind    TEXT NOT NULL CHECK (kind IN ('language', 'tool')),
    value   TEXT NOT NULL,
    PRIMARY KEY (team_id, kind, value)
);

-- Mirrors personal_knowledge_relations. `decided_by_writer` and
-- `deleted_at` are dropped here: a relations row is comparator *output*,
-- not a fact a peer needs delivered to it — any store holding the same
-- synced knowledge rows recomputes the same duplicates/conflicts_with
-- relations for itself, deterministically (FR-493).
CREATE TABLE IF NOT EXISTS team_knowledge_relations (
    from_id    UUID NOT NULL,
    to_id      UUID NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN (
        'reinforces', 'duplicates', 'supersedes',
        'conflicts_with', 'narrows', 'not_applicable_to')),
    basis      TEXT NOT NULL,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (from_id, to_id, kind)
);
