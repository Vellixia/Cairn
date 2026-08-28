# Security Policy

## Supported versions

Cairn is pre-1.0 and under active development. Only the most recent release
line receives fixes.

| Version | Supported |
|---|---|
| 0.1.0-alpha.x | Yes |
| < 0.1.0-alpha.1 (obsolete implementation) | No |

## Who can obtain administrator access

State this plainly, because it is the outer boundary of the entire role model
rather than a footnote to it:

**Whoever can set the server's environment and restart the process can always
obtain administrator access.**

The account named by `CAIRN_ADMIN_EMAIL` and `CAIRN_ADMIN_PASSWORD` is
re-applied on every start — its password, its `admin` role, and its `active`
status. That is deliberate, and it is the only recovery path a self-hosted
deployment has: an operator who demotes or disables the last administrator has
no supported API left to recover through, and without this they would be locked
out of their own server with no remedy short of editing the database by hand.

Two consequences follow, and neither is a defect:

- The environment-named account cannot be demoted, disabled, or have its
  password reset through the API. Each is refused with a message naming
  `CAIRN_ADMIN_EMAIL`, because a change a restart would silently revert is worse
  than a rejection — the operator walks away believing it took effect.
- Anyone with shell access to the host, or with the ability to change the
  service's environment and restart it, is effectively an administrator whatever
  the `users` table says.

This is correct for a self-hosted server: that person already controls the host
and the database, so the role model was never what stood between them and the
data. It is written down here so that nobody has to infer it from the source, and
so that a deployment which hands out restart permission understands what it is
handing out.

Accounts are otherwise created only by an administrator — through
`POST /api/admin/users`, or locally with `cairn-server users add`. There is no
self-registration route and no self-join route; both were removed as security
fixes.

## Reporting a vulnerability

Please report security issues privately rather than opening a public issue.

Use GitHub's private reporting: **Security → Report a vulnerability** on
<https://github.com/Vellixia/Cairn/security/advisories/new>.

Include what you need to describe the issue — affected version, environment,
reproduction steps, and impact. We aim to acknowledge a report within a few
working days and to keep you informed while a fix is prepared. Please give us a
reasonable opportunity to release a fix before disclosing publicly.

## What Cairn stores, and where

Understanding the data boundary is usually the fastest way to judge impact.

- **Local, by default.** Observations, memories, handoffs, sessions and tasks
  live in a SQLite database under your Cairn home directory. Nothing leaves the
  machine unless you explicitly link a project to a server.
- **Never persisted.** Full conversations and raw tool output are not stored.
  Captured payloads are bounded and summarized, and values matching common
  secret patterns are redacted before anything is written.
- **Never transmitted.** Raw observation content cannot be sent to a server:
  there is no observation entity type on the wire, so such a payload cannot be
  constructed. A memory or handoff carries evidence as identifiers and a count,
  not content. Local-only memories are never queued for sync.
- **Credentials.** A server API token is stored in the Cairn home directory
  with `0600` permissions.

## Deployment notes

If you run the server components:

- Terminate TLS in front of `cairn-server`; it speaks plain HTTP.
- Set `CAIRN_WEB_ORIGIN` to your web UI origin so browser requests are
  restricted to it.
- Supply `DATABASE_URL` and any secrets through environment variables or your
  orchestrator's secret store. Never bake them into an image.
- Restrict network access to PostgreSQL to the server component.
