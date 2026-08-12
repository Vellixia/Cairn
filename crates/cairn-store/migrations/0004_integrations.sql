-- The local integration record (FR-182, FR-183, data-model.md).
--
-- All of this is machine-local state. **None of these tables has an outbox
-- entity type and the outbox enqueue path is never called for any of them**:
-- an agent configuration path, a content hash, or an integration health detail
-- must never reach the shared server (FR-183, FR-184, SC-120).
--
-- Two tables where a flag would have been shorter, and the reason: connect
-- asks "does this resource already exist", disconnect asks "is anyone else
-- still using it", and doctor asks "who does this serve". One `satisfied_by`
-- string answered only the third, and under it disconnecting Codex deleted the
-- `AGENTS.md` block OpenCode was still relying on (D28).

-- One row per connected agent on this machine.
--
-- Survives while the agent holds any binding, including a binding to a
-- manager-owned resource awaiting withdrawal (FR-244).
CREATE TABLE IF NOT EXISTS agent_integrations (
    agent                TEXT PRIMARY KEY,
    adapter_version      INTEGER NOT NULL,
    detected_version     TEXT,
    compatibility        TEXT NOT NULL,
    level                TEXT NOT NULL,
    completion_guarantee TEXT NOT NULL,
    connected_at         TEXT NOT NULL,
    last_verified_at     TEXT
);

-- One row per detected integration manager. Holds no path into the manager's
-- own storage and no manager credential (FR-232).
CREATE TABLE IF NOT EXISTS manager_integrations (
    manager          TEXT PRIMARY KEY,
    detected_version TEXT,
    compatibility    TEXT NOT NULL,
    target_apps      TEXT NOT NULL,
    connected_at     TEXT NOT NULL,
    last_verified_at TEXT
);

-- One *physical* thing Cairn installed: a file, a managed block, or a
-- configuration entry. Identified by where it is, not by who uses it.
--
-- `owner = 'manager'` implies `content_hash IS NULL`: Cairn did not write the
-- bytes and does not own them, so verification compares presence and effective
-- configuration instead of equality.
CREATE TABLE IF NOT EXISTS installed_resources (
    id                UUID PRIMARY KEY,
    kind              TEXT NOT NULL,
    owner             TEXT NOT NULL,
    scope             TEXT NOT NULL,
    location          TEXT NOT NULL,
    content_hash      TEXT,
    artifact_schema   INTEGER,
    artifact_revision TEXT,
    activation        TEXT NOT NULL,
    installed_at      TEXT NOT NULL,
    last_verified_at  TEXT,
    -- One bit about Cairn's own edit, not a copy of the developer's file
    -- (FR-156, FR-238): whether the container Cairn wrote into was on a single
    -- line, so removal can put the layout back exactly.
    container_single_line INTEGER NOT NULL DEFAULT 0,
    -- Whether Cairn created the enclosing key, so pruning removes only what
    -- Cairn added.
    created_container     INTEGER NOT NULL DEFAULT 0
);

-- One row per physical resource (FR-146 steady state).
CREATE UNIQUE INDEX IF NOT EXISTS installed_resources_identity
    ON installed_resources (kind, location);

-- One agent's dependency on an installed resource: the reference count that
-- makes shared resources safe (D28, FR-243).
--
-- Several bindings may point at one resource. Two do in practice: the
-- `AGENTS.md` managed block serves Codex and OpenCode, and Claude Code's
-- per-user Skill can serve OpenCode, which scans `~/.claude/skills`.
CREATE TABLE IF NOT EXISTS resource_bindings (
    agent       TEXT NOT NULL REFERENCES agent_integrations (agent) ON DELETE CASCADE,
    kind        TEXT NOT NULL,
    resource_id UUID NOT NULL REFERENCES installed_resources (id) ON DELETE CASCADE,
    bound_at    TEXT NOT NULL,
    -- An agent depends on exactly one resource per kind.
    PRIMARY KEY (agent, kind)
);

CREATE INDEX IF NOT EXISTS resource_bindings_by_resource
    ON resource_bindings (resource_id);

-- What Cairn has established about one capability on this installation: the
-- record behind `confidence` (FR-242, FR-245, D19a).
--
-- `introspection` evidence is version-independent — it proves a fact about a
-- resource Cairn wrote. `observation` evidence is version-bound: what a
-- previous build did is not evidence about this one, so the row is deleted
-- when the detected version changes.
CREATE TABLE IF NOT EXISTS capability_evidence (
    agent          TEXT NOT NULL REFERENCES agent_integrations (agent) ON DELETE CASCADE,
    capability     TEXT NOT NULL,
    evidence       TEXT NOT NULL,
    established_at TEXT NOT NULL,
    agent_version  TEXT,
    -- `context_at_session_open` only: whether the establishing delivery
    -- carried a degraded briefing.
    degraded       INTEGER,
    PRIMARY KEY (agent, capability)
);

-- The explicit ownership transition FR-228 requires. Present only while a
-- migration is in flight; at most one per (agent, kind).
CREATE TABLE IF NOT EXISTS migration_states (
    id                UUID PRIMARY KEY,
    agent             TEXT NOT NULL,
    kind              TEXT NOT NULL,
    source_owner      TEXT NOT NULL,
    source_scope      TEXT NOT NULL,
    source_location   TEXT NOT NULL,
    target_owner      TEXT NOT NULL,
    target_scope      TEXT NOT NULL,
    target_location   TEXT NOT NULL,
    phase             TEXT NOT NULL,
    overlap_permitted INTEGER NOT NULL,
    started_at        TEXT NOT NULL,
    -- Redacted; never carries file content.
    last_error        TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS migration_states_identity
    ON migration_states (agent, kind);

-- Metadata for content preserved before a forced repair (D39, FR-222).
--
-- The artifact file holds **only** Cairn-owned prior content: the managed
-- block body, the canonical serialization of the owned entry, or a whole file
-- Cairn generated in full. Never the enclosing configuration file (FR-238).
-- Its content is never logged and never enters diagnostics; only its path is
-- ever printed (FR-239).
CREATE TABLE IF NOT EXISTS recovery_artifacts (
    id            UUID PRIMARY KEY,
    agent         TEXT NOT NULL,
    kind          TEXT NOT NULL,
    source_path   TEXT NOT NULL,
    artifact_path TEXT NOT NULL,
    content_hash  TEXT NOT NULL,
    created_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS recovery_artifacts_retention
    ON recovery_artifacts (agent, kind, created_at);

-- Additive extensions to Feature 001 entities (data-model.md).

-- The raw vendor tool name kept as bounded provenance (FR-122, D36).
-- Normalized to [A-Za-z0-9_.-], truncated to 64 characters, and passed through
-- redaction like every other field. Not part of any outbox payload —
-- observations never sync at all (FR-055).
ALTER TABLE observations ADD COLUMN vendor_tool TEXT;

-- The sealed close (D22, FR-240). Set inside the seal transaction at session
-- close and cleared when the handoff is written.
--
-- Progress is guaranteed while the daemon runs: the synthesis task retries
-- with bounded backoff, and the maintenance tick that already reaps idle
-- sessions sweeps anything left owing. A terminal session never sits silently
-- owing a handoff.
ALTER TABLE sessions ADD COLUMN handoff_pending INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN handoff_attempts INTEGER NOT NULL DEFAULT 0;
-- Redacted failure reason; never file or conversation content.
ALTER TABLE sessions ADD COLUMN handoff_error TEXT;

-- The sweep reads terminal sessions still owing a handoff, oldest first.
CREATE INDEX IF NOT EXISTS sessions_awaiting_handoff
    ON sessions (handoff_pending, ended_at);
