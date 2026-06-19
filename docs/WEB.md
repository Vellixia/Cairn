# Web dashboard (0.4.0)

The Cairn web dashboard is a single-admin console: one username + password,
one httpOnly cookie session. CLI / MCP clients authenticate with **device
tokens** (HS256 JWTs) issued by the admin from the **Devices** panel.

```
┌──────────────────────────────────────────────────────────────────────────┐
│  Browser                        Cairn server                              │
│  ┌───────────────┐  cookie       ┌─────────────────────────┐              │
│  │  /login       │ ───────────►  │  POST /api/auth/login    │              │
│  │  /dashboard   │  cairn_       │  POST /api/auth/logout   │              │
│  │  /setup       │  session      │  GET  /api/auth/me       │              │
│  │  /setup       │ ◄───────────  │  POST /api/auth/setup    │              │
│  └───────────────┘               └─────────────────────────┘              │
│                                                                          │
│  ┌───────────────┐  bearer       ┌─────────────────────────┐              │
│  │  cairn-cli    │ ───────────►  │  any /api/*              │              │
│  │  cairn-mcp    │  JWT in       │  Authorization: Bearer  │              │
│  │  agent        │  Authorization│  ...                     │              │
│  └───────────────┘  header       └─────────────────────────┘              │
└──────────────────────────────────────────────────────────────────────────┘
```

## Auth surface

### Cookie (web)

| Endpoint | Method | Auth | Purpose |
|---|---|---|---|
| `/api/auth/status` | GET | public | `{ admin_exists, setup_required }` |
| `/api/auth/setup` | POST | public, CAS | first-run wizard |
| `/api/auth/login` | POST | public, rate-limited 5/min | username + password → cookie |
| `/api/auth/logout` | POST | idempotent | clears cookie |
| `/api/auth/me` | GET | cookie | current session info; sliding TTL extension |

Cookie attributes: `HttpOnly; SameSite=Strict; Path=/; Max-Age=86400` (default),
`Secure` when TLS is configured.

### Bearer (CLI / MCP)

Device tokens (HS256 JWTs) — the existing 0.3.x flow. Tokens carry `scope`
(admin / write / read) and an optional `exp`. Token id is stored in the meta
store; the bearer itself is never persisted in cleartext.

### `auth()` middleware composition

Every request goes through `auth()` (in `crates/cairn-api/src/lib.rs`):

1. Public endpoints (`/api/health`, `/api/pair/claim`, the admin auth surface) — pass.
2. Admin cookie — if `cairn_session` is signed and the embedded generation
   matches the live admin record, the request is treated as the admin.
3. Device-token bearer — the existing JWT path; respected when no admin
   cookie is present.
4. Loopback fallback — only when there are zero device tokens AND no admin
   (first-run before `/setup`). Lets the operator visit `/setup` on localhost.

### Rate limits

| Endpoint | Limit | Why |
|---|---|---|
| `/api/auth/login` | 5/min/IP | brute-force defense |
| `/api/auth/setup` | 3/min/IP | first-run is rare |
| `/api/pair/claim` | 5/min/IP | brute-force defense |
| everything else | 60/min/IP | existing global default |

## Dashboard surface

### Layout

- Left rail: sectioned sidebar (Server / Memory / Context / Reliability /
  Share / Devices). `aria-current="page"` on the active item.
- Top: ⌘K trigger + server health pill + reliability score + profile chip.
- Center: per-section routes (Next.js App Router).
- Right-bottom: toast tray with `aria-live="polite"` and `role="alert"` for
  errors.

### Keyboard

| Keys | Action |
|---|---|
| ⌘K / Ctrl+K | Toggle command palette (cmdk) |
| ? | Toggle keyboard shortcuts modal |
| esc | Close any open dialog |

### Section routes

| Path | Purpose |
|---|---|
| `/dashboard` | Overview (signal-dense) |
| `/dashboard/settings` | Session info, sign out |
| `/dashboard/memory` | Remember (write) |
| `/dashboard/memory/recall` | Search (BM25 + semantic) |
| `/dashboard/memory/wakeup` | High-importance memories |
| `/dashboard/context` | File inspector (read modes + expand) |
| `/dashboard/context/assemble` | Token-budget assembly |
| `/dashboard/reliability` | Edit-guard score |
| `/dashboard/reliability/anchor` | Task anchor (set/update) |
| `/dashboard/reliability/checkpoints` | Snapshot + rollback |
| `/dashboard/share/sanitize` | Redact secrets + classify |
| `/dashboard/share/export` | Build sanitized bundle |
| `/dashboard/pool` | Pool + federate |
| `/dashboard/devices` | **Admin: issue / list / revoke device tokens** |
| `/dashboard/devices/pair` | **Admin: generate pairing code** |
| `/dashboard/devices/audit` | **Admin: last 50 audit events** |

### Admin actions in the UI

The admin can do everything the CLI could, from the dashboard:

- **Issue a device token**: pick name + scope (admin/write/read) + optional
  expiry. Server signs the JWT, returns it once in the response, and stores
  only the metadata.
- **Revoke a device token**: marks the id revoked; future bearer calls 401.
- **Generate a pairing code**: short 8-char code, TTL 1–60 min (default 10).
  Same-store pattern as `cairn pair-code` — the claim endpoint signs a fresh
  JWT at claim time.

## Prebuilt static export

`web/out/` is committed so `cargo build` is hermetic — no Node toolchain
required to ship the cairn binary. The CI `web:build` job runs
`npm ci && npm run build` and uploads the result as a build artifact for
cross-validation.

To rebuild the dashboard from source:

```sh
cd web
npm ci
npm run build          # writes web/out/
```

Source maps under `_next/static/**/*.js.map` are excluded from git to keep
the repo lighter.

## Security headers

The dashboard adds four headers to every response, including 401s from `auth()`:

```
X-Frame-Options: DENY
X-Content-Type-Options: nosniff
Referrer-Policy: no-referrer
Permissions-Policy: clipboard-write=(self)
```

CSP is intentionally not added yet — the static fallback HTML embeds inline
`<style>` and a tiny inline `<script>`. A future iteration that ships the
dashboard prebuilt can adopt per-response nonce CSP.

## CORS

| Scenario | Behavior |
|---|---|
| Same-origin (most common) | Browser default — no CORS headers needed |
| `CAIRN_CORS_ORIGINS` empty | Same-origin only |
| `CAIRN_CORS_ORIGINS=https://app.example.com,https://admin.example.com` | Specific origins echoed (with credentials) |
| `CAIRN_CORS_ORIGINS=*` | **Refused** with a logged warning — auth surface area never permits wildcard credentials |
