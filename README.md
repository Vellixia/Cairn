# Cairn

Persistent, project-aware memory for AI coding agents.

Cairn captures bounded structured events from supported agents, delivers them to a
canonical server, and returns relevant memory to later sessions. PostgreSQL owns
knowledge, evidence, relations, sessions, handoffs, governance, retrieval, and
idempotency receipts. The local daemon owns only durable delivery spools, receipts,
bounded context cache, hook correlation, and integration metadata.

## Build

The workspace uses its pinned Rust toolchain:

```bash
cargo build --workspace --release
export PATH="$PWD/target/release:$PATH"
```

Build the web application separately:

```bash
cd web
npm ci
npm run build
```

Supported targets: macOS arm64/x86_64, Linux arm64/x86_64, and Windows x86_64.

## Setup

Create an API token in web **Settings**, then run the only human CLI command from
the repository to connect:

```bash
CAIRN_SERVER_URL=https://cairn.example.com \
CAIRN_SERVER_TOKEN="$CAIRN_INSTALL_TOKEN" \
cairn setup
```

For headless use, pass the same values as protected JSON on stdin:

```json
{
  "server_url": "https://cairn.example.com",
  "server_token": "…",
  "account_id": "optional-expected-account-uuid",
  "web_url": "https://cairn.example.com"
}
```

`setup` detects the Git repository, selects an existing server project by remote,
installs supported-agent integration, starts `cairnd`, verifies the credential and
project membership, and prints the web URL. It cannot register an account, create
membership, or grant access.

Run `cairn setup` again to repair Cairn-owned integration bytes. User edits that no
longer match recorded ownership are reported as conflicts and are not overwritten.
See [integration ownership](docs/integrations.md).

Repositories must have a remote that matches an existing project. Administrators
create accounts and setup-ready projects with that exact remote, then grant
membership, in web before machine setup.

## Agent interface

Hooks capture lifecycle and safe structured activity automatically. MCP exposes five
tools:

- `cairn_context`
- `cairn_search`
- `cairn_remember`
- `cairn_session`
- `cairn_handoff`

Explicit session and handoff calls are recovery overrides. Normal capture, session
boundaries, delivery, consolidation, and handoff require no manual MCP calls.

`cairn hook` and `cairn mcp` are hidden machine adapters installed by `setup`; they
are not human administration commands.

## Offline behavior

`cairnd` accepts privacy-filtered capture into bounded typed spools and drains them
independently of agent lifetime. A full spool rejects new capture visibly; it never
silently drops a boundary event. Delivery is at-least-once with stable operation
identity, while the server records one canonical effect and returns the prior receipt
for retries.

Eligible cached context is finite-age and labelled with its age and identity. Search
reports server unavailability. Authentication denial invalidates matching cache
immediately. Offline clients do not claim to know whether access was revoked.

## Web

Human work lives in six destinations:

- **Project selector** — choose an authorized project.
- **Overview** — bounded counts, recent accepted activity, delivery recency, and
  retrieval effectiveness.
- **Memory** — project/personal scope, search, mutation, evidence, verification,
  relations, graph, and retrieval explanation.
- **Sessions** — history, handoffs, and accepted-event replay.
- **Governance** — proposals, conflicts, ratification, retirement, and supersession.
- **Settings** — password, tokens, projects, membership, privacy policy, logical
  import/export, users, and server health. Administrator controls are role-gated.

Disconnected health is always shown with its last report timestamp and stale status.

## Deployment

Example deployment:

```bash
cp deploy/.env.example deploy/.env
docker compose -f deploy/docker-compose.yml up -d
```

Recommended deployment serves web at `/` and API at `/api` on one origin. For split
origins, set `CAIRN_API_ORIGIN` on the web container and configure server
`--web-origin` to the exact web origin.

Initial administrator credentials come from deployment environment only when no
administrator exists; bootstrap creates an active account with the admin role. Restart
never replaces a password changed in web. PostgreSQL backup and restore are operator
infrastructure; web logical import/export is for healthy-deployment transfer and
excludes credentials.

Whoever can set `CAIRN_ADMIN_EMAIL` and `CAIRN_ADMIN_PASSWORD` and restart the server
can always obtain administrator access. Protect deployment environment accordingly.

## Legacy stores

When `setup` finds a legacy SQLite database, it preserves the original, makes a
verified backup including WAL state, creates a fresh edge database, and writes a
versioned import bundle plus conservation report. Only unambiguously safe pending
operations keep their original identities. Task, local-only, unsupported, and
ambiguous records remain offline as `removed_feature`; scope is never widened.

Upload an eligible bundle in web **Settings**. Server import is resumable and
idempotent, with an accepted, rejected, retained, pending, or unchanged disposition
for each source record. Migration failure does not block new safe capture.

## Development checks

```bash
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cd web && npm run typecheck && npm run check:api-contract && npm run build
```

PostgreSQL integration tests use `CAIRN_TEST_DATABASE_URL`. Tests that need it report
a skip when the variable is absent.

## Privacy

Raw prompts, transcripts, diffs, command output, credentials, and unbounded payloads
do not cross the machine boundary. Both edge and server enforce the safe-event shape,
bounds, path restrictions, and secret screening. Refusals name the policy class and
never echo rejected content.

Cairn is pre-1.0. Contracts and storage schemas may still change between releases.
