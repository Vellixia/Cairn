-- Feature 003 — project intelligence. Additive only.
--
-- What is absent from this file *is* the privacy boundary
-- (`contracts/privacy-sync.md` §Server schema and allowlist delta). These tables
-- deliberately do not exist here, and the absence is the mechanism rather than a
-- rule someone must remember:
--
--   evidence_facts            memory_evidence_facts     verification_runs
--   continuity_checkpoints    reusable_patterns         pattern_applications
--   task_changes              criterion_evidence        observations
--
-- A record with nowhere to go cannot be sent by mistake.

-- ---------------------------------------------------------------------------
-- Canonical knowledge on the shared memory row
-- ---------------------------------------------------------------------------

ALTER TABLE memories ADD COLUMN IF NOT EXISTS topic_key             TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS value_key             TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS importance            TEXT NOT NULL DEFAULT 'normal';
ALTER TABLE memories ADD COLUMN IF NOT EXISTS effective_from        TIMESTAMPTZ;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS superseded_at         TIMESTAMPTZ;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS stale_at              TIMESTAMPTZ;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS pinned                BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS reinforcement_count   INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS distinct_origin_count INTEGER NOT NULL DEFAULT 0;

-- What a shared memory may say about its evidence: a state, an authority, a
-- time, a count, and verifier **kind** names. Never a subject, an observed
-- value, a locator, a digest or a fingerprint (FR-502, D66, D76).
ALTER TABLE memories ADD COLUMN IF NOT EXISTS verification           TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS verification_authority TEXT;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS last_verified_at       TIMESTAMPTZ;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS verification_basis     JSONB NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS evidence_fact_count    INTEGER NOT NULL DEFAULT 0;

-- `pin_reason` is deliberately absent: free text a session wrote about local
-- context, and the conservative default applies.

CREATE INDEX IF NOT EXISTS memories_subject
    ON memories (project_id, topic_key, value_key) WHERE topic_key IS NOT NULL;

-- ---------------------------------------------------------------------------
-- The decisions that produce canonical knowledge
-- ---------------------------------------------------------------------------

-- Append-only, and keyed by the endpoint pair plus the kind — so two machines
-- that detect the same conflict while offline write the same primary key and
-- the merge absorbs the second exactly as it absorbs a local duplicate (D78).
--
-- `basis_evidence_id` and `rationale` are absent: the first names evidence that
-- never leaves its machine, the second is free text about local context.
CREATE TABLE IF NOT EXISTS memory_relations (
    from_memory_id     UUID NOT NULL,
    to_memory_id       UUID NOT NULL,
    -- The same six the local store accepts, spelled the same way. A kind the
    -- server does not know is a CHECK violation that fails the whole push, so
    -- this list is not a shorter summary of the local one — it is the local one.
    kind               TEXT NOT NULL CHECK (kind IN (
        'reinforces', 'duplicates', 'supersedes',
        'conflicts_with', 'narrows', 'not_applicable_to')),
    project_id         UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    decided_by_session UUID NOT NULL,
    decided_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    basis              TEXT NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at         TIMESTAMPTZ,
    PRIMARY KEY (from_memory_id, to_memory_id, kind)
);

CREATE INDEX IF NOT EXISTS memory_relations_project
    ON memory_relations (project_id, updated_at);

-- ---------------------------------------------------------------------------
-- Task work state
-- ---------------------------------------------------------------------------

-- No column is added to `tasks`. The local counter is not transmitted and the
-- state digest is derived on both sides from the records below, so there is
-- nothing about a task itself for the server to hold (D80).

CREATE TABLE IF NOT EXISTS task_criteria (
    id           UUID PRIMARY KEY,
    task_id      UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    project_id   UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    ordinal      INTEGER NOT NULL,
    label        TEXT NOT NULL,
    text         TEXT NOT NULL,
    state        TEXT NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'satisfied', 'blocked', 'waived')),
    verification TEXT NOT NULL DEFAULT 'unverified'
        CHECK (verification IN ('unverified', 'verified', 'failed')),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at   TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS task_criteria_task ON task_criteria (task_id, ordinal);
CREATE INDEX IF NOT EXISTS task_criteria_project ON task_criteria (project_id, updated_at);

CREATE TABLE IF NOT EXISTS task_blockers (
    id                 UUID PRIMARY KEY,
    task_id            UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    project_id         UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    description        TEXT NOT NULL,
    state              TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'cleared')),
    opened_by_session  UUID NOT NULL,
    cleared_by_session UUID,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at         TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS task_blockers_task ON task_blockers (task_id, state);
CREATE INDEX IF NOT EXISTS task_blockers_project ON task_blockers (project_id, updated_at);
