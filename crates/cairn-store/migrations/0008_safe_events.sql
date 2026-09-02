-- Feature 005 — server-authoritative autonomous memory, local schema v8
-- (data-model.md §5).
--
-- Additive only. No existing table changes meaning, no existing row is
-- rewritten, and nothing here is dropped: v8 demotes the local store from
-- canonical holder to spool, cache and machine state, and that demotion is a
-- statement about which side *decides*, not a licence to delete what the
-- device still has.
--
-- Statement order:
--   1. Durable sequences — `session_event_seq`, `command_seq` — first, because
--      the spools' identities are derived from them.
--   2. The two spools and their claim indexes.
--   3. Capture disposition counts.
--   4. Migration and authority state: `authority_mode`, `migration_state`,
--      `retained_local`, `legacy_pattern_claims`.
--   5. `authority_mode`'s single row is seeded as a Rust step in `migrate.rs`'s
--      `finish(8, tx)` hook, inside this same transaction — the timestamp has
--      to be an RFC 3339 string to match every other timestamp in this schema,
--      and SQLite's `datetime('now')` produces a different format.

-- ---------------------------------------------------------------------------
-- Step 1 — durable ordinals
-- ---------------------------------------------------------------------------

-- The per-session event ordinal, and the reason it lives in a table of its own
-- rather than being computed as `MAX(session_seq) + 1` over the spool.
--
-- Hooks are separate, short-lived processes; they cannot share a counter, so
-- the daemon assigns it. But the daemon cannot derive it from the spool
-- either: the spool *drains*. A counter recovered from `MAX(session_seq)`
-- restarts at 1 the moment delivery empties the session's rows, and the next
-- event then re-derives an identity a delivered event already used — which the
-- server, being idempotent on event id, would accept as a duplicate and
-- silently discard. The counter is therefore durable and independent of
-- whether anything is still queued (data-model.md §1.4).
CREATE TABLE session_event_seq (
  session_id TEXT PRIMARY KEY REFERENCES sessions(id),
  next_seq   INTEGER NOT NULL DEFAULT 1
);

-- The same durability for commands, with one difference: a command is not
-- always inside a session. An explicit `cairn remember` from the CLI has no
-- session to be the scope of, so the store's own `writer_id` is, and the
-- counter is keyed by the scope kind as well as its key.
CREATE TABLE command_seq (
  scope_kind TEXT NOT NULL CHECK (scope_kind IN ('session','store')),
  scope_key  TEXT NOT NULL,
  next_seq   INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY (scope_kind, scope_key)
);

-- ---------------------------------------------------------------------------
-- Step 2 — the event spool
-- ---------------------------------------------------------------------------

-- Approved events awaiting the server.
--
-- `account_id` is NOT NULL and is matched exactly on claim. A claim predicate
-- that treats NULL as "any account" is how one identity comes to deliver
-- another's events under its own credential, so there is no NULL to open the
-- door with.
--
-- `boundary_class` is the capacity policy's discriminator: 1 marks a row the
-- overflow rule may never drop (session open, close, compaction), because
-- every other event is routed by the session structure those rows establish.
-- Shedding one would not lose an event, it would corrupt the interpretation of
-- everything still queued (§5, FR-785).
--
-- `payload` holds the approved `SafeCanonicalEvent` and nothing else. There is
-- deliberately no column here for a raw transcript or a vendor's original JSON:
-- raw material does not leave the process that saw it, so it has nowhere in
-- this schema to be stored on the way out (Principle V).
CREATE TABLE event_spool (
  event_id        TEXT PRIMARY KEY,
  session_id      TEXT NOT NULL REFERENCES sessions(id),
  project_id      TEXT NOT NULL REFERENCES projects(id),
  account_id      TEXT NOT NULL,
  session_seq     INTEGER NOT NULL,
  kind            TEXT NOT NULL,
  payload         TEXT NOT NULL,
  payload_bytes   INTEGER NOT NULL,
  boundary_class  INTEGER NOT NULL CHECK (boundary_class IN (0, 1)),
  state           TEXT NOT NULL CHECK (state IN
                    ('pending','in_flight','delivered','failed','refused')),
  attempts        INTEGER NOT NULL DEFAULT 0,
  claimed_at      TEXT,
  next_attempt_at TEXT,
  last_error_kind TEXT,
  created_at      TEXT NOT NULL,
  UNIQUE (session_id, session_seq)
);

-- The claim predicate's index, in the order the predicate reads it: only
-- `pending` rows, only this account's, only those whose backoff has elapsed.
CREATE INDEX event_spool_claim ON event_spool (state, account_id, next_attempt_at);

-- Overflow shedding walks the capture-class rows oldest first, and saturation
-- is the question "is anything capture-class left". Both are this index.
CREATE INDEX event_spool_capacity ON event_spool (boundary_class, created_at);

-- ---------------------------------------------------------------------------
-- Step 3 — the command spool
-- ---------------------------------------------------------------------------

-- Knowledge commands awaiting the server, once it is authoritative.
--
-- `payload` carries **intent only**. Derived state — a state machine's next
-- state, a supersession decision, a verification authority — is the server's
-- to compute, and a client that could send it could assert it
-- (knowledge-commands.md §3.1, Principle IX).
--
-- `verification_run` and `verification_attestation` are two report shapes, not
-- two trust levels: both are recorded `remote_attested`. Which route a report
-- arrived on says nothing about which verifier executed
-- (verification-summary.md §4, FR-811b/FR-811h).
--
-- `session_id` and `project_id` are nullable because a sessionless,
-- store-scoped command is a real thing — `cairn remember` outside any session
-- writes personal knowledge that belongs to no project.
CREATE TABLE command_spool (
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
                'verification_run','verification_attestation')),
  payload     TEXT NOT NULL,
  state       TEXT NOT NULL CHECK (state IN
                ('pending','in_flight','delivered','failed','refused')),
  attempts    INTEGER NOT NULL DEFAULT 0,
  claimed_at      TEXT,
  next_attempt_at TEXT,
  last_error_kind TEXT,
  created_at  TEXT NOT NULL,
  UNIQUE (scope_kind, scope_key, command_seq)
);

CREATE INDEX command_spool_claim ON command_spool (state, account_id, next_attempt_at);

-- Commands drain in the order they were issued within their scope, so the
-- claim orders on `command_seq` once it has narrowed to a scope.
CREATE INDEX command_spool_order ON command_spool (scope_kind, scope_key, command_seq);

-- ---------------------------------------------------------------------------
-- Step 4 — capture dispositions
-- ---------------------------------------------------------------------------

-- What happened to each attempted capture, counted per day.
--
-- Counts, not records: a disposition carries no payload content (FR-749d,
-- FR-741), so there is nothing to keep except how often it happened. The
-- vocabulary is closed and enforced here (data-model.md §4) rather than left
-- to the writer, because an unrecognized disposition is a silent hole in the
-- honesty this table exists to provide — `capture_deadline_exceeded` in
-- particular is the row that says the agent saw success while Cairn dropped
-- the event (FR-749c).
CREATE TABLE capture_disposition_counts (
  project_id  TEXT NOT NULL,
  agent       TEXT NOT NULL,
  kind        TEXT NOT NULL,
  disposition TEXT NOT NULL CHECK (disposition IN (
                'captured','declined_by_policy','capture_deadline_exceeded',
                'redaction_failed','privacy_refused','no_safe_semantic_mapping',
                'spooled','spool_overflow_dropped','spool_saturated_dropped',
                'transmitted','accepted','rejected_by_server','persisted')),
  day         TEXT NOT NULL,
  n           INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (project_id, agent, kind, disposition, day)
);

-- ---------------------------------------------------------------------------
-- Step 5 — authority and migration state
-- ---------------------------------------------------------------------------

-- Which side this store treats as authoritative for durable knowledge.
--
-- One row, `id = 1`. A store moves `feature_004` → `migrating` →
-- `server_authoritative`, and it moves *itself*: the server's own
-- `server_authority` row is a separate switch thrown by an administrator, and
-- a client may reach `server_authoritative` locally before its server does
-- (migration-cutover.md §1, FR-876a).
--
-- The seed row is written by `migrate.rs`'s `finish(8, tx)` hook.
CREATE TABLE authority_mode (
  id         INTEGER PRIMARY KEY CHECK (id = 1),
  mode       TEXT NOT NULL CHECK (mode IN
               ('feature_004','migrating','server_authoritative')),
  changed_at TEXT NOT NULL
);

-- Per-phase migration progress, so an interrupted migration resumes rather
-- than restarts (migration-cutover.md §6).
CREATE TABLE migration_state (
  phase        TEXT PRIMARY KEY,
  state        TEXT NOT NULL,
  detail_count INTEGER,
  started_at   TEXT,
  finished_at  TEXT
);

-- Records the server could not accept, which therefore stay local (FR-871).
--
-- The shape is awkward on purpose. It has to be able to name *every* retained
-- record type, and a relation has no id of its own — it is the triple
-- `from|to|kind` — so one nullable column cannot serve all three cases and the
-- CHECK enumerates them instead.
--
-- `dedupe_key` exists because SQLite treats NULLs as distinct in a UNIQUE
-- index, and by that CHECK every row has at least one NULL. A UNIQUE over the
-- natural columns would therefore never collide, and `--retry-retained` would
-- insert a second copy of every record it re-tried. One non-null discriminator
-- string is what actually deduplicates.
CREATE TABLE retained_local (
  ref_kind     TEXT NOT NULL CHECK (ref_kind IN ('knowledge','pattern','relation')),
  domain       TEXT CHECK (domain IN ('project','personal','team')),
  knowledge_id TEXT,
  relation_key TEXT,
  reason       TEXT NOT NULL CHECK (reason IN
                 ('local_only','server_refused','possession_indeterminate',
                  'owner_unclaimed')),
  detected_at  TEXT NOT NULL,
  dedupe_key   TEXT NOT NULL,
  -- `dedupe_key` is declared above this CHECK rather than below it, which is
  -- the one place this script departs from the column order in
  -- `data-model.md` §5. SQLite's grammar ends the column list at the first
  -- table constraint, so a column definition after a table-level CHECK is a
  -- syntax error. Nothing about the schema changes: column order is not
  -- semantics, and both constraints below still see every column.
  CHECK ((ref_kind = 'knowledge' AND domain IS NOT NULL AND knowledge_id IS NOT NULL
                                  AND relation_key IS NULL)
      OR (ref_kind = 'pattern'   AND domain IS NULL     AND knowledge_id IS NOT NULL
                                  AND relation_key IS NULL)
      OR (ref_kind = 'relation'  AND domain IS NULL     AND knowledge_id IS NULL
                                  AND relation_key IS NOT NULL)),
  UNIQUE (dedupe_key)
);

-- One-time establishment of who owns a pattern that predates ownership
-- (FR-867b).
--
-- Feature 004's local patterns have no owner. Attributing them to whichever
-- credential happens to be active at migration time would be asserting an
-- identity rather than establishing one (Principle XI), so an authenticated
-- claimant states the claim explicitly and it is persisted *here, first* —
-- before any delivery attempt. A claim that were only in flight would be
-- re-made with a different owner after a crash.
--
-- Two UNIQUE constraints, doing different work. `pattern_id` is
-- UUIDv5(owner_user_id ‖ content_key), so the same owner claiming the same
-- content twice derives the same id and collides: promoting twice yields one
-- record (SC-760). `(owner_user_id, content_key)` says the same thing from the
-- other side and is what makes the intent legible. Two *different* owners
-- claiming identical content are two different rows, and that is correct —
-- they are two people's patterns that happen to read alike.
CREATE TABLE legacy_pattern_claims (
  local_pattern_id TEXT PRIMARY KEY REFERENCES reusable_patterns(id),
  owner_user_id    TEXT NOT NULL,
  content_key      TEXT NOT NULL,
  pattern_id       TEXT NOT NULL UNIQUE,
  claimed_at       TEXT NOT NULL,
  UNIQUE (owner_user_id, content_key)
);
