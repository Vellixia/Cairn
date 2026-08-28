-- Feature 004 — collaborative global memory (migration.md).
--
-- Additive only, with one exception: `outbox` is rebuilt to widen its
-- `entity_type` CHECK, the same reason and the same recipe
-- `0005_project_intelligence.sql` already used once (FR-528, FR-530). No
-- other existing table is touched and no existing row is rewritten (FR-523).
--
-- Statement order, per migration.md §Local migration:
--   1. New tables and their indexes (personal/team knowledge and their
--      relations/applicability/FTS, project_traits, writer_identity,
--      sync_cursor) — before anything that reads or rebuilds around them.
--   2. `sync_cursor` backfilled from `sync_meta` (§3).
--   3. `outbox` rebuild (§2.12/§1).
--   4. `writer_identity` seed — not expressible in portable SQL (it needs a
--      fresh UUID), so it runs as a Rust step in `migrate.rs`'s `finish(7,
--      tx)` hook, inside this same transaction, immediately after this
--      script — the same pattern migration 5 already uses for
--      `task_criteria` (0005_project_intelligence.sql, migrate.rs `finish`).

-- ---------------------------------------------------------------------------
-- Step 1 — personal knowledge
-- ---------------------------------------------------------------------------

-- One user's durable knowledge, following that user across every project they
-- touch and every device they use. There is deliberately NO `project_id`
-- column: a record that cannot name a project cannot leak one
-- (0005_project_intelligence.sql:236-238, D403, FR-517).
-- NOT server-bound: unlike team_knowledge, this table is never refused on
-- server-instance mismatch (D438, FR-567). One local store can hold the
-- personal knowledge of more than one identity — e.g. after relinking to a
-- different server, where the same human is a different account — and rows
-- are partitioned by owner_user_id; recall surfaces only the currently
-- linked identity's rows.
CREATE TABLE IF NOT EXISTS personal_knowledge (
    id                  TEXT PRIMARY KEY,
    owner_user_id       TEXT NOT NULL,
    -- Reuses the existing five-value memory-type vocabulary (fact, decision,
    -- convention, failure, procedure) rather than inventing a parallel one:
    -- the classification a fact belongs to does not change because of who it
    -- belongs to.
    knowledge_type      TEXT NOT NULL CHECK (knowledge_type IN (
        'fact', 'decision', 'convention', 'failure', 'procedure')),
    content             TEXT NOT NULL,
    topic_key           TEXT,
    value_key           TEXT,
    -- Local only, never transmitted (§6). Exact-duplicate detection, same
    -- construction as `memories.content_norm_digest` (`knowledge.rs:121-138`).
    content_norm_digest TEXT,
    -- Local only, never transmitted (§6, D434). Set only when this record was
    -- created by promotion (D418); NULL for a personal note recorded
    -- directly. A salted digest of the source project's identity, computed
    -- with this machine's local salt — never the identity itself, and never
    -- comparable across machines by design (FR-516, FR-551, FR-552).
    origin_digest       TEXT,
    -- This store's opaque writer identity and this row's position in that
    -- writer's stream (D407, D408, FR-445). Never compared across writers.
    writer_id           TEXT NOT NULL,
    writer_seq          INTEGER NOT NULL,
    created_at          TEXT NOT NULL,
    -- Cached pointer, maintained the same way memories.superseded_by_id is:
    -- derived from a `supersedes` row in personal_knowledge_relations and
    -- rebuildable from it (below). Not a violation of "immutable after
    -- creation" (D405) any more than the same cache is for memories today —
    -- content is never rewritten; this pointer is maintenance state.
    superseded_by_id    TEXT REFERENCES personal_knowledge(id),
    -- Tombstone. A personal note is forgotten, never edited (FR-440, FR-441).
    forgotten_at        TEXT,
    CHECK (value_key IS NULL OR topic_key IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS personal_knowledge_owner
    ON personal_knowledge (owner_user_id, forgotten_at);
CREATE INDEX IF NOT EXISTS personal_knowledge_topic
    ON personal_knowledge (owner_user_id, topic_key) WHERE topic_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS personal_knowledge_content_norm
    ON personal_knowledge (owner_user_id, content_norm_digest)
    WHERE content_norm_digest IS NOT NULL;
-- Gap/duplicate detection within one writer's stream (D408, FR-445, FR-492).
CREATE UNIQUE INDEX IF NOT EXISTS personal_knowledge_writer_seq
    ON personal_knowledge (writer_id, writer_seq);

-- Zero or more conditions under which a personal record applies to a project.
-- No row for a record means universal (D411, FR-435). Applicability is
-- set-membership over a closed vocabulary, never a score (D410, FR-434).
CREATE TABLE IF NOT EXISTS personal_knowledge_applicability (
    personal_id TEXT NOT NULL REFERENCES personal_knowledge(id),
    kind        TEXT NOT NULL CHECK (kind IN ('language', 'tool')),
    value       TEXT NOT NULL,
    PRIMARY KEY (personal_id, kind, value)
);

-- Reconciliation among one user's own personal entries, reusing 003's
-- write-time comparator and read-time derivation unchanged (D406). Same six
-- kinds as memory_relations, same primary key shape, same reason for it:
-- recording a decision twice is a no-op (003's memory_relations PK pattern,
-- reused here). `decided_by_writer` names the writer, not a session:
-- personal-knowledge relations are decided by the comparator running against
-- one user's entries wherever recalled, with no project session in scope. No
-- `basis_evidence_id`: evidence facts are project-scoped and cannot attach to
-- a project-less record (D419).
CREATE TABLE IF NOT EXISTS personal_knowledge_relations (
    from_id    TEXT NOT NULL,
    to_id      TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN (
        'reinforces', 'duplicates', 'supersedes',
        'conflicts_with', 'narrows', 'not_applicable_to')),
    basis      TEXT NOT NULL CHECK (basis IN (
        'deterministic_rule', 'evidence', 'explicit_agent', 'explicit_user')),
    decided_by_writer TEXT NOT NULL,
    decided_at TEXT NOT NULL,
    deleted_at TEXT,
    PRIMARY KEY (from_id, to_id, kind)
);

CREATE INDEX IF NOT EXISTS personal_knowledge_relations_to
    ON personal_knowledge_relations (to_id, kind);

-- Mirrors memory_fts exactly (0002_memory_fts.sql): FTS5 external-content
-- table, same tokenizer, same three triggers. A separate index per domain
-- because BM25 scores from different corpora are not comparable (D425); no
-- cross-domain relevance comparator is invented.
CREATE VIRTUAL TABLE IF NOT EXISTS personal_fts USING fts5(
    content,
    content='personal_knowledge',
    content_rowid='rowid',
    tokenize='unicode61'
);

CREATE TRIGGER IF NOT EXISTS personal_knowledge_fts_ai
    AFTER INSERT ON personal_knowledge BEGIN
    INSERT INTO personal_fts(rowid, content) VALUES (new.rowid, new.content);
END;

CREATE TRIGGER IF NOT EXISTS personal_knowledge_fts_ad
    AFTER DELETE ON personal_knowledge BEGIN
    INSERT INTO personal_fts(personal_fts, rowid, content)
        VALUES ('delete', old.rowid, old.content);
END;

CREATE TRIGGER IF NOT EXISTS personal_knowledge_fts_au
    AFTER UPDATE ON personal_knowledge BEGIN
    INSERT INTO personal_fts(personal_fts, rowid, content)
        VALUES ('delete', old.rowid, old.content);
    INSERT INTO personal_fts(rowid, content) VALUES (new.rowid, new.content);
END;

-- ---------------------------------------------------------------------------
-- Step 2 — team knowledge
-- ---------------------------------------------------------------------------

-- The server-wide default, proposed by any member and made authoritative only
-- by an admin. Immutable content, same as personal (D405); the one mutation
-- 004 allows is the state transition below (D409). No `project_id` column
-- (D403) — same sentence as personal_knowledge above.
CREATE TABLE IF NOT EXISTS team_knowledge (
    id                   TEXT PRIMARY KEY,
    knowledge_type       TEXT NOT NULL CHECK (knowledge_type IN (
        'fact', 'decision', 'convention', 'failure', 'procedure')),
    content              TEXT NOT NULL,
    topic_key            TEXT,
    value_key            TEXT,
    content_norm_digest  TEXT,
    -- Local only, never transmitted (§6, D434). Same construction and same
    -- per-machine scoping as personal_knowledge.origin_digest above
    -- (FR-516, FR-551, FR-552).
    origin_digest        TEXT,
    -- The one field this table's rows mutate in place, and only by CAS
    -- (D409, FR-454). Three values, so the CAS predicate is the state itself
    -- (`WHERE id = ? AND state = ?expected`) rather than a separate integer
    -- revision column like task_criteria's — there is nothing a numeric
    -- revision would express that the three-value state does not already.
    state                TEXT NOT NULL DEFAULT 'proposed' CHECK (state IN (
        'proposed', 'authoritative', 'retired')),
    -- Traceable reference only, never project-identifying content (FR-459).
    proposed_by_user_id  TEXT NOT NULL,
    ratified_by_user_id  TEXT,
    ratified_at          TEXT,
    writer_id            TEXT NOT NULL,
    writer_seq           INTEGER NOT NULL,
    created_at           TEXT NOT NULL,
    superseded_by_id     TEXT REFERENCES team_knowledge(id),
    -- Who retired it, alongside `retired_at` (FR-457).
    --
    -- Ratification already recorded both halves; retirement recorded only the
    -- clock. "Every state transition MUST be recorded with who acted and when"
    -- is not satisfied by a timestamp on its own — a retirement is the act that
    -- removes guidance from every user on the server, and it is the one most
    -- worth being able to attribute afterwards.
    retired_by_user_id   TEXT,
    retired_at           TEXT,
    CHECK (value_key IS NULL OR topic_key IS NOT NULL),
    -- Authoritative or retired both imply a ratification happened; retiring
    -- an entry that was never ratified is not a reachable transition
    -- (FR-453, FR-456).
    CHECK (state = 'proposed'
           OR (ratified_by_user_id IS NOT NULL AND ratified_at IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS team_knowledge_state ON team_knowledge (state);
CREATE INDEX IF NOT EXISTS team_knowledge_topic
    ON team_knowledge (topic_key) WHERE topic_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS team_knowledge_proposer
    ON team_knowledge (proposed_by_user_id, state);
CREATE UNIQUE INDEX IF NOT EXISTS team_knowledge_writer_seq
    ON team_knowledge (writer_id, writer_seq);

-- Same shape and same closed vocabulary as personal_knowledge_applicability
-- (D410, FR-460): `language | tool` only, `topic` removed (D439, FR-569).
CREATE TABLE IF NOT EXISTS team_knowledge_applicability (
    team_id TEXT NOT NULL REFERENCES team_knowledge(id),
    kind    TEXT NOT NULL CHECK (kind IN ('language', 'tool')),
    value   TEXT NOT NULL,
    PRIMARY KEY (team_id, kind, value)
);

-- Reconciliation among team knowledge entries, same six kinds and same
-- primary-key shape as memory_relations (0005_project_intelligence.sql) and
-- personal_knowledge_relations above: recording a decision twice is a no-op.
-- `duplicates` and `conflicts_with` are detected automatically
-- (`RelationKind::is_automatic`, `domain.rs:393-395`); `supersedes` here is
-- written only by the ratifying admin, as a deliberate act, never inferred.
CREATE TABLE IF NOT EXISTS team_knowledge_relations (
    from_id    TEXT NOT NULL,
    to_id      TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN (
        'reinforces', 'duplicates', 'supersedes',
        'conflicts_with', 'narrows', 'not_applicable_to')),
    basis      TEXT NOT NULL CHECK (basis IN (
        'deterministic_rule', 'evidence', 'explicit_agent', 'explicit_user')),
    decided_by_writer TEXT NOT NULL,
    decided_at TEXT NOT NULL,
    deleted_at TEXT,
    PRIMARY KEY (from_id, to_id, kind)
);

CREATE INDEX IF NOT EXISTS team_knowledge_relations_to
    ON team_knowledge_relations (to_id, kind);

-- Mirrors memory_fts exactly, same as personal_fts above, over team_knowledge.
CREATE VIRTUAL TABLE IF NOT EXISTS team_fts USING fts5(
    content,
    content='team_knowledge',
    content_rowid='rowid',
    tokenize='unicode61'
);

CREATE TRIGGER IF NOT EXISTS team_knowledge_fts_ai
    AFTER INSERT ON team_knowledge BEGIN
    INSERT INTO team_fts(rowid, content) VALUES (new.rowid, new.content);
END;

CREATE TRIGGER IF NOT EXISTS team_knowledge_fts_ad
    AFTER DELETE ON team_knowledge BEGIN
    INSERT INTO team_fts(team_fts, rowid, content)
        VALUES ('delete', old.rowid, old.content);
END;

CREATE TRIGGER IF NOT EXISTS team_knowledge_fts_au
    AFTER UPDATE ON team_knowledge BEGIN
    INSERT INTO team_fts(team_fts, rowid, content)
        VALUES ('delete', old.rowid, old.content);
    INSERT INTO team_fts(rowid, content) VALUES (new.rowid, new.content);
END;

-- ---------------------------------------------------------------------------
-- Step 3 — project traits, writer identity, per-namespace sync cursors
-- ---------------------------------------------------------------------------

-- A project's derived stack signals: manifest and lockfile presence only
-- (Cargo.toml => rust+cargo, package.json => node, pnpm-lock.yaml => pnpm,
-- Dockerfile => docker, etc. — D413). Derived from the repository, never
-- guessed (Constitution VI, FR-437). LOCAL ONLY, never synchronized (FR-438).
CREATE TABLE IF NOT EXISTS project_traits (
    project_id TEXT NOT NULL REFERENCES projects(id),
    kind       TEXT NOT NULL CHECK (kind IN ('language', 'tool')),
    value      TEXT NOT NULL,
    PRIMARY KEY (project_id, kind, value)
);

-- A single opaque identity for this local store, established once (D407,
-- FR-490). Not a device registry: no name, no lifecycle, no server-side
-- table. LOCAL ONLY, never synchronized: see the writer_id/writer_seq
-- columns above for what travels instead.
--
-- The `CHECK (id = 1)` singleton pattern is the simplest correct way to state
-- "exactly one row, forever" in SQLite DDL. `writer_id` is generated once, by
-- this migration's Rust finish-hook (`migrate.rs`'s `finish(7, tx)`), and
-- never regenerated.
CREATE TABLE IF NOT EXISTS writer_identity (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    writer_id  TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- One pull position per synchronization namespace, replacing the single
-- project-keyed cursor in sync_meta (D426, FR-486, FR-487). Backfilled from
-- sync_meta below.
CREATE TABLE IF NOT EXISTS sync_cursor (
    namespace         TEXT PRIMARY KEY,
    pull_cursor       TEXT,
    last_success_at   TEXT,
    -- Per-namespace backoff state (D427, FR-497) — the mechanism that keeps a
    -- blocked team or personal namespace from throttling project sync, which
    -- the single process-global backoff at cairnd/src/sync.rs cannot do
    -- today.
    -- Reserved by `data-model.md` §7's schema and deliberately unwritten.
    --
    -- Per-namespace backoff is held in memory, per worker task
    -- (`cairnd::sync::NamespaceClock`), which is what
    -- `contracts/sync-namespaces.md` §4 asks for — "a `HashMap<SyncNamespace,
    -- Duration>` or an equivalent per-namespace field". Nothing reads or writes
    -- this column, and the accessors that used to exist for it were removed
    -- rather than left as an API implying live state: a retry deadline that
    -- survived a daemon restart is a behaviour change nobody asked for, and a
    -- reader who found the column populated would reasonably assume it governed
    -- something.
    backoff_until     TEXT,
    -- The last capability fingerprint observed for whatever this namespace
    -- talks to, replacing sync_meta.server_capability's single value
    -- (FR-498).
    server_capability TEXT
);

-- ---------------------------------------------------------------------------
-- Step 4 — sync_cursor backfilled from sync_meta (migration.md §3)
-- ---------------------------------------------------------------------------

-- Every existing row becomes namespace `project:<project_id>`, and
-- `pull_cursor` is carried over verbatim — it is an opaque RFC-3339 string
-- produced by the server's `page_cursor` (`cairn-server/src/sync.rs`), and
-- 004 does not reinterpret it. `last_success_at` and `server_capability` are
-- carried the same way. `backoff_until` has no source column and starts
-- NULL — no store has ever computed a per-namespace backoff before this
-- migration.
--
-- `sync_meta` is retained, not dropped (migration.md §3): it is the only
-- durable audit trail of what a project synced before this feature, and
-- nothing in this migration changes the code that still reads it.
INSERT INTO sync_cursor (namespace, pull_cursor, last_success_at, server_capability)
SELECT 'project:' || project_id, pull_cursor, last_success_at, server_capability
  FROM sync_meta;

-- ---------------------------------------------------------------------------
-- Step 5 — the outbox rebuild (migration.md §1, FR-528)
-- ---------------------------------------------------------------------------
--
-- SQLite cannot widen a CHECK constraint in place;
-- `0005_project_intelligence.sql` already rebuilt this exact table for this
-- exact reason (widening `entity_type` to add `memory_relation`,
-- `task_criterion`, `task_blocker` plus the `blocked` state), so the
-- precedent, the recipe and the cost of a rebuild are already established.
--
-- No row is dropped, no row's `payload`, `idempotency_key`, `state`,
-- `attempts` or `last_error` changes. `claimed_at` is copied verbatim, so an
-- in-flight claim's staleness is judged exactly as it would have been before
-- the rebuild — this is the "release claims" step, and it is nothing more
-- than carrying the column across unchanged.

CREATE TABLE outbox_new (
    id                    TEXT PRIMARY KEY,
    -- Nullable now: a personal_knowledge/team_knowledge row belongs to no
    -- project, so it has none to name. The CHECK below makes "nullable for
    -- exactly the two domain-knowledge types, populated for every other
    -- type" a constraint the database enforces rather than a convention a
    -- caller must remember.
    project_id            TEXT REFERENCES projects(id),
    server_project_id     TEXT,
    -- Twelve names: the eight that existed, plus the two knowledge types and
    -- the two relation types.
    --
    -- The relation types are here because both relations tables exist in server
    -- Postgres as well as in this store (global-memory.md §2, server migration
    -- 0003), and a table on the server is reachable only through the outbox. A
    -- relation also has no single parent row to travel inside — it names two —
    -- so unlike applicability, which rides in its knowledge row's payload, a
    -- relation needs an entity type of its own.
    entity_type           TEXT NOT NULL CHECK (entity_type IN (
        'project', 'task', 'session', 'memory', 'handoff',
        'memory_relation', 'task_criterion', 'task_blocker',
        'personal_knowledge', 'personal_knowledge_relation',
        'team_knowledge', 'team_knowledge_relation')),
    entity_id             TEXT NOT NULL,
    operation             TEXT NOT NULL CHECK (operation IN ('upsert', 'delete')),
    idempotency_key       TEXT NOT NULL UNIQUE,
    payload               TEXT NOT NULL,
    state                 TEXT NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'in_flight', 'delivered', 'failed', 'blocked')),
    attempts              INTEGER NOT NULL DEFAULT 0,
    last_error            TEXT,
    created_at            TEXT NOT NULL,
    delivered_at          TEXT,
    claimed_at            TEXT,
    blocked_reason        TEXT,
    blocked_at_capability TEXT,
    -- The routing and backoff key (D426, D427). For the eight pre-existing
    -- entity types this is always `project:<project_id>`; for the two new
    -- types it is `personal:<server_instance_id>:<owner_user_id>` (D438,
    -- FR-568) or `team:<server_instance_id>`.
    namespace             TEXT NOT NULL,
    -- Project-less for exactly the four domain types, populated for every
    -- other one. A relation between two personal records has no more of a
    -- project than the records do.
    CHECK ((entity_type IN ('personal_knowledge', 'personal_knowledge_relation',
                            'team_knowledge', 'team_knowledge_relation'))
           = (project_id IS NULL))
);

INSERT INTO outbox_new
    (id, project_id, server_project_id, entity_type, entity_id, operation,
     idempotency_key, payload, state, attempts, last_error, created_at,
     delivered_at, claimed_at, blocked_reason, blocked_at_capability, namespace)
SELECT id, project_id, server_project_id, entity_type, entity_id, operation,
       idempotency_key, payload, state, attempts, last_error, created_at,
       delivered_at, claimed_at, blocked_reason, blocked_at_capability,
       'project:' || project_id
  FROM outbox;

DROP TABLE outbox;
ALTER TABLE outbox_new RENAME TO outbox;

CREATE INDEX IF NOT EXISTS outbox_pending ON outbox (state, created_at);
-- Replaces outbox_claimable's (project_id, ...) shape: the claim predicate
-- moves from "this project" to "this namespace" (D427), and project rows'
-- namespace is exactly their project_id restated, so this index serves every
-- claim query, project or global, with one definition.
CREATE INDEX IF NOT EXISTS outbox_claimable
    ON outbox (namespace, state, created_at);
