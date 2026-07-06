---
title: "Administering Cairn"
type: guide
status: living
updated: 2026-07-01
---

# Administering Cairn

The Cairn **server** runs **inside a Docker container**; there is no
`cairn-server` binary on the host. The **client** binary (`cairn`,
`crates/cairn-client/`) is a real, separate binary that does run on the
host - it's what you use to talk to the containerized server (`onboard`,
`setup`, `doctor`, MCP, hooks, etc.). All admin operations happen either:

1. **At first boot, from environment variables** - `CAIRN_ADMIN_USERNAME` +
   `CAIRN_ADMIN_PASSWORD` in `.env` (or compose `environment:`).
2. **At any time, from the web dashboard** at <http://127.0.0.1:7777>
   once you're logged in.

There is no `docker exec` workflow for admin tasks - everything above is
either an env var read at container startup or a dashboard action. If your
admin session is lost, wipe the data volume and start over with a new
password in `.env`.

## First boot - env-only admin bootstrap

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

## After first boot - dashboard

Log in at <http://127.0.0.1:7777/login> with the admin credentials.

### Mint a device token

`/you/tokens` -> "Mint token" -> fill the form -> submit. The bearer
token appears **once** in the success toast; copy it immediately. Use
it as `Authorization: Bearer <token>` on subsequent API calls, or pass
it to `cairn setup <agent> --token <token>` to wire an AI agent, or run
on the new device:

```sh
cairn onboard --server http://your-host:7777 --token <jwt>
```

### Rotate the admin password

`/you/settings` -> "Rotate password" -> enter old + new -> submit. The
rotation bumps the admin generation counter, invalidating every existing
session cookie. Anyone still logged in gets bounced to `/login` on next
request.

If the dashboard password-rotation form isn't available in your build,
the fallback is `docker compose down -v` (wipes the admin record) ->
restart -> `/setup` wizard mints a fresh admin.

### Reset the admin record (emergency)

If you've lost the password and have shell access to the host:

```sh
docker compose down -v        # wipes the cairn-data volume (and MinIO bucket)
docker compose up -d          # restarts; cairn-admin-guard sees no admin record
                               # -> /setup wizard is open
```

There is intentionally no HTTP "reset admin" route. Any admin-reset
endpoint would itself require an authenticated admin - chicken and egg.

## Curl equivalents

For scripts and CI, the dashboard endpoints are HTTP and stable:

```sh
# Mint a write-scope device token (admin session required)
curl -X POST http://127.0.0.1:7777/api/devices/tokens \
  -H 'Cookie: cairn_session=...' \
  -H 'Content-Type: application/json' \
  -d '{"name":"ci-runner","scope":"write"}'

# List tokens
curl http://127.0.0.1:7777/api/devices/tokens \
  -H 'Cookie: cairn_session=...'

# Revoke a token
curl -X POST http://127.0.0.1:7777/api/devices/tokens/<id>/revoke \
  -H 'Cookie: cairn_session=...'
```

All three require an admin session cookie (the dashboard handles login) -
`/api/devices/*` is cookie-only by design, so a bearer token cannot call
these endpoints even if it's scoped `admin`. `/api/devices/tokens` returns
the bearer token **once** in the response body; subsequent reads only
return metadata.

## Why is there no `cairn` *server* binary on the host?

This is specifically about the **server** (`cairn-server`, the in-container
axum process) - not the client. The `cairn` client binary
(`crates/cairn-client/`) is real and is exactly what you run on the host
for everything in this guide (`cairn onboard`, `cairn setup`, `cairn
doctor`, ...). What doesn't exist on the host is a `cairn-server` binary
or a `docker exec`-into-the-container admin workflow: Docker is the only
install path for the server (see `docs/reference/decisions.md` ADR-029),
the user never SSHes into the container, and the dashboard + env vars
cover every admin operation.

## See also

- `docs/guides/upgrading.md` - version-to-version upgrade notes
- `docs/reference/architecture.md` - the full crate graph + HTTP route map
- `docs/reference/decisions.md` - ADR-029 (delete the `cairn-server` crate;
  server is Docker-only) and ADR-030 (rename `cairn-cli` -> `cairn` client
  binary)
