---
title: "Upgrading to 0.5.0"
type: guide
status: living
updated: 2026-07-02
---

# Upgrading to 0.5.0

0.5.0 adds the **Context + Reliability + Distribution + Proactive** layers (23 sprints, 22
crates). The single-admin/cookie-session model from 0.4.0 is unchanged - this release expands
*what* Cairn does, not *how* you log in.

## What changed for end users

- The dashboard still serves a single admin behind a cookie session.
- The **Devices** panel now exposes a per-token **scope** dropdown (`read` / `write` /
  `admin`) and lists revocation time.
- Browser-extension capture moved server-side: `POST /api/extensions/capture` accepts an
  Origin-gated JSON payload and stores it as a `Memory` with `source = "extension"`. The
  bundled Chrome extension has been retired.
- A **mobile companion** PWA lives at `/mobile` (installable; uses the service worker at
  `/sw.js`).
- A landing page replaces the old `/` redirect for unauthenticated visitors; the authed
  redirect to `/dashboard` is preserved.

## What changed for operators

- 21 crates in the workspace. The old 14-crate dep graph is gone - `cairn-session`,
  `cairn-pack`, `cairn-registry`, `cairn-sync`, `cairn-bench`, `cairn-proactive`,
  `cairn-proxy`, and `cairn-ingest` are new.
- **A datastore is required.** `cairn-store` ships a pluggable backend (currently
  SurrealDB, plus an in-memory backend for hermetic tests). If you were on the 0.4
  SQLite backend, run `docker compose up -d surreal` before restarting Cairn. As of
  v0.8.0 the datastore is SurrealDB, not HelixDB - see "Upgrading to 0.8.0" below if
  you're coming from a HelixDB-backed 0.5.0-0.7.x install.
- `deploy/` templates and the Chrome extension under `extensions/chrome/` were removed.
  Use `cairn onboard` (or the new `cairn install --docker` subcommand) to bootstrap
  a fresh stack.
- `cairn-bench` now produces a single CSV row per fixture (`LongMemEval`, `horizon`,
  `retention`) and prints token savings alongside MRR.
- The Cairn repository does not commit `web/out/`. The directory is created at
  compile time by `crates/cairn-api/build.rs` if missing; the Next.js static export
  is gitignored. CI must run `cd web && npm ci && npm run build` before
  `cargo build --workspace` if the dashboard is needed at runtime; otherwise the
  binary falls back to its built-in page.

## Migration steps

### 1. Back up before you touch anything

Use the dashboard export feature or the `GET /api/share/export` API endpoint.

### 2. Point at SurrealDB

`docker compose up -d surreal` starts SurrealDB (no host port - reachable only over the
compose network). Set in `.env`:

```sh
CAIRN_DB_URL=ws://localhost:8000
```

`CAIRN_DB_URL` defaults to `ws://localhost:8000` if unset. Running the audit pipeline
against an in-memory backend would lose data on restart, so production installs should
always point this at a real, persistent SurrealDB server.

### 3. Upgrade the binary

```sh
git pull --tags
cargo build --workspace --release
# or via the one-liner installer:
curl -fsSL https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.sh | sh
```

The 0.4.0 device tokens still authenticate - JWTs are HS256 and the secret is the same
(`CAIRN_SECRET_KEY`). Existing sessions are not invalidated.

### 4. Run the e2e harness

```sh
pwsh scripts/e2e.ps1            # 20 scenarios, ~67/69 assertions pass
```

The harness needs `cairn` and `cairn` on `$PATH` plus a running SurrealDB. It exercises
memory, context, guardrails, sessions, sync, federation, registry, sync, ingest,
proactive, and the mobile companion.

## Removed

- `deploy/` (Compose templates, k8s manifests, Helm chart) - replaced by
  `cairn install --docker` and the root `docker-compose.yml`.
- `extensions/chrome/` - moved to `POST /api/extensions/capture`.
- `web/out/_next/` build artifacts - gitignored; rebuild with `cd web && npm run build`.

## New config keys

| Key | Default | Notes |
|---|---|---|
| `CAIRN_DB_URL` | `ws://localhost:8000` | SurrealDB server URL |
| `CAIRN_EMBED_PROVIDER` | `hashing` | `onnx` opt-in via `cairn-embed` feature |
| `CAIRN_PROACTIVE_DEFAULT` | `on` | Set `off` to disable auto-inject for all users |
| `CAIRN_REGISTRY_URL` | _(unset)_ | Enables federation; pull-based, cursor: `revocations_since(since)` |
| `CAIRN_PROXY_ADDR` | `127.0.0.1:7780` | The `cairn.sh` reverse-proxy listener |
| `CAIRN_PUSH_VAPID_KEY` | _(unset)_ | Enables PWA push; pair with `CAIRN_PUSH_VAPID_SECRET` |

## Verifying the upgrade

```sh
cargo build --workspace
cargo test --workspace         # 330 lib tests pass; 5 ignored are live-SurrealDB ones
docker compose up -d           # in-container cairn-server now resolves config + bootstrap from .env
pwsh scripts/e2e.ps1           # end-to-end harness
```

## Data persistence: which volume holds what

Cairn's `docker-compose.yml` uses **two** Docker volumes:

| Volume | Persists | Notes |
|---|---|---|
| `cairn-data` | Admin record, audit log, sessions, ledger, push subscriptions | Mounted at `/data` inside the cairn container |
| `cairn-surreal-data` | **All SurrealDB data** - memories, device tokens, sync state, pairing codes, checkpoint metadata | RocksDB-backed, mounted at `/data` inside the surreal container |

**Critical:** `docker compose down` preserves both volumes. `docker compose down -v`
**wipes both**. Losing `cairn-surreal-data` silently invalidates every device token (the
JWT signatures still pass, but the token ids no longer exist in SurrealDB - every request
returns 401 `"unknown_token"`). Recovery: mint new tokens via the dashboard,
then re-run `cairn setup --server <url> --token <jwt>` on each device.

## Token rotation after secret-key or data changes

When a device token returns 401, the error body now includes a machine-readable
`reason` field:

| `reason` | Cause | Fix |
|---|---|---|
| `bad_signature` | `CAIRN_SECRET_KEY` rotated | Restore the old key, or mint a fresh token |
| `unknown_token` | Token revoked or SurrealDB data lost | Mint a new token and re-run `cairn setup` |
| `insufficient_scope` | Token scope does not permit the endpoint | Upgrade token scope in the dashboard |

## Upgrading to 0.8.0

v0.8.0 replaces the HelixDB + MinIO datastore with SurrealDB (Sprint 1). This is a
storage-engine swap, not a config-compatible upgrade:

- **No data migration path.** Old HelixDB/MinIO data is not carried forward - a fresh
  SurrealDB instance starts empty. If you need your existing memories, export them first
  with the dashboard's export feature (or `GET /api/share/export`) *before* upgrading,
  then re-import after (`cairn import` / `POST /api/share/import`).
- `CAIRN_HELIX_URL`, `CAIRN_HELIX_TOKEN`, `CAIRN_HELIX_NS`, `HELIX_PORT`,
  `MINIO_ROOT_USER`, and `MINIO_ROOT_PASSWORD` are gone. Replace them with `CAIRN_DB_URL`
  (default `ws://localhost:8000`), `CAIRN_DB_USER` (default `root`), `CAIRN_DB_PASS`,
  `CAIRN_DB_NS` (default `cairn`), `CAIRN_DB_TIMEOUT_SECS` (default `10`), and, for the
  bundled docker-compose stack, `SURREAL_USER` / `SURREAL_PASS`.
- `docker-compose.yml`'s `minio-guard`, `minio`, `minio-init`, and `helix` services are
  replaced by `surreal-guard` and `surreal`. The `helix-minio` volume is replaced by
  `cairn-surreal-data`. Run `docker compose down` (not `-v`, unless you've already
  exported) before pulling, then `docker compose up -d` to bring up the new stack.
- The SurrealDB Rust SDK and server major versions must match (both `3.x`) - the bundled
  compose stack pins `surrealdb/surrealdb:v3`. A `v2.x` server rejects the `v3.x`
  client's WebSocket subprotocol handshake.
- MSRV bumped 1.85 -> 1.89 (required by `surrealdb ~3.1`).

`cairn doctor` now validates the token with a real server request (not just a
presence check), and `cairn setup` refuses to write an invalid token to agent config
files.

## Open items

- 2FA / TOTP is still not implemented. Tracked for 0.6.0.
- Per-tenant quotas are enforced by the new `OrgId` column but no admin UI surfaces them
  yet - `cairn tenant quota <org> --set N` is the workaround.
- `cairn-registry` ships with Local/Team/Public trust scopes; cross-scope imports
  return `RegistryError::ScopeDenied` and do not auto-elevate.
