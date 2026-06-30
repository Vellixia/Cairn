# Fix: `/memory/architecture`, `/memory/heatmap`, `/registry`, `/registry/packs` no longer crash

**Flows:** 06 architecture-report-and-heatmap, 03 pack-registry
**Severity:** resolved
**Discovered:** 2026-06-30
**Fixed:** 2026-06-30 (commit pending on `0.7.1`)

## Root cause

`HelpButton` consumed `content.title` where `content` came from `HELP["/registry"]`,
`HELP["/memory/architecture"]`, and `HELP["/memory/heatmap"]` -- none of those
keys exist in `web/src/components/helpCopy.ts`. The optional-chaining-less
property access blew up before the page rendered, producing the
`TypeError: Cannot read properties of undefined (reading 'title')` reported by
the dashboard flow audit.

## Fix

`web/src/components/HelpButton.tsx:38-39` -- `content` is now optional, and
the component falls back to a generic `FALLBACK_HELP` const when the route
has no entry in `helpCopy.ts`:

```ts
export function HelpButton({ content }: { content?: HelpContent }) {
  const c: HelpContent = content ?? FALLBACK_HELP;
  ...
}
```

## Verification

After rebuilding `cairn:dev` (the docker-compose.override.yml in this branch
pins the published `ghcr.io/vellixia/cairn:latest`; for local verification
the override must be bypassed with `docker compose -f docker-compose.yml
--project-name cairn up -d cairn`):

- `GET /memory/architecture` renders "Architecture" heading + Nodes/Edges/
  Communities/Isolation/Languages stats. Help button shows aria-label
  `Help: Help` (fallback active). No `Application error`.
- `GET /memory/heatmap` renders "Activity" heading + heatmap grid + month
  labels. Help button aria-label `Help: Help`. No `Application error`.
- `GET /registry` and `GET /registry/packs` render the Pack registry page
  with the navigation tabs (Packs / Trusted Keys / Revocations) and an
  empty-state "No packs published yet". No `TypeError`.

## Follow-up

The `FALLBACK_HELP` is generic. Adding per-route entries to
`web/src/components/helpCopy.ts` (e.g. `/registry`, `/memory/architecture`,
`/memory/heatmap`) would replace the fallback with route-specific copy and
turn the `aria-label="Help: Help"` into something more descriptive. Tracked as
a follow-up; not required for the bug fix.