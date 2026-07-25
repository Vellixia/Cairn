-- Cairn local storage: initial schema (design artifact for crates/cairn-storage-local/migrations)
-- SQLite, WAL mode. Connection pragmas (not part of migration):
--   PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000;

CREATE TABLE repositories (
    id                          TEXT PRIMARY KEY,               -- UUIDv7
    repo_uuid                   TEXT NOT NULL UNIQUE,           -- <git-common-dir>/cairn/repository-id
    canonical_path              TEXT NOT NULL,                  -- mutable metadata
    default_remote_name         TEXT,
    default_remote_url          TEXT,
    copied_from_repository_id   TEXT REFERENCES repositories(id),
    registered_at               TEXT NOT NULL                   -- RFC 3339 UTC
);

CREATE TABLE worktrees (
    id              TEXT PRIMARY KEY,                           -- UUIDv7
    repository_id   TEXT NOT NULL REFERENCES repositories(id),
    worktree_uuid   TEXT NOT NULL UNIQUE,                       -- <absolute-git-dir>/cairn/worktree-id
    path            TEXT NOT NULL,                              -- mutable
    is_main         INTEGER NOT NULL CHECK (is_main IN (0, 1)),
    registered_at   TEXT NOT NULL
);

CREATE TABLE snapshots (
    id                  TEXT PRIMARY KEY,                       -- UUIDv7
    worktree_id         TEXT NOT NULL REFERENCES worktrees(id),
    branch              TEXT,                                   -- NULL = detached HEAD
    head_commit         TEXT NOT NULL,
    staged_fp           TEXT NOT NULL,
    unstaged_fp         TEXT NOT NULL,
    untracked_fp        TEXT NOT NULL,
    snapshot_fp         TEXT NOT NULL,
    fp_schema_version   INTEGER NOT NULL,
    created_at          TEXT NOT NULL,
    UNIQUE (worktree_id, snapshot_fp)
);

CREATE TABLE sessions (
    id                  TEXT PRIMARY KEY,                       -- UUIDv7, stable session identifier
    repository_id       TEXT NOT NULL REFERENCES repositories(id),
    worktree_id         TEXT NOT NULL REFERENCES worktrees(id),
    local_user          TEXT NOT NULL,
    agent_type          TEXT NOT NULL,
    agent_instance_id   TEXT NOT NULL,
    agent_pid           INTEGER,                                -- supporting liveness metadata only, never identity
    resume_token_hash   TEXT NOT NULL,                          -- BLAKE3(resume token); raw token never stored
    lease_expires_at    TEXT NOT NULL,                          -- staleness clock: start = now + initial lease; heartbeat extends
    state               TEXT NOT NULL CHECK (state IN ('active', 'recovering', 'stopped', 'interrupted')),
    start_snapshot_id   TEXT NOT NULL REFERENCES snapshots(id),
    current_snapshot_id TEXT NOT NULL REFERENCES snapshots(id),
    started_at          TEXT NOT NULL,
    ended_at            TEXT,
    last_heartbeat_at   TEXT NOT NULL,
    recovering_since    TEXT                                    -- set on first entry into recovering; preserved across restarts
);

-- FR-017 / FR-034: at most one live session per agent instance per repository.
CREATE UNIQUE INDEX sessions_one_live_per_instance
    ON sessions (repository_id, agent_instance_id)
    WHERE state IN ('active', 'recovering');

CREATE INDEX sessions_by_repo_state ON sessions (repository_id, state);

CREATE TABLE events (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,          -- total order; assigned in serialized per-worktree txn
    id              TEXT NOT NULL UNIQUE,                       -- UUIDv7
    idempotency_key TEXT NOT NULL UNIQUE,                       -- arch rule 6: duplicate returns prior result
    event_type      TEXT NOT NULL,
    repository_id   TEXT REFERENCES repositories(id),
    worktree_id     TEXT REFERENCES worktrees(id),
    session_id      TEXT REFERENCES sessions(id),
    snapshot_id     TEXT REFERENCES snapshots(id),
    payload         TEXT NOT NULL,                              -- schema-validated JSON
    recorded_at     TEXT NOT NULL
);

CREATE INDEX events_by_repo_seq     ON events (repository_id, seq);
CREATE INDEX events_by_worktree_seq ON events (worktree_id, seq);
CREATE INDEX events_by_session_seq  ON events (session_id, seq);

-- Constitution III / FR-020: append-only enforced in storage, not convention.
CREATE TRIGGER events_no_update BEFORE UPDATE ON events
BEGIN
    SELECT RAISE(ABORT, 'events are append-only');
END;

CREATE TRIGGER events_no_delete BEFORE DELETE ON events
BEGIN
    SELECT RAISE(ABORT, 'events are append-only');
END;

-- Arch rule 2: snapshots immutable.
CREATE TRIGGER snapshots_no_update BEFORE UPDATE ON snapshots
BEGIN
    SELECT RAISE(ABORT, 'snapshots are immutable');
END;

CREATE TRIGGER snapshots_no_delete BEFORE DELETE ON snapshots
BEGIN
    SELECT RAISE(ABORT, 'snapshots are immutable');
END;

CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT INTO meta (key, value) VALUES ('fp_schema_version', '1');
