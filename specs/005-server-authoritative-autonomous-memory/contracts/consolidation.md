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

**"Closed" is read from the server's own `sessions` row** — `ended_at IS NOT NULL` or a status
other than `active` — which synchronization already maintains. No new client assertion is
introduced for it, and in particular a client cannot declare a session closed in order to have
it consolidated sooner.

A **generation** is one round of a session's pending work: everything enqueued for that session
between the moment it was last `done` and the moment it becomes `done` again. A long-lived
session has many, and they are the unit eligibility and age are measured over — which is what
makes it wrong for one generation to inherit another's clock or another's latch.

**Eligibility latches for the duration of a generation.** Once a trigger fires,
`consolidation_session.eligible_since` is set and the session stays eligible until its pending
work is exhausted. Without the latch the tail of a partial batch strands: 205 events trigger at
200, the pass takes 200, and the remaining five satisfy no trigger of their own — not 200, not
ten minutes on a clock that has just been reset, and not a closed session. They would wait for a
condition they can no longer meet, which is a stall the five-minute lease does not even explain.

The latch is a column rather than worker state because a restart must not lose it; an in-memory
latch strands exactly the same five events across a deployment. It is cleared when the
generation finishes, and cleared again when a `done` session re-opens, because a new generation
has met no threshold of its own.

**A re-opened session's clock starts over.** `oldest_enqueued_at` is *reset* to the new work's
enqueue time on `done → pending`, not minimised against the old value. Minimising would carry
completed work forward as the age of the new generation's first event, so a single fresh event
in a long-lived session would be instantly age-eligible on the strength of work consolidated a
day ago. For a generation still `pending` or `claimed` the existing value is the true age of
work still waiting, and is preserved.

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
   AND attempts < 5                     -- five means five that actually ran (§4.1)
 ORDER BY session_seq
 LIMIT 200;

-- 3. count the attempt for every event this pass will process, BEFORE it runs
UPDATE consolidation_work SET attempts = attempts + 1
 WHERE project_id = $p AND session_id = $s AND event_id = ANY($batch);

COMMIT;   -- row lock released; the lease stands as committed state
```

After processing, the worker first marks every successfully processed event `done`, then
retires only the still-`pending` fifth-attempt failures, then releases the session lease. These
statements run in one close transaction, in this order:

```sql
-- 1. success wins, including success on attempt 5
UPDATE consolidation_work SET state = 'done'
 WHERE project_id = $p AND session_id = $s AND state = 'pending'
   AND event_id = ANY($consolidated);

-- 2. only unsuccessful fifth attempts remain pending and become failed
UPDATE consolidation_work SET state = 'failed', last_error = $last_error
 WHERE project_id = $p AND session_id = $s AND state = 'pending' AND attempts >= 5;

-- 3. close the session, or re-open it — ONE statement, so it cannot fall between the two
UPDATE consolidation_session
   SET state = CASE WHEN EXISTS (SELECT 1 FROM consolidation_work
                                  WHERE project_id = $p AND session_id = $s
                                    AND state = 'pending')
                    THEN 'pending'      -- more work: re-elect immediately
                    ELSE 'done'         -- drained
               END,
       claimed_by = NULL, claim_expires_at = NULL
 WHERE project_id = $p AND session_id = $s AND claimed_by = $worker;
```

Step 3 must be a `CASE`, not a guard. An earlier form used
`… SET state='done' … AND NOT EXISTS (pending)`: when work remained the `UPDATE` matched no
row, so the session stayed `claimed` with its lease intact and was not re-elected **until the
lease expired** — a five-minute stall after every full batch. A session with 500 pending events
would drain at 200 events per five minutes for no reason. The `CASE` releases the lease and
sets the correct next state in the same statement.

`consolidation_work.state` moves `pending → done` in step 1 and `pending → failed` in step 2.
Nothing else writes it, so the `consolidation_pending` partial index drains.

A pass longer than the lease extends `claim_expires_at` as a heartbeat.

### 4.1 `attempts` and the five-attempt rule

`attempts` lives on `consolidation_work`, per event, not on the lease — a session may be
elected many times legitimately, and counting elections would fail healthy sessions.

- It increments **once per event, at the start of the pass that will process it**, in the same
  transaction as the claim: `UPDATE consolidation_work SET attempts = attempts + 1 WHERE …
  event_id = ANY($batch)`. Incrementing at the start rather than on failure is what makes a
  worker that dies mid-pass still count its attempt; otherwise a crash loop would retry forever.
- An unsuccessful event reaching `attempts >= 5` moves to `failed` with `last_error`, so it
  never enters a sixth pass. The sweep runs in the **close** transaction, after the pass has
  actually run and **after successful event ids have moved to `done`**. Both placements matter:
  a claim-time sweep would prevent attempt 5 from running, while a close-time failure sweep
  before the success update would misclassify a successful fifth attempt as failed.

  ```sql
  -- CLOSE transaction, after the pass
  UPDATE consolidation_work SET state = 'done'
   WHERE project_id = $p AND session_id = $s AND state = 'pending'
     AND event_id = ANY($consolidated);

  UPDATE consolidation_work SET state = 'failed', last_error = $last_error
   WHERE project_id = $p AND session_id = $s AND state = 'pending' AND attempts >= 5;
  ```

  `$last_error` is the error from the pass that just ran; it exists at close time and does not
  exist at claim time. A failed event leaves the `pending` predicate and becomes visible in
  system health. It is never retried automatically.
- Events that reach 5 attempts do **not** block their session: step 3's `EXISTS` looks only for
  `pending`, so a session whose remainder has all failed is correctly marked `done`.
- A failed batch persists nothing — extraction, governance and persistence run in one
  transaction — so a partial candidate cannot exist (FR-808). The `attempts` increment is in the
  claim transaction and therefore survives the rollback of the processing transaction, which is
  what makes the counter monotonic.

The edge cases are exact:

| History | Close result | Next election |
|---|---|---|
| Attempts 1–4 fail | event stays `pending` with attempts 1–4 | retried while `attempts < 5` |
| Attempt 5 succeeds | step 1 changes it to `done`; step 2 cannot match it | none |
| Attempt 5 fails | it remains `pending`; step 2 changes it to `failed` | none |
| Worker crashes after an attempt begins | claim transaction already incremented `attempts`; lease reclaim resumes from that count | never more than five starts |

Attempt 6 never runs because claim selection requires `attempts < 5`. A `failed` event never
strands its session: it is outside the pending predicate before step 3 chooses `pending` versus
`done` and clears the lease either way.

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
  derived from the project, the session and the normalized keys — deliberately **not** the event
  set — so persistence is an upsert.

The phrase "a restart repeats none of it" is wrong and is not used: the reclaim mechanism
requires repetition. What must not repeat is the *effect*.

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
| 6a | Relation attribution | A relation a **session rule** produces records that session. A relation a **project rule** produces has no single session, and `memory_relations.decided_by_session` is `NOT NULL`: it records the **nil UUID**, which the codebase already uses for an unattributed act (`crates/cairnd/src/handlers.rs:2522-2545`). Inventing a session would misattribute the relation to work that did not decide it. |
| 6b | Relation basis | Every relation consolidation records carries a basis from the closed set `deterministic_rule`, `consolidation_reinforcement` (automatic reinforcement under FR-801a), `evidence`, `explicit_agent`, `explicit_user`. The first two are producible only by consolidation, so an inferred relation is always distinguishable from a requested one (FR-802). |
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

A candidate refused at gate 2 has no normalized keys, so this derivation is unavailable to it.
A **refusal** is identified instead by
`UUIDv5(CAIRN_REFUSAL_NS, project_id ‖ session_id ‖ refusal_reason ‖ digest(proposal))`, so
several distinct malformed proposals in one session record several distinct refusals. Deriving
them from the key pair would collapse every `key_normalization_failed` in a session onto one
row and undercount refusals, which FR-807 and SC-705 depend on being accurate.

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

## 7.1 The corroboration endpoint

FR-798a requires a reinforcement to have a persisted endpoint to reinforce *from*. It is a row
in the **same table as the knowledge it corroborates**, because both endpoints of a
reinforcement relation must be knowledge records.

It is marked `origin_kind = 'corroboration'`, extending the vocabulary
`explicit | consolidated` (`data-model.md` §6). That one marker carries every rule FR-798a needs:

- **Recall excludes it** — every retrieval query filters `origin_kind <> 'corroboration'`, so it
  is never returned as independent knowledge.
- **Counts exclude it** — including the funnel's `knowledge_accepted`.
- **Identity is stable** — it carries the deterministic `candidate_id` from §7, so a re-executed
  batch upserts the same row instead of adding a second.
- **It is visible where it belongs** — the memory detail view shows corroborations as the
  evidence behind a reinforcement count, which is what they are.

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
