# Cairn

Project-intelligence and reliability system for AI coding agents. This
repository currently implements the **local session foundation**
([spec](specs/001-local-session-foundation/spec.md)): register a Git
repository under a stable Git-private identity, inspect exact repository
state, take deterministic BLAKE3 snapshots, and run agent sessions bound to
those snapshots — fully offline, with an append-only local event history.

## Quick usage

```console
$ cargo build --workspace          # binaries: cairn (CLI), cairnd (daemon)
$ cd your-git-repo
$ cairn init                       # register (idempotent; auto-starts cairnd)
$ cairn status                     # exact state: branch, HEAD, staged/unstaged/untracked, ignored summary
$ cairn session start --agent claude-code
$ cairn session show               # start snapshot vs current snapshot
$ cairn session stop
$ cairn daemon status
```

Every command supports `--json` for a stable machine-readable envelope
(`cairn.cli.v1` — [contract](specs/001-local-session-foundation/contracts/cli-json-contract.md)).
Resume tokens are issued in `--json` mode only and are accepted via
`--resume-token-stdin`, `CAIRN_RESUME_TOKEN`, or `--resume-token-file` —
never as ordinary arguments, never printed in human output or logs.

## Workspace layout

| Crate | Owns |
|---|---|
| `crates/cairn-domain` | Pure types: identities, fingerprint canonicalization, session state machine. No IO. |
| `crates/cairn-protocol` | IPC + CLI DTOs, error codes, JSON Schemas (golden-diffed in CI). |
| `crates/cairn-git` | Git CLI adapter: porcelain v2 parsing, identity markers, ignored-summary walker, fingerprint pipeline. |
| `crates/cairn-events` | 11-type append-only event catalog, idempotency keys, replay. |
| `crates/cairn-session` | Lifecycle policy: idempotent start, resume-token leases, recovery, staleness. |
| `crates/cairn-storage-local` | SQLite (WAL): migrations, DAOs, serialized transactional event append. |
| `apps/daemon` (`cairnd`) | IPC server (UDS / DACL-restricted named pipe), filesystem watcher, recovery sweeper. |
| `apps/cli` (`cairn`) | Thin IPC client with daemon auto-spawn. |
| `fixtures/repositories` | Deterministic Git fixtures for tests. |

Architecture rules (from the [plan](specs/001-local-session-foundation/plan.md)):
events are append-only and immutable (enforced by SQLite triggers); event
append + projection update share one transaction serialized per worktree;
filesystem notifications are advisory hints only — `git` reconciliation
produces every authoritative snapshot.

## Development

```console
$ cargo test --workspace                       # full suite
$ cargo test -p cairn-daemon --test perf -- --ignored   # SC-007 perf bounds
$ cargo clippy --workspace --all-targets -- -D warnings
```

Validation scenarios: [quickstart.md](specs/001-local-session-foundation/quickstart.md).
Governance: [constitution](.specify/memory/constitution.md).
