# 11 - Command palette "Enter" crashes on `/registry/trust` and `/registry/revocations`

**Severity:** high (P1) — blocks palette navigation to two routes
**Status:** BUG, unfixed at end of web/test phase
**Repros:** 100% reliable on production build, 0% on dev (port 3100)

## Symptom
On the dashboard home page (`/`), pressing `Cmd+K` (or `Ctrl+K`), typing
`revocations` (or `trust`), and pressing `Enter`:

- URL does **not** change from `/?cb=...`
- `document.title` becomes empty
- body innerText: `"Application error: a client-side exception has occurred (see the browser console for more information)."`
- Only the offending static-exported pages crash. Direct navigation
  (`location.assign("/registry/revocations")` or clicking the sidebar) renders
  both routes fine. The crash is **client-side transition state**, not the
  route handler.

## Reproduction
```sh
# production (cairn:dev container, port 7777)
http://127.0.0.1:7777/
Cmd+K -> type "revocations" -> Enter -> crash
http://127.0.0.1:7777/registry/revocations  (direct nav, PASS)

# development (Next dev server, port 3100)
http://127.0.0.1:3100/
Cmd+K -> type "revocations" -> Enter -> /registry/revocations renders OK
```

## What was tried
- Chunk audit: dumped all 25 chunks loaded for `/registry/trust` and
  grepped for `.title`, `["title"]`, `[title]`. All accesses are null-safe
  (`null != t ? t : i`, `null == v ? void 0 : v.title`, etc.).
- `HelpButton` (chunk `7824-64a5a15ded423891.js` function 79358) confirmed
  null-safe. `FALLBACK_HELP` constant defined. TrustContent.tsx:145 and
  RevocationsContent.tsx:28 both pass `HELP["/registry"]` (valid key in
  helpCopy map). Compiled output preserves `null != t ? t : i` defensive
  read.
- Error capture via:
  - `window.addEventListener('error', e => ...)` — never fires (Next.js
    production error overlay swallows the original `Error`).
  - `window.addEventListener('unhandledrejection', ...)` — never fires.
  - `window.Error` constructor patch (captures every `new Error()`) — only
    captured React internal `DetermineComponentFrameRoot` errors, not the
    original `TypeError`.
  - `console.error` patch — captured `l_(t.value)` with `t.value === "{}"`
    (a stringified `errorInfo`), no original message.
  - The reported `TypeError: Cannot read properties of undefined (reading
    'title')` message is only visible in DevTools because Chrome unwraps
    the errorInfo before display. The `Error.stack` shown in the React
    fiber is `l_ -> n.callback -> nB -> nV -> aq -> aY -> a9 -> ...`,
    ending at `M` in `2117-8e55345e9af6d99b.js` (Next.js router). No
    source location for the actual `.title` access is visible.

## What is known
1. Production build of `web/out/` differs from dev build. The crash
   reproduces 100% on production, 0% on dev. Some minifier pass or
   production-only behavior is involved.
2. `chunk 7824` (HelpButton + helpCopy) loads **only** on the two crashing
   routes. The chunk source is 16758 bytes, the compiled `c(e)` function is
   null-safe. So either:
   - the crash predates chunk 7824 (a parent component throws before
     HelpButton renders), or
   - a different chunk has the bug and was not loaded into the test
     snapshot.
3. Direct URL nav does not crash. The crash is a client-side transition
   state mismatch from `CommandPalette` `nav()` calling `router.push()`.
4. The crash happens **after** the URL has been set but **before** the new
   page's chunks render. The home page error overlay stays on screen
   because the route change errored out.

## What was NOT tried (deliberately)
- Adding `try/catch` + `Error.stack` capture in `CommandPalette.tsx::nav()`.
  This would work but mixes a debug probe into a production build, which
  is out of scope for the audit phase.
- Bisecting production chunks by deleting one at a time. Each
  `next build` takes 10+ seconds and would require rebuilding `cairn:dev`
  repeatedly. Pinned cairn:dev sha for the duration of the audit phase
  per protocol.
- Comparing production `page-3ebce6245530ff0e.js` to dev
  `/_next/static/chunks/app/(app)/registry/trust/page.js` source. Would
  need to dump dev chunk first; out of scope for this audit pass.

## Fix recommendation
- Short term: add a `try { router.push(href); } catch (e) { console.error(e); }`
  wrapper in `CommandPalette.tsx::nav()` to surface the real error.
- Medium term: bisect production chunks (likely `7824` is innocent, suspect
  a minifier-induced `.title` access in another chunk), then fix the
  actual throw site.
- Long term: enable `swcMinify` source maps in `next.config.mjs` so
  production crashes are debuggable without sniffing the minified
  output. Currently no `productionBrowserSourceMaps: true`.

## Evidence
- `web/test/screenshots/11-palette-trust-crash.png` (production crash)
- `web/test/screenshots/11-palette-dev-ok.png` (dev same flow, no crash)
- console.error capture with `args: ["{}"]` and stack ending at `l_` in
  `fd9d1056-986e88678380c101.js:1:56405`.
