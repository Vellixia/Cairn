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
| `user_instruction_signal` | *(none)* — the signal only. Carrying prompt text would be a transcript. |
| `decision_signal` | *(none)* — same reason. The claim is extracted from surrounding events. |
| `capture_declined`, `capture_failed` | `disposition` enum (§4), `stage` enum |
| `agent_quiesced` | *(none)* |

**No free text derived from user or assistant messages appears anywhere in this table.**
`failure_note`, `command_line` and `test_command` are tool-level strings that pass redaction
first, and are the only free-text fields in the model.

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
`privacy_refused`, `spooled`, `spool_overflow_dropped`, `transmitted`, `accepted`,
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
);

CREATE TABLE authority_mode (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  mode TEXT NOT NULL CHECK (mode IN ('feature_004','migrating','server_authoritative')),
  changed_at TEXT NOT NULL
);

CREATE TABLE migration_state (
  phase TEXT PRIMARY KEY, state TEXT NOT NULL, detail_count INTEGER,
  started_at TEXT, finished_at TEXT
);
```

Spool overflow (FR-785): drop the **oldest capture-class** rows first; never drop a row with
`boundary_class = 1` (session open/close, compaction), because those route everything else.
Each drop increments `spool_overflow_dropped`.

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

CREATE TABLE consolidation_work (
  event_id     UUID PRIMARY KEY REFERENCES safe_events(event_id),
  project_id   UUID NOT NULL,
  session_id   UUID NOT NULL,
  state        TEXT NOT NULL CHECK (state IN ('pending','claimed','done','failed')),
  claim_owner  UUID, claim_expires_at TIMESTAMPTZ,
  attempts     INT NOT NULL DEFAULT 0, last_error TEXT
);
CREATE INDEX consolidation_pending ON consolidation_work (state, claim_expires_at);

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
  memory_id UUID,                    -- set when accepted or reinforced
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
  degradation_level TEXT NOT NULL,   -- FR-836 declared levels
  budget_tokens INT, budget_spent INT,
  latency_ms INT NOT NULL,
  delivery_state TEXT NOT NULL,      -- generated|transmitted|acknowledged|unavailable|failed
  failure_reason TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Retention (FR-847): retrieval_traces and their items are retained 90 days, then deleted
-- oldest-first by a bounded sweep in the same background task as consolidation. Trace volume
-- is therefore bounded by traffic x 90 days, not unbounded.

CREATE TABLE retrieval_trace_items (
  trace_id UUID NOT NULL REFERENCES retrieval_traces(trace_id),
  memory_id UUID NOT NULL, domain TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('considered','selected')),
  selection_rule TEXT, rank INT,
  PRIMARY KEY (trace_id, memory_id)
);

CREATE TABLE verification_reports (        -- runs reported, never states asserted
  report_id UUID PRIMARY KEY,
  memory_id UUID NOT NULL, project_id UUID NOT NULL, account_id UUID NOT NULL,
  verdict TEXT NOT NULL CHECK (verdict IN ('passed','failed','inconclusive')),
  verifier_kind TEXT NOT NULL, authority TEXT NOT NULL,
  run_at TIMESTAMPTZ NOT NULL, received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE memories
  ADD COLUMN verification_state     TEXT,   -- DERIVED from verification_reports
  ADD COLUMN verification_authority TEXT,
  ADD COLUMN last_verified_at       TIMESTAMPTZ,
  ADD COLUMN origin_kind            TEXT;   -- explicit | consolidated  (FR-816)

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
  session_id UUID NOT NULL REFERENCES sessions(id),
  memory_id  UUID NOT NULL,
  delivered_at   TIMESTAMPTZ NOT NULL,
  delivery_point TEXT NOT NULL,
  PRIMARY KEY (session_id, memory_id)
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

Because `delivered_context` is server-side, deleting the local store does not cause a session's
knowledge to be re-delivered, and does not appear in the durability-loss list.

---

## 8. Entity relationships

```
SafeCanonicalEvent ──▶ consolidation_work ──▶ ConsolidationRun
                                                    │
                                                    ▼
                                          KnowledgeCandidate ──▶ Memory
                                                    │              │
                                        candidate_source_events    │
                                                    │              ▼
                                          SafeCanonicalEvent   memory_relations
                                                                   │
Memory ◀── retrieval_trace_items ──▶ RetrievalTrace                │
   ▲                                                               │
   └── verification_reports (derive state) ────────────────────────┘
```
