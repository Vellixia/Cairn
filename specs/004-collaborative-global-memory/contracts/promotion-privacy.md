# Contract: Promotion and Privacy

**Feature**: `004-collaborative-global-memory`

This contract guarantees that project memory becomes personal or team knowledge only through
an explicit act that passes a deterministic, fail-closed gate in a fixed order, that a
promoted record's authority never inherits a check that only ever ran against one project,
that privacy here is two separate layers — some values have no column to hold them at all,
and everything else (`content`, plainly free text) is validated by one shared, pure function
that runs at every entry point capable of creating global content, not only at promotion —
and that the two live leaks discovered in the shipped wire boundary — a misdescribed field
check and an under-redacted handoff payload — are closed as part of the same privacy work
rather than left for a later pass.

**D433 — why this contract was restructured.** An earlier draft of this feature's privacy
guarantee, and the FR-517 it was built on, treated the whole guarantee as structural — as if
no personal or team record could carry a project path or a command because the tables simply
had nowhere to put one. That was true only of the four column classes in §5 below (**Layer
A**). It was never true of `content`: `content` is free text, and nothing enforced its shape
outside the promotion path. `evaluate_promotion`'s `possible_secret` check (the original
check 3, §2) was the *only* place an absolute path, a credentialed URL, or an
environment-variable assignment in `content` was ever refused — and `evaluate_promotion` is
called only by promotion. **Direct personal creation and team proposal never called it at
all.** A `cairn_remember action=create target=personal` call, or a team proposal, could carry
exactly the content the promotion gate existed to refuse, straight onto the same tables that
gate was supposed to protect. §2a closes that bypass by moving content validation into a
function every entry point calls; §2b states the corrected two-layer guarantee plainly, so
this contract does not make the same claim again.

## 1. The promotion gate is a pure function

**FR-506, FR-507, D415 (verified by SC-461).** Modeled explicitly on the existing pattern-promotion gate
(`crates/cairn-store/src/patterns.rs`, contract `specs/003-project-intelligence/contracts/
patterns.md` §"The promotion gate") and on `derive_subject`'s discipline of enforcing a rule
"by what the function can see" rather than by review
(`crates/cairn-core/src/knowledge.rs:10-17`):

```rust
pub fn evaluate_promotion(
    content: &str,
    topic_key: Option<&str>,
    value_key: Option<&str>,
    proposed_applicability: &[(ApplicabilityKind, String)],
    project_identity: &ProjectIdentity,                // name, git_common_dir, remote — passed to check 1's
                                                        // validate_global_content call as the single-element
                                                        // project_identities slice (§2a, D446), and read again
                                                        // by check 7's origin-digest computation
    target: PromotionTarget,                           // Pattern | Personal | Team
    promoter_is_project_member: bool,                  // for check 6
    source_state: MemoryState,                         // for check 2
) -> Result<PromotionApproval, PromotionRejection>;
```

No database handle, no clock, no network call, no I/O of any kind — every input the gate
needs is passed in by value, and the same inputs always produce the same result. This is
what makes it unit-testable against a seeded adversarial corpus without a store, exactly as
003's `privacy_promotion` test already exercises the pattern gate this way
(`specs/003-project-intelligence/contracts/privacy-sync.md` §"Promotion — the highest-risk
path", "a seeded adversarial corpus").

**FR-544, FR-545 (D433, D446, D447).** `evaluate_promotion` no longer validates `content` or
`applicability` itself. Its first check calls the shared `validate_global_content` (§2a),
passing `project_identity` as the single-element `project_identities` slice (§2a), and
propagates whatever it returns — the same function direct personal creation, team proposal,
and server-side ingest (§2c) call directly, with no `evaluate_promotion` involved at all,
because those three paths have no promotion to gate. This is what makes the five entry points
identical on this one question: promotion's gate composes the shared validator plus checks
only promotion can run (§2); direct creation, team proposal, and server-side ingest call the
shared validator alone, because they have nothing else to check (no source memory, no evidence
to strip) — ingest supplies the union of the pushing user's project memberships as
`project_identities` (§2c) rather than a single project's tokens, and direct creation and team
proposal supply the tokens of the project the caller is currently working in, if any, which
may be empty (§2a, FR-580).

```rust
pub struct PromotionApproval {
    pub sanitized_applicability: Vec<(ApplicabilityKind, String)>,
    pub origin_digest: String,
}

pub struct PromotionRejection {
    /// Which gate check stopped it — a fixed name from §2's table, e.g.
    /// "not_a_member". Always set.
    pub check: &'static str,
    /// For check 1 only: which of the validator's classes it delegated to and
    /// got back, e.g. "absolute_path". `None` for every other check, because no
    /// other check has a class — it *is* its own reason.
    ///
    /// Both fields, deliberately. `check` alone cannot satisfy SC-421's "names
    /// the class it failed on" for a content failure, and folding the class into
    /// `check` would make one field mean two things depending on its value.
    /// Neither field can hold offending text: both are `&'static str` drawn from
    /// fixed vocabularies, so there is no `String` on this type at all — which
    /// is what makes "never echoes the value" a property of the type rather than
    /// a rule the caller must remember.
    pub class: Option<&'static str>,
}
```

`PromotionRejection` has no field that *could* hold the rejected content — the same
"absence beats validation" argument §5 makes for the storage tables applies here to the
rejection type itself: a reviewer cannot accidentally log the offending string through this
type because the type has no string field to put it in.

## 2. The gate's own checks, fixed order (D416, revised by D433)

**Order is fixed and this contract does not reorder it.** Each check either passes silently
or returns immediately with its name — the first failing check stops evaluation; no later
check ever runs once one has failed, and no rejection ever names more than one check.

D416 originally fixed **ten** checks. D433 moved two of them — `possible_secret` and
`applicability_invalid` — out of `evaluate_promotion` and into the shared
`validate_global_content` (§2a), because neither one needs anything a promotion source
provides that a direct creation or a team proposal does not also have: they check `content`
and `applicability` themselves, not anything about where the record came from. D446 moves a
third: `project_identifying` (the original check 4) needs nothing from a promotion either — it
screens `content`, `topic_key`, `value_key`, and every applicability value against a set of
identity tokens, and §2a's validator now takes that token set as its own fifth parameter
(`project_identities`), so the check is satisfied by delegation exactly as check 1 already is,
rather than restated a second time inside the gate. The gate below therefore fixes **eight**
checks — the ones that stay, because each one needs a piece of project-memory context
(`source_state`, evidence metadata, the source's own verification state, or the promoter's
membership) that only a promotion actually has:

| # | Check | Refuses when | Passing example | Failing example |
|---|---|---|---|---|
| 1 | `shared_content_validation` | `validate_global_content(content, topic_key, value_key, sanitized_applicability, project_identities)` (§2a) returns `Err` | Content and applicability both pass §2a's checks, including `project_identifying` and `command_shaped` | Any input §2a itself would refuse — see its class list |
| 2 | `source_not_active` | The source memory's `state` is not `active` | An active memory about a retry policy | A `superseded` memory about a since-changed retry policy |
| 3 | `no_subject` | The source has no `topic_key` | `topic_key = "retry.backoff"` | A free-form memory with no `topic_key` at all |
| 4 | `evidence_leak` | Content carries an evidence or observation identifier; a bare count of supporting evidence may travel, the identifiers themselves must not | "Verified by 2 configuration checks" | "See evidence `4f2a1c...` for the config value" |
| 5 | `verification_reset` | *(never refuses, resets nothing — see §3)* | — | — |
| 6 | `not_a_member` | `target = Team` and the promoter is not a member of the source project | A team promotion by a member of the source project | The same promotion attempted by an admin who has never been added to that project |
| 7 | `origin_computation` | *(never refuses — computes the salted digest, see §4)* | — | — |
| 8 | `evaluation_incomplete` | Any prior check could not be evaluated for lack of information | Every required input present | The caller omits `project_identity`, leaving check 1's delegation and check 7's origin computation unable to run |

**Where the ninth check went (D446).** The original check 4, `project_identifying`, is not
dropped — it is now enforced by `validate_global_content` (§2a) for every one of the five
entry points, not only for promotion, which is strictly stronger than a check that only ever
ran on the promotion path. Check 1 above is where a promotion inherits it.

Checks 5 and 7 are listed at their fixed position in the order because D416 numbers every
check positionally, but neither one is a *rejection* class — check 5 always succeeds and, per
§3's correction, resets nothing because there is nothing to reset; check 7 always succeeds and
computes the origin digest (§4). They occupy slots in the fixed sequence because their side
effects (making the absence of verification explicit, computing the digest) are themselves
part of what "passing the gate" means, and fixing their position keeps the whole sequence's
rationale — "the reported reason is stable" (patterns.md's own framing for its checks) —
intact even though these two never *report* a reason.

**Check 8 — fail-closed on missing information (FR-518, FR-549).** If any input the gate
would need to answer an earlier check is absent (no `project_identity` supplied when check 1's
delegation or check 7's origin computation needs it, no `source_state` when check 2 needs it),
the gate refuses with `evaluation_incomplete` rather than skipping the unanswerable check and
proceeding. This is the same "no is the default" discipline the pattern gate already applies
(`patterns.md` — every failure is a named refusal, never a silent skip) extended to the case of
missing inputs specifically, which the original check list did not need to name because its
caller always supplied a complete `SourceFacts` row from the store. `validate_global_content`
(§2a) applies the identical discipline independently, one layer earlier, for the inputs it
alone owns — with the one named exception FR-580 states: an *empty* `project_identities` set
is not a missing input, it is a present input with nothing in it, and the `project_identifying`
check passes rather than refuses (§2a).

## 2a. The shared content validator — closing the bypass (D433, D446, FR-544–FR-549, FR-577–FR-580)

```rust
pub fn validate_global_content(
    content: &str,
    topic_key: Option<&str>,
    value_key: Option<&str>,
    applicability: &[(ApplicabilityKind, String)],
    project_identities: &[ProjectIdentity],
) -> Result<(), GlobalContentRejection>;
```

Pure, total, and shared: no database handle, no clock, no network call — the same discipline
as `evaluate_promotion` (§1), and for the same reason (unit-testable against a seeded
adversarial corpus with no store). "Total" means it has a defined answer for every input it
can be called with, including the degenerate ones (empty `applicability`, `None` keys, an
**empty `project_identities` slice**) — there is no input shape that leaves it unable to
decide, and an empty `project_identities` slice is answered by *passing* the
`project_identifying` check rather than by refusing (FR-580, see below).

**`project_identities` (D446, FR-545), what it holds at each of the five entry points:**

| Entry point | `project_identities` |
|---|---|
| Direct personal creation | The tokens of the project the caller is currently working in, if any; empty if the caller is not inside a linked project |
| Personal promotion | The single source project's tokens — `evaluate_promotion` passes its own `project_identity` (§1) as a one-element slice |
| Team proposal | Same as direct personal creation — the current project's tokens, or empty |
| Team promotion | Same as personal promotion — the source project's tokens |
| Server-side synchronization ingest (§2c) | The **union** of every project the pushing user is a member of, because the server cannot know which project the client was in when it created the item, only every project it could have been (D447) |

**FR-545 (verified by SC-438) — the five entry points, all of them, no exceptions.**
`validate_global_content` MUST be called by, and only by, code on these five paths, because
these are the only five places global content can be created:

1. Direct personal creation (`cairn_remember action=create target=personal`)
2. Personal promotion (`evaluate_promotion`, check 1, target `Personal`)
3. Team proposal (`cairn team propose` / the promotion-shaped direct-team-proposal path)
4. Team promotion (`evaluate_promotion`, check 1, target `Team`)
5. Server-side synchronization ingest (§2c, D447) — the server, receiving a `personal_knowledge`
   or `team_knowledge` item pushed by a client

**Before D433, only paths 2 and 4 ran a content check at all** — because that check
(`possible_secret`) lived inside `evaluate_promotion`, and paths 1 and 3 never call
`evaluate_promotion`. **This was a real bypass, not a hypothetical one**: nothing stood
between an operator typing an absolute path into `cairn_remember action=create
target=personal` and that path landing, unvalidated, in `personal_knowledge.content`. Moving
the check into a function every one of the four client-side paths calls directly closed that
bypass — but it left a second one open, which D447 (§2c) closes: nothing on the *server*
re-ran the same validation, so a modified client, an old client, or a client with a bug that
skipped its own call to `validate_global_content` could still write unvalidated content
straight into the server's store, from which it would propagate to the user's other devices on
the next pull. **This feature does not repeat that mistake a second time**: path 5 exists
specifically so client trust is never the only thing standing between an operator's mistake
(or a compromised client) and a privacy leak, and any future path capable of creating a
`personal_knowledge` or `team_knowledge` row — client-side or server-side — inherits the
obligation to call this function by the same reasoning FR-545 states plainly. SC-424a verifies
this across all five: a seeded adversarial corpus driven through every entry point leaves no
project-identifying token, file path, or shell command in any stored free-text field or
applicability value.

**FR-546 — the rejection class list, nine members (D446)**, in the order checked (first
match wins, exactly like §2's checks):

| Class | Refuses when `content` — or, per FR-578 below, an applicability value — contains |
|---|---|
| `absolute_path` | An absolute filesystem path (`/…`, or a rooted path on any supported OS) |
| `home_dir_ref` | A home-directory reference (`~/…`) |
| `drive_letter_path` | A Windows drive-letter path (`C:\…`) |
| `file_uri` | A `file://` reference |
| `credentialed_url` | A URL carrying embedded credentials (`https://user:pass@host/…`) |
| `env_assignment` | An environment-variable-assignment shape (`NAME=value`, all-caps identifier followed by `=`) |
| `encoded_secret_shape` | A run of characters shaped like an encoded secret (long base64/hex run) |
| `project_identifying` | The project it came from, by name, by a component of its shared identity (`git_common_dir`), or by a remote host/organisation/repository token, screened against the input `project_identities` after separator folding (see below) — **an empty `project_identities` slice passes this check rather than refusing** (FR-580, D446) |
| `command_shaped` | A shell command invocation — see §2a's *prose versus invocation* rule below for what separates one from a sentence naming a tool |

### `command_shaped` — prose versus invocation, not "has a flag"

The distinction this class draws is **grammatical position**, and it has to be, because the
obvious alternative does not hold. Phase 2 first implemented it as "a known command name
followed by a flag or a path", which reads as reasonable and admits `cargo test`, `rm target`,
`sudo reboot`, `npm install` and `git status` — five plain commands, none carrying a flag. A
heuristic that lets `rm target` through is not a screen for command-shaped content.

What separates the two cases is where the program name sits:

- **Prose** names a tool inside a sentence, with a preposition or a copula around it: *"Use
  `cargo nextest` for the test suite"* (this contract's own passing example), *"docker is the
  deployment target"*, *"Prefer git over a bespoke tool"*. The program is a noun.
- **An invocation** puts the program in imperative head position with its operands after it:
  *"cargo test"*, *"rm -rf ./target"*, *"Run make"*. The program is a verb.

Three rules, in order:

1. **Shell syntax is conclusive on its own** — `&&`, `||`, `$(`, a backtick, a pipe, `2>&1`.
   No sentence contains these.
2. **A flag, a path or a redirect in the tail is conclusive** — prose about a tool does not
   carry `--workspace` or `./target`.
3. **Otherwise, position decides.** An explicit invoking verb (`run`, `execute`, `invoke`, a
   shell prompt marker) before the program settles it with nothing needing to follow. A
   program clause-initial with a subcommand or operand after it is an invocation; one followed
   by an English function word, or by nothing, is a sentence about the tool.

`use` and `prefer` are deliberately **not** invoking verbs. "Use cargo" means adopt this tool;
"Run cargo" means execute it. That difference is why this contract's passing example passes,
and it is a real distinction rather than a concession made to keep a test green.

The last two classes are new in this feature (D446): before this repair, `project_identifying`
lived only in the promotion gate (§2, formerly check 4) and `command_shaped` did not exist
anywhere,
so a shell command in `content` passed direct personal creation and team proposal unchecked.
Both now run at all five entry points, exactly like the original seven. SC-421's seeded
adversarial corpus covers all nine classes; a class added to the validator with no
corresponding corpus entry leaves that criterion unmet, not silently unverified.

**`project_identifying` folds separators on both sides before comparing.** Both the content and
each identity token are reduced to their letters and digits, lower-cased, before the match. Not
cosmetic: an applicability value must be `[a-z0-9_]{1,64}` after normalization, so a project
named `acme-widgets` *cannot appear there in its own spelling at all* — a raw substring screen
could never have fired for any project whose name contains a hyphen or a dot, which is most of
them. Folding makes `acme-widgets`, `acme_widgets`, `acme.widgets` and `acmewidgets` one name.

An identity shorter than three characters is not screened on. A project named `ci` would
otherwise refuse most English prose, and a two-character token carries no information about
whether the content is about that project.

Folding deletes separators rather than normalizing them to one, which over-refuses across a
sentence boundary: *"Use acme. Widgets are fine."* folds to a string containing `acmewidgets`
and is refused. That is the accepted direction of error and it is recorded rather than left to
be rediscovered — over-refusing costs an author one rewrite, while normalizing separators
instead of deleting them would let `acmewidgets` through, which is a bypass anyone could reach
by accident.

**Applicability runs through two stages, not one.** First, the closed-vocabulary and format
check §2's former check 7 used — each `(kind, value)` pair's `kind` must be one of the two
vocabulary members (`global-memory.md` §4, D439) and `value` must match `[a-z0-9_]{1,64}`
after `normalize_value_key`; a failure here is reported as its own class,
`invalid_applicability`. It is **not a tenth content class**: the nine describe what a value
*says*, and this one describes what a value *is*. It travels through the same `Result` and the
same type because no caller needs to branch on which kind of refusal it received — the remedy
for both is to fix the input — but it is not a member of `CONTENT_CLASSES`, and the audit that
screens for duplicate implementations (SC-453) enumerates the nine, not ten. Second — this is FR-578, new in this feature — once a value has
passed that format check, the value string itself is run through the same nine classes above
**as if it were content** (D446): a value shaped like a project name or a shell command is
refused under `project_identifying` or `command_shaped`, not under `invalid_applicability`,
because it is not the *format* that is wrong, it is what the value *says*. SC-448 verifies
this with a project-identifying value such as an internal product name, refused identically
whether it appears in `content` or in an applicability value.

**FR-579 (verified by SC-453) — `validate_global_content` is the only implementation of
these classes.** No other component may re-implement, duplicate, or partially restate any of
the nine content classes or the applicability format check — not the promotion gate (which
delegates, §2), not the server's ingest handler (§2c, which calls this same function), not a
client-side pre-check that runs before the call reaches here. SC-453 verifies this by an audit
that fails when a second implementation is introduced — not by inspecting today's code, which
would pass today and pass again once a duplicate exists — because a second implementation,
however faithful at the moment it is written, is a second place the two can drift apart; this
feature keeps exactly one.

**FR-547 (verified by SC-439) — the rejection never carries the offending value.**

```rust
pub struct GlobalContentRejection {
    /// One of the **nine** content class names above, or
    /// `invalid_applicability` — which is not a tenth content class but the
    /// applicability format refusal, travelling through the same type because no
    /// caller needs to branch on which it received. Nothing else, and never the
    /// matched text: this type has one field and it is a fixed string, so there
    /// is nowhere for content to go.
    pub class: &'static str,
    // Deliberately no `value: String` field. Exactly the same argument §1 makes for
    // `PromotionRejection`: a reviewer cannot log the rejected content through this type
    // because the type has no field to put it in.
}
```

A rejection is reported, logged, and returned as a bare class name. No caller of
`validate_global_content` — the MCP handler, the CLI, the promotion gate, the server's
promotion endpoint — may reconstruct or forward the offending substring from anywhere else in
the same request; the value that failed validation is simply never carried past the function
that rejected it.

**FR-548 (verified by SC-440) — atomicity.** A rejection from `validate_global_content`, at
any of the five entry points, leaves no row in `personal_knowledge`/`team_knowledge`, no
partially written row (there is no multi-statement write for these tables to be partial in the
middle of — creation is one `INSERT`, gated by this check running first), and no queued outbox
entry at the four client-side entry points. At the fifth, server-side ingest (§2c), the
equivalent guarantee is FR-581: the item is not persisted, not partially persisted, and not
acknowledged as delivered to the pushing client. The check runs before the `INSERT`
(client-side) or before the server's own persistence statement (ingest) is attempted, not
concurrently with it and not as a post-write validation that would then have to also delete
what it just wrote.

**FR-549 — fail-closed, with one named exception (FR-580, D446, verified by SC-454).** A
check inside `validate_global_content` that cannot be evaluated — malformed UTF-8 that defeats
a pattern match, an `applicability` entry whose `kind` fails to parse as `ApplicabilityKind`
before the vocabulary check can even run — rejects rather than lets the ambiguous input
through. The same "no is the default" discipline as gate check 8 (§2), applied one layer
earlier. **The single documented exception**: `project_identifying` given an *empty*
`project_identities` slice is not a check that cannot be evaluated — it is a check that has
been evaluated and found to have nothing to match, which is a vacuous pass, not an unevaluable
input. Implementing this fail-closed would refuse every global creation made outside a linked
project, which is the ordinary case for cross-project personal knowledge, and would make the
feature unusable. Nothing else in this function, or in the gate (§2), carries this exception —
an input that is genuinely absent (no `project_identity` argument at all, as opposed to one
holding zero tokens) still fails closed exactly as FR-518 requires. SC-454 asserts the vacuous
case and the unevaluable case **separately**: an implementation that conflates them — refusing
the vacuous case, or passing the unevaluable one — fails whichever half of SC-454 it
conflated, not merely a single combined check.

## 2b. Two privacy layers, not one (D433, D452, D456/F11, FR-550)

Every privacy guarantee this feature makes holds for exactly one of two reasons, and this
contract states which one for each value, so that "impossible" and "checked" are never
conflated again. **Re-derived here (D456/F11), not merely restated**: an earlier draft of this
table put "a file path" and "a command" in Layer A, as if a dedicated column for either had
ever existed. It never did — a path or a command that appears in a personal or team record
lives inside `content`, exactly like any other free text, and was always Layer B. Listing them
in Layer A overstated the guarantee for exactly the values FR-550 exists to protect against
that overstatement.

| Layer | Values | Why the guarantee holds | Where |
|---|---|---|---|
| **A — structural** | Project identifier, evidence reference, observation id, verification field of any kind — not an authority, not a state, not a timestamp | No column exists for any of these. There is nowhere to put the value, so no code path — correct or buggy — can write it | §5 |
| **B — validated** | `content` (a file path or a command, if present, arrives as part of this free text), topic key, value key, and every `applicability` value | Free text or an open string, checked by `validate_global_content` (§2a) at all five entry points before the row is written | §2a |

**FR-550 (verified by SC-467, an audit over the shipped documentation, not a review habit).**
This document, and every other contract in this feature, MUST say which layer a given
guarantee belongs to whenever it states one, and MUST NOT describe a Layer B value —
`content`, a topic key, a value key, or an applicability value foremost among them — as
structurally incapable of carrying a path or a command. Layer A's absence-of-a-column argument
(§5's "why absence beats validation") is a real and strictly stronger guarantee than Layer B's
validated-free-text argument, precisely because it requires no code to run correctly; that is
exactly why it must not be claimed for a field, like `content`, where it does not hold. SC-467
exists because an earlier draft of this documentation made exactly that false claim once (the
Layer A table this section corrects); its audit fails on the forbidden phrasing itself, not on
a reviewer noticing, which is why this section states plainly, in its own table above, which
layer each value actually rests on rather than trusting the next editor to remember. A swept
check of this contract and of `global-memory.md` confirms neither now makes that claim: §5
below and `global-memory.md` §2's column tables list only Layer A values, and every mention of
`content`, a topic key, a value key, or an applicability value in either document is now
qualified as validated (Layer B), not structural.

## 2c. Server-side synchronization ingest — the fifth entry point (D447, FR-545, FR-577, FR-581, SC-449, SC-456)

**The gap this closes.** §2a's four client-side calls to `validate_global_content` close the
bypass a client's own code could open. They do nothing about a client that does not run that
code at all — a modified client, an old client built before this feature existed, or a client
with a bug. Any of these can still push a `personal_knowledge` or `team_knowledge` item
straight to `POST /api/sync/batch`, and nothing on the server re-checked it before this
feature. **The design must not accept client trust for a privacy boundary.** Server-side
ingest is the fifth mandatory entry point precisely because it is the one entry point that does
not trust the other four.

**What the server screens against.** The server calls the identical `validate_global_content`
(§2a) — not a re-implementation, not a subset, the same function (FR-579) — with
`project_identities` set to the **union of every project the pushing user is a member of**.
The server cannot know which project the client was working in when it created the item; it
can know every project that user could have been in. This is deliberately broader than any one
client-side check, and it catches exactly the case a client-side check structurally cannot:
content naming project X, pushed by a client that happened to be working in project Y at the
time.

**This layers with the client check; it does not replace it (FR-577).** A well-behaved client
still validates before it ever queues the item — that check is cheaper, catches the mistake
immediately, and needs no network round trip. Ingest validation is the backstop for when that
first check did not run, not a reason to remove it.

**Refusal is permanent, and unlike anything else in this contract (FR-581, D447):**

| | Capability refusal (`409 unknown_entity_type`, `sync-namespaces.md` §11a) | Ingest content refusal (this section) |
|---|---|---|
| Cause | The server predates schema 3 and does not recognize the entity type at all | The server recognizes the entity type and rejects this specific item's content |
| Outbox state | `blocked` — retained, re-probed on a bounded schedule against *that* server | Not entered at all: the item is refused, not held for later delivery |
| Becomes deliverable after | A server upgrade — the very next capability probe that observes the entity type releases it, unchanged, with no client action (`sync-namespaces.md` §11a) | Never, for this content unchanged. Retrying the identical payload against the identical server can never succeed, because nothing about the server's capabilities is the obstacle |
| Reported to the client as | A capability gap (`sync-namespaces.md` §11) | A `GlobalContentRejection` class (§2a) — the same discipline as every other refusal this contract defines: a class name, never the content |

**The client MUST be able to tell these apart without inspecting a message string (FR-581,
SC-456).** Treating an ingest refusal as if it were a capability refusal — holding the item,
re-probing, waiting for an upgrade — would hold it forever, because no upgrade ever makes a
`project_identifying` value stop naming a project. The distinction is a **typed field**, not
prose in an error message: a capability refusal is `409 unknown_entity_type`; an ingest content
refusal reports a distinct, differently-coded response carrying the `GlobalContentRejection`
class, so client-side handling branches on the response shape rather than on parsing a string.

**The refused item is not persisted, not partially persisted, and not acknowledged as
delivered (FR-581).** The pushing client's outbox row is neither `blocked` nor `delivered`; the
refusal is reported synchronously, in the same response to the batch that carried it, exactly
as a client-side refusal is reported synchronously to whoever requested the
create/promote/propose call that triggered it (§9's FR-520, restated here for the fifth entry
point). An ingest refusal MUST NOT throttle the namespace the way a transient failure's backoff
would (`sync-namespaces.md` §4) — it is not a transient condition that resolves with time or
retry, and treating it as one would slow down every *other* item in the same batch for no
reason connected to them. SC-456 verifies exactly this: the refused namespace remains eligible
rather than blocked, and unthrottled, asserted by the namespace continuing to push subsequent
items at unchanged throughput.

**SC-449** is this section's verification obligation: a client that bypasses its own local
validation and pushes personal or team content containing a project-identifying token or a
shell command is refused by the server; the record is absent from the server store; and it
never reaches the user's other devices, because it was never stored in the first place.

## 3. No verification field at all — why there is nothing to reset (D417, D452)

**FR-513, FR-517.** A personal or team record — created by promotion or authored directly, it
makes no difference — has **no verification field of any kind: not an authority, not a state,
not a timestamp.** This is stronger than "resets to `unverified`," and deliberately so:
"resets to `unverified`" still admits a column that could hold `unverified` today and something
else tomorrow. There is no such column. `unverified` is not a value stored anywhere on these
tables; it is simply what is true of a record that has no verification representation to
check.

**The argument for why promotion in particular must discard whatever the source held, stated
in full**: a deterministic check that established `verified` on a *project* memory ran
*against that specific project* — it read a configuration file, a Git ref, or a runtime state
that exists at a specific path, on a specific checkout. "Cairn verified that this project's
`config/app.yml` sets port 9000" is a true, checkable claim about one project. Stripped of that
project (which promotion, by design, must do — `validate_global_content`'s
`project_identifying` class, §2a, exists precisely so the promoted record names no project),
the claim that remains — "this port is 9000" — was never independently checked against *any*
project the personal or team record might later apply to. Carrying a verified state forward
would assert a verification that no longer has a subject to be verified against.

This is not a new principle invented for promotion — it is 003's own doctrine, "a model's
opinion is never verification" (the rule that separates an agent's assertion from a
deterministic check), extended one step further: **a deterministic check's *scope* is also
part of what makes it verification, and that scope does not survive promotion.** An
attestation was never trustworthy on its own; a verified-but-now-scopeless claim would have
been a new, narrower failure mode. D452 closes it more strongly than a reset ever could: not by
discarding the value, but by never having a field to hold one.

**Check 5, `verification_reset` (§2), never refuses and resets nothing.** It keeps its name and
its fixed position from D416's original numbering, but there is no `verification` column on
`personal_knowledge` or `team_knowledge` for it to write to (§5) — the check exists at this
position in the sequence solely to make the absence explicit at exactly the point a project
memory becomes a personal or team record, which is the one moment this discipline needs
restating even though no field anywhere changes value.

**SC-422** asserts this at the level that actually matters: the **stored and serialized**
representations of a promoted record, inspected field by field, in **both** the local SQLite
store and the server's Postgres store, and on the wire in between, carry no verification field
— a test that adds one, anywhere in that path, fails. **SC-424** extends the same field-by-field
inspection to every personal or team record regardless of origin — promoted or directly
authored — covering the full Layer A list (§5), not verification alone.

## 4. Origin as a salted digest — local-only, machine-salted, never transmitted (D434)

**FR-516 (replaced), FR-551, FR-552.** The original wording of FR-516 required two promotions
from the same project to be recognizable, full stop, while T015 (this feature's own test plan)
required two *machines* to produce *different* digests for the same project. Both cannot hold
if the digest travels: a digest that is the same across machines for the same project is, by
definition, the same across machines. **The contradiction is resolved by making the digest
never leave the machine that computed it.** `origin_digest` is scoped to one machine's
recognition, not to one user's or one project's, and every downstream statement in this
contract now says so.

Follows `crates/cairn-core/src/paths.rs:97-108`'s existing pattern-origin mechanism, applied to
personal and team records instead of patterns:

```rust
// crates/cairn-store/src/patterns.rs:460-464 (the existing precedent, unchanged)
fn origin_ref(project_id: Uuid) -> Result<String> {
    let salt = cairn_core::paths::machine_salt()?;
    Ok(cairn_core::digest(&format!("{salt}:{project_id}")))
}
```

The new `origin_digest` field on `personal_knowledge` and `team_knowledge` computes
identically: `digest("{machine_salt}:{source_project_id}")`, using the same
`machine_salt()` file already created for pattern promotion
(`paths::machine_salt_path()`, `paths.rs:86-88`) — this feature adds no second salt file,
because the property needed ("stable on this machine, meaningless off it, not reversible to
a project id") is identical to the one the salt already provides. Two promotions from the
same project, on the same machine, produce the same `origin_digest` — sufficient to
recognize "these two team entries both originated from wherever this machine's project X
is" without that digest, or anything derivable from it, naming project X.

**FR-551 — it MUST NOT be transmitted.** `origin_digest` is a local-only column, exactly like
`content_norm_digest` (§ everywhere that field is mentioned): it is written to the local store
and to the server's copy of the row it accompanies only as an opaque value the server never
interprets or compares — or, more precisely, wherever a server-side use of it would require
the server to hold it at all, this contract requires it be *withheld from the wire entirely*
rather than sent and merely unused. **The enumeration argument, stated in full**: the server
already knows every project identity on it — every `projects.id`, every `git_common_dir`, every
remote it has ever recorded. If `origin_digest` were transmitted, a party holding that
enumerable list could recompute `digest(candidate_salt : known_project_id)` for every project
it knows about and test the transmitted digest against each result — the digest would not need
to be *reversed*, only *matched* against a short, fully known candidate list, which is a far
cheaper attack than reversing a hash. Keeping the digest off the wire does not merely make this
harder; it removes the attack, because the party that could run it never receives the value to
test in the first place.

**FR-552 — the accepted cost, stated plainly.** Because the digest never travels and the salt
is machine-local, **two devices belonging to the same user do not correlate promotions made
from the same project.** A user who promotes the same fact from two different laptops produces
two `team_knowledge` (or `personal_knowledge`) rows with two different, uncorrelated
`origin_digest` values — `classify_proposal`'s content-based reconciliation (`global-memory.md`
§6) may still recognize them as the same fact by content, but nothing about `origin_digest`
itself will say "these came from the same project." This is an accepted limitation, not an
oversight: the alternative (a digest that correlates across machines) requires transmitting
something the server could enumerate against, and D434 chooses the smaller, explicitly-named
cost over the larger, silent risk.

**SC-441** is this section's verification obligation, stated as one scenario: two promotions
from the same project on one machine share an origin digest; the same two promotions made on
a second machine share a different one; and no origin digest appears in any transmitted
payload, verified by inspecting the wire.

**The six questions, answered (reproduced from the repair addendum verbatim):**

| Question | Answer |
|---|---|
| Two devices of the same user recognize the same source project? | **No.** Accepted limitation, FR-552. |
| Team promotions from different users recognize the same source project? | **No.** Different machines, different salts. |
| Local-only or server-visible? | **Local-only.** Never transmitted. |
| Where does the salt come from? | The existing machine-local salt already used for pattern `origin_ref`. No new key material, no new crypto. |
| Durable across machines? | **No,** deliberately. |
| Reversible or usable to enumerate project identities? | **No** — the server never receives it, so the party that knows every project id never holds a digest to test against it. Had it been transmitted, the server could have brute-forced it over its own project list; that is precisely why it does not travel. |

## 5. Structural privacy (D419) — Layer A in detail: the columns that deliberately do not exist

**FR-517 (re-derived per D456/F11 — see §2b).** `personal_knowledge` and `team_knowledge`
(full schemas in `global-memory.md` §2) have **no column** for — this is Layer A of §2b's
two-layer table, in full:

| Absent column class | What it would have let leak |
|---|---|
| A project identifier of any kind (no `project_id`, no `git_common_dir`) | Which project the knowledge came from |
| An evidence reference | Which files or runs backed the claim |
| An observation identifier | Which specific evidence record backed the claim |
| A verification field of any kind — no authority, no state, no timestamp (D452, §3) | Any claim that a deterministic check ran, or that any record — promoted or directly authored — carries a verification it never earned |

**A file path and a command are deliberately not on this list.** There never was a dedicated
`file_path` or `command` column for either table to lack — a path or a command that appears in
a personal or team record arrives as ordinary text inside `content` (Layer B), validated by
`validate_global_content` (§2a), not held back by a missing column. An earlier draft of this
table listed them here anyway, overstating a Layer A guarantee for a Layer B value; D456/F11's
re-derivation (§2b) removes them from this table for that reason, not because the leak vector
itself changed.

**Why absence beats validation, for the four classes that genuinely are absent columns**: a
validation rule ("reject this field if it contains a path") must be written correctly, must be
kept in sync with every new leak vector anyone discovers, and must run on every write path that
could reach the column — it is a rule someone must remember, and §6 below is a concrete case of
exactly that kind of rule silently falling out of sync with its own documentation. A column
that does not exist requires none of that: there is no code path, correct or buggy, that can
populate a column absent from the table. This is 003's own doctrine, quoted directly from its
migration:

> "A record with nowhere to go cannot be sent by mistake."
> — `cairn-server/migrations/0002_project_intelligence.sql:1-12`

applied here to *storage* rather than to *transmission* — the same argument, one layer
earlier: it is not merely that these fields never cross the wire, it is that the personal and
team tables could not hold them even if a bug tried to write one in.

## 6. The repair: the wire check does not enforce what it claims to

**FR-531 through FR-535.** The doc comment on the server's field check states an allowlist:

```rust
// crates/cairn-server/src/sync.rs:295-296
/// The allowlist enforced on the wire.
fn reject_forbidden_fields(item: &SyncItem) -> Result<(), ApiError> {
```

The body it documents is a **non-recursive, top-level-key-only denylist**:

```rust
// crates/cairn-server/src/sync.rs:296-313 (structure, condensed)
fn reject_forbidden_fields(item: &SyncItem) -> Result<(), ApiError> {
    if FORBIDDEN_ENTITY_TYPES.contains(&item.entity_type.as_str()) { return Err(...); }
    let Some(object) = item.payload.as_object() else { return Ok(()); };
    for field in FORBIDDEN_OBSERVATION_FIELDS {
        if object.contains_key(*field) { return Err(...); }
    }
    // ... FORBIDDEN_SESSION_FIELDS, same shape
    Ok(())
}
```

Two things are true about this function that its own doc comment does not say: it is a
denylist (a fixed list of *forbidden* names), not an allowlist (a fixed list of *permitted*
names) — the distinction matters because an allowlist fails closed on an unrecognized field
and a denylist fails open on one; and `object.contains_key(*field)` only ever inspects the
payload's **top-level** keys — nothing recurses into a nested object or array element, so a
forbidden name one level deep is invisible to it.

### The enumerated live leaks in `handoff_payload`

`handoff_payload` (`crates/cairn-store/src/outbox.rs:459-478`) sends `changed_files`,
`completed_work`, `failures`, `decisions`, and `tests_executed` verbatim. None of those four
*array* names is itself on the forbidden list — the forbidden list contains the names of the
*fields that fill them* (`"path"`, `"summary"`, `"command"`, `"outcome"`), which the
non-recursive check never sees once they are nested inside those arrays:

| Vector | Where it originates | Why the current check misses it |
|---|---|---|
| Absolute paths in `changed_files` | `derive_changed_files` (`cairn-core/src/handoff.rs:82-105`) collects `o.path` from `FileChanged` observations verbatim — the doc comment at `handoff.rs:91` says outright, "Observations carry absolute paths" | `changed_files` is not `"path"`; the array's *elements* are strings, not objects with a `"path"` key, so there is no key for the denylist to match at any depth |
| The same paths again, reformatted, in `completed_work` | `derive_completed` (`handoff.rs:152-171`) formats up to ten of `changed_files`'s entries into a prose sentence: `"Changed {n} file(s): {shown.join(", ")}{suffix}"` | The forbidden name `"path"` never appears; the paths are inside a free-text string, not a keyed field at all |
| Observation summaries in `failures` | `derive_failures` (`handoff.rs:119-131`) pushes `o.summary` for every `Error` observation and every failed `TestRun` | `"summary"` is on the forbidden list, but `failures` is an array of plain strings, not of `{summary: ...}` objects — the summary text is *inlined*, so there is no `"summary"` key anywhere in the payload for the check to find |
| Observation summaries in `decisions` | `derive_decisions` pushes `o.summary` for every `Decision` observation, the same shape | Same reason: no `"summary"` key exists in the transmitted structure |
| Commands in `tests_executed[].command` | `TestRunRecord { command, outcome, occurred_at }` (`handoff.rs:109-117`), sent as `h.tests_executed` inside `handoff_payload` | `"command"` and `"outcome"` **are** forbidden names, but they are one level inside each array element (`tests_executed[i].command`), and `object.contains_key("command")` only checks the top-level payload object, never `tests_executed[i]` |

### The fix, specified as behavior

The corrected check **recurses**: it walks every object and array in the payload, at every
depth, and rejects if any object anywhere contains a forbidden key — not only the top-level
object. This alone closes the `tests_executed[].command`/`.outcome` vector.

The `changed_files`/`completed_work`/`failures`/`decisions` vectors are not closed by
recursion alone, because their leak is in the *values*, not in forbidden *keys* — a
denylist-on-keys, however deep it recurses, cannot catch an absolute path sitting inside an
otherwise-permitted array of strings. The fix there is a **behavior change at the source**,
not a deeper check:

- `changed_files` MUST carry repository-relative paths only. `derive_changed_files`
  (`handoff.rs:82-105`) is corrected to relativize every path against the repository root
  before it is collected — not merely deduplicated against a relative counterpart as today's
  "keep the shorter, repository-relative form and drop the absolute duplicate" logic does,
  but to relativize unconditionally, so a path with **no** relative counterpart (the case the
  current dedup logic does not handle, since it only fires when *both* forms are present) is
  still relativized rather than surviving verbatim.
- `completed_work`'s prose formatting reads from the now-always-relative `changed_files`, so
  it inherits the fix with no separate change.
- `failures` and `decisions` keep observation **summaries** — this feature does not remove
  them, because a failure/decision summary is meant to be human-readable prose about what
  happened, not raw evidence. Nothing in FR-531 through FR-535 asks for `failures`/`decisions`
  content itself to change; the leak this contract closes is specifically the **absolute
  path** class, which does not appear in an ordinary error or decision summary unless the
  summary itself happens to quote one — and that residual risk is already covered by the
  existing secret-pattern redaction (`cairn-core/src/redact.rs`, run at capture time,
  `crates/cairnd/src/capture.rs:49-52`) rather than by this field-shape check, which was
  never meant to be a content scanner.
- `tests_executed` keeps `name`-equivalent and `outcome`, drops `command`: the record shape
  sent on the wire becomes `{outcome, occurred_at}` (dropping `command` entirely, since
  `TestRunRecord` has no separate `name` field today — the closest analog, `command`, is
  exactly what must not travel). A caller inspecting sync history can see that N tests ran
  and how many passed; it cannot see what was run.

**This is a removal and a normalization, not an addition** — no new field appears on the
wire, one field (`tests_executed[].command`) is dropped, and one existing field
(`changed_files`, and by extension the paths embedded in `completed_work`'s prose) is
constrained to a narrower value space than it holds today. Because
`reject_beyond_capability`'s `carries_meaning` check
(`crates/cairn-server/src/sync.rs:221-238`) already treats an *absent* field as meaningless
rather than as a capability requirement, **an old server accepts this payload without a 409**:
it receives a `handoff` payload with one fewer key than before and values narrower than
before, both of which are strict subsets of what it already accepted, so no capability
advertisement, negotiation, or schema bump is needed for this specific repair (unlike the
genuinely new `personal_knowledge`/`team_knowledge` entity types in `sync-namespaces.md`,
which do require one).

## 7. The stale client-side duplicate

**FR-534.** `crates/cairn-core/src/wire.rs:1700-1708`'s `REJECTED_OBSERVATION_FIELDS` lists
seven names — the original Feature-001 set. The server's live, enforced list,
`FORBIDDEN_OBSERVATION_FIELDS` (`crates/cairn-server/src/sync.rs:21-56`), has twenty-seven —
the seven plus twenty added for Feature 003. No production code reads the client-side
constant to make a decision (confirmed by grep across `crates/`); it functions purely as
documentation-shaped code that happens to compile, and it is wrong. This feature's fix:
either delete `REJECTED_OBSERVATION_FIELDS`/`REJECTED_SESSION_FIELDS` from `wire.rs` entirely
(since nothing reads them and a stale duplicate is worse than no duplicate), or replace them
with a doc-comment pointer to the server's list as the single source of truth
(`"see cairn-server::sync::FORBIDDEN_OBSERVATION_FIELDS — the authoritative, enforced list;
this crate keeps none of its own"`). Either resolution is acceptable; what is not acceptable
is leaving a twenty-item-short list presented as if it were the boundary, which is precisely
what invited this contract's own §6 finding to go unnoticed for as long as it did.

## 8. Contract documentation corrected to match deployed behavior

**FR-534**, applying to `specs/003-project-intelligence/contracts/privacy-sync.md`
specifically: its claims of "an explicit field allowlist," 16 or 19 forbidden field names, and
6 forbidden entity types are each corrected in place to state what §6 and the inventory
established as fact: a top-level-only denylist, 27 forbidden field names, 9 forbidden entity
types. This feature's own writing follows that corrected count throughout (`identity-
administration.md` and `sync-namespaces.md` both cite the 7-vs-27 discrepancy directly rather
than repeating the old contract's numbers).

## 9. Other permitted-immutability exceptions this contract does NOT create

To be explicit about scope: `personal-knowledge`/`team_knowledge`'s content, once promoted,
is exactly as immutable as any directly-authored entry (`global-memory.md` §3) — promotion
creates a normal immutable row, not a specially-privileged mutable one.

**FR-519**: deleting or forgetting the source memory a personal or team record was promoted
from does not alter, hide, or delete the promoted record — mirroring the existing pattern
precedent exactly ("Source memory deleted → Pattern survives; `source_memory_id` cleared,
`origin_deleted = 1`," `specs/003-project-intelligence/contracts/privacy-sync.md`'s deletion
table). The promoted personal or team record carries no `source_memory_id` at all (§5 — no
column for it), so there is nothing to clear; the record simply continues to exist, entirely
independent of its source's later fate, because it was never a reference to that source in
the first place.

**FR-520**: a promotion refusal is reported synchronously, in the same response that
requested the promotion (the `cairn_remember action=promote target=personal|team` call
returns `PromotionRejection` directly in its reply body) — never through a separate poll, a
notification, or a deferred status the caller must come back and check.

## Invariants

1. `evaluate_promotion` performs no I/O and reads no clock; every output is a pure function
   of its explicit arguments (D415).
2. The eight checks in §2 run in fixed order; the first failing check stops evaluation and
   is the only one named in the response (D416, revised by D433, revised again by D446).
3. `PromotionRejection` carries a check name only, never the content that failed it
   (FR-510, FR-511, FR-512).
4. A personal or team record carries no verification field of any kind, regardless of the
   source memory's verification state when the record arrived by promotion — there is
   nothing to reset because there is nothing to hold a value (FR-513, FR-517, D417, D452,
   SC-422, SC-424).
5. Every applicability fact on a promoted record has already passed the same closed-
   vocabulary validation, and the same nine-class content screen, as a directly-created
   entry; none is silently dropped (FR-514, FR-578).
6. A team promotion is refused unless the promoter is a member of the source project, and
   always lands `proposed`, never `authoritative` (FR-515).
7. `origin_digest` is `digest(machine_salt || source_project_id)`, computed and stored
   locally only, and is never transmitted to the server in any payload (FR-516, FR-551,
   D434).
8. `personal_knowledge` and `team_knowledge` have no column capable of holding a project
   identifier, an evidence reference, an observation identifier, or a verification field of
   any kind; a file path or a command, if present, lives only inside the validated `content`
   field (Layer B, §2b), never in a column of its own (FR-517, D419, D456/F11).
9. Any promotion-gate check that cannot be evaluated for missing input refuses rather than
   proceeding (FR-518).
10. `handoff_payload`'s field check recurses into every nested object and array, not only
    the top level (FR-535); `changed_files` and every path embedded in `completed_work`'s
    prose are repository-relative only; `tests_executed` entries carry no `command` field.
11. The corrected handoff payload is accepted without a `409` by a server that predates
    this feature, because it removes and narrows fields rather than adding any.
12. Deleting or forgetting a memory that was promoted leaves the promoted personal or team
    record completely unchanged (FR-519).
13. A promotion refusal is always returned in the same response that requested the
    promotion (FR-520).
14. `validate_global_content` runs on all five entry points capable of creating global
    content — direct personal creation, personal promotion, team proposal, team promotion,
    and server-side synchronization ingest — and no such entry point may bypass it (FR-544,
    FR-545, D433, D447).
15. A rejection from `validate_global_content` reports its class only, one of the ten in
    §2a's fixed set, and never the content, key, or applicability value that failed it
    (FR-546, FR-547).
16. A rejected direct creation, promotion, or team proposal leaves no row, no partial row,
    and no outbox entry in any table; a rejected ingest is not persisted, not partially
    persisted, and not acknowledged as delivered (FR-548, FR-581).
17. `validate_global_content` fails closed: an input it cannot evaluate is rejected, never
    silently passed — with the single named exception that an *empty* `project_identities`
    slice passes the `project_identifying` check rather than failing it, verified
    separately from the genuinely-unevaluable case (FR-549, FR-580, SC-454).
18. Every privacy guarantee this contract or `global-memory.md` states names its layer —
    Layer A (no column exists) or Layer B (validated free text) — and no Layer B value —
    `content`, a topic key, a value key, or an applicability value — is ever described as
    structurally incapable of carrying a path or a command (FR-550, SC-467).
19. `origin_digest` never appears in any transmitted sync payload, in either direction
    (FR-551); two devices of the same user promoting the same fact do not share an
    `origin_digest`, an accepted and documented limitation (FR-552).
20. Server-side ingest calls `validate_global_content` with `project_identities` set to the
    union of the pushing user's project memberships; a refusal there is permanent, reports a
    class only, never enters the `blocked` outbox state, and never throttles the namespace,
    distinguishably from a capability refusal by a typed field rather than a message string
    (FR-577, FR-581, D447, SC-449, SC-456).
21. `validate_global_content` is the only implementation of its nine classes; no gate, no
    ingest handler, and no client-side pre-check re-implements, duplicates, or partially
    restates any of them, verified by an audit that fails when a second implementation
    appears (FR-579, SC-453).
