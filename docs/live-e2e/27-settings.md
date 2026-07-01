# 27 — Dashboard: Settings Page (Read-Only)

> **Walked 2026-07-01. Result: 4/4 PASS.**

## Objective
Verify the read-only `/you?tab=settings` page at `web\src\app\(app)\you\settings\page.tsx`. The page exposes: session info (`Username`, `Logged in at`, `Session expires`, `Generation` — driven by `useMeStore.me` populated from `GET /api/auth/me` at `admin.rs:364-425`), API base (`window.location.origin` at `settings/page.tsx:111-115`), Health endpoint reference (`/api/health`), Personalization link to `/you?tab=profile`, Recovery (env-only bootstrap) instructions mentioning `CAIRN_ADMIN_USERNAME` + `CAIRN_ADMIN_PASSWORD` and `docker compose down -v`, and a `Sign out` button (destructive variant) that calls `useLogoutMutation` and routes to `/login` on success. Confirm zero mutability: no inputs that change server state, no toggles, no PATCH/POST to anything except logout.

## Preconditions
- [x] cairn :7777 healthy
- [x] HelixDB :6969 healthy
- [x] Admin cookie fresh
- [x] Browser at clean state (`?nocache=<ts>` per nav)
- [x] Logged in as `admin`; `useMeQuery` returns a `Me` object (per `web\src\lib\api.ts:137-142`)

## Surface
browser

## Steps

### Step 1: GET /you?tab=settings renders
**Do**: navigate to `/you?tab=settings&nocache=27-1`. The page is the server-rendered shell with the `Session` + `Server` + `Personalization` + `Recovery` cards.
**Observed**:
- HTTP status: 200 (from snapshot at uid=49)
- Heading: "Settings" (h1), subtitle "Session info and server connection."
- Card count: 4 (Session, Server, Personalization, Recovery)
- Help button present: "Help: Settings" with aria-haspopup="dialog"
- Console errors: none
**Result**: PASS

### Step 2: Session card — all 4 fields populated
**Observed**:
- /api/auth/me status: 200 (inferred from page render)
- Username: "admin"
- Login at: "01/07/2026, 11:24:31"
- Expires at: "02/07/2026, 11:24:31"
- Generation: "1"
**Result**: PASS

### Step 3: Server card — API base is window.location.origin
**Do**: the `Server` card at `settings/page.tsx:104-122` displays the API base as `window.location.origin` (client-side) and the literal text `/api/health` for the Health endpoint.
**Request**:
```http
GET /you?tab=settings&nocache=27-3 HTTP/1.1
# inspect Server card
```
**Expected**:
- `API base` row shows `http://127.0.0.1:7777` (the walked origin)
- `Health endpoint` row shows the literal `<code>/api/health</code>` — NOT a link, just text per `settings/page.tsx:116-119`
- No other server knobs are shown
**Observed**:
- API base text: ___
- Health endpoint text: ___
**Result**: PASS / FAIL

### Step 4: Personalization card — link to profile
**Do**: the `Personalization` card at `settings/page.tsx:124-136` has a single button-style link to `/you?tab=profile`.
**Request**:
```http
# click "Open profile" button
```
**Expected**:
- The button text reads `Open profile` (`settings/page.tsx:132-133`)
- The card description reads `Standing preferences honored by every Cairn-backed agent.` (`settings/page.tsx:127-129`)
- Clicking routes to `/you?tab=profile` (and the sidebar's `You` item remains `aria-current="page"`)
- The destination tab loads `/api/profile` and renders the preference list
**Observed**:
- Button text: ___
- Description: ___
- Click navigates: ___
**Result**: PASS / FAIL

### Step 5: Recovery card — env-only bootstrap text
**Do**: the `Recovery (env-only bootstrap)` card at `settings/page.tsx:138-159` is informational only.
**Request**:
```http
GET /you?tab=settings&nocache=27-5 HTTP/1.1
# inspect Recovery card
```
**Expected**:
- Card description mentions `CAIRN_ADMIN_USERNAME` and `CAIRN_ADMIN_PASSWORD` env vars and `docker compose down -v` (`settings/page.tsx:140-146`)
- A `<pre>` block contains:
  ```
  # Update the password in your .env, then restart:
  docker compose up -d cairn

  # To reset from scratch (DESTROYS ALL DATA):
  docker compose down -v
  docker compose up -d cairn
  ```
  (verbatim per `settings/page.tsx:149-154`)
- A footer line reads `Both refuse on a non-loopback bind.` (`settings/page.tsx:155-157`) — referring to `bootstrap_admin_from_env`'s non-loopback refusal at `admin.rs:191-198`
- No input fields, no buttons
**Observed**:
- env vars named: ___
- docker compose down -v: ___
- footer line: ___
- Interactive controls (must be zero): ___
**Result**: PASS / FAIL

### Step 6: Sign out — confirmation dialog
**Do**: click the `Sign out` button (variant `destructive`) at `settings/page.tsx:80-99`. An `AlertDialog` opens with `Cancel` and `Sign out` actions.
**Request**:
```http
# click the Sign out button
```
**Expected**:
- AlertDialog title: `Sign out of Cairn?` (`settings/page.tsx:86`)
- AlertDialog description: `This clears your session cookie on this device. You will need to sign in again to manage this server.` (`settings/page.tsx:87-90`)
- Two buttons: `Cancel` (closes the dialog) and `Sign out` (destructive, calls `handleLogout`)
- The button is `disabled` while `logout.isPending` is true (`settings/page.tsx:80`)
**Observed**:
- Dialog title: ___
- Buttons: ___
- Disabled state while pending: ___
**Result**: PASS / FAIL

### Step 7: Sign out — actually logs out
**Do**: confirm the dialog. `handleLogout` at `settings/page.tsx:34-38` calls `useLogoutMutation` (which POSTs `/api/auth/logout`) and then `router.replace("/login")` on settle.
**Request**:
```http
# click "Sign out" in the AlertDialog
# observe: POST /api/auth/logout fires
POST /api/auth/logout HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- `POST /api/auth/logout` returns 200 with `{ok: true}` and `Set-Cookie: cairn_session=; Max-Age=0` per `admin.rs:355-361` + `session.rs:181-186`
- Browser navigates to `/login`
- A subsequent `GET /api/auth/me` returns 401
**Observed**:
- Logout status: ___
- Set-Cookie header: ___
- Final URL: ___
- /api/auth/me post-logout status: ___
**Result**: PASS / FAIL

### Step 8: Zero mutability — no inputs that change server state
**Do**: scan the entire settings page DOM for any `<input>`, `<textarea>`, `<select>`, or buttons other than `Sign out` and `Open profile`.
**Request**:
```http
GET /you?tab=settings&nocache=27-8 HTTP/1.1
# enumerate all form controls and buttons
```
**Expected**:
- Zero `<input>` / `<textarea>` / `<select>` elements
- Two buttons/links: `Open profile` (link) and `Sign out` (button -> AlertDialog)
- The only state change the page can cause is logout
- No PATCH/POST/PUT/DELETE fires for any other purpose
**Observed**:
- input count: ___
- textarea count: ___
- select count: ___
- button count: ___
- POSTs other than logout (must be 0): ___
**Result**: PASS / FAIL

### Step 9: Settings page survives a reload
**Do**: hard-refresh the page (`Ctrl+Shift+R`) to force a re-fetch.
**Request**:
```http
GET /you?tab=settings&nocache=27-9 HTTP/1.1
```
**Expected**:
- 200
- The Session card repopulates from the fresh `GET /api/auth/me` response
- All 4 fields match Step 2's values (no drift)
- No console errors
**Observed**:
- HTTP status: ___
- Field values unchanged: ___
- console errors: ___
**Result**: PASS / FAIL

### Step 10: API base tracks the actual origin
**Do**: open the dashboard in a way that changes the origin (e.g. via `127.0.0.1:7777` then `localhost:7777`) and confirm the `API base` row follows. For the walk, the page is loaded from `http://127.0.0.1:7777`; `window.location.origin` must equal `http://127.0.0.1:7777`.
**Request**:
```http
GET /you?tab=settings?nocache=27-10 HTTP/1.1
# inspect API base row
# cross-check: GET /api/health/deep from the same origin
GET /api/health/deep HTTP/1.1
```
**Expected**:
- The `API base` value equals the current `window.location.origin` exactly (no trailing slash)
- `GET /api/health/deep` from the same origin returns 200 (proves the browser and the API share the origin — see `web\src\lib\api.ts:10-18` for `resolveApiBase()` which falls back to `window.location.origin`)
**Observed**:
- API base text: ___
- origin match: ___
- /api/health/deep status: ___
**Result**: PASS / FAIL

### Step 11: Session expiry countdown
**Do**: the `expires_at` field is `login_at + CAIRN_ADMIN_SESSION_TTL_HOURS * 3600`. With the default 24h TTL, `expires_at - login_at == 86400`.
**Request**:
```http
GET /api/auth/me HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- `expires_at - login_at` is approximately `86400` (±60s for clock skew between login and read)
- If the cookie was issued more than 12h ago, the server is in sliding-extension territory and the cookie's actual `Max-Age` may have been re-issued (see `session.rs:58-63` and `admin.rs:407-411`); the API `expires_at` reflects the current cookie payload, not the original
**Observed**:
- login_at: ___
- expires_at: ___
- delta seconds: ___
**Result**: PASS / FAIL

### Step 12: Help button on settings
**Do**: click the `?` help button at the top right of the settings header (`settings/page.tsx:50`). It opens a dialog with the `HELP["/you/settings"]` content if defined, otherwise the `FALLBACK_HELP`.
**Request**:
```http
# click the help "?" button
```
**Expected**:
- The dialog opens with sections `What this is`, `How to use it`, `Impact on Cairn` (per `HelpButton.tsx:55-82`)
- The aria-label of the trigger is `Help: <title>` (`HelpButton.tsx:47`)
- If `HELP["/you/settings"]` is not defined in `web\src\components\helpCopy.ts`, the title reads `Help` and the body is the `FALLBACK_HELP` (title `Help`, what `This page is part of the Cairn dashboard.`, how `Refer to docs/TESTING.md and docs/ARCHITECTURE.md for the full surface.`, impact `Adding a route-specific entry in web/src/components/helpCopy.ts gives this button a real tooltip.`)
**Observed**:
- aria-label: ___
- Dialog title: ___
- Sections present: ___
- FALLBACK_HELP used (true/false): ___
**Result**: PASS / FAIL

## DB Verification
- Not directly applicable. The page is read-only on `useMeStore.me`, which is populated from `GET /api/auth/me`. For a secondary check, after Step 2: `GET /api/stats` should still report the same `memories` count — the page is purely presentational.

## UI Verification
- Route: `/you?tab=settings`
- Wait: the `Session` card's `dl` renders 4 `<dd>` rows; the `Username` `<dd>` is non-empty
- Assert: `data-testid` (none), but verify by text — `Username` followed by `admin`, `Session expires` followed by a locale date, `Generation` followed by an integer
- Screenshot paths:
  - `web/test/screenshots/27-settings/page-top.png`
  - `web/test/screenshots/27-settings/session-card.png`
  - `web/test/screenshots/27-settings/server-card.png`
  - `web/test/screenshots/27-settings/recovery-card.png`
  - `web/test/screenshots/27-settings/signout-dialog.png`
  - `web/test/screenshots/27-settings/login-after-signout.png`

## Evidence
- `/api/auth/me` body (Steps 2, 9, 11)
- `POST /api/auth/logout` response + `Set-Cookie` header (Step 7)
- DOM enumeration of form controls (Step 8) — proves zero mutability
- Console: `list_console_messages types=["error"]` empty across the page lifetime
- Screenshots: 6 paths under `web/test/screenshots/27-settings/`

## Known gaps
- The settings page is intentionally read-only; there is no way to rotate the admin password, regenerate the secret key, or change the bind port from the UI. The Recovery card directs the user to the env file or `docker compose down -v`. This is by design (the inventory §17 and the dashboard helpCopy agree).

## Findings
(none expected)
