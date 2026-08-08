-- Cairn server schema (FR-055).
--
-- This is the allowlist, in DDL. There is deliberately **no observations
-- table**: raw observations are local, and referencing one as evidence does not
-- make it shareable. Evidence lives on memories and handoffs as identifiers,
-- a count and an optional digest.

CREATE TABLE IF NOT EXISTS users (
    id            UUID PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    display_name  TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Browser sign-in for the web UI.
CREATE TABLE IF NOT EXISTS web_sessions (
    token_hash TEXT PRIMARY KEY,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);

-- Personal API tokens: the daemon's long-lived credential, revocable without
-- touching the account (D10).
CREATE TABLE IF NOT EXISTS api_tokens (
    id           UUID PRIMARY KEY,
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    token_hash   TEXT NOT NULL UNIQUE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    revoked_at   TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS projects (
    id                UUID PRIMARY KEY,
    name              TEXT NOT NULL,
    -- Normalized remote: repository-link metadata, a discovery hint only.
    repository_remote TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at        TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS projects_remote ON projects (repository_remote);

-- Flat membership: a user is a member or is not (FR-057).
CREATE TABLE IF NOT EXISTS project_members (
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, user_id)
);

CREATE TABLE IF NOT EXISTS tasks (
    id                  UUID PRIMARY KEY,
    project_id          UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title               TEXT NOT NULL,
    goal                TEXT NOT NULL,
    acceptance_criteria JSONB NOT NULL DEFAULT '[]'::jsonb,
    status              TEXT NOT NULL
        CHECK (status IN ('todo', 'in_progress', 'done', 'blocked')),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at          TIMESTAMPTZ
);

-- Minimal session provenance. `worktree_path`, `agent_session_key`,
-- `daemon_run_id` and `last_event_at` are local-only and have no column here.
CREATE TABLE IF NOT EXISTS sessions (
    id                  UUID PRIMARY KEY,
    project_id          UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    task_id             UUID,
    user_id             UUID REFERENCES users(id),
    agent               TEXT NOT NULL,
    branch              TEXT NOT NULL,
    commit_sha          TEXT,
    previous_session_id UUID,
    status              TEXT NOT NULL
        CHECK (status IN ('active', 'completed', 'interrupted')),
    started_at          TIMESTAMPTZ NOT NULL,
    ended_at            TIMESTAMPTZ,
    end_reason          TEXT,
    deleted_at          TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS sessions_project ON sessions (project_id, started_at DESC);

CREATE TABLE IF NOT EXISTS memories (
    id                UUID PRIMARY KEY,
    project_id        UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    type              TEXT NOT NULL
        CHECK (type IN ('fact', 'decision', 'convention', 'failure', 'procedure')),
    scope             TEXT NOT NULL
        CHECK (scope IN ('project', 'branch', 'task', 'session')),
    scope_key         TEXT NOT NULL,
    content           TEXT NOT NULL,
    state             TEXT NOT NULL DEFAULT 'active'
        CHECK (state IN ('active', 'stale', 'superseded')),
    superseded_by_id  UUID,
    -- Provenance references only. The observations stayed on their machine.
    origin_session_id UUID NOT NULL,
    observation_ids   JSONB NOT NULL DEFAULT '[]'::jsonb,
    evidence_count    INTEGER NOT NULL DEFAULT 0,
    evidence_digest   TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at        TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS memories_scope ON memories (project_id, scope, scope_key, state);
CREATE INDEX IF NOT EXISTS memories_search
    ON memories USING GIN (to_tsvector('english', content));
CREATE INDEX IF NOT EXISTS memories_updated ON memories (project_id, updated_at);

CREATE TABLE IF NOT EXISTS handoffs (
    id               UUID PRIMARY KEY,
    project_id       UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    session_id       UUID NOT NULL,
    trigger          TEXT NOT NULL
        CHECK (trigger IN ('pre_compact', 'session_end', 'recovered')),
    goal             TEXT NOT NULL,
    progress         TEXT NOT NULL,
    completed_work   JSONB NOT NULL DEFAULT '[]'::jsonb,
    remaining_work   JSONB NOT NULL DEFAULT '[]'::jsonb,
    changed_files    JSONB NOT NULL DEFAULT '[]'::jsonb,
    decisions        JSONB NOT NULL DEFAULT '[]'::jsonb,
    failures         JSONB NOT NULL DEFAULT '[]'::jsonb,
    tests_executed   JSONB NOT NULL DEFAULT '[]'::jsonb,
    repository_state JSONB NOT NULL DEFAULT '{}'::jsonb,
    next_step        TEXT NOT NULL,
    agent_note       TEXT,
    observation_ids  JSONB NOT NULL DEFAULT '[]'::jsonb,
    evidence_count   INTEGER NOT NULL DEFAULT 0,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at       TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS handoffs_session ON handoffs (session_id, created_at DESC);

-- What the server has already applied. Redelivery is a no-op that returns the
-- same result (FR-056, SC-009).
CREATE TABLE IF NOT EXISTS sync_state (
    idempotency_key TEXT PRIMARY KEY,
    project_id      UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    entity_type     TEXT NOT NULL,
    entity_id       UUID NOT NULL,
    applied_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS sync_state_project ON sync_state (project_id, applied_at DESC);
