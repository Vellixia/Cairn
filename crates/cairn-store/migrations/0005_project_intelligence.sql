-- Feature 003 — project intelligence (migration.md).
--
-- Additive only. No existing migration is edited, and no existing row is
-- rewritten beyond the two documented backfills in step 2 (FR-513, FR-514).
--
-- `ADD COLUMN` in SQLite writes the table header, not the rows, so a store with
-- 100,000 memories migrates in milliseconds and the existing rows are untouched
-- literally rather than only semantically.
--
-- `CHECK` constraints cannot be added to an existing SQLite table without
-- rebuilding it, which would rewrite every row in a user's `memories` table.
-- The predicates a CHECK would express are therefore enforced at the repository
-- boundary for `memories`, `tasks`, `sessions`, `outbox` and `sync_meta`, and
-- asserted by test. New tables carry their CHECK constraints in DDL as usual,
-- per the Feature 001 convention.
--
-- `memory_fts` and its three triggers are deliberately not touched. An
-- external-content FTS table with triggers is the most likely thing a careless
-- additive migration breaks, and `migration_alpha4` asserts it was not.

-- ---------------------------------------------------------------------------
-- Step 1 — additive columns on `memories`
-- ---------------------------------------------------------------------------

ALTER TABLE memories ADD COLUMN topic_key              TEXT;
ALTER TABLE memories ADD COLUMN value_key              TEXT;
ALTER TABLE memories ADD COLUMN content_norm_digest    TEXT;
ALTER TABLE memories ADD COLUMN importance             TEXT NOT NULL DEFAULT 'normal';
ALTER TABLE memories ADD COLUMN verification           TEXT NOT NULL DEFAULT 'unverified';
ALTER TABLE memories ADD COLUMN verification_authority TEXT;
ALTER TABLE memories ADD COLUMN last_verified_at       TEXT;
ALTER TABLE memories ADD COLUMN effective_from         TEXT;
ALTER TABLE memories ADD COLUMN superseded_at          TEXT;
ALTER TABLE memories ADD COLUMN stale_at               TEXT;
ALTER TABLE memories ADD COLUMN pinned                 INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN pinned_at              TEXT;
ALTER TABLE memories ADD COLUMN pinned_by_session      TEXT;
ALTER TABLE memories ADD COLUMN pin_reason             TEXT;
ALTER TABLE memories ADD COLUMN reinforcement_count    INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN distinct_origin_count  INTEGER NOT NULL DEFAULT 1;

-- ---------------------------------------------------------------------------
-- Step 2 — two backfills, both bounded and explicit
-- ---------------------------------------------------------------------------

-- (a) Known, exact: a memory was effective from when it was created.
UPDATE memories SET effective_from = created_at WHERE effective_from IS NULL;

-- (b) Approximated, and the only one in the feature. For a memory already
--     superseded before this feature existed, `updated_at` is the best
--     available record of when that happened, because `supersede_memory` sets
--     `state`, `superseded_by_id` and `updated_at` together and nothing else
--     touches a superseded row.
--
--     It is wrong only for a memory superseded and then touched again by
--     something else, in practice a delete tombstone, which clears content
--     anyway. Its consequence is bounded: `superseded_at` is read only by
--     `as_of` historical queries, so an imprecise value can misplace a
--     pre-existing superseded memory in a historical window. It cannot affect
--     current knowledge, reconciliation, verification or context.
UPDATE memories SET superseded_at = updated_at
 WHERE state = 'superseded' AND superseded_at IS NULL;

-- Deliberately NOT backfilled, and each absence is a decision:
--   topic_key, value_key       inferring a subject from prose is what FR-317
--                              and D46 forbid
--   content_norm_digest        computed on the next write or by
--                              `doctor --rebuild-derived`; NULL means "no
--                              exact-duplicate detection for this row yet"
--   verification               'unverified' is the honest state; no evidence
--                              exists, so nothing is verified
--   verification_authority     meaningless unless verified, and nothing is
--   stale_at                   a memory already stale has no authoritative
--                              instant, and several paths touch `updated_at`,
--                              so it is a worse source here than for
--                              supersession. NULL means UNKNOWN, which is
--                              exactly what a historical answer will say
--   reinforcement_count        no relations exist yet
--   distinct_origin_count      exactly one origin session, which is true
--   pinned                     no one has pinned anything

-- ---------------------------------------------------------------------------
-- Step 3 — new tables (data-model.md §3)
-- ---------------------------------------------------------------------------

-- Reconciliation decisions. These records *are* the reconciliation: there is no
-- canonical row, so the project's answer is derived from proposals and
-- decisions and there is nothing for a later writer to overwrite (D44, D47).
--
-- The primary key is what makes recording the same decision twice a no-op, on
-- one machine and across a merge (FR-305, FR-336, I2). `conflicts_with` has its
-- endpoints normalized to (min, max) before the write, so two machines
-- detecting one conflict while offline produce one row rather than two facing
-- opposite ways (D78).
CREATE TABLE IF NOT EXISTS memory_relations (
    from_memory_id     TEXT NOT NULL REFERENCES memories(id),
    to_memory_id       TEXT NOT NULL REFERENCES memories(id),
    kind               TEXT NOT NULL CHECK (kind IN (
        'reinforces', 'duplicates', 'supersedes',
        'conflicts_with', 'narrows', 'not_applicable_to')),
    project_id         TEXT NOT NULL REFERENCES projects(id),
    decided_by_session TEXT NOT NULL,
    decided_at         TEXT NOT NULL,
    basis              TEXT NOT NULL CHECK (basis IN (
        'deterministic_rule', 'evidence', 'explicit_agent', 'explicit_user')),
    basis_evidence_id  TEXT,
    rationale          TEXT,
    deleted_at         TEXT,
    PRIMARY KEY (from_memory_id, to_memory_id, kind)
);

CREATE INDEX IF NOT EXISTS memory_relations_project ON memory_relations (project_id, kind);
CREATE INDEX IF NOT EXISTS memory_relations_to ON memory_relations (to_memory_id, kind);
CREATE INDEX IF NOT EXISTS memory_relations_basis_evidence
    ON memory_relations (project_id, basis_evidence_id);

-- Bounded, redacted, attributable records of an observed state of the world.
--
-- Local, always: there is no outbox entity type and no server table for this,
-- which is what makes "evidence content never leaves the machine" a property of
-- the schema rather than a promise (FR-502, I8).
CREATE TABLE IF NOT EXISTS evidence_facts (
    id                   TEXT PRIMARY KEY,
    project_id           TEXT NOT NULL REFERENCES projects(id),
    kind                 TEXT NOT NULL CHECK (kind IN (
        'observation', 'file', 'git_ref', 'configuration',
        'test_outcome', 'command_outcome', 'runtime_state', 'schema_version')),
    collector            TEXT NOT NULL CHECK (collector IN ('cairn', 'agent')),
    subject              TEXT NOT NULL,
    observed_value       TEXT,
    value_digest         TEXT,
    source_locator       TEXT,
    fingerprint          TEXT,
    observation_id       TEXT,
    repo_branch          TEXT NOT NULL,
    repo_commit          TEXT,
    collected_at         TEXT NOT NULL,
    collected_by_session TEXT NOT NULL,
    local_only           INTEGER NOT NULL DEFAULT 1 CHECK (local_only IN (0, 1)),
    deleted_at           TEXT
);

-- The drift-marking lookup: exact locator equality, capped per event.
CREATE INDEX IF NOT EXISTS evidence_facts_locator
    ON evidence_facts (project_id, source_locator);
CREATE INDEX IF NOT EXISTS evidence_facts_kind ON evidence_facts (project_id, kind);
CREATE INDEX IF NOT EXISTS evidence_facts_fingerprint
    ON evidence_facts (project_id, fingerprint);

-- Evidence links carry a role, so a fact can contradict as well as support
-- (FR-359). The row survives deletion of the fact, so the reference resolves to
-- "evidence deleted" rather than disappearing (FR-358, FR-505).
CREATE TABLE IF NOT EXISTS memory_evidence_facts (
    memory_id          TEXT NOT NULL REFERENCES memories(id),
    evidence_id        TEXT NOT NULL,
    role               TEXT NOT NULL CHECK (role IN ('supports', 'contradicts')),
    attached_at        TEXT NOT NULL,
    attached_by_session TEXT NOT NULL,
    PRIMARY KEY (memory_id, evidence_id, role)
);

CREATE INDEX IF NOT EXISTS memory_evidence_facts_evidence
    ON memory_evidence_facts (evidence_id);

-- Append-only deterministic checks. A later run never rewrites an earlier one;
-- only the memory's or criterion's cached state changes (FR-364).
CREATE TABLE IF NOT EXISTS verification_runs (
    id              TEXT PRIMARY KEY,
    memory_id       TEXT,
    criterion_id    TEXT,
    project_id      TEXT NOT NULL REFERENCES projects(id),
    verifier        TEXT NOT NULL CHECK (verifier IN (
        'file_exists', 'file_digest', 'git_ref', 'git_commit', 'configuration',
        'schema_version', 'test_outcome', 'command_outcome', 'runtime_state')),
    evidence_id     TEXT,
    expected_digest TEXT,
    observed_digest TEXT,
    result          TEXT NOT NULL CHECK (result IN ('verified', 'drifted', 'inconclusive')),
    detail          TEXT,
    repo_branch     TEXT NOT NULL,
    repo_commit     TEXT,
    checked_at      TEXT NOT NULL,
    triggered_by    TEXT NOT NULL CHECK (triggered_by IN (
        'background_pass', 'on_demand', 'attach')),
    CHECK (memory_id IS NOT NULL OR criterion_id IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS verification_runs_memory
    ON verification_runs (memory_id, checked_at DESC);
CREATE INDEX IF NOT EXISTS verification_runs_criterion
    ON verification_runs (criterion_id, checked_at DESC);
CREATE INDEX IF NOT EXISTS verification_runs_result
    ON verification_runs (project_id, result);

-- Structured work state at a boundary, anchored to the handoff Cairn already
-- derives. Not a summary of conversation, and not dependent on any provider's
-- compression quality (FR-421, D55). Local.
CREATE TABLE IF NOT EXISTS continuity_checkpoints (
    id                        TEXT PRIMARY KEY,
    session_id                TEXT NOT NULL REFERENCES sessions(id),
    project_id                TEXT NOT NULL REFERENCES projects(id),
    handoff_id                TEXT NOT NULL REFERENCES handoffs(id),
    trigger                   TEXT NOT NULL CHECK (trigger IN (
        'context_compacting', 'session_closed', 'explicit')),
    assumed_branch            TEXT NOT NULL,
    assumed_commit            TEXT,
    assumed_task_id           TEXT,
    assumed_task_state_digest TEXT,
    relevant_paths            TEXT NOT NULL DEFAULT '[]',
    path_fingerprints         TEXT NOT NULL DEFAULT '[]',
    criteria_snapshot         TEXT NOT NULL DEFAULT '[]',
    open_blockers             TEXT NOT NULL DEFAULT '[]',
    pinned_constraints        TEXT NOT NULL DEFAULT '[]',
    next_action               TEXT NOT NULL DEFAULT '',
    created_at                TEXT NOT NULL,
    restored_at               TEXT,
    restore_count             INTEGER NOT NULL DEFAULT 0,
    deleted_at                TEXT
);

CREATE INDEX IF NOT EXISTS continuity_checkpoints_session
    ON continuity_checkpoints (session_id, created_at DESC);
CREATE INDEX IF NOT EXISTS continuity_checkpoints_project
    ON continuity_checkpoints (project_id, created_at DESC);

-- Project-independent transferable knowledge.
--
-- There is deliberately NO `project_id` column. That absence is the design: a
-- pattern that cannot name a project cannot leak one (D61, FR-393). `origin_ref`
-- is a machine-salted digest, never a name, path or remote.
CREATE TABLE IF NOT EXISTS reusable_patterns (
    id                  TEXT PRIMARY KEY,
    title               TEXT NOT NULL,
    problem             TEXT NOT NULL,
    signals             TEXT NOT NULL,
    signal_digest       TEXT NOT NULL,
    applicability       TEXT NOT NULL DEFAULT '[]',
    root_cause          TEXT NOT NULL,
    root_cause_digest   TEXT NOT NULL,
    approach            TEXT NOT NULL,
    constraints         TEXT NOT NULL DEFAULT '[]',
    trust               TEXT NOT NULL CHECK (trust IN (
        'candidate', 'sanitized', 'validated', 'contested')),
    origin_ref          TEXT NOT NULL,
    origin_deleted      INTEGER NOT NULL DEFAULT 0 CHECK (origin_deleted IN (0, 1)),
    source_memory_id    TEXT,
    sanitization_report TEXT NOT NULL DEFAULT '{}',
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    deleted_at          TEXT,
    CHECK (json_array_length(signals) BETWEEN 2 AND 16)
);

-- Duplicate refusal enforced structurally as well as by the gate, so a race
-- cannot create one (gate check 10).
CREATE UNIQUE INDEX IF NOT EXISTS reusable_patterns_identity
    ON reusable_patterns (signal_digest, root_cause_digest) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS reusable_patterns_signal ON reusable_patterns (signal_digest);
CREATE INDEX IF NOT EXISTS reusable_patterns_trust ON reusable_patterns (trust);

-- Outcomes, independence and counterexamples.
--
-- The unique key is the anti-poisoning mechanism: ten sessions in one project
-- describing one incident produce ONE row, so the distinct-project count is 1
-- (FR-402, SC-314).
CREATE TABLE IF NOT EXISTS pattern_applications (
    id                TEXT PRIMARY KEY,
    pattern_id        TEXT NOT NULL REFERENCES reusable_patterns(id),
    project_id        TEXT NOT NULL REFERENCES projects(id),
    session_id        TEXT NOT NULL,
    signal_digest     TEXT NOT NULL,
    outcome           TEXT NOT NULL CHECK (outcome IN (
        'resolved', 'not_applicable', 'failed')),
    discovery         TEXT NOT NULL CHECK (discovery IN ('independent', 'cairn_suggested')),
    alternative_cause TEXT,
    evidence_id       TEXT,
    is_origin         INTEGER NOT NULL DEFAULT 0 CHECK (is_origin IN (0, 1)),
    applied_at        TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS pattern_applications_incident
    ON pattern_applications (pattern_id, project_id, signal_digest);
CREATE INDEX IF NOT EXISTS pattern_applications_pattern
    ON pattern_applications (pattern_id, outcome);

-- Stably identified acceptance criteria.
--
-- `id` is stable across every update, and `label` is derived from `ordinal` at
-- creation and NOT renumbered when another criterion is added or removed:
-- renumbering would silently change what "AC-2" means in a handoff, a
-- checkpoint or a session's memory (FR-481).
CREATE TABLE IF NOT EXISTS task_criteria (
    id           TEXT PRIMARY KEY,
    task_id      TEXT NOT NULL REFERENCES tasks(id),
    ordinal      INTEGER NOT NULL,
    label        TEXT NOT NULL,
    text         TEXT NOT NULL,
    state        TEXT NOT NULL DEFAULT 'pending' CHECK (state IN (
        'pending', 'satisfied', 'blocked', 'waived')),
    verification TEXT NOT NULL DEFAULT 'unverified' CHECK (verification IN (
        'unverified', 'verified', 'failed')),
    revision     INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    deleted_at   TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS task_criteria_ordinal
    ON task_criteria (task_id, ordinal) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS task_criteria_task ON task_criteria (task_id, ordinal);

-- Append-only, with one transition. Reopening creates a new blocker, so "who
-- said this was blocked and who said it was not" stays answerable (FR-485).
CREATE TABLE IF NOT EXISTS task_blockers (
    id                TEXT PRIMARY KEY,
    task_id           TEXT NOT NULL REFERENCES tasks(id),
    description       TEXT NOT NULL,
    state             TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'cleared')),
    opened_by_session TEXT NOT NULL,
    opened_at         TEXT NOT NULL,
    cleared_by_session TEXT,
    cleared_at        TEXT,
    deleted_at        TEXT
);

CREATE INDEX IF NOT EXISTS task_blockers_task ON task_blockers (task_id, state);

-- The append-only local revision history. This is what makes "no assertion is
-- lost even when a later one replaces it" true (FR-488), and what
-- `cairn task history` reads. Local: a peer receives the criteria and blockers
-- themselves, not this.
CREATE TABLE IF NOT EXISTS task_changes (
    id             TEXT PRIMARY KEY,
    task_id        TEXT NOT NULL REFERENCES tasks(id),
    local_revision INTEGER NOT NULL,
    kind           TEXT NOT NULL CHECK (kind IN (
        'goal_changed', 'title_changed', 'status_changed',
        'criterion_added', 'criterion_text', 'criterion_state',
        'criterion_verification', 'criterion_removed',
        'blocker_opened', 'blocker_cleared')),
    subject_id     TEXT,
    session_id     TEXT NOT NULL,
    prior_value    TEXT,
    new_value      TEXT,
    blind_write    INTEGER NOT NULL DEFAULT 0 CHECK (blind_write IN (0, 1)),
    changed_at     TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS task_changes_task ON task_changes (task_id, local_revision);

CREATE TABLE IF NOT EXISTS criterion_evidence (
    criterion_id        TEXT NOT NULL REFERENCES task_criteria(id),
    evidence_id         TEXT NOT NULL,
    attached_at         TEXT NOT NULL,
    attached_by_session TEXT NOT NULL,
    PRIMARY KEY (criterion_id, evidence_id)
);

-- ---------------------------------------------------------------------------
-- Step 4 — the remaining additive columns and the `memories` indexes
-- ---------------------------------------------------------------------------

-- A monotone counter for THIS store only. It is never transmitted and is absent
-- from the server schema, which is what makes it sound as a concurrency token:
-- an `expected_revision` can only have come from a read against this store
-- (FR-488, FR-490, D80).
ALTER TABLE tasks ADD COLUMN local_revision INTEGER NOT NULL DEFAULT 1;

-- The task state a session bound at. NULL for every existing session, and it
-- stays NULL: a session that bound before this feature existed genuinely does
-- not know, and synthesizing a snapshot would produce a false divergence report.
ALTER TABLE sessions ADD COLUMN task_snapshot_at_bind TEXT;

-- Recoverable capability refusal (D81, FR-418). Nullable and defaulting to NULL,
-- so every existing queued, in-flight, delivered and failed row is untouched and
-- stays exactly as claimable as it was. No existing row becomes `blocked` by
-- migration: that state is only ever reached by an actual capability refusal.
ALTER TABLE outbox ADD COLUMN blocked_reason        TEXT;
ALTER TABLE outbox ADD COLUMN blocked_at_capability TEXT;

ALTER TABLE sync_meta ADD COLUMN server_capability TEXT;

CREATE INDEX IF NOT EXISTS memories_topic
    ON memories (project_id, topic_key, state) WHERE topic_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS memories_subject
    ON memories (project_id, scope, scope_key, topic_key) WHERE topic_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS memories_verification
    ON memories (project_id, verification) WHERE verification <> 'unverified';
CREATE INDEX IF NOT EXISTS memories_pinned
    ON memories (project_id, scope, scope_key) WHERE pinned = 1;
CREATE INDEX IF NOT EXISTS memories_temporal
    ON memories (project_id, effective_from, superseded_at);
CREATE INDEX IF NOT EXISTS memories_content_norm
    ON memories (project_id, content_norm_digest) WHERE content_norm_digest IS NOT NULL;

-- ---------------------------------------------------------------------------
-- Step 5 — supersession relations from the existing links
-- ---------------------------------------------------------------------------

-- What makes existing supersessions visible to `derive_subject` and to sync.
-- `basis` is `explicit_user` because a Feature 001 supersession was always an
-- explicit act: `supersede_memory` has no automatic caller. `rationale` names
-- the migration, so the provenance is honest about where the record came from.
--
-- `decided_at` reuses `updated_at`, with the same caveat as backfill (b). It is
-- recorded but never read by the derivation, so its imprecision changes no
-- outcome.
INSERT OR IGNORE INTO memory_relations
    (from_memory_id, to_memory_id, kind, project_id,
     decided_by_session, decided_at, basis, rationale)
SELECT s.id, m.id, 'supersedes', m.project_id,
       s.origin_session_id, m.updated_at, 'explicit_user',
       'migrated from Feature 001 superseded_by_id'
  FROM memories m
  JOIN memories s ON s.id = m.superseded_by_id
 WHERE m.superseded_by_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- Step 6 — criteria from the existing JSON arrays
--
-- Not expressible here. A criterion's `id` is a UUIDv7 by the convention every
-- other identifier in this schema follows, and SQLite can only produce a random
-- value shaped like one — which would claim a time ordering it does not have.
-- So the conversion runs as a Rust step in `migrate.rs`, inside **this same
-- transaction**, immediately after these statements. An interruption still
-- rolls the whole migration back (migration.md §Proof, assertion 12).
-- ---------------------------------------------------------------------------
