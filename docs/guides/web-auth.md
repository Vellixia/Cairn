---
title: "Web dashboard (v0.5.0)"
type: guide
status: living
updated: 2026-07-01
---

# Web dashboard (v0.5.0)

The Cairn web dashboard is a single-admin console: one username + password,
one httpOnly cookie session. CLI / MCP clients authenticate with **device
tokens** (HS256 JWTs) issued by the admin from the **Devices** panel.

```
""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""
"  Browser                        Cairn server                              "
"  """""""""""""""""  cookie       """""""""""""""""""""""""""              "
"  "  /login       " """""""""""--  "  POST /api/auth/login    "              "
"  "  /dashboard   "  cairn_       "  POST /api/auth/logout   "              "
"  "  /setup       "  session      "  GET  /api/auth/me       "              "
"  "  /setup       " -"""""""""""  "  POST /api/auth/setup    "              "
"  """"""""""""""""""               """"""""""""""""""""""""""""              "
"                                                                          "
"  """""""""""""""""  bearer       """""""""""""""""""""""""""              "
"  "  cairn    " """""""""""--  "  any /api/*              "              "
"  "  cairn-mcp    "  JWT in       "  Authorization: Bearer  "              "
"  "  agent        "  Authorization"  ...                     "              "
"  """"""""""""""""""  header       """"""""""""""""""""""""""""              "
"""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""""
```

## Auth surface

### Cookie (web)

| Endpoint | Method | Auth | Purpose |
|---|---|---|---|
| `/api/auth/status` | GET | public | `{ admin_exists, setup_required }` |
| `/api/auth/setup` | POST | public, CAS | first-run wizard |
| `/api/auth/login` | POST | public | username + password -> cookie |
| `/api/auth/logout` | POST | idempotent | clears cookie |
| `/api/auth/me` | GET | cookie | current session info; sliding TTL extension |

Cookie attributes: `HttpOnly; SameSite=Strict; Path=/; Max-Age=86400` (default),
`Secure` when TLS is configured.

### Bearer (CLI / MCP)

Device tokens (HS256 JWTs) - unchanged since 0.4.0. Tokens carry `scope`
(admin / write / read) and an optional `exp`. Token id is stored in the meta
store; the bearer itself is never persisted in cleartext.

### `auth()` middleware composition

Every request goes through `auth()` (in `crates/cairn-api/src/lib.rs`):

1. Public endpoints (`/api/health`, the admin auth surface) - pass through
   unchanged.
2. Admin cookie - if `cairn_session` is present, signed, and the embedded
   generation matches the live admin record, the request is treated as the
   admin (all scopes).
3. Device-token bearer - the existing JWT path; respected when no admin
   cookie is present, so CLI / MCP clients keep working. A token whose
   scope doesn't cover the requested method/path gets a distinct
   `InsufficientScope` outcome (403 `forbidden`), separate from a
   missing/invalid token (401 `unauthenticated`).
4. Loopback fallback - only when **all three** of the following hold at
   once: zero device tokens exist, no admin record exists yet (first-run,
   before `/setup`), and the request's source address is loopback. Any one
   of those three conditions failing closes the fallback - for example, an
   admin visiting from a non-loopback address after tokens exist gets a
   401, not silent access. The admin-cookie path supersedes this fallback
   once `/setup` has been completed.

`/api/devices/*` (token issuance, revocation, and the audit log) is
**cookie-only by design** - it checks the `cairn_session` cookie and
nothing else. A bearer token, even one scoped `admin`, cannot authenticate
against these endpoints; only an active dashboard browser session can.

## Dashboard surface

### Layout (Sprint 27)

- Left rail: **flat** sidebar with 4 entries - **Now** (static label) /
  Memory / Trust / You. State persists per-browser in `localStorage` under
  the key `cairn-sidebar-v3`. On mount, the page removes the legacy keys
  `cairn-sidebar-v1`, `cairn-sidebar-v2`, and `cairn-infocard-dismissed-v1`
  from `localStorage` and `sessionStorage`. `aria-current="page"` on the
  active item, matched by path only (query is ignored so `?tab=<sub>` does
  not retrigger selection).
- Hubs render as **single pages** at flat URLs: `/memory`, `/trust`, `/you`.
  Tabs are surfaced via `?tab=<sub>`. The 21 deep-link routes
  (`/memory/recall`, `/trust/score`, etc.) still render their single sub-page
  for direct linking - they do not show the hub shell.
- Top: K trigger + server health pill + reliability score + profile chip.
- Center: per-hub tabs on `md`; collapses to a `<select>` on `<md` for
  mobile fallback.
- Right-bottom: toast tray with `aria-live="polite"` and `role="alert"` for
  errors.

#### HelpDialog pattern (Sprint 27)

Each of the 22 page routes shows a compact `?` icon in its page header that
opens a shadcn Dialog with the page's help copy (keyed by route in
`web/src/components/helpCopy.ts`). The dialog has three blocks:

- **What this is** - one-sentence purpose.
- **How to use it** - 1-3 bullets of the smallest action that works.
- **Impact on Cairn** - concrete downstream effect (tokens, reliability,
  memory, savings).

The trigger button carries `aria-label="Help: <title>"` for screen readers.
Help copy is route-keyed (not session-dismissible) so deep-links always
show the same content. The previous inline `InfoCard` pattern (Sprint 26)
was retired in Sprint 27 because it visually overwhelmed the dashboard and
landed inside nested components (the Trust Score page ended up with 11
nested cards).

### Overview page (`/` -> Now)

Signal-dense landing page composed of:

1. **KPI hero** - 4 cards: Memories, Reliability, Token savings, Active
   devices. Tones follow semantic color tokens (`positive` / `warning` /
   `danger` / `info` / `neutral`).
2. **HealthRow** - 5 status pills (Server, Helix, Embedder, Reliability, PWA)
   refetched every 30 s. Backed by existing `/api/health`,
   `/api/setup/health`, `/api/stats` - no new backend.
3. **ActivityTimeline** - last 8 audit events from `/api/devices/audit`.
4. **SavingsChart** - 7-day rolling Recharts AreaChart of
   `wakeup_tokens + recall_tokens` from `/api/metrics`. Empty state with
   `PiggyBank` icon when ledger is empty.
5. **DriftAnchorCard** - current task anchor (read + edit) + reliability
   summary + link to the drift center.
6. **TokensSavedHeadline** - `saved_bytes` from `/api/metrics` rendered as a
   large number with an arrow delta vs the prior 7 days (computed
   client-side from `/api/ledger?limit=1000`).
7. **ReliabilitySparkline** - 30-sample savings sparkline (Recharts
   `LineChart`) over the last 30 minutes, normalized to 0-100 across the
   visible window.
8. **MemoryTierDonut** - Recharts `PieChart` grouped by `tier` from
   `/api/memory/wakeup?limit=200`.
9. **SourceMixBar** - plain-CSS horizontal stacked bar over the last 7
   days, grouped by `source` from `/api/ledger?limit=500` (no Recharts,
   keeps the bundle lean).
10. **LastAdminActionCard** - newest entry from `/api/devices/audit`, with
    actor, kind, detail, and relative time.
11. **Recent memory** - last 5 wakeup memories from `/api/memory/wakeup`.

### Keyboard

| Keys | Action |
|---|---|
| K / Ctrl+K | Toggle command palette (cmdk) |
| ? | Toggle keyboard shortcuts modal |
| esc | Close any open dialog |

### Section routes

| Path | Purpose |
|---|---|
| `/` -> `/memory` | Overview (Now) |
| `/memory` | Memory hub: remember / recall / wakeup / graph / inspector / assemble / savings |
| `/memory/recall` | Search (BM25 + semantic) |
| `/memory/wakeup` | High-importance memories |
| `/memory/assemble` | Token-budget assembly |
| `/memory/inspector` | File inspector (read modes + expand) |
| `/trust` | Trust hub: score / anchor / checkpoints / drift / sanitize / registry / pool |
| `/trust/score` | Reliability score |
| `/trust/anchor` | Task anchor (set/update) |
| `/trust/checkpoints` | Snapshot + rollback |
| `/trust/drift` | Drift center |
| `/trust/sanitize` | Redact secrets + classify |
| `/you` | You hub: profile / tokens / audit / sessions / settings |
| `/you/tokens` | **Admin: issue / list / revoke device tokens** |
| `/you/audit` | **Admin: last 50 audit events** |
| `/you/sessions` | Per-session detail list |
| `/you/settings` | Session info, sign out |

### Admin actions in the UI

The admin can do everything the CLI could, from the dashboard:

- **Issue a device token**: pick name + scope (admin/write/read) + optional
  expiry. Server signs the JWT, returns it once in the response, and stores
  only the metadata.
- **Revoke a device token**: marks the id revoked; future bearer calls 401.

## Static export

`web/out/` is **gitignored** (no `.gitkeep` in the repo). The `build.rs` in
`cairn-api` creates the directory at compile time if missing so `cargo build`
is hermetic - no Node toolchain required. The Docker build runs
`npm run build` before compiling Rust so the container ships the full
dashboard.

To rebuild the dashboard from source:

```sh
cd web
npm ci
npm run build          # writes web/out/
```

`web/out/` is fully gitignored. Build artifacts are never committed.

## Security headers

The dashboard adds four headers to every response, including 401s from `auth()`:

```
X-Frame-Options: DENY
X-Content-Type-Options: nosniff
Referrer-Policy: no-referrer
Permissions-Policy: clipboard-write=(self)
```

CSP is intentionally not added yet - the static fallback HTML embeds inline
`<style>` and a tiny inline `<script>`. A future iteration that ships the
dashboard prebuilt can adopt per-response nonce CSP.

## CORS

| Scenario | Behavior |
|---|---|
| Same-origin (most common) | Browser default - no CORS headers needed |
| `CAIRN_CORS_ORIGINS` empty | Same-origin only |
| `CAIRN_CORS_ORIGINS=https://app.example.com,https://admin.example.com` | Specific origins echoed (with credentials) |
| `CAIRN_CORS_ORIGINS=*` | **Refused** with a logged warning - auth surface area never permits wildcard credentials |
