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

### 13.7 The mapping algorithm

The design says signals carry a classification and two justified tokens. This is how the local
machine gets there from transient vendor material, deterministically, without a model and
without retaining a fragment of what it read.

The material is read **in memory only**, during the same hook invocation that already parses,
redacts and gates it. Nothing derived from it is retained except the four output values.

```
INPUT   transient vendor text T (a user prompt, or an assistant turn)
        the session vocabulary V (§13.3), already built from accepted lower-seq events
OUTPUT  (kind, subject_token, object_token, justified_by_seq)   or   DECLINE
```

**Step 1 — redact.** `redact(T)`. Every later step reads only the redacted form, so a secret
cannot influence classification or token selection.

**Step 2 — classify, from a fixed lexicon.** Cairn ships a closed, versioned map from marker
phrases to a `decision_kind` or `instruction_kind`. It is a literal table, not a heuristic:

| Marker (case-folded, whole-word) | Kind |
|---|---|
| `use`, `switch to`, `go with`, `adopt` | `adopt` |
| `don't use`, `drop`, `stop using`, `reject` | `reject` |
| `later`, `defer`, `not now`, `postpone` | `defer` |
| `must`, `always`, `require`, `enforce` | `constrain` / `require` |
| `never`, `forbid`, `don't` | `revert` / `forbid` |
| `prefer`, `rather than`, `instead of` | `prefer` |
| `revert`, `undo`, `roll back` | `revert` |
| `only in`, `scope to`, `limit to` | `scope` |
| `actually`, `no —`, `correction` | `correct` |

Matching is **longest-phrase-wins**, and a longer match consumes its span. `don't use X` matches
`don't use` (⇒ `reject`) and the shorter `don't` never competes for the same span — without
this rule the commonest phrasing in the table would decline itself, and SC-701a needs 14 of 20
scenarios to produce a record.

Zero markers ⇒ **DECLINE**. Two or more markers of different kinds **on non-overlapping spans**
⇒ **DECLINE**: an ambiguous instruction is not a fact about the project, and guessing between
them would fabricate one.

**Step 3 — candidate tokens.** Case-fold `redact(T)`, split on non-token characters, and form
unigrams plus adjacent bigrams joined by `_`. Normalize each through the value-key normalizer
(§7). This yields candidates in exactly the shape a key must have.

**Step 4 — intersect with the vocabulary.** Keep only candidates present in `V`. **This is the
step that makes the whole design safe**: every surviving token is something the event stream
already established, so no word that is merely *in the prose* can survive. A sentence
contributes nothing unless it names a file, module, command, test or established key that
Cairn already knows.

**Step 4a — choose the event kind, from the source role.** The kind follows from **which vendor
field the material came from**, not from any reading of the text:

| Source field | Emits |
|---|---|
| the user-prompt field (§13.10) | `user_instruction_signal` |
| the assistant-message field (§13.10) | `decision_signal` |

This is deterministic because the adapter knows which field it read, and it needs no grammatical
analysis. It is also the right semantics: an instruction is something the *user* said, and a
decision is something the *session* concluded.

An earlier form resolved the overlap by "grammatical person — a second-person imperative is an
instruction", which is not a deterministic rule at all. Person detection over free text is
exactly the language analysis this design refuses to do, and it was undefined for every input
that is not a clean imperative.

The marker table's two columns now constrain rather than choose: a marker that has no counterpart
for the chosen kind ⇒ **DECLINE**. So `adopt`/`reject`/`defer`/`revert` from a user prompt
decline (they are decision-only markers), and `scope`/`correct` from an assistant message
decline (instruction-only). `prefer` and `constrain`/`require` exist in both columns and map by
source role with no ambiguity left to resolve.

**Step 5 — assign roles, deterministically.**

- `subject_token` — the surviving candidate with the **highest vocabulary rank**, where rank is
  fixed and total: established project `topic_key` > established project `value_key` > module
  token > file token > test token > command verb. Ties break on lowest `session_seq` of the
  justifying event — a token justified by an established project key rather than an event sorts
  last for this purpose, since it has no seq — then lexicographically. Both tiebreaks exist so
  the result cannot depend on iteration order.
- `object_token` — the highest-ranked *remaining* candidate. If none remains, the closed
  enumeration for the kind may supply it (for `adopt`/`reject`, the object may be the subject's
  established `value_key`); otherwise **DECLINE**.
- `justified_by_seq` — the highest `session_seq` among the events justifying the two tokens,
  recorded so a server refusal can name what was missing (`safe-events.md` §7.1 step 7).

**Step 6 — decline unless complete.** A signal is emitted only with all four values. Otherwise
nothing is emitted and a `capture_declined` disposition is recorded with the reason that
actually applies, from the `decline_reason` vocabulary (`data-model.md` §1.3):

| Condition | `decline_reason` |
|---|---|
| no marker matched | `no_safe_semantic_mapping` |
| markers of two different kinds on non-overlapping spans | `ambiguous_classification` |
| marker has no counterpart for the source role (§4a) | `ambiguous_classification` |
| fewer than two tokens survive step 4 | `insufficient_vocabulary` |
| subject and object normalize to the same token | `insufficient_vocabulary` |
| the justifying event was dropped or refused | `insufficient_vocabulary` |
| the agent emits no semantic signals (§13.10) | `policy_excluded` |

Recording one reason for every case would make the decline rate uninterpretable — an
implementer could not tell a lexicon that never matches from a vocabulary that is too thin.

### 13.8 When Cairn declines — and why that is the right default

DECLINE, with the disposition counted so the rate is visible:

| Condition | Reason |
|---|---|
| no marker matched | nothing indicates a decision or instruction was expressed |
| markers of two different kinds | ambiguous; guessing would fabricate a claim |
| fewer than two tokens survive step 4 | the claim would name something the event stream never established |
| subject and object normalize to the same token | a claim about nothing |
| the justifying event was dropped or refused | the grounding does not exist server-side |


The last condition is **not** a client decline — it is a server refusal, because only the server
knows which lexicon versions it can reproduce. An event whose `lexicon_version` the server does
not recognise is rejected at ingest, not declined at capture.

Declining is the correct outcome, not a shortfall. A recorded decision Cairn cannot ground in
its own event stream is a claim it cannot explain later, and an unexplainable claim in durable
memory is worse than an absent one.

### 13.9 What baseline 005 does not learn

**The reasoning is not learned.** R7 records *that* a decision was taken, of what kind, about
which subject, naming which object. The "because" lives in the conversation and stays there —
it is prose, it is not in any vocabulary, and no step above can carry it.

So a decision reads as *"this project adopted `postgresql` for `storage_authority`"*, never
*"because the team wanted stronger transactional guarantees"*. That is a real limitation and it
is stated here rather than discovered later. It is also the direct consequence of the privacy
contract: reasoning is expressed only in prose, and prose does not cross this boundary. A later
extractor with a different privacy posture could learn more; baseline 005 deliberately does not.

### 13.10 Where the transient material comes from, per vendor

Step 4a keys the event kind on the **source role**, so the exact vendor field each role reads
must be named rather than assumed. Checked against official documentation on 2026-08-30.

| Agent | User-prompt source | Assistant-text source | Subagent |
|---|---|---|---|
| Claude Code | `UserPromptSubmit.prompt` | `Stop.last_assistant_message`, `SubagentStop.last_assistant_message` | `SubagentStop.agent_id`, `.agent_type` |
| Codex CLI | `UserPromptSubmit.prompt` | `Stop.last_assistant_message`, `SubagentStop.last_assistant_message` | `SubagentStop.agent_id`, `.agent_type` |
| OpenCode | **not used** — see below | **none exists** | not established |

Rules that follow from the evidence:

- **`last_assistant_message` is nullable** on Codex (`string | null`, per the vendor's field
  table). A null is not an empty decision: it is DECLINE, not a signal with empty tokens.
- **`StopFailure.last_assistant_message` MUST NOT be read.** On Claude Code that field carries
  the API error string itself — *"API Error: Rate limit reached"* — not model prose. Feeding an
  error string to classification would manufacture decisions out of infrastructure failures.
- **`MessageDisplay.delta` MUST NOT be read.** It streams partial assistant text; classification
  over a fragment would fire on half a sentence. Only the settled turn text is read.
- **OpenCode emits no semantic signals in baseline 005.** Its v1 prompt text is not in a named
  field at all — it must be walked out of `chat.message`'s `output.parts[]` entries of
  `type: "text"` — and that hook is absent from the vendor's documentation, appearing only in
  published type definitions. Its assistant-text hook, `experimental.text.complete`, is
  undocumented and carries an `experimental.` prefix. OpenCode 2 exposes `event.prompt.text` but
  is beta, and exposes **no assistant-text hook of any kind**.

  So Cairn declines semantic signal capture for OpenCode, reported as `declined_by_cairn` with
  the reason — the same posture, for the same reason, as declining its delivery surface
  (FR-838b). This is a Cairn decision about an unstable surface, not a claim that OpenCode
  cannot do it. OpenCode's structural capture is unaffected: R1–R6 need no prompt or assistant
  text, so its `failure`, `convention` and `procedure` learning works exactly as the other two
  agents'.

**Consequence for SC-701a.** Its scenario set is drawn from the agents that emit semantic
signals — Claude Code and Codex CLI. OpenCode remains in SC-701 and SC-706, which test capture,
and its exclusion here is recorded in the capture matrix rather than left implicit.

The material from these fields is read in memory during the hook invocation that already parses
and redacts it, and is discarded when §13.7 completes. No vendor field named here is ever
persisted, locally or centrally.
