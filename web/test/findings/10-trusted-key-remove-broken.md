# Finding 10-3: Remove trusted key mutation calls non-existent API path

- Run: 4 (Registry flows)
- Date: 2026-06-30
- Severity: bug
- Page: http://127.0.0.1:7777/registry/trust

## Symptom
Clicking the per-row menu button -> "Remove" on a trusted key confirms
in the dialog, but the key remains in the table. The list query
(`useTrustedKeysQuery` via `qk.registryTrustedKeys`) never invalidates
the cache because the mutation reported success.

## Root cause
`web/src/lib/queries.ts:229`:

```ts
delJSON<unknown>(`/registry/trusted-keys?key=${encodeURIComponent(key)}`)
```

Same path-prefix bug as Finding 10-1. The API lives at
`/api/registry/trusted-keys` (see `crates/cairn-api/src/lib.rs:321`).

The corresponding list query at line 179 and add-key mutation at line
216 are correctly prefixed with `/api/`. Only `useRemoveTrustedKeyMutation`
is wrong.

The cairn-server returns dashboard HTML with HTTP 200 for any
`/registry/...` URL (the Next.js static export catches it). The mutation
treats 200 as success, no error toast appears, no cache invalidation.

## Suggested fix
```ts
delJSON<unknown>(`/api/registry/trusted-keys?key=${encodeURIComponent(key)}`)
```

## Evidence
- Network: `DELETE /registry/trusted-keys?key=a52bac...` -> 200
  text/html, not 200 application/json.
- A11y snapshot taken after the Remove confirmation still shows the
  `a52bac73...` row.
- `useTrustedKeysQuery` did not re-fetch (no `GET /api/registry/trusted-keys`
  after the DELETE in the network log).
