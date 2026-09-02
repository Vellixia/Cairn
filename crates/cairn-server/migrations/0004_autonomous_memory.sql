-- Feature 005 — server-authoritative autonomous memory, server schema v4
-- (data-model.md §6).
--
-- Additive. One existing table gains one column (`memories.origin_kind`); no
-- existing column is renamed, retyped or dropped, and no existing row is
-- rewritten.
--
-- Two shapes recur and both are load-bearing, so they are stated once here
-- rather than re-explained at each table:
--
-- **The polymorphic reference.** Project, personal and team knowledge live in
-- three different tables, so a bare UUID cannot name a record — the same UUID
-- can legitimately exist in all three. Every table that references knowledge
-- therefore carries `ref_kind` + `domain` + an id, and a STORED generated
-- `reference_key` that folds them into one string. Where the reference takes
-- part in row identity, it is the generated key that is in the primary key,
-- never the bare UUID. The CHECK is repeated on every such table on purpose:
-- application validation alone is not what SC-766 asks for.
--
-- **Domain is not erased by `PatternRef`.** `domain IS NULL` on a polymorphic
-- row means the row holds a `PatternRef`, not that the pattern has no domain.
-- The `shared_patterns` row it resolves to carries `domain = 'personal'`
-- explicitly, and the column is CHECKed to that one value so the encoding can
-- never be read as a domain-less record (Constitution IV, data-model.md §6.1).

-- ---------------------------------------------------------------------------
-- Step 1 — safe events
-- ---------------------------------------------------------------------------

-- The typed, privacy-approved events the daemon delivers.
--
-- `event_id` is the whole idempotency mechanism: it is derived on the client
-- from the session and a durable ordinal, so a retried batch re-derives the
-- same key and the insert collides instead of duplicating.
--
-- `account_id` is bound from the authenticated credential, never from the
-- body. `session_id` is checked against that account server-side before an
-- event is accepted: an event names a session, and a name is not a proof
-- (FR-768, Principle XI).
--
-- `occurred_at` is advisory and is labelled so. It comes from a machine whose
-- clock the server cannot vouch for; `received_at` is the server's own and is
-- what anything ordered by time actually uses.
--
-- There is deliberately no column here for a transcript, a prompt, a diff or a
-- vendor's original JSON. `content` holds the approved per-kind structure and
-- nothing else — raw material has nowhere on this row to land (Principle V).
CREATE TABLE safe_events (
  event_id         UUID PRIMARY KEY,
  project_id       UUID NOT NULL REFERENCES projects(id),
  session_id       UUID NOT NULL REFERENCES sessions(id),
  account_id       UUID NOT NULL REFERENCES users(id),
  agent            TEXT NOT NULL,
  kind             TEXT NOT NULL,
  vendor_event     TEXT,
  session_seq      BIGINT NOT NULL,
  contract_version INT NOT NULL,
  content          JSONB NOT NULL,
  occurred_at      TIMESTAMPTZ NOT NULL,
  received_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (session_id, session_seq)
);

CREATE INDEX safe_events_project_time ON safe_events (project_id, received_at DESC);

-- ---------------------------------------------------------------------------
-- Step 2 — consolidation
-- ---------------------------------------------------------------------------

-- One row per session with work outstanding, and the reason it exists at all.
--
-- Consolidation claims a *session's* events as a unit, and the obvious way to
-- write that is `GROUP BY session_id ... FOR UPDATE SKIP LOCKED`. PostgreSQL
-- refuses it: a locking clause is not permitted with GROUP BY (SQLSTATE
-- 0A000). A group is not a row, and only a row can be locked. So the group is
-- given a row.
CREATE TABLE consolidation_session (
  project_id         UUID NOT NULL,
  session_id         UUID NOT NULL,
  state              TEXT NOT NULL DEFAULT 'pending'
                       CHECK (state IN ('pending','claimed','done')),
  claimed_by         TEXT,
  claim_expires_at   TIMESTAMPTZ,
  oldest_enqueued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (project_id, session_id)
);

-- Election reads oldest-first over everything not finished. Partial, so a
-- server whose history is mostly `done` does not pay for it on every claim.
CREATE INDEX consolidation_session_elect
  ON consolidation_session (oldest_enqueued_at) WHERE state <> 'done';

-- The events themselves, queued for consolidation.
--
-- `session_seq` is here because a batch has to be consolidated in the order
-- the work happened, and `event_id` is a UUIDv5 — derived from content, so it
-- orders arbitrarily. Ordering a batch by its primary key would shuffle a
-- session's history.
CREATE TABLE consolidation_work (
  event_id     UUID PRIMARY KEY REFERENCES safe_events(event_id),
  project_id   UUID NOT NULL,
  session_id   UUID NOT NULL,
  session_seq  BIGINT NOT NULL,
  enqueued_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  state        TEXT NOT NULL CHECK (state IN ('pending','done','failed')),
  attempts     INT NOT NULL DEFAULT 0,
  last_error   TEXT,
  FOREIGN KEY (project_id, session_id)
    REFERENCES consolidation_session (project_id, session_id)
);

CREATE INDEX consolidation_pending
  ON consolidation_work (project_id, session_id, session_seq) WHERE state = 'pending';

-- What a consolidation attempt did, whether or not it produced anything.
--
-- A run with zero candidates is still a run, and recording it is the
-- difference between "consolidation found nothing" and "consolidation never
-- happened" — two states a health report must not conflate.
CREATE TABLE consolidation_runs (
  run_id              UUID PRIMARY KEY,
  project_id          UUID NOT NULL,
  session_id          UUID,
  started_at          TIMESTAMPTZ NOT NULL,
  finished_at         TIMESTAMPTZ,
  events_claimed      INT,
  candidates_proposed INT,
  candidates_accepted INT,
  candidates_refused  INT,
  extractor_kind      TEXT NOT NULL,
  state               TEXT NOT NULL
);

-- What the extractor proposed and what Cairn decided about it.
--
-- The extractor proposes; Cairn decides (Principle IX), and both halves are
-- recorded so "why is this knowledge here" has an answer that is not a guess.
--
-- `topic_key` and `value_key` are stored **after** Cairn's normalization, never
-- as proposed: the normalized form is what reconciliation matches on, and
-- keeping the proposal here would leave two candidate rows disagreeing about
-- what the same subject is called.
--
-- `candidate_id` is deterministic (consolidation.md §7) and derived from the
-- project, the session and the normalized keys — deliberately *not* from the
-- event set, which is not stable across a reclaim. That makes re-executing a
-- reclaimed batch an upsert rather than a second corroboration record.
CREATE TABLE knowledge_candidates (
  candidate_id        UUID PRIMARY KEY,
  run_id              UUID NOT NULL REFERENCES consolidation_runs(run_id),
  project_id          UUID NOT NULL,
  proposed_kind       TEXT NOT NULL,
  proposed_domain     TEXT NOT NULL,
  topic_key           TEXT,
  value_key           TEXT,
  content             TEXT NOT NULL,
  decision            TEXT NOT NULL CHECK (decision IN
                        ('accepted','reinforced','duplicate','conflicted','refused')),
  refusal_reason      TEXT,
  result_ref_kind     TEXT CHECK (result_ref_kind IN ('knowledge','pattern')),
  result_domain       TEXT CHECK (result_domain IN ('project','personal','team')),
  result_knowledge_id UUID,
  CHECK ((result_ref_kind IS NULL AND result_domain IS NULL AND result_knowledge_id IS NULL)
      OR (result_ref_kind = 'knowledge' AND result_domain IS NOT NULL
                                         AND result_knowledge_id IS NOT NULL)
      OR (result_ref_kind = 'pattern'   AND result_domain IS NULL
                                         AND result_knowledge_id IS NOT NULL)),
  UNIQUE (run_id, topic_key, value_key)
);

-- Which events a candidate was derived from: the provenance link that makes a
-- consolidated record explainable rather than merely present.
CREATE TABLE candidate_source_events (
  candidate_id UUID NOT NULL REFERENCES knowledge_candidates(candidate_id),
  event_id     UUID NOT NULL REFERENCES safe_events(event_id),
  PRIMARY KEY (candidate_id, event_id)
);

CREATE INDEX candidate_source_events_event ON candidate_source_events (event_id);

-- ---------------------------------------------------------------------------
-- Step 3 — retrieval traces
-- ---------------------------------------------------------------------------

-- One row per authenticated retrieval, created **before** anything is
-- generated.
--
-- The four states are the whole point of the table (Principle X). A trace is
-- `requested` the moment an authenticated caller asks; it becomes `generated`
-- when a briefing exists; it becomes `transmitted` only when an authenticated,
-- idempotent outcome report says a hook actually delivered it; and `failed`
-- when it did not. Nothing collapses those into "we sent context": generating
-- a briefing is not evidence that an agent received one.
--
-- `acknowledgement_state` stays `unavailable` throughout in this feature. There
-- is no producer for `acknowledged`, and there is deliberately no default that
-- pretends otherwise — receipt is reported as no evidence because that is what
-- there is (FR-838e).
--
-- The three CHECKs enforce the parts of that lifecycle a NULL could otherwise
-- hide: no latency before generation, no degradation level claimed for a
-- briefing that was never built, and no failure without a stated reason.
CREATE TABLE retrieval_traces (
  trace_id                 UUID PRIMARY KEY,
  project_id               UUID NOT NULL,
  session_id               UUID NOT NULL,
  account_id               UUID NOT NULL,
  trigger                  TEXT NOT NULL,
  delivery_point           TEXT NOT NULL,
  degradation_level        TEXT,
  budget_tokens            INT,
  budget_spent             INT,
  latency_ms               INT,
  delivery_state           TEXT NOT NULL CHECK (delivery_state IN
                             ('requested','generated','transmitted','failed')),
  acknowledgement_state    TEXT NOT NULL DEFAULT 'unavailable'
                             CHECK (acknowledgement_state IN ('unavailable','acknowledged')),
  failure_reason           TEXT,
  transmission_reported_at TIMESTAMPTZ,
  created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK ((delivery_state = 'requested' AND latency_ms IS NULL)
      OR (delivery_state <> 'requested' AND latency_ms IS NOT NULL)),
  CHECK ((delivery_state IN ('generated','transmitted') AND degradation_level IS NOT NULL)
      OR delivery_state IN ('requested','failed')),
  CHECK ((delivery_state = 'failed' AND failure_reason IS NOT NULL)
      OR (delivery_state <> 'failed' AND failure_reason IS NULL))
);

-- Traces and their items are retained 90 days and then deleted oldest-first by
-- a bounded sweep in the same background task as consolidation (FR-847), so
-- volume is bounded by traffic × 90 days rather than growing without limit.
CREATE INDEX retrieval_traces_session ON retrieval_traces (session_id, created_at DESC);
CREATE INDEX retrieval_traces_sweep ON retrieval_traces (created_at);

-- What each retrieval considered and what it selected.
--
-- `source_updated_at` is copied from the referenced row at selection time
-- because the delivery-dedup upsert needs to compare against the version that
-- was actually delivered, and by the time an outcome is reported the source
-- row may have moved on.
CREATE TABLE retrieval_trace_items (
  trace_id          UUID NOT NULL REFERENCES retrieval_traces(trace_id) ON DELETE CASCADE,
  ref_kind          TEXT NOT NULL CHECK (ref_kind IN ('knowledge','pattern')),
  domain            TEXT CHECK (domain IN ('project','personal','team')),
  knowledge_id      UUID NOT NULL,
  reference_key     TEXT GENERATED ALWAYS AS (
    CASE WHEN ref_kind = 'knowledge'
         THEN 'knowledge:' || domain || ':' || knowledge_id::text
         ELSE 'pattern:' || knowledge_id::text END
  ) STORED NOT NULL,
  status            TEXT NOT NULL CHECK (status IN ('considered','selected')),
  selection_rule    TEXT,
  rank              INT,
  source_updated_at TIMESTAMPTZ NOT NULL,
  CHECK ((ref_kind = 'knowledge' AND domain IS NOT NULL)
      OR (ref_kind = 'pattern'   AND domain IS NULL)),
  PRIMARY KEY (trace_id, reference_key)
);

-- ---------------------------------------------------------------------------
-- Step 4 — verification, reported rather than asserted
-- ---------------------------------------------------------------------------

-- Verification **runs**, as reported. Not verification states.
--
-- `authority` is assigned by the server and never read from the payload. Every
-- client-reported result in Feature 005 is `remote_attested`: a bearer token
-- proves which account is talking, and an HTTP route proves which URL was
-- called, and neither proves which verifier executed. `cairn` is reserved for
-- server-executed verification, and no route in this feature produces
-- `remote_cairn` (verification-summary.md §4, FR-811b/FR-811h).
--
-- `project_id` is nullable because personal and team knowledge are
-- project-independent; a personal record's report has no project to name, and
-- inventing one would leak the project the reporter happened to be in.
--
-- The uniqueness rule is deliberately per account: the same account retrying
-- the same logical run is one report, while two accounts reporting the same
-- record are two, because they are two pieces of evidence.
CREATE TABLE verification_reports (
  report_id     UUID PRIMARY KEY,
  ref_kind      TEXT NOT NULL CHECK (ref_kind IN ('knowledge','pattern')),
  domain        TEXT CHECK (domain IN ('project','personal','team')),
  knowledge_id  UUID NOT NULL,
  reference_key TEXT GENERATED ALWAYS AS (
    CASE WHEN ref_kind = 'knowledge'
         THEN 'knowledge:' || domain || ':' || knowledge_id::text
         ELSE 'pattern:' || knowledge_id::text END
  ) STORED NOT NULL,
  project_id    UUID,
  owner_user_id UUID,
  account_id    UUID NOT NULL,
  verdict       TEXT NOT NULL CHECK (verdict IN ('passed','failed','inconclusive')),
  verifier_kind TEXT NOT NULL,
  authority     TEXT NOT NULL,
  run_at        TIMESTAMPTZ NOT NULL,
  received_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK ((ref_kind = 'knowledge' AND domain IS NOT NULL)
      OR (ref_kind = 'pattern'   AND domain IS NULL)),
  UNIQUE (reference_key, account_id, verifier_kind, run_at)
);

CREATE INDEX verification_reports_ref ON verification_reports (reference_key, run_at DESC);

-- Project knowledge already has its verification columns, added by migration 2
-- (`verification`, `verification_authority`, `last_verified_at`,
-- `verification_basis`, `evidence_fact_count`). They are NOT re-added and NOT
-- renamed: `verification` is the canonical state column and there is no
-- `verification_state`. Feature 005 changes who computes them — the server,
-- from reports — not what they are called.
--
-- The other three record types have no such columns and gain none. Their
-- derived summaries live in one table instead, keyed by the same canonical
-- reference key everything else uses.
CREATE TABLE knowledge_verification (
  ref_kind               TEXT NOT NULL CHECK (ref_kind IN ('knowledge','pattern')),
  domain                 TEXT CHECK (domain IN ('personal','team')),
  knowledge_id           UUID NOT NULL,
  reference_key          TEXT GENERATED ALWAYS AS (
    CASE WHEN ref_kind = 'knowledge'
         THEN 'knowledge:' || domain || ':' || knowledge_id::text
         ELSE 'pattern:' || knowledge_id::text END
  ) STORED NOT NULL,
  verification           TEXT NOT NULL DEFAULT 'unverified',
  verification_authority TEXT,
  verification_basis     JSONB NOT NULL DEFAULT '[]'::jsonb,
  evidence_fact_count    INTEGER NOT NULL DEFAULT 0,
  last_verified_at       TIMESTAMPTZ,
  CHECK ((ref_kind = 'knowledge' AND domain IS NOT NULL)
      OR (ref_kind = 'pattern'   AND domain IS NULL)),
  PRIMARY KEY (reference_key)
);

-- Pre-cutover verification values, kept as an audit trail and never derived
-- from.
--
-- A value written before the server computed verification was asserted by a
-- client, and demoting it is the honest move. Deleting it would destroy the
-- record of what was previously claimed; trusting it would carry the overclaim
-- forward. It is kept here, out of the derivation path.
CREATE TABLE legacy_verification_audit (
  domain                  TEXT NOT NULL,
  knowledge_id            UUID NOT NULL,
  legacy_state            TEXT,
  legacy_authority        TEXT,
  legacy_last_verified_at TIMESTAMPTZ,
  demoted_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (domain, knowledge_id)
);

-- ---------------------------------------------------------------------------
-- Step 5 — reusable patterns, as a personal-domain record
-- ---------------------------------------------------------------------------

-- The safe, canonical shape of a reusable pattern.
--
-- This is a **replacement** record, not a relaxation of the local one. The
-- local `reusable_patterns` row is refused by the synchronization boundary and
-- stays refused: `reusable_pattern` is a forbidden entity type, and the row
-- carries `signal_digest`, `origin_ref` and `sanitization_report`, all refused
-- field names. Those columns are absent here on purpose, and each absence is a
-- decision:
--
--   signals, signal_digest  the digest is a refused name; the raw signals are
--                           local evidence
--   origin_ref              refused name; the source project must not be
--                           nameable from a project-independent record
--   sanitization_report     refused name; local diagnostic output
--   source_memory_id        names a project memory, which is exactly what a
--                           project-independent record must not do
--   origin_deleted          a fact about a local row that no longer travels
--
-- `domain` is `personal` and CHECKed to that single value: a pattern is a
-- personal-domain record of type `pattern`, not a fourth domain and not a
-- record without one (Constitution IV).
--
-- `trust` is CHECKed to the single value `sanitized`, which is the only trust
-- the server can actually establish — that this record passed the privacy
-- gate. `validated` and `contested` are derived locally from
-- `pattern_applications`, which stay local-only, so the server has no evidence
-- for them and a client asserting one would be asserting a state earned
-- privately on a record the server cannot check. That is the same class of
-- overclaim as client-asserted verification, and it is refused the same way.
CREATE TABLE shared_patterns (
  pattern_id    UUID PRIMARY KEY,
  domain        TEXT NOT NULL DEFAULT 'personal' CHECK (domain = 'personal'),
  owner_user_id UUID NOT NULL REFERENCES users(id),
  title         TEXT NOT NULL,
  problem       TEXT NOT NULL,
  root_cause    TEXT NOT NULL,
  approach      TEXT NOT NULL,
  constraints   JSONB NOT NULL DEFAULT '[]'::jsonb,
  applicability JSONB NOT NULL DEFAULT '[]'::jsonb,
  trust         TEXT NOT NULL DEFAULT 'sanitized' CHECK (trust IN ('sanitized')),
  topic_key     TEXT,
  value_key     TEXT,
  content_key   TEXT NOT NULL,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  forgotten_at  TIMESTAMPTZ
);

-- `pattern_id` is UUIDv5(owner ‖ content_key), so this unique index and the
-- primary key say the same thing from two directions. Both are kept: the index
-- is what makes a repeat promotion an upsert and migration re-runnable, and it
-- states the rule in terms a reader can check without deriving a UUID.
CREATE UNIQUE INDEX shared_patterns_identity ON shared_patterns (owner_user_id, content_key);

CREATE INDEX shared_patterns_owner ON shared_patterns (owner_user_id, forgotten_at);

CREATE INDEX shared_patterns_search
  ON shared_patterns USING GIN (to_tsvector('english', problem || ' ' || approach));

-- ---------------------------------------------------------------------------
-- Step 6 — server-side text search for the non-project domains
-- ---------------------------------------------------------------------------

-- FR-806 puts server-side retrieval over personal and team knowledge in scope,
-- and neither table had a text index: only `memories` did
-- (`memories_search`, 0001_init.sql). These mirror it exactly.
--
-- Expression indexes over `to_tsvector`, not a materialized `tsvector` column,
-- which is why there is no trigger here and why that is not an omission. The
-- local SQLite side needs triggers because FTS5 keeps a shadow table that has
-- to be told when a row changes; a PostgreSQL expression index is maintained
-- by the index machinery itself. Adding a trigger would give a second thing to
-- keep correct and buy nothing.
--
-- Ownership is *not* in the index, and must not be read as being in it. These
-- make a search fast; they do not make it safe. Every read path filters
-- `personal_knowledge` on `owner_user_id` and applies the team rules before a
-- row is rendered (data-model.md §6.1).
CREATE INDEX personal_knowledge_search
  ON personal_knowledge USING GIN (to_tsvector('english', content));

CREATE INDEX team_knowledge_search
  ON team_knowledge USING GIN (to_tsvector('english', content));

-- ---------------------------------------------------------------------------
-- Step 7 — capability health and delivery
-- ---------------------------------------------------------------------------

-- Per-agent, per-capability health, so a matrix cell is never blank.
--
-- `no_evidence` is a first-class status rather than an absent row, and that is
-- the honesty this table exists for: "we have never observed this" and "this
-- works" are different answers, and a missing row would render as neither.
--
-- `evidence_kind` separates configuration read-back from runtime capture
-- (FR-852). A capability declared by introspecting an adapter has not been
-- observed working, and a matrix that showed both the same way would report
-- confidence it has not earned.
--
-- `writer_id` is in the primary key because a capability is observed on a
-- *machine* (FR-857). One account on two laptops can legitimately see two
-- different answers, and collapsing them would let a working machine hide a
-- broken one.
CREATE TABLE integration_health (
  project_id    UUID NOT NULL,
  account_id    UUID NOT NULL,
  writer_id     TEXT NOT NULL,
  agent         TEXT NOT NULL,
  capability    TEXT NOT NULL,
  stage         TEXT NOT NULL,
  status        TEXT NOT NULL CHECK (status IN
                  ('supported','unsupported_by_vendor','declined_by_cairn',
                   'adapter_unimplemented','runtime_failure','no_evidence')),
  evidence_kind TEXT CHECK (evidence_kind IS NULL
                            OR evidence_kind IN ('introspection','observation')),
  observed_at   TIMESTAMPTZ,
  degraded      BOOLEAN,
  PRIMARY KEY (project_id, account_id, writer_id, agent, capability, stage)
);

-- What a session has already been given, so it is not given it again.
--
-- A **server** table, because selection is server-side and the server is the
-- only party holding both sides of the comparison.
--
-- A row here is written only after an authenticated transmission outcome says
-- the item actually reached the agent. Writing it at generation time would
-- suppress the redelivery of context that a failed transmission means the
-- agent never saw — the dedup would be enforcing a delivery that did not
-- happen (Principle X).
CREATE TABLE delivered_context (
  session_id        UUID NOT NULL REFERENCES sessions(id),
  ref_kind          TEXT NOT NULL CHECK (ref_kind IN ('knowledge','pattern')),
  domain            TEXT CHECK (domain IN ('project','personal','team')),
  knowledge_id      UUID NOT NULL,
  reference_key     TEXT GENERATED ALWAYS AS (
    CASE WHEN ref_kind = 'knowledge'
         THEN 'knowledge:' || domain || ':' || knowledge_id::text
         ELSE 'pattern:' || knowledge_id::text END
  ) STORED NOT NULL,
  delivered_at      TIMESTAMPTZ NOT NULL,
  source_updated_at TIMESTAMPTZ NOT NULL,
  delivery_point    TEXT NOT NULL,
  CHECK ((ref_kind = 'knowledge' AND domain IS NOT NULL)
      OR (ref_kind = 'pattern'   AND domain IS NULL)),
  PRIMARY KEY (session_id, reference_key)
);

-- The capture funnel, counted rather than recorded.
--
-- Both client-reported and server-observed dispositions land here. The
-- vocabulary is closed (data-model.md §4) and is CHECKed rather than left to
-- the writer: an unrecognized disposition would be a silent hole in the one
-- table whose job is to say what was lost. `capture_deadline_exceeded` in
-- particular is the row that records the agent seeing success while Cairn
-- dropped the event (FR-749c).
--
-- A disposition carries no payload content (FR-749d, FR-741), so there is
-- nothing to keep beyond how often it happened.
CREATE TABLE capture_dispositions (
  project_id  UUID NOT NULL,
  account_id  UUID NOT NULL,
  agent       TEXT NOT NULL,
  kind        TEXT NOT NULL,
  disposition TEXT NOT NULL CHECK (disposition IN (
                'captured','declined_by_policy','capture_deadline_exceeded',
                'redaction_failed','privacy_refused','no_safe_semantic_mapping',
                'spooled','spool_overflow_dropped','spool_saturated_dropped',
                'transmitted','accepted','rejected_by_server','persisted')),
  day         DATE NOT NULL,
  n           BIGINT NOT NULL DEFAULT 0,
  PRIMARY KEY (project_id, account_id, agent, kind, disposition, day)
);

-- ---------------------------------------------------------------------------
-- Step 8 — the memories origin discriminator
-- ---------------------------------------------------------------------------

-- Whether a project memory was written explicitly or produced by consolidation
-- (FR-816).
--
-- Nullable with no default and no backfill: every row that already exists
-- predates the distinction, and stamping them all `explicit` would be
-- inventing a provenance nobody recorded. NULL means "written before Cairn
-- tracked this", which is the truth.
ALTER TABLE memories ADD COLUMN IF NOT EXISTS origin_kind TEXT;

-- ---------------------------------------------------------------------------
-- Step 9 — server authority
-- ---------------------------------------------------------------------------

-- Whether this deployment has cut over, as one row an administrator moves once.
--
-- Separate from the client's own `authority_mode`, and deliberately so: a
-- client may reach `server_authoritative` locally before its server does, and
-- one global flag could not express that (migration-cutover.md §1, FR-876a).
CREATE TABLE server_authority (
  id         INT PRIMARY KEY CHECK (id = 1),
  mode       TEXT NOT NULL CHECK (mode IN ('pre_cutover','server_authoritative')),
  cutover_at TIMESTAMPTZ
);

-- Every deployment starts before its own cutover, including a brand-new one.
-- Initializing at `server_authoritative` would be asserting that a migration
-- established canonical possession, and on a fresh database nothing has.
INSERT INTO server_authority (id, mode, cutover_at)
VALUES (1, 'pre_cutover', NULL)
ON CONFLICT (id) DO NOTHING;
