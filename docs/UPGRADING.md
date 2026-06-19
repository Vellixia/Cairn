# Upgrading to 0.4.0

0.4.0 replaces the unauthenticated "open dashboard" model with a single admin account
behind a cookie session, and moves device-token issuance into the web UI.

## What changed for end users

- Visiting `/` now redirects to `/dashboard`.
- On a fresh install, the dashboard shows a **setup wizard** at `/setup` the first time
  it loads. After you create the admin, subsequent visits go to `/login` and then
  `/dashboard`.
- The CLI now uses **device tokens** (HS256 JWTs) you mint from the dashboard's
  **Devices** panel, instead of the previously-pasted dev token. The CLI's own
  `cairn token create` and `cairn pair-code` commands still work for automation.

## Migration steps

### 1. Pick your bootstrap mode

The admin record lives in the same meta store as the rest of Cairn state. You can
seed it three ways, in priority order:

| Priority | Method | When to use |
|----------|--------|-------------|
| 1 | `CAIRN_ADMIN_PASSWORD_HASH` in `.env` | Production. Pre-hash with: `cairn-server admin password --print-hash` (see below) — or any Argon2id tool that emits the PHC format (`$argon2id$v=19$m=...$t=...$p=...$salt$hash`). |
| 2 | `CAIRN_ADMIN_PASSWORD` in `.env` | Loopback dev only. Refused on non-loopback binds unless `CAIRN_INSECURE=1`. |
| 3 | First-run `/setup` wizard | Easiest path. Visit `http://localhost:7777/setup`, set a username and an 8+ char password. |

The default username is `admin`. Override with `CAIRN_ADMIN_USERNAME`.

### 2. Migrate existing device tokens

If you were using `cairn token create` in 0.3.x, the tokens still work — they were
HS256 JWTs the whole time. The only thing that changed is that you can now mint,
list, and revoke them from the dashboard.

To copy a token out of the store into a CLI machine:

```sh
# On the server host:
cairn-server token list

# The id column is the JWT id, but the bearer itself was never persisted in cleartext.
# Re-issue from the dashboard if you've lost it, then paste into the CLI's env.
```

### 3. Existing 0.3.x deployments without an admin

The server starts cleanly without an admin. `GET /api/auth/status` returns
`{"admin_exists": false, "setup_required": true}`, the dashboard redirects to
`/setup`, and the only writable routes until you create one are `/setup`,
`/login`, `/health`, and `/pair/claim`. Device tokens issued before the admin
exists still authenticate via `Authorization: Bearer …` because the auth
middleware tries cookie → bearer → loopback in that order.

### 4. Recovering a lost password

Both recovery commands are **loopback-only** (mirroring the existing TLS gate):

```sh
# On the server host:
cairn-server admin password   # reads CAIRN_ADMIN_PASSWORD or prompts; bumps generation
cairn-server admin reset      # deletes the admin; next /setup creates a new one
```

`admin password` bumps the `generation` counter on the persisted admin record,
which immediately invalidates every existing cookie session. `admin reset` writes
a tombstone sentinel (`__deleted__`) under the `admin` meta key — HelixDB's
append-only schema can't physically remove rows, so readers treat the tombstone
as absent. The next call to `/api/auth/setup` (with `Store::set_meta_if_absent`)
succeeds.

## Verifying the upgrade

```sh
cargo build --workspace
cargo test --workspace         # 113 lib tests pass; 5 ignored are the live HelixDB ones
cargo run -p cairn-server -- serve
# Visit http://localhost:7777 — should land on /setup (or /login if you set an admin)
```

The new web dashboard is committed prebuilt at `web/out/` so `cargo build` is
hermetic — no Node toolchain required.

## Removed

- The standalone landing page (`web/src/app/page.tsx` was a marketing pitch;
  replaced by a server-side redirect to `/dashboard`).
- The fallback `INDEX_HTML` no longer calls authed endpoints, so a fresh
  checkout with no `web/out/` build no longer produces a broken UI that shows
  "invalid or missing device token".

## Open items

- 2FA / TOTP is not yet implemented. The cookie payload schema leaves room for a
  second factor field without breaking existing sessions.
- Multi-admin isn't supported. Adding it requires an `AdminRecord` per user
  rather than a single record; tracked for a later release.
