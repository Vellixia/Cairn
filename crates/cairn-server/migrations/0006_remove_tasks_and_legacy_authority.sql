-- Tasks are removed from live runtime. Keep exact source rows in one
-- versioned, retrieval-excluded archive before destructive DDL.
CREATE TABLE IF NOT EXISTS removed_feature_archives (
  feature       TEXT PRIMARY KEY,
  format_version INTEGER NOT NULL,
  source_counts JSONB NOT NULL,
  payload       JSONB NOT NULL,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

DO $$
DECLARE
  task_count BIGINT;
  criteria_count BIGINT;
  blockers_count BIGINT;
  session_count BIGINT;
  memory_count BIGINT;
  relation_count BIGINT;
  legacy_sync_count BIGINT;
  task_sync_count BIGINT;
  archived JSONB;
BEGIN
  SELECT count(*) INTO task_count FROM tasks;
  SELECT count(*) INTO criteria_count FROM task_criteria;
  SELECT count(*) INTO blockers_count FROM task_blockers;
  SELECT count(*) INTO session_count FROM sessions WHERE task_id IS NOT NULL;
  SELECT count(*) INTO memory_count FROM memories WHERE scope = 'task';
  SELECT count(*) INTO relation_count
    FROM memory_relations r
    WHERE EXISTS (SELECT 1 FROM memories m WHERE m.id = r.from_memory_id AND m.scope = 'task')
       OR EXISTS (SELECT 1 FROM memories m WHERE m.id = r.to_memory_id AND m.scope = 'task');
  SELECT count(*) INTO legacy_sync_count FROM sync_state;
  SELECT count(*) INTO task_sync_count FROM sync_state WHERE entity_type = 'task';

  INSERT INTO removed_feature_archives (feature, format_version, source_counts, payload)
  VALUES (
    'tasks', 1,
    jsonb_build_object(
      'tasks', task_count, 'task_criteria', criteria_count,
      'task_blockers', blockers_count, 'sessions_with_task_id', session_count,
      'task_memories', memory_count, 'relations_touching_task_memories', relation_count,
      'legacy_sync_state', legacy_sync_count, 'sync_state_tasks', task_sync_count
    ),
    jsonb_build_object(
      'tasks', (SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.id), '[]'::jsonb) FROM tasks t),
      'task_criteria', (SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.id), '[]'::jsonb) FROM task_criteria c),
      'task_blockers', (SELECT COALESCE(jsonb_agg(to_jsonb(b) ORDER BY b.id), '[]'::jsonb) FROM task_blockers b),
      'sessions_with_task_id', (SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.id), '[]'::jsonb) FROM sessions s WHERE s.task_id IS NOT NULL),
      'task_memories', (SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY m.id), '[]'::jsonb) FROM memories m WHERE m.scope = 'task'),
      'relations_touching_task_memories', (SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY r.from_memory_id, r.to_memory_id, r.kind), '[]'::jsonb) FROM memory_relations r WHERE EXISTS (SELECT 1 FROM memories m WHERE m.id = r.from_memory_id AND m.scope = 'task') OR EXISTS (SELECT 1 FROM memories m WHERE m.id = r.to_memory_id AND m.scope = 'task')),
      'legacy_sync_state', (SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.idempotency_key), '[]'::jsonb) FROM sync_state s),
      'sync_state_tasks', (SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.idempotency_key), '[]'::jsonb) FROM sync_state s WHERE s.entity_type = 'task')
    )
  ) ON CONFLICT (feature) DO NOTHING;

  SELECT payload INTO archived FROM removed_feature_archives WHERE feature = 'tasks';
  IF jsonb_array_length(archived->'tasks') <> task_count
     OR jsonb_array_length(archived->'task_criteria') <> criteria_count
     OR jsonb_array_length(archived->'task_blockers') <> blockers_count
     OR jsonb_array_length(archived->'sessions_with_task_id') <> session_count
     OR jsonb_array_length(archived->'task_memories') <> memory_count
     OR jsonb_array_length(archived->'relations_touching_task_memories') <> relation_count
     OR jsonb_array_length(archived->'legacy_sync_state') <> legacy_sync_count
     OR jsonb_array_length(archived->'sync_state_tasks') <> task_sync_count THEN
    RAISE EXCEPTION 'task archive conservation failed';
  END IF;
END $$;

DELETE FROM memory_relations r
WHERE EXISTS (SELECT 1 FROM memories m WHERE m.id = r.from_memory_id AND m.scope = 'task')
   OR EXISTS (SELECT 1 FROM memories m WHERE m.id = r.to_memory_id AND m.scope = 'task');
DELETE FROM memories WHERE scope = 'task';
DROP TABLE IF EXISTS task_criteria;
DROP TABLE IF EXISTS task_blockers;
DROP TABLE IF EXISTS tasks;
ALTER TABLE sessions DROP COLUMN IF EXISTS task_id;
ALTER TABLE memories DROP CONSTRAINT IF EXISTS memories_scope_check;
ALTER TABLE memories ADD CONSTRAINT memories_scope_check CHECK (scope IN ('project', 'branch', 'session'));
DROP TABLE IF EXISTS sync_state;
DROP TABLE IF EXISTS server_authority;
DROP TABLE IF EXISTS client_migrations;
