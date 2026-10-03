-- Atomic logical-import receipt. Replaying one import id returns its recorded
-- result; changing the bundle under that id is refused.
CREATE TABLE logical_imports (
  import_id   UUID PRIMARY KEY,
  imported_by UUID NOT NULL REFERENCES users(id),
  bundle      JSONB NOT NULL,
  report      JSONB,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  finished_at TIMESTAMPTZ
);
