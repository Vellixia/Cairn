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
#   reconciliation: reinforced  →  memory 0192f4…
#   subject: infrastructure.production_database (project)

cairn memory add --type fact --scope project \
  --topic-key infrastructure.production_database --value-key postgresql \
  "prod DB: postgresql"
#   reconciliation: reinforced  →  memory 0192f4…
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
#     0192f5… reinforces 0192f4…  (deterministic_rule, session 0192a1…, claude-code)
#     0192f6… reinforces 0192f4…  (deterministic_rule, session 0192b7…, codex)
```

**What you see**: one answer in the briefing, not three. All three memories still exist, each with its
own session provenance. `cairn context` spends budget once for this fact instead of three times.

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
```

```bash
cairn memory search --topic-key service.api_port
#   [fact] The API listens on 8080   active  ✓ verified (configuration) 2026-08-14
```

An assertion of importance is not evidence:

```bash
cairn evidence add --memory <id> --kind runtime_state \
  --subject "the team thinks this matters" --value important
#   error: verifier_unavailable — no deterministic verifier for that subject
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
#       recorded at abc123 on main, task revision 7
#       current:      def456 on main, task revision 8
#       task changed: criterion added — AC-4 "production smoke passes"
#       files changed by another session: src/config.rs
#                                        (session 0192e9…, claude-code)
#       previous next action (may be stale):
#           "finish the retry backoff in config.rs"
```

**What you see**: Cairn does not tell you to carry on editing `config.rs` from a commit that has moved.

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
| US1 | Repetition becomes one answer with reinforcement, not three competing truths |
| US2 | Today's answer and July's answer, both correct, neither rewritten |
| US3 | Two agents disagree and Cairn says so instead of picking |
| US4 | A fact reports what verified it, when, and at which commit |
| US5 | Configuration moves and Cairn says the claim drifted, without editing it |
| US6 | Ten compactions later the agent still knows the goal, the state and the next step |
| US7 | Two offline machines merge to a visible conflict, clock order irrelevant |
| US8 | A prior project's fix arrives labelled unverified here |
| US9 | Eleven applications, and Cairn still says nobody validated it independently |
| US10 | 5,000 memories, 800 tokens, and nothing critical is lost |
| US11 | Two sessions, two criteria, both survive; readiness derived, completion still yours |
