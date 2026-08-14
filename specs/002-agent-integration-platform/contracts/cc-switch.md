# Contract: CC Switch Integration Manager

**Feature**: `002-agent-integration-platform`

CC Switch is an **integration manager**, not an agent adapter. It produces no sessions, no
observations, and no lifecycle events. It distributes two Cairn resources — the MCP server
and the Skill — into applications it manages (FR-101, FR-103).

Everything here rests on CC Switch's documented public surface (research D33). Nothing else
is used.

## What Cairn may touch

| May | May not |
|---|---|
| The `ccswitch://v1/import` deep link | `~/.cc-switch/cc-switch.db` |
| Reading target applications' own configuration to verify | `~/.cc-switch/settings.json` |
| Detecting that CC Switch is installed, and its version | `~/.cc-switch/skills/`, `backups/`, any other private file |

**FR-232 is absolute**: no operation — connect, distribute, migrate, repair, disconnect —
writes to CC Switch's own storage, and no requirement in this feature may be satisfied by
doing so.

## Detection

CC Switch is detected by the presence of its application installation and, where obtainable
without authentication, its version. Detection reads nothing from its database. An
undetectable version classifies as `compatible_unverified` (FR-186).

Reported as a manager, never in the agent list (FR-101).

## Distribution — the import flow

CC Switch's documented interface is **import only**. There is no documented removal or query
interface, and Cairn does not invent one (FR-233).

```
cairn integration distribute --via cc-switch --resource mcp --apps claude,codex,opencode
```

1. Cairn builds the deep link from its own MCP configuration:

   ```
   ccswitch://v1/import?resource=mcp&name=cairn&apps=claude,codex,opencode&config=%7B%22command%22%3A%22cairn%22%2C%22args%22%3A%5B%22mcp%22%5D%7D
   ```

   The `config` value is the same secret-free block `cairn integration export mcp` emits.

2. Cairn opens the link (or prints it, when it cannot). CC Switch shows **its own
   confirmation dialog** — that boundary belongs to CC Switch and Cairn does not attempt to
   pass it.

3. Cairn returns `manager_action_required` with status `awaiting_user`. **The operation has
   not completed.**

4. After the developer confirms, `cairn doctor` inspects each target application's real
   configuration and records ownership from what it finds (FR-234).

The Skill import is the same flow with `resource=skill`:

```
ccswitch://v1/import?resource=skill&name=cairn&repo=Vellixia/Cairn&directory=skills/cairn&branch=<pinned-ref>
```

This is why the Skill's canonical source is a repository path rather than a generated
artifact (D29): CC Switch fetches Skills from a public Git repository, so the repository
*is* the distribution channel, and there is no second copy to drift.

### Skill Git ref

Direct installation always uses the Skill embedded in the running binary — the binary is the
source of truth, and no network is involved. Only the manager path needs a Git ref, and CC
Switch accepts a narrower ref space than Git does.

**What CC Switch actually does** — `src-tauri/src/services/skill.rs`, `main`, verified
2026-08-11:

- `download_repo` builds `https://github.com/{owner}/{name}/archive/refs/heads/{branch}.zip`.
- `assert_github_archive_url` rejects any URL whose path does not begin
  `/{owner}/{name}/archive/refs/heads/` — a deliberate guard against a `branch` value that
  redirects the download to a release asset.
- If that download fails it **silently retries `main`, then `master`**.

So a commit SHA or a tag in `branch=` does not resolve to that commit: it becomes a request for
a *branch of that name*, 404s, and CC Switch then installs `main`. That is worse than an error,
because the developer ends up with a Skill revision the binary never expected and no signal
that it happened.

**Therefore the ref is always a real branch, and always one Cairn controls and never rewrites**
(D29):

| Build | `<pinned-ref>` |
|---|---|
| The embedded Skill revision has a published `skill-release/<schema>-<revision>` branch | that branch |
| It does not — every development build, and any build from a dirty tree | **none** — the Skill import is refused |

The branch name encodes the Skill's own schema and revision, for example
`skill-release/1-c07d4419b2ae`. It is created once for that revision and **never moved**, so it
behaves like a pinned ref while still being a `refs/heads` ref CC Switch accepts.

Because the name identifies **content**, not a Cairn release, later Cairn releases that ship the
same Skill reuse the same branch and leave it pointing at the commit that first introduced that
content. A release verifies the branch by fetching it and recomputing the revision from what it
actually contains; a mismatch between a branch's content and its name fails the release, and no
release ever force-updates a branch (D29a).

Where the embedded revision has no published branch, `cairn integration distribute --resource
skill` fails with `unpublished_skill_ref`, states why, and gives the manual path. It never emits
a branch name it has not established exists, precisely because the failure mode is a silent
fallback rather than an error. **A development build can still distribute the MCP resource**,
which carries no Git ref at all.

After distribution, `cairn doctor` reads the installed `SKILL.md`'s
`metadata.cairn_skill_revision` and compares it with the embedded digest. A mismatch is
`outdated`, with the remedy naming the correct ref — and it is also the backstop that would
catch a `main` fallback arriving by any other route.

## Removal

CC Switch documents no automated removal. Therefore (FR-233, FR-149):

```json
{
  "ok": false,
  "error": { "code": "manager_action_required",
             "message": "CC Switch owns the Cairn MCP entry for codex" },
  "data": {
    "manager": "cc-switch",
    "resource_kind": "mcp",
    "applications": ["codex"],
    "action": "remove",
    "method": "manual_ui",
    "uri": null,
    "instructions": "In CC Switch: open MCP, select `cairn`, turn off the Codex binding — or remove the server if no application still needs it. Cairn does not modify CC Switch's own storage.",
    "verify_with": "cairn doctor codex",
    "status": "awaiting_user"
  }
}
```

Rules for this outcome:

- `uri` is present only for `import`, and only when the link carries no secret. Removal has
  no documented link, so it is `null` — never a fabricated one.
- `status` is `awaiting_user` until verification observes the change; then `verified`, or
  `not_performed` if verification finds it unchanged.
- The enclosing operation **fails** with `manager_action_required` (exit 1). Cairn never
  reports success on the strength of having asked (FR-233).
- The local record keeps `owner = manager` until verification says otherwise (FR-234).
- **The record survives a native disconnect** (FR-244, D28a). `cairn disconnect codex` removes
  the resources Cairn owns directly and drops their bindings, but the manager-owned resource,
  its binding, and this pending action stay — otherwise there would be nothing left to verify
  the withdrawal against. The agent's `AgentIntegration` row is removed only once its last
  binding is gone, which happens when verification observes the manager-owned entry actually
  removed.

If CC Switch later publishes a documented removal interface, the adapter may use it, and the
same verification still gates the record update (FR-235).

## Ownership

CC Switch writes only per-user configuration (D33), which means a Cairn resource it owns and
a Cairn resource Cairn installs directly usually target the **same file at the same scope**.
That is a genuine collision, and it is reported rather than resolved by relocation:

- If CC Switch owns `mcp` for an application, `cairn connect` for that agent will not install
  a direct `mcp` entry; it returns `conflicting_owner` naming both (FR-146, FR-219).
- If both already exist, doctor reports `conflicting_owner` and repair explains rather than
  choosing (FR-174).

Manager-owned rows carry no `content_hash`: Cairn did not write the bytes, so it verifies
presence and effectiveness instead of equality (see [data-model.md](../data-model.md)).

## Migration between owners

**Direct → CC Switch** may be automated up to the manager's confirmation boundary (FR-236):

```
planned → (deep link opened) → awaiting_user
        → target_verified  (doctor finds the entry in the app's own config)
        → source_removed   (Cairn removes only what it owns directly)
```

**CC Switch → direct** never deletes the manager's resource behind its back (FR-237):

```
planned → target_installed (Cairn's own entry, where precedence permits)
        → target_verified
        → manager_action_required   (the developer withdraws it in CC Switch)
        → verified → source_removed
```

Until verification confirms exactly one owner, the resource stays `migrating` (FR-228), and
doctor reports it that way rather than as `duplicated`.

Where the two owners target the same effective slot and overlap would be ambiguous, automatic
migration is refused with `migration_unsafe` and the manual sequence is printed (FR-148,
D38). For Claude Code's `mcp` at user scope — the same `~/.claude.json` CC Switch writes —
this is the expected outcome.

## Provider switching

Switching provider or configuration inside CC Switch changes provider credentials and model
routing, none of which Cairn manages (FR-200). Cairn's resources are unaffected, and
`cairn doctor` after a switch must report every Cairn resource healthy with zero duplicates
(SC-113). This is a fixture case, not an assumption.

## What Cairn never does

- Manage providers, credentials, OAuth material, API keys, model routing, pricing, or usage.
- Read CC Switch's database for any purpose, including detection.
- Install native lifecycle adapters for applications CC Switch happens to support — Gemini,
  Hermes, Grok Build, OpenClaw, and Claude Desktop reach Cairn through the generic MCP path
  only (FR-106).
