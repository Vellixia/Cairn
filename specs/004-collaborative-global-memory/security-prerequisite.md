# Security Prerequisite: Server Authorization Hardening

**Applies to**: `main` (`v0.1.0-alpha.5`, `96178fc`), independent of Feature 004.
**Status**: required on `main` **before** Feature 004 begins (D-U2). Not a 004 task, not in
004's `tasks.md` — a standalone patch, specified here so it can be reviewed and shipped on
its own timeline.

These are **live, exploitable defects in shipped behavior**, present regardless of whether
Feature 004 ever ships. 004 declares this patch a hard prerequisite because it introduces
two knowledge domains — personal-global and team-global — meant to be trusted *more*
broadly than project memory, and building that on an authorization layer that cannot
currently keep an uninvited user out of a project at all would let the new, wider-reaching
knowledge inherit an existing hole rather than a fixed one.

---

## 1. The five defects

### 1.1 Public self-registration

**Location**: `POST /api/auth/register` → `register` (`crates/cairn-server/src/api.rs:
72-98`), unauthenticated (`api.rs:21`).

```rust
async fn register(...) -> ApiResult<impl IntoResponse> {
    if body.password.len() < 8 {
        return Err(ApiError::invalid("password must be at least 8 characters"));
    }
    ...
    sqlx::query(
        "INSERT INTO users (id, email, display_name, password_hash) VALUES ($1, $2, $3, $4)",
    )
    ...
}
```

The only gate on a full account is an 8-character password — no invitation, no approval, no
allowlist. `auth.rs:122-126` names the exposure directly: a fresh deployment has "no users,
and `/api/auth/register` is the only route that makes one — which leaves nobody able to
sign in and open[s] registration to whoever reaches the server first." The admin bootstrap
(§5) exists *because* this route is open to anyone.

**Exploit**: anyone reaching the server's HTTP port gets a full account for free.
**Blast radius**: on its own, none — but it is step 1 of the chain in §2, and every
downstream defect assumes the attacker already holds a valid, self-issued account.
**Fix**: remove public self-registration entirely; users are created only by an
authenticated admin action (§5).

### 1.2 Open project self-join

**Location**: `POST /api/projects/{id}/join` → `join_project` (`api.rs:278-299`).

```rust
async fn join_project(..., user: CurrentUser, Path(id): Path<Uuid>) -> ApiResult<Json<Value>> {
    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM projects WHERE id = $1")
        .bind(id).fetch_optional(&state.pool).await?;
    if exists.is_none() { return Err(ApiError::not_found("no such shared project")); }
    sqlx::query(
        "INSERT INTO project_members (project_id, user_id) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(id).bind(user.id).execute(&state.pool).await?;
    Ok(Json(json!({ "id": id, "joined": true })))
}
```

The only check is "does this project row exist." Any authenticated user becomes a full
member of *any* project by UUID — no invitation, no approval, no notice to existing
members. `cairnd/src/sync.rs:263-267` frames joining as "the user picks" (D14), a
human-in-the-loop CLI decision, but that restraint lives entirely in the CLI; the server
enforces nothing beyond existence, so any client that skips the CLI bypasses it.

**Exploit**: `POST /api/projects/{uuid}/join` with any bearer token and any project UUID.
**Blast radius**: full membership — every `require_member`-gated route (tasks, sessions,
full memory `content`, handoffs, sync history) for a project never joined by invitation.
**Fix**: membership is granted by an existing member/admin action
(`POST /api/projects/{id}/members`, §5's endpoint list), never by the joining user's own
request against a bare UUID.

### 1.3 Membership-blind lookup, contradicting its own doc comment

**Location**: `GET /api/projects/lookup` → `lookup_projects` (`api.rs:307-329`).

```rust
/// A discovery *hint*. Returns only projects the caller may already see, and
/// never links anything on its own (D14).
async fn lookup_projects(..., _user: CurrentUser, Query(q): Query<LookupQuery>) -> ... {
    let rows = sqlx::query(
        "SELECT id, name FROM projects
         WHERE repository_remote = $1 AND deleted_at IS NULL ORDER BY created_at",
    )
    ...
}
```

The comment claims "only projects the caller may already see." The SQL has **no
`project_members` join and no membership predicate** — it matches purely on
`repository_remote`, typically a public git URL. Doc comment and body directly contradict
each other.

**Exploit**: any authenticated caller supplies a `repository_remote` string they can
observe (a public GitHub URL) and gets back the project's `id` — the exact UUID §1.2 needs
— with zero membership prerequisite.
**Blast radius**: converts "I know a project's public remote" into "I have its UUID,"
bridging §1.1 to §1.2.
**Fix**: add a membership filter (`JOIN project_members ... WHERE user_id = $caller`) so
the endpoint can only ever return projects the caller already belongs to, matching its own
doc comment. This is also what makes safe auto-link possible (brief §5): once lookup cannot
return a project the caller isn't already in, `cairn link` with no `--project` can safely
auto-select a unique match.

### 1.4 Unscoped tombstone

**Location**: `tombstone` (`cairn-server/src/sync.rs:626-640`), reachable via a `delete`
item in `POST /api/sync/batch`.

```rust
async fn tombstone(tx: &mut Transaction<'_, Postgres>, entity: &str, id: Uuid) -> ApiResult<()> {
    let sql = match entity {
        "memory" => "UPDATE memories SET deleted_at = now(), content = '' WHERE id = $1",
        "handoff" => "UPDATE handoffs SET deleted_at = now(), goal = '', progress = '', ...",
        "session" => "UPDATE sessions SET deleted_at = now(), end_reason = NULL WHERE id = $1",
        "task" => "UPDATE tasks SET deleted_at = now() WHERE id = $1",
        "project" => "UPDATE projects SET deleted_at = now() WHERE id = $1",
        other => return Err(ApiError::invalid(format!("cannot delete {other}"))),
    };
    sqlx::query(sql).bind(id).execute(&mut **tx).await?;
    Ok(())
}
```

Every branch's `WHERE` names only `id = $1` — **no `project_id` predicate at all** — even
though `sync_batch` verifies membership on exactly one `project_id` (the envelope's,
`sync.rs:106-113`), never the target row's actual project.

**Exploit**: a member of *any* project submits a batch under that project's id containing a
`delete` naming an `entity_id` belonging to a different project. Membership passes (the
envelope's project is genuine); the tombstone fires against the other project's row anyway.
UUIDs are UUIDv7 (time-ordered, not fully random), narrowing the guessing space for an
attacker who can bound the target's creation time.
**Blast radius**: destructive — for `memory`/`handoff`, content is irreversibly blanked in
the same statement that soft-deletes it; for `session`/`task`/`project`, the row is
soft-deleted — all by a caller with no legitimate relationship to that project.
**Fix**: every branch gains `AND project_id = $2`, bound to the envelope's verified
`project_id`.

### 1.5 Sync upserts that never re-verify entity ownership

**Location**: `upsert_task` (`sync.rs:382-398`), `upsert_memory` (`:434-565`),
`upsert_handoff` (`:571-623`), `upsert_criterion` (`:953-988`), `upsert_blocker`
(`:991-1023`).

```rust
"INSERT INTO tasks (id, project_id, title, goal, acceptance_criteria, status, updated_at)
 VALUES ($1, $2, $3, $4, $5, $6, now())
 ON CONFLICT (id) DO UPDATE SET
     title = EXCLUDED.title, goal = EXCLUDED.goal,
     acceptance_criteria = EXCLUDED.acceptance_criteria,
     status = EXCLUDED.status, updated_at = now()"
```

`sync_batch` verifies membership on `body.project_id` once, up front (`sync.rs:106-113`).
The `ON CONFLICT (id) DO UPDATE` never re-checks the existing row's own `project_id` —
which is not even in the `SET` list — against the verified one. All five upserts share this
shape.

**Exploit**: a member of project A submits a batch with `project_id = A` and an
`entity_id` known/derived to belong to project B; the upsert overwrites B's row without
ever checking B against A's membership.
**Blast radius**: content overwrite (not deletion) of another project's task/memory/
handoff/criterion/blocker, by a caller verified only against a *different* project. Requires
already knowing/deriving cross-project UUIDs — no enumeration endpoint exists for these
entities — so §1.2/§1.3's join-then-read path is the more directly exploitable route, but
this is an independent defect: `require_member` checks the envelope, never the entity, in
all five functions.
**Fix**: each upsert adds an ownership re-check — a `WHERE project_id = $N` predicate on the
conflict branch (rejecting/no-op'ing on mismatch), or a pre-check query confirming the
target row's `project_id`, if it exists, matches the envelope before the upsert runs.

---

## 2. The full exploit chain

1. **Register** — `POST /api/auth/register`, any email, 8-char password (§1.1). A valid,
   bearer-token-eligible identity in one request.
2. **Lookup by a public git remote** — `GET /api/projects/lookup?remote=<public-url>`
   (§1.3). The target's remote is often public (a public repo, a pasted URL, a guessable
   name). Returns the project's `id` with zero membership check.
3. **Join** — `POST /api/projects/{id}/join` with that UUID (§1.2). Immediate, full,
   unapproved membership.
4. **Read and write everything** — every `require_member`-gated route now succeeds (tasks,
   sessions, full memory `content`, handoffs). Writes via `POST /api/sync/batch` succeed
   against this project, and — per §1.4/§1.5 — can reach *other* projects' rows the
   attacker was never a member of, given a known or derived UUID.

Every step needs only network access to the server and a project's public git remote — no
social engineering, no credential theft, no privilege the server doesn't hand out by design.

---

## 3. Explicit non-finding: `api_tokens.token_hash` hashing is correct as-is

`token_hash` is `cairn_core::digest` — plain SHA-256, not Argon2
(`crates/cairn-server/src/auth.rs:78-80`). **This is not a defect and must not be "fixed."**
A minted token is 32 bytes of CSPRNG entropy, hex-encoded
(`auth::random_token`, `auth.rs:82-86`). Argon2 defends a *low-entropy* secret (a
human-chosen password) against offline brute-force by making each guess expensive; a
256-bit random token has no meaningfully guessable space for a slow hash to defend, so
Argon2 here would be pure per-request overhead with no security benefit. `password_hash`
already correctly uses Argon2 (`auth.rs:88-94`) for the one field that actually needs it —
the two fields are already hashed appropriately for what each protects.

The only token-related change 004 makes is adding an optional `expires_at` column to
`api_tokens` (D-U2/brief §3) — a lifecycle addition, orthogonal to this non-finding.

---

## 4. Missing foreign keys that widen the blast radius

| Column | Names | Location |
|---|---|---|
| `sessions.task_id` | `tasks.id` | `cairn-server/migrations/0001_init.sql:74` |
| `sessions.previous_session_id` | `sessions.id` | `0001_init.sql:79` |
| `memories.origin_session_id` | `sessions.id` | `0001_init.sql:103` |
| `memories.superseded_by_id` | `memories.id` | `0001_init.sql:101` |
| `handoffs.session_id` | `sessions.id` | `0001_init.sql:120` |
| `memory_relations.from_memory_id`/`to_memory_id` | `memories.id` | `0002_project_intelligence.sql:54-55` |

Each is a raw `UUID` column with no `REFERENCES`. This does not by itself grant access, but
it widens §1.4/§1.5's blast radius: the database does not enforce that these ids point at a
row in the same project, or at a row that exists at all, so an exploit can leave them
pointing at soft-deleted, blanked, or cross-project rows with no constraint violation to
surface the inconsistency. A foreign key does not close §1.4/§1.5 alone — the
application-level `project_id` check is still required — but it is the difference between
an authorization bug producing a silently inconsistent database and one the database itself
refuses to represent.

**Fix**: add the five FKs above. The `memory_relations` pair is deliberately left
unconstrained by 003 to tolerate out-of-order delivery (`0002_project_intelligence.sql:
53-69`) — leave that pair as-is; it is not part of this fix.

---

## 5. Removing self-registration requires the admin bootstrap to keep working

Closing §1.1 removes the *only* way this server creates a first account today. `auth::
ensure_admin` (`auth.rs:131-176`), run from `main.rs`'s `seed_admin` at every start when
`CAIRN_ADMIN_EMAIL`/`CAIRN_ADMIN_PASSWORD` are set, already exists to solve exactly this —
its own comment (`auth.rs:122-126`) says so. It upserts by email and re-applies the password
every start (`auth.rs:150-166`), the environment being the declared source of truth.

**Recommendation: keep the env-seeded admin bootstrap; remove only self-registration.**
They solve different problems — `ensure_admin` gives a fresh deployment its first,
operator-controlled account with no HTTP surface at all; `/api/auth/register` gives *any*
network caller one, which is what must close. Keeping the bootstrap means a fresh server
still boots with exactly one admin account as it does today, every subsequent account comes
from that admin via `POST /api/admin/users` (brief §4), and no existing deployment's
`CAIRN_ADMIN_EMAIL`/`CAIRN_ADMIN_PASSWORD` configuration needs to change.

---

## 6. Verification checklist

Each fix ships with a test that would fail without it. `tests/tests/scope_audit.rs:376-398`
is the cautionary example of why this matters: it splits `search.rs`'s source on the
literal `"fn scope_bucket"`, which does not exist (the logic is an inline SQL `CASE`), so
`.unwrap_or_default()` yields `""` and its four `assert!(!bucket.contains(...))` checks pass
against the empty string — permanently, regardless of what the code does. It has passed on
every commit since it was written and would keep passing if the behavior it claims to guard
were reintroduced tomorrow. A dead test is indistinguishable in CI output from a working
one; none of the checks below may be built that way.

- **Self-registration removed**: `POST /api/auth/register` returns 404/410; no code path
  can construct a `users` row from an unauthenticated request.
- **Admin bootstrap still works**: a fresh database plus the two env vars produces exactly
  one usable admin account after startup, and re-running with the same vars does not create
  a second (`AdminOutcome::Updated`, `auth.rs:110-121`, must still fire post-patch). A fresh
  database with no admin env vars and self-registration removed must leave a detectable,
  operator-visible zero-account state, not a silent dead end.
- **Self-join removed**: `POST /api/projects/{id}/join` against a project the caller has no
  relationship to returns 403/404; the replacement admin/member-add endpoint is the only
  path that inserts a `project_members` row for anyone but the project's creator.
- **Lookup is membership-scoped**: two users, two projects, no shared membership — `GET
  /api/projects/lookup?remote=...` for user A must never return user B's project, even
  supplying B's exact `repository_remote`. Must exercise the real SQL, not the doc comment.
- **Tombstone is project-scoped**: two projects, one entity per type (`memory`, `handoff`,
  `session`, `task`, `project`) under project A — a `delete` submitted under project B's
  verified membership must fail or no-op, and A's row must be unchanged (content intact,
  `deleted_at` still null) afterward.
- **Upserts are ownership-checked**: for each of the five upserts, an item naming an
  `entity_id` under a *different* verified project must fail or be rejected, and the
  existing row's fields must be unchanged afterward — not just an error code, the actual
  row content.
- **`scope_audit.rs:376-398` repaired**: must locate the real scope-precedence logic (the
  SQL `CASE` in `search.rs`, not a nonexistent `fn scope_bucket`) and assert against its
  actual content, so a single-character edit wiring `importance`/`pinned`/`verification`/
  `verification_authority` into scope ranking makes the test fail — a live test, not a
  vacuous one.
- **Missing FKs added**: a migration test that each of the five columns in §4 now rejects
  an insert naming a nonexistent target row (`memory_relations`' endpoints excluded, by
  design).
- **Token hashing unchanged**: an explicit assertion that `auth::hash_token` remains
  `cairn_core::digest`, not converted to Argon2 — "fixing" something already correct is
  itself a regression this checklist guards against, adding latency with no security gain.
