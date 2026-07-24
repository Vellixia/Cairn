-- Feature 002: project/task binding foundation.
-- Applies after exact Feature 001 migration 0001_init.sql.
-- SQLx executes this migration transactionally.

ALTER TABLE sessions
    ADD COLUMN binding_mode TEXT NOT NULL DEFAULT 'local_unbound'
    CHECK (binding_mode IN ('local_unbound', 'project_bound'));

CREATE INDEX sessions_by_binding_state
    ON sessions (binding_mode, state);

-- Legacy events remain NULL. Every event inserted after this migration must
-- carry a real explicit aggregate.
ALTER TABLE events
    ADD COLUMN aggregate_type TEXT
    CHECK (
        aggregate_type IS NULL
        OR aggregate_type IN ('repository', 'worktree', 'session', 'project', 'task')
    );
ALTER TABLE events ADD COLUMN aggregate_id TEXT;
ALTER TABLE events ADD COLUMN aggregate_seq INTEGER
    CHECK (aggregate_seq IS NULL OR aggregate_seq > 0);

CREATE UNIQUE INDEX events_one_aggregate_seq
    ON events (aggregate_type, aggregate_id, aggregate_seq)
    WHERE aggregate_type IS NOT NULL;
CREATE INDEX events_by_aggregate_seq
    ON events (aggregate_type, aggregate_id, aggregate_seq)
    WHERE aggregate_type IS NOT NULL;

CREATE TABLE event_aggregate_heads (
    aggregate_type TEXT NOT NULL
        CHECK (aggregate_type IN ('repository', 'worktree', 'session', 'project', 'task')),
    aggregate_id TEXT NOT NULL,
    last_seq INTEGER NOT NULL CHECK (last_seq >= 0),
    PRIMARY KEY (aggregate_type, aggregate_id)
);

CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    description TEXT,
    status TEXT NOT NULL CHECK (status IN ('active', 'archived')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX projects_by_status_id ON projects (status, id);
CREATE INDEX projects_by_name_id ON projects (name, id);

CREATE TABLE project_repository_associations (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    repository_id TEXT NOT NULL REFERENCES repositories(id),
    associated_at TEXT NOT NULL,
    event_seq INTEGER NOT NULL UNIQUE REFERENCES events(seq),
    UNIQUE (repository_id),
    UNIQUE (project_id, repository_id)
);
CREATE INDEX project_repositories_by_project
    ON project_repository_associations (project_id, repository_id);

CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    latest_revision_number INTEGER NOT NULL CHECK (latest_revision_number > 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX tasks_by_project_id ON tasks (project_id, id);
CREATE INDEX tasks_by_project_title_id ON tasks (project_id, title, id);

CREATE TABLE task_revisions (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    revision_number INTEGER NOT NULL CHECK (revision_number > 0),
    parent_revision_id TEXT REFERENCES task_revisions(id),
    goal_contract_json TEXT NOT NULL,
    goal_contract_schema_version INTEGER NOT NULL
        CHECK (goal_contract_schema_version = 1),
    goal_contract_fingerprint TEXT NOT NULL
        CHECK (
            length(goal_contract_fingerprint) = 64
            AND goal_contract_fingerprint NOT GLOB '*[^0-9a-f]*'
        ),
    created_at TEXT NOT NULL,
    UNIQUE (task_id, revision_number)
);
CREATE INDEX task_revisions_by_task_number
    ON task_revisions (task_id, revision_number DESC);
CREATE INDEX task_revisions_by_parent ON task_revisions (parent_revision_id);

CREATE TABLE operation_idempotency (
    idempotency_key TEXT PRIMARY KEY,
    method TEXT NOT NULL
        CHECK (method IN (
            'project.create',
            'project.update',
            'project.repository_associate',
            'task.create',
            'task.revise',
            'session.bind'
        )),
    request_fingerprint TEXT NOT NULL
        CHECK (
            length(request_fingerprint) = 64
            AND request_fingerprint NOT GLOB '*[^0-9a-f]*'
        ),
    result_kind TEXT NOT NULL
        CHECK (result_kind IN (
            'event',
            'project_repository_association',
            'session_binding'
        )),
    result_locator TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX operation_idempotency_by_result
    ON operation_idempotency (result_kind, result_locator);
CREATE INDEX operation_idempotency_by_created_at
    ON operation_idempotency (created_at);

CREATE TABLE session_bindings (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    task_revision_id TEXT NOT NULL REFERENCES task_revisions(id),
    bound_at TEXT NOT NULL,
    binding_event_seq INTEGER NOT NULL UNIQUE REFERENCES events(seq)
);
CREATE INDEX session_bindings_by_project
    ON session_bindings (project_id, session_id);
CREATE INDEX session_bindings_by_revision
    ON session_bindings (task_revision_id, session_id);

CREATE TRIGGER events_require_explicit_aggregate
BEFORE INSERT ON events
WHEN NEW.aggregate_type IS NULL
  OR NEW.aggregate_id IS NULL
  OR NEW.aggregate_seq IS NULL
BEGIN
    SELECT RAISE(ABORT, 'new events require explicit aggregate scope');
END;

CREATE TRIGGER events_reject_partial_aggregate
BEFORE INSERT ON events
WHEN (NEW.aggregate_type IS NULL) <> (NEW.aggregate_id IS NULL)
   OR (NEW.aggregate_type IS NULL) <> (NEW.aggregate_seq IS NULL)
   OR NEW.aggregate_seq <= 0
BEGIN
    SELECT RAISE(ABORT, 'invalid event aggregate scope');
END;

CREATE TRIGGER project_associations_require_active_project
BEFORE INSERT ON project_repository_associations
WHEN NOT EXISTS (
    SELECT 1 FROM projects
    WHERE id = NEW.project_id AND status = 'active'
)
BEGIN
    SELECT RAISE(ABORT, 'project must be active for repository association');
END;

CREATE TRIGGER tasks_require_active_project
BEFORE INSERT ON tasks
WHEN NOT EXISTS (
    SELECT 1 FROM projects
    WHERE id = NEW.project_id AND status = 'active'
)
BEGIN
    SELECT RAISE(ABORT, 'project must be active for task creation');
END;

CREATE TRIGGER task_revisions_require_active_project_and_current_number
BEFORE INSERT ON task_revisions
WHEN NOT EXISTS (
    SELECT 1
    FROM tasks
    JOIN projects ON projects.id = tasks.project_id
    WHERE tasks.id = NEW.task_id
      AND projects.status = 'active'
      AND tasks.latest_revision_number = NEW.revision_number
)
BEGIN
    SELECT RAISE(ABORT, 'task revision requires active project and current number');
END;

CREATE TRIGGER session_bindings_validate_relationships
BEFORE INSERT ON session_bindings
WHEN NOT EXISTS (
    SELECT 1
    FROM sessions
    JOIN worktrees ON worktrees.id = sessions.worktree_id
    JOIN project_repository_associations
      ON project_repository_associations.repository_id = worktrees.repository_id
     AND project_repository_associations.project_id = NEW.project_id
    JOIN projects ON projects.id = NEW.project_id
    JOIN task_revisions ON task_revisions.id = NEW.task_revision_id
    JOIN tasks ON tasks.id = task_revisions.task_id
    WHERE sessions.id = NEW.session_id
      AND sessions.repository_id = worktrees.repository_id
      AND sessions.binding_mode = 'local_unbound'
      AND projects.status = 'active'
      AND tasks.project_id = NEW.project_id
)
BEGIN
    SELECT RAISE(ABORT, 'invalid session binding relationships');
END;

CREATE TRIGGER task_revisions_validate_parent
BEFORE INSERT ON task_revisions
WHEN (NEW.revision_number = 1 AND NEW.parent_revision_id IS NOT NULL)
  OR (
        NEW.revision_number > 1
        AND NEW.parent_revision_id IS NOT NULL
        AND NOT EXISTS (
            SELECT 1
            FROM task_revisions parent
            WHERE parent.id = NEW.parent_revision_id
              AND parent.task_id = NEW.task_id
              AND parent.revision_number < NEW.revision_number
        )
     )
BEGIN
    SELECT RAISE(ABORT, 'invalid task revision parent');
END;

CREATE TRIGGER sessions_binding_mode_monotonic
BEFORE UPDATE OF binding_mode ON sessions
WHEN NOT (
    NEW.binding_mode = OLD.binding_mode
    OR (OLD.binding_mode = 'local_unbound' AND NEW.binding_mode = 'project_bound')
)
BEGIN
    SELECT RAISE(ABORT, 'session binding mode is monotonic');
END;

CREATE TRIGGER sessions_project_bound_requires_projection
BEFORE UPDATE OF binding_mode ON sessions
WHEN NEW.binding_mode = 'project_bound'
 AND OLD.binding_mode <> 'project_bound'
 AND NOT EXISTS (
     SELECT 1 FROM session_bindings WHERE session_id = NEW.id
 )
BEGIN
    SELECT RAISE(ABORT, 'project-bound session requires binding projection');
END;

CREATE TRIGGER projects_immutable_identity
BEFORE UPDATE ON projects
WHEN NEW.id <> OLD.id OR NEW.created_at <> OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'project identity and creation time are immutable');
END;

CREATE TRIGGER tasks_immutable_identity_and_ownership
BEFORE UPDATE ON tasks
WHEN NEW.id <> OLD.id
  OR NEW.project_id <> OLD.project_id
  OR NEW.title <> OLD.title
  OR NEW.created_at <> OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'task identity, ownership, title, and creation time are immutable');
END;

CREATE TRIGGER tasks_revision_counter_is_sequential
BEFORE UPDATE OF latest_revision_number ON tasks
WHEN NEW.latest_revision_number <> OLD.latest_revision_number + 1
BEGIN
    SELECT RAISE(ABORT, 'task revision counter must advance by one');
END;

CREATE TRIGGER project_associations_no_update
BEFORE UPDATE ON project_repository_associations
BEGIN
    SELECT RAISE(ABORT, 'project repository associations are immutable');
END;
CREATE TRIGGER project_associations_no_delete
BEFORE DELETE ON project_repository_associations
BEGIN
    SELECT RAISE(ABORT, 'project repository associations cannot be deleted');
END;
CREATE TRIGGER task_revisions_no_update
BEFORE UPDATE ON task_revisions
BEGIN
    SELECT RAISE(ABORT, 'task revisions are immutable');
END;
CREATE TRIGGER task_revisions_no_delete
BEFORE DELETE ON task_revisions
BEGIN
    SELECT RAISE(ABORT, 'task revisions cannot be deleted');
END;
CREATE TRIGGER session_bindings_no_update
BEFORE UPDATE ON session_bindings
BEGIN
    SELECT RAISE(ABORT, 'session bindings are immutable');
END;
CREATE TRIGGER session_bindings_no_delete
BEFORE DELETE ON session_bindings
BEGIN
    SELECT RAISE(ABORT, 'session bindings cannot be deleted');
END;
CREATE TRIGGER operation_idempotency_no_update
BEFORE UPDATE ON operation_idempotency
BEGIN
    SELECT RAISE(ABORT, 'operation idempotency records are immutable');
END;
CREATE TRIGGER operation_idempotency_no_delete
BEFORE DELETE ON operation_idempotency
BEGIN
    SELECT RAISE(ABORT, 'operation idempotency records cannot be deleted');
END;
CREATE TRIGGER projects_no_delete
BEFORE DELETE ON projects
BEGIN
    SELECT RAISE(ABORT, 'projects cannot be deleted');
END;
CREATE TRIGGER tasks_no_delete
BEFORE DELETE ON tasks
BEGIN
    SELECT RAISE(ABORT, 'tasks cannot be deleted');
END;

INSERT INTO meta (key, value) VALUES ('local_schema_version', '2');
