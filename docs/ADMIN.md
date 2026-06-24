# Administering Cairn

Cairn runs **inside a Docker container**; the host has only one binary
(`cairn`, the client). All admin operations happen either:

1. **At first boot, from environment variables** — `CAIRN_ADMIN_USERNAME` +
   `CAIRN_ADMIN_PASSWORD` in `.env` (or compose `environment:`).
2. **At any time, from the web dashboard** at <http://127.0.0.1:7777>
   once you're logged in.

There is no `cairn` host binary and no `docker exec` workflow. If your
admin session is lost, wipe the data volume and start over with a new
password in `.env`.

## First boot — env-only admin bootstrap

The cairn container reads these two vars at startup and, **only when no
admin record exists yet**, mints the admin record automatically:

```sh
# .env (in the project root, or wherever `docker compose up` reads it)
CAIRN_ADMIN_USERNAME=admin
CAIRN_ADMIN_PASSWORD=replace-with-a-strong-password
```

`docker compose up` runs the `cairn-admin-guard` init service first; it
refuses to bring the stack up if:

- `CAIRN_ADMIN_USERNAME` is empty.
- `CAIRN_ADMIN_PASSWORD` is shorter than 8 chars.
- `CAIRN_ADMIN_PASSWORD` equals `CAIRN_ADMIN_USERNAME`.

To opt out and let the dashboard `/setup` wizard mint the admin on first
visit instead, comment out `CAIRN_ADMIN_PASSWORD` in `.env`. The guard
will let compose proceed, and the first dashboard visit will mint the
admin through the wizard.

After the admin record exists, these vars are ignored. To bootstrap a
fresh admin again, `docker compose down -v` (wipes the data volume) and
restart.

## After first boot — dashboard

Log in at <http://127.0.0.1:7777/login> with the admin credentials.

### Mint a device token

`/settings/tokens` → "Mint token" → fill the form → submit. The bearer
token appears **once** in the success toast; copy it immediately. Use
it as `Authorization: Bearer <token>` on subsequent API calls, or pass
it to `cairn setup <agent> --token <token>` to wire an AI agent.

### Generate a pair code

`/settings/pair` → click "Generate pair code" → a 10-minute code appears.
Enter it on the new device:

```sh
cairn pair CAIRN-XXXX-XXXX --server http://your-host:7777
```

The pair code is single-use; regenerate as needed.

### Rotate the admin password

`/settings/admin` (if present in v0.6.0+) → "Rotate password" → enter
old + new → submit. The rotation bumps the admin generation counter,
invalidating every existing session cookie. Anyone still logged in
gets bounced to `/login` on next request.

If the dashboard password-rotation form isn't available in your build,
the fallback is `docker compose down -v` (wipes the admin record) →
restart → `/setup` wizard mints a fresh admin.

### Reset the admin record (emergency)

If you've lost the password and have shell access to the host:

```sh
docker compose down -v        # wipes the cairn-data volume (and MinIO bucket)
docker compose up -d          # restarts; cairn-admin-guard sees no admin record
                               # → /setup wizard is open
```

There is intentionally no HTTP "reset admin" route. Any admin-reset
endpoint would itself require an authenticated admin — chicken and egg.

## Curl equivalents

For scripts and CI, the dashboard endpoints are HTTP and stable:

```sh
# Mint a write-scope device token (admin session required)
curl -X POST http://127.0.0.1:7777/api/devices/tokens \
  -H 'Cookie: cairn_session=…' \
  -H 'Content-Type: application/json' \
  -d '{"name":"ci-runner","scope":"write"}'

# List tokens
curl http://127.0.0.1:7777/api/devices/tokens \
  -H 'Cookie: cairn_session=…'

# Revoke a token
curl -X POST http://127.0.0.1:7777/api/devices/tokens/<id>/revoke \
  -H 'Cookie: cairn_session=…'

# Generate a pair code
curl -X POST http://127.0.0.1:7777/api/devices/pair-codes \
  -H 'Cookie: cairn_session=…' \
  -H 'Content-Type: application/json' \
  -d '{"name":"laptop"}'
```

All four require an admin session cookie (the dashboard handles login).
`/api/devices/tokens` returns the bearer token **once** in the response
body; subsequent reads only return metadata.

## Why no `cairn` host binary?

See `docs/DECISIONS.md` ADR-029. Short version: Docker is the only
install path; the user never SSHes into the container; the dashboard
+ env vars cover everything.

## See also

- `docs/UPGRADING.md` — version-to-version upgrade notes
- `docs/ARCHITECTURE.md` — the full crate graph + HTTP route map
- `docs/DECISIONS.md` — ADR-029 (env-only admin bootstrap) and
  ADR-030 (no host binary)
