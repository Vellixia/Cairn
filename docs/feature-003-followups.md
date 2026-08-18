# Feature 003 — follow-ups

Recorded so they are not rediscovered from scratch. None is a correctness,
privacy, data-loss or corruption risk; each was reviewed and left as it is for
a reason, and the reason is here.

The source of record is
[`specs/003-project-intelligence/compatibility.md`](../specs/003-project-intelligence/compatibility.md)
§Open notes. This is the developer-facing copy.

## Storage and migration

**`CHECK` constraints on `memories` are enforced in code, not in DDL.** SQLite
cannot add a `CHECK` to an existing table without rebuilding it, which would
rewrite every row — exactly what FR-513 forbids. The predicates are enforced at
the repository boundary and asserted by test. A future migration that rebuilds
`memories` for another reason should add them; new tables carry theirs in DDL
already.

**`superseded_at` for pre-existing supersessions is approximated from
`updated_at`.** The feature's single documented approximation, with its bound
stated in the migration's own SQL comment. It affects only where a memory
superseded before this release lands in an `as_of` query.

**`stale_at` is NULL for memories that went stale before the upgrade.** Their
historical applicability is reported as *unknown* rather than as a bounded
fact. No authoritative instant exists, and inferring one would be a second
approximation stacked on the first (D82).

**`sessions.task_snapshot_at_bind` is NULL for pre-upgrade sessions,** so no
divergence is reported for them. That is honest — the state they bound at is
unknowable — and it self-corrects as sessions turn over.

## Knowledge

**Reconciliation depends on agents supplying topic keys, and on their being
specific enough.** Because a shared value key no longer merges, a coarse key
costs deduplication rather than correctness: the failure mode is *less useful*
rather than *wrong*, which is the right direction for it to fail in.

Mitigated on three surfaces — the tool description, the always-on contract and
the Skill — measured by the adoption metric in `cairn status`, and observed per
agent by the non-gating effectiveness evaluation
([contracts/evaluation.md](../specs/003-project-intelligence/contracts/evaluation.md)
§Topic-key effectiveness).

**A `size`-class path fingerprint cannot see a same-length edit.** It applies
only to files over the payload cap, where source edits are rare, and the class
travels with the comparison so the weaker check is visible rather than implied
(D79).

## Patterns

**Pattern suggestion matches signals lexically.** A pattern whose signals are
worded differently from the receiving project's error text will not surface.
Accepted deliberately: a missed suggestion costs nothing, a false one costs
trust. `pattern_signals_min` and the paired corpus bound the false-positive
side.

Worth knowing for anyone changing it: matching compares **distinguishing
tokens**, not whole strings. A pattern's signals are error signatures as
someone wrote them down; a project's signals are the error text Cairn
recorded, and the two are never character-for-character equal. The first
implementation compared whole strings and would have suggested nothing, ever
— see `crates/cairn-core/src/patterns.rs`.

**The web UI does not surface patterns, evidence or checkpoints.** They are
local to the machine and the shared server holds none of them, so there is
nothing for a browser to show. If a future feature shares patterns, the UI
follows then.

## Verification

**Attested evidence never re-verifies on its own.** A recheck of attested
evidence yields `needs_recheck` until the agent attests again, so an abandoned
attested claim decays to `needs_recheck` and stays there. That is the intended
resting state rather than a stuck one: Cairn cannot re-run an agent's
observation, and pretending otherwise is the failure this avoids.

**`verification_basis` on the server is a list of verifier kinds with no
ordering guarantee.** Displayed as a set. Harmless, noted so nobody reads
meaning into the order.

**A relation's `basis` can differ between two machines holding the same
decision.** The primary key is `(from, to, kind)`, so a conflict Cairn detected
here and an agent asserted there collapses to one row on each machine, keeping
whichever basis that machine wrote first. Convergence is about the decision
set, and requiring the bases to match would be requiring the two machines to
have had the same history.

## Testing

**Three tests in `sync_degradation.rs` are serialized behind a mutex.** Each
runs two servers against a database of its own, and PostgreSQL's connection
limit is a fixed resource the whole run shares. Without the lock the failures
land in whichever test asks next and say nothing about the code they were
testing.

**`Server::start` skips silently when `CAIRN_TEST_DATABASE_URL` is unset, and
panics when it is set but unreachable.** That is the right pair, and it means a
red suite can be a stopped container rather than a defect — check the container
before reading the failures. See the implementation log's Checkpoint L.

**The performance fixture is scaled.** `perf_intelligence` builds a tenth of
the stated population and says so in its own output. The index paths it
measures are the same ones the full scale would use; what it cannot show is
behaviour that only appears at 5,000 memories. Anyone changing a query on the
subject-read or drift-marking path should build the full fixture once by hand.
