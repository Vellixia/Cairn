# Changelog

All notable changes to Cairn are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Cairn is pre-1.0. Until 1.0.0, minor versions may change behaviour, storage
schemas, and the wire protocol without a deprecation period.

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

[0.1.0-alpha.1]: https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.1
