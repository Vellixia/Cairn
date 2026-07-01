# 17 — Push Notifications: Subscribe, Unsubscribe, List

## Objective
Verify the push-subscription surface: `POST /api/push/subscribe` (idempotent on endpoint; first-UA-wins), `POST /api/push/unsubscribe` (`{id}` returns 204 No Content; no-op if absent), `GET /api/push/list`. Confirm the service worker (`web/public/sw.js`) registers `push` + `notificationclick` listeners and that a click on a notification navigates to `notification.data.url`.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] Browser at clean state with HTTPS or `localhost` (the service worker is registered only in those contexts per `web/src/app/(app)/layout.tsx:26-36`)
- [ ] The push store is on-disk at `<data_dir>/push/` and configured (the endpoints return 503 only when the store is absent; the walk assumes it is present)
- [ ] No leftover `PUSH-2026-07-01-*` endpoints from prior walks

## Surface
combined: API + browser

## Steps

### Step 1: GET /api/push/list (baseline)
**Do**: list existing subscriptions before any subscribe.
**Request**:
```http
GET /api/push/list HTTP/1.1
```
**Expected**:
- 200
- Body: `PushSubscriptionRecord[]` (empty on a fresh volume)
- Each record: `{id, endpoint, keys: {p256dh, auth}, user_agent, created_at, last_seen_at, last_event_id}`
- The endpoint is public-allow-listed (no cookie required)
**Observed**:
- HTTP status: ___
- Array length: ___
**Result**: PASS / FAIL

### Step 2: POST /api/push/subscribe — first subscription
**Do**: subscribe a synthetic endpoint with the standard VAPID-shape keys.
**Request**:
```http
POST /api/push/subscribe HTTP/1.1
Content-Type: application/json
{
  "endpoint": "https://fcm.googleapis.com/fcm/send/PUSH-2026-07-01-1",
  "keys": {
    "p256dh": "BNcRdreALRFXTkOOUHK1EtK2wtaz5Ry4YfYCA_0QTpQtUbVlUls0VJXg7A8u-Ts1XbjhazAkj7I2e0dyPckJBgc",
    "auth": "tBHItJI5svbpez7KI4CCXg"
  },
  "user_agent": "Mozilla/5.0 (PUSH-2026-07-01-walk)"
}
```
**Expected**:
- 200
- Body: `PushSubscriptionRecord{id, endpoint, keys, user_agent, created_at, last_seen_at, last_event_id}`
- Capture `id` for Steps 3 + 4
**Observed**:
- HTTP status: ___
- id: ___
**Result**: PASS / FAIL

### Step 3: POST /api/push/subscribe — idempotent re-subscribe (same endpoint, same UA)
**Do**: re-subscribe with the same endpoint and UA. The store must update the existing record (bump `last_seen_at`) and not insert a duplicate.
**Request**:
```http
POST /api/push/subscribe HTTP/1.1
Content-Type: application/json
{
  "endpoint": "https://fcm.googleapis.com/fcm/send/PUSH-2026-07-01-1",
  "keys": {"p256dh": "BNcRdreALRFXTkOOUHK1EtK2wtaz5Ry4YfYCA_0QTpQtUbVlUls0VJXg7A8u-Ts1XbjhazAkj7I2e0dyPckJBgc", "auth": "tBHItJI5svbpez7KI4CCXg"},
  "user_agent": "Mozilla/5.0 (PUSH-2026-07-01-walk)"
}
```
**Expected**:
- 200
- Body: `PushSubscriptionRecord{..., id matches Step 2's id}` (same record, not a new one)
- A subsequent `GET /api/push/list` still has exactly 1 entry for this endpoint
**Observed**:
- HTTP status: ___
- id matches: ___
- list count: ___
**Result**: PASS / FAIL

### Step 4: POST /api/push/subscribe — first-UA-wins
**Do**: re-subscribe the same endpoint with a different `user_agent`. The store keeps the **first** UA and updates only `last_seen_at`.
**Request**:
```http
POST /api/push/subscribe HTTP/1.1
Content-Type: application/json
{
  "endpoint": "https://fcm.googleapis.com/fcm/send/PUSH-2026-07-01-1",
  "keys": {"p256dh": "BNcRdreALRFXTkOOUHK1EtK2wtaz5Ry4YfYCA_0QTpQtUbVlUls0VJXg7A8u-Ts1XbjhazAkj7I2e0dyPckJBgc", "auth": "tBHItJI5svbpez7KI4CCXg"},
  "user_agent": "Mozilla/5.0 (PUSH-2026-07-01-second-ua)"
}
```
**Expected**:
- 200
- Body: `PushSubscriptionRecord{..., user_agent: "Mozilla/5.0 (PUSH-2026-07-01-walk)" (the FIRST UA, not the second)}`
- `last_seen_at` is updated to ~now
**Observed**:
- HTTP status: ___
- user_agent after: ___
- last_seen_at: ___
**Result**: PASS / FAIL

### Step 5: GET /api/push/list (post-subscribe)
**Do**: confirm the subscription is present with the right fields.
**Request**:
```http
GET /api/push/list HTTP/1.1
```
**Expected**:
- 200
- Array length == 1
- The single record matches the Step 4 response
**Observed**:
- HTTP status: ___
- Array length: ___
**Result**: PASS / FAIL

### Step 6: POST /api/push/subscribe — second distinct endpoint
**Do**: subscribe a second, distinct endpoint.
**Request**:
```http
POST /api/push/subscribe HTTP/1.1
Content-Type: application/json
{
  "endpoint": "https://updates.push.services.mozilla.com/wpush/v2/PUSH-2026-07-01-2",
  "keys": {"p256dh": "BNcRdreALRFXTkOOUHK1EtK2wtaz5Ry4YfYCA_0QTpQtUbVlUls0VJXg7A8u-Ts1XbjhazAkj7I2e0dyPckJBgc", "auth": "tBHItJI5svbpez7KI4CCXg"},
  "user_agent": "Mozilla/5.0 (PUSH-2026-07-01-second)"
}
```
**Expected**:
- 200
- Body: a new record with a different `id`
- `GET /api/push/list` now returns 2 entries
**Observed**:
- HTTP status: ___
- second id: ___
**Result**: PASS / FAIL

### Step 7: POST /api/push/unsubscribe — by id (204)
**Do**: unsubscribe the second subscription by id.
**Request**:
```http
POST /api/push/unsubscribe HTTP/1.1
Content-Type: application/json
{"id": "<id-from-step-6>"}
```
**Expected**:
- 204 No Content
- The subsequent `GET /api/push/list` no longer includes that id
**Observed**:
- HTTP status: ___
- List length: ___
**Result**: PASS / FAIL

### Step 8: POST /api/push/unsubscribe — absent id (204 no-op)
**Do**: try to unsubscribe an id that does not exist. Per `crates/cairn-api/src/push.rs:300-315`, the call is a no-op and returns 204.
**Request**:
```http
POST /api/push/unsubscribe HTTP/1.1
Content-Type: application/json
{"id": "00000000-0000-0000-0000-000000000000"}
```
**Expected**:
- 204
- No error
**Observed**:
- HTTP status: ___
**Result**: PASS / FAIL

### Step 9: Service worker registers
**Do**: open the dashboard `/` route in a fresh tab and watch for the service worker registration. The `web/public/sw.js` file registers `push` and `notificationclick` listeners.
**Expected**:
- 200 on the page
- The service worker registers (visible in DevTools > Application > Service Workers, or via `navigator.serviceWorker.controller` in the console)
- `list_console_messages types=["error"]` empty
**Observed**:
- Service worker registered: ___
- Controller available: ___
- Screenshot: `docs/live-e2e/screenshots/17-push/sw-registered.png`
**Result**: PASS / FAIL

### Step 10: Service worker — push event handler
**Do**: dispatch a synthetic `push` event to the registered service worker via DevTools. The handler at `web/public/sw.js` should call `registration.showNotification` with the payload's `title` and `body`.
**Request** (DevTools evaluate_script):
```js
async () => {
  const reg = await navigator.serviceWorker.ready;
  await reg.showNotification('PUSH-2026-07-01 title', {
    body: 'PUSH-2026-07-01 body',
    data: { url: '/memory?tab=wakeup&nocache=17-10' }
  });
  return 'shown';
}
```
**Expected**:
- A notification appears with the title and body from the payload
- The handler does not throw; the call resolves to `"shown"`
- `list_console_messages types=["error"]` empty
**Observed**:
- Notification appeared: ___
- Console errors: ___
- Screenshot: `docs/live-e2e/screenshots/17-push/notification-shown.png`
**Result**: PASS / FAIL

### Step 11: Service worker — notificationclick navigates to data.url
**Do**: simulate a click on the notification. The `notificationclick` listener navigates the focused client to `notification.data.url`.
**Request** (DevTools evaluate_script):
```js
async () => {
  const reg = await navigator.serviceWorker.ready;
  const notif = await reg.getNotifications();
  if (notif.length === 0) return 'no-notification';
  notif[0].click();
  return 'clicked';
}
```
**Expected**:
- The page navigates to `/memory?tab=wakeup&nocache=17-10`
- The previous URL is replaced in history
- `list_console_messages types=["error"]` empty
**Observed**:
- New URL: ___
- Console errors: ___
- Screenshot: `docs/live-e2e/screenshots/17-push/click-navigated.png`
**Result**: PASS / FAIL

## DB Verification
- The push store is on-disk at `<data_dir>/push/` (`crates/cairn-api/src/push.rs:64-86`). Use `/api/push/list` as the read proxy.
- After Step 2: list has 1 entry.
- After Step 3: list still has 1 entry (idempotent re-subscribe).
- After Step 4: the same record keeps the first UA; `last_seen_at` is updated.
- After Step 6: list has 2 entries.
- After Step 7: list has 1 entry (the second is removed).
- After Step 8: list still has 1 entry (no-op for absent id).

## UI Verification
- `/` registers the service worker without console errors.
- The synthetic `showNotification` call surfaces a real OS-level notification.
- The `notificationclick` handler navigates to `data.url`.
- `list_console_messages types=["error"]` empty across all three steps.

## Evidence
- Screenshots: `docs/live-e2e/screenshots/17-push/{sw-registered,notification-shown,click-navigated}.png`
- API responses for Steps 1-8
- The `id` captured in Step 2 + the matching id in Step 3 (idempotent re-subscribe)
- The UA before/after Step 4 (proves first-UA-wins)

## Known gaps
- The service worker `push` and `notificationclick` handlers are wired (`web/public/sw.js:1-119`), but no `cairn-server` code path actually delivers push notifications. The `/api/push/*` endpoints manage the subscription records only; there is no POST to a push provider (FCM / Mozilla autopush / web-push library) in the current build. The walk exercises the subscription lifecycle + the SW handlers; it does not exercise an end-to-end "server -> provider -> device" delivery. Not a P0 finding; documented here per the runbook.

## Findings
(none expected)
