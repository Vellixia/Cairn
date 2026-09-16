-- Forward repair for databases which recorded version 13 before the removed
-- feature artifact became part of the manifest. Never edit 0013: applied
-- migration history is immutable.
ALTER TABLE removed_feature_manifest ADD COLUMN artifact_path TEXT;
ALTER TABLE removed_feature_manifest ADD COLUMN artifact_sha256 TEXT;

-- Old exported-retained records lack a verified artifact identity. Require a
-- new export rather than allowing cleanup against an unknown file.
UPDATE removed_feature_manifest
   SET disposition = 'retained_pending_export'
 WHERE feature = 'tasks'
   AND disposition NOT IN ('retained_pending_export', 'exported_pending_cleanup', 'exported_cleaned');
