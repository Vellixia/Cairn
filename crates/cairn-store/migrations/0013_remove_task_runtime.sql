-- Legacy Task data is never destroyed by an automatic runtime migration.
-- Setup must first create and verify a versioned `removed_feature` bundle.
-- Until that explicit export has a conservation report, historical rows remain
-- intact and unreachable by the V1 runtime.
CREATE TABLE IF NOT EXISTS removed_feature_manifest (
    feature TEXT PRIMARY KEY,
    bundle_version INTEGER NOT NULL,
    disposition TEXT NOT NULL,
    created_at TEXT NOT NULL
);

INSERT OR IGNORE INTO removed_feature_manifest
    (feature, bundle_version, disposition, created_at)
VALUES ('tasks', 1, 'retained_pending_export', CURRENT_TIMESTAMP);
