# Specification Quality Checklist: Server-Authoritative Autonomous Memory

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-08-29
**Feature**: [spec.md](../spec.md)

Each item below is a reviewable yes/no question about the quality of **spec.md** — never
about how the feature is or will be implemented. `[x]` means spec.md already satisfies the
item, with the citation showing where; `[ ]` means the item is genuinely open and names, in
parentheses, what a reader should go check before treating it as settled.

## Content Quality

- [x] CHK001 Is the specification free of implementation detail — no language, framework,
      function or SQL names — except where an architectural constraint is itself the product
      contract? (Authority, privacy boundary, ingest boundary and migration guarantees are
      stated as observable behaviour; no Rust item or table name appears in a requirement.)
- [x] CHK002 Is every requirement written as user-observable behaviour rather than internal
      structure? (FR-701–FR-905 are phrased as MUST-statements about what the system does,
      not how it is built.)
- [x] CHK003 Are the mandatory template sections all present and in order? (User Scenarios &
      Testing, Requirements, Key Entities, Success Criteria, Assumptions — plus the house
      conventions Clarifications and Out of Scope.)
- [x] CHK004 Does the spec avoid a Constitution Check section, which house convention places
      in plan.md rather than spec.md? (Confirmed: `## Constitution Check` appears in all four
      prior `plan.md` files and in no prior `spec.md`.)

## Requirement Completeness

- [x] CHK005 Is server-authoritative durable knowledge stated as a requirement rather than
      implied? (FR-701, FR-702, FR-712)
- [x] CHK006 Is SQLite's demotion to a disposable edge role stated, including what it is still
      responsible for? (FR-702, FR-705, FR-709, FR-710)
- [x] CHK007 Is the local-deletion durability invariant testable as written, including its
      exceptions? (FR-703, FR-704, FR-706, SC-713, SC-714)
- [x] CHK008 Is Feature 004 data migration specified with explicit safety guarantees rather
      than as a configuration change? (FR-861–FR-878, SC-719–SC-723)
- [x] CHK009 Is rich vendor-specific capture required, with the two-field allowlist explicitly
      superseded? (FR-717, FR-719, FR-720)
- [x] CHK010 Does the canonical safe semantic event model enumerate the signals it must be able
      to express? (FR-734)
- [x] CHK011 Is the content-free restriction on non-tool events explicitly lifted? (FR-735)
- [x] CHK012 Is the local privacy and redaction boundary specified as deterministic and
      fail-closed? (FR-755, FR-756, FR-757, FR-764)
- [x] CHK013 Is "no raw transcript persistence by default" stated? (FR-750, FR-751, FR-754,
      SC-730)
- [x] CHK014 Is the safe-event server ingest specified as strongly typed, authenticated,
      authorized, idempotent, bounded and versioned? (FR-765, FR-767, FR-768, FR-770, FR-773,
      FR-774)
- [x] CHK015 Is the existing synchronization boundary's refusal list explicitly preserved
      rather than relaxed? (FR-766, SC-731)
- [x] CHK016 Is automatic consolidation specified without requiring an explicit tool call?
      (FR-793, SC-701)
- [x] CHK017 Is candidate reconciliation specified against the existing machinery rather than a
      new one? (FR-798, FR-799, FR-804, FR-823)
- [x] CHK018 Is verification specified as derived rather than assertable by consolidation?
      (FR-811)
- [x] CHK019 Are the project, personal and team domains preserved with their existing
      semantics? (FR-817–FR-826)
- [x] CHK020 Is project-truth precedence stated and non-displaceable? (FR-821, SC-710)
- [x] CHK021 Is explicit `cairn_remember` retained, with its changed role stated? (FR-815,
      FR-815a, FR-816, FR-831)
- [x] CHK022 Is automatic retrieval required without a tool call? (FR-827, SC-708)
- [x] CHK023 Is context bounding preserved and protected from inflation? (FR-829, FR-830,
      SC-709)
- [x] CHK024 Is retrieval traceability specified in enough detail to reconstruct a selection?
      (FR-839–FR-841, FR-848, SC-729)
- [x] CHK025 Is delivery telemetry specified so it cannot overclaim? (FR-842–FR-844, FR-850,
      SC-712)
- [x] CHK026 Is integration health required to distinguish configuration from runtime capture?
      (FR-851–FR-853, FR-859, SC-724, SC-725)
- [x] CHK027 Is temporary server outage behaviour specified, including what may degrade?
      (FR-781–FR-789, SC-715)
- [x] CHK028 Is retry and replay idempotency stated for events, consolidation and migration?
      (FR-770, FR-786, FR-797, FR-868, SC-716, SC-721)
- [x] CHK029 Is the web dashboard and control plane specified rather than deferred? (FR-879–
      FR-895)
- [x] CHK030 Is memory detail with full provenance specified? (FR-883, FR-884, FR-885)
- [x] CHK031 Are relations exposed, and is the relation graph bounded and non-authoritative?
      (FR-901–FR-905)
- [x] CHK032 Are non-goals stated explicitly with a reason each? (Out of Scope, seventeen
      entries, four recording alternatives considered and rejected on 2026-08-30.)
- [x] CHK033 Is the end-to-end acceptance scenario defined on a real repository, per Principle
      VII? (End-to-End Acceptance Scenario, five phases)
- [x] CHK034 Does the spec close, or explicitly re-defer, each item Feature 004 deferred to
      Feature 005? (Pattern synchronization closed by FR-708/FR-708a/SC-738; web administration
      and team curation closed by FR-889/FR-890; file-content applicability re-deferred with a
      stated reason in Out of Scope.)

## Requirement Clarity

- [x] CHK035 Is every requirement singular — one obligation per identifier — so a test can fail
      it precisely?
- [x] CHK036 Are requirement identifiers unique and non-colliding with Features 001–004?
      (Verified mechanically after each pass; currently 272 FR and 63 SC identifiers, all within
      the FR-7xx/FR-9xx and SC-7xx bands this feature reserved, with zero intersection with the
      identifiers used by the four prior features.)
- [x] CHK037 Is the reason for departing from the leading-digit numbering habit recorded, so a
      later reader does not read it as an error? (Front matter; research.md §7)
- [x] CHK038 Are deliberate identifier gaps between semantic blocks noted? (Front matter)
- [x] CHK039 Is every MUST genuinely a MUST rather than a preference? (FR-901 is deliberately
      MAY and now states that FR-902–FR-905 bind only if the graph is built and that nothing
      else depends on it.)

## Privacy Boundary

- [x] CHK040 Is the separation between semantic extraction and privacy enforcement stated as a
      requirement, not an aspiration? (FR-758, FR-759)
- [x] CHK041 Is the refusal record structurally prevented from carrying refused content?
      (FR-757, SC-705)
- [x] CHK042 Is the single-implementation rule for rejection classes preserved? (FR-760)
- [x] CHK043 Is raw material held locally required not to outlive the work that reads it?
      (FR-763 — discarded once parsing, redaction and the privacy checks complete or fail, never
      written to durable local storage, and never retained pending a later extraction step.)
- [x] CHK044 Is server-side independent enforcement required, so a hostile client cannot store
      forbidden content? (FR-777)
- [x] CHK045 Is it settled whether, and under what name, repository-relative file identity may
      cross the safe-event boundary? (Settled 2026-08-30: the field is `repo_file`, carrying
      repository-relative identity only, with the refused name `path` not reused and the server
      validating independently. FR-777b–FR-777e, SC-743, SC-744.)
- [x] CHK046 Is it settled where semantic extraction runs and what a model may see? (Settled
      2026-08-30: extraction runs on the server over already-approved safe events, and any
      model — hosted included — sees only material Cairn was already permitted to persist
      centrally. The previously specified local transient extraction boundary was removed rather
      than refined. FR-749, FR-763–FR-763b, FR-805a–FR-805c, SC-741, SC-742.)

## Authorization & Identity

- [x] CHK047 Is authorization identity required to come from the authenticated caller rather
      than the payload, preserving Feature 004's hardening? (FR-769)
- [x] CHK048 Is consolidation forbidden from crossing personal ownership? (FR-810)
- [x] CHK049 Is consolidation forbidden from ratifying team guidance? (FR-809, SC-734)
- [x] CHK050 Is credential switching handled for spooled events? (FR-790)
- [x] CHK051 Is server instance binding preserved across the authority change and the
      migration? (FR-791, FR-875)
- [x] CHK052 Are retrieval and web views required to enforce the same authorization as the API?
      (FR-834, FR-892)

## Autonomy Under Governance

- [x] CHK053 Is it stated that a model may propose but not decide durability, domain or
      supersession? (FR-805)
- [x] CHK054 Is automatic supersession forbidden? (FR-800, SC-735)
- [x] CHK055 Is the narrowing of the "reinforcement is never automatic" rule stated explicitly
      rather than left implicit? (FR-801a — this is a deliberate change to an existing contract
      and is flagged as such.)
- [x] CHK056 Is autonomously produced knowledge distinguishable from human-asserted knowledge?
      (FR-816, FR-885)
- [x] CHK057 Is consolidation prevented from attributing work to a project by guesswork?
      (FR-812)

## Scope Discipline

- [x] CHK058 Is the feature specified as one product feature with one acceptance story, rather
      than split? (Single spec, single End-to-End Acceptance Scenario.)
- [x] CHK059 Is speculative infrastructure excluded with a reason rather than by silence? (Out
      of Scope; FR-711, FR-904, SC-737)
- [x] CHK060 Does the spec avoid preserving Feature 004 machinery merely because it was
      expensive to build? (research.md §6 classifies each mechanism KEEP / SIMPLIFY /
      REPURPOSE / REMOVE with the reason it exists; FR-874 and FR-876 encode the two
      conclusions that bear on requirements.)
- [x] CHK061 Are stale motivating assumptions corrected rather than converted into
      requirements? (research.md §1.7 identifies three claims that no longer hold on `main`;
      none appears as a requirement.)

## Success Criteria

- [x] CHK062 Are all success criteria measurable? (SC-701–SC-751 state counts, percentages or
      absolute zeros. SC-739–SC-751 were rewritten after the consistency pass found several that
      could not fail — see CHK087.)
- [x] CHK063 Are success criteria free of technology names? (No framework, language, datastore
      or library is named in any SC.)
- [x] CHK064 Does at least one success criterion exist for each major requirement area?
- [x] CHK065 Are the "100% of trials" criteria accompanied by a defined trial population?
      (Measurable Outcomes now defines the population once — at least ten independent trials per
      supported agent with the capability under test, on a real repository — and states that an
      absolute-zero criterion is failed by a single counterexample.)

## Traceability

- [x] CHK066 Is every factual claim about current behaviour cited to code at a named commit?
      (research.md, throughout, at `f76a9fe`.)
- [x] CHK067 Is the baseline commit recorded in the spec itself, not only in research? (Front
      matter)
- [x] CHK068 Are the constitutional conflicts this feature creates resolved in the constitution
      rather than in an implementation note? (Constitution is currently **v1.2.1**. The 1.2.0
      entry amended Principles III, V, II, VI and added IX, X and XI for the original
      architecture. The 1.2.1 entry closes the gap the 2026-08-30 decisions opened: Principle V
      gained the extractor boundary, and Principle II was clarified on what "a new service"
      forbids. Both entries are current governance; neither is superseded.)

## Falsification Coverage

An independent adversarial review was run against this specification with a mandate to break
it, checking every claim against source at `f76a9fe`. It returned findings in five classes;
this section records what was found and what was done, because a checklist that does not
survive a hostile read is decoration.

- [x] CHK069 Were internal contradictions found and resolved rather than argued away? (Nine
      found. Patterns required to be durable while the only boundary that could carry them
      refuses them — FR-708b and a narrowed SC-731. Extraction with no local process eligible to
      host it — since resolved by moving extraction server-side, not by the local transient
      boundary that first pass specified. Reinforcement required from a candidate that has no persisted endpoint —
      FR-798a. A retrieval deadline that destroyed the determinism guarantee — FR-835/FR-836
      declared levels. Traces required to persist rendered context that mixes domains —
      FR-839/FR-886 now record identities and accounting. Migration per store versus authority
      per server — FR-876. A "by default" hedge that made an absolute prohibition unfalsifiable
      — FR-750. A durability claim that concealed a live server dependency — SC-713/FR-710a.
      Content-derived event identity that would discard genuinely repeated acts — FR-738.)
- [x] CHK070 Were constitutional conflicts found beyond those already amended, and closed?
      (Six. Principle II's "graphs" and "a cache tier"; Principle V's "local paths" and "data
      leaves the machine only when the user has chosen to share it"; Principle VI's determinism
      sentence; Principle IV's provenance target. All are now amended in v1.2.0 with the reason
      recorded, and the rule that authorization identity is never read from a payload — relied
      on throughout and stated nowhere — became Principle XI.)
- [x] CHK071 Were requirements found that were not grounded in current `main`, and corrected?
      (Six. FR-838 described post-compaction delivery as an existing behaviour to continue when
      no such automatic delivery exists for any vendor. FR-853 targeted a defect already fixed
      and would have passed vacuously; it now names the real one. FR-806 and an assumption
      asserted server-side full-text search that covers only project memory. FR-712a records
      that the local merge rule cannot apply a server-side correction, so repurposing the
      replicas as a cache is a behaviour change rather than a reinterpretation. FR-730 now marks
      itself as preservation. Most consequentially, the claim that repository-relative paths
      cannot cross the boundary was **false** — handoffs already carry `changed_files` — which
      reframed an open question from a prohibition into a field-naming decision.)
- [x] CHK072 Were authorization gaps found in the new surfaces, and closed? (Eight, each a
      class Feature 004 had already paid to close, reappearing on a new key: an unverified
      session identifier in an event body (FR-769a); consolidation-authored team proposals with
      no truthful proposer (FR-809a); retrieval traces with no stated readership, enumerating
      another account's personal knowledge (FR-846a); cached briefings not bound to the
      credential that assembled them (FR-790a); a migration drain bypassing the per-author claim
      filter (FR-864a); web curation preserving authorization but not the compare-and-swap that
      prevents double ratification (FR-889a); new project-scoped reads with no membership guard
      (FR-894a); and a safe-event schema free to declare the very field names the other boundary
      refuses (FR-777a).)
- [x] CHK073 Were untestable or vacuous requirements found and made falsifiable? (Eleven.
      Self-selecting agent populations in SC-701, SC-706, SC-708 and SC-712 now test against a
      pre-declared list, so a failing agent is a failure rather than a reclassification. SC-711
      now requires an explanation a reviewer can reproduce the selection from. SC-727 separates
      the automatable assertion from the human demonstration. SC-736 fixes its corpus at fifty
      pairs. SC-737 became a manifest comparison rather than a claim about source text. FR-724
      gained an observable test. FR-903, FR-882 and FR-890 had undefined bounds or undefined
      escape terms — "low-value", "where appropriate", "bounded" with no number.)
- [x] CHK074 Was any review finding rejected, with a reason? (One. The review reported the
      constitution as amended without a version bump or history entry — a governance violation.
      It read the file mid-edit: the amendment history and the 1.2.0 footer were written
      afterwards, and the file is now internally consistent. No other finding was rejected.)

## Architectural Decisions (Session 2026-08-30)

- [x] CHK075 Is consolidation's execution home decided, and is its progress durable? (In-process
      background work inside the existing server process; PostgreSQL holds backlog, progress and
      claim state so a restart loses no completed work; an abandoned claim is reclaimed and
      re-executed, and re-execution produces no duplicate durable effect. FR-793a–FR-793d,
      FR-797, SC-739.)
- [x] CHK076 Is capture independent of consolidation availability and backlog? (FR-814 now
      forbids back-pressure on ingestion and forbids reporting a backlog as an ingestion
      failure. SC-740.)
- [x] CHK077 Is the local machine's responsibility bounded to parsing, normalization, redaction,
      privacy checks and safe-event construction? (FR-749, with the section preamble stating it
      and FR-763 forbidding retention pending a later extraction step.)
- [x] CHK078 Is it stated that extraction receives only already-approved material, with no side
      channel to the machine? (FR-763b, FR-805a, SC-741.)
- [x] CHK079 Is the model's permitted output enumerated, and its forbidden decisions enumerated?
      (FR-805b: may propose content, kind, topic key, value key, source event references; may not
      decide durability, authorization, domain ownership, privacy acceptance, verification or
      supersession. SC-742.)
- [x] CHK080 Are model-proposed source event references verified rather than trusted? (FR-805c —
      existence, project and session context, and prior acceptance.)
- [x] CHK081 Is `repo_file` fully constrained and independently validated by the server?
      (FR-777c lists every rejected form; FR-777d requires server-side validation; SC-743 tests
      it adversarially on POSIX and Windows shapes.)
- [x] CHK082 Are all four file-identity dispositions distinguished — absolute-inside-repo,
      outside-repo, vendor-absent, and a hostile absolute value on the wire? (FR-777e local
      relativization; FR-777f out-of-repository; FR-777g unavailable-from-vendor; FR-777d
      server-side refusal. SC-743, SC-744.)
- [x] CHK083 Is model-proposed identity normalized by Cairn before use, deterministically and
      without embeddings? (FR-796a–FR-796c, SC-745. A failing key refuses the candidate rather
      than being repaired, because repair changes what the candidate collides with.)
- [x] CHK084 Is the post-cutover behaviour of a legacy client explicit, and is its data safe?
      (FR-876b's `upgrade_required` is distinguishable from a generic error and from a capability
      deferral; FR-876c leaves the local store untouched; SC-746, SC-747.)
- [x] CHK085 Is convergence-machinery retirement no longer contingent on a dormant device?
      (FR-876e supersedes the earlier every-store-migrated condition and records why.)
- [x] CHK086 Is server-instance binding preserved across cutover? (FR-875, which now covers both
      migration and cutover — a refused client can still establish it is bound to the same
      instance, so an upgrade prompt cannot be induced by repointing a client. A separate
      cutover-binding requirement was folded into FR-875 rather than left as a second identifier
      for one obligation.)

- [x] CHK087 Did a second adversarial pass run against the six decisions, and were its findings
      resolved? (Yes. Fourteen introduced contradictions, two required constitutional changes,
      eleven stale citations and nine weak success criteria. The most consequential: `repo_file`
      had no disposition for a vendor supplying an *absolute* path — which is the majority case,
      Claude Code included — so the common path was neither expressible nor recordable as
      unavailable (FR-777e–FR-777g). Also: only `path` had been renamed while `summary`,
      `command`, `details`, `exit_code` and `outcome` remain refused names (FR-777a1); key
      identity was asserted server-side while the existing dedup digest is a refused field
      (FR-796d); existing records' un-normalized keys would silently stop colliding (FR-867a);
      "a restart repeats none of it" contradicted the mandated reclaim (FR-793b); and retiring
      convergence at cutover would have removed machinery post-cutover migrations still need
      (FR-876e).)
- [x] CHK088 Are the new success criteria falsifiable? (SC-741 now requires an adversarial
      corpus that attempts the ingress rather than asserting a type system prevents it; SC-742
      now stubs the extractor with adversarial output rather than asking whether model influence
      was recorded, which nothing records; SC-747 compares record-level content rather than file
      bytes, which WAL churn makes meaningless; SC-743 requires the length bound to be a stated
      number. SC-748–SC-751 cover requirements that previously had no criterion.)
- [x] CHK089 Were duplicate obligations introduced by the decisions removed? (A second
      server-instance-binding requirement was subsumed into FR-875, and its identifier retired
      rather than left dangling; FR-793c and FR-813 were separated so each states one
      obligation.)

## Consistency Repair (Session 2026-08-30, second pass)

- [x] CHK090 Was the "already left the machine, therefore no new egress" reasoning removed
      wherever it was asserted? (Yes. It survived in one clarification answer and is withdrawn
      there explicitly, naming it as the derivation-as-loophole argument Constitution v1.2.1
      Principle V refuses. The two remaining occurrences are in the constitution itself, where
      they *refute* the argument. FR-805d now makes the naming and disclosure duty testable, and
      FR-805e forbids assuming a provider's retention behaviour.)
- [x] CHK091 Is a hosted extractor's compliance treated as something to establish rather than
      assume? (FR-805e; and an Assumption records the Phase 0 verification list — provider,
      model, endpoint, retention, training use, zero-retention eligibility, caching, isolation,
      required disclosure, and behaviour when a compliant mode is unavailable — with the
      instruction that the plan must report a blocker rather than record unverified compliance.
      No provider is selected by this specification.)
- [x] CHK092 Is extraction replaceable, so no requirement depends on a hosted extractor
      existing? (FR-805f.)
- [x] CHK093 Is a capture-deadline miss both agent-invisible and Cairn-visible? (FR-749b keeps
      the hook successful and non-blocking; FR-749c requires a distinct disposition surfaced in
      health and counters; FR-749d forbids the record carrying payload content. SC-752 fails
      either way — on an agent-facing error, or on a drop that health cannot see. This is the
      distinction Principle X exists for: fail-soft describes the agent's experience, not what
      Cairn is permitted to know about itself.)
- [x] CHK094 Do the edge cases agree with FR-777e/f/g? (Yes. The single edge case that said an
      absolute path is server-refused is replaced by four, matching the four dispositions:
      absolute-inside-repo is relativized locally and crosses; outside-repo carries an
      out-of-repository disposition; vendor-absent is unavailable-from-vendor; and an absolute
      value arriving on the wire is refused by the server's own validation.)
- [x] CHK095 Is all "restart repeats none" language gone? (Yes, from spec, research and CHK075.
      The surviving phrase is in CHK087's record of the contradiction, which is history. The
      operative wording is now: no completed work lost, abandoned claims reclaimed and
      re-executed, re-execution producing no duplicate durable effect.)
- [x] CHK096 Is delivery stated at effect level rather than as "accepted exactly once"? (Yes —
      however many times delivery is retried, at most one canonical event and one consolidation
      input exist, and a `duplicate` answer is success.)
- [x] CHK097 Is the supported-agent population fixed before implementation, with evidence?
      (FR-838a–FR-838f and research.md §9. Claude Code, Codex CLI and OpenCode for capture;
      Claude Code and Codex CLI for automatic delivery. Vendor documentation checked
      2026-08-30.)
- [x] CHK098 Is OpenCode's exclusion from delivery stated as **Cairn's decision** rather than
      silently dropped or blamed on the vendor? (FR-838b and SC-708 both say `declined_by_cairn`
      and both say why: OpenCode 2 does expose the hooks, and Cairn declines to depend on a beta
      surface. An OpenCode *capture* failure still fails SC-701 and SC-706.)
- [x] CHK099 Are the delivery points per agent unambiguous? (FR-838, FR-838c, FR-838d. Both
      committed agents expose a prompt-time hook and a session-start `compact` source, so
      post-compaction delivery is established for both — reached through a compaction-opened
      session, never by returning context from the post-compaction event, which one vendor
      documents as impossible.)
- [x] CHK101 Is OpenCode's delivery exclusion classified truthfully? (FR-838b — reported as
      **declined by Cairn** with the reason, not as unsupported by the vendor. OpenCode 2's
      hooks exist; they are beta. Calling that a vendor absence would be false, and the capture
      matrix distinguishes the two.)
- [x] CHK102 Is receipt acknowledgement stated as no-evidence rather than proven-absent?
      (FR-838e, SC-712 — status is `unavailable / no evidence`; zero agents report
      unsupported-by-vendor, which the evidence does not license.)
- [x] CHK103 Can a server-side verification summary exist without raw evidence egress or
      client-asserted state? (FR-811a–FR-811d — raw evidence and runs stay local; the server
      derives state from an attested run report bound to its account and project, never accepts
      `verified` as a claim, carries no observed values or locators, and holds knowledge as
      unverified where a summary cannot be established.)
- [x] CHK100 Is the git branch reconciled against the spec metadata? (The working branch is
      `feature-005-spec`; the feature directory is `005-server-authoritative-autonomous-memory`.
      Both are now recorded, with the reason they differ — this repository's feature script
      creates no branch.)

## Final Consistency Repair (Session 2026-08-30, third pass)

- [x] CHK104 Do reusable patterns have a complete server lifecycle? (`shared_patterns` is the
      redefined safe shape — the local representation stays refused and is not sent. Promotion,
      authorship binding, retrieval keeping its existing general-pool budget treatment, cache, deletion and
      migration are all defined; pattern *applications* stay local. FR-708/708a/708b, SC-738,
      `contracts/knowledge-commands.md` §3.3.)
- [x] CHK105 Is the semantic mapping a stated deterministic algorithm rather than an intention?
      (`contracts/extraction.md` §13.7 — redact, classify from a closed versioned lexicon,
      candidate tokens, intersect with the vocabulary, assign roles by fixed rank with two
      tiebreaks, decline unless complete. No model, no free text.)
- [x] CHK106 Is declining defined, and is it counted? (§13.8 — six named conditions, each
      recording `no_safe_semantic_mapping`. Declining is the correct outcome: a claim Cairn
      cannot ground in its own event stream is one it could not explain later.)
- [x] CHK107 Can SC-701 pass on a structural memory alone? (No — SC-701a requires 14 of 20
      pre-registered decision/instruction scenarios to produce a matching `decision` or
      `convention` record, and explicitly fails a run whose records are all structural.
      SC-701b tests that no prompt word crosses unless independently in the vocabulary.)
- [x] CHK108 Is the "reasoning is not learned" limitation stated rather than discovered later?
      (§13.9, and it is a consequence of the privacy contract: reasoning is prose, and prose
      does not cross this boundary.)
- [x] CHK109 Does a partial consolidation batch re-elect its session immediately? (Yes —
      `contracts/consolidation.md` §4 uses a single `CASE` statement. The earlier `NOT EXISTS`
      guard left the session `claimed` until lease expiry, stalling a large session for five
      minutes per batch.)
- [x] CHK110 Are `attempts` and the five-attempt rule located and defined? (§4.1 — per event on
      `consolidation_work`, incremented in the claim transaction so a worker that dies still
      counts its attempt, and a failed event does not block its session from closing.)
- [x] CHK111 Is the claim that project memory was already server-authoritative removed?
      (Yes. `contracts/migration-cutover.md` §3.1 now refuses `memory` and `memory_relation`
      alongside personal and team, and records the audit that disproved the claim: the upsert's
      conflict predicate is scoped to the project, not the author.)
- [x] CHK112 Do the two refusal lists agree? (`migration-cutover.md` §3.1 and
      `knowledge-commands.md` §2 are the same list, and each says so.)
- [x] CHK113 Are reads preserved after cutover? (§11.9 — the refusal is write-shaped. A demoted
      cache with no read path could never refill, which would contradict FR-704.)
- [x] CHK114 Is `KnowledgeRef` applied beyond traces and dedup? (`knowledge_candidates` result
      references, verification reports, the entity-relationship diagram, and the `pattern`
      domain are all keyed by it.)
- [x] CHK115 Can a sessionless command be represented without inventing a session? (Yes —
      `command_spool.session_id` is nullable and identity is scoped: `scope_kind` ∈
      {session, store} with its own durable counter. Shipped code already represents a
      sessionless CLI act as the nil UUID; a synthetic session would leave the second active
      session in the worktree that its own comment warns against.)
- [x] CHK116 Is legacy `last_verified_at` handled honestly? (Cleared on the record — a timestamp
      beside `unverified` asserts a run the server cannot substantiate — and moved to
      `legacy_verification_audit`, labelled untrusted, never read by a derivation.)
- [x] CHK117 Do personal, team and pattern records have somewhere to hold a summary? (Yes —
      `knowledge_verification`, keyed by the `ref_kind`/id discriminator for either
      `KnowledgeRef` or `PatternRef`; nullable `domain` is excluded from the PK and constrained
      by CHECK. Those tables do not gain the project columns; one derivation, two locations.)
- [x] CHK118 Is the FR-798b contradiction resolved? (FR-798c withdraws the source-event clause
      and explains why it was self-defeating: the event set is not stable across a reclaim, so
      an identity including it produces the duplicate the requirement existed to prevent.
      Source events remain recorded as evidence.)
- [x] CHK119 Is OpenCode's exclusion stated as Cairn's decision in the criterion itself?
      (SC-708 — `declined_by_cairn`, never a vendor limitation; OpenCode 2 does expose the
      hooks.)

## Pre-Task Repair (Session 2026-08-30, fourth pass)

- [x] CHK120 Are patterns kept out of the domain vocabulary without becoming domain-less?
      (FR-708c/FR-819. `KnowledgeRef.domain` remains project/personal/team; the canonical
      `shared_patterns` row has `domain = personal`, while `PatternRef(pattern_id)` remains a
      distinct reference shape. Constitution IV is not amended.)
- [x] CHK121 Is a relation given its correct reference shape? (`RelationRef(from, to, kind)` —
      `memory_relations` has no surrogate key and is not given one. Used by drain and possession.)
- [x] CHK122 Are server-backed patterns least-privilege? (FR-708d/FR-708e — owner-scoped by
      default; widening goes through team propose-and-ratify, never through pattern visibility.
      Storing centrally is durability, not publication. SC-761.)
- [x] CHK123 Is pattern promotion idempotent with a privacy-safe identity? (FR-708f —
      `pattern_id` from owner plus a digest of normalized problem/root cause/approach, all
      fields that already cross. The local `signal_digest + root_cause_digest` identity uses two
      refused names and could not travel. SC-760.)
- [x] CHK124 Is pattern trust prevented from being asserted? (FR-708g — the server stores only
      `sanitized`, the one level it can establish. `validated`/`contested` derive from
      local-only applications and stay local, labelled machine-local. SC-762.)
- [x] CHK125 Does five attempts mean five that ran, with success winning on the last one? (The
      close transaction first marks `$consolidated` ids `done`, then marks only still-pending
      rows with `attempts >= 5` failed, then releases/reopens/closes the lease. The truth table
      explicitly verifies: failures 1–4 retry; attempt-5 success is done; attempt-5 failure is
      failed; attempt 6 cannot be selected; a crash after start consumes an attempt; failed rows
      cannot strand the session.)
- [x] CHK126 Is there one authoritative migration flow? (Phase 2 in §4 now names every drained
      record type with its reference shape; §12.0 points at it rather than restating it.)
- [x] CHK127 Can retained-local name every retained record type? (`ref_kind ∈
      knowledge|pattern|relation`, with a CHECK that the right columns are populated for each.)
- [x] CHK128 Is verification provenance non-assertable? (Every client HTTP result is
      `remote_attested`; choosing `/api/verification/runs` cannot produce `remote_cairn`.
      `cairn` is server-executed only, `remote_cairn` has no baseline producer, and a payload
      naming authority is refused rather than ignored.)
- [x] CHK129 Is `prefer` classification deterministic? (Step 4a keys the event kind on the
      **source role** — which vendor field the material came from — replacing an undefined
      "grammatical person" test. A marker with no counterpart for the chosen kind declines.)
- [x] CHK130 Are the vendor source fields recorded exactly? (§13.10 — `UserPromptSubmit.prompt`
      and `Stop`/`SubagentStop.last_assistant_message` for Claude Code and Codex CLI, verified
      against vendor documentation on 2026-08-30. `StopFailure.last_assistant_message` and
      `MessageDisplay.delta` are explicitly excluded: the first carries an API error string, the
      second a partial stream.)
- [x] CHK131 Is OpenCode's semantic-signal decline recorded rather than silent? (FR-727e and the
      capture matrix — v2 beta exposes `event.prompt.text` plus pre-dispatch `system`, `messages`
      and `tools`; Cairn has not established a stable dedicated settled-assistant-message
      completion boundary. Structural capture is unaffected, and SC-701a's population is
      stated.)
- [x] CHK132 Is the legacy verification demotion product-authorized? (FR-811e–FR-811g and
      SC-763, so it is no longer plan-only mechanism: unsubstantiated server state demotes,
      client-earned state is untouched, old values survive only as untrusted audit metadata, and
      the demoted count is reported.)

## Final Pre-Task Semantic Repair (Session 2026-08-30)

- [x] CHK133 Does every canonical pattern name its domain while retaining `PatternRef`?
      (`shared_patterns.domain = personal`; type is `pattern`; reference-domain NULL means only
      `PatternRef`, not a domain-less record. Verified independently in spec, plan, data model,
      retrieval, verification, web, migration and command contracts.)
- [x] CHK134 Is legacy pattern ownership established before delivery rather than inferred from
      active credentials? (`legacy_pattern_claims` persists local id, owner, content key and
      pattern id; same-owner retry is idempotent; other-owner re-claim is refused; credential
      switch cannot re-key; unclaimed rows are retained and reported. FR-867b, SC-764.)
- [x] CHK135 Are all polymorphic references structurally constrained? (Candidate results,
      retrieval trace items, delivered context, verification reports and verification summaries
      each carry a CHECK: knowledge iff domain non-null; pattern iff domain null. SC-766.)
- [x] CHK136 Are duplicated verification-summary keys identical? (`data-model.md` and
      `verification-summary.md` both generate the same complete `reference_key`, use it as the
      primary key, and carry the same structural CHECK. Same-UUID personal, team and pattern
      summaries coexist.)
- [x] CHK137 Are retrieval examples and invariants ref-kind-aware? (Dedup storage, worked
      session example, trace item shape, authorization, web rendering and invariant text all use
      `KnowledgeRef(domain,id)` or `PatternRef(pattern_id)`; storage identity uses their generated
      complete `reference_key` rather than `ref_kind` plus UUID.)
- [x] CHK138 Does verification authority report only what authentication proves? (Both client
      routes produce `remote_attested`; generic bearer auth proves account only; `cairn` is
      server-executed, and `remote_cairn` awaits a separately specified stronger evidence path.
      FR-811h, SC-765.)
- [x] CHK139 Does official OpenCode evidence support the exact wording? (Current v2 docs mark
      plugin APIs beta, expose `event.prompt.text`, and expose `system`, `messages`, `tools`
      immediately before model dispatch. Baseline remains `declined_by_cairn` because no stable
      dedicated settled-assistant-message completion boundary was established.)
- [x] CHK140 Does every database identity preserve the full logical reference?
      (`retrieval_trace_items`, `delivered_context`, `knowledge_verification` and
      `verification_reports` use generated `reference_key`; knowledge includes domain, pattern
      has its own prefix. SC-767's identical-UUID matrix is independently exercised.)
- [x] CHK141 Does verification report idempotency include reporting identity? (Natural key is
      `(reference_key, account_id, verifier_kind, run_at)`: same-account retry deduplicates;
      distinct authenticated accounts do not collapse. Authority remains server-assigned.
      FR-811i.)

## Notes

- No item is incomplete. All 141 checks pass against the specification as it now stands.
- CHK045 and CHK046 tracked two of the three `[NEEDS CLARIFICATION]` markers and were closed on
  2026-08-30 along with the third. The specification now carries no clarification markers, and no
  open product question remains.
- CHK039 and CHK065 were wording and rigour defects found by this checklist's first pass and
  were closed in the same pass; they are recorded here because the checklist is a record of what
  was actually checked, not only of what remained wrong.
- The specification grew from 193 requirements at first draft, to 206 after falsification, to
  259 after the architectural decisions of 2026-08-30 and the consistency passes that followed.
  Essentially none of the growth is product scope: it is authorization, testability, and the
  consequences of the six decisions. No open product question remains.
- `/speckit-analyze` is not applicable at this stage: it performs cross-artifact analysis
  across spec.md, plan.md and tasks.md. plan.md now exists; tasks.md does not, by design, so the
  cross-artifact analysis in this repair pass stood in for it.
