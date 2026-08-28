# Feature Specification: Cairn Collaborative Global Memory

**Feature Branch**: `004-collaborative-global-memory`

**Created**: 2026-08-21

**Status**: Draft

**Input**: Cairn extends its project-scoped memory with two new domains that follow the
*person*, not the project: personal knowledge that follows one user across every project
they touch and every device they use, and team knowledge that any member can propose but
only an administrator can ratify into the server's shared default. Both are additive and
bounded: structurally incapable of displacing project truth, and prevented from naming the
project they came from by a shared validator that every creation path must pass — a rule
enforced in code, not a shape the schema makes impossible. Built on administered accounts,
explicit project membership and a safe auto-link, replacing the self-registration and
open-join paths this feature retires.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Administered accounts, no self-registration (Priority: P1)

An administrator creates every account that can reach Cairn. No one signs up on their
own. A new account gets a one-time temporary password and can do nothing else until it is
changed. Disabling an account cuts off access immediately, including any API token a
client already has cached.

**Why this priority**: Every other capability in this feature — project membership,
personal knowledge, team knowledge — assumes accounts and roles already exist and can be
trusted. This story closes the standing chain that let anyone become anyone: register,
discover a project, join it, read and write it.

**Independent Test**: On a server with no accounts, confirm no public route creates one.
Have an admin create a user, confirm the returned temporary password authenticates
exactly once for the password-change route and nothing else. Disable the account and
confirm a request bearing its still-unexpired token is refused.

**Acceptance Scenarios**:

1. **Given** a running server, **When** an unauthenticated request attempts to create an
   account by any route, **Then** it is refused and no account is created.
2. **Given** an administrator creating a new account, **When** creation succeeds, **Then**
   a temporary password is returned exactly once, the account is marked as requiring a
   password change, and requesting it again returns nothing.
3. **Given** an account requiring a password change, **When** it attempts any action other
   than changing its password, **Then** it is refused; **when** it attempts to mint an API
   token, **Then** that is refused too.
4. **Given** an active account with a valid, cached API token, **When** an administrator
   disables that account, **Then** a request bearing that token is refused immediately, and
   every token issued to that account stops working.
5. **Given** a server with exactly one administrator, **When** someone attempts to demote
   or disable that administrator such that zero admins would remain, **Then** the action is
   refused.
6. **Given** a member who never used their temporary password, **When** an administrator
   resets that member's password, **Then** a new temporary password is returned exactly
   once, the previous one no longer authenticates, and the member is again required to
   change it before doing anything else.
7. **Given** an active account with an API token past its expiry, **When** a request bears
   that token, **Then** it is refused, indistinguishably from a request bearing a revoked
   token.

---

### User Story 2 - Explicit membership, safe discovery, safe auto-link (Priority: P2)

Joining a project requires an existing member or an admin to add the joiner. Looking up a
project by its shared identity only shows projects the caller already belongs to. Running
`cairn link` in a freshly cloned repository with no project specified can safely
auto-select the one project the caller already belongs to — never one the caller merely
discovered.

**Why this priority**: This closes the other half of the same authorization chain as
Story 1, and it is the prerequisite for every domain that moves knowledge between
machines: a link that could ever attach the wrong project would poison personal and team
knowledge alike.

**Independent Test**: Add a user to a project explicitly. Confirm lookup by shared
identity returns that project only to members. Clone the repository on a second machine
as that same user, run link with no project argument, and confirm it selects the same
project with no prompt.

**Acceptance Scenarios**:

1. **Given** a project the caller is not a member of, **When** the caller looks it up by
   shared identity, **Then** it does not appear in the results.
2. **Given** a user already a member of a project, **When** that member or an admin adds
   another user to it, **Then** the added user can subsequently see and sync that project,
   and the addition records who added them and when.
3. **Given** a freshly cloned repository whose remote matches exactly one project the
   caller belongs to, **When** `cairn link` runs with no project specified, **Then** that
   project is selected automatically with no prompt.
4. **Given** a freshly cloned repository whose remote matches zero or more than one of the
   caller's memberships, **When** `cairn link` runs with no project specified, **Then**
   auto-link does not occur and the caller is asked to specify one.
5. **Given** a user removed from a project's membership, **When** that user next attempts
   to sync or read it, **Then** access is refused.
6. **Given** an admin viewing a project's membership list, **When** membership changes,
   **Then** the list reflects the change immediately.

---

### User Story 3 - Personal knowledge follows me across my projects (Priority: P3)

A habit, a preference, or a fact a person keeps re-learning — recorded once as personal —
shows up unprompted the next time that same person opens a different project of theirs,
without naming where it came from, and without ever being visible to anyone else.

**Why this priority**: The first payoff of a domain that outlives one project. It needs
only one user and one machine to demonstrate, and it is testable in complete isolation
from team knowledge or multi-device sync.

**Independent Test**: Record a personal note while working in project A. Open unrelated
project B as the same user and confirm the note is retrievable there. Confirm a different
user never sees it in either project.

**Acceptance Scenarios**:

1. **Given** a user recording a personal note while in project A, **When** that user later
   opens project B, **Then** the note is retrievable there without naming project A.
2. **Given** a personal note with no applicability facts, **When** it is evaluated against
   any project, **Then** it applies to that project.
3. **Given** a personal note restricted to one language, **When** it is evaluated against a
   project lacking that language among its derived traits, **Then** it does not apply; in a
   project that does carry the trait, it does.
4. **Given** a personal note recorded by one user, **When** any other user searches or
   opens context in any project, **Then** that other user never sees it.
5. **Given** a personal note its owner no longer wants, **When** that owner forgets it,
   **Then** it stops appearing in search and in context.
6. **Given** an applicability value outside the closed vocabulary, **When** a personal
   note attempts to use it, **Then** creation is refused rather than storing it with the
   value silently dropped.

---

### User Story 4 - Personal knowledge follows me to my second device (Priority: P4)

The same person, working from a laptop and later a workstation, sees the same personal
knowledge on both — even when each device recorded something before either had ever
synced, and even when the two devices' clocks disagree about what time it is.

**Why this priority**: Cross-device is the harder half of "personal knowledge," and the
story that proves the domain is genuinely synchronized rather than merely cached on one
machine. It validates the multi-writer safety the rest of the feature depends on.

**Independent Test**: Record a personal note on device A while offline. Separately record
a disagreeing personal note on device B while offline. Bring both online, sync, and
confirm both notes survive on both devices regardless of sync order or relative clock
skew.

**Acceptance Scenarios**:

1. **Given** a personal note recorded on device A, **When** device B, same user, next
   syncs, **Then** the note is retrievable on device B.
2. **Given** both devices offline and each recording a disagreeing personal note on the
   same subject, **When** both sync, **Then** both notes survive on both devices and are
   marked as disagreeing, independent of which device synced first or which clock was
   ahead.
3. **Given** two devices for one user independently producing byte-identical personal
   notes, **When** both sync, **Then** neither note is lost and neither is discarded as a
   duplicate of the other's write.
4. **Given** a device with nothing queued to push, **When** time passes without that
   device writing anything, **Then** personal knowledge recorded on another of that user's
   devices still arrives.

---

### User Story 5 - Team guidance: proposed by anyone, decided by an admin (Priority: P5)

A member notices something true across the whole server's projects and proposes it as
team knowledge. It stays invisible to everyone — including the proposer — until an
administrator ratifies it, at which point it becomes the server's shared default
guidance. An administrator can retire it later.

**Why this priority**: The highest-authority domain in the feature, deliberately last
among the write-capable stories because it depends on roles (Story 1) and membership
(Story 2) already being trustworthy, and because getting write authority wrong here is
the most damaging mistake this feature could make.

**Independent Test**: Have a member propose a team entry and confirm no one sees it in
recall. Have an admin ratify it and confirm it now appears server-wide. Have an admin
retire it and confirm it stops appearing.

**Acceptance Scenarios**:

1. **Given** a member of a project proposing a team knowledge entry, **When** the proposal
   is recorded, **Then** it does not appear in anyone's recall, including the proposer's.
2. **Given** a proposed entry, **When** a non-admin attempts to ratify it, **Then** the
   action is refused.
3. **Given** a proposed entry, **When** an admin ratifies it, **Then** it becomes visible in
   recall to every user on the server, labelled with who proposed it and who ratified it.
4. **Given** an authoritative team entry, **When** an admin retires it, **Then** it stops
   appearing in recall while its history remains inspectable.
5. **Given** two admins independently attempting to ratify the same proposal from what each
   believes is its current state, **When** both attempts race, **Then** exactly one
   succeeds and the other is told the entry's actual current state rather than being
   silently applied on top of it.

---

### User Story 6 - One answer, bounded, project truth always first (Priority: P6)

Asking Cairn for context or searching returns project knowledge exactly as it always has.
Personal and team knowledge are appended only into space project knowledge does not need,
never displacing anything project-specific, and never appearing at all in the terse
minimum-depth briefing.

**Why this priority**: Last, because it is the safety property that makes every other
domain story trustworthy. It proves that adding two new domains cannot regress the one
guarantee the whole system already depended on: guaranteed context is project-only.

**Independent Test**: Fill a project's context to its budget with project-only content,
then confirm adding personal and team knowledge leaves the briefing byte-identical. Free
up budget and confirm bounded personal and team sections then appear, capped, never
inside the guaranteed reserve.

**Acceptance Scenarios**:

1. **Given** a project whose guaranteed and ranked context already consumes the full
   budget, **When** personal and team knowledge exist, **Then** the assembled briefing is
   unchanged and contributes zero global content.
2. **Given** headroom in the ranked portion of the budget, **When** context is assembled
   at standard depth, **Then** personal and team sections appear after every project
   section, bounded to no more than the documented share of the total budget.
3. **Given** a request at minimum depth, **When** context is assembled, **Then** no
   personal or team content appears regardless of available headroom.
4. **Given** a search spanning domains, **When** results are returned, **Then** project,
   personal and team results appear in three separate arrays, never merged, and the
   project result count is unaffected by the other two.

---

### Edge Cases

- **An old server that predates this feature.** It refuses the two new entity types by
  capability, exactly as it would refuse any unknown type; the personal and team
  namespaces sit blocked while the project namespace keeps synchronizing at full speed.
- **Two devices, offline, creating contradictory personal knowledge.** Handled the same
  deterministic way as a project-level disagreement: both entries survive, both are
  marked as disagreeing, and no clock or arrival order decides a winner.
- **A promotion whose content contains an absolute path.** The gate refuses it by name at
  the class it matched, echoes none of the offending text, and leaves no partial personal
  or team record behind.
- **A team entry that is never ratified.** It remains `proposed` indefinitely, invisible
  to every recall surface including its proposer's, and is never counted as team
  knowledge anywhere until an admin acts on it.
- **A disabled user with a still-cached API token.** The token is revoked at the moment of
  disabling, not at its next use, so a request bearing it is refused immediately rather
  than succeeding once more.
- **A temporary password that is never changed.** The account remains permanently
  confined to the password-change route — no token can ever be minted for it — until an
  administrator resets it.
- **A local store linked to two different server instances.** It records which server
  instance its team knowledge came from and refuses to merge a second instance's team
  knowledge into the same store, treating the mismatch as a capability boundary rather
  than silently combining two teams' guidance.
- **A project with no derivable traits.** Only universal personal and team records — those
  with no applicability facts — apply to it; any record restricted to a kind never
  matches, because there is nothing for it to match against.
- **Global knowledge when the token budget is already fully consumed by project
  sections.** It contributes nothing, and the assembled briefing is byte-identical to what
  this feature's predecessor produced.
- **A promoted record whose source memory is later forgotten.** The promoted record is
  unaffected: it carries no live reference back to its source, only a salted digest of
  the source project's identity, so forgetting the source changes nothing about it.
- **A client that skips its own local validation and pushes project-identifying or
  command-shaped content to the server.** The server runs the same content validator at
  ingest, scoped to every project that pushing user is a member of; the record is refused,
  never persisted, and never reaches that user's other devices.
- **A pre-004 client synchronizing against a 004 server.** Project synchronization
  continues unchanged; only the removed self-registration and self-join routes stop
  working, each answering with a stable, documented status naming its replacement rather
  than a bare not-found.

## Requirements *(mandatory)*

### Identity, roles and account lifecycle

- **FR-401**: System MUST require every user account to be created by an administrator;
  no unauthenticated or self-service path may create one.
- **FR-402**: System MUST assign every user exactly one role, admin or member, at the
  server level, not per project.
- **FR-403**: Account creation MUST reveal the temporary password exactly once, in the
  creation response itself. It MUST NOT be stored in retrievable form and MUST NOT be
  obtainable afterward by any route, including by the administrator who created the
  account.
- **FR-404**: A newly created account MUST be marked as requiring a password change
  before it can do anything else.
- **FR-405**: Users MUST be able to change their own password, and doing so MUST end the
  requirement to change it.
- **FR-407**: System MUST refuse every authenticated action other than the password
  change itself while an account requires a password change — including minting an API
  token — so its temporary credential authenticates only to the password-change route and
  to nothing else.
- **FR-408**: Administrators MUST be able to disable an account and to re-enable it.
- **FR-409**: Disabling an account MUST immediately revoke every API token issued to that
  account, so a token already cached by a client cannot outlive the account.
- **FR-410**: A disabled account MUST be refused authentication by any means, including a
  password that remains otherwise valid.
- **FR-411**: Administrators MUST be able to list accounts along with each one's role and
  status.
- **FR-412**: Administrators MUST be able to promote a member to admin and to demote an
  admin to member.
- **FR-413**: System MUST never leave a server with zero active administrators.
  The guarantee MUST be enforced atomically within the single statement that performs the
  demotion or disable — conditioned on another active administrator still existing — and
  MUST NOT be implemented as a separate count followed by an update, which two concurrent
  requests can both pass.
- **FR-414**: When existing accounts are first assigned roles, the account matching the
  server's configured administrator identity MUST become admin; where none is configured,
  the oldest account MUST become admin; every other existing account MUST become member.
- **FR-415**: System MUST assign every server instance a single identity, established
  once and never reassigned.
- **FR-416**: A server's instance identity MUST be discoverable by every client connected
  to it.
- **FR-417**: Administrators MUST be able to set an expiry on an API token; tokens MAY
  also continue to be issued with none.

### Break-glass administration

*These requirements were allocated after the FR-401..FR-417 block closed, and are kept
here rather than at the end of the document because they belong to identity. Ids are not
monotonic with document order; the block is semantic, not positional.*

- **FR-539**: The account named by the server's environment MUST always be restored to
  administrator and to active status when the server starts, so that no mistaken demotion
  or deactivation — including of the last remaining administrator — can leave the server
  with no way to administer it.
- **FR-540**: The environment-named account MUST be exempt from the forced
  password-change requirement, because the environment re-establishes its password on every
  start and a forced change would be undone by the next restart.
- **FR-541**: System MUST refuse any attempt to deactivate or demote the
  environment-named account, and the refusal MUST name the environment setting that
  governs it, rather than accepting a change that a restart would silently revert.
- **FR-542**: An account defined by the server's own environment MUST NOT be treated as
  self-registration for the purposes of FR-401, because it is configured by the operator
  who controls the server rather than claimed by whoever reaches it first.
- **FR-543**: Documentation MUST state plainly that whoever can set the server's
  environment and restart the process can always obtain administrator access, since this is
  the outer boundary of the role model and not an implementation detail.

### Project membership and safe discovery

- **FR-418**: System MUST NOT provide any route through which a user adds themselves to a
  project's membership.
- **FR-419**: An existing member or an admin MUST be able to add another user to a
  project, and the addition MUST record who added them and when.
- **FR-420**: An existing member or an admin MUST be able to remove a user from a
  project's membership.
- **FR-421**: Removing a user from a project's membership MUST take effect immediately:
  a subsequent sync or read request for that project from that user MUST be refused.
- **FR-422**: Looking up a project by its shared identity MUST return only projects the
  requesting user is already a member of.
- **FR-423**: Users MUST be able to list the projects they are a member of.
- **FR-424**: Linking a local project with no project specified MUST auto-select a server
  project only when exactly one of the caller's memberships matches, and MUST do so
  without prompting the caller to join anything they were not already a member of.
- **FR-425**: Where zero or more than one of the caller's memberships match, linking with
  no project specified MUST require the caller to specify one explicitly rather than
  guess.
- **FR-426**: Discovery MUST NOT be able to grant membership under any circumstance; it
  may only report membership that already exists.
- **FR-427**: Administrators MUST be able to view a project's full membership list.
- **FR-428**: `cairn link` MUST continue to accept an explicit project argument regardless
  of whether auto-link would apply, and an explicit choice MUST NOT be overridden by
  auto-link.

### Personal global memory

- **FR-431**: A user MUST be able to record a personal knowledge entry from within any
  project they can access.
- **FR-432**: A personal knowledge entry MUST belong to exactly one user and MUST never
  be visible to any other user, through any surface.
- **FR-434**: A personal knowledge entry MAY carry zero or more applicability facts, each
  naming a kind and a value drawn from a closed, documented vocabulary.
- **FR-435**: A personal knowledge entry with no applicability facts MUST apply to every
  project.
- **FR-436**: A personal knowledge entry with one or more applicability facts MUST apply
  to a project only when, for every kind it names, at least one of its values for that
  kind is among that project's derived traits.
- **FR-437**: A project's traits MUST be derived deterministically from files already
  present in its working tree, and MUST NOT be guessed, inferred from file content, or
  asked of a language model.
- **FR-438**: Project traits MUST remain local to the machine that derived them and MUST
  NOT be synchronized.
- **FR-439**: A user MUST be able to view the traits derived for a given project.
- **FR-440**: A personal knowledge entry MUST be immutable after creation; a change MUST
  be recorded as forgetting the entry and, where applicable, creating a new one.
- **FR-441**: A user MUST be able to forget a personal knowledge entry, after which it
  MUST NOT be returned by search or by context.
- **FR-442**: Personal knowledge MUST reconcile among one user's own entries using the
  same deterministic reconciliation already used for project memory: content-identical
  entries deduplicate, entries sharing a subject but differing in content coexist, and
  entries disagreeing on the same subject are marked in conflict rather than one silently
  prevailing.
- **FR-444**: A user MUST be able to search their own personal knowledge and retrieve it
  in context regardless of which project they are currently in.
- **FR-445**: A personal knowledge entry MUST record which local writer created it and a
  per-writer sequence number, used only to detect gaps or duplicates within that writer's
  own stream.
- **FR-446**: An applicability value outside the closed vocabulary MUST cause the
  creation of a personal knowledge entry to be refused rather than stored with the value
  silently dropped.

### Team global memory

- **FR-451**: A member of at least one project MUST be able to propose a team knowledge
  entry.
- **FR-452**: A proposed team knowledge entry MUST NOT be returned by search, context or
  any other recall surface until it is ratified.
- **FR-453**: Only an admin MUST be able to ratify a proposed team knowledge entry,
  transitioning it to authoritative.
- **FR-454**: A ratification or retirement request MUST be refused, naming the entry's
  actual current state, when that state no longer matches what the request expected at the
  moment of the attempt.
- **FR-455**: The agent tool surface MUST NOT be able to create a team knowledge entry
  directly; the only path to one is a proposal followed by an administrator's ratification
  through the CLI or server administration.
- **FR-456**: An admin MUST be able to retire an authoritative team knowledge entry, after
  which it MUST NOT be returned by recall.
- **FR-457**: Every team knowledge state transition MUST be recorded with who acted and
  when, and MUST remain inspectable after the transition.
- **FR-458**: An authoritative team knowledge entry MUST be visible to every user on the
  server it belongs to, regardless of that user's project memberships.
- **FR-459**: A team knowledge entry's record of who proposed it MUST be kept only as a
  traceable reference, never as project-identifying content (see FR-517, FR-546 for the
  general project-identity prohibition).
- **FR-460**: A team knowledge entry MAY carry applicability facts under the same closed
  vocabulary and matching rule as a personal one; an entry with none applies to every
  project on that server.
- **FR-461**: Team knowledge MUST be immutable after creation; retiring an entry MUST NOT
  alter its content.
- **FR-462**: Two disagreeing authoritative team knowledge entries MUST both remain
  visible with the disagreement surfaced, never silently resolved by order of
  ratification.
- **FR-463**: A user with no membership in any project MUST still see authoritative team
  knowledge, because it is a server-wide default rather than something scoped to project
  membership.
- **FR-464**: A user MUST be able to list team knowledge entries and their state, scoped
  to what their role permits: a member sees authoritative entries and their own
  proposals; an admin sees every state.
- **FR-465**: Retiring a team knowledge entry MUST NOT be reversible by re-ratifying the
  same record; guidance restored after retirement MUST be recorded as a new proposal.

### Unified bounded recall and domain separation

- **FR-469**: Search results MUST report project, personal and team knowledge in three
  separate result arrays, never merged into one list.
- **FR-470**: A project search's result count and ranking MUST be unaffected by the
  presence or absence of personal or team results.
- **FR-471**: Each domain MUST be ranked within itself only; a relevance score computed
  within one domain's index MUST NOT be compared against another's.
- **FR-472**: A caller MUST be able to request search restricted to one or more specific
  domains.
- **FR-473**: The context assembler's guaranteed, reserved content MUST remain
  project-only; no personal or team content may ever be admitted into that reserve.
- **FR-474**: The combined personal and team sections MUST NOT exceed a fixed fraction of
  the **total context budget**, applied as a ceiling independent of how much space project
  sections left unused.
- **FR-475**: Where project sections already consume the entire budget, an assembled
  context MUST be unaffected in content or size by the existence of personal or team
  knowledge.
- **FR-476**: Personal sections MUST be considered ahead of team sections whenever both
  compete for the same remaining space, reflecting that personal knowledge is specific to
  the requesting user while team knowledge is the server-wide default.
- **FR-477**: A context request at minimum depth MUST exclude personal and team sections
  entirely, with no configuration able to override this.
- **FR-478**: When diagnostics are requested, the system MUST report for each personal or
  team item whether it was included or excluded and why, using the same selection reason
  vocabulary Feature 003 already defines for project sections. These reasons MUST NOT
  appear in the rendered briefing.
- **FR-480**: The estimated size of an assembled context MUST never exceed the requested
  budget, including when personal and team sections are present.
- **FR-481**: A caller with no personal or team knowledge of their own MUST see zero
  difference in project search or context relative to a caller who never touches either
  domain.
- **FR-482**: An importance hint on a personal or team item MUST NOT change its section's
  precedence and MUST NOT admit it into reserved context, exactly as the existing
  invariant already forbids for project memory.

### Synchronization, namespaces and multi-device concurrency

- **FR-486**: Personal and team knowledge MUST synchronize through their own namespaces,
  independent of the project namespace and of each other.
- **FR-487**: Each synchronization namespace MUST track its own pull position; advancing
  or resetting one namespace's position MUST NOT affect another's.
- **FR-488**: A synchronization failure or capability block in one namespace MUST NOT
  prevent, delay, throttle or interrupt another namespace's continued synchronization at
  full speed.
- **FR-489**: A namespace with nothing queued to push MUST still periodically check for
  content produced elsewhere, so that knowledge from another writer arrives without the
  receiving machine writing anything first.
- **FR-490**: Every local store MUST have a single, durable, opaque identity distinct
  from any user or project identity, established once when the store is first created.
- **FR-491**: That local identity MUST be incorporated into whatever makes a synchronized
  write unique, so that two different local stores producing byte-for-byte identical
  content are never mistaken for the same write.
- **FR-492**: A per-writer sequence number MUST be usable only to detect gaps or
  duplicates within that writer's own stream, and MUST NOT be compared across writers or
  used to break a tie between them.
- **FR-493**: Disagreement on the same subject — between two different writers' personal
  or team knowledge, or between one user's own personal entries — MUST be resolved the
  same deterministic way as project knowledge, by reconciliation and relations, never by
  comparing wall-clock time or arrival order, in either domain.
- **FR-495**: A local store MUST record which server instance its team knowledge came
  from.
- **FR-496**: A local store MUST refuse to merge team knowledge sourced from a different
  server instance than the one already recorded for it — team knowledge is bound to the
  server instance that ratified it — and the refusal MUST be reported to the user rather
  than silently dropped.
- **FR-497**: Backoff after a failed synchronization attempt MUST be tracked per
  namespace, so a failing personal or team namespace does not slow retries on the project
  namespace.
- **FR-498**: Personal and team entity types MUST each be advertised through the existing
  capability mechanism, and a server that does not support them MUST cause only those
  entity types to be held back, never the project namespace.
- **FR-499**: An older server's refusal of a personal or team item MUST leave that item
  retained locally in a recoverable state, neither delivered nor permanently failed, and
  MUST NOT be retried against that server.
- **FR-500**: When a server is subsequently upgraded to support personal or team
  knowledge, previously held-back items MUST be delivered without user intervention and
  applied exactly once.
- **FR-501**: A pulled personal or team record whose dependency has not yet arrived MUST
  be retried using the same bounded, oldest-first replay Cairn already uses for project
  records.
- **FR-502**: Releasing claimed but unfinished synchronization work at daemon start MUST
  apply independently across all three namespaces.

### Privacy boundary and the promotion gate

- **FR-506**: Promotion from project memory to personal or team knowledge MUST be an
  explicit act; Cairn MUST NOT promote automatically.
- **FR-507**: A promotion MUST pass a single fixed-order gate before any personal or team
  record is created; the first failing check MUST stop promotion and MUST be reported by
  name.
- **FR-508**: Promotion MUST be refused when the source memory is not currently active.
- **FR-509**: Promotion MUST be refused when the source memory carries no subject
  identity.
- **FR-510**: Promotion MUST be refused when the content validator refuses the source
  content (FR-546).
- **FR-511**: Promotion MUST be refused when the source content names the project it came
  from — by project name, by a component of its shared identity, or by a remote host,
  organization or repository token.
- **FR-512**: Promotion MUST be refused when the source content carries an evidence or
  observation identifier; a count of supporting evidence MAY travel, the identifiers
  themselves MUST NOT.
- **FR-513**: The stored and serialized representations of a personal or team record MUST
  contain no verification field of any kind — not an authority, not a state, not a
  timestamp — so that no promoted claim can present itself as verified.
- **FR-514**: Every applicability fact proposed at promotion MUST be validated against the
  closed vocabulary; a value outside it MUST cause the promotion to be refused rather than
  silently dropped.
- **FR-515**: Promotion to team knowledge MUST additionally be refused unless the
  promoter is a member of the source project, and MUST always land in the proposed state,
  never authoritative.
- **FR-516**: A promoted record MUST record its origin only as a salted digest of the
  source project's identity, and that digest MUST be sufficient to recognize two
  promotions made from the same project **on the same machine**.
- **FR-517**: A personal or team knowledge record MUST have no column for a project
  identifier, an evidence reference, an observation identifier, or any verification field,
  so that those values are structurally impossible rather than merely forbidden; a file
  path and a command are Layer B, not absent columns — they are free-text content,
  governed by the content validator (FR-544, FR-546).
- **FR-518**: Any promotion gate check that cannot be evaluated for lack of information
  MUST cause promotion to be refused rather than to proceed.
- **FR-519**: Deleting or forgetting the memory a personal or team record was promoted
  from MUST NOT alter, hide or delete the promoted record.
- **FR-520**: A promotion refusal MUST be reported synchronously, in the same response
  that requested the promotion, never through a separate channel the caller must poll.

### Compatibility, capability advertisement and migration

- **FR-521**: Introducing personal and team knowledge MUST NOT require any change to the
  existing memory scope or its stored representation.
- **FR-522**: Where a server that predates this feature causes the personal and team
  namespaces to sit blocked (FR-488), that degradation MUST be reported by name.
- **FR-523**: Migration of an existing local store MUST preserve every existing row
  unchanged and MUST assign every new field a documented default.
- **FR-524**: Migration of an existing server MUST assign a role to every existing
  account deterministically, per the documented backfill rule, and MUST never produce a
  server with zero admins.
- **FR-525**: An interrupted migration MUST leave the store on its prior, working schema
  version rather than a partially upgraded one.
- **FR-527**: The existing six-tool agent surface MUST NOT gain a seventh tool; every
  capability this feature adds MUST be exposed as new actions or fields on the existing
  six.
- **FR-528**: Adding the two new entity types to the local outbox MUST widen its existing
  type constraint through a migration that preserves every existing outbox row.
- **FR-529**: The two new capability names for personal and team knowledge MUST extend
  the existing one-way capability advertisement without introducing a handshake or a
  negotiation step of any kind.
- **FR-530**: Widening the outbox's entity-type constraint MUST be proven by rebuilding
  the store's real prior schema through its actual migration history and asserting row
  and byte equality before and after.

### Repairs

- **FR-531**: The handoff payload MUST NOT transmit an absolute local path in any field,
  including where one currently survives inside prose or inside a recorded shell command;
  every such value MUST be excluded or redacted before transmission.
- **FR-532**: The wire-level field check MUST be corrected to deny what it is documented
  to deny, and its documentation MUST match its enforced behavior.
- **FR-533**: The scope-ordering test that currently passes without exercising its target
  MUST be replaced with one that actually exercises the scope-ordering logic it claims to
  verify.
- **FR-534**: The privacy and synchronization contract documentation MUST be corrected to
  match the deployed field names, forbidden-field counts and entity-type counts, rather
  than describing a wire format that does not exist.
- **FR-535**: The corrected wire-level field check MUST deny a forbidden field wherever it
  appears in a payload, not only at the top level.

### Repairs from design analysis

These requirements were produced by a `/speckit-analyze` pass over this document and are
grouped here by the analysis finding that produced each one, rather than filed into the
subject block each one amends. Every entry below carries a trailing parenthetical naming
the subject section it amends.

- **FR-544**: System MUST validate the free-text content of every personal or team
  knowledge record with a single shared validator, so that the same rule applies no matter
  which surface created the record. (amends Privacy boundary and the promotion gate)
- **FR-545**: The content validator MUST run at all five entry points capable of creating
  global content: direct personal creation, personal promotion, team proposal, team
  promotion, and server-side synchronization ingest. No entry point may bypass it. (amends
  Privacy boundary and the promotion gate)
- **FR-546**: The content validator MUST reject content containing an absolute filesystem
  path, a home-directory reference, a drive-letter path, a `file://` reference, a URL
  carrying credentials, an environment-variable assignment, a run of characters shaped like
  an encoded secret, a token identifying a project, or a shell command invocation. (amends
  Privacy boundary and the promotion gate)
- **FR-547**: A rejection MUST report only the class of the rejection, and MUST NOT echo,
  quote, log, or return the offending content. (amends Privacy boundary and the promotion
  gate)
- **FR-548**: A rejected creation or promotion MUST leave no record, no partial record, and
  no queued outbox entry behind. (amends Privacy boundary and the promotion gate)
- **FR-549**: The content validator MUST fail closed: a check that cannot be evaluated
  rejects. (amends Privacy boundary and the promotion gate)
- **FR-550**: Documentation MUST distinguish, for every privacy guarantee it states,
  whether the guarantee holds because no column exists to carry the value or because free
  text is validated; a free-text field MUST NOT be described as structurally incapable of
  carrying a path or a command. (amends Privacy boundary and the promotion gate)
- **FR-551**: The origin digest MUST NOT be transmitted, so that the server, which knows
  every project identity, never holds a digest it could test those identities against.
  (amends Privacy boundary and the promotion gate)
- **FR-552**: Documentation MUST state that origin recognition is per-machine, and that two
  devices of the same user will not correlate promotions from the same project — an
  accepted limitation of keeping the digest off the wire. (amends Privacy boundary and the
  promotion gate)
- **FR-553**: Administrators MUST be able to reset another account's password. (amends
  Identity, roles and account lifecycle)
- **FR-554**: A password reset MUST return a new temporary password exactly once, on the
  reset response itself, and MUST NOT allow it to be retrieved again afterward. (amends
  Identity, roles and account lifecycle)
- **FR-555**: A password reset MUST invalidate the account's previous password
  immediately. (amends Identity, roles and account lifecycle)
- **FR-556**: A password reset MUST revoke every API token issued to that account. (amends
  Identity, roles and account lifecycle)
- **FR-557**: A password reset MUST place the account back into the state requiring a
  password change. (amends Identity, roles and account lifecycle)
- **FR-558**: Resetting the password of a disabled account MUST NOT re-enable it; the
  account MUST remain disabled and MUST remain unable to authenticate until an
  administrator separately re-enables it. (amends Identity, roles and account lifecycle)
- **FR-559**: A password reset MUST be refused for the account named by the server's
  environment, because its credential is re-established from that environment on every
  start and a reset would be silently undone; the refusal MUST name the environment
  setting. (amends Identity, roles and account lifecycle)
- **FR-560**: Two concurrent operations that would each individually be legal, but that
  together would remove the last administrator, MUST result in exactly one succeeding and
  one being refused. (amends Identity, roles and account lifecycle)
- **FR-561**: While a namespace is blocked for want of a server capability, the client MUST
  re-probe the server's advertised capabilities on a bounded, backed-off schedule. The
  probe is a capability read, not a retry of the held items, so the held items are still
  never retried against a server that cannot accept them. (amends Synchronization,
  namespaces and multi-device concurrency)
- **FR-562**: When a probe observes the required capability, the namespace MUST return to
  eligible and the held entries MUST be released for delivery preserving their original
  idempotency keys, so an entry that was partially delivered before is applied exactly
  once. (amends Synchronization, namespaces and multi-device concurrency)
- **FR-563**: The return to eligible MUST require no local write, no user command and no
  daemon restart. (amends Synchronization, namespaces and multi-device concurrency)
- **FR-567**: Personal knowledge MUST NOT be refused on the basis of server instance. A
  local store MUST be able to hold the personal knowledge of more than one identity,
  partitioned by the identity that owns it, and recall MUST surface only the personal
  knowledge of the identity currently linked. (amends Synchronization, namespaces and
  multi-device concurrency)
- **FR-568**: The personal synchronization namespace MUST be keyed by both the server
  instance and the owning account, so two identities of the same human never merge.
  (amends Synchronization, namespaces and multi-device concurrency)
- **FR-569**: The applicability vocabulary MUST contain only kinds that can be derived
  deterministically from files present in a working tree; it consists of `language` and
  `tool`. (amends Personal global memory)
- **FR-570**: Documentation MUST distinguish a record's own subject key from an
  applicability fact, since both were previously called a topic. (amends Personal global
  memory)
- **FR-572**: A successful password change MUST invalidate the temporary credential
  immediately. (amends Identity, roles and account lifecycle)
- **FR-573**: The only way to obtain a new temporary credential MUST be an administrator
  password reset (FR-553). (amends Identity, roles and account lifecycle)
- **FR-574**: Any operation that could reduce the number of active administrators MUST
  serialize against all other such operations on a single application-wide lock held for
  the duration of the transaction, so that the check and the write cannot interleave
  (amends Identity, roles and account lifecycle).
- **FR-577**: Server-side ingest MUST validate a personal or team record before persisting
  it, MUST refuse the item rather than storing it, and MUST report the refusal to the
  pushing client as a rejection class without echoing the offending content. (amends
  Privacy boundary and the promotion gate)
- **FR-578**: Applicability values MUST be validated by the same content validator and
  against the same classes as free-text content; a value that would be refused as content
  MUST be refused as an applicability value. (amends Privacy boundary and the promotion
  gate)
- **FR-579**: The content validator MUST be the only implementation of these classes; no
  other component may re-implement, duplicate, or partially restate them. (amends Privacy
  boundary and the promotion gate)
- **FR-580**: Where no project identity is available to screen against, the
  project-identifying check MUST pass rather than fail; a check with nothing to match is
  vacuous, and MUST be distinguished from a check that cannot be evaluated, which still
  fails closed. (amends Privacy boundary and the promotion gate)
- **FR-581**: A record refused at ingest MUST NOT be persisted, partially persisted, or
  acknowledged as delivered; the client MUST be able to distinguish this refusal from a
  capability refusal, because it is permanent and retrying unchanged cannot succeed.
  (amends Synchronization, namespaces and multi-device concurrency)
- **FR-582**: A personal or team record MUST carry its writer identity and writer sequence
  on the wire and in the server store, so a peer can detect a gap in a writer's stream.
  (amends Synchronization, namespaces and multi-device concurrency)
- **FR-583**: The writer sequence MUST NOT be consulted as an ordering key, a tiebreak, or
  a conflict-resolution input by any importer; it is diagnostic only. (amends
  Synchronization, namespaces and multi-device concurrency)
- **FR-584**: Level 0 reserve returned unspent to the general pool MUST remain unavailable
  to personal and team sections; global allowance is computed from the non-reserve pool
  only. (amends Unified bounded recall and domain separation)
- **FR-585**: An API token past its expiry MUST be refused, and the refusal MUST be
  indistinguishable to the caller from a revoked token, so expiry cannot be probed.
  (amends Identity, roles and account lifecycle)
- **FR-586**: A pre-004 client MUST continue to synchronize projects against a 004 server
  unchanged; only the removed account and self-join routes cease to function. (amends
  Compatibility, capability advertisement and migration)
- **FR-587**: A removed route MUST answer with a stable, documented status and a message
  naming its replacement, rather than a bare not-found, so an operator can diagnose it
  without reading release notes. (amends Compatibility, capability advertisement and
  migration)
- **FR-588**: The release MUST document, in operator-facing terms, that self-registration
  and self-join are gone, that accounts are now administrator-created, and what an
  operator must do for users who relied on either. (amends Compatibility, capability
  advertisement and migration)
- **FR-589**: The periodic check of FR-489 MUST occur on a bounded interval stated as a
  number, and that number MUST be the one a test asserts against; a namespace MUST NOT
  poll on the daemon's tick cadence, and "the documented background interval" MUST have a
  documented referent. (amends Synchronization, namespaces and multi-device concurrency)
- **FR-590**: Re-enabling a disabled account MUST NOT restore any token revoked while it
  was disabled; a re-enabled account MUST obtain fresh credentials through the ordinary
  token-minting route. (amends Identity, roles and account lifecycle)
- **FR-591**: A stored account identity MUST be invalidated when the credential it was
  learned from changes, and the daemon MUST fail closed — establishing no `personal:*` lane
  and attributing no knowledge to the previous account — until a live authenticated identity
  lookup succeeds. (amends Identity, roles and account lifecycle)
- **FR-592**: A `team:*` pull cursor MUST record the caller visibility context under which
  it was advanced, and MUST be discarded rather than advanced when the server reports a
  different context, so that a widening of the caller's view of the team feed cannot skip
  rows that view now includes. (amends Synchronization, namespaces and multi-device
  concurrency)

### Key Entities

- **KnowledgeDomain**: Which of project, personal or team a durable record belongs to.
  Orthogonal to memory scope; answers "whose knowledge is this" rather than "how narrow".
- **ApplicabilityKind**: The closed vocabulary a record's applicability facts are drawn
  from — language and tool — nothing else is representable. Distinct from a record's own
  `topic_key`, which is the knowledge's subject key inherited from Feature 003 and is not
  an applicability fact (FR-570).
- **ApplicabilityFact**: One `(kind, value)` pair naming a condition under which a
  personal or team record applies to a project. A record with none is universal.
- **PersonalKnowledge**: An immutable, single-user record with no project identity,
  optional applicability facts, and its own reconciliation and lifecycle, independent of
  any one project.
- **TeamKnowledge**: An immutable, server-wide record that begins proposed and becomes
  visible only once an admin ratifies it, with the same applicability model as personal
  knowledge.
- **TeamState**: The lifecycle of a team knowledge entry — proposed, authoritative, or
  retired — advanced only by an explicit, state-checked transition.
- **PromotionTarget**: The domain a promotion is aimed at — a reusable pattern, personal
  knowledge, or team knowledge — each subject to its own gate requirements.
- **PromotionRejection**: The named reason a promotion attempt failed, identifying which
  gate check stopped it without echoing the content that failed it.
- **GlobalContentRejection**: The named reason `validate_global_content` refused a
  personal or team record's free-text content or an applicability value, identifying only
  one of nine rejection classes, never the offending content. The validator takes an
  explicit `project_identities` input — the set of identity tokens to screen against — and
  an empty set passes the project-identifying check rather than failing it. Returned at all
  five entry points capable of creating global content — direct personal creation, personal
  promotion, team proposal, team promotion, and server-side synchronization ingest — not
  only at the promotion gate.
- **WriterIdentity**: A single opaque identity per local store, used to keep two devices'
  otherwise-identical writes from colliding, and to scope per-writer sequence numbers.
  Never a user-visible device name or registry.
- **ProjectTrait**: A fact about a project's stack, derived deterministically from its
  working tree and never synchronized, used only to evaluate applicability.
- **SyncNamespace**: One of the three independent synchronization lanes — project,
  personal, team — each with its own cursor, backoff and capability state.
- **ServerRole**: A user's server-level standing, admin or member, independent of project
  membership.
- **UserStatus**: Whether an account is active or disabled; disabling revokes every token
  the account holds.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-401**: Every account on a tested server traces to an administrator's creation
  action; zero accounts trace to a self-service path.
- **SC-402**: A temporary password is revealed exactly once, in the account creation
  response itself; no subsequent request, by any route or by any caller including the
  creating administrator, retrieves it, in 100% of trials.
- **SC-403**: An account requiring a password change succeeds at zero actions other than
  changing its password, across every tested route, in 100% of trials.
- **SC-404**: Disabling an account revokes 100% of that account's live API tokens at the
  instant of disabling, verified by testing a token issued immediately beforehand.
- **SC-405**: A server backfilled from an unrostered state ends with the documented
  administrator and never zero admins, across every seeded configuration.
- **SC-406**: Project lookup by shared identity returns zero projects the caller is not a
  member of, across a corpus mixing member and non-member projects.
- **SC-407**: Auto-link with no project specified selects correctly in the single-match
  case and prompts rather than guesses in every zero-match and multi-match case, across a
  seeded corpus of repositories.
- **SC-408**: A user removed from a project's membership loses read and sync access
  within one subsequent request, in 100% of trials.
- **SC-409**: A personal knowledge entry is retrievable from every other project of its
  owner and from none belonging to any other user, in 100% of seeded cases.
- **SC-410**: Applicability matching admits a record to a project in 100% of cases where
  its stated kinds are covered by that project's traits, and to zero cases where they are
  not.
- **SC-411**: Two disagreeing personal entries recorded offline on two devices for one
  user both survive sync in 100% of trials, independent of sync order or relative clock
  skew.
- **SC-412**: A device with nothing queued to push receives another device's synchronized
  personal or team knowledge within twice the documented pull interval — 60 seconds at the
  specified 30-second interval — without pushing anything first, in 100% of trials. The
  criterion names the number rather than deferring to "the documented interval", because a
  criterion whose bound is stated only by reference cannot be asserted if the reference is
  never resolved.
- **SC-413**: A proposed team entry is absent from every recall surface, including the
  proposer's own, in 100% of cases, until an admin ratifies it.
- **SC-414**: Ratification attempted by anyone other than an admin is refused in 100% of
  attempts.
- **SC-415**: Two concurrent ratification attempts against one proposal yield exactly one
  success and one refusal naming the entry's current state, in 100% of trials.
- **SC-416**: An authoritative team entry is visible to every account on its server
  regardless of project membership, and a retired one is visible to none, in 100% of
  cases.
- **SC-417**: Search returns project, personal and team results in three distinct arrays
  in 100% of responses, and a project result count is identical whether or not personal
  or team results exist.
- **SC-418**: Where project-priority context already consumes the entire requested
  budget, the assembled context is byte-identical whether or not personal or team
  knowledge exists, in 100% of trials.
- **SC-419**: Personal and team sections never occupy more than the documented share of
  the total budget, and estimated size never exceeds the requested budget, across every
  tested corpus.
- **SC-420**: A context request at minimum depth contains zero personal or team content,
  regardless of available budget, in 100% of trials.
- **SC-421**: Every promotion attempt in a seeded corpus covering **all nine** rejection
  classes — absolute paths, home-directory references, drive-letter paths, `file://`
  references, credentialed URLs, environment assignments, secret-shaped runs,
  project-identifying tokens and shell command invocations — is refused, names the class it
  failed on, and creates no partial record, in 100% of cases. The corpus MUST cover every
  class the validator declares, so a class added to the validator without a corresponding
  corpus entry leaves this criterion unmet rather than silently unverified.
- **SC-422**: The serialized form of a promoted record, inspected field by field on the
  wire and in both stores, contains no verification field; a test that adds one fails.
- **SC-423**: Forgetting or deleting a memory that was previously promoted leaves the
  promoted record unchanged, in 100% of cases.
- **SC-424**: A personal or team knowledge record has no field for a project identifier, an
  evidence reference, an observation identifier, or verification of any kind — not an
  authority, not a state, not a timestamp — verified by inspecting the stored and serialized
  forms field by field, so that adding such a field fails the test.
- **SC-424a**: Personal and team knowledge records built from a seeded adversarial corpus
  carry no project-identifying token, file path, or shell command in any free-text field or
  applicability value — verified by driving that corpus through all five entry points and
  asserting refusal, so the criterion tests the validator rather than the schema.
- **SC-425**: Against a synchronization peer that does not support personal or team
  knowledge, project synchronization continues at unchanged throughput while the other two
  namespaces report themselves blocked by name, in 100% of trials.
- **SC-426**: A personal or team item held back by an older peer is delivered exactly
  once, without user intervention, within one synchronization cycle after that peer is
  upgraded, in 100% of trials.
- **SC-427**: Two local stores producing byte-identical personal or team content
  independently never collide as a single write, in 100% of trials.
- **SC-428**: A local store that has recorded one server instance's identity refuses to
  merge a second instance's team knowledge into itself, in 100% of trials.
- **SC-429**: A project with no derivable traits admits only universal personal and team
  records and zero kind-restricted ones, in 100% of trials.
- **SC-430**: The agent tool surface remains exactly six tools after this feature ships,
  verified by test.
- **SC-431**: The transmitted handoff payload contains zero absolute local paths, across a
  corpus that includes paths with no repository-relative counterpart.
- **SC-432**: Migrating an existing local store and an existing server preserves every
  existing row unchanged, assigns every new field its documented default, and an
  interrupted migration leaves the store on its prior working schema version, verified by
  test.
- **SC-433**: A server whose administrator rows have been corrupted outside the supported
  API — by direct database mutation or by legacy state predating role assignment —
  recovers full administration on the next start through the environment-named account,
  with no database surgery and no reinstallation. This is a repair path for externally
  corrupted state, not the result of any operation the API permits.
- **SC-434**: An attempt to deactivate or demote the environment-named account is refused
  with a message that tells the operator which environment setting to change instead, and
  the account's role and status are observably unchanged afterwards.
- **SC-435**: A reader of the shipped documentation can state, without reading the source,
  who on a deployment is ultimately able to obtain administrator access and why.
- **SC-436**: A deactivated account is refused a fresh login using a password that remains
  otherwise correct, verified separately from the revocation of its existing tokens, so a
  regression in either one cannot be masked by the other.
- **SC-437**: An attempt to demote or deactivate the last remaining administrator that is
  not the environment-named account is refused at the moment it is made, verified at
  runtime rather than only at migration time.
- **SC-438**: Content naming a project — by project name, by a path component of its
  repository location, or by a remote host, organisation or repository token — and content
  carrying a shell command are each refused identically at every entry point, verified by
  exercising all five with the same inputs.
- **SC-439**: A rejection message, log line and API response contain no fragment of the
  rejected content, verified by asserting the offending substring appears nowhere in any
  output.
- **SC-440**: After a rejected creation or promotion, no record, partial record or outbox
  entry exists, verified by inspecting all three.
- **SC-441**: Two promotions from the same project on one machine share an origin digest;
  the same two promotions made on a second machine share a different one; and no origin
  digest appears in any transmitted payload, verified by inspecting the wire.
- **SC-442**: An administrator resets a member's password; the old password immediately
  fails, the new temporary password authenticates only to the password-change route, and
  every token the member held is refused.
- **SC-443**: Resetting a disabled account's password leaves it disabled, verified by
  attempting authentication with the new temporary password and being refused.
- **SC-444**: Two concurrent demotions of the two remaining administrators result in
  exactly one success and one refusal, verified against a real database under genuine
  concurrency rather than sequentially or by reasoning about isolation levels.
- **SC-445**: Personal and team content queued against a server that does not support it
  is held while project sync continues; after that peer is replaced by a supporting server
  at the same configured endpoint, and with no new local write and no restart, the held
  content delivers automatically and exactly once.
- **SC-447**: A local store linked in turn to two different server instances retains both
  identities' personal knowledge rather than refusing either, and recall in each linked
  context returns only that identity's entries — verified by asserting the other
  identity's entries are absent from search, context and listing.

### Repairs from design analysis

These criteria correspond to the new requirements in the "Repairs from design analysis"
subsection of Requirements, produced by the second `/speckit-analyze` repair pass, and
carry the same trailing parenthetical naming the subject section they amend.

- **SC-448**: An applicability value that would be refused as content is refused as an
  applicability value, verified with a project-identifying value such as an internal
  product name. (amends Privacy boundary and the promotion gate)
- **SC-449**: A client that pushes personal or team content containing a
  project-identifying token or a shell command — bypassing its own local validation — is
  refused by the server, the record is absent from the server store, and it never reaches
  the user's other devices. (amends Privacy boundary and the promotion gate)
- **SC-450**: A record created on one device and pulled by another inserts successfully
  with its writer identity and sequence intact, and a deliberately withheld middle record
  is reported as a detected gap rather than silently ignored. (amends Synchronization,
  namespaces and multi-device concurrency)
- **SC-451**: With a large unspent Level 0 reserve released to the general pool and global
  records available to fill it, the global sections consume none of the released reserve,
  verified by asserting global spend against the non-reserve pool alone. (amends Unified
  bounded recall and domain separation)
- **SC-452**: A request bearing a token past its expiry is refused, and its refusal is
  identical in status and body to that of a revoked token. (amends Identity, roles and
  account lifecycle)
- **SC-453**: No component other than the content validator implements any of its rejection
  classes, verified by an audit that fails when a second implementation is introduced —
  not by inspection of the current code, which would pass today and pass again after a
  duplicate is added. (amends Privacy boundary and the promotion gate)
- **SC-454**: Global content creation succeeds when no project identity is available to
  screen against, and is refused when a required input is structurally absent — the two
  cases are asserted separately, so an implementation that conflates a vacuous check with an
  unevaluable one fails one of them. (amends Privacy boundary and the promotion gate)
- **SC-455**: Reordering, withholding or renumbering a writer's sequence changes nothing
  about which records survive reconciliation or which is derived as canonical, verified by
  replaying one corpus under permuted sequences and asserting identical derived output; the
  reconciliation input carries no sequence field, so a tiebreak that consulted one would not
  compile. (amends Synchronization, namespaces and multi-device concurrency)
- **SC-456**: An ingest refusal and a capability refusal are distinguishable by the client
  without inspecting a message string, the refused item is never reported as delivered, and
  the refused namespace remains eligible rather than blocked and unthrottled — verified by
  asserting the namespace continues to push subsequent items at unchanged throughput.
  (amends Synchronization, namespaces and multi-device concurrency)
- **SC-457**: A client built before this feature completes a full project synchronization
  cycle against a server that has it — push, pull and cursor advance — with no namespace
  blocked and no throughput loss, verified against a real pre-004 client binary rather than
  a simulated one. (amends Compatibility, capability advertisement and migration)
- **SC-458**: A request to a removed route returns the documented status and a body naming
  its replacement, and the shipped release documentation states that self-registration and
  self-join are gone, that accounts are administrator-created, and what an operator must do
  for users who relied on either — verified by asserting the response body and by checking
  the documentation names all three. (amends Compatibility, capability advertisement and
  migration)
- **SC-459**: `MemoryScope` has exactly four variants and the `memories` table's scope
  `CHECK`, its stored representation, and its exhaustive resolution match are byte-identical
  before and after this feature — verified by asserting the variant list and the `CHECK`
  text, so that adding a fifth variant fails. (amends Backward compatibility and
  non-displacement)
- **SC-460**: No route on the agent tool surface creates a team knowledge entry in the
  authoritative state, and every entry the surface can create is in the proposed state —
  verified by exercising every action the six tools expose and asserting the resulting state,
  so a new action that could ratify fails the test rather than passing unnoticed. (amends
  Team global memory)
- **SC-461**: No sequence of recall, search, context assembly or synchronization creates a
  personal or team record that no explicit promotion or creation request asked for, verified
  across a seeded workload by asserting the global record count is unchanged. (amends
  Privacy boundary and the promotion gate)
- **SC-462**: Where personal and team items compete for the same remaining space and only one
  fits, the personal item is the one included, in 100% of trials. (amends Unified bounded
  recall and domain separation)
- **SC-463**: Selection reasons for personal and team items are present in the diagnostic
  output and absent from the rendered briefing, verified by inspecting the rendered form field
  by field, so that a reason reaching the briefing fails the test. (amends Unified bounded
  recall and domain separation)
- **SC-464**: An importance hint of every supported value on a personal or team item changes
  neither its section's precedence nor its admission into reserved context, verified by
  asserting the assembled context is byte-identical across all hint values. (amends Unified
  bounded recall and domain separation)
- **SC-465**: No authenticated route adds the caller to a project's membership, verified by
  exercising every route the server exposes and asserting the caller's membership set is
  unchanged — so a route added later that grants self-membership fails the test. (amends
  Secure project collaboration)
- **SC-466**: Two disagreeing authoritative team entries on one subject are both returned
  with the disagreement surfaced, and the order in which they were ratified changes nothing
  about which are returned, in 100% of trials. (amends Team global memory)
- **SC-467**: Every privacy guarantee in the shipped design documentation names which of the
  two mechanisms it rests on, and no free-text field is described as structurally incapable of
  carrying a path or a command — verified by an audit over the documentation that fails on the
  forbidden phrasing rather than by review. (amends Privacy boundary and the promotion gate)
- **SC-468**: A relevance score computed in one domain's index is never compared against
  another's, verified by construction: the ranking input for each domain is a distinct type
  carrying no other domain's score, so a cross-domain comparison would not compile. (amends
  Unified bounded recall and domain separation)
- **SC-469**: A project's derived traits appear in no transmitted payload and no server table,
  verified by inspecting the wire across a corpus of projects whose traits are all distinct,
  so that a trait becoming synchronized later fails the test. (amends Personal global memory)
- **SC-470**: A token revoked while its account was disabled is still refused after the
  account is re-enabled, verified with a token issued before the disable and a request made
  after the re-enable — asserted separately from the disable-time revocation, so a
  regression that clears `revoked_at` on re-enable cannot be masked by the disable test
  passing. (amends Identity, roles and account lifecycle)

## Assumptions

- **The security hardening patch has already landed.** The self-registration route, the
  open project-join route, the unfiltered project lookup, the unscoped tombstone and the
  top-level-only wire denylist are all closed on `main` before this feature begins; this
  feature builds on that clean ground rather than fixing those holes itself.
- **The constitution is amended to v1.1.0 before this feature ships.** Principle IV is
  refined from "memory is never global or ambient" to "never *ambient*": every durable
  record now carries an explicit, addressable domain, and a companion principle
  establishes that global knowledge must never displace project truth.
- **One server hosts one team.** Multiple organizations or nested teams sharing one
  server is not a case this feature has to handle.
- **A closed vocabulary of language and tool is enough applicability signal.**
  Cairn does not need a score, an embedding, or a model judgment to decide whether personal
  or team knowledge fits a project.
- **Manifest and lockfile presence is a good-enough proxy for a project's stack.** A
  project whose stack cannot be seen from its working tree simply receives only universal
  global knowledge; nothing tries to guess further.
- **API tokens remain the only per-device credential.** This feature does not introduce a
  device registry, a device identity a user manages, or any user-visible device list.
- **Administrators are trusted to ratify responsibly.** Ratification is not further gated
  by review, voting or quorum in this feature.
- **The server columns already sent by the client but written by nobody stay inert.**
  Wiring them is out of scope here to avoid unrelated scope creep, and this feature
  documents that choice rather than silently leaving it unexplained.
- **The outbox attempt ceiling and the deferred-record expiry remain unaddressed.** Both
  are pre-existing gaps this feature neither introduces nor fixes.
- **A member's proposal is a genuine attempt at team-wide truth, not spam.** This feature
  does not add proposal rate limits or moderation queues beyond the single ratify/retire
  decision an admin already makes.

## Out of Scope

- **Team and shared reusable-pattern synchronization** — deferred to **Feature 005**;
  reusable patterns stay local to one machine exactly as they were left before this
  feature.
- **Web UI administration and team-curation screens** — deferred to **Feature 005**; this
  feature ships administration as CLI and server endpoints only.
- **Organizations, multiple teams per server, or nested groups** — one server is one team
  throughout this feature.
- **Embeddings, vector stores, semantic recall, knowledge graphs, decay or confidence
  engines** — retrieval stays lexical and deterministic across every domain this feature
  adds.
- **A device registry or device subsystem** — API tokens remain the only per-device
  credential; a writer identity is not a device name or an inventory entry.
- **SSO, OAuth, multi-factor authentication or cross-server federation** — authentication
  stays password-and-token based, unchanged in kind.
- **A fifth memory scope, including any form of a global memory scope** — domain is
  orthogonal to scope, and scope itself is untouched by this feature.
- **Mapping the daemon's currently unmapped requests onto the agent tool surface** —
  criterion-verification actions, memory lookup by identifier or by subject, task history
  and derived-value rebuild are unrelated to what this feature adds.
- **An outbox attempt ceiling, or an expiry for deferred synchronization records** — both
  are pre-existing gaps noted here but not fixed by this feature.
- **Richer applicability that requires reading file content** — deferred to **Feature
  005** along with the rest of the applicability work; the vocabulary this feature ships
  (`language`, `tool`) is limited to what can be derived from files present in a working
  tree without inspecting their content.
