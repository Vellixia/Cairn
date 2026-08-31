# Data Model — Feature 005

**Local schema**: v7 → **v8**  |  **Server schema**: v3 → **v4**

Naming rule, binding everywhere below: **no field name may collide with a name the existing
synchronization boundary refuses** (`crates/cairn-server/src/sync.rs:27-61`, refused
recursively at any depth). The refused set is `summary`, `path`, `command`, `details`,
`exit_code`, `observations`, `observed_value`, `source_locator`, `value_digest`, `fingerprint`,
`relevant_paths`, `criteria_snapshot`, `sanitization_report`, `origin_ref`, `alternative_cause`,
`signal_digest`, `pin_reason`, `rationale`, `basis_evidence_id`, `path_fingerprints`,
`task_snapshot_at_bind`, `detail`, `prior_value`, `new_value`, `content_norm_digest`,
`local_revision`, plus `outcome` at top level. The **session** refusals apply equally
(`FORBIDDEN_SESSION_FIELDS`): `worktree_path`, `agent_session_key`, `daemon_run_id`,
`last_event_at`, `last_turn_ended_at`. Every substitute chosen below is listed in
`contracts/safe-events.md` §2.

---

## 1. SafeCanonicalEvent

The single record that crosses the machine boundary. Envelope plus a closed per-kind content
union. Nothing else is transmitted from capture.

### 1.1 Envelope

| Field | Type | Notes |
|---|---|---|
| `event_id` | UUID | Deterministic. §1.4. |
| `contract_version` | u16 | Starts at 1. FR-742. |
| `kind` | enum | One of §1.2. |
| `agent` | enum | `claude_code` \| `codex` \| `opencode`. |
| `vendor_event` | string ≤64 | The vendor's own event name, sanitized. Provenance only (FR-724). |
| `session_id` | UUID | The **synced** Cairn session id, which the server already holds. |
| `session_seq` | u64 | Per-session monotonic ordinal, daemon-assigned. §1.4. |
| `occurred_at` | RFC3339 | Client clock. **Advisory only** — never used for ordering or identity (FR-780). |
| `content` | union | Per-kind, §1.3. Absent for kinds that carry none. |

`project_id` and `account_id` are **not** envelope fields. They are bound server-side from the
authenticated credential and the verified session (FR-769, FR-769a). A client cannot assert
them.

`agent_session_key` is **not** an envelope field and must not become one: it is on
`FORBIDDEN_SESSION_FIELDS`, the server has deliberately never had a column for it
(`crates/cairn-server/migrations/0001_init.sql:68-70`), and putting it on this boundary would
weaken a standing refusal and breach FR-777a/SC-751. The daemon holds the synced session UUID;
that is what travels.

### 1.2 Event kinds (21)

`session_opened`, `session_closed`, `context_compacting`, `context_compacted`,
`subagent_started`, `subagent_completed`, `tool_started`, `tool_succeeded`, `tool_failed`,
`file_read`, `file_changed`, `command_executed`, `test_executed`, `test_result`,
`research_activity`, `user_instruction_signal`, `decision_signal`, `capture_declined`,
`capture_failed`, `session_resumed`, `agent_quiesced`.

The seven Feature 001–003 lifecycle events map onto this set without loss, so handoff
generation, checkpointing and context delivery keep working (FR-744).

### 1.3 Per-kind content

| Kind | Fields |
|---|---|
| `session_opened`, `session_resumed` | `open_trigger` ∈ {`startup`,`resume`,`clear`,`compact`,`fork`} |
| `session_closed` | `close_reason` ≤64 |
| `context_compacting`, `context_compacted` | `compaction_trigger` ∈ {`manual`,`auto`} |
| `subagent_started`, `subagent_completed` | `subagent_ref` ≤128, `subagent_kind` ≤64, `parent_session_seq` u64 |
| `tool_started`, `tool_succeeded` | `vendor_tool` ≤64, `tool_class` enum |
| `tool_failed` | `vendor_tool`, `tool_class`, `failure_kind` enum, `failure_note` ≤512 (redacted), `exit_status` i32? |
| `file_read`, `file_changed` | `repo_file` (§2), `change_kind` ∈ {`created`,`modified`,`deleted`,`renamed`}, `repo_file_from` (renames), `file_identity` ∈ {`present`,`out_of_repository`,`unavailable_from_vendor`} |
| `command_executed` | `command_line` ≤512 (redacted), `exit_status` i32? |
| `test_executed` | `test_command` ≤512 (redacted) |
| `test_result` | `test_outcome` ∈ {`passed`,`failed`,`unknown`}, `exit_status` i32?, `tests_total` u32?, `tests_failed` u32? |
| `research_activity` | `resource_kind` ∈ {`docs`,`web`,`repository`,`other`} |
| `user_instruction_signal` | `instruction_kind` ∈ {`require`,`forbid`,`prefer`,`scope`,`correct`}, `subject_token`, `object_token`, `justified_by_seq` u64?, `lexicon_version` u16 |
| `decision_signal` | `decision_kind` ∈ {`adopt`,`reject`,`defer`,`constrain`,`prefer`,`revert`}, `subject_token`, `object_token`, `justified_by_seq` u64?, `lexicon_version` u16 |
| `capture_declined`, `capture_failed` | `disposition` enum (§4), `stage` enum, `decline_reason` enum ∈ {`no_safe_semantic_mapping`,`ambiguous_classification`,`insufficient_vocabulary`,`vendor_unavailable`,`policy_excluded`} |
| `agent_quiesced` | *(none)* |

**No free text derived from user or assistant messages appears anywhere in this table.**
`failure_note`, `command_line` and `test_command` are tool-level strings that pass redaction
first, and are the only free-text fields in the model.

`subject_token` and `object_token` are **not** free text. They are vocabulary-justified tokens:
key-shaped (`[a-z0-9_]`, dot-segmented, ≤128 / ≤64 chars) **and** required to appear in the
session's derived vocabulary — the file and module tokens, command verbs, test identifiers and
established project keys visible in that session's own events. A token outside that set is
refused, by the client when constructing and by the server independently
(`contracts/extraction.md` §13). This is what lets a decision survive the boundary without a
prompt fragment surviving with it: a sentence's words are not in the vocabulary, and neither is
a credential.

### 1.4 Identity — stable across hook invocations and retries

Each hook run is a separate short-lived process and cannot share a counter. The daemon can.

```
hook (stateless)  ──canonical event──▶  daemon
                                         │  one SQLite transaction:
                                         │    session_seq = next_seq++ from session_event_seq
                                         │    event_id    = UUIDv5(CAIRN_EVENT_NS,
                                         │                    session_id ‖ session_seq)
                                         │    INSERT INTO event_spool
                                         ▼
                                    spooled, identity fixed forever
```

Two properties this depends on, both deliberate:

- **The counter is durable and independent of the spool.** It lives in `session_event_seq`
  and is never derived from `MAX(session_seq)` over `event_spool`. The spool drains, and sheds
  rows under the capacity policy; a counter read from it would reset, re-derive an already-used
  `event_id`, and the server would answer `duplicate` — silently discarding a real event.
- **Identity keys on `session_id`, not the vendor key.** A session id is never reused. Shipped
  code frees the vendor key on deletion so it *can* be reused
  (`crates/cairn-store/src/repo.rs:1610-1616`), so keying on it would collapse a resumed
  session's events onto an earlier session's ids.

Consequences: identity is assigned once and never recomputed, so any number of delivery
retries carry the same `event_id`; the server's primary key makes a redelivery a `duplicate`
rather than a second event (FR-770); a genuinely repeated act gets a new ordinal and is a
distinct event (FR-738); and nothing depends on a clock (FR-780).

**The server re-derives and verifies.** `event_id` travels on the wire, but the server
recomputes `UUIDv5(CAIRN_EVENT_NS, session_id ‖ session_seq)` and refuses a mismatch
(`event_id_mismatch`). Without this, idempotency would be client-controlled: a buggy or hostile
client could submit a colliding id, be answered `duplicate`, and suppress a genuine event, or
pre-claim ids it can guess.

---

## 2. `repo_file`

Repository-relative file identity. **Maximum 1024 bytes UTF-8, maximum 64 path segments.**

Validation, applied identically on the client when constructing and on the server when
accepting (FR-777d):

1. non-empty; 2. no leading `/`; 3. no `..` segment; 4. no drive-letter prefix (`C:`);
5. no UNC prefix (`\\`); 6. separators normalized to `/`; 7. no empty interior segment;
8. within both bounds.

Four dispositions, matching FR-777e–g:

| Vendor supplies | `file_identity` | `repo_file` |
|---|---|---|
| absolute path inside the repo | `present` | relativized locally against the repository root |
| path outside the repo | `out_of_repository` | absent |
| nothing | `unavailable_from_vendor` | absent |
| absolute value arriving on the wire | *(refused)* | server rejects the event |

The repository root is machine configuration and is never transmitted (FR-753).

---

## 3. Numeric bounds

| Bound | Value |
|---|---|
| `repo_file` | 1024 bytes, 64 segments |
| Free-text event field (`command_line`, `test_command`, `failure_note`) | 512 bytes |
| `vendor_event`, `vendor_tool`, `subagent_kind` | 64 chars |
| Serialized event content | 8 KiB |
| Serialized whole event | 16 KiB |
| Events per ingest batch | 256 |
| Ingest request body | 1 MiB |
| Spool capacity | 50,000 events **or** 256 MiB, whichever binds first |
| Consolidation batch | 200 events |
| Extraction input | 200 events / 256 KiB |
| `topic_key` | 128 chars, 6 segments (unchanged) |
| `value_key` | 64 chars (unchanged) |
| Retrieval trace items | 200 per trace |

---

## 4. Capture dispositions

`captured`, `declined_by_policy`, `capture_deadline_exceeded`, `redaction_failed`,
`privacy_refused`, `no_safe_semantic_mapping`, `spooled`, `spool_overflow_dropped`,
`spool_saturated_dropped`,
`transmitted`, `accepted`,
`rejected_by_server`, `persisted`.

`capture_deadline_exceeded` is the FR-749c disposition: the agent sees success, Cairn counts a
drop. A disposition record carries no payload content (FR-749d, FR-741).

---

## 5. Local schema v8 (SQLite)

Additive. No existing table changes meaning.

```sql
CREATE TABLE event_spool (
  event_id        TEXT PRIMARY KEY,
  session_id      TEXT NOT NULL REFERENCES sessions(id),
  project_id      TEXT NOT NULL REFERENCES projects(id),
  account_id      TEXT NOT NULL,          -- authored-by; exact match on claim, never NULL-open
  session_seq     INTEGER NOT NULL,
  kind            TEXT NOT NULL,
  payload         TEXT NOT NULL,          -- the approved SafeCanonicalEvent
  payload_bytes   INTEGER NOT NULL,
  boundary_class  INTEGER NOT NULL,       -- 1 = never dropped on overflow
  state           TEXT NOT NULL CHECK (state IN
                    ('pending','in_flight','delivered','failed','refused')),
  attempts        INTEGER NOT NULL DEFAULT 0,
  claimed_at      TEXT,
  next_attempt_at TEXT,
  last_error_kind TEXT,
  created_at      TEXT NOT NULL,
  UNIQUE (session_id, session_seq)
);
CREATE INDEX event_spool_claim ON event_spool (state, account_id, next_attempt_at);

CREATE TABLE capture_disposition_counts (
  project_id TEXT NOT NULL, agent TEXT NOT NULL, kind TEXT NOT NULL,
  disposition TEXT NOT NULL, day TEXT NOT NULL, n INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (project_id, agent, kind, disposition, day)
);

CREATE TABLE session_event_seq (           -- durable ordinal; survives spool drain
  session_id TEXT PRIMARY KEY REFERENCES sessions(id),
  next_seq   INTEGER NOT NULL DEFAULT 1
);   -- commands use `command_seq` (below), which also covers sessionless scopes

CREATE TABLE command_seq (                 -- durable counters; one row per scope
  scope_kind TEXT NOT NULL CHECK (scope_kind IN ('session','store')),
  scope_key  TEXT NOT NULL,                -- session id, or the store's writer_id
  next_seq   INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY (scope_kind, scope_key)
);

CREATE TABLE command_spool (               -- knowledge commands awaiting the server
  command_id  TEXT PRIMARY KEY,            -- UUIDv5(scope_kind ‖ scope_key ‖ command_seq)
  scope_kind  TEXT NOT NULL CHECK (scope_kind IN ('session','store')),
  scope_key   TEXT NOT NULL,
  session_id  TEXT REFERENCES sessions(id),   -- NULL for a sessionless command
  project_id  TEXT REFERENCES projects(id),
  account_id  TEXT NOT NULL,               -- exact match on claim; never NULL-open
  command_seq INTEGER NOT NULL,
  kind        TEXT NOT NULL,               -- remember | supersede | reinforce | relate |
                                           -- pin | forget | personal_create | personal_forget |
                                           -- team_propose | pattern_promote | pattern_forget |
                                           -- verification_run | verification_attestation
                                           --   (two report shapes; both are remote_attested.
                                           --    A route name does not establish a verifier.)
  payload     TEXT NOT NULL,               -- intent only; no derived state (§3.1 of the contract)
  state       TEXT NOT NULL CHECK (state IN
                ('pending','in_flight','delivered','failed','refused')),
  attempts    INTEGER NOT NULL DEFAULT 0,
  claimed_at TEXT, next_attempt_at TEXT, last_error_kind TEXT,
  created_at  TEXT NOT NULL,
  UNIQUE (scope_kind, scope_key, command_seq)
);

CREATE TABLE authority_mode (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  mode TEXT NOT NULL CHECK (mode IN ('feature_004','migrating','server_authoritative')),
  changed_at TEXT NOT NULL
);

CREATE TABLE retained_local (              -- records the server could not accept (FR-871)
  -- must be able to name EVERY retained record type, including a relation, which has no id
  ref_kind     TEXT NOT NULL CHECK (ref_kind IN ('knowledge','pattern','relation')),
  domain       TEXT CHECK (domain IN ('project','personal','team')), -- knowledge only
  knowledge_id TEXT,                       -- knowledge id or pattern_id; NULL for a relation
  relation_key TEXT,                       -- 'from|to|kind' for a relation; NULL otherwise
  reason       TEXT NOT NULL CHECK (reason IN
                 ('local_only','server_refused','possession_indeterminate',
                  'owner_unclaimed')),
  detected_at  TEXT NOT NULL,
  CHECK ((ref_kind = 'knowledge' AND domain IS NOT NULL AND knowledge_id IS NOT NULL
                                  AND relation_key IS NULL)
      OR (ref_kind = 'pattern'   AND domain IS NULL     AND knowledge_id IS NOT NULL
                                  AND relation_key IS NULL)
      OR (ref_kind = 'relation'  AND domain IS NULL     AND knowledge_id IS NULL
                                  AND relation_key IS NOT NULL)),
  -- SQLite treats NULLs as distinct in a UNIQUE index, and every row has a NULL by the CHECK
  -- above, so a naive UNIQUE would not deduplicate and `--retry-retained` would insert twice.
  -- A single non-null discriminator key is used instead.
  dedupe_key   TEXT NOT NULL,   -- 'knowledge:<domain>:<id>' | 'pattern:<id>' | 'relation:<key>'
  UNIQUE (dedupe_key)
);

CREATE TABLE migration_state (
  phase TEXT PRIMARY KEY, state TEXT NOT NULL, detail_count INTEGER,
  started_at TEXT, finished_at TEXT
);

CREATE TABLE legacy_pattern_claims (       -- one-time owner establishment, FR-867b
  local_pattern_id TEXT PRIMARY KEY REFERENCES reusable_patterns(id),
  owner_user_id    TEXT NOT NULL,          -- authenticated claimant, persisted before delivery
  content_key      TEXT NOT NULL,          -- safe normalized-content digest, §6.2
  pattern_id       TEXT NOT NULL UNIQUE,   -- UUIDv5(owner_user_id || content_key)
  claimed_at       TEXT NOT NULL,
  UNIQUE (owner_user_id, content_key)
);
```

Spool overflow (FR-785), in order:

1. Drop the **oldest capture-class** rows (`boundary_class = 0`) first, incrementing
   `spool_overflow_dropped`.
2. **If the bound is reached and no capture-class row remains** — the spool is entirely
   boundary-class — Cairn stops accepting new events for that store and enters a
   `spool_saturated` state. It does **not** drop a boundary row: session open, close and
   compaction rows are what every other event is routed by, and shedding them would corrupt the
   session structure of everything already queued.

   In `spool_saturated`: capture continues to be attempted and each new event is recorded as
   `spool_saturated_dropped` with its kind and session, so the loss is counted rather than
   silent; the agent is still never blocked (FR-781); and the condition is surfaced by
   `cairn status` and in capture health as a distinct, actionable state. It clears as soon as
   delivery drains rows.

This is the one place the capacity policy can lose a boundary event, and it is resolved by
refusing new work rather than by corrupting queued work.

---

## 6. Server schema v4 (PostgreSQL)

```sql
CREATE TABLE safe_events (
  event_id         UUID PRIMARY KEY,          -- idempotency is the key itself
  project_id       UUID NOT NULL REFERENCES projects(id),
  session_id       UUID NOT NULL REFERENCES sessions(id),
  account_id       UUID NOT NULL REFERENCES users(id),   -- bound from credential
  agent            TEXT NOT NULL,
  kind             TEXT NOT NULL,
  vendor_event     TEXT,
  session_seq      BIGINT NOT NULL,
  contract_version INT  NOT NULL,
  content          JSONB NOT NULL,
  occurred_at      TIMESTAMPTZ NOT NULL,      -- advisory
  received_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (session_id, session_seq)
);
CREATE INDEX safe_events_project_time ON safe_events (project_id, received_at DESC);

-- The group that gets claimed must be a lockable ROW: PostgreSQL forbids a locking clause
-- with GROUP BY (SQLSTATE 0A000). Hence a lease table, one row per session.
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
CREATE INDEX consolidation_session_elect
  ON consolidation_session (oldest_enqueued_at) WHERE state <> 'done';

CREATE TABLE consolidation_work (
  event_id     UUID PRIMARY KEY REFERENCES safe_events(event_id),
  project_id   UUID NOT NULL,
  session_id   UUID NOT NULL,
  session_seq  BIGINT NOT NULL,           -- batch order; event_id is a UUID and orders arbitrarily
  enqueued_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  state        TEXT NOT NULL CHECK (state IN ('pending','done','failed')),
  attempts     INT NOT NULL DEFAULT 0, last_error TEXT,
  FOREIGN KEY (project_id, session_id) REFERENCES consolidation_session (project_id, session_id)
);
CREATE INDEX consolidation_pending
  ON consolidation_work (project_id, session_id, session_seq) WHERE state = 'pending';

CREATE TABLE consolidation_runs (
  run_id UUID PRIMARY KEY, project_id UUID NOT NULL, session_id UUID,
  started_at TIMESTAMPTZ NOT NULL, finished_at TIMESTAMPTZ,
  events_claimed INT, candidates_proposed INT, candidates_accepted INT,
  candidates_refused INT, extractor_kind TEXT NOT NULL, state TEXT NOT NULL
);

CREATE TABLE knowledge_candidates (
  candidate_id UUID PRIMARY KEY,     -- deterministic, §7 of contracts/consolidation.md
  run_id UUID NOT NULL REFERENCES consolidation_runs(run_id),
  project_id UUID NOT NULL,
  proposed_kind TEXT NOT NULL,       -- fact|decision|convention|failure|procedure
  proposed_domain TEXT NOT NULL,
  topic_key TEXT, value_key TEXT,    -- AFTER Cairn normalization, never as proposed
  content TEXT NOT NULL,
  decision TEXT NOT NULL CHECK (decision IN
    ('accepted','reinforced','duplicate','conflicted','refused')),
  refusal_reason TEXT,               -- fixed vocabulary, FR-804a
  -- KnowledgeRef of the record this candidate became or reinforced (§6.1)
  result_ref_kind     TEXT CHECK (result_ref_kind IN ('knowledge','pattern')),
  result_domain       TEXT CHECK (result_domain IN ('project','personal','team')),
  result_knowledge_id UUID,   -- KnowledgeRef id, or the pattern_id when ref_kind='pattern'
  CHECK ((result_ref_kind IS NULL AND result_domain IS NULL AND result_knowledge_id IS NULL)
      OR (result_ref_kind = 'knowledge' AND result_domain IS NOT NULL
                                         AND result_knowledge_id IS NOT NULL)
      OR (result_ref_kind = 'pattern'   AND result_domain IS NULL
                                         AND result_knowledge_id IS NOT NULL)),
  UNIQUE (run_id, topic_key, value_key)
);

CREATE TABLE candidate_source_events (
  candidate_id UUID NOT NULL REFERENCES knowledge_candidates(candidate_id),
  event_id     UUID NOT NULL REFERENCES safe_events(event_id),
  PRIMARY KEY (candidate_id, event_id)
);

CREATE TABLE retrieval_traces (
  trace_id UUID PRIMARY KEY,
  project_id UUID NOT NULL, session_id UUID NOT NULL, account_id UUID NOT NULL,
  trigger TEXT NOT NULL,             -- session_open | prompt_submit | explicit
  delivery_point TEXT NOT NULL,
  degradation_level TEXT,            -- NULL only while requested or on pre-generation failure
  budget_tokens INT, budget_spent INT,
  latency_ms INT,                     -- NULL only while requested
  delivery_state TEXT NOT NULL CHECK (delivery_state IN
    ('requested','generated','transmitted','failed')),
  acknowledgement_state TEXT NOT NULL DEFAULT 'unavailable' CHECK
    (acknowledgement_state IN ('unavailable','acknowledged')),
                                      -- baseline 005 has no producer for acknowledged
  failure_reason TEXT,
  transmission_reported_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK ((delivery_state = 'requested' AND latency_ms IS NULL)
      OR (delivery_state <> 'requested' AND latency_ms IS NOT NULL)),
  CHECK ((delivery_state IN ('generated','transmitted') AND degradation_level IS NOT NULL)
      OR delivery_state IN ('requested','failed')),
  CHECK ((delivery_state = 'failed' AND failure_reason IS NOT NULL)
      OR (delivery_state <> 'failed' AND failure_reason IS NULL))
);

-- Retention (FR-847): retrieval_traces and their items are retained 90 days, then deleted
-- oldest-first by a bounded sweep in the same background task as consolidation. Trace volume
-- is therefore bounded by traffic x 90 days, not unbounded.

CREATE TABLE retrieval_trace_items (
  trace_id      UUID NOT NULL REFERENCES retrieval_traces(trace_id) ON DELETE CASCADE,
  -- KnowledgeRef: (domain, id). Project, personal and team knowledge live in
  -- DIFFERENT tables, so a bare memory_id cannot name a personal or team record.
  ref_kind      TEXT NOT NULL CHECK (ref_kind IN ('knowledge','pattern')),
  domain        TEXT CHECK (domain IN ('project','personal','team')),  -- NULL iff pattern
  knowledge_id  UUID NOT NULL,                       -- knowledge id, or pattern_id
  reference_key TEXT GENERATED ALWAYS AS (
    CASE WHEN ref_kind = 'knowledge'
         THEN 'knowledge:' || domain || ':' || knowledge_id::text
         ELSE 'pattern:' || knowledge_id::text END
  ) STORED NOT NULL,
  status        TEXT NOT NULL CHECK (status IN ('considered','selected')),
  selection_rule TEXT, rank INT,
  source_updated_at TIMESTAMPTZ NOT NULL,  -- copied from referenced row for delivery upsert
  CHECK ((ref_kind = 'knowledge' AND domain IS NOT NULL)
      OR (ref_kind = 'pattern'   AND domain IS NULL)),
  PRIMARY KEY (trace_id, reference_key)
);

CREATE TABLE verification_reports (        -- runs reported, never states asserted
  report_id     UUID PRIMARY KEY,            -- SERVER-assigned
  ref_kind      TEXT NOT NULL CHECK (ref_kind IN ('knowledge','pattern')),
  domain        TEXT CHECK (domain IN ('project','personal','team')),  -- NULL iff pattern
  knowledge_id  UUID NOT NULL,               -- KnowledgeRef id, or pattern_id
  reference_key TEXT GENERATED ALWAYS AS (
    CASE WHEN ref_kind = 'knowledge'
         THEN 'knowledge:' || domain || ':' || knowledge_id::text
         ELSE 'pattern:' || knowledge_id::text END
  ) STORED NOT NULL,
  project_id    UUID,                        -- NULLABLE: personal/team are project-independent
  owner_user_id UUID,                        -- set for personal knowledge and PatternRef
  account_id    UUID NOT NULL,               -- the reporting account, from the credential
  verdict       TEXT NOT NULL CHECK (verdict IN ('passed','failed','inconclusive')),
  verifier_kind TEXT NOT NULL,
  authority     TEXT NOT NULL,               -- SERVER-assigned; never from the payload
  run_at        TIMESTAMPTZ NOT NULL,
  received_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK ((ref_kind = 'knowledge' AND domain IS NOT NULL)
      OR (ref_kind = 'pattern'   AND domain IS NULL)),
  -- Same authenticated account retrying the same logical run is a duplicate. Different
  -- accounts reporting the same record are distinct reports.
  UNIQUE (reference_key, account_id, verifier_kind, run_at)
);

-- Verification columns ALREADY EXIST on the server `memories` table and are NOT re-added:
--   verification, verification_authority, last_verified_at, verification_basis,
--   evidence_fact_count   (0002_project_intelligence.sql:31-35).
-- `verification` is the canonical state column; there is no `verification_state`.
-- Feature 005 changes who computes them, not what they are called
-- (contracts/verification-summary.md §1).
ALTER TABLE memories
  ADD COLUMN origin_kind TEXT;   -- explicit | consolidated  (FR-816)

-- Reusable patterns (FR-708/708a/708b, SC-738). The LOCAL representation is refused by the
-- synchronization boundary and stays refused: `reusable_pattern` is a FORBIDDEN_ENTITY_TYPE,
-- and its rows carry `signal_digest`, `origin_ref` and `sanitization_report`, all refused
-- field names. This is the REPLACEMENT safe shape, and it is a different record — the local
-- one is not relaxed, it is not sent.
CREATE TABLE shared_patterns (
  -- Deterministic, privacy-safe identity: see §6.2. NOT a random UUID, because promotion must
  -- be idempotent across retries and migration re-runs.
  pattern_id    UUID PRIMARY KEY,
  domain        TEXT NOT NULL DEFAULT 'personal' CHECK (domain = 'personal'),
  owner_user_id UUID NOT NULL REFERENCES users(id),   -- owner; visibility is owner-only (§6.2)
  title         TEXT NOT NULL,
  problem       TEXT NOT NULL,
  root_cause    TEXT NOT NULL,
  approach      TEXT NOT NULL,
  constraints   JSONB NOT NULL DEFAULT '[]',
  applicability JSONB NOT NULL DEFAULT '[]',          -- language/tool vocabulary only
  trust         TEXT NOT NULL DEFAULT 'sanitized' CHECK (trust IN ('sanitized')),
                -- Server-side trust is deliberately single-valued: see §6.2.
  topic_key     TEXT, value_key TEXT,                 -- normalized, for reconciliation
  content_key   TEXT NOT NULL,                       -- privacy-safe duplicate identity, §6.2
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  forgotten_at  TIMESTAMPTZ
);
CREATE UNIQUE INDEX shared_patterns_identity ON shared_patterns (owner_user_id, content_key);
CREATE INDEX shared_patterns_search
  ON shared_patterns USING GIN (to_tsvector('english', problem || ' ' || approach));

-- Deliberately absent, and why:
--   signals, signal_digest   -- the digest is a refused name; raw signals are local evidence
--   origin_ref               -- refused name; the source project must not be nameable
--   sanitization_report      -- refused name; the report is local diagnostic output
--   source_memory_id         -- names a project memory, which a project-independent record
--                            -- must not do
--   origin_deleted           -- a fact about a local record that no longer travels
-- A pattern is a project-independent PERSONAL-domain record of type pattern. It inherits that
-- domain's rules: it MUST NOT name the project it came from, and its origin stays a
-- machine-local salted digest that is never transmitted (FR-708a/FR-708c/FR-819).

-- Verification summaries for the non-project domains. `memories` has its own five columns;
-- personal_knowledge, team_knowledge and shared_patterns do not, and gain none.
CREATE TABLE knowledge_verification (
  ref_kind      TEXT NOT NULL CHECK (ref_kind IN ('knowledge','pattern')),
  domain        TEXT CHECK (domain IN ('personal','team')),  -- NULL iff ref_kind='pattern'
  knowledge_id  UUID NOT NULL,   -- knowledge id, or pattern_id
  reference_key TEXT GENERATED ALWAYS AS (
    CASE WHEN ref_kind = 'knowledge'
         THEN 'knowledge:' || domain || ':' || knowledge_id::text
         ELSE 'pattern:' || knowledge_id::text END
  ) STORED NOT NULL,
  verification  TEXT NOT NULL DEFAULT 'unverified',
  verification_authority TEXT,
  verification_basis     JSONB NOT NULL DEFAULT '[]',
  evidence_fact_count    INTEGER NOT NULL DEFAULT 0,
  last_verified_at       TIMESTAMPTZ,
  CHECK ((ref_kind = 'knowledge' AND domain IS NOT NULL)
      OR (ref_kind = 'pattern'   AND domain IS NULL)),
  PRIMARY KEY (reference_key)
);

CREATE TABLE legacy_verification_audit (   -- pre-cutover values, untrusted, never derived from
  domain       TEXT NOT NULL,
  knowledge_id UUID NOT NULL,
  legacy_state TEXT, legacy_authority TEXT, legacy_last_verified_at TIMESTAMPTZ,
  demoted_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (domain, knowledge_id)
);

-- The closed capability vocabulary FR-728/FR-855/SC-726 require. Health is declared per agent
-- AND per canonical event kind, so a cell is never blank.
--   capability = 'event:<kind>'      for each of the 21 kinds (capture)
--              | 'deliver:session_open' | 'deliver:prompt_time'
--              | 'deliver:post_compaction' | 'receipt'
-- status uses the six-value vocabulary below; 'no_evidence' is a first-class answer.
CREATE TABLE integration_health (
  project_id UUID NOT NULL, account_id UUID NOT NULL,
  writer_id  TEXT NOT NULL,                  -- the machine it was observed on (FR-857)
  agent TEXT NOT NULL,
  capability TEXT NOT NULL, stage TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN
    ('supported','unsupported_by_vendor','declined_by_cairn',
     'adapter_unimplemented','runtime_failure','no_evidence')),
  evidence_kind TEXT,                        -- introspection | observation
  observed_at TIMESTAMPTZ, degraded BOOLEAN,
  PRIMARY KEY (project_id, account_id, writer_id, agent, capability, stage)
);

CREATE TABLE delivered_context (           -- per-session dedup; server-side, §7
  session_id     UUID NOT NULL REFERENCES sessions(id),
  ref_kind       TEXT NOT NULL CHECK (ref_kind IN ('knowledge','pattern')),
  domain         TEXT CHECK (domain IN ('project','personal','team')),  -- NULL iff pattern
  knowledge_id   UUID NOT NULL,                      -- knowledge id, or pattern_id
  reference_key  TEXT GENERATED ALWAYS AS (
    CASE WHEN ref_kind = 'knowledge'
         THEN 'knowledge:' || domain || ':' || knowledge_id::text
         ELSE 'pattern:' || knowledge_id::text END
  ) STORED NOT NULL,
  delivered_at   TIMESTAMPTZ NOT NULL,
  source_updated_at TIMESTAMPTZ NOT NULL,   -- the record's updated_at when delivered
  delivery_point TEXT NOT NULL,
  CHECK ((ref_kind = 'knowledge' AND domain IS NOT NULL)
      OR (ref_kind = 'pattern'   AND domain IS NULL)),
  PRIMARY KEY (session_id, reference_key)
);

CREATE TABLE capture_dispositions (        -- funnel source; client-reported + server-observed
  project_id UUID NOT NULL, account_id UUID NOT NULL, agent TEXT NOT NULL,
  kind TEXT NOT NULL, disposition TEXT NOT NULL, day DATE NOT NULL,
  n BIGINT NOT NULL DEFAULT 0,
  PRIMARY KEY (project_id, account_id, agent, kind, disposition, day)
);

CREATE TABLE server_authority (
  id INT PRIMARY KEY CHECK (id = 1),
  mode TEXT NOT NULL CHECK (mode IN ('pre_cutover','server_authoritative')),
  cutover_at TIMESTAMPTZ
);
```

`integration_health.status` is the FR-855 vocabulary, and `evidence_kind` is what keeps
configuration read-back from reading as runtime capture (FR-852). `no_evidence` is a first-class
value, which is what lets receipt be reported honestly (FR-838e).

---

## 6.1 `KnowledgeRef` — naming knowledge across domains

Project knowledge lives in `memories`, personal knowledge in `personal_knowledge`, team
knowledge in `team_knowledge`. They are separate tables with separate ownership rules, so a
bare `memory_id` cannot name a personal or team record, and `memories.account_id` does not
exist as a way to check who owns one.

Every cross-domain reference is therefore a **`KnowledgeRef = (domain, knowledge_id)`** with
`domain ∈ {project, personal, team}` — the three domains Constitution IV fixes, and no others.
It is used identically in retrieval traces, delivered-context dedup, authorization and web
rendering.

**A reusable pattern is not a fourth domain and is not domain-less.** Its canonical
`shared_patterns` row carries `domain = personal`; `pattern` is the record type. It remains a
distinct record type with its own table and lifecycle, and is referenced as
**`PatternRef = pattern_id`**. Thus `PatternRef` omits a domain component without erasing the
canonical record's personal domain.

Where a table must reference either, it carries a discriminator rather than a widened domain:

```
ref_kind  TEXT NOT NULL CHECK (ref_kind IN ('knowledge','pattern'))
domain    TEXT     CHECK (domain IN ('project','personal','team'))  -- NULL iff ref_kind='pattern'
record_id UUID NOT NULL
reference_key TEXT GENERATED ALWAYS AS (
  CASE WHEN ref_kind = 'knowledge'
       THEN 'knowledge:' || domain || ':' || record_id::text
       ELSE 'pattern:' || record_id::text END
) STORED NOT NULL
CHECK ((ref_kind = 'knowledge' AND domain IS NOT NULL)
    OR (ref_kind = 'pattern'   AND domain IS NULL))
```

Here `domain IS NULL` means only that the polymorphic row contains a `PatternRef` rather than a
`KnowledgeRef`. Resolving that `PatternRef` MUST find a `shared_patterns` row whose own domain is
explicitly `personal`. Every concrete table using this shape repeats the `CHECK`; application
validation alone is insufficient (SC-766). Wherever the reference participates in row identity,
the generated non-null `reference_key` participates in the primary or unique key. Thus equal
UUIDs in different knowledge domains remain different identities, while `PatternRef(id)` has
its own `pattern:<id>` identity (FR-819a, SC-767).

**A memory relation is not a `KnowledgeRef` either.** A relation is identified by its own natural
key, `RelationRef = (from_memory_id, to_memory_id, kind)`, which is the primary key
`memory_relations` already has. It has no `knowledge_id` of its own and must not be given one.

| Domain | Table | Owner check for a reader |
|---|---|---|
| `project` | `memories` | reader is a member of `memories.project_id` |
| `personal` | `personal_knowledge`; `shared_patterns` (type `pattern`) | reader **is** the row's `owner_user_id` |
| `team` | `team_knowledge` | reader is a member of the server's team; `proposed` rows additionally require author-or-admin |

A `PatternRef` resolves against `shared_patterns`, whose visibility rule is in §6.2.

Two rules follow, both binding:

- **`updated_at` comparisons resolve per domain**, against the referenced table's own column.
  There is no single table to read it from.
- **Authorization resolves per domain before a reference is rendered.** A `personal` ref whose
  owner is not the reader is withheld entirely — not rendered as an opaque id, which would
  still disclose that the record exists. A project member must never be able to infer a
  colleague's personal knowledge from a trace identifier, a rank gap, or a count.

## 6.2 Reusable patterns — visibility, identity and trust

### Visibility is owner-only by default

A local reusable pattern is **one developer's** cross-project knowledge. FR-708 asks for
durability, not for an audience. Making a promoted pattern readable by every authenticated
account would take a private record and publish it to the whole server as a side effect of
backing it up — a widening no requirement asks for, and one Constitution V forbids as a side
effect (*"moving knowledge from one domain to a wider one is always an explicit act, never a
side effect"*).

So `shared_patterns` carries `domain = personal`, is scoped to `owner_user_id`, and reads,
retrieval, traces and web views all filter on it. The table name describes where it is stored,
not who can see it.

Widening to a team is a **separate, explicit act with its own governance**: the owner proposes
the content as a new team-domain record, and a human administrator ratifies it, exactly as any
other team guidance (Product Constraints: *"an agent may propose; only a human administrator
may make team-wide guidance authoritative"*). The personal pattern remains owner-only. There
is no pattern-specific sharing path, and pattern visibility is never the mechanism by which
something becomes team-wide.

### Identity is derived from safe content

Local duplicate identity is `signal_digest + root_cause_digest`, and the safe shape carries
neither — both are refused field names. A random UUID cannot replace them: promotion must be
idempotent across a retry and a migration re-run, or the same pattern lands twice.

```
content_key = digest( normalize(problem) ‖ normalize(root_cause) ‖ normalize(approach) )
pattern_id  = UUIDv5(CAIRN_PATTERN_NS, owner_user_id ‖ content_key)
```

Every input is a field that legitimately crosses the boundary, so the identity discloses
nothing the record does not already carry. `UNIQUE (owner_user_id, content_key)` makes a repeat
promotion an upsert rather than a duplicate, and makes migration re-runnable.

Two patterns that differ only in `title` collapse to one, which is correct: the title is a
label, and the problem, cause and approach are the pattern.

### Server trust is narrowed to one value

Local `trust` is `candidate | sanitized | validated | contested`, and `validated` and
`contested` are derived from `pattern_applications` — which stay local-only (FR-707). The server
therefore has **no evidence** from which to derive them, and a client asserting `validated`
would be asserting a state it earned privately, on a record the server cannot check. That is the
same class of overclaim as client-asserted verification.

So the server stores exactly one trust value, `sanitized`, meaning what it can actually
establish: this record passed the privacy gate. `validated` and `contested` remain **local**
states, derived locally from local applications, displayed locally, and labelled as
machine-local rather than canonical.

This is a deliberate narrowing rather than an omission. If server-side trust is wanted later it
needs an evidence path — application outcomes reported as runs, the way verification works — and
that is a separate feature, not an implicit consequence of storing patterns centrally.

## 7. Per-session context de-duplication

`delivered_context` is a **server** table, because selection is server-side and the server is
the only party holding both sides of the comparison. Placing it on the client would make the
rule uncomputable. It records what a session already received; prompt-time selection is:

```
relevant(session, prompt)
  MINUS delivered_context[session]
  PLUS  any delivered item whose updated_at > its delivered_at
```

Session-open uses the full briefing budget. Prompt-time uses an incremental budget of **25% of
the briefing budget** and carries only new or changed items, so the two delivery points cannot
restate each other (FR-829, FR-830). Both write a trace with their `delivery_point`.

A row is written **when transmission is attempted and did not fail**, never at selection time.
Writing at selection would suppress, for the life of the session, items the agent never
received — dedup withholding what was only ever generated.

The server inserts `retrieval_traces(delivery_state='requested')` after authenticated
account/project/session binding and before selection begins. Generation changes it to `generated`
or `failed`. A later authenticated transmission-outcome report names only the server-issued
`trace_id` plus the bounded outcome. In one transaction, `generated → transmitted` upserts the
trace's selected `retrieval_trace_items` into `delivered_context`; `generated → failed` records a
content-free failure reason and writes no delivery rows. Repeating the same terminal outcome is a
no-op success; a conflicting terminal outcome is refused. Baseline 005 always leaves
`acknowledgement_state = 'unavailable'`: returning context establishes transmission, not receipt.

Because `delivered_context` is server-side, deleting the local store does not cause a session's
knowledge to be re-delivered, and does not appear in the durability-loss list.

---

## 8. Entity relationships

```
SafeCanonicalEvent ──▶ consolidation_work ──▶ ConsolidationRun
                                                    │
                                                    ▼
                                          KnowledgeCandidate
                                                    │
                       result = KnowledgeRef(domain, id)  |  PatternRef(pattern_id)
                                                    │
              ┌──────────────┬──────────────┬───────┴────────┐
              ▼              ▼              ▼                ▼
          memories   personal_knowledge  team_knowledge  shared_patterns
             project       personal          team        personal/type=pattern
              │              │              │                │
              └──────────────┴──────┬───────┴────────────────┘
                                    │  referenced by (ref_kind, domain?, id)
                                    │  identity = generated reference_key
              ┌─────────────────────┼─────────────────────┐
              ▼                     ▼                     ▼
    retrieval_trace_items   delivered_context     verification_reports
              │                                           │
              ▼                                     (derive state)
       RetrievalTrace

candidate_source_events ──▶ SafeCanonicalEvent   (evidence, additive)
memory_relations                                 (project domain only)
```
