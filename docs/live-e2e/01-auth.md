# 01 — Auth: first-run, login, logout, me, rate limit

## Objective
Verify the auth surface: status probe, login mints a session cookie, /me reads session info, logout clears the cookie, rate limiter fires at 5 fails/min, and the /login + /setup/wizard pages render without errors.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh (`%TEMP%\opencode\walk-cookies.txt`)
- [ ] Browser at clean state (`?nocache=<ts>` per nav)
- [ ] Admin exists (env bootstrap, no /setup needed)

## Surface
combined: API + browser

## Steps

### Step 1: GET /api/auth/status
**Do**: probe whether admin exists and whether setup is required.
**Request**:
```http
GET /api/auth/status HTTP/1.1
```
**Expected**:
- 200
- Body: `{"admin_exists": true, "setup_required": false}`
**Observed**:
- HTTP status: ___
- Body: ___
**Result**: PASS / FAIL

### Step 2: GET /api/auth/status (no cookie)
**Do**: open browser to `/login?nocache=01-2`
**Expected**:
- 200
- Snapshot shows the username + password form
- Snapshot shows no error banner
- `list_console_messages types=["error"]` is empty
**Observed**:
- Snapshot ref: ___
- Console errors: ___
- Screenshot: `docs/live-e2e/screenshots/01-auth/login.png`
**Result**: PASS / FAIL

### Step 3: POST /api/auth/login
**Do**: POST credentials with curl, save the cookie.
**Request**:
```http
POST /api/auth/login HTTP/1.1
Content-Type: application/json
{"username":"admin","password":"AuditPass2026!"}
```
**Expected**:
- 200
- Set-Cookie: `cairn_session=<jwt>; HttpOnly; SameSite=Strict`
- Body: `{"expires_at": <unix-ts>, "username": "admin"}`
- Audit kind: `login_ok`
**Observed**:
- HTTP status: ___
- Cookie name + value (redacted): ___
- Audit log entry: ___
**Result**: PASS / FAIL

### Step 4: GET /api/auth/me
**Do**: with the cookie, call /me.
**Request**:
```http
GET /api/auth/me HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{"username": "admin", "generation": 1, "login_at": <ts>, "expires_at": <ts>}`
- generation === 1 (first login)
**Observed**:
- HTTP status: ___
- Body: ___
- generation: ___
**Result**: PASS / FAIL

### Step 5: Browser — navigate to / with valid cookie
**Do**: open `/ ?nocache=01-5` in browser
**Expected**:
- 200
- Topbar shows "signed in as admin"
- Sidebar shows all 5 hubs (Now, Memory, Trust, Registry, You)
- No redirect to /login
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Sidebar hubs visible: ___
- Screenshot: `docs/live-e2e/screenshots/01-auth/overview.png`
**Result**: PASS / FAIL

### Step 6: POST /api/auth/logout
**Do**: POST /api/auth/logout with cookie.
**Request**:
```http
POST /api/auth/logout HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Set-Cookie: `cairn_session=; Max-Age=0`
- Body: `{"ok": true}`
**Observed**:
- HTTP status: ___
- Set-Cookie: ___
- Body: ___
**Result**: PASS / FAIL

### Step 7: GET /api/auth/me (post-logout)
**Do**: call /me with the (now-invalid) cookie.
**Request**:
```http
GET /api/auth/me HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 401
- Error envelope: `{"error": "unauthenticated", "error_code": "unauthenticated"}`
**Observed**:
- HTTP status: ___
- Error code: ___
**Result**: PASS / FAIL

### Step 8: Rate limit — 5 failed logins
**Do**: POST /api/auth/login with a bad password 6 times in a row, in a tight loop.
**Request** (6x):
```http
POST /api/auth/login HTTP/1.1
Content-Type: application/json
{"username":"admin","password":"wrong-password"}
```
**Expected**:
- First 5 attempts: 401 with `error_code: unauthenticated`
- 6th attempt: 429 (rate limited)
- Audit log has 6 `login_failed` entries: 5 with `detail: "bad password"`, 1 with whatever the rate-limit denial records
**Observed**:
- First 401 count: ___
- 429 received on attempt #: ___
- Audit log entries: ___
**Result**: PASS / FAIL

### Step 9: Browser — /login accepts successful login
**Do**: re-login via the browser form on `/login?nocache=01-9`, fill admin / AuditPass2026!, submit.
**Expected**:
- Form submits, page redirects to /
- Topbar shows "signed in as admin"
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Screenshot: `docs/live-e2e/screenshots/01-auth/post-login.png`
**Result**: PASS / FAIL

### Step 10: Browser — /setup/wizard renders
**Do**: navigate to `/setup/wizard?nocache=01-10`
**Expected**:
- 200 (or 302 to /setup if admin exists; depends on whether the wizard short-circuits)
- If 200: snapshot shows a 4-step wizard (creds → embed provider → optional pair → health)
- `list_console_messages types=["error"]` empty
**Observed**:
- HTTP status / redirect: ___
- Wizard step labels visible: ___
- Screenshot: `docs/live-e2e/screenshots/01-auth/setup-wizard.png`
**Result**: PASS / FAIL

## DB Verification
- N/A (no DB writes; auth state is the in-memory admin + audit log).
- Confirm via `GET /api/devices/audit` (auth surface) that `login_ok` / `login_failed` / `setup` entries appear with the expected `detail` strings. The `setup` kind will only appear once per fresh volume.

## UI Verification
- `/login` shows username + password form, no error banner.
- `/` (post-login) shows topbar with username, sidebar with 5 hubs, "Recent memory" panel populated.
- `/setup/wizard` renders a multi-step form (creds → embed provider → optional pair → health).
- `list_console_messages types=["error"]` empty on all three pages.

## Evidence
- Screenshots: `docs/live-e2e/screenshots/01-auth/login.png`, `overview.png`, `post-login.png`, `setup-wizard.png`
- Audit log dump from `/api/devices/audit`
- Network: capture `POST /api/auth/login` and `POST /api/auth/logout` for status codes

## Findings
(none expected)
