# Feature 001 SQLite schema baseline

- Producer commit: `4a06c4125715bb4b78b54e49c81eccd82100a7b7`
- Runtime fixture: `fixtures/databases/feature-001-v1.sqlite3`
- SQLite: `3.51.0`
- `0001_init.sql` SHA-256: `9369accb9ed7469ae7d570bd39b3afd5f75dc87c95e106753b56e3f590067008`
- Frozen and working-tree 0001 hashes: identical
- SQLx migration version: 1 (`init`, successful)
- SQLx checksum hex: `3F34B50A95A77B005234F52EE98AF1CAA05A5D05C762207675A4995DD09C2D70271B3B71EBF7C7D7B811EDCD6B067339`
- `PRAGMA quick_check`: `ok`

## Runtime schema inventory

| Table | Ordered columns |
|---|---|
| `repositories` | `id`, `repo_uuid`, `canonical_path`, `default_remote_name`, `default_remote_url`, `copied_from_repository_id`, `registered_at` |
| `worktrees` | `id`, `repository_id`, `worktree_uuid`, `path`, `is_main`, `registered_at` |
| `snapshots` | `id`, `worktree_id`, `branch`, `head_commit`, `staged_fp`, `unstaged_fp`, `untracked_fp`, `snapshot_fp`, `fp_schema_version`, `created_at` |
| `sessions` | `id`, `repository_id`, `worktree_id`, `local_user`, `agent_type`, `agent_instance_id`, `agent_pid`, `resume_token_hash`, `lease_expires_at`, `state`, `start_snapshot_id`, `current_snapshot_id`, `started_at`, `ended_at`, `last_heartbeat_at`, `recovering_since` |
| `events` | `seq`, `id`, `idempotency_key`, `event_type`, `repository_id`, `worktree_id`, `session_id`, `snapshot_id`, `payload`, `recorded_at` |
| `meta` | `key`, `value` |
| `_sqlx_migrations` | `version`, `description`, `installed_on`, `success`, `checksum`, `execution_time` |

Indexes: `events_by_repo_seq`, `events_by_session_seq`,
`events_by_worktree_seq`, `sessions_by_repo_state`, and
`sessions_one_live_per_instance`.

Triggers: `events_no_delete`, `events_no_update`, `snapshots_no_delete`, and
`snapshots_no_update`.

## Capture commands

```sh
shasum -a 256 crates/cairn-storage-local/migrations/0001_init.sql
sqlite3 fixtures/databases/feature-001-v1.sqlite3 'PRAGMA quick_check;'
sqlite3 -json fixtures/databases/feature-001-v1.sqlite3 \
  "SELECT type,name,tbl_name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name;"
sqlite3 -json fixtures/databases/feature-001-v1.sqlite3 \
  "SELECT m.name,p.cid,p.name,p.type,p.[notnull],p.dflt_value,p.pk FROM sqlite_master m JOIN pragma_table_info(m.name) p WHERE m.type='table' ORDER BY m.name,p.cid;"
```

The capture is observational. Migration `0001_init.sql` remains byte-for-byte
unchanged.
