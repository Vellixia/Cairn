# Quickstart: Cairn MVP

**Feature**: `001-cairn-mvp` | How to validate that each user story actually works.

This is the acceptance walkthrough. Each section maps to a user story's Independent Test
and can be run on its own once that story is implemented. **Feature 001 is the Cairn MVP:
it is done when every section below passes, not when any one of them does.**

## Prerequisites

- Rust toolchain (workspace `rust-toolchain.toml`), `git` on `PATH`
- For US6/US7: Docker for PostgreSQL, Node for the web UI
- A scratch Git repository: `mkdir /tmp/cairn-demo && cd /tmp/cairn-demo && git init && git commit --allow-empty -m init`

```bash
cargo build --workspace --release
```

## US1 — Session work is captured and handed off

```bash
cd /tmp/cairn-demo
cairn init
cairn connect claude-code
```

Start a Claude Code session in that directory. Have it edit a file and run a test that
fails. Send a second prompt after the agent finishes responding — the same session must
continue, because finishing a turn is not the end of a session. Then end the session and:

```bash
cairn status
cairn handoff show --json
```

**Passes when**: `cairn status` shows the project, branch, commit and a session; the
handoff names the edited file, the failing test, and a next step — none of which you
wrote. The failing test came from a `PostToolUseFailure` event, not from guessing at a
success payload. `cairn session show --json` reports status `completed`, with exactly one
`session_end` handoff — the turn boundary in the middle produced a checkpoint, not a
handoff, and did not split the work into two sessions.

Also verify the interruption path. Cairn has no liveness signal (D16), so the boundary is
daemon start, not process death:

```bash
# kill the agent without a clean exit, then:
cairn daemon stop && cairn daemon start
cairn session list --json
```

The session must be `interrupted` with a handoff. Then confirm the resume path: restart the
daemon mid-session while an agent is still working and send another tool call — the session
must return to `active`, and the handoff already written must remain.

Concurrency: start two agent sessions in the same directory, and a third in a second
worktree. `cairn session list --json` must report three distinct sessions, every
observation must land on the session that produced it, `cairn session show` without
`--session` must return `ambiguous_session` rather than guessing, and ending one must not
touch the others.

## US2 — The next session starts already informed

With the previous session's handoff in place:

```bash
cairn context --budget 3000 --json
```

**Passes when**: the briefing contains the current branch and commit, the previous
handoff's remaining work and next step, and reports `estimated_tokens` **at or below**
`budget` — both in Cairn-estimated tokens, not any model's tokenizer — with
`truncated`/`omitted_sections` populated honestly. Run it against a project
with far more memory than fits and confirm the budget still holds exactly — overflow is a
bug, dropping project-scope memory is correct. Start a new agent session and confirm the
same content arrives as context at `SessionStart`.

Fallback: stop the daemon, then start an agent session. The session must start within the
context deadline with `degraded: true` and a reported reduced-context state — never a hang
and never an error the agent surfaces.

Check the empty case in a fresh repository: the briefing must succeed with
`no_prior_history: true`.

Check the branch case: `git switch -c feature/x`, refresh context, and confirm the
briefing reflects the new branch and prioritizes its memory.

## US3 — Durable, scoped memory and recall

```bash
cairn memory add --type convention --scope project "Errors are returned, never logged and swallowed"
cairn memory add --type failure   --scope branch  "Integration tests need the local queue running"
cairn memory add --type decision  --scope task    "Chose the outbox over dual writes"
cairn memory search "tests" --json
```

**Passes when**: results are ordered task → branch → project, each carries its origin
session id, and only `active` memories appear. Every one of those memories was created from
the command line with no supporting observations — `evidence_count` is `0`, `session_id` is
present, and that is a valid memory, not a degraded one (FR-019). Then:

```bash
cairn memory forget <id>          # soft delete
cairn memory search "outbox" --state superseded --json
```

Supersede a memory through the agent (`cairn_remember` with `action: supersede`) and
confirm the original is retained as `superseded` and linked to its replacement.

Offline check: disable the network and repeat every command above. All must succeed.

## US4 — Work is organized by task

```bash
cairn task new --title "Add rate limiting" \
  --goal "Requests over the limit get 429" \
  --criterion "429 returned above threshold" \
  --criterion "Limit is configurable"
cairn task set-status <id> in_progress
```

Start a session bound to that task, end it, then start another on the same task.

**Passes when**: `cairn context` leads with the task goal and acceptance criteria,
task-scoped memory ranks first, and the briefing includes the previous session's handoff
for that task.

## US5 — The developer controls what Cairn stores

```bash
cairn privacy exclude --path "secrets/**"
cairn privacy exclude --command "aws sts*"
```

Run a session that reads `secrets/prod.env` and echoes a value shaped like an API key.

```bash
cairn status --json | jq '.data.observation_count'
```

**Passes when**: no observation exists for the excluded path, no stored record contains
the key (grep the database file), and every stored payload is within the cap.

Deletion semantics — each checked separately, in this order, against one session that
produced both a memory and a handoff:

```bash
cairn delete observation <observation-id>   # memory + handoff survive
cairn delete session <session-id>           # memory + handoff STILL survive
cairn handoff show --session <session-id>   # readable, origin marked deleted
cairn delete memory <memory-id>             # that memory only
cairn delete handoff <handoff-id>           # that handoff only
```

After the observation delete, the memory and handoff survive and the evidence reference
resolves as deleted. After the session delete, the session and its observation content are
cleared but **the memory and the handoff still survive**, with their origin marked deleted —
a session delete must never remove the durable records it produced. `cairn delete session
<id> --with-memories` is the only way to take the memories too. The memory and handoff
deletes each remove exactly one record. A memory created with `local_only` must never
produce an outbox row.

## US6 — Project memory shared with teammates

```bash
docker compose up -d postgres
cargo run --bin cairn-server                 # runs migrations on start
```

In the web UI, register a user and create an API token. Then:

```bash
cairn auth token set                          # paste the token
cairn link
cairn sync now
cairn sync status --json
```

**Passes when**: the project and its memory appear on the server; a second member of the
project can search and read them with provenance intact; a non-member gets `403`.

Observation boundary: dump the server database and inspect every sync payload. Provenance
must be present as session id, observation ids, and an evidence count — and there must be
**no observation row and no observation content anywhere**: no summary, no path, no
command, no details. Attempting to send one must come back `rejected`.

Cross-machine identity: clone the same repository to a second path (standing in for another
machine), run `cairn link --project <the same id>`, and confirm both clones sync into one
shared project rather than creating two. A repository with no remote must link the same way
by identifier.

Idempotency: run `cairn sync now` twice and confirm server state is unchanged and every
item reports `duplicate`.

Offline: stop the server, keep working, confirm all local commands succeed and the outbox
grows; restart the server and confirm the queue drains with no duplicates.

Opt-in: in a second, unlinked repository, run a session with a network capture attached
and confirm zero outbound requests carrying its data.

## US7 — Seeing and managing memory in a browser

```bash
cd web && npm install && npm run dev
```

Sign in and, without touching a terminal: find the project, open its overview, open a
recent session and read its handoff in full, search memory with a scope filter, open a
result and see its origin session, delete it, and open sync status.

**Passes when**: all seven actions complete in the UI and the deletion is reflected in
the next `cairn memory search` locally after sync.

## Measured checks

| Check | Where |
|---|---|
| No briefing ever exceeds its Cairn-estimated-token budget (100%), and ≥95% fit without dropping a high-priority section (SC-003) | US2 walkthrough, repeated |
| Estimator error against a real tokenizer is measured and conservative (SC-003, D8) | Polish, T075 |
| Recall of a known fact in the top 5 (SC-004) | US3 walkthrough |
| Capture-hook latency: median ≤10 ms, p95 ≤25 ms, every hook inside its 250 ms deadline, over 200 release-binary invocations (SC-007) | `cargo test --release -p cairn-e2e --test polish_performance` |
| `SessionStart` returns within its deadline on a cold daemon and a large repository (D15) | US2 fallback step, repeated on a large checkout |
| No secret-shaped value in storage (SC-008) | US5 seeded-secret fixture |
| Replayed batch is a no-op (SC-009) | US6 idempotency step |
| Unlinked project emits nothing, and a linked project transmits zero observation content (SC-010) | US6 opt-in and observation-boundary steps |

## Measurements on record

**Token estimator (T075, D8).** Cairn estimates **3.5 characters per token**. Measured
against a **real BPE tokenizer** (`cl100k_base`) on a briefing produced by the US2
walkthrough (1,092 characters): Cairn estimated 312 tokens, the tokenizer counted 279 —
**+11.8%, conservative**, which is the direction that keeps the estimated budget a safe
upper bound. `tests/tests/polish_estimator.rs` asserts the estimator never under-counts a
real tokenizer.

**Capture-hook latency (T074, SC-007).** SC-007 bounds what Cairn controls: the latency of
its own capture hook, measured absolutely rather than as a share of the agent's wall-clock
time (D17). 200 invocations per round, 3 rounds per run, release binaries, run against the
**production** 250 ms deadline. **Process-startup cost is included, not subtracted** — a
developer pays for the whole hook.

Eight independent runs, twenty-four rounds, 4,800 hook invocations:

| | median | p95 | max |
|---|---|---|---|
| budget | ≤10 ms | ≤25 ms | <250 ms |
| observed | **3.06 – 4.22 ms** | **3.29 – 6.59 ms** | **4.01 – 18.97 ms** |
| headroom | 42% of budget at worst | 26% of budget at worst | 7.6% of the deadline at worst |

Every round passed. The median sat at 3.06–3.09 ms across the first five runs and drifted to
3.4–4.2 ms under later machine load, so the figure is stable rather than a lucky sample. Where the time goes: a bare `/usr/bin/true` spawn costs ~1–2 ms on the
reference machine and `cairn --version` ~4.8 ms, so almost all of a hook is process startup
and Cairn's own connect-and-write is within noise of zero.

*Informational, not a gate*: the same hooks over a 50-call session cost +36.7% against a
~10 ms synthetic tool call, and the identical absolute cost is ~0.8% of a 500 ms tool call.
That ratio is a property of the workload, which is exactly why SC-007 no longer states one.

**`SessionStart` latency (D15).** Not yet measured against a large repository with a cold
filesystem cache. The 1,500 ms default stands unrevised; the reduced-context fallback is
exercised by `tests/tests/us2_context.rs`.

**Network isolation (SC-006).** `scripts/network-isolated-tests.sh` runs the local suites in
a container with `--network none` (loopback only): 8 suites, 47 tests, 0 failures.
