# Contract: Project Authorization

**Feature**: `004-collaborative-global-memory`

This contract guarantees that `project_members` is the *only* source of authorization for a
project-scoped route, that no route lets a user grant themselves membership, and that
discovery — looking a project up by its shared identity — can report what already exists but
can never cause it to exist. It depends on the prerequisite hardening patch
(`security-prerequisite.md`) having already removed `POST /api/projects/{id}/join`; this
contract specifies what replaces it.

## 1. The authorization predicate

Every project-scoped route is authorized by exactly one predicate, unchanged in shape from
today's `require_member` (`crates/cairn-server/src/auth.rs:231-243`):

```sql
SELECT user_id FROM project_members WHERE project_id = $1 AND user_id = $2
```

A route is authorized **iff** this query returns a row for the caller's `user_id` and the
route's `project_id`. There is no second predicate, no role escalation, no "admin bypasses
membership" clause: an admin who is not a member of project P has exactly the same access to
P as any other non-member (none), because `project_members` carries no role column
(`crates/cairn-server/migrations/0001_init.sql:48`, unchanged by this feature) and
`ServerRole` (identity-administration.md §1) governs server-level administration, not
project content. An admin who wants access to a project joins it the same way anyone does —
through §2 below, added by another member or admin **who is themselves already a member**.

### Every route this predicate gates

Unchanged in requirement, restated for completeness against the corrected code (`api.rs`
line numbers from the post-prerequisite-patch server):

| Route | Predicate applied |
|---|---|
| `POST /api/sync/batch` | `require_member(pool, body.project_id, user.id)` |
| `GET /api/sync/changes` | `require_member(pool, q.project_id, user.id)` |
| `GET /api/projects/{id}`, `/tasks`, `/sessions`, `/memories`, `/sync-status` | `require_member(pool, id, user.id)` |
| `GET /api/sessions/{id}`, `/api/sessions/{id}/handoff`, `GET /api/memories/{id}`, `DELETE /api/memories/{id}` | fetch the row, resolve its `project_id`, **then** `require_member` — the existing fetch-then-authorize pattern, unchanged |
| `POST/DELETE/GET /api/projects/{id}/members` (new, §2) | `require_member`, with an additional admin-or-member distinction on write (§2) |

The fetch-then-authorize routes are worth naming explicitly: they authorize against the
*entity's actual project*, not a caller-supplied one, which is what makes them safe once §4's
sync-write predicate closes the analogous gap on the write side.

## 2. Membership grant and revoke — never self-service

**FR-418**: no route exists through which a user adds themselves. Every membership grant
requires an already-authorized actor naming *someone else*.

`SC-465` verifies this by **exercising every route the server exposes** and asserting the
caller's own membership set is unchanged — not by observing that `POST /api/projects/{id}/join`
was deleted. The deleted route is the defect that was found; the requirement is about the routes
that *exist*, including ones added after this feature ships. A test asserting one named route is
absent passes unchanged on the day a different route grants self-membership, which is exactly
when it needed to fail.

### `POST /api/projects/{id}/members`

| | |
|---|---|
| Auth | `CurrentUser` + `require_member(id, caller)` — the caller must already be a member of `{id}`, or a server admin (server-level `role = admin` bypasses the membership check for this route specifically, so an admin can bootstrap membership on a project with no members left to add anyone) |
| Request | `{ "user_id": uuid }` (looked up by id, not email, to avoid an email-enumeration side channel on this route) |
| Response `201` | `{ "project_id": uuid, "user_id": uuid, "added_by_user_id": uuid, "created_at": timestamp }` |
| Errors | `403 forbidden` (caller not a member and not admin) · `404 not_found` (no such user or project) · `409 already_member` |

`project_members` gains exactly one column: `added_by_user_id UUID NULL REFERENCES
users(id)` (FR-419) — nullable, and **not** backfilled to any guessed value for pre-existing
rows (D432). The table already carries `created_at TIMESTAMPTZ NOT NULL DEFAULT now()`
(`0001_init.sql:52`), which already answers "when" — this feature adds no second timestamp
column, since one already existed and duplicating it would only invite the two to drift.
"Who added them" is genuinely new information the old schema could not express, but "when"
was never missing; `added_by_user_id` is `NULL` for every row that predates this feature,
because who granted a pre-existing membership was never recorded and this contract does not
fabricate an answer — 003's rule against inventing unrecorded state (Constitution VI, applied
here to a migration backfill rather than to a derived project fact) forbids guessing it, so a
pre-existing membership simply reports `added_by_user_id: null` rather than being attributed
to whoever created the project or to the account itself.

### `DELETE /api/projects/{id}/members/{user_id}`

| | |
|---|---|
| Auth | `CurrentUser` + `require_member(id, caller)` or server admin |
| Response `204` | no body |
| Errors | `403 forbidden` · `404 not_member` |

**FR-421 — immediate effect.** The `DELETE` removes the `project_members` row inside the same
transaction that the response commits on. `require_member` re-queries `project_members` on
*every* call — it holds no cache, no session-scoped membership snapshot — so the very next
request from the removed user against any project-scoped route (a sync push already in
flight, a background pull the daemon fires on its next tick, a web page load) re-evaluates
the predicate against the now-absent row and is refused with the same `403` a stranger would
get. There is no propagation delay to reason about because there is nothing to propagate: the
predicate is recomputed, not cached, on both sides.

### `GET /api/projects/{id}/members`

| | |
|---|---|
| Auth | `CurrentUser`, `role = admin` |
| Response `200` | `{ "members": [ { "user_id", "email", "display_name", "added_by_user_id", "created_at" } ] }` |

FR-427: full membership visibility is an administrative capability, not a member one — an
ordinary member can see *that* they are a member (via `GET /api/projects`, §3) but not
necessarily the full roster, which may include people from other teams the member has no
reason to enumerate.

## 3. `GET /api/projects/lookup` — the corrected discovery route

**The current doc comment is false, and this is the fix.** Quoted from the shipped code:

```rust
// crates/cairn-server/src/api.rs:309-329
/// A discovery *hint*. Returns only projects the caller may already see, and
/// never links anything on its own (D14).
async fn lookup_projects(
    State(state): State<AppState>,
    _user: CurrentUser,
    Query(q): Query<LookupQuery>,
) -> ApiResult<Json<Value>> {
    ...
    let rows = sqlx::query(
        "SELECT id, name FROM projects
         WHERE repository_remote = $1 AND deleted_at IS NULL ORDER BY created_at",
    )
    .bind(q.remote.trim())
    .fetch_all(&state.pool)
    .await?;
    ...
}
```

The comment claims a membership filter. **The SQL has none.** `_user: CurrentUser` is bound
and never read (the leading underscore says so) — the only check performed is "does an
authenticated session exist at all," and the query returns any project whose
`repository_remote` matches, regardless of who is asking. Combined with the now-removed
`POST /api/projects/{id}/join`, this was the whole exploit chain: register (removed), look up
a project by its public git remote (this route, unfiltered), join it (removed). Removing
`join` broke the chain's last link, but this route's doc comment continues to assert a
guarantee its own SQL does not enforce, and a future addition of any join-like route would
silently reopen the hole this contract exists to close.

### The corrected query (FR-422)

```sql
SELECT p.id, p.name FROM projects p
  JOIN project_members m ON m.project_id = p.id
 WHERE p.repository_remote = $1
   AND p.deleted_at IS NULL
   AND m.user_id = $2
 ORDER BY p.created_at
```

`$2` is the caller's `user_id`, taken from the now-*used* `CurrentUser` extraction. The
response shape is unchanged (`{ "projects": [ { "id", "name" } ] }`); what changes is which
rows can appear in it. **A caller who is not a member of any project matching that remote now
gets an empty array**, exactly as an unauthenticated caller effectively could not distinguish
"no such project" from "not your project" before — except now that ambiguity is the *correct*
answer rather than a byproduct of an unfiltered query.

### `GET /api/projects` — full membership list (FR-423)

Unchanged from the existing `list_projects` (`api.rs`, joined on `project_members` already);
restated because §5 depends on it: this route returns every project the caller is a member
of, with no remote filter, and is what `cairn link` consults when it has zero or more than
one lookup match.

## 4. Safe auto-link — discovery incapable of granting anything

**FR-424, FR-425, FR-426, FR-428.** `cairn link` with no `--project` flag walks this exact
decision tree, entirely against the corrected §3 routes:

```text
cairn link  (no --project, no --create)
    │
    ▼
GET /api/projects/lookup?remote=<normalized local remote>
    │
    ├── exactly one project returned
    │     └──▶ auto-select it: POST-equivalent internal call using that project's id
    │          as if the user had typed `cairn link --project <id>`.
    │          No join call is made, because membership already exists — lookup
    │          returned it only because the caller already passed the membership
    │          filter (§3). "Auto-select" therefore never crosses a privilege
    │          boundary: it picks among rows the caller was already entitled to
    │          see and act on.
    │
    ├── zero projects returned
    │     └──▶ refuse: "No shared project matches this repository, and you are
    │          not a member of one. Ask an admin or existing member to add you
    │          (`cairn project member add`), or pass --create to make a new
    │          shared project, or --project <id> if you already know it."
    │
    └── more than one project returned
          └──▶ refuse: "N shared projects match this repository's remote and
               you are a member of all of them: <list of id, name>. Specify
               one with --project <id>."
```

**Why this is safe where the unfiltered lookup was not**: the input to the decision (§3's
result set) can now contain *only* memberships the caller already holds. Auto-link cannot
manufacture a new membership — it has no code path that calls a membership-granting
endpoint at all. "Zero or more than one match" is refused rather than guessed (FR-425)
because guessing among memberships the caller already holds is still a decision only the
human should make when it is ambiguous; the auto-select case is safe specifically *because*
it is unambiguous, not because auto-selection is inherently safe.

### The cloned-repository scenario, end to end

A developer clones a repository whose upstream Cairn project already has three human members.
The developer's own account is added as a fourth member by one of the existing three, through
`POST /api/projects/{id}/members` (§2) — out of band, e.g. over chat, with the project id or
by the admin looking it up via `GET /api/projects/lookup` (which, being called *by an existing
member*, correctly returns the project). The developer then runs `cairn link` inside their
fresh clone with no flags:

1. `cairn-git` discovers `git_common_dir` for the new clone and normalizes the remote
   (`crates/cairn-git/src/lib.rs:97-124,159-181`) — a purely local, offline step untouched by
   this feature.
2. The daemon calls `GET /api/projects/lookup?remote=<normalized remote>` using the
   developer's own bearer token.
3. The corrected query (§3) joins against `project_members` for *this* caller's `user_id` —
   which now has exactly one matching row, because step 0 (out of band) already inserted it.
4. Lookup returns exactly one project. Auto-link fires. The local `projects` row is stamped
   `linked = 1`, `server_project_id = <that id>` — the same write `cairn link --project <id>`
   would have made explicitly (`cairnd::sync::link`, `crates/cairnd/src/sync.rs:265-305`,
   unchanged by this feature).
5. Sync begins immediately: the daemon's next drain tick pushes anything queued and pulls
   the shared project's existing content, because the caller is now, and was already, a
   verified member.

At no point does the developer's own `cairn link` invocation cause a membership to exist that
did not exist a moment before it ran. The membership was always granted by someone who was
already inside the boundary — auto-link only ever *discovers and uses* that fact.

### `--project` always wins (FR-428)

`cairn link --project <uuid>` bypasses the lookup-and-auto-select path entirely and attempts
to join `<uuid>` directly — but "join" here means what it always meant for an explicit,
user-typed target: the server's join semantics for an *explicit* project id are unaffected by
this contract except that the underlying self-join route no longer exists, so an explicit
`--project <uuid>` for a project the caller is not already a member of now fails with
`403 forbidden` rather than silently granting membership. The explicit-argument code path is
never overridden by whatever auto-link *would* have chosen — if a caller passes `--project`,
auto-link's decision tree above never runs.

## 5. Local identity stays local, shared identity stays server-assigned (D14)

Unchanged by this feature, restated because §4 depends on the distinction holding:

> "What this module identifies is the **local repository instance** — Git common directory,
> worktree path, branch, commit, status. That is not the identity of a shared Cairn project:
> the same repository is `/Users/a/project` on one machine and `/home/a/project` on another,
> so shared identity is server-assigned at `cairn link` (D14)."
> — `crates/cairn-git/src/lib.rs:1-8`

Local project identity: the canonicalized absolute filesystem path of
`git rev-parse --git-common-dir`, stored as the `UNIQUE` key `projects.git_common_dir`
(`crates/cairn-store/src/repo.rs:53-98`). Two clones of the same remote at different paths
are, and remain, two different local projects — each capable of independently linking to the
*same* shared project, which is exactly the scenario in §4.

Shared identity: assigned only at `cairn link`, either as a user-supplied UUID (`--project`)
or server-minted on `--create` — never derived from path, remote, or any hash of either
(`cairnd::sync::link`, `crates/cairnd/src/sync.rs:265-305`). `remote` remains "a discovery
*hint*" only (`cairn-git/src/lib.rs:32-33`), never a lookup key on its own — §3's corrected
query still filters *by* remote, but membership, not remote match, is what actually
authorizes the result.

This feature adds no new identity concept and changes no column on `projects`. What changes
is exclusively *who* is allowed to discover a `repository_remote` match.

## 6. Sync writes must re-validate the entity's own project

**FR-506 wiring note** (this defect is closed by the prerequisite patch, not by 004 — restated
here because 004's new personal/team namespaces must not repeat it). The prerequisite patch
adds an explicit `project_id` predicate to every upsert's `ON CONFLICT` branch and to
`tombstone`, closing the specific defect at `crates/cairn-server/src/sync.rs:626-640`:

```rust
// crates/cairn-server/src/sync.rs:627-638 (before the prerequisite patch)
let sql = match entity {
    "memory" => "UPDATE memories SET deleted_at = now(), content = '' WHERE id = $1",
    // ... no project_id in any WHERE clause
};
```

A caller who is a member of *some* project could tombstone a memory belonging to an unrelated
project, because the single membership check happened once against the batch's envelope
`project_id`, never against the row actually being written. The fix, applied to every
upsert and to `tombstone` alike:

```sql
UPDATE memories SET deleted_at = now(), content = ''
 WHERE id = $1 AND project_id = $2   -- $2 = the verified project_id from the envelope
```

**This contract's obligation for the two new domains**: `personal_knowledge` has no
`project_id` column at all (structural privacy, see `global-memory.md`), so this class of
defect cannot recur for it by construction — there is no project-scoped predicate to forget
because there is no project column. `team_knowledge` likewise carries no `project_id`.
Neither domain's sync path is authorized by `require_member` at all; personal writes are
authorized by "this token belongs to the owning user" and team writes by "this token's user
is a project member somewhere" (proposal) or "an admin" (ratify/retire) — see
`sync-namespaces.md` for the namespace-level authorization and `global-memory.md` §5 for the
team lifecycle permissions.

## Invariants

1. `project_members` is the only table any route consults to decide project-scoped
   authorization; no route grants access based on server role, request origin, or any
   other signal (FR-418).
2. No route exists that inserts a `project_members` row where `user_id` equals the
   requesting caller's own id — every insert names a *different* user than the caller
   (FR-418, FR-419), verified against the full live route set rather than by asserting the
   absence of one named route (SC-465).
3. Removing a `project_members` row takes effect on the very next request that checks it;
   no cache, session, or token carries a stale membership decision (FR-421).
4. `GET /api/projects/lookup` returns only rows for which the caller already holds a
   `project_members` entry; its result set is a strict subset of what
   `GET /api/projects` would return for the same caller (FR-422, FR-423).
5. Auto-link never calls a membership-granting endpoint; it only selects among memberships
   the lookup call already proved exist for the caller (FR-424, FR-426).
6. Auto-link fires only when lookup returns exactly one project; zero or multiple matches
   always require an explicit `--project` (FR-425).
7. `cairn link --project <uuid>` is never overridden by auto-link's decision, whether or
   not auto-link would have chosen the same or a different project (FR-428).
8. Local project identity remains the canonicalized `git rev-parse --git-common-dir` path;
   shared project identity remains assigned only at `cairn link`, never derived from path
   or remote (D14, unchanged by this feature).
9. Every sync upsert and every tombstone that targets a project-scoped table carries an
   explicit `project_id` predicate matching the row being written, not only the envelope's
   verified project (prerequisite patch; restated as a standing rule this feature does not
   relax).
10. Neither `personal_knowledge` nor `team_knowledge` is authorized through
    `project_members` or `require_member` at all — see `global-memory.md` and
    `sync-namespaces.md` for their respective authorization models.
