# Cairn

Persistent, project-aware memory for AI coding agents.

> **Alpha.** Cairn was rebuilt around this architecture; `v0.1.0-alpha.1` started a new
> release line, and the earlier v0.x releases have been retired. Pre-1.0, so APIs,
> storage schemas, and the wire protocol can change without a deprecation period.

An AI coding session starts blind. Everything the last session learned — the goal, what was
tried, what failed, which decisions were already made, which conventions this repository
follows — disappears when the context window ends. You re-explain, the agent re-discovers,
and the same dead ends get walked twice.

Cairn sits beside the agent and fixes that. It knows which repository, branch, and commit you
are on. It captures what a session actually does as structured facts. It turns the important
ones into scoped, durable memory. And it hands the next session a bounded briefing so work
resumes instead of restarting.

## Install and connect

Download a release archive, verify it, and put the binaries on your PATH. `cairn` and
`cairnd` must live in the same directory — `cairn` starts the daemon itself.

```bash
VERSION=0.1.0-alpha.3
TARGET=aarch64-apple-darwin   # x86_64-apple-darwin | x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu

curl -fsSLO https://github.com/Vellixia/Cairn/releases/download/v${VERSION}/cairn-v${VERSION}-${TARGET}.tar.gz
curl -fsSLO https://github.com/Vellixia/Cairn/releases/download/v${VERSION}/SHA256SUMS
sha256sum --ignore-missing -c SHA256SUMS

tar -xzf cairn-v${VERSION}-${TARGET}.tar.gz
sudo install -m 0755 cairn-v${VERSION}-${TARGET}/{cairn,cairnd} /usr/local/bin/
```

On Windows (PowerShell):

```powershell
$VERSION = "0.1.0-alpha.3"
$TARGET = "x86_64-pc-windows-msvc"

Invoke-WebRequest "https://github.com/Vellixia/Cairn/releases/download/v$VERSION/cairn-v$VERSION-$TARGET.zip" -OutFile cairn.zip
Invoke-WebRequest "https://github.com/Vellixia/Cairn/releases/download/v$VERSION/SHA256SUMS" -OutFile SHA256SUMS

Expand-Archive cairn.zip -DestinationPath .
mkdir "$env:LOCALAPPDATA\Cairn" -Force
Copy-Item "cairn-v$VERSION-$TARGET\cairn.exe","cairn-v$VERSION-$TARGET\cairnd.exe" "$env:LOCALAPPDATA\Cairn"
setx PATH "$env:PATH;$env:LOCALAPPDATA\Cairn"
```

Or build from source — nothing is needed beyond the pinned toolchain:

```bash
cargo build --workspace --release          # cairn, cairnd, cairn-server
export PATH="$PWD/target/release:$PATH"
```

Then, in a repository:

```bash
cd your-git-repo
cairn init                                 # register this repository
cairn connect claude-code                  # install hooks + the MCP server
```

**Supported platforms:** macOS on Apple silicon and Intel, Linux on x86_64 and arm64,
Windows on x86_64. The CLI and daemon talk over a Unix domain socket on Unix and a
named pipe on Windows — either way, nothing is exposed on the network.

Start a Claude Code session in that repository. Cairn starts its daemon on its own, opens a
session, and begins capturing. When the session ends:

```bash
cairn handoff show          # what happened, what changed, what remains, what's next
cairn context               # the briefing the next session will receive
```

Nothing leaves your machine unless you link the project to a server.

## How it works

```text
install → connect Claude Code → open a Git repository
        → Cairn detects repository, branch, commit
        → select or create a task
        → session starts automatically
        → agent receives relevant context
        → agent works normally
        → Cairn captures structured observations
        → important facts and decisions become scoped memory
        → session stops or compacts
        → Cairn writes a structured handoff
        → next session starts with the previous context restored
```

Memory carries explicit scope — **project**, **branch**, **task**, or **session** — and
provenance back to the session and observations that produced it. Recall ranks by scope
first: a fact about *this task* beats an unrelated one, however well it matches.

## Everyday commands

| Command | What it does |
|---|---|
| `cairn status` | Project, branch, commit, working tree, active sessions, integration mode |
| `cairn session list` | Every session, newest first |
| `cairn task new --title T --goal G --criterion C` | Create a task |
| `cairn memory add --type convention --scope project "…"` | Remember something |
| `cairn memory search "tests"` | Recall, ranked task → branch → project |
| `cairn handoff show` | Read the latest handoff |
| `cairn privacy exclude --path "secrets/**"` | Never capture that path |
| `cairn delete session <id>` | Remove a session; its memories survive |
| `cairn link --create` | Opt this project into server sharing |
| `cairn sync status` | Pending, failed, last successful sync |

Every command takes `--json` and prints a stable envelope.

## Sharing with a team (optional)

The quickest route is the example stack — PostgreSQL, the server, and the web UI:

```bash
cp deploy/.env.example deploy/.env        # then edit the password
docker compose -f deploy/docker-compose.yml up -d
curl -fsS http://127.0.0.1:8080/api/health   # {"ok":true}
open http://127.0.0.1:3100
```

Images are published per release, for `linux/amd64` and `linux/arm64`:

```
ghcr.io/vellixia/cairn-server:0.1.0-alpha.3
ghcr.io/vellixia/cairn-web:0.1.0-alpha.3
```

Only the shared components ship as containers. The local agent stays native.

To run the same thing from source instead:

```bash
docker compose up -d postgres
cargo run --release --bin cairn-server -- --web-origin http://127.0.0.1:3100
cd web && npm install && npm run build && npm run start
```

Register in the web UI, create a personal API token, then on each machine:

```bash
cairn auth token set <token> --server http://127.0.0.1:8080
cairn link --create            # or: cairn link --project <shared-project-id>
```

Two clones of one repository at different paths link to the *same* shared project by
identifier — path is never identity.

## Principles

- **Local-first.** Everything works offline. Capture, recall, briefing, handoff, and search
  never need a network. Cairn never blocks the agent it is attached to: capture hooks have a
  250 ms deadline, always exit 0, and drop work rather than wait.
- **Private by default.** No conversation transcripts. No unbounded command output. Secrets
  redacted before anything is written. **Raw observations never leave your machine** — a
  shared memory carries evidence identifiers and a count, never the observation rows behind
  them. A shared *handoff* does carry its own derived fields, including changed file paths
  and short statements like "test failed: cargo test", because that is what a handoff is
  for; nothing leaves at all until you link the project.
- **Simple.** A local daemon with SQLite, a small Axum server with PostgreSQL, a Next.js UI.
  No brokers, no vector databases, no knowledge graphs, no embeddings.
- **Honest about what it knows.** Cairn cannot tell whether an agent is still alive — nothing
  in the integration says so — so it does not guess. Sessions end at deterministic boundaries.

## Layout

| Path | What |
|---|---|
| `crates/cairn-core` | Domain types, redaction, budgeting, context and handoff synthesis |
| `crates/cairn-git` | Git CLI adapter |
| `crates/cairn-store` | SQLite schema, repositories, FTS5 search, outbox |
| `crates/cairnd` | The local daemon |
| `crates/cairn` | The CLI, the hook runtime, and the MCP server |
| `crates/cairn-server` | Axum + PostgreSQL shared server |
| `web/` | Next.js web UI |
| `tests/` | End-to-end suite against real repositories, SQLite and PostgreSQL |

Six MCP tools, no more: `cairn_context`, `cairn_search`, `cairn_remember`, `cairn_session`,
`cairn_task`, `cairn_handoff`.

## Development

```bash
cargo test --workspace                                   # local suite
docker compose up -d postgres
CAIRN_TEST_DATABASE_URL=postgres://cairn:cairn@localhost:5433/cairn cargo test --workspace
cd web && npx playwright test                            # UI acceptance
```

Specification, plan and task ledger live in [`specs/001-cairn-mvp/`](specs/001-cairn-mvp/);
project principles in [the constitution](.specify/memory/constitution.md).
