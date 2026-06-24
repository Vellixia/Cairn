# Upgrading to 0.5.0

0.5.0 adds the **Context + Reliability + Distribution + Proactive** layers (23 sprints, 22
crates). The single-admin/cookie-session model from 0.4.0 is unchanged â€” this release expands
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

- 21 crates in the workspace. The old 14-crate dep graph is gone â€” `cairn-session`,
  `cairn-pack`, `cairn-registry`, `cairn-sync`, `cairn-bench`, `cairn-proactive`,
  `cairn-proxy`, and `cairn-ingest` are new.
- **HelixDB is required.** `cairn-store` ships a pluggable backend (HelixDB +
  in-memory). If you were on the 0.4 SQLite backend, run a HelixDB container and
  `docker compose up -d helix` before restarting Cairn. The server refuses to start
  when `CAIRN_HELIX_URL` is unset (or unreachable from the bound interface).
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

```sh
cairn export dump.json           # personal memories (deterministic order)
cairn export --share share.json  # sanitized bundle (safe to inspect)
```

If you were on 0.3.x, both flows are unchanged from 0.4.0.

### 2. Point at HelixDB

`docker compose up -d helix` starts HelixDB on `:6969`. Set in `.env`:

```sh
CAIRN_HELIX_URL=http://localhost:6969
```

When the URL is unset *and* the bind address is non-loopback, the server refuses to start.
This is intentional â€” running the audit pipeline against an in-memory backend would lose
data on restart.

### 3. Upgrade the binary

```sh
git pull --tags
cargo build --workspace --release
# or via the one-liner installer:
curl -fsSL https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.sh | sh
```

The 0.4.0 device tokens still authenticate â€” JWTs are HS256 and the secret is the same
(`CAIRN_SECRET_KEY`). Existing sessions are not invalidated.

### 4. Run the e2e harness

```sh
pwsh scripts/e2e.ps1            # 20 scenarios, ~67/69 assertions pass
```

The harness needs `cairn` and `cairn` on `$PATH` plus a running HelixDB. It exercises
memory, context, guardrails, sessions, sync, federation, registry, sync, ingest,
proactive, and the mobile companion.

## Removed

- `deploy/` (Compose templates, k8s manifests, Helm chart) â€” replaced by
  `cairn install --docker` and the root `docker-compose.yml`.
- `extensions/chrome/` â€” moved to `POST /api/extensions/capture`.
- `web/out/_next/` build artifacts â€” gitignored; rebuild with `cd web && npm run build`.

## New config keys

| Key | Default | Notes |
|---|---|---|
| `CAIRN_HELIX_URL` | `http://localhost:6969` | Required for non-loopback binds |
| `CAIRN_EMBED_PROVIDER` | `hashing` | `onnx` opt-in via `cairn-embed` feature |
| `CAIRN_PROACTIVE_DEFAULT` | `on` | Set `off` to disable auto-inject for all users |
| `CAIRN_REGISTRY_URL` | _(unset)_ | Enables federation; pull-based, cursor: `revocations_since(since)` |
| `CAIRN_PROXY_ADDR` | `127.0.0.1:7780` | The `cairn.sh` reverse-proxy listener |
| `CAIRN_PUSH_VAPID_KEY` | _(unset)_ | Enables PWA push; pair with `CAIRN_PUSH_VAPID_SECRET` |

## Verifying the upgrade

```sh
cargo build --workspace
cargo test --workspace         # 330 lib tests pass; 5 ignored are live-HelixDB ones
docker compose up -d           # in-container cairn-server now resolves config + bootstrap from .env
pwsh scripts/e2e.ps1           # end-to-end harness
```

## Open items

- 2FA / TOTP is still not implemented. Tracked for 0.6.0.
- Per-tenant quotas are enforced by the new `OrgId` column but no admin UI surfaces them
  yet â€” `cairn tenant quota <org> --set N` is the workaround.
- `cairn-registry` ships with Local/Team/Public trust scopes; cross-scope imports
  return `RegistryError::ScopeDenied` and do not auto-elevate.
