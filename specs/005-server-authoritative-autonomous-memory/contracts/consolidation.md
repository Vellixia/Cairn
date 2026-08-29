# Contract — Consolidation

Turning accepted safe events into durable knowledge, without anyone asking, inside the server
process that already exists.

## 1. Placement

An in-process Tokio task in `cairn-server`. Not a new service, not a broker, not a distributed
worker platform, and **not client-driven** (FR-793a).

Client-driven consolidation was rejected on architecture, not cost: it would return the
decision about what becomes durable knowledge to the edge, which is the arrangement this
feature exists to end. It would also reintroduce a second place where knowledge is decided,
which is precisely what moving authority to the server removes.

Constitution II permits this: deferred work — *"consolidating captured activity into
knowledge, draining a queue"* — is ordinary engineering inside the existing processes, provided
it is bounded, observable and restartable. §6 supplies the bounds, §8 the observability, §4 the
restartability.

## 2. Pipeline

```
safe_events ──▶ consolidation_work ──▶ claim batch ──▶ ExtractionInput
                                                            │
                                                            ▼
                                                   CandidateProposal[]
                                                            │
                              ┌─── Cairn governance (§5) ───┘
                              ▼
        normalize keys → privacy gate → domain/scope → dedup → reconcile → persist
```

## 3. Batching

A batch is **one session's** pending events, up to 200, in `session_seq` order. Session-scoped
because a claim about a piece of work is derived from a sequence of acts within one session,
and ordering within a session is what makes the rules in `extraction.md` meaningful.

A session's events are consolidated when either: the session has closed, or 200 events have
accumulated, or the oldest pending event is older than 10 minutes. The last condition is what
makes a long-running session produce knowledge before it ends.

## 4. Claim, reclaim, restart

A `GROUP BY` cannot be combined with a locking clause. PostgreSQL rejects it outright —
verified on 18.6:

```
ERROR:  0A000: FOR UPDATE is not allowed with GROUP BY clause
```

The documentation states the rule generally: *"The locking clauses cannot be used in contexts
where returned rows cannot be clearly identified with individual table rows; for example they
cannot be used with aggregation"*
([SELECT, PostgreSQL 18](https://www.postgresql.org/docs/18/sql-select.html)). The same
restriction applies to `DISTINCT`, `HAVING`, window functions and set operations, and it is not
version-dependent — the sentence appears from 9.4 through 18.

So the group to claim must itself be a lockable **row**. A lease table gives that.

```sql
CREATE TABLE consolidation_session (
  project_id         UUID NOT NULL,
  session_id         UUID NOT NULL,
  state              TEXT NOT NULL DEFAULT 'pending',   -- pending | claimed | done
  claimed_by         TEXT,
  claim_expires_at   TIMESTAMPTZ,
  oldest_enqueued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (project_id, session_id)
);
CREATE INDEX ON consolidation_session (oldest_enqueued_at) WHERE state <> 'done';
```

A row is upserted here in the same transaction that enqueues an event into
`consolidation_work`, and the conflict action is stated because both obvious choices are wrong:

```sql
INSERT INTO consolidation_session (project_id, session_id, state, oldest_enqueued_at)
VALUES ($p, $s, 'pending', now())
ON CONFLICT (project_id, session_id) DO UPDATE
   SET state = CASE WHEN consolidation_session.state = 'done'
                    THEN 'pending'                       -- re-open a finished session
                    ELSE consolidation_session.state      -- never disturb a live claim
               END,
       oldest_enqueued_at = LEAST(consolidation_session.oldest_enqueued_at,
                                  EXCLUDED.oldest_enqueued_at);
```

Leaving `state` alone unconditionally would strand every event arriving after a session was
marked `done`: the partial index excludes `done`, so the session would never be elected again.
Setting `state = 'pending'` unconditionally would clobber a live `claimed` lease and let a
second worker elect a session mid-pass. The `CASE` re-opens only a finished session.

```sql
BEGIN;

-- 1. elect and claim exactly ONE session
UPDATE consolidation_session s
   SET state = 'claimed', claimed_by = $worker,
       claim_expires_at = now() + interval '5 minutes'
 WHERE (s.project_id, s.session_id) = (
         SELECT project_id, session_id
           FROM consolidation_session
          WHERE state = 'pending'
             OR (state = 'claimed' AND claim_expires_at < now())   -- reclaim
          ORDER BY oldest_enqueued_at
          FOR UPDATE SKIP LOCKED
          LIMIT 1)
RETURNING s.project_id, s.session_id;

-- 2. read that session's work, in order. No lock needed: the group is owned.
SELECT event_id, session_seq
  FROM consolidation_work
 WHERE project_id = $p AND session_id = $s AND state = 'pending'
 ORDER BY session_seq
 LIMIT 200;

COMMIT;   -- row lock released; the lease stands as committed state
```

On success the worker marks the events it consolidated `done`, then closes the session **only
if nothing remains**:

```sql
UPDATE consolidation_work SET state = 'done'
 WHERE project_id = $p AND session_id = $s AND event_id = ANY($consolidated);

UPDATE consolidation_session SET state = 'done', claimed_by = NULL, claim_expires_at = NULL
 WHERE project_id = $p AND session_id = $s AND claimed_by = $worker
   AND NOT EXISTS (SELECT 1 FROM consolidation_work
                    WHERE project_id = $p AND session_id = $s AND state = 'pending');
```

The `NOT EXISTS` guard is what makes the 200-event batch cap safe. A session with 500 pending
events would otherwise be closed after the first batch and the remaining 300 stranded behind
the partial index. With the guard, the session returns to `pending` — the `claimed_by` clause
having released the lease — and is re-elected until it is genuinely drained.

`consolidation_work.state` moves `pending → done` here, `pending → failed` after five attempts
(§4). Nothing else writes it, so the `consolidation_pending` partial index drains.

A pass longer than the lease extends `claim_expires_at` as a heartbeat.

Why each guarantee holds:

- **One session per batch** — the lease table has exactly one row per session, so `LIMIT 1`
  elects exactly one group. No aggregation appears anywhere, so `0A000` cannot arise.
- **No two workers on the same work** — `FOR UPDATE SKIP LOCKED` excludes concurrent claimers
  for the duration of the claiming transaction, and the `state` flip commits inside that window,
  so afterwards the election predicate itself excludes them. `SKIP LOCKED` is documented for
  exactly this: *"can be used to avoid lock contention with multiple consumers accessing a
  queue-like table."*
- **Ordering by `session_seq`** — step 2 is a plain `ORDER BY session_seq`. Ordering by
  `event_id` would be arbitrary: it is a UUIDv5.
- **Stale claims reclaimable** — the expiry predicate re-elects a session whose worker died
  mid-batch.
- **Restart-safe** — every piece of claim state is a committed row, not a lock. Advisory locks
  were rejected for this reason: they vanish on backend crash or server restart and cannot
  express a lease.
- **`LIMIT` is safe here** — *"If a `LIMIT` is used, locking stops once enough rows have been
  returned to satisfy the limit."* `OFFSET` is deliberately not used, since rows skipped by
  `OFFSET` would still be locked.

**Restart semantics, stated exactly (FR-793b):**

- No **completed** work is lost — `state='done'` is durable.
- An abandoned claim **is** reclaimed and **re-executed**. This is expected, not a defect.
- Re-execution produces **no duplicate durable effect**, because candidate identity (§7) is
  derived from the event set and the normalized keys, so persistence is an upsert.

The phrase "a restart repeats none of it" is wrong and is not used: the reclaim mechanism
requires repetition. What must not repeat is the *effect*.

- After `attempts >= 5`, an event moves to `failed` with `last_error`, becomes visible in
  system health, and stops being retried (FR-808 leaves input reprocessable up to that point).
- A failed batch persists **nothing** — extraction, governance and persistence run in one
  transaction, so a partial candidate cannot exist (FR-808).

## 5. Governance — what Cairn decides, never the extractor

Each proposal passes every gate, in this fixed order. Failing any gate refuses the candidate
and persists nothing for it.

| # | Gate | Rule |
|---|---|---|
| 1 | Source verification | Every cited `event_id` must exist, belong to this batch's project and session, and have been accepted. Otherwise `unverifiable_source` (FR-805c). |
| 2 | Key normalization | `topic_key`/`value_key` normalized by Cairn's deterministic function. A key that fails validation refuses the candidate; it is **never repaired** into a plausible one, because repair changes which existing knowledge the candidate collides with (FR-796a–c). |
| 3 | Privacy | The same deterministic checks and the same single implementation of each rejection class that govern any other content (FR-759, FR-760). |
| 4 | Domain and scope | Resolved by Cairn from the project and session context. The extractor's proposed domain is advisory; scope is never taken from the extractor at all (FR-805b). |
| 5 | Ownership | Personal knowledge takes its owner from the `account_id` the server bound at ingest — never from event content, never from extractor output (FR-810a, Principle XI). |
| 5a | Key ↔ evidence correspondence | Cairn MUST be able to re-derive the proposed key pair from the cited source events using its own rules. A proposal whose keys Cairn cannot re-derive is refused (`key_not_derivable`). Without this the extractor chooses which existing record gets reinforced — a well-formed proposal whose keys happen to match a high-value record would produce a durable reinforcement that a null extractor would not, which is precisely the difference SC-742 measures. |
| 6 | Duplicate / reinforcement | §6 of `extraction.md`; identity match on normalized keys. |
| 7 | Conflict | Same `topic_key`, overlapping scope, different `value_key` ⇒ `conflicts_with`, basis `deterministic_rule`. Never auto-resolved (FR-799). |
| 8 | Supersession | **Never automatic.** Consolidation may not supersede anything (FR-800). |
| 9 | Verification | Never asserted by consolidation. State is derived from run reports only (FR-811, `verification-summary.md`). |
| 10 | Team | Consolidation may produce a team **proposal** only. Ratification stays a human administrator's act (FR-809). |

### 5.1 Attribution of a caller-less process

Consolidation has no authenticated caller. Principle XI requires that a process acting without
one is attributed to itself and never to a user whose data it happened to read.

- `memories.origin_kind = 'consolidated'` distinguishes it from explicit creation (FR-816).
- A team proposal produced by consolidation is attributed to the automatic process, is
  distinguishable from a human proposer, and does **not** appear in any account's "my pending
  proposals" view as that account's work (FR-809a).
- Personal knowledge ownership comes from the ingest-time account binding (gate 5).

## 6. Resource bounds

Consolidation shares a process and a connection pool with request serving, so FR-814's
prohibition on back-pressure is not achievable by intention — only by limits (FR-793a1).

| Bound | Value |
|---|---|
| Concurrent consolidation tasks | 1 |
| Batch size | 200 events |
| Connection-pool share | `min(2, floor(max_connections / 5))`, and consolidation does not run at all below `max_connections = 5` |
| Claim lease | 5 minutes |
| Yield between batches | 100 ms |
| Max attempts | 5 |

The share is a **fraction with a floor**, not a fixed 2. `max_connections` is operator-set
(`CAIRN_SERVER_MAX_CONNECTIONS`, default 10 at `crates/cairn-server/src/db.rs:29`) and the e2e
suite deliberately runs servers with small pools; a fixed 2 would take two thirds of a pool of
3, which is the starvation FR-793a1, FR-814 and SC-740 exist to prevent.

Ingestion never waits on consolidation: the two share only the pool, and consolidation's share
is capped below it. A backlog is never reported to a client as an ingestion failure (FR-814).

## 7. Candidate identity — the idempotency mechanism

```
candidate_id = UUIDv5(CAIRN_CANDIDATE_NS,
                      project_id ‖ session_id ‖ topic_key ‖ value_key)
```

Keys are the **normalized** ones, so a syntactic variant cannot produce a second candidate.
Persistence is `INSERT … ON CONFLICT (candidate_id) DO NOTHING`.

**The source event set is deliberately not part of the identity.** It is not stable across a
re-execution: a reclaim after the lease expires sweeps in events that arrived meanwhile, and an
event that exhausts its attempts leaves the batch. Including the evidence set would give the
re-executed batch a different `candidate_id`, the upsert would not fire, and a second candidate
with the same key pair would reinforce again — producing exactly the second corroboration
record and second relation that FR-798b, FR-797, SC-703 and SC-739 forbid.

Evidence is recorded separately and additively: `candidate_source_events` is a union, so a
re-execution that saw more events adds rows there without changing which candidate they belong
to. `knowledge_candidates.run_id` records which run first created it; a later run that
re-derives the same candidate updates evidence, never identity.

## 8. Observability

`consolidation_runs` records, per pass: events claimed, candidates proposed, accepted,
refused, the refusal reasons, the extractor kind, and timings (FR-807). A pass is described by
**the set of events it claimed**, not a contiguous range — an interruption leaves the
consolidated set non-contiguous.

Reportable at any time, including mid-pass and immediately after a restart (FR-793c, SC-748):
backlog depth, oldest outstanding event, failure count.

## 9. Refusal vocabulary

Distinct from the event-rejection vocabulary (FR-804a): `key_normalization_failed`,
`key_not_derivable`, `privacy_refused`, `unverifiable_source`, `domain_unresolvable`,
`scope_unresolvable`, `conflicts_with_existing`, `bound_exceeded`,
`extractor_malformed_output`.

A refusal record carries the reason and never the content that caused it.
