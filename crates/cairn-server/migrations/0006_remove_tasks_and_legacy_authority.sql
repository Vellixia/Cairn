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
  counts JSONB;
  source_payload JSONB;
  archived_format INTEGER;
  archived_counts JSONB;
  archived_payload JSONB;
BEGIN
  SELECT jsonb_build_object(
    'tasks', (SELECT count(*) FROM tasks),
    'task_criteria', (SELECT count(*) FROM task_criteria),
    'task_blockers', (SELECT count(*) FROM task_blockers),
    'sessions_with_task_id', (SELECT count(*) FROM sessions WHERE task_id IS NOT NULL),
    'task_memories', (SELECT count(*) FROM memories WHERE scope = 'task'),
    'session_memories', (SELECT count(*) FROM memories WHERE origin_session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'relations_touching_task_memories', (SELECT count(*) FROM memory_relations r WHERE EXISTS (SELECT 1 FROM memories m WHERE m.id = r.from_memory_id AND m.scope = 'task') OR EXISTS (SELECT 1 FROM memories m WHERE m.id = r.to_memory_id AND m.scope = 'task')),
    'relations_touching_session_memories', (SELECT count(*) FROM memory_relations r WHERE EXISTS (SELECT 1 FROM memories m WHERE m.id = r.from_memory_id AND m.origin_session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)) OR EXISTS (SELECT 1 FROM memories m WHERE m.id = r.to_memory_id AND m.origin_session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL))),
    'handoffs', (SELECT count(*) FROM handoffs h WHERE h.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'safe_events', (SELECT count(*) FROM safe_events e WHERE e.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'consolidation_session', (SELECT count(*) FROM consolidation_session c WHERE c.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'consolidation_work', (SELECT count(*) FROM consolidation_work c WHERE c.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'consolidation_runs', (SELECT count(*) FROM consolidation_runs c WHERE c.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'retrieval_traces', (SELECT count(*) FROM retrieval_traces t WHERE t.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'retrieval_trace_items', (SELECT count(*) FROM retrieval_trace_items i WHERE i.trace_id IN (SELECT trace_id FROM retrieval_traces WHERE session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL))),
    'delivered_context', (SELECT count(*) FROM delivered_context d WHERE d.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'knowledge_candidates', (SELECT count(*) FROM knowledge_candidates k WHERE EXISTS (SELECT 1 FROM candidate_source_events c JOIN safe_events e ON e.event_id = c.event_id WHERE c.candidate_id = k.candidate_id AND e.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL))),
    'candidate_source_events', (SELECT count(*) FROM candidate_source_events c WHERE c.candidate_id IN (SELECT k.candidate_id FROM knowledge_candidates k WHERE EXISTS (SELECT 1 FROM candidate_source_events x JOIN safe_events e ON e.event_id = x.event_id WHERE x.candidate_id = k.candidate_id AND e.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)))),
    'legacy_sync_state', (SELECT count(*) FROM sync_state),
    'sync_state_tasks', (SELECT count(*) FROM sync_state WHERE entity_type = 'task')
  ) INTO counts;
  SELECT jsonb_build_object(
    'tasks', (SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.id), '[]'::jsonb) FROM tasks t),
    'task_criteria', (SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.id), '[]'::jsonb) FROM task_criteria c),
    'task_blockers', (SELECT COALESCE(jsonb_agg(to_jsonb(b) ORDER BY b.id), '[]'::jsonb) FROM task_blockers b),
    'sessions_with_task_id', (SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.id), '[]'::jsonb) FROM sessions s WHERE s.task_id IS NOT NULL),
    'task_memories', (SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY m.id), '[]'::jsonb) FROM memories m WHERE m.scope = 'task'),
    'session_memories', (SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY m.id), '[]'::jsonb) FROM memories m WHERE m.origin_session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'relations_touching_task_memories', (SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY r.from_memory_id, r.to_memory_id, r.kind), '[]'::jsonb) FROM memory_relations r WHERE EXISTS (SELECT 1 FROM memories m WHERE m.id = r.from_memory_id AND m.scope = 'task') OR EXISTS (SELECT 1 FROM memories m WHERE m.id = r.to_memory_id AND m.scope = 'task')),
    'relations_touching_session_memories', (SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY r.from_memory_id, r.to_memory_id, r.kind), '[]'::jsonb) FROM memory_relations r WHERE EXISTS (SELECT 1 FROM memories m WHERE m.id = r.from_memory_id AND m.origin_session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)) OR EXISTS (SELECT 1 FROM memories m WHERE m.id = r.to_memory_id AND m.origin_session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL))),
    'handoffs', (SELECT COALESCE(jsonb_agg(to_jsonb(h) ORDER BY h.id), '[]'::jsonb) FROM handoffs h WHERE h.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'safe_events', (SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY e.event_id), '[]'::jsonb) FROM safe_events e WHERE e.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'consolidation_session', (SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.project_id, c.session_id), '[]'::jsonb) FROM consolidation_session c WHERE c.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'consolidation_work', (SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.event_id), '[]'::jsonb) FROM consolidation_work c WHERE c.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'consolidation_runs', (SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.run_id), '[]'::jsonb) FROM consolidation_runs c WHERE c.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'retrieval_traces', (SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.trace_id), '[]'::jsonb) FROM retrieval_traces t WHERE t.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'retrieval_trace_items', (SELECT COALESCE(jsonb_agg(to_jsonb(i) ORDER BY i.trace_id, i.reference_key), '[]'::jsonb) FROM retrieval_trace_items i WHERE i.trace_id IN (SELECT trace_id FROM retrieval_traces WHERE session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL))),
    'delivered_context', (SELECT COALESCE(jsonb_agg(to_jsonb(d) ORDER BY d.session_id, d.reference_key), '[]'::jsonb) FROM delivered_context d WHERE d.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)),
    'knowledge_candidates', (SELECT COALESCE(jsonb_agg(to_jsonb(k) ORDER BY k.candidate_id), '[]'::jsonb) FROM knowledge_candidates k WHERE EXISTS (SELECT 1 FROM candidate_source_events c JOIN safe_events e ON e.event_id = c.event_id WHERE c.candidate_id = k.candidate_id AND e.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL))),
    'candidate_source_events', (SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.candidate_id, c.event_id), '[]'::jsonb) FROM candidate_source_events c WHERE c.candidate_id IN (SELECT k.candidate_id FROM knowledge_candidates k WHERE EXISTS (SELECT 1 FROM candidate_source_events x JOIN safe_events e ON e.event_id = x.event_id WHERE x.candidate_id = k.candidate_id AND e.session_id IN (SELECT id FROM sessions WHERE task_id IS NOT NULL)))),
    'legacy_sync_state', (SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.idempotency_key), '[]'::jsonb) FROM sync_state s),
    'sync_state_tasks', (SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.idempotency_key), '[]'::jsonb) FROM sync_state s WHERE s.entity_type = 'task')
  ) INTO source_payload;
  INSERT INTO removed_feature_archives (feature, format_version, source_counts, payload)
  VALUES ('tasks', 1, counts, source_payload) ON CONFLICT (feature) DO NOTHING;
  SELECT format_version, source_counts, payload
    INTO archived_format, archived_counts, archived_payload
    FROM removed_feature_archives WHERE feature = 'tasks';
  IF archived_format <> 1 OR archived_counts <> counts OR archived_payload <> source_payload THEN
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
