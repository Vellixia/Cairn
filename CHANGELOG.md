# Changelog

All notable changes to Cairn are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Cairn is pre-1.0. Until 1.0.0, minor versions may change behaviour, storage
schemas, and the wire protocol without a deprecation period.

## [Unreleased]

### Fixed

- **Five authorization holes in the sync and link paths.** Self-registration and
  self-join are gone; project discovery is scoped to the caller's memberships;
  tombstones and sync upserts carry a `project_id` predicate, so one project's
  records can no longer overwrite or delete another's. An operator now creates
  accounts with `cairn-server users add` rather than a public route.

### Added — Feature 004, collaborative global memory (in progress)

Two knowledge domains that follow the *person* rather than the project:
**personal** memory, private to one account and synchronized across that
account's devices, and **team** memory, proposed by any member and made
authoritative only by an administrator.

Cairn gains no `MemoryScope::Global`. Scope answers "how narrow inside a
project"; a new orthogonal **domain** answers "whose knowledge is this", and the
two never meet — which is what leaves the `memories` table, its four-variant
`CHECK` and every Feature 003 reconciliation semantic untouched.

Landed so far (Phase 2, the foundation):

- `KnowledgeDomain`, `ApplicabilityKind`, `TeamState`, `PromotionTarget`,
  `ServerRole`, `UserStatus`, `ApplicabilityFact`, `ProjectTrait`,
  `SyncNamespace`, `WriterIdentity`
- `PersonalKnowledge` and `TeamKnowledge`, neither of which has a field for a
  project identifier, an evidence reference, an observation identifier, or
  verification of any kind
- the applicability match predicate — AND across kinds, OR within a kind, no
  facts means universal
- `validate_global_content`, the one implementation of nine content rejection
  classes, run at every path that can create global knowledge
- the eight-check promotion gate, which delegates content screening to that
  validator rather than repeating it
- the salted, machine-local origin digest, which never crosses the wire
- both migrations: local `0007`, server `0003`

## [0.1.0-alpha.5] — 2026-08-21

Project intelligence. Cairn stops being a place memories are kept and starts
being a thing that knows what this project currently believes, what stands
behind each belief, and when the world has moved out from under one.

Nothing here needs a model, an embedding, a vector store or a graph database,
and none was added. Every answer is a deterministic function of recorded state,
which is what lets two machines that have never met agree.

### Added

- **Canonical project knowledge.** A durable fact can carry a `topic_key` and a
  `value_key` — what it is about, and what it asserts. Cairn derives the current
  answer for a subject on every read, from the proposals and the recorded
  decisions between them. Nothing is overwritten and no row is "the truth": a
  memory a session wrote is still exactly what that session wrote.
- **Reconciliation that refuses to guess.** The one case Cairn merges
  automatically is content identical after normalization. Agreeing on a value
  and differing in words is reported as *corroborating*, with the member it
  agrees with named and the one call that would collapse them — because only
  the agent can read both and say whether they are one claim.
- **Conflicts are shown, never resolved.** Two applicable answers that disagree
  produce a conflict warning naming both, with no winner picked by a clock, an
  identifier or an arrival order.
- **Evidence and verification.** A memory can carry evidence facts — a file, a
  configuration key, a Git ref, a command outcome — and Cairn re-checks them
  itself. A check Cairn ran and a claim an agent attested are recorded as
  different things and rendered differently, everywhere.
- **Drift as a state.** When the evidence behind a verified memory moves, the
  memory becomes `drifted`. It is never rewritten and never deleted: what a
  session recorded stays what it recorded, and the disagreement with the world
  is what gets reported.
- **Minimum-safe context.** A reserved share of every briefing is held for the
  work state and the warnings a session cannot safely proceed without, so a
  tight budget drops history rather than the thing you needed.
- **Compression-safe continuity.** A checkpoint records what was assumed —
  branch, commit, task state, relevant paths — and validates all of it before
  restoring. A checkpoint whose assumptions no longer hold reports its
  divergences and does **not** hand back a next action that no longer applies.
- **Evidence-aware tasks.** Acceptance criteria become stably identified records
  with their own state, verification and evidence. Readiness is derived and
  never sets a task's status by itself; a criterion is never `verified` on an
  agent's own attestation.
- **Multi-device convergence.** Proposals, decisions, criteria and blockers
  merge between machines with no clock deciding anything. Reversing two
  machines' clocks produces a byte-identical result — asserted, per case, by a
  corpus in which every scenario has a clock-reversed twin.
- **Mixed-version recovery.** Work an older server cannot hold is retained as
  `blocked`, retried zero times, never marked failed, and delivered exactly once
  after the server is upgraded — with no manual repair and no user action.
- **Reusable cross-project patterns.** A verified, evidence-backed solution can
  become a sanitized pattern with no project identity, offered in another
  project and always labelled unverified *there*. Ten applications in one
  project count once, an agent agreeing with Cairn's own suggestion is not
  confirmation, and a counterexample contests a pattern without deleting it or
  reducing what it has done elsewhere.
- **`cairn pattern`**, `cairn evidence`, `cairn verify`, `cairn memory subject`,
  `cairn memory reconcile`, `cairn memory pin`, `cairn task criterion`,
  `cairn task blocker` and `cairn doctor --rebuild-derived`.
- **`cairn status` reports the mechanism's reach**: the share of project memory
  carrying a subject, the conflicted, needs-recheck and drifted counts, and any
  sync degradation — so nobody has to run an evaluation to find out whether any
  of this is being used.

### Changed

- **The MCP surface is still exactly six tools.** Every new capability is an
  action on a tool that already exists. A Feature 001 call carrying only its
  original arguments gets the same answer it always did, plus new read-only
  fields — replayed against a corpus recorded before this feature existed.
- **`GET /api/version`** additionally reports `schema_version` and a
  `capabilities` array, so a peer can tell what a server can hold. A server that
  predates the fields answers without them, and that silence is the answer.
- **The always-on agent contract** gained four obligations: give a durable fact
  a subject specific enough to state the whole claim, attach evidence rather
  than asserting importance, reinforce a corroborating member when it is the
  same claim, and record a pattern's outcome including a negative one.

### Fixed

- **OpenCode no longer reports `automatic` continuity.** Its pre-compaction
  warning depends on the installed build exposing an experimental hook, so on a
  build without it Cairn was never told compaction was coming — while the agent
  had been told not to worry. It reports `agent_initiated`, which is the honest
  answer.

### Migration

Additive. Migration 5 adds columns and tables and rewrites no existing value.
`topic_key`, `value_key` and `content_norm_digest` are left NULL on existing
memories: inferring a subject from content is the one thing this design
refuses to do, and it refuses it at the migration too. Existing memories stay
free-form, searchable, briefable and syncable exactly as before.

One documented approximation: `superseded_at` for supersessions that happened
before this release is taken from `updated_at`. See
`specs/003-project-intelligence/migration.md` §Step 2(b).

### Continuity, verified against live agents

`continuity_mode` is derived from two separate capabilities rather than one.
Capturing a compaction boundary and delivering context after one are different
facts, and treating them as a single capability meant any agent that merely
reported a compaction claimed re-delivery too. `automatic` now additionally
requires that Cairn has **observed** a delivery on this installation, so a wrong
entry in the vendor table can only ever under-promise.

Post-compaction restoration happens at the next session open -- the first
boundary the model reads -- detected from Cairn's own records rather than a vendor
string, and the restored checkpoint is now rendered to an agent that is delivered
to, not only to one that asks.

Driven against real compactions in all three agents. Claude Code and Codex report
`automatic`; OpenCode reports `agent_initiated`, which is the truthful answer
because it never re-opens a session, and telling the agent to ask always works.
A generic MCP client still degrades to `unavailable_automatic`.

Codex's hook trust is read from Codex's own `config.toml`, where Codex writes it.
It was previously read from `hooks.json`, where it never appears, so an approved
trust was invisible and the continuity mode stayed at its most conservative value
permanently with no action available that could change it.

### Known limitations

- OpenCode cannot reach `automatic`: it publishes no post-compaction session
  open, and its compaction hook only biases the summarising model rather than
  placing text in the compacted context. A deterministic mechanism exists and is
  tracked in [#49](https://github.com/Vellixia/Cairn/issues/49).
- OpenCode's pre-compaction capability is reported `conditional` on a probe that
  can never be satisfied, so the stated condition is not actionable
  ([#50](https://github.com/Vellixia/Cairn/issues/50)). It understates a real
  capability and cannot cause an over-claim.
- Topic keys do not converge across agents. Value keys do. Recorded, with the
  measurements, in `evals/topic-key-effectiveness/`.

## [0.1.0-alpha.4] — 2026-08-13

The agent integration platform. Claude Code, Codex and OpenCode integrate
natively, any MCP-compatible client integrates through the protocol, CC Switch
can distribute Cairn, and each of them reports what it can actually do rather
than what its vendor documents.

### Added

- **Native integration for Codex and OpenCode**, alongside Claude Code. One
  canonical seven-event lifecycle sits behind all three, so no vendor event
  name, payload shape or ordering assumption reaches the daemon.
- **Generic MCP onboarding.** Any MCP-compatible client can connect over the
  protocol without a bespoke adapter.
- **The Cairn Skill and the rendered agent usage contract.** Both carry their
  own content-addressed revisions, independent of the package version.
- **`cairn agents`**, plus `connect`, `doctor`, `repair`, `disconnect` and
  `integration`. Each previews what it would change and names what it leaves
  alone before touching anything.
- **Integration ownership and migration**, including adoption of an existing
  Claude Code setup rather than competing with it.
- **CC Switch as an integration manager.** It is classified as a manager and
  never as an agent adapter, so Cairn asks it to act instead of editing state
  it does not own; removals it must perform surface as
  `manager_action_required`.
- **A capability model that distinguishes what a vendor documents from what
  Cairn has actually observed here** — `FULL`, `MCP_PLUS`, `MCP_ONLY`. FULL is
  earned by an ordinary session rather than declared, and is withdrawn when an
  agent updates past the evidence.
- **Cross-agent project memory continuity.** Decisions, failures, procedures
  and handoffs are keyed by project and task, never partitioned by the agent
  that produced them.
- **Source-preserving config mutation** for JSON/JSONC, TOML and Markdown, so
  Cairn edits a user's configuration without reformatting the parts it does
  not own.
- **Hosted Playwright end-to-end CI**, on top of the existing Linux, macOS and
  Windows suites.

### Changed

- Session close is sealed: the boundary is acknowledged once termination is
  durable and the handoff is synthesized after, which keeps a one-second
  vendor handler budget survivable without giving up the completion guarantee.

### Notes

- Windows is covered by the integration suites on the same terms as macOS and
  Linux.
- The live vendor-agent evidence, the cross-agent onboarding walkthrough and
  the CC Switch Skill distribution path are recorded as manual release
  evidence and are not yet complete at the time of this entry.

## [0.1.0-alpha.3] — 2026-08-12

Windows is a supported platform.

### Added

- **Windows support.** `cairn` and `cairnd` talk over a named pipe instead of
  a Unix domain socket when built for Windows, and releases publish an
  `x86_64-pc-windows-msvc` archive alongside the existing macOS and Linux
  ones. The full test suite runs on `windows-latest` in CI, not only on macOS
  and Linux.
- `cairn update` handles what Windows does differently: binaries carry `.exe`,
  and a running binary cannot be overwritten in place, so it is renamed aside
  before the new one takes its path.

### Fixed

- **GitLab tokens are redacted.** The pattern set covered GitHub, AWS, Slack,
  Google and OpenAI-shaped keys but not GitLab's, so a `glpat-` token — or any
  of the `gloas-`/`glrt-`/`glcbt-`/`gldt-` family — captured in a command was
  stored verbatim unless it happened to sit next to a key name the assignment
  pattern recognised. Redaction runs before any write (FR-049), so this was the
  difference between a token never being persisted and one being persisted and
  synced.
- **`cairn link` no longer denies a link it already has.** With no arguments
  it reported `linked: false` unconditionally, so an already-linked project
  was told it was not linked and pointed at `cairn link --create` — which
  would have created a second shared project for a repository that already
  had one — while `cairn status`, reading the same row, reported it linked.
  It now reads the stored state, and does so without needing a server or a
  token, since whether a project is linked is local (C1).
- **`cairn` no longer leaks its standard handles into the daemon it starts.**
  Windows `CreateProcess` hands a child every inheritable handle, not only the
  three named in `STARTUPINFO`, so the daemon received a duplicate of whatever
  pipe the CLI had been given for stdout and held it open for its whole life.
  Anything capturing that output — a shell pipeline, or the agent running a
  capture hook — waited for an EOF that could not arrive.

### Notes

- The API token file is written `0600` on Unix. Windows has no mode bits to
  set, so there the file is only as private as the user-profile directory
  holding it.
- Server-sync tests need a PostgreSQL service container and so still run only
  on Linux, as before.

## [0.1.0-alpha.2] — 2026-08-09

Hardening on top of alpha.1: a deployment can now define its own operator, the
web interface was rebuilt, and both the CLI and the server can say what version
they are and whether a newer one exists.

### Added

- **Operator account from the environment.** `CAIRN_ADMIN_EMAIL` and
  `CAIRN_ADMIN_PASSWORD` define the account a fresh deployment signs in with,
  applied after migrations and before the listener binds. The variables *are*
  the password rather than an initial value, so rotating one is an edit and a
  restart. Setting only one is a startup error.
- **Token management in the browser.** Personal API tokens can be created,
  reviewed and revoked from the web UI. A new token's plaintext is shown once,
  in a panel only the reader dismisses, because the server keeps a hash.
- **`cairn update`.** Finds a newer release and installs it, verifying the
  archive against the digest the release publishes and refusing on a mismatch.
  `cairn` and `cairnd` are replaced together. `--check` reports without
  installing. Prereleases are offered only to someone already running one.
- **`/api/version`.** What a deployment runs, the newest eligible release, and
  whether it is worth moving to — looked up once by the server and cached, so
  browsers never spend a visitor's GitHub rate limit. Shown in the sidebar and
  on the sign-in page, which is readable without signing in.
- **`cairn auth status`.** Whether a credential is stored and for which server.
  The token itself is never printed.
- **`cairn daemon logs`.** The daemon writes to `cairnd.log` and this reads it.
  Previously `cairn` started the daemon with stderr discarded, so it
  effectively had no log at all.

### Changed

- **The web interface was rebuilt on shadcn/ui** with a collapsible sidebar
  shell, replacing a fixed-width column whose navigation only existed inside a
  project. Adds a breadcrumb, per-route titles, a styled 404, real loading,
  empty and error states, and confirmations on irreversible actions.
- **`cairn status` names the server** a linked project syncs to, and says when
  no token is stored for it.

### Fixed

- **The session cookie is marked `Secure`** when `CAIRN_WEB_ORIGIN` is an
  `https` origin. It is deliberately not hardcoded: a browser silently drops a
  `Secure` cookie received in clear, which would break every plain-HTTP
  deployment. The logout cookie carries matching attributes.
- **Sessions nothing has driven for two hours are closed.** A `SessionEnd`
  arriving for an agent key the daemon never saw used to leave its session
  `active` forever, and two such sessions in one worktree made `cairn context`
  fail with `ambiguous_session` — the call an agent makes before it knows its
  own session key.
- **Memory provenance under session ambiguity.** Recording a memory swallowed
  every session-resolution error, so ambiguity quietly opened a throwaway
  session and stamped the memory with an origin that never did the work.
  Ambiguity is now the caller's to resolve; `cairn memory add --session` and the
  `cairn_remember` MCP tool can both name one.
- **Daemons no longer steal a socket that is already being served.** Several
  starting at once replaced each other in turn, and a client connected to a
  displaced daemon saw its connection close mid-request. A daemon now stands
  down if someone is already answering, and the CLI retries a handover with
  bounded backoff.
- **The account menu no longer throws.** Its theme label sat outside the menu
  group Base UI requires, which killed the menu the moment it rendered.
- **Copying a new token falls back** when the async clipboard API is
  unavailable, and selects the token if both paths fail, rather than telling
  the reader to transcribe a 64-character secret.
- **The mobile sidebar closes when you navigate**, instead of leaving the sheet
  over the page you asked for.
- Memory filters name themselves rather than both reading `any`; page
  subtitles wrap instead of truncating; memory search is debounced and
  clearable and reports how many matched.

## [0.1.0-alpha.1] — 2026-08-08

Cairn has been rebuilt around a local-first, project-aware agent memory
architecture. **This release starts a new release line.** The v0.x releases
published before this one belong to an obsolete implementation, are not
upgradeable to this one, and have been retired.

### Added

- **Local-first capture and recall.** A local daemon (`cairnd`) records
  structured observations from a coding agent's session — files changed,
  commands run, tests executed, errors — into SQLite. Full conversations and raw
  tool output are never persisted.
- **Project, branch, task and session scoped memory.** Every durable memory
  carries explicit scope and provenance back to the session and observations
  that produced it. Retrieval respects scope precedence.
- **Repository identity derived from Git.** Project, branch, commit and
  working-tree state come from Git rather than being guessed, so two clones at
  different paths resolve correctly.
- **Context briefings.** A bounded, deterministic briefing is assembled at
  session start within a token budget, degrading from the bottom rather than
  truncating arbitrarily.
- **Session continuity and handoffs.** Handoffs are synthesized at compaction,
  session end, and recovery, recording completed work, remaining work,
  decisions, failures, tests, and a next step.
- **Claude Code integration.** `cairn connect claude-code` installs the hooks
  and registers Cairn's MCP server; the daemon starts on its own.
- **Lexical memory search** over SQLite FTS5 — no embeddings, no vector store,
  no external index service.
- **Opt-in server sync.** A linked project delivers queued changes to a shared
  Cairn server through a transactional outbox. Delivery is exactly-once in
  effect: rows are claimed before they are sent, and the server claims each
  idempotency key inside the transaction that applies the change.
- **Web UI** for browsing projects, sessions, handoffs, tasks, memory and sync
  status without a terminal.
- **Privacy boundary enforced structurally.** There is no observation entity
  type on the wire, so a payload carrying raw observation content cannot be
  constructed. An unlinked project produces no outbound request at all, and
  secrets matching common patterns are redacted before anything is written.

### Release engineering

- Native release archives for macOS (arm64, x86_64) and Linux (x86_64, arm64),
  each containing `cairn`, `cairnd`, `cairn-server`, the license and an install
  note, published with `SHA256SUMS` and an SPDX SBOM.
- Container images for the server and web UI published to GHCR, built for
  `linux/amd64` and `linux/arm64`, with build provenance attestations.
- An example Compose stack under `deploy/` for running the server, web UI and
  PostgreSQL.

### Known limitations

- **Windows is not supported.** The daemon and CLI communicate over a Unix
  domain socket. Windows support is not implemented and no Windows artifact is
  published.
- The server and web UI are alpha. APIs, storage schemas and the wire protocol
  may change without a deprecation period before 1.0.0.
- Sharing requires running your own Cairn server; no hosted service exists.

[0.1.0-alpha.4]: https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.4
[0.1.0-alpha.3]: https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.3
[0.1.0-alpha.2]: https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.2
[0.1.0-alpha.1]: https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.1
