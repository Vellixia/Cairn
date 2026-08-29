# Contract — Semantic Extraction

Turning a bounded set of approved safe events into candidate claims. The extractor proposes;
Cairn governs (`consolidation.md` §5).

## 1. Decision

**Feature 005 ships a deterministic, rule-based extractor as the default and only supported
baseline. No hosted model provider is selected, and none is required.**

The reasoning is constitutional, not economic. Constitution v1.2.1 Principle V makes a
third-party extractor a *second recipient at a second boundary*, requiring that it be named,
disclosed as plainly as the server connection, scoped to one project and one account context,
and not assumed compliant. That obligation is satisfiable — but not by assumption, and nothing
in the acceptance story requires a model. The rules in §4 produce the failure, decision and
procedure candidates the end-to-end scenario calls for, from event structure alone.

A model extractor is permitted behind the same trait, subject to §5.

## 2. Interface

```rust
pub struct ExtractionInput {
    pub project_ref: ProjectRef,       // scoping, bound by the caller
    pub account_ref: AccountRef,       // scoping, bound by the caller
    pub session_ref: SessionRef,
    pub events: Vec<SafeCanonicalEvent>,   // ≤200, ≤256 KiB
}

pub struct CandidateProposal {
    pub kind: MemoryKind,              // fact|decision|convention|failure|procedure
    pub content: String,               // ≤2048 bytes
    pub topic_key: String,             // PROPOSED — normalized by Cairn before any use
    pub value_key: String,             // PROPOSED
    pub source_event_ids: Vec<Uuid>,   // verified by Cairn before any use
    pub proposed_domain: KnowledgeDomain, // advisory only
}

pub trait SemanticExtractor: Send + Sync {
    fn kind(&self) -> &'static str;
    fn extract(&self, input: &ExtractionInput) -> Result<Vec<CandidateProposal>, ExtractError>;
}
```

`ExtractionInput` is the **complete description of what any extractor may see** (FR-805a). It
contains only approved safe events, already scoped to one project and one account context
(FR-805a1) — extraction is not exempt from the membership guard every other read path has.

Note what the proposal cannot express: no durability, no verification, no supersession, no
scope, no authorization, no privacy verdict. Those are not fields, so an extractor cannot
assert them even by mistake (FR-805b). Structural prevention over procedural rules.

Extraction is replaceable without changing anything else in the pipeline (FR-805f). Nothing
in Feature 005 depends on a particular extractor or on a hosted one existing.

## 3. Baseline extractor

Kind: `deterministic_v1`. Pure function of the event sequence. No network, no model, no
configuration that changes its output.

Working set per batch:

- the ordered event sequence for one session
- files touched (`file_changed` with `file_identity = present`)
- test results (`test_result` with `test_outcome`)
- failures (`tool_failed`, `failure_kind`)
- commands (`command_executed`, `command_line`)

## 4. Rules

Each rule states a pattern over the sequence, the candidate it proposes, and how its keys are
built. Keys are proposed here and normalized by Cairn afterwards (`consolidation.md` §5 gate 2).

### 4.0 Two rule tiers, and why

R1, R2 and R4 are **session rules**: they read one session's ordered events, and the extractor
evaluates them from `ExtractionInput`.

R3, R5 and R6 are **project rules**: they need evidence across sessions ("≥3 sessions", "≥2
sessions"). A session-scoped `ExtractionInput` cannot see that, and widening it to a
cross-session corpus is not available — FR-805a1 confines an extraction request to one project
and one account context, and SC-749 tests it.

Project rules are therefore evaluated by **Cairn's own deterministic aggregator**, not by the
extractor. The aggregator runs in the same consolidation pass, reads `safe_events` for the
batch's project under the same scoping, and emits candidates directly. This keeps every rule
inside one project and one account, and has a second benefit: the rules that most resemble
policy claims about a project are the ones no extractor influences at all.

### R1 — Fix confirmed by tests *(session rule)*

```
test_result(failed) … file_changed(F)+ … test_result(passed)
```
⇒ **FAILURE**: "Tests were failing and passed after changes to `F₁…Fₙ`."
`topic_key = test.<suite-token>` · `value_key = fixed_by.<primary-file-token>`

The strongest signal in the model: a failure, a change, and evidence the change worked. This
is the rule that carries the end-to-end acceptance scenario.

### R2 — Persistent failure *(session rule)*

```
tool_failed(K) ≥3 times, same failure_kind, no subsequent success
```
⇒ **FAILURE**: "`K` fails repeatedly in this project."
`topic_key = failure.<kind-token>` · `value_key = unresolved`

### R3 — Established command *(project rule — aggregator)*

```
command_executed(C) ≥3 sessions, exit_status = 0
```
⇒ **CONVENTION**: "`C` is the established command for this project."
`topic_key = command.<verb-token>` · `value_key = <command-digest>` — a bounded digest, not
the command text: `VALUE_KEY_MAX_CHARS` is 64 and `command_line` is up to 512 bytes, so a
realistic command would refuse its own candidate as `key_normalization_failed`.

### R4 — Decision near change *(session rule)*

```
decision_signal … file_changed(F)+ within the same session
```
⇒ **DECISION**: "Work in `F₁…Fₙ` followed a decision point in session `S`."
`topic_key = area.<module-token>` · `value_key = changed.<session-token>`

Deliberately weak. `decision_signal` carries no text — carrying prompt text would be a
transcript — so the claim asserts only what the events establish. R4 exists to make the
decision *locatable*, not to state its content.

### R5 — Repeated procedure *(project rule — aggregator)*

```
identical ordered command_executed sequence in ≥2 sessions, all exit_status = 0
```
⇒ **PROCEDURE**: "The sequence `C₁ → C₂ → … → Cₙ` is used to accomplish work here."
`topic_key = procedure.<first-verb-token>` · `value_key = <sequence-digest>`

### R6 — Test suite identity *(project rule — aggregator)*

```
test_executed(T) ≥2 with a consistent test_command
```
⇒ **FACT**: "`T` is the test command for this project."
`topic_key = test.command` · `value_key = <command-digest>` (same bound, same reason as R3)

### 4.1 Honest limits

R1–R6 do not read prose, do not infer intent, and do not summarize conversation. They produce
fewer and blunter claims than a model would. That is the accepted trade for v1: the knowledge
they produce is *checkable against the events that produced it*, which is what makes
provenance meaningful. Extraction quality is precisely the thing a later model extractor
improves, behind an unchanged trait.

## 5. Gate for a hosted extractor

Before any hosted extractor is enabled, Phase 0 of the work that enables it must establish,
**from the provider's current official documentation, for the actual provider, model and
endpoint in the deployment**:

| # | Must establish |
|---|---|
| 1 | Provider, model and endpoint identity |
| 2 | Customer-content retention: whether, where, how long |
| 3 | Whether submitted content is used for training or model improvement |
| 4 | Eligibility for a zero-retention or no-training mode, and how it is enabled |
| 5 | Prompt or application-state caching behaviour |
| 6 | Project and account isolation |
| 7 | The disclosure the provider requires be made to end users |
| 8 | Behaviour when a compliant mode is unavailable or silently disabled |

Rules:

- **No provider is preselected**, here or in the plan.
- **No default configuration is assumed compliant.** Retention and training behaviour are
  account-, configuration- and model-dependent (FR-805e).
- If compliance **cannot** be established, the hosted extractor is not enabled. The
  deterministic baseline runs instead, and the blocker is reported. The privacy contract is not
  traded for extraction quality (FR-805e).
- If compliance **is** established, the extractor must additionally satisfy FR-805d: named,
  disclosed to affected users as plainly as the Cairn server connection itself, and scoped per
  §2. Cairn must not forward a safe event to an undisclosed third party.
- The reasoning "the material had already left the machine, so no new egress occurs" is
  **not available**. Constitution v1.2.1 Principle V names it as the derivation-as-loophole
  argument it refuses. Being permitted to reach the user's own server does not permit
  forwarding anywhere else.

## 6. Duplicate, reinforcement and conflict

Identity is the **normalized** key pair, not the wording (FR-796, FR-796d).

| Situation | Result |
|---|---|
| Same normalized `topic_key` + `value_key` as an existing record | reinforce |
| Same `topic_key`, different `value_key`, overlapping scope | conflict, basis `deterministic_rule` |
| Same `topic_key`, different `value_key`, disjoint scope | independent records |
| No match | new record |

The existing exact content-digest dedup is **not** available server-side:
`content_norm_digest` is a refused field name and SC-731 forbids weakening that refusal, so the
server cannot hold the value that rule compares. Key identity is therefore sufficient for
server-side reconciliation on its own (FR-796d).

Reinforcement needs a persisted endpoint to reinforce *from* (FR-798a). The corroboration
record carries the deterministic `candidate_id` from `consolidation.md` §7, is marked as
consolidation-authored corroboration, is never returned by recall as independent knowledge,
and is never counted as a distinct claim in any count a user reads.

**No embeddings.** The problem is syntactic and a syntactic function solves it (FR-796c).

## 7. Key normalization

The shipped `normalize_topic_key` (`crates/cairn-core/src/knowledge.rs:58-76`) already does
most of this, and this contract must not contradict it:

```
topic:  NFC → lowercase → split on '.'  → per segment: fold [' ', '-', '/'] to '_',
        filter to [a-z0-9_] → drop empty segments → rejoin with '.'
        → ≤6 segments, ≤128 chars
```

**`.` is a segment separator and is never folded.** A pipeline that folded `.` before splitting
would make segmentation dead, make `TOPIC_KEY_MAX_SEGMENTS` unreachable, and rewrite
`test.command` to `test_command` — breaking shipped tests and, through
`migration-cutover.md`, every existing record's key.

So topic keys need **no change**: they already fold space, `-` and `/` within a segment. The
genuine addition FR-796a requires is to **value keys**, which today lowercase and collapse
whitespace but fold no separators at all (`normalize_value_key`, and its test pinning
`"PostgreSQL\t16" → "postgresql 16"`):

```
value:  NFC → lowercase → collapse whitespace → fold [' ', '-'] to '_'
        → collapse repeated '_' → strip leading/trailing '_' → ≤64 chars
```

`Storage Authority`, `storage_authority` and `storage-authority` all resolve to
`storage_authority`. A key that fails validation refuses its candidate rather than being
repaired (FR-796b).

Changing `normalize_value_key` changes existing behaviour, so `migration-cutover.md`'s
re-keying phase applies it to existing value keys — and to value keys **only**.
