---
title: "Finding 10-2: Pack search box calls non-existent API path"
type: finding
status: resolved
updated: 2026-07-01
severity: medium
---

# Finding 10-2: Pack search box calls non-existent API path

- Run: 4 (Registry flows)
- Date: 2026-06-30
- Severity: bug
- Page: http://127.0.0.1:7777/registry/packs (search input)

## Symptom
Typing in the "Search packs..." textbox on `/registry/packs` does not
filter the table. The `useSearchPacks` query fires, returns the dashboard
HTML, and is treated as an empty/error result.

## Root cause
`web/src/app/(app)/registry/packs/PacksContent.tsx:186`:

```ts
getJSON<RegistryPackMeta[]>(`/registry/search?q=${encodeURIComponent(search)}`)
```

Same path-prefix bug as Findings 10-1 and 10-3. The API lives at
`/api/registry/search`. Curl confirmation: the API works at the prefixed
path, returns a JSON array of `RegistryPackMeta`.

## Suggested fix
```ts
getJSON<RegistryPackMeta[]>(`/api/registry/search?q=${encodeURIComponent(search)}`)
```

## Evidence
- Network: `GET /registry/search?q=...` returns 200 text/html (the
  Next.js dashboard catches the URL before the cairn-server can route it).
- All other registry hooks in `queries.ts` are correctly prefixed
  `/api/registry/...` (list, add trusted key, list trusted keys, list
  revocations). The three outliers are all in registry mutations and
  this one search query.
- A11y snapshot after typing in the search box shows the original
  un-filtered list, confirming the search query was discarded.
