-- Add server-owned handoff recovery commands to the typed command spool.
-- SQLite cannot widen a CHECK in place, so preserve every row while replacing
-- only this closed vocabulary.
CREATE TABLE command_spool_new (
  command_id  TEXT PRIMARY KEY,
  scope_kind  TEXT NOT NULL CHECK (scope_kind IN ('session','store')),
  scope_key   TEXT NOT NULL,
  session_id  TEXT REFERENCES sessions(id),
  project_id  TEXT REFERENCES projects(id),
  account_id  TEXT NOT NULL,
  command_seq INTEGER NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN (
              'remember','supersede','reinforce','relate','pin','forget',
              'personal_create','personal_forget','team_propose',
              'pattern_promote','pattern_forget',
              'verification_run','verification_attestation',
              'handoff_generate','handoff_annotate')),
  payload     TEXT NOT NULL,
  state       TEXT NOT NULL CHECK (state IN
              ('pending','in_flight','delivered','failed','refused')),
  attempts    INTEGER NOT NULL DEFAULT 0,
  claimed_at      TEXT,
  next_attempt_at TEXT,
  last_error_kind TEXT,
  created_at  TEXT NOT NULL,
  server_instance_id TEXT,
  UNIQUE (scope_kind, scope_key, command_seq)
);

INSERT INTO command_spool_new
SELECT command_id, scope_kind, scope_key, session_id, project_id, account_id,
       command_seq, kind, payload, state, attempts, claimed_at,
       next_attempt_at, last_error_kind, created_at, server_instance_id
  FROM command_spool;

DROP TABLE command_spool;
ALTER TABLE command_spool_new RENAME TO command_spool;
CREATE INDEX command_spool_claim
    ON command_spool (state, account_id, server_instance_id, next_attempt_at);
CREATE INDEX command_spool_order ON command_spool (scope_kind, scope_key, command_seq);
