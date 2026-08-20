# Quickstart: Cairn Project Intelligence

**Feature**: `003-project-intelligence`

The user-visible result, demonstrated on a real repository. One section per user story; each is
independently runnable and each ends in something a developer can see.

Everything below runs offline. No server, no API key, no network.

Every block below was produced by running the command against a real repository with a real daemon
(T146). **The vocabulary is normative; the identifiers and the spacing are not.** Identifiers are
elided as `0192f4…` throughout, counts and timestamps are whatever the walk produced, and a reader
checking this document against a run should score the outcome words, the field names and the shape —
not a full UUID where an ellipsis stands.

## Prerequisites

```bash
cargo build --workspace --release
export PATH="$PWD/target/release:$PATH"

cd ~/src/your-git-repo
cairn init
cairn connect claude-code --yes      # or codex, opencode
```

`connect` writes to your agent's configuration, so it asks first: without `--yes` it refuses with
`confirmation_required` and offers `--dry-run`.

`cairn status` is the project at a glance, including how far subject identity has actually reached
here (FR-499) and what wants attention:

```bash
cairn status
#   Project      your-git-repo (0192c0…)
#   Sharing      local only
#   Worktree     /Users/you/src/your-git-repo
#   Branch       main @ abc1234
#   Working tree 0 staged, 0 unstaged, 2 untracked
#   Integration  claude-code-hooks
#   Daemon       running
#   Recorded     1204 observations, 412 memories
#   Subjects     9% of project memory (38 of 412)
#   Attention    2 conflicted · 1 drifted
#   Sessions     none active
```

`Subjects` and `Attention` appear only when there is something to say: a project with no subjects has
no share to report, and one with nothing conflicted or drifted has nothing to draw attention to.

An existing Cairn store upgrades in place at first open. The schema version, and what each connected
agent's continuity actually is, come from `cairn doctor`:

```bash
cairn doctor
#   core        cli 0.1.0-alpha.4 · daemon 0.1.0-alpha.4 · schema 5 · project registered
#
#   claude-code  MCP_PLUS
#                continuity: agent_initiated — Cairn is warned before compaction but not
#                called back after; the agent must ask for context with reason=post_compaction
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
#   → if this is the same claim: cairn memory reinforce 0192f4… --from 0192f7…

# `--from` names the statement doing the confirming — the memory just written.
# Reinforcement is always one session saying *it* found the memory still true,
# never an inference (FR-321).
cairn memory reinforce 0192f4… --from 0192f7…
#   Reinforced. reinforcements 1 · distinct origins 2
```

Cairn will not decide for you that two differently-worded statements are one claim — that needs reading
them, which is judgement, not a rule. It tells you which memory yours agrees with and makes deciding
one call. If you skip the call, nothing is lost: both stand, and the briefing shows one plus a count.

**Reinforcing is not merging.** It records that a session found the memory still true, which is a real
act with a real author, and both statements remain answers — the subject stays `corroborated`. If you
have read both and they really are the same claim, the call that says so is `reconcile`:

```bash
cairn memory reconcile --from 0192f7… --to 0192f4… \
  --relation duplicates --basis explicit_agent \
  --rationale "same claim, different wording"
#   Decision recorded.
```

Now the subject has one answer and the other statement is accounted for as its duplicate — still
individually retrievable, with its own provenance.

An **identical** statement needs no call at all:

```bash
cairn memory add --type fact --scope project \
  --topic-key infrastructure.production_database --value-key postgresql \
  "Production runs PostgreSQL 16"
#   reconciliation: duplicate
#   identical to memory 0192f4… after normalization — recorded as a duplicate
```

Ask what the project holds:

```bash
cairn memory subject infrastructure.production_database
#   # infrastructure.production_database (project:0192c0…)
#
#   **Reconciliation**: reinforced
#
#   ## Answers
#   - postgresql — "Production runs PostgreSQL 16" `0192f4…` — unverified · 2 duplicate statements · 3 distinct origins
#
#   ## Decisions
#   - reinforces `0192f7…` → `0192f4…` (explicit_agent)
#   - duplicates `0192f7…` → `0192f4…` (explicit_agent)
#   - duplicates `0192f6…` → `0192f4…` (deterministic_rule)
```

### A coarse value key does not merge two different claims

```bash
cairn memory add --type decision --scope project \
  --topic-key auth.strategy --value-key jwt "JWT uses HS256 with a shared secret"
cairn memory add --type decision --scope project \
  --topic-key auth.strategy --value-key jwt "JWT uses RS256 with rotating public keys"
#   reconciliation: corroborating
#   agrees on value `jwt` with memory 0192h1…, but the wording differs
#   → if this is the same claim: cairn memory reinforce 0192h1… --from 0192h2…

cairn memory subject auth.strategy
#   # auth.strategy (project:0192c0…)
#
#   **Reconciliation**: corroborated
#
#   The value is agreed and the statements are several: 2 distinct statements.
#
#   ## Answers
#   - jwt — "JWT uses HS256 with a shared secret" `0192h1…` — unverified
#   - jwt — "JWT uses RS256 with rotating public keys" `0192h2…` — unverified
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
#   reconciliation: created
```

Nothing is reported, because nothing happened: a task-scoped answer and a project-scoped one are not
simultaneously applicable, so there is no conflict to detect.

```bash
```

```bash
cairn memory subject infrastructure.production_database --scope task --scope-key <task-id>
#   # infrastructure.production_database (task:0192t1…)
#
#   **Reconciliation**: settled
#
#   ## Answers
#   - sqlite — "This fixture suite uses SQLite in-memory" `0192s1…` — unverified

cairn memory subject infrastructure.production_database
#   # infrastructure.production_database (project:0192c0…)
#
#   **Reconciliation**: reinforced
#
#   ## Answers
#   - postgresql — "Production runs PostgreSQL 16" `0192f4…` — unverified · 2 duplicate statements · 3 distinct origins
```

**What you see**: inside that task the answer is SQLite; everywhere else it is PostgreSQL. No false
alarm.

---

## US2 — Knowledge evolves safely

```bash
cairn memory supersede --memory-id 0192f4… --type decision --scope project \
  --topic-key infrastructure.production_database --value-key cockroachdb \
  "Migrated production to CockroachDB in the 2026-08 migration"
#   Remembered 0192k1….
#     supersedes 0192f4…
```

The subject settles on the replacement. The statements that duplicated the memory you replaced follow
it into history — Cairn had already decided they were not separate claims, so promoting them to
competitors now would manufacture a conflict out of nothing (FR-321):

```bash
cairn memory subject infrastructure.production_database
#   **Reconciliation**: settled
#
#   ## Answers
#   - cockroachdb — "Migrated production to CockroachDB in the 2026-08 migration" `0192k1…` — unverified

cairn memory search --topic-key infrastructure.production_database
#   0192k1…  [decision/project] Migrated production to CockroachDB in the 2026-08 migration
#       active · value: cockroachdb
#       from claude-code session 0192a1… · 0 evidence
```

Ask what was true before, at any instant Cairn recorded:

```bash
cairn memory search --topic-key infrastructure.production_database --as-of 2026-08-01T00:00:00Z
#   as_of 2026-08-01T00:00:00Z — what this project believed then, not now
#
#   0192f4…  [fact/project] Production runs PostgreSQL 16
#       superseded · value: postgresql
#       from claude-code session 0192a1… · 0 evidence
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
#   → both stand until somebody decides: cairn memory reconcile --from <id> --to <id> --relation supersedes
```

```bash
cairn context
#   ## Warnings
#   1 conflict
#   ⚠ CONFLICT deploy.queue_backend — 2 competing answers (rabbitmq, sqs) and no recorded decision

cairn memory subject deploy.queue_backend
#   **Reconciliation**: conflicted
#
#   ⚠ 2 competing answers, and no winner. Resolve by superseding one, narrowing its scope,
#     or attaching verification that distinguishes them.
#
#   ## Answers
#   - sqs — "Deploys queue through SQS" `0192c1…` — unverified
#   - rabbitmq — "Deploys queue through RabbitMQ" `0192c4…` — unverified
```

**What you see**: both, attributed, with no winner picked. Resolve it deliberately:

```bash
cairn memory reconcile --from <rabbitmq-id> --to <sqs-id> \
  --relation supersedes --basis explicit_user \
  --rationale "SQS was decommissioned in July"
#   Decision recorded.

cairn memory subject deploy.queue_backend
#   **Reconciliation**: settled
#
#   ## Answers
#   - rabbitmq — "Deploys queue through RabbitMQ" `0192c4…` — unverified
```

---

## US4 — Evidence-backed knowledge

```bash
cairn memory add --type fact --scope project \
  --topic-key service.api_port --value-key 8080 "The API listens on 8080"

cairn evidence add --memory <id> --type configuration \
  --subject "API port" --value 8080 --locator "config/app.yml#server.port"
#   Evidence 0192d1… recorded.  collector: cairn

cairn verify --memory <id> --explain
#   ✓ verified                      (authority: cairn)
#
#   ## Runs
#   - configuration → verified at 2026-08-14T09:12:04Z (on_demand)
```

A configuration locator names its key after a `#`. Without one there is nothing to compare, and the
run says so rather than failing silently:

```bash
cairn verify --memory <id>
#   · unverified
#     last run: configuration → inconclusive
#     the locator names no key
```

```bash
cairn memory search --topic-key service.api_port
#   0192e1…  [fact/project] The API listens on 8080
#       active · value: 8080 · verified (cairn)
#       from claude-code session 0192a1… · 0 evidence
```

### Attested evidence is useful, and never wears the same badge

Cairn cannot call a staging API. An agent can, and telling Cairn the result is worth recording:

The attestation **is** the act that establishes the claim — Cairn will never re-collect it, so there is
nothing else to run:

```bash
cairn evidence add --memory <id> --type runtime_state --collector agent \
  --subject "GET /health version field" --value "2.4.1" --locator config/app.yml
#   Evidence 0192d9… recorded.  collector: agent

cairn memory show <id>
#   verification  verified (attested)
```

Asking Cairn to re-check it says exactly what it can and cannot do, and leaves a recheck owed:

```bash
cairn verify --memory <id>
#   · needs_recheck
#     last run: runtime_state → inconclusive
#     attested evidence is not re-collected; the agent must attest again
```

Attesting again pays it off — which is what stops an attested claim becoming permanently
unfalsifiable *and* stops it becoming permanently stale:

```bash
cairn evidence add --memory <id> --type runtime_state --collector agent \
  --subject "GET /health version field" --value "2.4.1" --locator config/app.yml
cairn memory show <id>
#   verification  verified (attested)
```

It is `verified`, and it says how. The two places where that difference decides something both refuse it:

```bash
cairn task criterion verify <ac-id> --evidence <attested-evidence-id>
#   cairn: attested_not_sufficient: that criterion is not verified: no deterministic check
#          this machine ran over Cairn-collected evidence established it

cairn pattern promote --memory <id> --dry-run --signal … --applies-when … --approach …
#   cairn: attested_not_sufficient: the source is verified by an agent's attestation;
#          promotion requires a deterministic check Cairn ran itself
```

---

## US5 — Drift detection

```bash
sed -i '' 's/port: 8080/port: 9000/' config/app.yml
cairn verify --memory <id>          # or wait for the maintenance pass

cairn verify --memory <id>
#   · drifted
#     last run: configuration → drifted
#     the configuration value differs from what was recorded

cairn memory search --topic-key service.api_port
#   0192e1…  [fact/project] The API listens on 8080
#       active · value: 8080 · drifted
#       from claude-code session 0192a1… · 0 evidence
```

```bash
cairn context
#   ## Warnings
#   1 drift
#   ⚠ DRIFT service.api_port — remembered "The API listens on 8080" — its evidence moved
```

The memory content is untouched. Cairn did not rewrite it and did not invent a replacement — the
lifecycle state and the verification state are separate axes, and only the second moved:

```bash
cairn memory show <id>
#   content       The API listens on 8080        ← unchanged
#   id            0192e1…
#   type          fact · project
#   state         active
#   verification  drifted
#   evidence      1 fact(s) · configuration
#   subject       service.api_port = 8080
#                 settled · a canonical answer
#   from          claude-code session 0192a1…
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
cairn session start --agent claude-code --task <task-id>
```

Work normally. Cairn captures. At each compaction boundary the adapter fires
`context_compacting`, and Cairn writes a checkpoint. After the tenth:

```bash
cairn context
#   # Cairn context
#
#   **Project**: your-git-repo
#   **Repository**: branch `main`, commit `def4567…`, working tree 0 staged, 3 unstaged, 0 untracked
#
#   ## Constraints
#   - Never mutate CC Switch's private database directly
#
#   ## Task: Retry backoff (todo)
#   Transient failures retry with jitter
#
#   Acceptance criteria:
#   - AC-1 satisfied · verified — backoff is exponential with jitter
#   - AC-3 satisfied · unverified — cargo test --workspace passes
#   - AC-2 pending · unverified — the retry cap is configurable
#
#   Progress: 1 verified · 1 satisfied but unverified · 0 blocked · 1 pending
#   Readiness: not_ready
#
#   ## Previous session
#   Next step: add the configurable cap to RetryConfig
#
#   ## Known failures
#   - A fixed 100 ms sleep — thundering herd under load
#
#   ---
#   continuity agent_initiated · checkpoint restored 10 time(s)
```

Both axes on every criterion, never collapsed: what somebody *asserted* about it and what Cairn
*checked* are different claims (FR-483). The progress counts and the readiness are O(1) in the size of
the task, which is what makes them survivable at any budget.

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
#       recorded at abc123456789
#       now at      def456789abc
#       the task changed since the checkpoint
#       files changed: src/config.rs
#         src/config.rs  (changed, digest)
#       previous next action (may be stale):
#           "finish the retry backoff in config.rs"
#
#   # Cairn context
#   …
#   ## Warnings
#   1 task_divergence
#   ⚠ TASK_DIVERGENCE AC-4 — criterion added — "production smoke passes" (this_machine)
```

The divergence leads, before anything written against the state that moved. The recorded action is
labelled **previous**, never `next`: acting on a stale instruction confidently is the failure this
tier exists to prevent (FR-434).

**What you see**: Cairn does not tell you to carry on editing `config.rs` from a commit that has moved.

### A change nobody told Cairn about

Now do it with no Cairn session at all — edit in your editor, run a formatter, apply a patch — and do
not commit:

```bash
$EDITOR src/config.rs                 # or: cargo fmt, or git apply, or an IDE refactor
git status --short                     # M src/config.rs — commit unchanged

cairn context --reason post_compaction
#   ⚠ CHECKPOINT DIVERGED
#       files changed: src/config.rs
#         src/config.rs  (changed, digest)
#       previous next action (may be stale):
#           "finish the retry backoff in config.rs"
```

The commit is unchanged, so no commit divergence is reported — only the fingerprint that moved.

**What you see**: the checkpoint compares the fingerprint it recorded, so it does not matter who made
the change or whether Cairn was watching. And a path it cannot fingerprint says so rather than
pretending:

```bash
cairn session checkpoint
#   Checkpoint recorded over 8 relevant paths.
```

The relevant paths are the ones this session actually touched, taken from what Cairn captured — a
session that has touched nothing checkpoints over none, and says so.

### An agent without a post-compaction signal

```bash
cairn doctor
#   claude-code  MCP_PLUS
#                continuity: agent_initiated — Cairn is warned before compaction but not
#                called back after; the agent must ask for context with reason=post_compaction
#
#   generic-mcp  MCP_ONLY   (no automatic session start)
#                continuity: unavailable_automatic — this agent reports no compaction event;
#                write a checkpoint with cairn_session action=checkpoint before you compact
```

Honest, and specific about what to do instead. **Cairn may under-promise a capability; it must never
claim a continuity mode the integration cannot deliver.** Claude Code writes the checkpoint before a
compaction and has no supported channel to return context afterwards, so it reports
`agent_initiated` — not `automatic`.

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
#   **Reconciliation**: conflicted
#
#   ⚠ 2 competing answers, and no winner. Resolve by superseding one, narrowing its scope,
#     or attaching verification that distinguishes them.
#
#   ## Answers
#   - postgresql — "prod is PostgreSQL" `0192f1…` — unverified
#   - cockroachdb — "prod is CockroachDB" `0192f8…` — unverified
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
#   **Reconciliation**: settled
#
#   ## Answers
#   - cockroachdb — "prod is CockroachDB" `0192f8…` — unverified
#
#   ## Decisions
#   - supersedes `0192f8…` → `0192f1…` (explicit_user)
```

### A peer's verification says how it was established

```bash
# machine B, reading a memory machine A verified
cairn memory search --topic-key service.api_port
#   0192e1…  [fact/project] The API listens on 8080
#       active · value: 8080 · verified (remote_cairn)
#       from unknown session 0192a1… · 0 evidence

# and one machine A only attested
cairn memory search --topic-key service.version
#   0192v1…  [fact/project] The service reports 2.4.1
#       active · value: 2.4.1 · verified (remote_attested)
#       from unknown session 0192a1… · 0 evidence
```

On machine A itself both keep the authority A established — a round trip through the server never
turns a check this machine ran into someone else's.

**What you see**: two verifications that would have looked identical now do not. Neither counts toward
this machine's task readiness, and neither can be promoted from here.

### An old server, and then an upgraded one

```bash
# server still on Feature 001's schema
cairn sync now
cairn sync status
#   Linked       yes
#   Pending      0
#   Failed       0
#   Last success 2026-08-14T09:14:02Z
#   Blocked      3 (waiting for: memory_relations, memory_subject_identity or memory_verification)
#     server: schema=1;capabilities=
#     3 item(s) are waiting for this server to gain memory_relations, memory_subject_identity or
#     memory_verification. Everything else syncs normally (0 queued), nothing has been lost, and the
#     retained work is delivered automatically once the server is upgraded.

cairn status
#   Retained     3 item(s) waiting for: memory_relations, memory_subject_identity or memory_verification
```

Nothing is lost and nothing is retried pointlessly. Upgrade the server, and the next drain cycle notices:

```bash
# after the server is upgraded
cairn sync now
cairn sync status
#   Linked       yes
#   Pending      0
#   Failed       0
#   Last success 2026-08-14T11:02:18Z

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
cairn evidence add --memory <id> --type configuration \
  --subject "docker default address pools" --value expanded \
  --locator "config/docker.yml#default-address-pools"
cairn verify --memory <id>
#   ✓ verified                      (authority: cairn)

cairn pattern promote --memory <id> \
  --signal "could not find an available non-overlapping ipv4 address pool" \
  --signal "docker bridge network create failure" \
  --applies-when "Docker bridge networking is in use" \
  --applies-when "the error names address-pool allocation" \
  --root-cause "the daemon's default-address-pools are fully allocated" \
  --approach "expand default-address-pools in the daemon configuration and restart" \
  --caveat "existing networks are not migrated to the new pool"
#   promoted 0192g1…
#     Expand Docker's default-address-pools when bridge network creation fails with an address-pool error

cairn pattern list
#   0192g1…  Expand Docker's default-address-pools when bridge network creation fails…
#     trust sanitized · applications 0 · distinct projects 0 · independently validated in 0 · counterexamples 0
```

Now in project B, months later, the same error surfaces and Cairn captures it:

```bash
cairn context
#   ## Patterns from other projects (unverified here)
#   - **Expand Docker's default-address-pools when bridge network creation fails with an
#     address-pool error** (sanitized, 10 signals matched): expand default-address-pools in the
#     daemon configuration and restart
```

A pattern is offered under its own heading and never mixed into this project's memory — a separate
array on the wire, and a separate section here, so it cannot be read as something this project knows.

**What you see**: the prior fix, offered — and explicitly not claimed to be true here.

### Promotion refuses what it should

The gate runs in order, and the first thing it asks is whether the source is verified at all — a
pattern starts from a check, not from a claim:

```bash
cairn pattern promote --memory <unverified-id> --dry-run --signal … --applies-when … --approach …
#   cairn: source_unverified: the source memory is not verified; a pattern starts from a
#          check, not from a claim
```

Given a verified source, the rest of the gate still refuses what it should:

```bash
cairn pattern promote --memory <id-of-a-project-fact> --dry-run …
#   cairn: not_transferable: this memory states what this project is configured with rather
#          than a problem and its resolution, so it transfers nowhere

cairn pattern promote --memory <id-with-a-path> --dry-run …
#   cairn: project_identifying: this candidate names an absolute filesystem path, which a
#          pattern must never carry

cairn pattern promote --memory <local-only-id> --dry-run …
#   cairn: local_only_memory: the source memory is local-only; a pattern derived from it
#          would be that memory travelling under another name

cairn pattern promote --memory <superseded-id> --dry-run …
#   cairn: source_not_active: the source memory is no longer active; promoting it would
#          export a conclusion this project has already replaced
```

The privacy refusal names the class and never echoes the value it found.

---

## US9 — Counterexamples prevent poisoning

In project B the symptom matches but the cause is different:

```bash
cairn pattern outcome 0192g1… --outcome not_applicable \
  --alternative-cause "a VPN route collision produced the same error; the pools were not exhausted"
#   recorded not_applicable (independent)
#     trust contested · applications 1 · distinct projects 1 · independently validated in 0 · counterexamples 1
```

Next time it surfaces:

```bash
cairn context
#   ## Patterns from other projects (unverified here)
#   - **Expand Docker's default-address-pools…** (contested, 10 signals matched): expand
#     default-address-pools in the daemon configuration and restart
#     - another cause found behind this: a VPN route collision produced the same error; the
#       pools were not exhausted
#     - check first: Docker bridge networking is in use
```

Repetition does not manufacture trust:

```bash
# ten sessions in project A each record the same incident resolved
cairn pattern show 0192g1…
#   trust contested · unverified in any project but where it was applied
#   applications 2 · distinct projects 2 · independently validated in 0 · counterexamples 1
```

**What you see**: ten repetitions in the origin project did not become ten applications — the
accounting is by project, not by how often somebody pressed the button — and Cairn still says nobody
has independently validated it. A pattern's own project cannot validate it (FR-402).

---

## US10 — Minimum safe context

A project with thousands of memories and a tight budget.

```bash
cairn memory search --limit 1 --json | jq '.total'      # 5000+

cairn context --token-budget 300
#   # Cairn context
#
#   **Repository**: branch `main`, commit `def4567…`, working tree 0 staged, 3 unstaged, 0 untracked
#
#   ## Warnings
#   1 conflict · 1 drift
#   ⚠ CONFLICT deploy.queue_backend — 2 competing answers (rabbitmq, sqs) and no recorded decision
#   ⚠ DRIFT service.api_port — remembered "The API listens on 8080" — its evidence moved
#
#   ## Constraints
#   - Never mutate CC Switch's private database directly
#
#   ## Task: Retry backoff (todo)
#   Acceptance criteria:
#   - AC-1 satisfied · verified — backoff is exponential with jitter
#   - AC-2 pending · unverified — the retry cap is configurable
#
#   Progress: 1 verified · 0 satisfied but unverified · 0 blocked · 1 pending
#   Readiness: not_ready
#
#   ---
#   280 of 300 estimated tokens; omitted: project_memory
```

**What you see**: at 800 tokens with 5,000 memories present, everything you cannot work without
survives, and Cairn says exactly what it dropped.

Why:

```bash
cairn context --token-budget 300 --explain
#   budget 300 · reserve 120 · reserve used 120 · released 0
#   INCLUDED
#     minimum_safe  repository scope_match                          26
#     minimum_safe  warning    conflict_warning                     41
#     minimum_safe  warning    drift_warning                        33
#     minimum_safe  task       task_binding                         39
#     minimum_safe  constraint pinned                               17
#     minimum_safe  criterion  task_binding                         15
#   OMITTED
#     memory ×12       budget_exhausted  — `cairn memory search`
```

Every omission carries a reason, and the count with the call that retrieves what went (FR-461).

### A task with forty criteria, and a budget that cannot hold them

```bash
cairn context --token-budget 400
#   ## Task: Retry backoff (todo)
#   Transient failures retry with jitter
#
#   Acceptance criteria:
#   - AC-1 satisfied · unverified — backoff is exponential with jitter
#   - AC-3 satisfied · unverified — cargo test --workspace passes
#   - AC-2 pending · unverified — the retry cap is configurable
#   …
#     (+31 more — `cairn task show 0192t9…`)
#
#   Progress: 0 verified · 2 satisfied but unverified · 0 blocked · 38 pending
#   Readiness: not_ready
#   Blocked by: staging credentials expired (+3 more open)
#
#   ---
#   387 of 400 estimated tokens; omitted: previous_handoff, project_memory

**What you see**: forty criteria do not fit in 800 tokens and Cairn does not pretend otherwise. What is
guaranteed is the *state* — goal, progress counts, readiness, the blocker that matters, the next action,
the warnings, the constraints — none of which grows with the task. Criterion text is admitted
blocked-first, and what did not fit is counted with the call that retrieves it.
```

---

## US11 — Evidence-aware tasks

```bash
cairn task show <task-id>
#   Release readiness
#   The release gate is evidence-backed
#   Status: todo
#   Revision: 6 (local)
#   State:    ff82530e6348c364
#   Acceptance criteria:
#     AC-1  satisfied · verified  (rev 3)  the config port is 9000
#     AC-2  pending · unverified  (rev 1)  the docker pools are expanded
#   BLOCKER   staging credentials expired
#   PROGRESS  1 verified · 0 satisfied but unverified · 0 blocked · 1 pending
#   BLOCKERS  1 open
#   READINESS not_ready
```

A criterion Cairn checked reads `pending · verified` until somebody asserts it is satisfied: the two
axes are independent, and only one of them is Cairn's to write.

```bash
cairn evidence add --type configuration --subject "API port" --value 9000 \
  --locator "config/app.yml#server.port"
cairn task criterion verify <ac-1-id> --evidence <evidence-id>
#   AC-1  pending · verified  (rev 2)  the config port is 9000

cairn task blocker open <task-id> --description "staging credentials expired"
#   Blocker opened.
```

Two sessions, two criteria, no lost work:

```bash
# session A                                       # session B
cairn task criterion set <ac-2-id> \              cairn task criterion set <ac-3-id> \
  --state satisfied --expected-revision 7           --state satisfied --expected-revision 7

cairn task show <task-id>
#   Revision: 9 (local)
#     AC-2  satisfied · unverified     ← session A's change survived
#     AC-3  satisfied · unverified     ← session B's change survived
```

### Two machines, two criteria, offline

```bash
# machine A, offline                      # machine B, offline
cairn task criterion set <ac-1-id> \      cairn task criterion set <ac-2-id> \
  --state satisfied                         --state satisfied

# both reconnect
cairn sync now                            cairn sync now

# machine A                                machine B
cairn task show <task-id>                 cairn task show <task-id>
#   Revision: 4 (local)                   #   Revision: 2 (local)   ← differs, and is never compared
#   State:    c48cd474e7b479e7            #   State:    c48cd474e7b479e7   ← identical
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
