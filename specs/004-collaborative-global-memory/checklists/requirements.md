# Specification Quality Checklist: Cairn Collaborative Global Memory

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-08-21
**Feature**: [spec.md](../spec.md)

Each item below is a reviewable yes/no question about the quality of **spec.md** — never
about how the feature is or will be implemented. `[x]` means spec.md already satisfies the
item, with the citation showing where; `[ ]` means the item is genuinely open and names, in
parentheses, what a reader should go check before treating it as settled.

## Requirement Completeness

- [x] CHK001 Does every new domain (project, personal, team) have a stated create, recall,
      immutable-then-forget lifecycle, rather than leaving one domain's lifecycle to
      inference from another's? (FR-431–FR-441 personal; FR-451–FR-465 team)
- [x] CHK002 Is server-instance identity itself, not only its use in refusing a merge,
      covered by a requirement to establish it and make it discoverable? (FR-415, FR-416)
- [x] CHK003 Can an administrator view a project's full membership list, not only add or
      remove individual members one at a time? (FR-427)
- [x] CHK004 Is a requirement stated for what a team knowledge state transition must record
      for later inspection, not only that the transition itself succeeds or fails? (FR-457)
- [x] CHK005 Can a user search and retrieve their own personal knowledge independent of
      which project they are currently in, not only see it appear unprompted in context?
      (FR-444)
- [x] CHK006 Is there a requirement for what happens when an API token's stated expiry is
      reached — refusal, silent non-issue, or something else? (It was a gap. FR-585 now
      states the refusal and, more usefully, requires it to be *indistinguishable* from a
      revoked token's — identical status and identical body — so a stale token cannot be used
      to probe whether it was ever valid for this server; SC-452 verifies both halves. D453)
- [x] CHK007 Is there a requirement for what happens when a project's membership is removed
      down to zero users — is that a valid, unremarkable state, or should something be
      refused the way a zero-admin server is refused? (Deliberate, and the mechanism that makes
      it safe is already required. The two states differ in recoverability, not severity: a
      zero-admin server is unrecoverable through any supported API, which is why FR-413 enforces
      a floor atomically. A zero-member project is recoverable — FR-419 says "an existing member
      **or an admin**", and `project-authorization.md` §2 states the server-admin bypass on that
      route exists precisely so membership can be bootstrapped on a project with none left.
      D458)

## Requirement Clarity

- [x] CHK008 Does a requirement name the closed applicability vocabulary's kinds inline,
      rather than leaving them only to the Key Entities section? (Originally open against
      FR-434 alone, which still only points at "a closed, documented vocabulary" — but the
      repair's FR-569 now inlines the vocabulary directly: "it consists of `language` and
      `tool`." The vocabulary itself also shrank from three kinds to two: `topic` was
      removed because, unlike `language` and `tool`, it cannot be derived deterministically
      from files in a working tree — FR-569, D439)
- [x] CHK009 Is "the documented background interval" a promise the spec itself bounds with a
      number, or is the number left entirely to design? (It was left to nothing at all, and the
      adversarial pass found the consequence. No document stated an interval, and
      `sync-namespaces.md` §5 had *argued against* naming one — which, followed literally, makes
      the pull-due timer elapse on every 500 ms tick, so each of three namespaces polls twice a
      second indefinitely. FR-589 now requires a stated bound,
      `PULL_INTERVAL_SECONDS = 30` is it, and SC-412 asserts 60 seconds rather than deferring to
      a referent that did not exist. Same defect class as CHK010, and this one hid an unbounded
      poll behind it. D458)
- [x] CHK010 Is "a documented fraction of the total budget" (FR-474) resolved to a number
      anywhere a reader of spec.md alone would find it, or does understanding the actual cap
      require the design brief? (It was not an intentional spec/plan split — it was an
      unimplementable requirement, since two implementations could both claim conformance and
      no test could be written. D450 pins the fraction at `0.15`, and FR-584 supplies the
      second, independent bound: the allowance is `min(floor(total_budget * 0.15),
      remaining_non_reserve)`, so released Level 0 reserve is unavailable to global sections.
      SC-419 and SC-451 verify the two terms separately, which matters because they bind in
      different situations)
- [x] CHK011 Are "role" (admin/member, server-level) and "state" (proposed/authoritative/
      retired, per-entry) kept as clearly distinct terms throughout, never used
      interchangeably? (`ServerRole` vs. `TeamState` Key Entities; FR-402 vs. FR-453)
- [x] CHK012 Is "member" used consistently to mean project membership, distinct from an
      account's active/disabled status? (FR-419–FR-421 vs. FR-408–FR-410; no overlap found)
- [x] CHK013 Does FR-478 state what "name why it was selected" must actually contain for a
      personal or team item — e.g., "applies universally" vs. "matched trait X" — or does it
      only require that *some* explanation exists? (FR-478 now binds the vocabulary to the
      one Feature 003 already defines for project sections, rather than inventing a second.
      D451 additionally resolves the contradiction the original wording carried — reasons had
      to be reported *and* had to never appear in the rendered briefing — by making them
      explain-only and enforcing it by construction: the rendered-briefing type has no reason
      field, so a renderer cannot leak them by forgetting to omit them. Compare to the
      concrete explanation
      vocabulary Feature 003 already uses for project sections; confirm the same specificity
      is intended here rather than a vaguer placeholder)

## Authorization & Identity

- [x] CHK014 Does a requirement close every route by which a user could create their own
      account, rather than only removing the one route known to exist today? (FR-401 is
      phrased as "no unauthenticated or self-service path," not naming a single route)
- [x] CHK015 Does a requirement close every route by which a user could add themselves to a
      project's membership? (FR-418, phrased the same way as FR-401)
- [x] CHK016 Is exactly one role assigned to every account, with a deterministic rule for
      accounts that exist before roles do? (FR-402, FR-414)
- [x] CHK017 Is token revocation on disable stated as immediate, not merely eventual or
      "on next use"? (FR-409, SC-404 — "at the instant of disabling")
- [x] CHK018 Is the effect of re-enabling a previously disabled account on its earlier,
      revoked tokens stated anywhere — do they stay dead, or does re-enabling imply anything
      about them? (It was stated in exactly one place: a paragraph in
      `identity-administration.md` §6, plus that contract's invariant 4. No requirement and no
      criterion covered it, so a credential-lifetime rule held by the intention of whoever wrote
      the paragraph. An implementer clearing `revoked_at` alongside `status` resurrects every
      token the account held before it was disabled, and every existing test still passes,
      because they all assert the disable side. FR-590 and SC-470 close it, with SC-470
      asserting the re-enable case separately. D458)
- [x] CHK019 Is the password-change requirement stated broadly enough to block every
      authenticated action, not enumerated action-by-action in a way a new endpoint could
      slip past? (FR-407 — "every authenticated action other than the password change
      itself")
- [x] CHK020 Is the zero-admin protection stated as an explicit requirement, enforced
      atomically rather than by a count-then-update sequence two concurrent requests could
      both pass? (FR-413's replacement text names the atomicity directly; FR-574 requires
      every demotion or disable to serialize on one application-wide advisory lock held for
      the transaction's duration, replacing an earlier `FOR UPDATE`-CTE design whose
      correctness argument depended on Postgres EvalPlanQual re-evaluation semantics — a
      version-sensitive guarantee the repair deliberately stopped relying on, D445)

## Privacy Boundary

- [x] CHK021 Does any requirement introduce a memory scope, table, or column that crosses
      project boundaries the way `MemoryScope::Global` would have? (Reviewed FR-431–FR-465:
      personal and team both explicitly carry no project identity, FR-517 and FR-459; domain
      is stated as orthogonal to scope, not a fifth scope value)
- [x] CHK022 Is every path from project memory to personal or team knowledge required to be
      explicit and gated, with no requirement describing an automatic or implicit promotion?
      (FR-506, FR-507)
- [x] CHK023 Does any requirement permit personal or team content into the context
      assembler's reserved allocation under any condition? (FR-473 states the reserve
      "MUST remain project-only," with no stated exception)
- [x] CHK024 Is the agent tool surface explicitly barred from making a team entry
      authoritative, with ratification named as a CLI/server-only path? (FR-455)
- [x] CHK025 Do the repair requirements (handoff leak, wire-check denylist, vacuous scope
      test, stale contract docs) state the correction of behavior that already ships on
      `main`, rather than reading as requirements for new behavior only? (FR-531–FR-535,
      each phrased as "MUST be corrected"/"MUST NOT transmit ... including where one
      currently survives")
- [x] CHK026 Is the exclusion of project identifiers, evidence references, file paths and
      elevated verification from personal/team records stated as a structural incapacity
      ("MUST have no column for"), not merely a behavioral promise not to populate one? (Yes
      for these — FR-517 — but the original FR-517 also claimed this of `content`, which is
      free text and so the claim was false there; content is now governed separately by
      validation, not structural incapacity — FR-544 through FR-550, with FR-550 forbidding
      documentation from ever again describing a free-text field as structurally incapable)
- [x] CHK027 Is the interaction between "no applicability facts proposed at promotion" and
      "a record with no applicability facts is universal" made explicit for the promotion
      path specifically, or only stated for direct creation? (It carries, and no restatement is
      wanted. FR-435 and FR-460 are stated about the *record* — "a personal knowledge entry with
      no applicability facts MUST apply to every project" — not about the path that created it,
      and a promoted entry with no facts is an entry with no facts. FR-514 adds a
      promotion-specific obligation without displacing the record-level default. Restating it
      would create a second normative sentence about one obligation, which D455 exists to
      prevent. D458)
- [x] CHK028 Is a promotion refusal explicitly required not to echo the offending content
      back to the caller? (FR-510 — "MUST NOT echo the offending text")
- [x] CHK029 Is "a salted digest" (FR-516) specified precisely enough that a reviewer could
      judge whether two independent implementations satisfy the same non-reversibility
      property, or does the spec rely on the word "digest" alone? (Resolved: FR-516's
      replacement text scopes recognition to "the same machine"; FR-551 requires the digest
      never be transmitted, which is what makes it non-reversible — the server, which knows
      every project identity, never holds a copy to test against them; FR-552 states the
      resulting per-machine-only limitation explicitly rather than leaving it implicit)

## Recall Bounding & Non-displacement

- [x] CHK030 Is the guaranteed/reserved portion of an assembled context required to stay
      project-only under every tested condition, not only the common case? (FR-473, SC-418)
- [x] CHK031 Is personal and team content's share of the total budget bounded by a stated
      cap, applied to what remains after the reserve rather than to the whole budget? (FR-474)
- [x] CHK032 Is a minimum-depth request required to exclude personal and team content
      entirely, with no configuration able to override that? (FR-477, SC-420)
- [x] CHK033 Are project, personal and team search results required to stay in separate
      arrays, never merged into one list a caller must re-split? (FR-469, SC-417)
- [x] CHK034 Is cross-domain relevance comparison explicitly forbidden, so a future
      implementation cannot quietly rank one domain's BM25 score against another's? (FR-471)
- [x] CHK035 Does a requirement state that a caller with no personal or team knowledge of
      their own sees zero difference from a caller who never touches either domain — i.e.,
      is backward compatibility for the common case a stated requirement, not an assumption?
      (FR-481)
- [x] CHK036 Is an importance hint on a personal or team item explicitly forbidden from
      changing section precedence or admitting the item into reserved context, mirroring the
      existing project-memory invariant rather than silently exempting the new domains?
      (FR-482)

## Synchronization & Concurrency

- [x] CHK037 Does each new domain synchronize through its own namespace, independent of the
      project namespace and of each other, rather than sharing one queue with a discriminator
      column? (FR-486)
- [x] CHK038 Is a capability block or failure in one namespace explicitly required not to
      prevent another namespace from continuing, closing the door on a single shared
      failure/backoff state? (FR-488, FR-497)
- [x] CHK039 Is a namespace with nothing queued to push still required to periodically check
      for content produced elsewhere, closing the "quiet machine never learns anything"
      hazard the current background-pull logic has? (FR-489)
- [x] CHK040 Is a durable, opaque writer identity required to be folded into what makes a
      synchronized write unique, so two devices' byte-identical payloads are not mistaken for
      one write? (FR-490, FR-491)
- [x] CHK041 Is a per-writer sequence number explicitly forbidden from being compared across
      writers or used to break a tie between them? (FR-492)
- [x] CHK042 Is a team knowledge state transition required to use a compare against the
      request's expected prior state, refusing a mismatch rather than silently applying it —
      i.e., is last-write-wins explicitly excluded for this one mutable transition? (FR-454)
- [x] CHK043 Is a local store required to refuse merging team knowledge sourced from a
      different server instance than the one already recorded for it? (FR-495, FR-496)
- [x] CHK044 Does a requirement cover releasing claimed-but-unfinished synchronization work
      at daemon start across all three namespaces, not only the project namespace that has
      this today? (FR-502)

## Compatibility & Migration

- [x] CHK045 Is an old server required to continue serving project synchronization at full
      speed while the two new namespaces sit blocked, with the degradation named rather than
      silent? (FR-522)
- [x] CHK046 Is the compatibility mechanism required to remain the existing one-way
      advertisement, with no requirement describing a handshake or negotiation step? (FR-529)
- [x] CHK047 Is local-store migration required to preserve every existing row and assign a
      documented default to every new field, rather than leaving default values
      unspecified? (FR-523)
- [x] CHK048 Is an interrupted migration required to leave the store on its prior working
      schema version, rather than a partially upgraded and untested one? (FR-525)
- [x] CHK049 Is server-side role migration required to never produce a server with zero
      admins, using the same deterministic backfill rule stated for a fresh bootstrap?
      (FR-524, FR-414)
- [x] CHK050 Is the outbox's widened entity-type constraint required to be proven by an
      actual rebuild-through-migration-history test with row/byte equality, rather than
      asserted by description? (FR-530)
- [x] CHK051 Does any requirement address the reverse compatibility direction — an
      older client's `cairnd` talking to a newer, schema-3 server — or does the spec only
      cover an old server against a new client? (It was silently assumed safe, and it was not:
      the security prerequisite *removes* two routes, and removal is a compatibility event for
      every client that predates it. FR-586 requires project synchronization to continue
      unchanged, FR-587 requires a removed route to answer with a stable documented status and
      a message naming its replacement rather than a bare not-found, and FR-588 requires the
      operator-facing release documentation. D454)
- [x] CHK052 Are deferrals to Feature 005 named explicitly, with no requirement in this
      feature silently depending on a capability Feature 005 would add? (Out of Scope names
      team/shared pattern sync and Web UI administration screens as deferred; no FR
      references either as a prerequisite)

## Scope Discipline

- [x] CHK053 Is `MemoryScope::Global` explicitly named as out of scope, rather than merely
      absent from the requirements by omission? (Out of Scope)
- [x] CHK054 Is the agent tool surface required to remain exactly six tools after this
      feature ships, with new capability required to land as actions or fields on the
      existing six? (FR-527, SC-430)
- [x] CHK055 Does the spec avoid introducing a device registry, a device-visible name, or any
      device lifecycle beyond the stated, narrow purpose of writer identity? (Out of Scope;
      `WriterIdentity` Key Entity — "Never a user-visible device name or registry")
- [x] CHK056 Does the spec avoid introducing organizations, multiple teams per server, or
      nested groups? (Out of Scope; Assumptions — "One server hosts one team")
- [x] CHK057 Does the spec avoid making any requirement depend on an embedding, vector store,
      or model judgment for applicability matching or reconciliation? (Out of Scope;
      Assumptions — "Cairn does not need a score, an embedding, or a model judgment"; FR-437
      forbids inferring traits from content or asking a language model)
- [x] CHK058 Are Web UI administration and team-curation screens named explicitly as
      deferred, rather than left unaddressed as though the CLI/server surface were the whole
      story? (Out of Scope)
- [x] CHK059 Is the absence of proposal rate limiting or a moderation queue stated as a
      deliberate assumption, rather than left as a silent gap a reader might mistake for an
      oversight? (Assumptions — "does not add proposal rate limits or moderation queues
      beyond the single ratify/retire model")

## Traceability

- [x] CHK060 Does every functional requirement in the identity/roles/lifecycle block trace to
      at least one success criterion? (Spot-checked: FR-401→SC-401, FR-403→SC-402,
      FR-409→SC-404, FR-413/FR-414→SC-405)
- [x] CHK061 Does every functional requirement in the privacy/promotion-gate block trace to
      at least one success criterion? (Spot-checked: FR-508–FR-512→SC-421, FR-513→SC-422,
      FR-519→SC-423, FR-517→SC-424)
- [x] CHK062 Does every success criterion trace back to at least one functional
      requirement, and every functional requirement forward to at least one success
      criterion? (Built as [traceability.md](../traceability.md). The mapping is
      complete for SC->FR; on the FR->artifact side it records two requirements with
      no owning design document, both now assigned tasks in tasks.md. Not a clean
      bill of health — read its findings section.)
- [x] CHK063 Does functional-requirement numbering stay within the block ranges the design
      brief allocated (FR-401–417, 418–430, 431–450, 451–468, 469–485, 486–505, 506–520,
      521–530, 531–538), with no requirement renumbered out of its block? (Verified against
      spec.md's section headings and id ranges)
- [x] CHK064 Does each repair requirement (FR-531–FR-535) trace to a specific, named,
      pre-existing defect rather than a hypothetical one? (Matches the verified ground-truth
      findings this feature builds on: the handoff path leak, the mis-documented wire
      denylist, the vacuous scope-bucket test, and the stale privacy-sync contract)

## Repair Coverage (D433–D445)

These items were added after the `REPAIR ADDENDUM` (decisions D433–D445) landed in spec.md,
covering the specific corrections the addendum made rather than re-running the categories
above end to end.

- [x] CHK065 Does the specification separate personal/team privacy into a structural
      guarantee (no column exists) and a validated guarantee (free-text content is checked
      by a shared validator), rather than describing content as structurally incapable of
      carrying a path? (FR-517 now scopes structural incapacity to genuine columns only —
      project identifier, evidence reference, observation identifier, file path, command,
      elevated verification; FR-544 and FR-550 require free-text content to be validated
      instead, and forbid documentation from ever describing a free-text field as
      structurally incapable)
- [x] CHK066 Is the content validator required to run at every entry point capable of
      creating global content, rather than only at the promotion gate, which is the bypass
      the design analysis found open at the other three? (FR-544, FR-545; SC-438 verifies
      all **five** entry points refuse the identical input identically. The count was four
      when this item was first written and all four were client-side, which the audit round
      identified as a boundary that held only while the client cooperated — server-side
      synchronization ingest is the fifth, screening against the union of the pushing user's
      project memberships, D447, FR-577, SC-449)
- [x] CHK067 Is the origin digest required to stay local and never be transmitted to the
      server, closing the reversibility "a salted digest" left unstated? (FR-551 — the
      server, which knows every project identity, must never hold a digest to test them
      against; FR-552 documents the resulting per-machine-only recognition limit as a
      deliberate, accepted tradeoff; SC-441 verifies no digest appears in any transmitted
      payload)
- [x] CHK068 Is an administrator password reset required to leave a disabled account
      disabled — never re-enabling it as a side effect of issuing a new credential? (FR-558;
      SC-443 verifies authentication with the freshly reset temporary password is still
      refused)
- [x] CHK069 Is the never-zero-admins guarantee required to be enforced by one serialized
      mechanism — a single application-wide lock held for the transaction's duration —
      rather than left to a check-then-update sequence two concurrent requests could both
      pass? (FR-413's atomicity clause and FR-574's advisory-lock requirement; FR-560 states
      the required outcome directly; SC-444 requires verification under genuine
      concurrency against a real database, not by reasoning about isolation levels)
- [x] CHK070 Is the transition a blocked namespace takes back to eligible itself specified —
      a bounded, backed-off capability probe, not a retry of the held items — closing the
      gap between "held items are never retried" (FR-499) and "they deliver after an
      upgrade" (FR-500) that previously left the trigger for delivery undefined? (FR-561
      through FR-563; SC-445 verifies the release happens with no local write, no user
      command and no restart)
- [x] CHK071 Does a requirement distinguish personal knowledge, which must never be refused
      on the basis of server instance, from team knowledge, which must be refused when it
      arrives from a different server instance — rather than binding both to the same
      instance check? (FR-496 team; FR-567 personal, requiring partitioning by owning
      identity instead of refusal; FR-568 keys the personal sync namespace by instance and
      account together; SC-428 verifies the team-side refusal)
- [x] CHK072 Is the applicability vocabulary limited to kinds a client can derive
      deterministically from files in a working tree, with no kind — like the removed
      `topic` — that could never be matched and would silently make a record inapplicable
      everywhere? (FR-569 closes the vocabulary to `language` and `tool`; FR-570 keeps a
      record's own `topic_key` explicitly distinct from an applicability fact, so the two
      senses of "topic" this feature used are not conflated)
- [x] CHK073 Does every MUST NOT requirement this repair adds trace to a success criterion
      that would actually catch a violation, the way FR-547→SC-439 and FR-551→SC-441 do — or
      does at least one, such as FR-567's requirement that personal knowledge is never
      refused on server-instance grounds, have no success criterion devoted to it at all?
      (Its own example is answered: SC-447 verifies the personal-partitions-instead-of-refuses
      guarantee directly, asserting that a store linked in turn to two instances retains both
      identities' personal knowledge and that each context returns only its own. The general
      question this item asked was **not** answered by the repair round, and running it
      properly — D457 — found six requirements with no criterion at all, now SC-453–SC-458.
      Closing this item is therefore contingent on CHK083, which restates it against the
      requirements this round added rather than the last one)

## Audit Coverage (D446–D457)

These items were added after the independent audit. Unlike the sections above, several of them
exist because a previously `[x]` item was certified against a **citation** rather than against a
mechanism — that failure mode is itself the subject of CHK082.

- [x] CHK074 Does a requirement name every rejection class the content validator must apply,
      including the two the third-pass repair omitted? (FR-546 now lists nine: absolute path,
      home-directory reference, drive-letter path, `file://` reference, credentialed URL,
      environment-variable assignment, encoded-secret shape, project-identifying token, and
      shell command invocation. The last two previously existed only as promotion-gate check 4,
      which two of the entry points never called. D446)
- [x] CHK075 Is the validator required to be the *only* implementation of those classes, rather
      than merely to exist? (FR-579. Without it, the repair for a bypass is a second copy of the
      rule, and two copies of one privacy rule drift)
- [x] CHK076 Is the behavior of the project-identifying check when no project identity is
      available stated as a requirement, and distinguished from a check that cannot be
      evaluated? (FR-580 states that an empty identity set **passes** — a check with nothing to
      match is vacuous, not unevaluable — and that a genuinely unevaluable check still fails
      closed per FR-549. This is the single named exception to fail-closed, and it is deliberate:
      implementing it fail-closed would refuse every global creation made outside a linked
      project, which is the normal case for cross-project personal knowledge. D446)
- [x] CHK077 Are applicability *values* covered by a requirement, or only their kinds? (FR-578
      and SC-448. The closed `language | tool` vocabulary constrains a fact's kind; its value was
      an unchecked open string, and `tool = "acme-internal-deploy"` names a project as surely as
      any sentence does)
- [x] CHK078 Does a requirement state what validates content arriving at the server from a
      client that did not validate it? (FR-545 makes server-side ingest the fifth entry point and
      FR-577 states the refusal; SC-449 verifies that a client bypassing its own validation is
      refused, the record is absent from the server store, and it never reaches the user's other
      devices. Before this, a privacy guarantee held only when the client cooperated. D447)
- [x] CHK079 Is an ingest refusal required to be distinguishable from a capability refusal?
      (FR-581. Their retry semantics are opposite — a capability refusal becomes retryable after a
      server upgrade, an ingest refusal never can — so conflating them makes one bad record
      throttle a namespace forever)
- [x] CHK080 Do the writer identity and writer sequence have a stated transmission status
      consistent with the invariant they serve? (FR-582 puts both on the wire and in the server
      store; FR-583 confines the sequence to diagnostics. The prior design classified both as
      never-transmitted while declaring them `NOT NULL` locally, which no pulled record could
      satisfy — and gap detection is meaningful only to a peer, so local-only defeated the
      purpose. SC-450 verifies both the insert and the gap report. D448)
- [x] CHK081 Is the absence of a verification field stated as absolute, rather than as an
      absence of authority *above* a value? (FR-513 and FR-517 now say no verification field of
      any kind — not an authority, not a state, not a timestamp. "No authority above `attested`"
      still admitted a field holding `attested`, which is a place for one project's deterministic
      check to become a project-independent claim one migration later. SC-422 asserts the stored
      *and* serialized forms in both stores. D452)
- [x] CHK083 Does every requirement — the ones this round added *and* the ones that predate
      it — trace to a success criterion that would catch a violation? (It did not. D457 ran the
      pass in two waves and the second found more than the first: six of the seventeen gaps were
      sentences this very audit round wrote, and eleven had survived three prior passes.
      FR-579, FR-580, FR-583, FR-586, FR-587, FR-588 and FR-581's distinguishability half now
      trace to SC-453–SC-458; FR-521, FR-455/FR-515, FR-506, FR-476, FR-478, FR-482, FR-418,
      FR-462, FR-550, FR-471 and FR-438 to SC-459–SC-469. Eight requirements are recorded there
      as deliberately carrying no dedicated criterion — FR-415, FR-428, FR-441, FR-461, FR-465,
      FR-520, FR-542, FR-559 — so the claim "every requirement has one" is not made where it is
      not true)
- [x] CHK085 Was the feature's own central constraint verified, or only stated? (It was only
      stated. FR-521 — no change to `MemoryScope` or its stored representation, the "do not add
      `MemoryScope::Global`" constraint that plan.md's Summary calls the one decision everything
      turns on — had a task and no success criterion through three passes. SC-459 now asserts the
      four-variant list and the `CHECK` text so a fifth variant fails. The general lesson is
      recorded in D457: a task is not a criterion, because a task can be reworded, deferred, or
      marked done against a different assertion than the one intended)
- [x] CHK084 Where a criterion enumerates a corpus, is the enumeration bound to the
      requirement it verifies rather than fixed independently of it? (SC-421 listed six
      rejection classes while FR-546 declared nine, and the same task cited both, so every
      cross-reference resolved and three classes had no corpus entry. SC-421 now requires the
      corpus to cover every class the validator declares, so adding a class without a corpus
      entry leaves the criterion unmet rather than silently unverified. D457)
- [x] CHK086 Do the artifacts describe the design that was actually implemented, or the one that
      was written down? (Six divergences surfaced during Phase 2 and are folded back in
      `traceability.md` §8a. Five were wording or SQL-dialect defects. One — `sync-namespaces.md`
      §6's twelve outbox entity types against `data-model.md`'s ten — was a **missing invariant**:
      both relations tables exist on the server, a server table is reachable only through the
      outbox, and a relation belongs to neither of the rows it names, so it cannot travel inside a
      parent's payload. Without the two relation entity types, FR-493's "disagreement is expressed
      as relations" would have held locally and nowhere else. The lesson is the inverse of CHK082's:
      a disagreement between two artifacts is worth reading as a question about the design, not
      only as a typo to normalize toward the majority)
- [x] CHK087 Where an artifact stated a behavioural rule loosely, did implementation confirm the
      rule or expose it as a heuristic? (Exposed one. `command_shaped` was implemented as "a
      command name followed by a flag or a path" — a reading the contract's prose permitted — and
      it admitted `cargo test`, `rm target`, `sudo reboot`, `npm install` and `git status`. The
      rule is now grammatical position, stated in `promotion-privacy.md` §2a with the five
      admitted commands named, so the next reader sees why the obvious rule was rejected rather
      than only what replaced it)
- [ ] CHK082 Does this checklist's own `[x]` marks rest on a named mechanism a reader can point
      at, or on a citation that a requirement exists? (Open deliberately, and it is the audit's
      most general finding. The third-pass sweep verified citation coverage — every requirement
      named a task, every task named a requirement — and certified a design in which the
      validator's class list omitted the very check the requirement depended on. Three artifacts
      agreed with each other and all three were wrong together. Four previously certified items
      were re-opened for this reason (D456: the never-transmitted table, the Layer A/B split, the
      entry-point count, and the budget invariant). This item stays open as a standing instruction
      to the next reviewer, not as a defect in spec.md)

## Notes

- **Status**: 87 items, 86 satisfied (`[x]`), **0 open** apart from CHK082, which is open by
  design — see below. CHK086 and CHK087 were added by the Phase 2 artifact-sync pass.
- **spec.md as evaluated**: 160 functional requirements, 70 success criteria. The criteria grew
  from 52 to 70 during this round rather than shrinking, because running the semantic pass
  properly (D457) found seventeen requirements with no criterion at all, and the adversarial pass
  behind it (D458) found two more.
- **The last four open items were closed by inspecting them, not by aging out.** CHK007 and
  CHK027 confirmed as deliberate; CHK009 and CHK018 turned out to be defects — an acceptance
  criterion with no referent concealing an unbounded poll, and a credential-lifetime rule that
  existed only as a contract paragraph. Both had carried a parenthetical reading "confirm this is
  deliberate" through three review passes. An open item with a reassuring parenthetical is
  indistinguishable from a confirmed one, which is why the disposition rather than the mark is
  what a later reviewer needs.
- Closed by the audit round (D446–D457): CHK073 by SC-447 plus its restatement as CHK083,
  CHK006 by FR-585/SC-452, CHK010 by D450's `0.15`
  plus FR-584, CHK013 by FR-478's bound vocabulary plus D451's explain-only resolution, and
  CHK051 by FR-586–FR-588. Three of those four had been characterised here as an intentional
  spec/plan split or an out-of-scope case; none of them was. A "worth a one-line
  confirmation" parenthetical is exactly where an unimplementable requirement hides, and
  CHK010 is the clearest example — the number was missing, not deferred.
- Items still marked `[ ]` are open questions for the spec author or reviewer, not defects in
  the feature design. CHK008 and CHK029 were in this same open state before the `REPAIR
  ADDENDUM`; both are now resolved (`[x]`) by FR-569 and by FR-516's replacement text plus
  FR-551/FR-552, respectively, and are no longer counted among the open items.
- CHK062 is now satisfied by traceability.md, which itself records ten open findings
  rather than a yes/no judgment call a reader can resolve by reading spec.md alone.
  traceability.md has not been regenerated against the repaired spec.md as of this update
  (D443 requires that regeneration); until it is, CHK062's citation of traceability.md
  should be re-verified against the file that exists on disk, not assumed current.
- CHK020, CHK026 and CHK029 were re-examined against the `REPAIR ADDENDUM` (D433, D434,
  D436, D445) and their citations updated in place; none changed checked state to open, but
  their parentheticals now describe the repaired requirement text, not the original.
- CHK065–CHK073 were added after the addendum landed, covering its repair decisions
  directly rather than re-deriving them from the categories above. CHK074–CHK082 were added
  after the independent audit, covering D446–D457 the same way; CHK083–CHK085 were added last,
  by the semantic pass reviewing this checklist's own additions and then the whole requirement
  set behind it.
- CHK082 is open by design rather than by omission: it asks whether this checklist's own
  `[x]` marks rest on a mechanism or on a citation, which is the question the third-pass
  sweep answered wrongly for the whole artifact set. It should stay open, and it is excluded
  from the open count above for that reason — it is a standing instruction to the next
  reviewer, not an unresolved question about spec.md.
- CHK073 was closed only after its general question was actually run rather than answered from
  its own parenthetical. Its example (FR-567) was already covered by SC-447; the question it
  was really asking was not, and running it produced six new criteria. That sequence is worth
  noting: an item whose citation checks out can still be the item that finds the most.
- This checklist evaluates spec.md as delivered for Feature 004, including the `REPAIR
  ADDENDUM` (D433–D445) and the audit round (D446–D457) folded into it. It does not evaluate
  `plan.md`, `tasks.md`, `research.md`, `traceability.md`, or any contract as their own
  artifacts.
