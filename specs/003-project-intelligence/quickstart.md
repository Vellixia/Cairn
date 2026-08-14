# Quickstart: Cairn Project Intelligence

**Feature**: `003-project-intelligence`

The user-visible result, demonstrated on a real repository. One section per user story; each is
independently runnable and each ends in something a developer can see.

Everything below runs offline. No server, no API key, no network.

## Prerequisites

```bash
cargo build --workspace --release
export PATH="$PWD/target/release:$PATH"

cd ~/src/your-git-repo
cairn init
cairn connect claude-code            # or codex, opencode
```

An existing Cairn store upgrades in place at first open. `cairn status` shows the schema version:

```bash
cairn status
#   project        your-git-repo
#   branch         main @ abc1234
#   schema         5
#   memory         412 memories · 38 with a subject · 4 pinned
#   knowledge      2 conflicted subjects · 7 needing recheck · 1 drifted
#   integrations   claude-code (FULL) · continuity: automatic
```

---

## US1 — Canonical project knowledge

Three sessions record the same thing. The briefing says it once.

```bash
# Three separate sessions, over three days, each recording the same fact.
cairn memory add --type fact --scope project \
  --topic-key infrastructure.production_database --value-key postgresql \
  "Production runs PostgreSQL 16"

cairn memory add --type fact --scope project \
  --topic-key infrastructure.production_database --value-key postgresql \
  "The production database is Postgres"
#   reconciliation: corroborating
#   agrees on value `postgresql` with memory 0192f4…, but the wording differs
#   → if this is the same claim: cairn memory reinforce 0192f4…

cairn memory reinforce 0192f4…
#   reinforced  →  memory 0192f4…  (explicit, session 0192a1…)
```

Cairn will not decide for you that two differently-worded statements are one claim — that needs reading
them, which is judgement, not a rule. It tells you which memory yours agrees with and makes collapsing
them one call. If you skip the call, nothing is lost: both stand, and the briefing shows one plus a
count.

An **identical** statement needs no call at all:

```bash
cairn memory add --type fact --scope project \
  --topic-key infrastructure.production_database --value-key postgresql \
  "Production runs PostgreSQL 16"
#   reconciliation: duplicate  →  memory 0192f4…   (content identical after normalization)
```

Ask what the project holds:

```bash
cairn memory subject infrastructure.production_database
#   SUBJECT  infrastructure.production_database  (project)
#   state    reinforced
#   answer   postgresql — "Production runs PostgreSQL 16"
#            reinforced 2 times · 3 distinct origin sessions · unverified
#   members  3 (all retrievable individually with their own provenance)
#   decisions
#     0192f5… reinforces 0192f4…  (explicit_agent,     session 0192a1…, claude-code)
#     0192f6… duplicates 0192f4…  (deterministic_rule, session 0192b7…, codex)
```

### A coarse value key does not merge two different claims

```bash
cairn memory add --type decision --scope project \
  --topic-key auth.strategy --value-key jwt "JWT uses HS256 with a shared secret"
cairn memory add --type decision --scope project \
  --topic-key auth.strategy --value-key jwt "JWT uses RS256 with rotating public keys"
#   reconciliation: corroborating
#   agrees on value `jwt` with memory 0192h1…, but the wording differs

cairn memory subject auth.strategy
#   SUBJECT  auth.strategy  (project)
#   state    CORROBORATED — the value is agreed, the statements are not
#   value    jwt
#   answers  "JWT uses HS256 with a shared secret"        (0192h1…, claude-code)
#            "JWT uses RS256 with rotating public keys"   (0192h2…, codex)
#   no reinforcement recorded — these are different claims about one value
```

Both survive. Neither is suppressed. The briefing shows one plus `+1 further statement`, and an agent
that needs to know which algorithm is in force is told there is more to read.

**What you see**: the repeated fact costs budget once instead of three times, every contributing memory
still exists with its own provenance — and two genuinely different claims are never quietly collapsed
into one just because an agent wrote a broad value key.

### The scope exception

```bash
cairn task new --title "Integration fixtures" --goal "Fixtures run without Docker"
cairn memory add --type decision --scope task \
  --topic-key infrastructure.production_database --value-key sqlite \
  "This fixture suite uses SQLite in-memory"
#   reconciliation: created  (no conflict — narrower scope)
```

```bash
cairn memory subject infrastructure.production_database
#   SUBJECT  infrastructure.production_database
#   project  postgresql — reinforced ×2
#   task     sqlite     — narrows the project answer
#   state    settled per scope · NO CONFLICT
```

**What you see**: inside that task the answer is SQLite; everywhere else it is PostgreSQL. No false
alarm.

---

## US2 — Knowledge evolves safely

```bash
cairn memory supersede --memory-id 0192f4… --type decision --scope project \
  --topic-key infrastructure.production_database --value-key cockroachdb \
  "Migrated production to CockroachDB in the 2026-08 migration"
```

```bash
cairn memory search --topic-key infrastructure.production_database
#   [decision] Migrated production to CockroachDB…   active   value: cockroachdb

cairn memory search --topic-key infrastructure.production_database --as-of 2026-08-01T00:00:00Z
#   as_of 2026-08-01T00:00:00Z
#   [fact] Production runs PostgreSQL 16              superseded  value: postgresql
```

**What you see**: today's answer, and July's answer, both correct. Reading a July session's handoff now
makes sense — it was written against a project that ran PostgreSQL, and Cairn can say so.

---

## US3 — Conflicts are visible

Two agents, in two worktrees, at the same time.

```bash
# worktree A, Claude Code
cairn memory add --type fact --scope project \
  --topic-key deploy.queue_backend --value-key sqs "Deploys queue through SQS"

# worktree B, Codex
cairn memory add --type fact --scope project \
  --topic-key deploy.queue_backend --value-key rabbitmq "Deploys queue through RabbitMQ"
#   reconciliation: conflict_detected
#   subject deploy.queue_backend now has 2 competing answers
```

```bash
cairn context
#   ⚠ CONFLICT  deploy.queue_backend
#       sqs      — "Deploys queue through SQS"       (claude-code, session 0192c1…)
#       rabbitmq — "Deploys queue through RabbitMQ"  (codex,       session 0192c4…)
#       unresolved — resolve with `cairn memory reconcile`
```

**What you see**: both, attributed, with no winner picked. Resolve it deliberately:

```bash
cairn memory reconcile --from <rabbitmq-id> --to <sqs-id> \
  --relation supersedes --basis explicit_user \
  --rationale "SQS was decommissioned in July"
#   subject deploy.queue_backend: settled → rabbitmq
```

---

## US4 — Evidence-backed knowledge

```bash
cairn memory add --type fact --scope project \
  --topic-key service.api_port --value-key 8080 "The API listens on 8080"

cairn evidence add --memory <id> --kind configuration \
  --subject "API port" --value 8080 --locator config/app.yml
#   evidence 0192d1…  collector: cairn  digest: 9f2e…

cairn verify --memory <id>
#   verified  ·  configuration @ config/app.yml  ·  abc1234 on main  ·  2026-08-14T09:12:04Z
#   authority: cairn  (a deterministic check Cairn ran itself)
```

```bash
cairn memory search --topic-key service.api_port
#   [fact] The API listens on 8080   active  ✓ verified (configuration) 2026-08-14  authority: cairn
```

An assertion of importance is not evidence:

```bash
cairn evidence add --memory <id> --kind runtime_state \
  --subject "the team thinks this matters" --value important
#   error: verifier_unavailable — no deterministic verifier for that subject
```

### Attested evidence is useful, and never wears the same badge

Cairn cannot call a staging API. An agent can, and telling Cairn the result is worth recording:

```bash
cairn evidence add --memory <id> --kind runtime_state --collector agent \
  --subject "GET /health version field" --value "2.4.1"
cairn verify --memory <id>
#   verified  ·  runtime_state  ·  authority: attested
#              established by an agent's submission, not by a check Cairn ran

cairn memory search --topic-key service.version
#   [fact] The service reports 2.4.1   active  ✓ verified (attested)  authority: attested
```

It is `verified`, and it says how. The two places where that difference decides something both refuse it:

```bash
cairn task criterion verify <ac-id>
#   error: attested_not_sufficient
#     A criterion is verified only on evidence Cairn collected itself.

cairn pattern promote --memory <id> --dry-run
#   refused: attested_not_sufficient
#     Cross-project promotion needs a deterministic check Cairn ran on this machine.
```

---

## US5 — Drift detection

```bash
sed -i '' 's/port: 8080/port: 9000/' config/app.yml
cairn verify --memory <id>          # or wait for the maintenance pass

cairn memory search --topic-key service.api_port
#   [fact] The API listens on 8080   active  ⚠ drifted
#          evidence config/app.yml now reads 9000 (was 8080)
```

```bash
cairn context
#   ⚠ DRIFT  service.api_port — remembered 8080, config/app.yml now says 9000
```

The memory content is untouched. Cairn did not rewrite it and did not invent a replacement:

```bash
cairn memory show <id>
#   content   The API listens on 8080        ← unchanged
#   state     active
#   verification  drifted
```

Replacing it is your call:

```bash
cairn memory supersede --memory-id <id> --type fact --scope project \
  --topic-key service.api_port --value-key 9000 "The API listens on 9000"
```

---

## US6 — Compression-safe continuity

Start real work, then compact ten times.

```bash
cairn task new --title "Retry backoff" --goal "Transient failures retry with jitter" \
  --criterion "backoff is exponential with jitter" \
  --criterion "the retry cap is configurable" \
  --criterion "cargo test --workspace passes"
cairn session start --agent claude-code --task-id <task-id>
```

Work normally. Cairn captures. At each compaction boundary the adapter fires
`context_compacting`, and Cairn writes a checkpoint. After the tenth:

```bash
cairn context
#   TASK       Retry backoff — transient failures retry with jitter    (in_progress)
#   CRITERIA   AC-1 satisfied ✓verified · AC-2 pending · AC-3 satisfied (unverified)
#   PROGRESS   1 verified · 1 satisfied but unverified · 0 blocked · 1 pending
#   NEXT       add the configurable cap to RetryConfig
#   CONSTRAINTS
#     • never mutate CC Switch's private database directly
#   REJECTED   fixed 100 ms sleep — thundering herd under load
#   REPOSITORY main @ def4567 · 3 unstaged
#   continuity automatic · checkpoint restored 10 times · state current
```

**What you see**: after ten compactions the agent still knows the goal, the criteria and their states,
what is left, what was already ruled out, and what to do next — from Cairn, not from a summary.

### Checkpoint divergence

While your session is compacting, a second session moves the ground:

```bash
# session B
git commit -am "refactor config loading"        # head abc123 → def456
cairn task criterion add <task-id> --text "production smoke passes"
# edits src/config.rs
```

Session A resumes:

```bash
cairn context --reason post_compaction
#   ⚠ CHECKPOINT DIVERGED
#       recorded at abc123 on main
#       current:      def456 on main
#       task changed: AC-4 added "production smoke passes"   (another machine)
#                     blocker opened "staging credentials"    (this machine)
#       files changed: src/config.rs  (digest differs; session 0192e9…, claude-code)
#       previous next action (may be stale):
#           "finish the retry backoff in config.rs"
```

**What you see**: Cairn does not tell you to carry on editing `config.rs` from a commit that has moved.

### A change nobody told Cairn about

Now do it with no Cairn session at all — edit in your editor, run a formatter, apply a patch — and do
not commit:

```bash
$EDITOR src/config.rs                 # or: cargo fmt, or git apply, or an IDE refactor
git status --short                     # M src/config.rs — commit unchanged

cairn context --reason post_compaction
#   ⚠ CHECKPOINT DIVERGED
#       commit unchanged (abc123)
#       files changed: src/config.rs  (digest differs; no Cairn session recorded a change)
#       previous next action (may be stale):
#           "finish the retry backoff in config.rs"
```

**What you see**: the checkpoint compares the fingerprint it recorded, so it does not matter who made
the change or whether Cairn was watching. And a path it cannot fingerprint says so rather than
pretending:

```bash
cairn session checkpoint
#   checkpoint 0192j1…  ·  8 relevant paths fingerprinted
#     6 digest · 1 size (vendor/large.bin exceeds the payload cap) · 1 unknown (secrets/** excluded)
```

### An agent without a post-compaction signal

```bash
cairn doctor
#   opencode   MCP_PLUS
#     continuity: agent_initiated
#       A checkpoint is written before compaction. This agent provides no
#       post-compaction signal, so continuity is not restored automatically —
#       call cairn_context(reason=post_compaction) after a compaction.
```

Honest, and specific about what to do instead.

---

## US7 — Multi-device consistency

Two machines, both offline, both linked to one shared project.

```bash
# machine A                                    # machine B
cairn memory add --type fact --scope project   cairn memory add --type fact --scope project \
  --topic-key infrastructure.production_database  --topic-key infrastructure.production_database \
  --value-key postgresql "prod is PostgreSQL"     --value-key cockroachdb "prod is CockroachDB"
```

Both reconnect:

```bash
cairn sync now      # on each machine
cairn memory subject infrastructure.production_database
#   SUBJECT  infrastructure.production_database  (project)
#   state    CONFLICTED
#   answers  postgresql  — machine A, session 0192f1… (claude-code)
#            cockroachdb — machine B, session 0192f8… (codex)
#   no winner selected — resolve with `cairn memory reconcile`
```

Identical on both machines, and identical whichever machine's clock is ahead. Nothing was decided by a
timestamp.

A supersession decided on A lands on B as the decision, not as an overwrite:

```bash
# machine A
cairn memory reconcile --from <cockroachdb-id> --to <postgresql-id> \
  --relation supersedes --basis explicit_user --rationale "migration completed"
cairn sync now

# machine B
cairn sync now
cairn memory subject infrastructure.production_database
#   state   settled
#   answer  cockroachdb
#   the postgresql memory is superseded here too, from A's recorded decision
```

### A peer's verification says how it was established

```bash
# machine B, reading a memory machine A verified
cairn memory search --topic-key service.api_port
#   [fact] The API listens on 8080   active
#          ✓ verified elsewhere  authority: remote_cairn
#            a deterministic check on another machine, 2026-08-14

# and one machine A only attested
cairn memory search --topic-key service.version
#   [fact] The service reports 2.4.1   active
#          ✓ verified elsewhere (attested)  authority: remote_attested
#            an agent's submission on another machine — not a check
```

**What you see**: two verifications that would have looked identical now do not. Neither counts toward
this machine's task readiness, and neither can be promoted from here.

### An old server, and then an upgraded one

```bash
# server still on Feature 001's schema
cairn sync now
cairn sync status
#   pending 0 · blocked 3 · failed 0 · last success 2026-08-14T09:14:02Z
#   ⚠ degraded: this server does not accept memory relations or task criteria
#     (server schema 1, this build expects 2). 3 items are retained and will be
#     delivered automatically when the server is upgraded. Memories, tasks,
#     sessions and handoffs are syncing normally.
```

Nothing is lost and nothing is retried pointlessly. Upgrade the server, and the next drain cycle notices:

```bash
# after the server is upgraded
cairn sync now
#   server capability changed (schema 1 → 2) · released 3 blocked items
#   applied 3 · duplicate 0 · rejected 0

cairn sync status
#   pending 0 · blocked 0 · failed 0 · last success 2026-08-14T11:02:18Z

# machine B
cairn sync now
cairn memory subject infrastructure.production_database
#   state   settled          ← A's supersession finally arrived, applied exactly once
```

**What you see**: no manual repair, no re-running anything, no lost intent.

---

## US8 — Reusable cross-project knowledge

In project A, a verified fix:

```bash
cairn memory add --type procedure --scope project \
  "Expand Docker's default-address-pools when bridge network creation fails with an address-pool error"
cairn evidence add --memory <id> --kind command_outcome \
  --subject "docker network create" --value "exit 0 after pool expansion" \
  --observation-id <captured-observation>
cairn verify --memory <id>          # verified

cairn pattern promote --memory <id> \
  --signal "could not find an available non-overlapping ipv4 address pool" \
  --signal "docker bridge network create failure" \
  --applies "Docker bridge networking in use" \
  --applies "the error names address-pool allocation" \
  --root-cause "the daemon's default-address-pools are fully allocated" \
  --approach "expand default-address-pools in the daemon configuration and restart" \
  --constraint "existing networks are not migrated to the new pool"
#   pattern 0192g1…  trust: sanitized  origin: opaque
#   sanitization: 10/10 checks passed
```

Now in project B, months later, the same error surfaces and Cairn captures it:

```bash
cairn context
#   PRIOR PATTERN (unverified in this project)
#     Docker cannot allocate a non-overlapping bridge network — trust: sanitized
#     Applies when: Docker bridge networking · the error names address-pool allocation
#     Known approach: expand default-address-pools and restart
#     Caveat: existing networks are not migrated
```

**What you see**: the prior fix, offered — and explicitly not claimed to be true here.

### Promotion refuses what it should

```bash
cairn pattern promote --memory <id-of-a-project-fact> --dry-run
#   refused: not_transferable
#     A project configuration fact is not transferable knowledge.

cairn pattern promote --memory <id-with-a-path> --dry-run
#   refused: project_identifying
#     The content contains an absolute path. (value not shown)

cairn pattern promote --memory <local-only-id> --dry-run
#   refused: local_only_memory
```

---

## US9 — Counterexamples prevent poisoning

In project B the symptom matches but the cause is different:

```bash
cairn pattern outcome 0192g1… --outcome not_applicable \
  --alternative-cause "a VPN route collision produced the same error; the pools were not exhausted"
#   recorded. trust: sanitized → contested. no success count changed.
```

Next time it surfaces:

```bash
cairn context
#   PRIOR PATTERN (unverified in this project) — trust: contested
#     Docker cannot allocate a non-overlapping bridge network
#     ⚠ Known alternative cause: a VPN route collision produced the same symptom.
#       Check this first: verify the configured network ranges are genuinely exhausted.
```

Repetition does not manufacture trust:

```bash
# ten sessions in project A each record the same incident resolved
cairn pattern show 0192g1…
#   applications 11 · distinct projects 2 · independently validated in 0 · counterexamples 1
#   trust contested
```

**What you see**: eleven applications, and Cairn still says nobody has independently validated it.

---

## US10 — Minimum safe context

A project with thousands of memories and a tight budget.

```bash
cairn memory search --limit 1 --json | jq '.total'      # 5000+

cairn context --token-budget 800
#   TASK       Retry backoff — transient failures retry with jitter   (in_progress)
#   NEXT       add the configurable cap to RetryConfig
#   BLOCKERS   staging credentials expired
#   ⚠ CONFLICT deploy.queue_backend — sqs vs rabbitmq
#   ⚠ DRIFT    service.api_port — remembered 8080, config/app.yml says 9000
#   CONSTRAINTS
#     • never mutate CC Switch's private database directly
#   REPOSITORY main @ def4567 · 3 unstaged
#   CRITERIA   AC-1 ✓ · AC-2 pending · AC-3 satisfied (unverified)
#
#   estimated 782 / 800 tokens · truncated · omitted: project_memory, branch_memory
```

**What you see**: at 800 tokens with 5,000 memories present, everything you cannot work without
survives, and Cairn says exactly what it dropped.

Why:

```bash
cairn context --token-budget 800 --explain
#   budget 800 · reserve 320 · reserve used 296 · released 24
#   INCLUDED
#     minimum_safe  task       task_binding                        96
#     minimum_safe  warning    conflict_warning                    41
#     minimum_safe  warning    drift_warning                       44
#     minimum_safe  memory     pinned                              38
#     relevant      memory     scope_match canonical_answer verified 24
#   OMITTED
#     memory ×4,988  budget_exhausted
#     memory ×7      not_canonical
#     pattern ×1     cap_reached
```

### A task with forty criteria, and a budget that cannot hold them

```bash
cairn context --token-budget 800
#   TASK       Retry backoff — transient failures retry with jitter   (in_progress)
#   PROGRESS   12 verified · 6 satisfied but unverified · 3 blocked · 19 pending
#   READINESS  not_ready
#   BLOCKERS   4 open — "staging credentials expired"
#   NEXT       add the configurable cap to RetryConfig
#   ⚠ 1 conflict · 1 drift
#   REPOSITORY main @ def4567 · 3 unstaged
#   CONSTRAINTS
#     • never mutate CC Switch's private database directly
#   CRITERIA   AC-9 blocked · AC-14 blocked · AC-3 satisfied (unverified)
#              + 37 criteria omitted — `cairn task get <id>`
#   BLOCKERS   1 of 4 shown — `cairn task get <id>`
#
#   estimated 794 / 800 tokens · truncated

**What you see**: forty criteria do not fit in 800 tokens and Cairn does not pretend otherwise. What is
guaranteed is the *state* — goal, progress counts, readiness, the blocker that matters, the next action,
the warnings, the constraints — none of which grows with the task. Criterion text is admitted
blocked-first, and what did not fit is counted with the call that retrieves it.
```

---

## US11 — Evidence-aware tasks

```bash
cairn task get <task-id>
#   TASK      Retry backoff (revision 7)
#   CRITERIA
#     AC-1  backoff is exponential with jitter   satisfied  ✓ verified
#     AC-2  the retry cap is configurable        pending      unverified
#     AC-3  cargo test --workspace passes        satisfied    unverified
#   BLOCKERS  1 open — staging credentials expired
#   PROGRESS  1 verified · 2 satisfied but unverified · 0 blocked · 1 pending
#   READINESS not_ready
```

Two sessions, two criteria, no lost work:

```bash
# session A                                       # session B
cairn task criterion set <ac-2-id> \              cairn task criterion set <ac-3-id> \
  --state satisfied --expected-revision 7           --state satisfied --expected-revision 7

cairn task get <task-id>
#   TASK Retry backoff (revision 9)
#     AC-2 satisfied     ← session A's change survived
#     AC-3 satisfied     ← session B's change survived
```

### Two machines, two criteria, offline

```bash
# machine A, offline                      # machine B, offline
cairn task criterion set <ac-1-id> \      cairn task criterion set <ac-2-id> \
  --state satisfied                         --state satisfied

# both reconnect
cairn sync now                            cairn sync now

# machine A                                machine B
cairn task get <task-id>                  cairn task get <task-id>
#   local_revision 7                      #   local_revision 7
#   state_digest   8b21c4…                #   state_digest   8b21c4…   ← identical
#     AC-1 satisfied  ← A's change        #     AC-1 satisfied  ← arrived from A
#     AC-2 satisfied  ← arrived from B    #     AC-2 satisfied  ← B's change
```

The two `local_revision` values agreeing is a coincidence and is never compared — each is a private
counter. The `state_digest` agreeing is the guarantee, because it is computed from the criteria
themselves.

A session that bound before either change is told about both:

```bash
cairn context
#   ⚠ TASK UPDATED
#     the task advanced since you started
#     changes:
#       • AC-1 pending → satisfied   (this machine)
#       • AC-2 pending → satisfied   (arrived from another machine)
```

**What you see**: neither change overwrote the other, both machines agree on the state, and the older
session learns about the remote change too — not just the local one.

An agent cannot self-certify:

```bash
cairn task criterion verify <ac-3-id>
#   AC-3 verified ✓  (test_outcome, from a captured cargo test run at def4567)

cairn evidence add --criterion <ac-2-id> --kind runtime_state \
  --subject "I checked it" --value ok --collector agent
cairn task criterion verify <ac-2-id>
#   error: attested_not_sufficient
#     A criterion is verified only on evidence Cairn collected itself.
```

Everything green still does not close the task:

```bash
cairn task blocker clear <blocker-id>
cairn task readiness <task-id>
#   READINESS ready — 4 verified · 0 unverified · 0 blocked · 0 pending
#   the task's status is unchanged. Complete it with:
#     cairn task update <task-id> --status done
```

---

## What was demonstrated

| Story | The user-visible result |
|---|---|
| US1 | Repetition costs budget once, and two different claims sharing a value key are never merged |
| US2 | Today's answer and July's answer, both correct, neither rewritten |
| US3 | Two agents disagree and Cairn says so instead of picking |
| US4 | A fact reports what verified it, when, at which commit — and whether Cairn checked it or an agent said so |
| US5 | Configuration moves and Cairn says the claim drifted, without editing it |
| US6 | Ten compactions later the agent still knows the goal, the state and the next step — and is told when a file moved beneath it, whoever moved it |
| US7 | Two offline machines merge to a visible conflict, clock order irrelevant; an old server strands nothing |
| US8 | A prior project's fix arrives labelled unverified here |
| US9 | Eleven applications, and Cairn still says nobody validated it independently |
| US10 | 5,000 memories and forty criteria at 800 tokens: the state survives, and what was dropped is named |
| US11 | Two sessions — or two offline machines — two criteria, both survive and both agree; readiness derived, completion still yours |
