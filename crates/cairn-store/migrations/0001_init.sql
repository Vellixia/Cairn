-- Cairn local schema (data-model.md).
--
-- SQLite is the local source of truth and works offline. Enums are lowercase
-- text with CHECK constraints so a database client stays readable. Deletion is
-- a tombstone: identity and timestamps survive, content is cleared.

CREATE TABLE IF NOT EXISTS users (
    id           TEXT PRIMARY KEY,
    email        TEXT,
    display_name TEXT NOT NULL,
    created_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS projects (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL,
    -- Local repository instance. Never leaves the machine (FR-064, D14).
    git_common_dir    TEXT NOT NULL UNIQUE,
    -- Normalized remote: a discovery hint, never shared identity.
    repository_remote TEXT,
    linked            INTEGER NOT NULL DEFAULT 0 CHECK (linked IN (0, 1)),
    -- Server-assigned shared identity, set by `cairn link`.
    server_project_id TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    deleted_at        TEXT
);

CREATE TABLE IF NOT EXISTS tasks (
    id                  TEXT PRIMARY KEY,
    project_id          TEXT NOT NULL REFERENCES projects(id),
    title               TEXT NOT NULL,
    goal                TEXT NOT NULL,
    acceptance_criteria TEXT NOT NULL DEFAULT '[]',
    status              TEXT NOT NULL
        CHECK (status IN ('todo', 'in_progress', 'done', 'blocked')),
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    deleted_at          TEXT
);

CREATE TABLE IF NOT EXISTS sessions (
    id                  TEXT PRIMARY KEY,
    project_id          TEXT NOT NULL REFERENCES projects(id),
    task_id             TEXT REFERENCES tasks(id),
    user_id             TEXT NOT NULL,
    agent               TEXT NOT NULL,
    branch              TEXT NOT NULL,
    commit_sha          TEXT,
    -- Scope and context. Never the uniqueness key (FR-010).
    worktree_path       TEXT NOT NULL,
    -- The agent's own session identifier. This is what routes events.
    agent_session_key   TEXT NOT NULL,
    previous_session_id TEXT REFERENCES sessions(id),
    status              TEXT NOT NULL
        CHECK (status IN ('active', 'completed', 'interrupted')),
    started_at          TEXT NOT NULL,
    ended_at            TEXT,
    last_event_at       TEXT NOT NULL,
    -- Set by the `Stop` turn checkpoint. Never ends a session (D16).
    last_turn_ended_at  TEXT,
    daemon_run_id       TEXT NOT NULL,
    end_reason          TEXT,
    deleted_at          TEXT
);

-- Session start is idempotent per agent session, not per worktree (FR-010).
CREATE UNIQUE INDEX IF NOT EXISTS sessions_agent_key
    ON sessions (project_id, agent_session_key) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS sessions_worktree
    ON sessions (project_id, worktree_path, status);
CREATE INDEX IF NOT EXISTS sessions_task_recent
    ON sessions (task_id, ended_at DESC);
CREATE INDEX IF NOT EXISTS sessions_branch_recent
    ON sessions (project_id, branch, ended_at DESC);

CREATE TABLE IF NOT EXISTS observations (
    id            TEXT PRIMARY KEY,
    session_id    TEXT NOT NULL REFERENCES sessions(id),
    type          TEXT NOT NULL CHECK (type IN (
        'file_read', 'file_changed', 'command_run', 'test_run',
        'error', 'decision', 'discovery', 'user_instruction')),
    occurred_at   TEXT NOT NULL,
    branch        TEXT NOT NULL,
    commit_sha    TEXT,
    path          TEXT,
    command       TEXT,
    exit_code     INTEGER,
    outcome       TEXT,
    summary       TEXT NOT NULL,
    details       TEXT,
    payload_bytes INTEGER NOT NULL DEFAULT 0,
    truncated     INTEGER NOT NULL DEFAULT 0 CHECK (truncated IN (0, 1)),
    deleted_at    TEXT
);

CREATE INDEX IF NOT EXISTS observations_session
    ON observations (session_id, occurred_at);

CREATE TABLE IF NOT EXISTS memories (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL REFERENCES projects(id),
    type              TEXT NOT NULL CHECK (type IN (
        'fact', 'decision', 'convention', 'failure', 'procedure')),
    scope             TEXT NOT NULL
        CHECK (scope IN ('project', 'branch', 'task', 'session')),
    scope_key         TEXT NOT NULL,
    content           TEXT NOT NULL,
    state             TEXT NOT NULL DEFAULT 'active'
        CHECK (state IN ('active', 'stale', 'superseded')),
    superseded_by_id  TEXT REFERENCES memories(id),
    -- Mandatory. Evidence is not (FR-019).
    origin_session_id TEXT NOT NULL,
    local_only        INTEGER NOT NULL DEFAULT 0 CHECK (local_only IN (0, 1)),
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    deleted_at        TEXT
);

CREATE INDEX IF NOT EXISTS memories_scope
    ON memories (project_id, scope, scope_key, state);

-- Zero or more rows per memory. Survives deletion of the observation so the
-- reference stays resolvable and reports "evidence deleted" (FR-052).
CREATE TABLE IF NOT EXISTS memory_evidence (
    memory_id      TEXT NOT NULL REFERENCES memories(id),
    observation_id TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    PRIMARY KEY (memory_id, observation_id)
);

CREATE TABLE IF NOT EXISTS handoffs (
    id               TEXT PRIMARY KEY,
    session_id       TEXT NOT NULL REFERENCES sessions(id),
    -- No `stop`: a turn checkpoint is not a handoff boundary (FR-032, D16).
    trigger          TEXT NOT NULL
        CHECK (trigger IN ('pre_compact', 'session_end', 'recovered')),
    goal             TEXT NOT NULL,
    progress         TEXT NOT NULL,
    completed_work   TEXT NOT NULL DEFAULT '[]',
    remaining_work   TEXT NOT NULL DEFAULT '[]',
    changed_files    TEXT NOT NULL DEFAULT '[]',
    decisions        TEXT NOT NULL DEFAULT '[]',
    failures         TEXT NOT NULL DEFAULT '[]',
    tests_executed   TEXT NOT NULL DEFAULT '[]',
    repository_state TEXT NOT NULL DEFAULT '{}',
    next_step        TEXT NOT NULL,
    agent_note       TEXT,
    -- Observation identifiers only. Never their content (FR-055).
    evidence         TEXT NOT NULL DEFAULT '[]',
    created_at       TEXT NOT NULL,
    deleted_at       TEXT
);

CREATE INDEX IF NOT EXISTS handoffs_session
    ON handoffs (session_id, created_at DESC);

-- Transactional outbox (D9). There is deliberately no observation entity type,
-- so a payload carrying observation content cannot be constructed (SC-010).
CREATE TABLE IF NOT EXISTS outbox (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL REFERENCES projects(id),
    server_project_id TEXT NOT NULL,
    entity_type       TEXT NOT NULL
        CHECK (entity_type IN ('project', 'task', 'session', 'memory', 'handoff')),
    entity_id         TEXT NOT NULL,
    operation         TEXT NOT NULL CHECK (operation IN ('upsert', 'delete')),
    idempotency_key   TEXT NOT NULL UNIQUE,
    payload           TEXT NOT NULL,
    state             TEXT NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'in_flight', 'delivered', 'failed')),
    attempts          INTEGER NOT NULL DEFAULT 0,
    last_error        TEXT,
    created_at        TEXT NOT NULL,
    delivered_at      TEXT
);

CREATE INDEX IF NOT EXISTS outbox_pending ON outbox (state, created_at);

CREATE TABLE IF NOT EXISTS sync_meta (
    project_id      TEXT PRIMARY KEY REFERENCES projects(id),
    last_success_at TEXT,
    pull_cursor     TEXT
);
