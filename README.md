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
VERSION=0.1.0-alpha.4
TARGET=aarch64-apple-darwin   # x86_64-apple-darwin | x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu

curl -fsSLO https://github.com/Vellixia/Cairn/releases/download/v${VERSION}/cairn-v${VERSION}-${TARGET}.tar.gz
curl -fsSLO https://github.com/Vellixia/Cairn/releases/download/v${VERSION}/SHA256SUMS
sha256sum --ignore-missing -c SHA256SUMS

tar -xzf cairn-v${VERSION}-${TARGET}.tar.gz
sudo install -m 0755 cairn-v${VERSION}-${TARGET}/{cairn,cairnd} /usr/local/bin/
```

On Windows (PowerShell):

```powershell
$VERSION = "0.1.0-alpha.4"
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

`cairn connect` shows exactly what it would change and asks before touching anything. See
[docs/integrations.md](docs/integrations.md) for the whole surface: which agents are
supported and what each one can actually do, where each resource is written and why,
what `--shared` changes, and how to check, repair and remove an integration.

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
| `cairn agents` | Which agents are installed, and what each integration actually provides |
| `cairn connect [agent]` | Install or update an integration (preview first with `--dry-run`) |
| `cairn doctor` | Check every installed resource and say what to run to fix it |
| `cairn repair` | Restore what Cairn owns and nothing else |
| `cairn disconnect <agent>` | Remove this agent's integration; your memory is untouched |
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
ghcr.io/vellixia/cairn-server:0.1.0-alpha.4
ghcr.io/vellixia/cairn-web:0.1.0-alpha.4
```

Only the shared components ship as containers. The local agent stays native.

The web image is not tied to a hostname. It calls the same origin that served the
page, so behind a reverse proxy publishing the UI at `/` and the API at `/api` on
one domain there is nothing to configure. For a split-origin deployment set
`CAIRN_API_ORIGIN` on the web container — it is read at start, so no rebuild is
needed to move domains.

To run the same thing from source instead:

```bash
docker compose up -d postgres
cargo run --release --bin cairn-server -- --web-origin http://127.0.0.1:3100
cd web && npm install && npm run build && npm run start
```

There is no sign-up form. An administrator creates the account (see
[Administering a server](#administering-a-server)), hands over the one-time
temporary password, and the user changes it on first use. Then, on each machine:

```bash
cairn auth login --server http://127.0.0.1:8080   # change the temporary password
cairn auth token set <token> --server http://127.0.0.1:8080
cairn link --project <shared-project-id>          # or: cairn link --create
```

Linking to an existing project needs a membership an administrator or an existing
member granted first — naming the identifier is not enough:

```bash
cairn project member add <project-id> dev@example.com
cairn project member list <project-id>
cairn project member remove <project-id> dev@example.com
```

`cairn link` with no `--project` auto-selects only when discovery returns exactly
one project the caller already belongs to. It never joins anything.

Two clones of one repository at different paths link to the *same* shared project by
identifier — path is never identity.

## Personal and team knowledge

Alongside project memory, which stays scoped to the repository it came from, a
server carries two domains that are not:

- **Personal** — yours, across every project and every machine you sign in from.
  Create it directly, or promote a project memory into it.
- **Team** — server-wide guidance every account sees. It cannot be authored
  directly: someone proposes it, an administrator ratifies it, and only then does
  it become visible. A retired entry stays retired; restoring its guidance means a
  new proposal.

```bash
cairn team list
cairn team propose "Prefer the workspace lockfile over a per-crate one"
cairn team ratify <id>
cairn team retire <id>
```

Both domains are stripped of anything that identifies where they came from.
Content carrying an absolute path, a home directory, a credentialed URL, an
environment assignment, a secret-shaped run, a project-identifying token or a
shell command invocation is refused — locally when you write it, and again at the
server when it arrives, so a client that skips its own check gains nothing. A
refusal names the class it tripped and never echoes the content back.

They never displace project context: personal and team sections come last, are
capped at 15% of the budget, cannot touch the reserved level, and are excluded
entirely at `depth: "minimum"`.

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

## Administering a server

Every account on a Cairn server is created by an administrator. There is no
self-registration and no self-join — both were removed as security fixes, along
with a project-discovery route that returned projects the caller was not a member
of.

```bash
# On the server host, once, to bootstrap:
CAIRN_ADMIN_EMAIL=you@example.com CAIRN_ADMIN_PASSWORD='...' cairn-server

# Or create an account directly against the database:
cairn-server users add --email dev@example.com --display-name "Dev" --password '...'

# Thereafter, as an administrator:
cairn user create --email dev@example.com --display-name "Dev"
cairn user list
cairn user promote dev@example.com
cairn user disable dev@example.com
cairn user reset-password dev@example.com
```

`cairn user create` prints a one-time temporary password. It is shown **once** —
no route reads it back, not even for the administrator who created the account.
If it is lost, reset it.

**If your users relied on registering or joining themselves**, those two routes
now answer `410 Gone` and name their replacement in the response body. The
operator does the work instead:

| A user who used to… | An administrator now runs |
|---|---|
| register an account | `cairn user create --email … --display-name …` (`POST /api/admin/users`) |
| join a project by its identifier | `cairn project member add <project-id> <email>` (`POST /api/projects/{id}/members`) |

A project-discovery lookup returns only projects the caller is already a member
of, so discovery cannot be used to find something to join.

**Whoever can set the server's environment and restart the process can always
obtain administrator access.** The account named by `CAIRN_ADMIN_EMAIL` is
restored to `admin` and `active` on every start, which is the break-glass path
for an operator who has locked themselves out. It cannot be demoted, disabled or
reset through the API. See [SECURITY.md](SECURITY.md) for what that means for a
deployment.
