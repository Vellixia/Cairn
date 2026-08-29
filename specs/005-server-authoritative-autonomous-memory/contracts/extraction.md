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

---

## 13. Semantic signal content — closing the decision/instruction gap

### 13.1 The gap

The first pass gave `decision_signal` and `user_instruction_signal` **no content**, reasoning
that any text derived from a prompt is a transcript fragment. The consequence was not noticed
at the time: the information is destroyed at the machine boundary and is unrecoverable
afterwards, so no amount of server-side extraction can learn what was decided. Feature 005
exists to learn decisions, constraints and procedures — not only that a test went from red to
green. As written, the design could not do the thing it is for.

### 13.2 Why a length-capped text field is not the answer

The obvious repair — "carry 256 redacted bytes" — fails. Redaction is pattern-based and
self-described as *"a mechanism, not a guarantee"* (`crates/cairn-core/src/redact.rs:5-6`), and
a bounded free-text field derived from a prompt is still a prompt fragment. Constitution v1.2.1
Principle V prefers **structural prevention to procedural rules**: *"a record that has no column
for a secret cannot carry one."* A capped text column is a procedural rule wearing a number.

Nor is a normalized-charset token sufficient on its own. `the api key is sk-abc123` normalizes
to `the_api_key_is_sk_abc123`, which is inside the charset and inside the length. Shape alone
constrains nothing.

### 13.3 The mechanism: vocabulary-justified tokens

A semantic signal carries a **closed enum plus tokens the local layer must justify against
evidence already in the event stream**. This is the same rule as the key↔evidence gate in
`consolidation.md` §5 gate 5a, applied one layer earlier.

```
decision_signal {
  decision_kind: adopt | reject | defer | constrain | prefer | revert
  subject_token: <vocabulary-justified>
  object_token:  <vocabulary-justified | closed enum>
}

user_instruction_signal {
  instruction_kind: require | forbid | prefer | scope | correct
  subject_token: <vocabulary-justified>
  object_token:  <vocabulary-justified | closed enum>
}
```

**The session vocabulary** is derived deterministically, with no prose input, from:

| Source | Contributes |
|---|---|
| `repo_file` path segments in this session's events | module and file tokens |
| `command_line` leading binary and subcommand | command verbs |
| `test_command` suite identifiers | test tokens |
| existing `topic_key`s in this project's knowledge | established subject tokens |
| existing `value_key`s in this project's knowledge | established value tokens |

A token that is **not in the session vocabulary is refused**. Both sides check it: the client
before constructing the event, and the server independently against the events it already holds
for that session (`safe-events.md` §7.1). This is deterministic, fail-closed, and evaluable
without a model.

**Ordering, and why it is not a race.** The server can only justify a token against events it
has already accepted, and events arrive incrementally — a signal citing a file could otherwise
arrive before the `file_changed` event that put the token in the vocabulary. Three rules close
this:

1. Events are delivered in `session_seq` order per session, and the spool claims them in that
   order, so an event that established a token is delivered before one that cites it.
2. A semantic signal's tokens must be justified by events with a **lower `session_seq`** in the
   same session. A token justified only by a later event is refused — the machine that built the
   signal knew the earlier events too, so it could not legitimately have cited a later one.
3. If the justifying event was itself dropped — deadline, spool overflow, or a server refusal —
   the signal is refused with `token_not_in_vocabulary`, and the refusal is counted. This is the
   correct outcome rather than an unfortunate one: the server declines to record a claim whose
   grounding it does not hold.

**Redaction runs before vocabulary derivation**, not after. A command line is redacted first,
so a credential never enters the vocabulary and therefore can never justify a token. This
matters because `command_line` is one of the vocabulary sources: without the ordering, a secret
in a command would become a legitimising token for itself.

**Repository content cannot smuggle prose.** A file path contributes its *path segments*, each
normalized to the key charset and bounded — not file contents, which Cairn never reads. A
deliberately-named file can only contribute a token that is already visible in the repository to
anyone who can read it, and which the reader of that project can already see.

### 13.4 Why this satisfies the constitution

- **No transcript.** A prompt sentence cannot survive: its words are not in the vocabulary, so
  the token is refused. `the_api_key_is_sk_abc123` is refused for the same reason a sentence is
  — it is not a file, module, command, test or established key.
- **Secrets.** Free-text vocabulary sources — `command_line`, `test_command` — are redacted on
  the client and screened again on the server before they contribute anything, so a credential
  in a command never enters the vocabulary. `repo_file` path segments are also screened, so a
  file *named* after a credential cannot contribute one either. The honest limit: a token can
  only ever repeat something already present in an event the server accepted, so a semantic
  signal discloses nothing the event stream did not already carry. It is not an independent
  disclosure channel — which is the claim that matters, and is weaker than "a credential can
  never appear".
- **Deterministic and fail-closed.** Vocabulary membership is set containment. Unjustifiable
  token ⇒ refusal, never a best guess.
- **A model is not required.** Deterministic rules map a signal to `decision_kind` and choose
  the nearest vocabulary token. A local model MAY propose tokens instead, but the vocabulary
  check governs either way, so the model is an optimization and never the gate.
- **Smallest sufficient change.** Two enums and two constrained tokens per signal. No new
  service, store, broker or field type.

### 13.5 What it makes learnable

Tokens are key-shaped by construction, so a semantic signal feeds the existing identity
machinery directly: `topic_key = subject_token`, `value_key = object_token`. A decision becomes
a first-class DECISION memory with provenance, reinforcement, conflict detection and
supersession — all mechanisms Feature 003 already built.

This adds two extractor rules, which are **session rules** (`extraction.md` §4.0):

**R7 — Recorded decision.** `decision_signal{kind, subject, object}` ⇒ **DECISION**:
"This project `<kind>` `<object>` for `<subject>`."
`topic_key = decision.<subject_token>` · `value_key = <object_token>`

The `decision.` prefix is load-bearing. R1–R6 derive keys from structural evidence; R7 and R8
take theirs from a token the client supplied, so an unprefixed key would let one crafted signal
name an existing high-value topic and register a `conflicts_with` against it — a poisoning
primitive, and one that gate 5a cannot catch because the cited event *is* the key. Namespacing
confines client-originated claims to their own key space, where they still reinforce, conflict
and supersede among themselves, but cannot collide with structurally-derived knowledge.

**R8 — Standing instruction.** `user_instruction_signal{require|forbid, subject, object}`,
observed in ≥2 sessions ⇒ **CONVENTION** (project rule — aggregator):
"`<object>` is `<required|forbidden>` for `<subject>` here."
`topic_key = instruction.<subject_token>` · `value_key = <object_token>`, namespaced for the
same reason as R7.

R8 is an aggregator rule because a standing convention should rest on repetition, not on one
instruction in one session.

### 13.6 Honest limits

This learns *that* a decision was taken, about *what*, and in *which direction*. It does not
learn the reasoning behind it — the "because" lives only in the conversation and stays there.
A DECISION produced by R7 is a durable, checkable, provenance-bearing claim about a subject in
this project's own vocabulary. It is not a summary of the discussion, and the spec should not
imply that it is.
