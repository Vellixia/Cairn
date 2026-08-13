# Testing tiers

Cairn's suite already has four tiers. Nothing here reorganises it; this file
writes down the boundaries it grew, so a new test lands in the right place
without a judgement call, and so the two real gaps are visible instead of
implicit.

Constitution Principle VII asks for behaviour to be verified through
user-observable behaviour. That is what tiers 3 and 4 are for. Tiers 1 and 2
exist for a narrower reason: they are the only tiers fast enough to run while
you are still typing, and a daemon whose logic can only be reached by spawning
a process does not get tested at that speed.

## The tiers

A tier is defined by what it **refuses**, not by how abstract it feels. The
question is always the same: what does this test need in order to run?

| Tier | Needs | Refuses | Lives in | Tests |
| --- | --- | --- | --- | --- |
| **1 — pure** | nothing outside memory | filesystem, socket, database, clock, subprocess | `#[cfg(test)]` beside the code | 100 |
| **2 — component** | a real store, a real repository, a temp dir; in-process | spawning a Cairn binary | `#[cfg(test)]` beside the code | 103 |
| **3 — journey** | the real binaries; a user-visible outcome | anything with no user story behind it | `tests/tests/us*.rs`, `manual_mcp_mode.rs` | 82 |
| **4 — hostile** | something to go wrong — a crash, a race, corruption, no network | happy paths | `hostile_environment.rs`, `concurrency.rs`, `storage_contention.rs`, `windows_transport.rs` | 18 |

303 tests, of which 298 run on a given Unix host — `windows_transport.rs`
contributes 5 that are `#![cfg(windows)]`. Tiers 1 and 2 are split by module
rather than by individual test, so a handful of pure tests sit in a
component-tier module and are counted with it; the totals are exact, the
boundary between the two is approximate by a few tests either way.

Tiers 1 and 2 are both colocated with the code they test, and should stay
there. The distinction between them is not about where the file sits — it is
about what the test costs and therefore how often it can run. Moving tier 2
into `crates/*/tests/` would buy a directory and nothing else.

### Tier 1 — pure

No I/O of any kind. `redact`, `budget`, `bound`, `context`, `handoff`,
`release`, the wire types, the render functions. These are the tests that make
a refactor cheap, and the ones a bug should be reduced to whenever it can be.

The rule is deliberately mechanical rather than aesthetic. "Test one file at a
time" sounds like the same thing and is not: it keys tests to structure, so a
rename silently relocates coverage, and it gives no answer for a function whose
whole job is to touch a database. "Touches nothing" is checkable, and survives
refactoring.

### Tier 2 — component

Real SQLite (`Store::open_memory`), real `git`, real temp directories — but
in-process, with no Cairn binary spawned. `crates/cairnd/src/capture.rs` is the
worked example: a `fixture()` that opens an in-memory store, seeds a project
and a session, and drives `capture` directly.

This tier is where the store's own behaviour is proved — transactions,
contention, FTS5 search, the outbox — and it is the tier the daemon is missing
(see Gaps).

### Tier 3 — journey

The real `cairn`, `cairnd` and `cairn-server` binaries, driven the way a user
or an agent drives them: hooks on stdin, CLI arguments, MCP calls, the web UI.
One file per user story, which is the layout the suite already uses.

A tier 3 test **must cite the requirement it proves** in a doc comment
(`FR-047`, `SC-003`). This is the tier where the spec is discharged, and a test
that cannot name what it discharges probably belongs in tier 2.

### Tier 4 — hostile

Defined by what goes wrong, because otherwise it becomes the drawer everything
uncategorised ends up in. A test belongs here when it needs one of:

- a process killed rather than stopped (`SIGKILL`, `TerminateProcess`)
- concurrency — several clients, threads or daemons contending
- damaged or hostile data — a corrupt database, an unwritable home, no `git`
- no network, or a server that was taken away
- a platform-specific transport (Unix socket rename, named-pipe ownership)
- a bounded-latency claim measured against a release build

Everything on that list has tests: `hostile_environment.rs` (4),
`concurrency.rs` (5), `storage_contention.rs` (2), `windows_transport.rs` (5,
Windows only), and `polish_performance.rs` and `polish_estimator.rs` (1 each).

They are grouped by *kind* rather than by subject, which is what makes placement
obvious — the first-use race belongs with the other contention tests, not with
the repository-discovery tests it happens to be about.

## Gates

Tiers only pay for themselves if they run at different times. Measured on an
M-series laptop, with everything already built:

- tiers 1 + 2 (`cargo test --workspace --lib --bins`) — **203 tests, 4.3s**
- tiers 3 + 4 (`cargo test --workspace --test '*'`) — **95 tests, ~15s**, of
  which `hostile_environment` alone is 13.5s and sets the floor

Tier 2's share of that 4.3s is dominated by the daemon's `Repo` fixture, which
does a real `git init` per test. Identity and signing are passed as environment
variables rather than as `git config` calls for exactly this reason — the naive
version cost three extra subprocesses per fixture and took the gate to 14.6s,
which is no longer a gate anyone leaves running.

Measure these on an idle machine. Leaked daemons from earlier runs (gap 3 below)
made the same command range from 15s to 45s, which is more variance than any of
the differences these tiers are meant to expose.

So:

| When | What |
| --- | --- |
| on save | `cargo test --workspace --lib --bins` |
| before a commit | the above, plus the `us*` journey files |
| CI | everything, on all three platforms, plus the web suite |
| CI, and before a release | the guards below |

The numbers are the argument. A 4s gate is one you will actually leave running;
a 20s one is one you will start skipping, and a skipped gate tests nothing. That
also sets a budget: tier 2 is allowed to be slower than tier 1, but not so much
slower that the combined gate stops being used.

## Traceability

Every requirement in the spec is cited by a test, or is listed as an exception
with a reason. `scripts/requirement-coverage.sh` enforces it, and the `guards`
CI job runs it. Today: **73 of 76 cited, 3 exceptions.**

A requirement that no test names cannot be checked when it changes. The suite
passes, the product works, and nobody can answer "which test proves FR-030?"
without reading everything — which is exactly the question a change to FR-030
raises. So the citation goes in the doc comment of the test that proves it:

```rust
/// The daemon reconciles sessions from a previous run (FR-009, D16).
```

Ten requirements were uncited when the check was written. Seven were already
*proved* by a test that simply never named them, so they were cited rather than
excused — one doc comment each. That is the intended response to a gap here; the
exception list is for requirements a test cannot discharge at all, and an entry
in it is an admission that nothing checks that requirement.

The check counts citations only from test code: `tests/`, `web/e2e/`, and
`#[cfg(test)]` modules inside `crates/`. A requirement named in a doc comment on
production code states an intention — it does not pin the behaviour, which is the
whole point.

It also fails on a *stale* exception: an entry for a requirement that is now
cited quietly grants an exemption nobody needs, and hides the next regression
behind it.

## Guards

Three requirements cannot be discharged by adding a test, because they are
constraints on what Cairn must *not* do or *not* need. A passing test suite is
compatible with all three being violated.

- **FR-025** — no embeddings, vector store or knowledge graph. Proved by
  asserting the dependency graph does not contain one, not by a test.
- **FR-063** — no knowledge-graph visualization or analytics in the web UI.
  Same shape.
- **FR-045 / SC-006** — fully offline. `scripts/network-isolated-tests.sh` runs
  the suite in a container with loopback only, so a code path that needed the
  network fails rather than quietly degrading. It existed from the start and
  **nothing ran it**: the offline guarantee was asserted by a file nobody
  executed, and the `cairn link` bug fixed in this release was an
  offline-correctness bug that reached a release. The `offline` CI job now runs
  it. The script warms a registry and target volume with the network allowed and
  then removes the network, because `cargo --offline` can only succeed against a
  registry that already holds the crates; both phases are the script's own, so
  the local and CI paths cannot drift apart.

Guards are not a tier. They are separate CI jobs, because they answer a
different kind of question and they fail for different reasons. `guards` is
instant and deliberately not queued behind `offline`, which builds a container.

## Gaps

Two, both concrete.

**1. The daemon's tier 2 now exists, and is not finished.** `crates/cairnd` had
**9 unit tests for 2,300 lines**, all of them in `capture.rs`; it now has 55,
across `handlers.rs` (15), `state.rs` (8), `recover.rs` (7), `briefing.rs` (6),
`sync.rs` (6) and `handoffs.rs` (4). Two fixtures in
`crates/cairnd/src/testsupport.rs` carry it:

- `daemon()` — a `Daemon` over `Store::open_memory()`, for everything that only
  touches storage. Worktree paths point under `NOWHERE`, which cannot resolve, so
  the missing-worktree fallback (FR-009) is exercised rather than assumed.
- `Repo` — the same plus a real `git init` checkout, seeded into the daemon's
  repository cache. Most handlers record the branch and commit they ran against,
  because repository state is *derived* from Git rather than guessed
  (Principle VI), so they cannot be driven over a fabricated path.

What is still missing is coverage of the handlers that need more than a fresh
repository — sync drain accounting against a live server, and the context
assembly path, which calls `git_status` with `?` and so has no
missing-worktree fallback of its own. Those remain tier 3 for now.

Across the workspace, 5,169 of 14,910 lines sat in files with no `#[cfg(test)]`
module when this was written. `crates/cairn/src/main.rs` (979),
`crates/cairn-server/src/api.rs` (737) and `crates/cairn/src/update.rs` (316)
are the largest remaining, and the CLI and the server API are both reachable the
same way the daemon turned out to be.

**2. `foundation.rs` no longer straddles tiers 3 and 4.** It held three journey
tests and five hostile ones, and took 14.0s — setting the floor for every e2e
run, including the ones that only wanted the journeys. The hostile cases each
spawn a `cairn` that cannot reach a daemon and wait out its start timeout, which
is where the time went.

Split as: `foundation.rs` keeps the three journeys (**0.5s**),
`hostile_environment.rs` takes the four broken-environment cases (13.5s), and the
first-use race moved to `concurrency.rs`, where the other contention tests live.
Grouping tier 4 by *kind* rather than by subject is what makes that last move
obvious.

**3. Leaked daemons made timings and flakes unreadable.** A test that takes a
bare `sandbox_socket()` instead of a `Sandbox` — to drive `cairn` against a
deliberately broken environment — had no owner to stop the daemon afterwards.
`cairn` spawns `cairnd` *before* the request fails, so a daemon bound the socket
and nothing ever stopped it; the socket lives in the system temp directory rather
than the test's own `TempDir`, so it outlived the test too.

They accumulate across runs. Eighteen were found on one machine, all live, all
competing for CPU with the suite that spawned them — which is enough to make a
timing-sensitive test fail for a reason unconnected to the code, and enough to
make any measurement of the gates above meaningless. `cairn_e2e::DaemonSocket`
now owns such a socket and stops its daemon on drop. Verified by counting
processes before and after: one leak per run of `hostile_environment.rs` without
it, none with it.

The general rule: **a test that starts a daemon must own something whose `Drop`
stops it.** `Sandbox` already does; nothing else should hand-roll it.

## Writing a new test

1. Can it run with no I/O? Tier 1, beside the code.
2. Does it need a real store, repository or temp dir, but no Cairn process?
   Tier 2, beside the code, using `Store::open_memory`.
3. Is something being broken, raced, corrupted or disconnected? Tier 4, and
   cite the requirement.
4. Otherwise it is a user-visible journey. Tier 3, in the `us*` file for its
   story, and cite the requirement.

If a test looks like it belongs in two tiers, it is usually two tests.
