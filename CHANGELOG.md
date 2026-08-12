# Changelog

All notable changes to Cairn are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Cairn is pre-1.0. Until 1.0.0, minor versions may change behaviour, storage
schemas, and the wire protocol without a deprecation period.

## [Unreleased]

### Added

- **Windows support.** `cairn` and `cairnd` now talk over a named pipe
  instead of a Unix domain socket when built for Windows, and releases
  publish an `x86_64-pc-windows-msvc` archive alongside the existing macOS
  and Linux ones. `cairn update` handles the one real difference from
  Unix: Windows will not let a running binary be overwritten in place, so
  it is renamed aside first.

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

[0.1.0-alpha.2]: https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.2
[0.1.0-alpha.1]: https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.1
