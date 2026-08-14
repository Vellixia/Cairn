# Contract: Evaluation Harness

**Feature**: `003-project-intelligence`

This feature cannot be judged by "feels smarter". Every claim it makes is a deterministic property
over a curated corpus, and **no release gate reads a model's judgement** (FR-511, SC-325).

## The corpus

`tests/knowledge/` — JSON fixtures, versioned in the repository, loaded by pure functions with no
database and no daemon. This is what makes the reconciliation, verification and continuity logic
testable at `cargo test -p cairn-core` speed.

```text
tests/knowledge/
├── reconciliation/
│   ├── equivalent/          ≥20 cases — content identical after normalization → one answer
│   ├── distinct/            ≥20 PAIRED cases — superficially similar, must NOT merge
│   ├── coarse_value_key/    ≥15 ADVERSARIAL cases — one topic + one value key, materially
│   │                        different statements (HS256/RS256, port 8080 tcp/udp,
│   │                        "postgres 14"/"postgres 16") → Corroborated, never merged
│   ├── duplicate_content/   identical normalized content, differing whitespace/case/punctuation
│   └── free_form/           no topic key — must never merge, reinforce or supersede
├── conflict/
│   ├── real/                ≥15 — same scope, same key, incompatible value
│   ├── scope_exception/     ≥10 — project vs task, project vs branch (must NOT conflict)
│   └── disjoint/            ≥10 — branch/main vs branch/feature, task T1 vs T2 (must NOT interact)
├── supersession/            history preservation, `as_of` answers, chains ≥3 deep
├── merge/                   two-store offline scenarios, each with a clock-reversed twin
│   ├── symmetric_relation/  the same conflict detected independently on both stores
│   ├── task_divergence/     different criteria changed offline on each store
│   └── blocked_recovery/    capability refusal, server upgrade, delivery
├── verification/            every documented transition, and every undocumented one as a negative
│   └── authority/           cairn · attested · remote_cairn · remote_attested, and every
│                            strict consumer's refusal
├── drift/                   fingerprint change → recheck → verified | drifted | inconclusive
├── budget/                  memory populations 0 · 10 · 500 · 5000 × budgets 200 … 12000
│   └── oversized_task/      tasks with 5 · 40 · 200 criteria whose text exceeds the budget
├── continuity/              ten-cycle compaction traces; field-presence assertions per cycle
├── staleness/               every divergence class, alone and in combination
│   └── external_edit/       relevant paths changed with no Cairn session: editor, formatter,
│                            git apply, IDE refactor; plus excluded and oversized paths
├── patterns/
│   ├── promote/             gate-passing candidates
│   ├── refuse/              one case per refusal class, ten classes
│   ├── independence/        1 project × 10 sessions; 3 projects × 1 session; suggested-only
│   └── counterexample/      not_applicable with an alternative cause
├── privacy/                 seeded adversarial corpus (see below)
└── tasks/                   concurrent criterion updates, revision divergence, readiness
```

**Paired positive/negative is the design.** "Zero false merges" is only measurable against cases that
*look* mergeable and must not be. Every `equivalent/` case has a sibling in `distinct/` differing in
exactly the way that matters.

### The adversarial privacy corpus

`tests/knowledge/privacy/` seeds one case per class the promotion gate must refuse:

```text
provider keys      sk-…, ghp_…, github_pat_…, glpat-…, xoxb-…, AKIA…, ASIA…, AIza…
structured         PEM private key block, JWT, bearer credential
connection strings postgres://user:pass@…, mongodb+srv://…, redis://…
key=value          API_KEY=…, "token": "…", PASSWORD=…
absolute paths     /Users/x/src/repo, /home/x/repo, C:\Users\x\repo, \\server\share
project identity   project name in 4 casings, repository_remote with and without credentials,
                   server_project_id, git_common_dir, a user email
```

Each asserts: refused, the refusal names the class, the refusal does **not** echo the value, and no
partial pattern exists afterwards.

## Metrics and gates

Every row is a required check unless marked otherwise.

| # | Metric | Target | Test | SC |
|---|---|---|---|---|
| 1 | Duplicate reconciliation accuracy on `equivalent/` | one canonical answer, 100% | `us1_reconciliation` | SC-301 |
| 2 | False merges on `distinct/` + `free_form/` + `coarse_value_key/` | **0** | `us1_reconciliation` | SC-301 |
| 2a | Unrequested `reinforces` relations written by any automatic path | **0** | `us1_reconciliation::no_automatic_reinforcement` | SC-301 |
| 2b | `coarse_value_key/` cases yielding `Corroborated` with every statement retained | 100% | `us1_reconciliation::corroboration` | SC-301 |
| 3 | Silent winners on `conflict/real/` | **0** | `us3_conflict` | SC-302 |
| 4 | False conflicts on `scope_exception/` + `disjoint/` | **0** | `us3_conflict` | SC-302 |
| 5 | Lost writes under 32-way concurrency | **0** | `us3_conflict::concurrent_proposals` | SC-303 |
| 6 | Offline merge: proposals preserved, conflict detected | 100% | `us7_offline_merge` | SC-304 |
| 7 | Clock-reversal invariance of the merged state | byte-identical | `clock_swap_invariance` | SC-304 |
| 8 | Supersession history byte-identical; `as_of` correct | 100% | `us2_temporal` | SC-305 |
| 8a | A transition with no authoritative instant is reported as unknown applicability, never as a bounded fact | 100% | `us2_temporal::unknown_applicability` | SC-305 |
| 9 | Verification transitions: documented reachable, undocumented unreachable | exhaustive | `us4_evidence::state_machine` | SC-306 |
| 10 | Drift sequence `verified → needs_recheck → drifted`, memory unchanged | 100% | `us5_drift` | SC-307 |
| 11 | `estimated_tokens <= budget` across the budget matrix | 100% | `us10_min_safe_context` | SC-308 |
| 12 | Tier 0a guaranteed work state present at the minimum budget with 5,000 memories | 100% | `us10_min_safe_context` | SC-309 |
| 12a | Task with 200 criteria at the minimum budget: Tier 0a complete, omission counts reported with a retrieval path, budget not exceeded | 100% | `us10_min_safe_context::oversized_task` | SC-309 |
| 12b | Criterion admission follows the documented action order | 100% | `us10_min_safe_context::action_order` | SC-309 |
| 13 | With no task, warnings, pins or checkpoint: the `briefing` object, `estimated_tokens`, `truncated` and `omitted_sections` are identical to the pre-feature output | byte-identical on those fields | `us10_min_safe_context::no_regression` | SC-308 |
| 14 | Continuity fields present after each of ten cycles | 100% | `us6_continuity` | SC-310 |
| 15 | Divergence detection per class and in combination | 100% | `us6_continuity::staleness` | SC-311 |
| 15a | Path change detected with no Cairn session involved and the commit unmoved | 100% | `us6_continuity::external_edit` | SC-311 |
| 15b | A path that cannot be fingerprinted is reported as such, never as unchanged | 100% | `us6_continuity::not_fingerprintable` | SC-311 |
| 16 | Diverged checkpoint never emits a live `next_action` | 100% | `us6_continuity::staleness` | SC-311 |
| 17 | Pattern suggestion labelled unverified in the receiving project | 100% | `us8_patterns` | SC-312 |
| 18 | Counterexample increases no success count; pattern retained | 100% | `us9_counterexamples` | SC-313 |
| 19 | Independence: 10 sessions in 1 project → distinct count 1 | exact | `us9_counterexamples` | SC-314 |
| 20 | `cairn_suggested` without local evidence does not validate | 100% | `us9_counterexamples` | SC-314 |
| 21 | Promotion refuses every seeded privacy violation, echoes nothing | 100% | `privacy_promotion` | SC-315 |
| 22 | No forbidden field or entity type accepted on the wire | 100% | `privacy_payloads` (extended) | SC-316 |
| 23 | Concurrent criterion updates both persist | 100% | `us11_task_criteria` | SC-317 |
| 24 | Revision divergence reported with its change list | 100% | `us11_task_criteria` | SC-318 |
| 25 | Criterion never `verified` on attested evidence alone, nor on any imported verification | 100% | `us11_task_criteria` | SC-328 |
| 25a | Promotion refuses a source verified only by attestation | 100% | `privacy_promotion::attested_source` | SC-328 |
| 25b | Authority preserved across sync; no surface renders attested and deterministic alike | 100% | `us7_offline_merge::authority_survives` | SC-329 |
| 25c | Authority derivation prefers the deterministic basis when both exist | 100% | `us4_evidence::authority` | SC-329 |
| 26 | Session-open and capture latency within Feature 001 budgets, loaded | 100% | `perf_intelligence` | SC-319 |
| 27 | No verification work on the session-open path | 0 runs | `perf_intelligence` | SC-320 |
| 28 | Every D75 bound asserted at its default | exhaustive | `bounds` | SC-320 |
| 29 | No model client, embedding, vector or graph dependency | absent | `ci_hermeticity` (extended) | SC-321 |
| 30 | alpha.4 migration: rows lost 0, rows rewritten 0, defaults correct | exact | `migration_alpha4` | SC-322 |
| 31 | Feature 001 + 002 suites pass unchanged; six tools; no observation entity type | 100% | existing suites | SC-323 |
| 32 | Every derived value equals its rebuild | 100% | `rebuild_equivalence` | SC-324 |
| 33 | Relation application order does not change the derivation | 100% | `relation_order_invariance` | SC-324 |
| 34 | Degraded sync against a schema-1 server delivers everything accepted, and retains the rest as `blocked` | 100% | `sync_degradation` | SC-326 |
| 34a | Blocked work is delivered after a server upgrade, exactly once, with no user action | 100% | `sync_degradation::recovers_after_upgrade` | SC-331 |
| 34b | Blocked work is never retried against a server known to lack the capability | 0 retries | `sync_degradation::no_futile_retry` | SC-326 |
| 34c | Offline task divergence: both criterion changes present, identical state digest on both stores | 100% | `us11_task_criteria::offline_convergence` | SC-330 |
| 34d | One symmetric conflict detected on two stores converges to exactly one durable relation | exact | `clock_swap_invariance::symmetric_relation` | SC-324 |
| 35 | No scope, partition or filter added; no importance/pin/verification scope override | 100% | `scope_audit` (extended) | SC-327 |
| 36 | MCP: Feature 001/002 calls byte-identical in pre-existing fields | 100% | `mcp_backward_compatibility` | SC-323 |

## Tiers

The five tiers Feature 002 established (D40), unchanged in shape.

| Tier | Runs | Needs | In CI |
|---|---|---|---|
| 1. Unit | `cargo test -p cairn-core -p cairn-store` | nothing | required |
| 2. Corpus | `cargo test -p cairn-core --test knowledge` | the JSON corpus | required |
| 3. Integration | `cargo test -p cairn-e2e` | real repositories, real SQLite, real daemon | required |
| 4. Shared | same, with `CAIRN_TEST_DATABASE_URL` | PostgreSQL | required |
| 5. Live agent | manual | an installed, authenticated agent | **release evidence only** |

Tier 2 is new and is where most of Feature 003's correctness lives, because the reconciliation
derivation, the verification state machine and the staleness comparison are pure functions.

Network-isolated running is unchanged: the local product runs with loopback only, which is the proof
of FR-477 (fully offline).

## Performance measurement

`perf_intelligence` measures against a **loaded** project — the scale FR/SC state — rather than an
empty one:

```text
5,000 memories · 500 topic-keyed subjects · 10,000 evidence facts
1,200 relations · 200 verification runs · 40 patterns · 30 tasks × 6 criteria
```

| Measured | Budget | Baseline |
|---|---|---|
| Capture hook, per adapter | ≤10 ms median, ≤25 ms p95, inside 250 ms | Feature 001/002 unchanged |
| Session open, context assembled | inside the 1,500 ms context deadline | Feature 001 unchanged |
| Session close | inside the adapter's own budget | Feature 002 SC-128 unchanged |
| Subject warning derivation | bounded by `subject_warning_scan_max` | new |
| Drift marking per observation | ≤8 indexed lookups | new |
| Background verification pass | ≤200 facts, ≤50 runs, ≤2,000 ms | new |
| Verification runs during session open | **0** | new |

A saturated host is an **invalid measurement**, not a failure — the correction recorded in
`docs/feature-001-followups.md` §6 applies here too.

## Topic-key effectiveness — informational, never a gate

Everything above measures whether the deterministic engine is **correct**. None of it measures whether
the engine ever **activates**, and that is a separate question with a separate answer.

Reconciliation only fires on memories that carry a subject. If real agents write
`database.prod`, `prod.database`, `infrastructure.database` and `production.db` for one concept, then
Cairn can be perfect and still reconcile almost nothing. That risk is named in the plan; this is how it
is observed rather than assumed.

```text
evals/topic-key-effectiveness/       INFORMATIONAL · RELEASE EVIDENCE
                                     NOT a CI gate · NOT a deterministic correctness gate
├── corpus.md      a curated set of durable project facts, in prose, per project archetype
├── protocol.md    how to run it: fresh session per agent, the corpus prompts, what to record
├── RESULTS.md     dated findings per agent and per release
└── analysis.md    what the numbers mean and what would change the design
```

Run manually against **Claude Code, Codex and OpenCode**, each with its native integration, against the
same curated corpus.

| Measure | Definition |
|---|---|
| Topic-key adoption rate | share of durable project facts recorded with a `topic_key` |
| Value-key specificity | share of value keys that fully determine the proposition (judged by the reviewer, recorded per case) |
| Same-fact cross-session consistency | for one fact recorded in *n* sessions of one agent, the number of distinct topic keys used |
| Cross-agent consistency | for one fact recorded by each agent, the number of distinct topic keys used |
| Missed grouping | facts that should share a subject and did not |
| False grouping | facts that share a subject and should not — expected to be **0**, since only identical content merges, and any non-zero result is a design finding |
| Safely reconcilable share | facts that ended in a `Settled`, `Reinforced` or `Corroborated` subject rather than isolated |

**Boundary, stated explicitly.** This evaluation involves model behaviour, so:

- it is **never** a CI check and **never** a release gate;
- it cannot fail a build, and no threshold is defined for it;
- it produces release *evidence* — a dated table in `RESULTS.md` — and design *input*;
- its only permitted effect on the deterministic system is to propose corpus cases and prompt wording
  for a human to review.

What it is actually for: telling us whether Feature 003 improves real agent continuity, or whether the
deterministic machinery sits idle behind inconsistent keys. A low adoption rate is a **product** finding
that would send us back to the usage contract, the Skill and the tool descriptions — not to a similarity
heuristic, which D46 rejects on correctness grounds and which this measurement cannot justify.

The complementary always-on signal is the adoption share `cairn status` reports (FR-499), which gives
every user the same number for their own project without anyone running an evaluation.

## The model-judgement boundary

No gate above reads a model's output. If a semantic-equivalence evaluation is ever wanted — "would a
human call these two memories the same claim?" — it lives outside the gates with an explicit trust
boundary:

```text
evals/semantic-equivalence/        NOT a CI gate, NOT a release gate
    ├── cases.jsonl                candidate pairs drawn from real projects
    ├── run.md                     how to run it and what it does not prove
    └── RESULTS.md                 dated findings
```

Its only permitted use is to **propose corpus cases** for a human to review and add to
`reconciliation/` as deterministic fixtures. It may never gate a release, set a threshold, or change
stored state. This is the same boundary FR-512 draws for agents: intelligence proposes, Cairn decides.

## Release evidence

| Evidence | Where |
|---|---|
| Tiers 1–4 green on `ubuntu-latest` and `macos-latest` | required CI checks |
| The corpus metric table, with actual numbers | release notes |
| `migration_alpha4` against a real alpha.4 store | required CI, plus one manual run against a developer's live store |
| `perf_intelligence` numbers from both CI platforms | release notes; a saturated host is re-run |
| The quickstart, run end to end on a real repository | manual, per release |
| Tier 5 live-agent walkthrough for each connected agent | manual, per release |
| Topic-key effectiveness table for all three native agents | manual, per release; informational only |
| `cairn doctor --rebuild-derived` reporting zero differences | manual, per release |

The last row is worth stating: a release where any derived value differs from its rebuild ships a known
inconsistency, and that is a blocker rather than a note.
