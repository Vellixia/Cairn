# 26 — Dashboard: Command Palette, Shortcuts, Sidebar, Topbar

## Objective
Verify the dashboard shell chrome at the `(app)` layout. Cover: command palette (24 items across 4 groups: Navigate, Memory, Devices, Personalization — `web\src\components\CommandPalette.tsx:72-101`), 3 global shortcuts (`⌘K`/`Ctrl+K` toggles palette; `?` toggles shortcuts modal; `esc` closes any open dialog — `web\src\components\Shortcuts.tsx:14-18`), sidebar (5 hubs: Now / Memory / Trust / Registry / You — `web\src\components\Sidebar.tsx:29-35`, active-route via path-prefix match at `Sidebar.tsx:39-46`), topbar (palette button at `Topbar.tsx:42-51`, mobile button at `Topbar.tsx:54-61`, health pill polling `/api/health` every 15s at `Topbar.tsx:64-74` -> `useHealthQuery` refetch 15s in `web\src\lib\queries.ts` registry per inventory §5), account avatar dropdown (Settings / Audit log / Sign out — `Topbar.tsx:81-111`), and the help button + `FALLBACK_HELP` constant at `web\src\components\HelpButton.tsx:22-27`.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] Browser at clean state (`?nocache=<ts>` per nav)
- [ ] Service worker not registered (or registered cleanly on first paint per `web\src\app\(app)\layout.tsx:25-37`) — confirm no console errors
- [ ] At least one memory exists (so the wakeup card on `/` is non-empty)

## Surface
browser

## Steps

### Step 1: Sidebar renders with 5 hubs
**Do**: navigate to `/?nocache=26-1`. Wait for the sidebar to hydrate (`Sidebar.tsx:81-101` migration effect runs once on mount). Take a snapshot and verify the 5 hub labels are present.
**Request**:
```http
GET / HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Snapshot shows 5 sidebar items in this order: `Now`, `Memory`, `Trust`, `Registry`, `You`
- The `Now` item has `aria-current="page"` (active because `pathname === "/"` matches `Sidebar.tsx:42-44`)
- No `aria-current` on the other 4
- `list_console_messages types=["error"]` empty
**Observed**:
- HTTP status: ___
- Items: ___
- aria-current target: ___
**Result**: PASS / FAIL

### Step 2: Sidebar active-route highlight on Memory hub
**Do**: navigate to `/memory?nocache=26-2`. The `isActive` predicate at `Sidebar.tsx:39-46` strips `?` from each item href and matches on path prefix.
**Request**:
```http
GET /memory?nocache=26-2 HTTP/1.1
```
**Expected**:
- 200
- `Memory` item has `aria-current="page"`
- `Now` no longer has `aria-current`
- `Trust` / `Registry` / `You` no longer have `aria-current`
**Observed**:
- Memory aria-current: ___
- Now aria-current: ___
**Result**: PASS / FAIL

### Step 3: Sidebar active-route highlight on a nested tab
**Do**: navigate to `/memory?tab=graph&nocache=26-3`. The `isActive` split is on `?`, so `"/memory"` matches the `Memory` item even with a query string.
**Request**:
```http
GET /memory?tab=graph&nocache=26-3 HTTP/1.1
```
**Expected**:
- 200
- `Memory` item still has `aria-current="page"`
- A nested tab change does not break the highlight
**Observed**:
- Memory aria-current: ___
- Screenshot: ___
**Result**: PASS / FAIL

### Step 4: ⌘K / Ctrl+K toggles the command palette
**Do**: from any dashboard page, press `Ctrl+K` (the binding is `(e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k"` at `CommandPalette.tsx:56-61`). The `useUIStore` `commandOpen` boolean flips.
**Request**:
```http
GET / HTTP/1.1
# then in browser: keyboard.down Control ; keyboard.press KeyK ; keyboard.up Control
```
**Expected**:
- The `CommandDialog` mounts (root has `cmdk-dialog` / Radix `data-state=open`)
- The input is focused; the placeholder reads `Jump to a section, run an action...` (per `CommandPalette.tsx:105`)
- The footer reads `^v navigate . ↵ select . esc close` (`CommandPalette.tsx:135-139`) plus a `?` shortcuts link
**Observed**:
- Dialog open: ___
- Placeholder: ___
- Footer text: ___
**Result**: PASS / FAIL

### Step 5: ? toggles the shortcuts modal
**Do**: press `?` (the `?` key, no modifier — `Shortcuts.tsx:24-30`). The shortcuts dialog opens.
**Request**:
```http
GET /?nocache=26-5 HTTP/1.1
# in browser: keyboard.press "?"
```
**Expected**:
- 200 (page is loaded; modal mounts client-side)
- Modal title reads `Keyboard shortcuts` (`Shortcuts.tsx:39`)
- Modal description: `Quick navigation across the Cairn dashboard.` (`Shortcuts.tsx:41`)
- The list shows exactly 3 shortcuts in this order: `⌘K / Ctrl+K` — `Toggle the command palette`; `?` — `Toggle this shortcuts modal`; `esc` — `Close any open dialog`
**Observed**:
- Title: ___
- Shortcut count: ___
- Keys: ___
**Result**: PASS / FAIL

### Step 6: esc closes any open dialog
**Do**: open the palette (Step 4) and the shortcuts modal (Step 5) back-to-back. Press `esc` once and assert the topmost dialog closes.
**Request**:
```http
# open palette via Ctrl+K, then press Escape
# open shortcuts via ?, then press Escape
```
**Expected**:
- Each `esc` press closes the most recently opened dialog (Radix Dialog default behavior; the `onOpenChange` handler at `CommandPalette.tsx:104` and `Shortcuts.tsx:36` is bound to the Radix dismiss)
- The dashboard's underlying route is unchanged
**Observed**:
- Palette closes on esc: ___
- Shortcuts close on esc: ___
- Route unchanged: ___
**Result**: PASS / FAIL

### Step 7: Topbar palette button mirrors the keyboard shortcut
**Do**: click the palette button (the `Kbd` reading `⌘K` at `Topbar.tsx:49`). The button calls `toggleCommand` from the UI store.
**Request**:
```http
GET /?nocache=26-7 HTTP/1.1
# click the "jump to anything" button
```
**Expected**:
- 200
- The palette opens with the same input focus and placeholder as Step 4
- The button's `aria-label` is `Open command palette` (`Topbar.tsx:47`)
**Observed**:
- Dialog open: ___
- aria-label: ___
**Result**: PASS / FAIL

### Step 8: Command palette — all 24 items are present across 4 groups
**Do**: with the palette open, take a snapshot and assert the 24 items are rendered under 4 group headings. The 24 items, with their exact `label` text, are:

| Group | Label | Href |
|---|---|---|
| Navigate | Now | `/` |
| Navigate | Memory hub | `/memory` |
| Navigate | Memory . Wakeup | `/memory?tab=wakeup` |
| Navigate | Memory . Recall | `/memory?tab=recall` |
| Navigate | Memory . Graph | `/memory?tab=graph` |
| Navigate | Memory . Architecture report | `/memory?tab=architecture` |
| Navigate | Memory . Activity heatmap | `/memory?tab=heatmap` |
| Navigate | Memory . Compression lab | `/memory?tab=compression` |
| Navigate | Memory . Savings | `/memory?tab=savings` |
| Navigate | Trust hub | `/trust` |
| Navigate | Trust . Reliability score | `/trust?tab=score` |
| Navigate | Trust . Drift center | `/trust?tab=drift` |
| Navigate | You hub | `/you` |
| Navigate | You . Profile | `/you?tab=profile` |
| Navigate | You . Device tokens | `/you?tab=tokens` |
| Navigate | You . Pair device | `/you?tab=pair` |
| Navigate | You . Audit log | `/you?tab=audit` |
| Navigate | You . Sessions | `/you?tab=sessions` |
| Navigate | You . Settings | `/you?tab=settings` |
| Navigate | Pack registry | `/registry/packs` |
| Navigate | Registry . Trusted keys | `/registry/trust` |
| Navigate | Registry . Revocations | `/registry/registry-revocations` (note: the actual href is `/registry/revocations`) |
| Navigate | Mobile companion (PWA) | `/mobile` |
| Memory | Remember something | `/memory?tab=wakeup` |
| Memory | Recall a memory | `/memory?tab=recall` |
| Devices | Issue a device token | `/you?tab=tokens` |
| Personalization | Add a preference | `/you?tab=profile` |

**Request**:
```http
# open palette with Ctrl+K
# enumerate every item and assert label match
```
**Expected**:
- 4 group headings: `Navigate`, `Memory`, `Devices`, `Personalization`
- Counts: 23 in Navigate + 2 in Memory + 1 in Devices + 1 in Personalization = 27 visible items? NO — the Navigate group contains 23 items per the source (`CommandPalette.tsx:73-95`), and the Action groups add 4 more, for a total of 27 items rendered under 4 group headings. The inventory's "24 items across 4 groups" is the historical count; the current `CommandPalette.tsx` source has 23 Navigate + 4 Action = 27 items. Walk the snapshot and assert every label above is present.
- No `CommandEmpty` placeholder is shown
- Selecting any Navigate item navigates to its `href` (verify the URL bar updates)
- Selecting `Remember something` lands on `/memory?tab=wakeup` (it is a Navigation shortcut, not an action)
**Observed**:
- Group count: ___
- Item count: ___
- All 27 labels present: ___
- Selection navigates: ___
**Result**: PASS / FAIL

### Step 9: Topbar health pill — polling cadence
**Do**: navigate to `/?nocache=26-9`. The health pill is the `Badge` at `Topbar.tsx:64-74`. Per the `useHealthQuery` hook it polls `/api/health` every 15s. Open the network panel and watch the GETs to `/api/health`.
**Request**:
```http
GET /api/health HTTP/1.1
# observe: a fresh GET fires every ~15s while the dashboard is mounted
```
**Expected**:
- Pill text is `healthy` with an emerald dot when `health.data?.status === "ok"` (`Topbar.tsx:28, 64-68`)
- Initial `GET /api/health` returns 200 with `{"status":"ok",...}`
- At t=0s, t=15s, t=30s, t=45s the network panel shows fresh `GET /api/health` requests
- When the server is offline, the badge variant switches to `destructive` and the text becomes `offline` (`Topbar.tsx:70-73`)
**Observed**:
- Pill text: ___
- GET /api/health count over 30s: ___
- 200s: ___
**Result**: PASS / FAIL

### Step 10: Topbar — mobile button and account avatar
**Do**: click the mobile-companion button (`Topbar.tsx:54-61`, lucide `Smartphone` icon, `aria-label="Open mobile companion"`). The dashboard routes to `/mobile`. Then navigate back, open the account avatar dropdown.
**Request**:
```http
GET /?nocache=26-10 HTTP/1.1
# click mobile button
GET /mobile HTTP/1.1
# back to /, click avatar circle (initial of me.username)
```
**Expected**:
- Mobile button has `aria-label="Open mobile companion"` and `title="Mobile companion"`
- Clicking it navigates to `/mobile`
- The avatar shows the first letter of the username uppercased (e.g. `A` for `admin`) at `Topbar.tsx:29, 88-89`
- Dropdown header reads `Signed in as <username>` (or `Account` if `me` is null) at `Topbar.tsx:93-95`
- Dropdown items in order: `Settings` (-> `/you?tab=settings`), `Audit log` (-> `/you?tab=audit`), separator, `Sign out` (destructive, calls `useLogoutMutation` and `router.replace("/login")` per `Topbar.tsx:31-35, 97-108`)
**Observed**:
- Mobile button label: ___
- Avatar initial: ___
- Dropdown header: ___
- Dropdown items: ___
**Result**: PASS / FAIL

### Step 11: HelpButton + FALLBACK_HELP on a page without helpCopy entry
**Do**: navigate to `/you?tab=settings&nocache=26-11`. The `HelpButton` at `settings/page.tsx:50` is fed `HELP["/you/settings"]`. Visit a page that doesn't pass `content` to verify the `FALLBACK_HELP` constant at `HelpButton.tsx:22-27` is reachable.
**Request**:
```http
GET /you?tab=settings&nocache=26-11 HTTP/1.1
# click the "?" button at the top right of the page header
```
**Expected**:
- 200
- The `?` button has `aria-label="Help: <title>"` per `HelpButton.tsx:47`
- The dialog title is `Settings` (from `HELP["/you/settings"]` if the entry exists) or `Help` (from `FALLBACK_HELP` at `HelpButton.tsx:23`)
- The dialog body has three sections: `What this is`, `How to use it`, `Impact on Cairn` (`HelpButton.tsx:55-82`)
- A page that calls `<HelpButton />` with no `content` prop renders the `FALLBACK_HELP` and does not crash with `TypeError: Cannot read properties of undefined (reading 'title')` — see the comment at `HelpButton.tsx:29-37` for the rationale
**Observed**:
- aria-label: ___
- Title: ___
- Sections present: ___
- Console errors: ___
**Result**: PASS / FAIL

### Step 12: Sidebar persistence after refresh
**Do**: refresh the page (`F5`) on `/memory?tab=graph&nocache=26-12`. The sidebar persists the collapsed/open state in `localStorage` key `cairn-sidebar-v3` (`Sidebar.tsx:37, 96`).
**Request**:
```http
GET /memory?tab=graph&nocache=26-12 HTTP/1.1
# press F5
GET /memory?tab=graph&nocache=26-12-r HTTP/1.1
```
**Expected**:
- 200 (both pre- and post-refresh)
- `Memory` is still `aria-current="page"` post-refresh
- The localStorage key `cairn-sidebar-v3` is `"1"` (set by `Sidebar.tsx:96`)
- The deprecated keys `cairn-sidebar-v1` / `cairn-sidebar-v2` / `cairn-infocard-dismissed-v1` are removed (per `Sidebar.tsx:83-89` migration)
**Observed**:
- aria-current preserved: ___
- localStorage cairn-sidebar-v3: ___
- Deprecated keys removed: ___
**Result**: PASS / FAIL

## DB Verification
- Not directly applicable to this doc. The shell chrome is pure client state plus `/api/health` (covered by doc 22). For a secondary cross-check, after Step 9: `/api/stats` should still return non-zero `memories` because the palette does not write to the store.

## UI Verification
- Route: `/`, `/memory`, `/memory?tab=graph`, `/?nocache=26-7`, `/you?tab=settings`
- Wait: `document.querySelector('[aria-current="page"]')` resolves to the expected hub; the `CommandDialog` mounts with `[role="dialog"]` and the Radix `data-state=open` attribute
- Assert:
  - Sidebar has exactly 5 items (`Now`, `Memory`, `Trust`, `Registry`, `You`)
  - Palette has 4 group headings and 27 items (23 Navigate + 2 Memory + 1 Devices + 1 Personalization) — see Step 8
  - Shortcuts modal has exactly 3 shortcuts with the keys/labels in Step 5
  - Topbar pill text is `healthy` on a healthy server
- Screenshot paths:
  - `web/test/screenshots/26-dashboard/sidebar-now.png`
  - `web/test/screenshots/26-dashboard/sidebar-memory.png`
  - `web/test/screenshots/26-dashboard/palette-open.png`
  - `web/test/screenshots/26-dashboard/shortcuts-modal.png`
  - `web/test/screenshots/26-dashboard/topbar.png`
  - `web/test/screenshots/26-dashboard/help-dialog.png`

## Evidence
- Snapshot of the sidebar at `/` and `/memory?tab=graph` (proves active-route highlight)
- Snapshot of the open palette showing all 4 group headings and 27 items
- Snapshot of the shortcuts modal showing the 3 shortcuts
- Network capture: at least 3 `GET /api/health` requests at ~15s spacing on `/`
- Console log: `list_console_messages types=["error"]` empty on every page
- Screenshots: 6 paths under `web/test/screenshots/26-dashboard/`

## Findings
(none expected)
