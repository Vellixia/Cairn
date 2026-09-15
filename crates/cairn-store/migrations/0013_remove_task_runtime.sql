-- Task records are retained only in offline migration bundles. Runtime edge
-- state has no task authority, task references, or task delivery path.
DROP INDEX IF EXISTS sessions_task_recent;
DROP TABLE IF EXISTS criterion_evidence;
DROP TABLE IF EXISTS task_changes;
DROP TABLE IF EXISTS task_blockers;
DROP TABLE IF EXISTS task_criteria;
DROP TABLE IF EXISTS criterion_evidence;
ALTER TABLE sessions DROP COLUMN task_snapshot_at_bind;
ALTER TABLE sessions DROP COLUMN task_id;
DROP TABLE IF EXISTS tasks;

ALTER TABLE continuity_checkpoints DROP COLUMN assumed_task_id;
ALTER TABLE continuity_checkpoints DROP COLUMN assumed_task_state_digest;
ALTER TABLE continuity_checkpoints DROP COLUMN criteria_snapshot;
ALTER TABLE continuity_checkpoints DROP COLUMN open_blockers;

DELETE FROM memories WHERE scope = 'task';
