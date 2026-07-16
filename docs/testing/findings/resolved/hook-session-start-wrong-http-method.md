---
title: "Finding: cairn hook SessionStart never registers the auto-detected project (wrong HTTP method)"
type: finding
status: open
updated: 2026-07-03
severity: high
---

# Finding: cairn hook SessionStart never registers the auto-detected project (wrong HTTP method)

**Flow:** 31-v0.8.0-surreal-scope-projects
**Severity:** high
**Discovered:** 2026-07-03

## What happened
`crates/cairn-client/src/hook.rs`'s `SessionStart` handler calls
`rc.post_spooled("/api/projects/upsert", ...)`, which sends an HTTP **POST**
(`RemoteClient::post` wraps `ureq::post(...)`). The server's route is registered
`.route("/api/projects/upsert", patch(upsert_project))` in `crates/cairn-api/src/lib.rs` -
**PATCH only**. Every call gets a `405 Method Not Allowed`. That's not a
`ureq::Error::Transport`, so the Sprint 9 offline-spool logic correctly treats it as "the server
answered, nothing to retry" and drops it - by design for a genuine HTTP error, just triggered
here by the wrong method rather than a real client mistake.

Confirmed live: `curl -X POST http://127.0.0.1:7777/api/projects/upsert ...` returns `405`; the
identical body via `curl -X PATCH` returns `200` and the project immediately appears in
`GET /api/projects`.

Confirmed via `git log -S'"/api/projects/upsert"' -- crates/cairn-client/src/hook.rs`: this has
been broken since the feature was introduced in `b7024a2` ("feat(cairn-client): auto project
detection + registry (v0.8.0 Sprint 3)"), which already used `.post(...)`. It predates the v0.8.0
Sprint 9 offline-spool work entirely - spool only changed what happens on a transport failure,
not the HTTP method used for this call.

Auto project detection - the entire point of Sprint 3 - has silently never worked end-to-end in
production since it shipped. It only appeared to work in `crates/cairn-tests`' hermetic
integration tests because those exercise the server's `upsert_project` handler directly (or via
the correct PATCH method), never through the real `cairn` CLI client.

## Expected
After `cairn hook SessionStart` runs once inside a git-initialized project directory (with
`CAIRN_SERVER`/`CAIRN_TOKEN` set), `GET /api/projects` includes an entry for that project.

## Actual
`GET /api/projects` never gains an entry from the hook, in any session. A manually-issued
`PATCH` with the same body succeeds immediately.

## Suggested fix
Add a `patch()` counterpart to `RemoteClient`'s existing `get()`/`post()` methods in
`crates/cairn-client/src/hook.rs` (`ureq::patch(...)`, same `with_scope_headers` wrapping), and
switch the `SessionStart` project-upsert call site to use it - ideally via a `patch_spooled`
sibling to `post_spooled` so the Sprint 9 offline-spool protection isn't lost for this call.
