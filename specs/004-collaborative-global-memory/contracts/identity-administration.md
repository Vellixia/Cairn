# Contract: Identity and Administration

**Feature**: `004-collaborative-global-memory`

This contract guarantees that every account on a Cairn server traces to an administrator's
deliberate act, that a freshly issued credential cannot be used for anything beyond changing
itself, that disabling an account removes its access at that instant rather than at the next
token expiry, and that the server can never end up with nobody able to administer it. It
assumes the prerequisite hardening patch (`security-prerequisite.md`) has already removed
public self-registration and unrestricted project joining from `main`; this contract replaces
what those routes did with an administrator-driven model.

## 1. Roles, status and the account row

`users` gains three columns (server migration `0003_collaborative_global_memory.sql`):

| Column | Type | Default | Meaning |
|---|---|---|---|
| `role` | `TEXT CHECK ('admin','member')` | `'member'` | Server-level standing (FR-402). Not per-project — `project_members` still carries no role (`crates/cairn-server/migrations/0001_init.sql:48`, "a user is a member or is not"). |
| `status` | `TEXT CHECK ('active','disabled')` | `'active'` | Whether the account can authenticate at all (FR-408). |
| `must_change_password` | `BOOLEAN` | `false` | Set on every admin-created account; cleared only by a successful password change (FR-404, FR-405). |

`password_changed_at` is added alongside for audit and is not read by any authorization
decision.

There is no per-project role. `ServerRole` (`admin | member`) and `UserStatus`
(`active | disabled`) are new `cairn-core` enums, each following the existing `text_enum!`
pattern already used for `MemoryScope` (`crates/cairn-core/src/domain.rs:112-120`) and
`OutboxEntityType` (`domain.rs:164-174`): exhaustive, serde `snake_case`, `FromStr`.

## 2. Account creation — admin-only, temporary password

**Deleted** (prerequisite patch): `POST /api/auth/register`
(`crates/cairn-server/src/api.rs:72-98`). No unauthenticated or self-service path to a user
row survives anywhere on the server (FR-401).

### `POST /api/admin/users`

| | |
|---|---|
| Auth | `CurrentUser`, `role = admin` |
| Request | `{ "email": string, "display_name": string }` |
| Response `201` | `{ "id": uuid, "email": string, "display_name": string, "role": "member", "status": "active", "temporary_password": string }` |
| Errors | `403 forbidden` (not admin) · `409 email_taken` |

The server generates the temporary password (the same `random_token`-class generator used
for API tokens, `crates/cairn-server/src/auth.rs:82-86`, hex-encoded and truncated to a
human-typeable length rather than reused as a bearer token) and returns it **once**, in this
response body, hashed with Argon2 into `password_hash` exactly as a self-chosen password
would be (FR-403). `must_change_password` is set `true` at the same insert. There is no
endpoint that retrieves a temporary or current password after creation — only reset-by-admin
(below) produces a new one.

### `GET /api/admin/users`

| | |
|---|---|
| Auth | `CurrentUser`, `role = admin` |
| Response `200` | `{ "users": [ { "id", "email", "display_name", "role", "status", "must_change_password", "created_at" } ] }` |

FR-411: every account, its role and its status, in one call. No pagination is specified;
the existing `projects`/`tokens` list endpoints are unpaged and this follows the same
precedent.

### `PATCH /api/admin/users/{id}`

| | |
|---|---|
| Auth | `CurrentUser`, `role = admin` |
| Request | `{ "role"?: "admin"\|"member", "status"?: "active"\|"disabled" }` (either or both) |
| Response `200` | the updated user row, same shape as the list entry |
| Errors | `403 forbidden` · `404 not_found` · `409 last_admin` (demoting or disabling the server's only remaining admin) |

Promote/demote (FR-412) and disable/enable (FR-408) share one endpoint because both are
"change this account's standing," and both are subject to the same last-admin guard.

## 2a. Administrator password reset (D435, FR-553–FR-559)

Nothing in the shipped design let an administrator recover a member's account that had lost
or forgotten its temporary password before ever using it — the spec referred to
"resetting" a confined account, but no route implemented it. This section adds one.

### `POST /api/admin/users/{id}/reset-password`

| | |
|---|---|
| Auth | `CurrentUser`, `role = admin` |
| Request | *(empty body)* |
| Response `200` | `{ "id": uuid, "email": string, "temporary_password": string }` |
| Errors | `403 forbidden` (not admin) · `404 not_found` · `409 env_admin_reset_refused` (target is the environment-named account, see below) |

**FR-553.** Administrators can reset any other account's password through this one route.

**FR-554 — revealed exactly once.** The new temporary password is generated the same way
account creation's is (§2, the `random_token`-class generator, hex-encoded and truncated to a
human-typeable length) and appears **only** in this response body. It is hashed with Argon2
into `password_hash` at the same moment, exactly as account creation does, and there is no
route — including one restricted to the administrator who performed the reset — that can
retrieve it again afterward. This is the same "revealed once, never again" discipline §2
already applies to account creation, extended to reset (cross-referenced fully in §4a).

**FR-555 — the old password stops working immediately.** `password_hash` is overwritten in
the same statement that records the new temporary password; the account's previous password,
whatever it was, no longer authenticates from the instant this request commits.

**FR-556 — every token revoked.** In the same transaction as the password update:

```sql
UPDATE users SET password_hash = $2, must_change_password = true WHERE id = $1;
UPDATE api_tokens SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL;
```

This is the identical defense-in-depth shape §6 already uses for disabling — a cached bearer
token must not survive a credential reset any more than it survives a disable, because both
are "this account's standing has been forcibly changed by an administrator" events.

**FR-557 — back into the lockout.** `must_change_password` is set `true` by the same
statement, so the account re-enters exactly the state a newly created account starts in (§4):
the only reachable route is `POST /api/auth/password`, until the member sets a password of
their own choosing.

**FR-558 — resetting a disabled account does not re-enable it.** The reset touches
`password_hash`, `must_change_password`, and `api_tokens.revoked_at` only; it never touches
`status`. A disabled account that is reset remains `status = 'disabled'` and remains refused
authentication by every path (§6, FR-410) — the new temporary password authenticates to
nothing, because `CurrentUser`'s `status = 'active'` check runs before `must_change_password`
is ever consulted.

**Why re-enabling on reset would be wrong, not merely unspecified**: a password reset and an
enable/disable decision are two different administrative judgments — "this account's
credential needs to change" says nothing about "this account should be allowed to
authenticate at all." Coupling them would mean a credential operation could silently reverse
an access decision an administrator made deliberately and separately (possibly for reasons
unrelated to the current admin performing the reset — a different administrator, a policy
review, an incident response). An administrator who wants both must perform both: reset the
password, then separately `PATCH /api/admin/users/{id}` with `status: "active"`. This mirrors
§6's own precedent exactly — re-enabling already does not restore revoked tokens, for the
same reason: two distinct decisions must not be conflated by one action doing both.

**FR-559 — the environment-named account refuses reset.** Exactly as `cairn user disable`
and the demote path already refuse `CAIRN_ADMIN_EMAIL`'s account outright (§3a, D429), reset
does too, and for the identical reason: that account's password is re-established from the
environment on every process start (`auth.rs:128-130`), so a reset would be silently undone by
the very next restart, leaving the operator believing a reset had taken effect when it had
not. The refusal names the environment setting, matching §3a's existing wording:

```text
Refused: alice@example.com is this server's environment-defined administrator
(CAIRN_ADMIN_EMAIL). Its password is controlled by CAIRN_ADMIN_PASSWORD and re-applied
on every server start; resetting it here would be silently undone. Change
CAIRN_ADMIN_PASSWORD and restart the server instead.
```

**SC-442 and SC-443** are this section's verification obligations: an administrator resets a
member's password, the old password immediately fails, the new temporary password
authenticates only to the password-change route, and every token the member held is refused
(SC-442); resetting a disabled account's password leaves it disabled, verified by attempting
authentication with the new temporary password and being refused (SC-443, FR-558).

## 3. The last-admin guarantee (D436 — atomic, not read-count-then-update)

**FR-413 (replaced), FR-560.** No action may leave the server with zero administrators, and
this guarantee is enforced **atomically, inside the single statement** that performs the
demotion or disable — conditioned on another active administrator still existing at the
instant that statement takes effect — never as a `SELECT` that counts admins followed by a
separate `UPDATE` that trusts what the count said, even when both live in the same
transaction.

**Why a `SELECT` followed by an `UPDATE` is insufficient, with the concrete race.** Suppose
Alice and Bob are the server's only two active administrators, and two requests arrive
concurrently: one demoting Alice, one demoting Bob (this is exactly SC-444's scenario — it is
also exactly as plausible as one admin demoting themselves while a colleague demotes them at
the same moment, not a contrived case). Under a read-count-then-update implementation, at
ordinary `READ COMMITTED` isolation:

```text
T1 (demote Alice)                          T2 (demote Bob)
--------------------------------------     --------------------------------------
SELECT count(*) WHERE id <> Alice          SELECT count(*) WHERE id <> Bob
  → sees Bob, still active → count = 1       → sees Alice, still active → count = 1
count > 0, so proceed                       count > 0, so proceed
UPDATE users SET role='member'              UPDATE users SET role='member'
  WHERE id = Alice                            WHERE id = Bob
COMMIT                                      COMMIT
```

Both `SELECT`s run before either `UPDATE` commits, so both see the *other* admin as still
active, both pass their own count check, and both `UPDATE`s succeed — the server ends with
**zero** active administrators, even though each individual demotion, read in isolation,
looked perfectly legal against the state its own transaction observed. This is a classic
write-skew anomaly: the two transactions never touch the same row, so no ordinary row lock
ever forces one to wait for the other, yet their combined effect violates an invariant that
spans both rows.

**The fix: serialize on one advisory lock, then apply a conditional statement** (D445, FR-574).

```sql
-- Every transaction that could reduce the active-admin count takes this lock first.
-- One fixed key, transaction-scoped, released on commit or rollback.
SELECT pg_advisory_xact_lock(4770040001);

UPDATE users
   SET role = 'member'        -- or: SET status = 'disabled', identical shape
 WHERE id = $target_id
   AND EXISTS (
       SELECT 1 FROM users
        WHERE role = 'admin' AND status = 'active' AND id <> $target_id
   )
RETURNING id;
-- 0 rows returned => refused: this account is the last active admin.
```

There is no separate read step whose result the write later trusts — the `EXISTS` subquery
and the `UPDATE` are one statement, evaluated together. The advisory lock is what closes the
race the SQL shape alone cannot: T2 blocks on `pg_advisory_xact_lock` before it evaluates
anything at all, and resumes only once T1 has committed and released it. T2 then evaluates
`EXISTS` against the committed state, finds no other active admin, matches zero rows, and is
refused. The API layer reports the empty result as `409 last_admin` to the
loser, exactly as if its own `EXISTS` check had failed outright; the winner's demotion
commits normally. `RETURNING id` returning zero rows (the ordinary, non-concurrent refusal
path — the caller's own count truly is zero) and a caught serialization failure (the
concurrent-race path) are both surfaced to the caller as the same `409 last_admin` — the
caller does not need to distinguish which one happened, only that its change did not take
effect and no admin was lost.

**FR-560.** Two concurrent operations that would each individually be legal, but that
together would remove the last administrator, always result in exactly one succeeding and one
refused — never both succeeding and never both refused (a legal demotion must not be blocked
by a concurrent operation that itself gets rolled back).

The check is `id <> $target_id` — an admin who is not the last one may always demote or
disable themselves; the guard exists for the *server*, not to prevent self-service.

### Deterministic backfill (FR-414)

Migration `0003` backfills `role` for every pre-existing account with a single deterministic
rule, run once, in order:

1. If `CAIRN_ADMIN_EMAIL` is set and matches an existing account's `email` (case-sensitive,
   the same comparison `auth::ensure_admin` already uses, `crates/cairn-server/src/auth.rs:
   131-176`), that account becomes `role = 'admin'`.
2. Otherwise, the account with the earliest `created_at` becomes `admin`.
3. Every other existing account becomes `member`.

This is deterministic given the state of `users` and the environment at migration time, and
it never produces zero admins because step 2 always names exactly one row when step 1 does
not fire and `users` is non-empty. A server with zero pre-existing users backfills nothing
and admits its first admin through the ordinary `POST /api/admin/users` path against an
operator-provisioned bootstrap credential — the same `CAIRN_ADMIN_EMAIL` /
`CAIRN_ADMIN_PASSWORD` env seeding already performed by `auth::ensure_admin`
(`auth.rs:131-176`, run from `main.rs`'s `seed_admin`, `main.rs:170-194`) continues
unchanged and is what creates that first row on a truly empty server.

## 3a. The env-seeded admin is the break-glass path (D429)

`ensure_admin` (`crates/cairn-server/src/auth.rs:131-176`) upserts the account named by
`CAIRN_ADMIN_EMAIL`/`CAIRN_ADMIN_PASSWORD` on **every** process start — its own doc comment
says so directly:

> "The environment is the source of truth, so this re-applies the password on every start:
> rotating it means editing the variable and restarting."
> — `auth.rs:128-130`

and the upsert itself, today, touches only two columns on conflict:

```rust
// crates/cairn-server/src/auth.rs:150-156
ON CONFLICT (email) DO UPDATE
    SET password_hash = EXCLUDED.password_hash,
        display_name  = EXCLUDED.display_name
```

**This feature extends that `SET` clause to also assign `role = 'admin'` and
`status = 'active'`.** The rationale is the same one already written for why this account
exists at all — "Naming the account in the environment closes both gaps" (`auth.rs:124-126`,
referring to the two gaps of "nobody can sign in" and "open registration to whoever arrives
first"). A third gap opens once roles and disabling exist: an operator (or an admin acting on
the wrong row, or a scripting mistake) could demote or disable the one account whose
credentials are held outside the database entirely — and without this extension, that would
brick administration with no recovery path, because the next restart would restore the
*password* (the existing upsert already does that) but leave `role = 'member'` or
`status = 'disabled'` untouched, since neither column exists in today's `SET` list. Extending
the `SET` clause closes that: every process start unconditionally re-asserts that the
env-named account is `admin` and `active`, regardless of what the database currently says.

**The env-seeded admin is exempt from `must_change_password`.** `ensure_admin`'s upsert never
sets `must_change_password = true`, and no other code path sets it for this specific account.
This is the deliberate other half of the same reasoning, not an oversight: the environment is
already the source of truth for this account's password, and `ensure_admin` already
re-applies it on every start. Forcing a password change would be reverted on the very next
restart — an unbreakable loop in which the account can never finish the lockout it started
in. **Every account created through `POST /api/admin/users` / `cairn user create`, by
contrast, does go through the ordinary `must_change_password` flow (§4) with no exemption.**
The asymmetry is intentional: one account's credential lives in the environment and is
re-asserted by the process; every other account's credential lives only in the database and
is chosen once, by a human, through the normal flow — so only the second kind needs the
lockout that flow exists to enforce.

**`cairn user disable` and the demote path refuse this account outright.** `PATCH
/api/admin/users/{id}` and the CLI's `cairn user disable <email>` / `cairn user demote <email>`
check, before anything else, whether `{id}`'s email matches the server's configured
`CAIRN_ADMIN_EMAIL`. If it does, the request is refused — not merely warned against — with:

```text
Refused: alice@example.com is this server's environment-defined administrator
(CAIRN_ADMIN_EMAIL). Its role and status are controlled by the server's environment,
not by this API. Change or unset CAIRN_ADMIN_EMAIL and restart the server instead.
```

A silent revert on the next restart would be a worse outcome than an explicit refusal now: an
operator who believed they had disabled the account would have no indication that the next
deploy or crash-restart undid it. Refusing up front means the only way to actually change
this account's standing is the one already-documented way — edit the environment and
restart — which is truthful about where the authority actually lives.

**The trust statement this implies, written down rather than left implicit**: whoever can set
`CAIRN_ADMIN_EMAIL` and `CAIRN_ADMIN_PASSWORD` in the server's environment and restart the
process can always obtain an active administrator account on that server, regardless of any
role or status change made through the API. For a self-hosted, single-team deployment this is
the correct boundary — that operator already controls the host process and the database
directly, so this grants no capability they did not already have — but it is the edge of the
entire role model as specified here, and a deployment that wants a stricter boundary (no
environment-level break-glass at all) is out of scope for this feature.

**This is what makes the never-zero-admins guarantee (FR-413, §3) hold at *runtime*, not only
at migration time.** The migration backfill (§3) guarantees the server *starts* with at least
one admin. D429 is the separate, ongoing guarantee that an operator can always *recover* one,
at any later point, without direct database access — by restarting the process with the
right environment variables set. The two rules compose: backfill answers "how does the first
admin get here," D429 answers "what happens if every admin is later locked out."

**Reconciling this with FR-401.** FR-401 requires that "every user account MUST be created by
an administrator; no unauthenticated or self-service path may create one." The env-seeded
account does not violate this: it is not created through any HTTP route at all, self-service
or otherwise — `ensure_admin` runs from the server process's own startup
(`main.rs`'s `seed_admin`, `main.rs:170-194`), reading configuration the *operator* set on the
host running the server. That is operator provisioning through the deployment's own
configuration surface, the same category of act as running the initial database migration or
setting the listen port — not a "self-service" path a caller reaches over the network. FR-401
governs routes reachable by a user; the environment is reachable only by whoever already
controls the server's deployment, which is the same trust boundary D429 already names above.

## 4. The `must_change_password` lockout

**FR-404, FR-407**. While a user's `must_change_password = true`:

| Route | Behavior |
|---|---|
| `POST /api/auth/password` | **The only reachable route.** Accepts the change; see §5. |
| `GET /api/health`, `GET /api/version` | Unaffected — these never require `CurrentUser` and carry no account-specific content. |
| Every other authenticated route (`GET /api/auth/me`, `POST/GET /api/tokens`, all `/api/projects/*`, `/api/sync/*`, `/api/admin/*`) | `403 password_change_required` |
| `POST /api/tokens` (mint an API token) | `403 password_change_required` — explicitly, even though it is otherwise an ordinary authenticated route |

The check runs once, centrally, as an extractor-level gate immediately after `CurrentUser`
resolves the account (alongside the existing `CurrentUser` extractor at
`crates/cairn-server/src/auth.rs:32-53`), before any handler body runs — so a new route added
later inherits the lockout automatically rather than needing to remember it.

**Why token minting is named explicitly rather than left to fall out of the general rule**:
an API token, once minted, is a long-lived bearer credential independent of the password that
created it. If `POST /api/tokens` were reachable during the lockout, an attacker who obtained
only the one-time temporary password (over a compromised channel, from a support ticket, from
shoulder-surfing) could mint a token that outlives the temporary password entirely and never
requires knowing the real password at all. Refusing token minting during the lockout closes
that laundering path: the *only* thing a temporary password can produce is a permanent
password, never a permanent credential.

Web sessions (`web_sessions`, `crates/cairn-server/migrations/0001_init.sql:17-22`) are
likewise refused: `POST /api/auth/login` still succeeds (the account is not disabled), but
the same central gate applies to every subsequent request the resulting session cookie
carries, so a logged-in-but-locked account can reach only the password-change screen.

## 4a. The temporary-credential lifecycle, stated once, in full (D440)

**FR-403 (replaced), FR-407, FR-572, FR-573.** A temporary password — whether it came from
account creation (§2) or from an administrator reset (§2a) — has exactly one lifecycle, with
no route anywhere that reads a step out of order:

1. **Revealed exactly once**, in the creation response or the reset response itself, and
   nowhere else, ever. It is hashed with Argon2 into `password_hash` at the moment it is
   generated; the plaintext is never persisted, logged, or made retrievable through any
   later route — including a route restricted to the same administrator who generated it
   (FR-403, FR-554).
2. **Authenticates only to `POST /api/auth/password`** while `must_change_password = true`
   (§4) — it cannot mint a token, cannot reach any project route, cannot reach any admin
   route, cannot do anything except change itself (FR-407).
3. **Invalidated the instant the password change succeeds** — `password_hash` is
   overwritten by the user's own chosen password in the same statement that clears
   `must_change_password`, so the temporary credential does not linger as a valid-but-unused
   alternate password after the real one is set (FR-572).
4. **The only way to obtain a new one is an administrator reset** (§2a) — there is no
   "resend" or "regenerate" route a user can reach themselves; a lost or expired temporary
   password requires an administrator's act, by the same reasoning FR-401 requires an
   administrator's act to create the account in the first place (FR-573).

Every word above applies identically whether the temporary credential came from creation or
from reset — this section exists so that guarantee is stated once, rather than once per
endpoint with the risk of the two copies drifting apart.

## 5. Changing password

### `POST /api/auth/password`

| | |
|---|---|
| Auth | `CurrentUser` (bearer token or session cookie) — reachable regardless of `must_change_password` |
| Request | `{ "current_password": string, "new_password": string }` |
| Response `200` | `{ "ok": true }` |
| Errors | `401 invalid_credentials` (current password wrong) · `422 password_too_short` (new password `< 8` chars, the existing rule from the deleted `register` handler) |

On success (FR-405): `password_hash` is re-derived with Argon2 exactly as at creation,
`must_change_password` is set `false`, `password_changed_at = now()`, and — because a
password change is a credential rotation — **every existing web session for this user is
destroyed** (`destroy_web_session`-style delete, not merely marked, following
`auth.rs:221-227`'s existing outright-delete precedent for logout). API tokens are *not*
revoked by an ordinary password change (only account disabling revokes tokens, §6) — a
token is a separate credential the user chose to mint, and revoking every token on every
routine password change would be a much larger blast radius than the lockout this endpoint
is closing.

## 6. Disabling revokes tokens at that instant

**FR-409, FR-410**. `PATCH /api/admin/users/{id}` with `status: "disabled"` does, in one
transaction:

```sql
UPDATE users SET status = 'disabled' WHERE id = $1;
UPDATE api_tokens SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL;
```

Both statements commit together. This is deliberate defense in depth rather than relying on
`status` alone: `user_for_api_token` (`crates/cairn-server/src/auth.rs:178-195`) already
checks `revoked_at IS NULL` on every lookup, so even if a future code path resolved a token to
a user without re-checking `status`, the token itself is already dead. **A cached token must
not outlive the account it was issued to** — an agent's `cairnd` holding a bearer token in
`sync_meta`/keychain has no way to learn synchronously that the account was disabled except by
the next request failing, and that next request must fail rather than succeed on a stale
cache.

Re-enabling (`status: "active"`) does **not** restore the revoked tokens — `revoked_at` is not
cleared. A re-enabled user mints fresh tokens through the ordinary `POST /api/tokens` flow
(FR-590, SC-470).

This was decided here and stated nowhere else for most of the design's life: no requirement
covered it and no criterion asserted it, so the property held by the intention of whoever wrote
this paragraph. That is not enough for a credential-lifetime rule — an implementer clearing
`revoked_at` alongside `status` would be making a plausible-looking change that resurrects every
token an account held before it was disabled, and every existing test would still pass, because
they all assert the *disable* side. `SC-470` asserts the re-enable side separately for exactly
that reason.
This mirrors existing token semantics: revocation (`DELETE /api/tokens/{id}` →
`revoke_token`, `api.rs:232-240`) has never been reversible.

Disabled-account authentication is refused **by any means** (FR-410): `CurrentUser`'s bearer
and cookie paths both resolve to a `user_id` first and must then check `status = 'active'`
before returning `Ok`, so a still-valid password used against `POST /api/auth/login` for a
disabled account fails with the same `401` a wrong password would produce — the response does
not distinguish "wrong password" from "disabled account," which avoids confirming account
existence to an unauthenticated caller.

## 7. API token expiry

**FR-417**. `api_tokens` gains `expires_at TIMESTAMPTZ NULL`.

### `POST /api/tokens` (extended)

| | |
|---|---|
| Request | `{ "name"?: string, "expires_at"?: RFC3339 timestamp }` — `expires_at` omitted or `null` means no expiry, exactly today's behavior |
| Response | unchanged shape, `expires_at` added to the returned row |

`user_for_api_token` (`auth.rs:178-195`) gains one additional predicate alongside its
existing `revoked_at IS NULL` check: `AND (expires_at IS NULL OR expires_at > now())`.

**An expired token is refused indistinguishably from a revoked one — identical status, identical
body** (FR-585, SC-452, D453). No separate `410` or `"expired"` error code is introduced, and that
is a security property rather than a simplification. A distinguishable refusal tells whoever holds
a stale token that it was *once valid for this server*, which is an oracle about both the server's
history and the account it belonged to. Nothing legitimate needs the distinction: the remedy for
either refusal is the same — obtain a new token from an administrator. `SC-452` asserts the two
responses match in status and in body, so a well-meaning later change that adds a helpful "your
token expired" message fails a test instead of silently reopening the oracle.

This is explicitly **not** the fix to the token-hashing question:

> **`api_tokens.token_hash` keeps `cairn_core::digest` (a fast SHA-256), and this is
> correct.** A minted token is 32 bytes of CSPRNG entropy (`auth::random_token`,
> `auth.rs:82-86`) — 256 bits of keyspace. Argon2 exists to slow down an offline
> dictionary/brute-force attack against a *low-entropy* secret (a human password). Applying
> it to a value no attacker can guess by trying candidates adds computation on every request
> for zero security benefit; a fast collision-resistant digest is the correct primitive here,
> exactly as it is for `web_sessions.token_hash` (same primitive, same reasoning,
> `auth.rs:207-213`). **Do not "fix" this in a later pass** — the only field this feature
> adds to `api_tokens` is the optional `expires_at` above.

## 8. `server_instance_id`

**FR-415, FR-416**. New server table, single row:

```sql
CREATE TABLE server_instance (
    id         UUID PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Populated by migration `0003` with one generated UUID, at migration time, and never again.
There is no `UPDATE` statement anywhere that touches this table — immutability is structural
(a table with one row and no update path), the same discipline `reusable_patterns` uses for
"no `project_id` column" (`0005_project_intelligence.sql:236-238`) applied to "no mutation
path" instead of "no column."

### `GET /api/version` (extended)

Additive to the existing `VersionPayload` (`crates/cairn-server/src/version.rs:54-72`),
which already carries `schema_version` and `capabilities` unauthenticated:

```json
{
  "current": "0.3.0",
  "schema_version": 3,
  "capabilities": ["memory_relations", "task_criteria", "task_blockers",
                    "memory_subject_identity", "memory_verification",
                    "personal_knowledge", "team_knowledge"],
  "server_instance_id": "b7e1c2b4-..."
}
```

`server_instance_id` is exposed on the same unauthenticated, already-polled endpoint every
client fetches every drain cycle (`cairnd::sync::refresh_capability`, called once per drain
cycle, `crates/cairnd/src/sync.rs:791-856`) — no new probe, no new round trip. This is what
makes it "discoverable by every client connected to it" (FR-416) at zero additional cost: the
client was already going to ask.

## 9. `cairn user` CLI

New subcommand, local-machine, daemon-mediated exactly like `cairn auth`/`cairn link` (no
direct server calls from the CLI process; the daemon holds the stored token and makes the
HTTP request):

```text
cairn user create --email <e> --display-name <n>
    → creates the account server-side, prints:
      "Created member@example.com. Temporary password: <shown once>.
       They must change it before doing anything else."

cairn user list
    → table: email · role · status · must_change_password · created_at

cairn user disable <email>
    → "Disabled alice@example.com. 3 API token(s) revoked immediately."

cairn user enable <email>
    → "Enabled alice@example.com. Existing tokens remain revoked; mint a new one."

cairn user promote <email>
    → "alice@example.com is now admin."
    → on the last-admin guard tripping for a *demote*, not promote — promote never fails this way

cairn user demote <email>
    → "alice@example.com is now member." or refuses:
      "Refused: alice@example.com is the only administrator. Promote someone else first."

cairn user reset-password <email>
    → calls POST /api/admin/users/{id}/reset-password (§2a), prints:
      "Reset alice@example.com. Temporary password: <shown once>.
       Every existing token was revoked. They must change it before doing anything else."
    → on a disabled account:
      "Reset alice@example.com. Temporary password: <shown once>. The account remains
       disabled and cannot authenticate until an administrator re-enables it."
    → on the environment-named account:
      "Refused: alice@example.com is this server's environment-defined administrator
       (CAIRN_ADMIN_EMAIL). Change CAIRN_ADMIN_PASSWORD and restart the server instead."
```

Every subcommand requires the local `cairn auth` credential to itself belong to an admin —
the CLI performs no local authorization decision; it forwards to `/api/admin/users` and
reports whatever the server decides, including `403 forbidden` rendered as "Refused: your
account is not an administrator on this server."

`cairn auth` gains one addition for the temporary-password flow: `cairn auth token set` and
interactive login both surface `password_change_required` as a distinguishable outcome
("Your password must be changed before you can do anything else. Run `cairn auth
change-password`.") rather than a generic authentication failure, and a new `cairn auth
change-password` prompts for current and new password and calls `POST /api/auth/password`.

## 10. Error codes

| Code | HTTP | Meaning |
|---|---|---|
| `password_change_required` | 403 | The account's `must_change_password` is set; only `POST /api/auth/password` is reachable |
| `password_too_short` | 422 | New password is under 8 characters |
| `invalid_credentials` | 401 | Wrong current password, or authentication failed (also covers disabled accounts, deliberately indistinguishable) |
| `email_taken` | 409 | `POST /api/admin/users` with an email already in `users` |
| `last_admin` | 409 | The requested role/status change would leave zero active admins, whether refused outright or as the loser of a concurrent-demotion race (§3) |
| `forbidden` | 403 | Caller is not an admin, for an admin-only route |
| `env_admin_reset_refused` | 409 | `POST /api/admin/users/{id}/reset-password` targeted the environment-named account (§2a, §3a) |

## 11. Deleted endpoints

Per the prerequisite patch (D-U2) and restated here because this contract depends on their
absence:

- `POST /api/auth/register` — removed. No route creates a user row without an admin acting
  through `POST /api/admin/users`.
- `POST /api/projects/{id}/join` — removed. Covered in full by
  [`project-authorization.md`](./project-authorization.md); mentioned here because its
  removal is part of the same "no self-service identity/access grant" principle this
  contract enforces for accounts.

## Invariants

1. Every row in `users` traces to either the migration backfill (FR-414) or a `POST
   /api/admin/users` call made by an account that was itself `role = admin` at the time —
   no other write path to `users` exists (FR-401).
2. A newly created account has `must_change_password = true` and cannot mint an API token,
   cannot read or write any project, and cannot reach any route except
   `POST /api/auth/password` until that flag clears (FR-404, FR-407).
3. `must_change_password` clears only as a side effect of a successful `POST
   /api/auth/password`; no admin action clears it directly (FR-405).
4. Disabling an account revokes every one of its live API tokens in the same transaction
   as the status change; re-enabling never restores a revoked token (FR-409, FR-410, FR-590),
   asserted as two separate cases so a regression in either cannot be masked by the other
   (SC-404, SC-470).
5. A disabled account is refused authentication through every path — bearer token, web
   session login, and any bearer token minted before disabling — with no path that still
   succeeds (FR-410).
6. No write to `users.role` or `users.status` may result in zero rows satisfying
   `role = 'admin' AND status = 'active'`, enforced atomically inside the demoting or
   disabling statement itself, serialized against every other such operation by one
   application-wide advisory lock — never by a separate count evaluated before the write
   (FR-413, FR-574, D436, D445).
6b. Two concurrent operations that would each individually be legal, but that together would
   leave zero active admins, always resolve to exactly one success and one `409 last_admin`
   refusal, verified under real concurrency (FR-560, SC-444).
6a. The env-seeded `CAIRN_ADMIN_EMAIL` account is unconditionally `role = 'admin'`,
   `status = 'active'`, and never `must_change_password` on every process start,
   regardless of any prior API-driven change to that row; no API route can disable,
   demote, or lock it, and its password remains controlled exclusively by
   `CAIRN_ADMIN_PASSWORD` (D429).
7. `server_instance.id` is written exactly once, by the migration, and by nothing
   afterward (FR-415).
8. `server_instance_id` is readable, unauthenticated, by every client that already polls
   `GET /api/version` — no additional network round trip is introduced to learn it
   (FR-416).
9. `api_tokens.token_hash` remains a fast digest (`cairn_core::digest`), not a slow
   password hash; only `expires_at`, nullable and defaulting to no expiry, is added to the
   token model (FR-417).
10. `POST /api/auth/register` and `POST /api/projects/{id}/join` do not exist on the
    server; no test, client, or documentation refers to them as reachable.
11. An administrator can reset any other account's password; the response reveals the new
    temporary password exactly once and no later route, for any caller, retrieves it again
    (FR-553, FR-554).
12. A password reset invalidates the old password immediately, revokes every one of the
    account's tokens in the same transaction, and sets `must_change_password = true`
    (FR-555, FR-556, FR-557).
13. Resetting a disabled account's password never changes `status`; the account remains
    disabled and remains refused authentication by every path, including with the new
    temporary password (FR-558).
14. A password reset targeting the environment-named account is refused, naming
    `CAIRN_ADMIN_EMAIL`/`CAIRN_ADMIN_PASSWORD`, for the same reason disable and demote
    already refuse it (FR-559, D429).
15. A temporary credential — from creation or from reset — authenticates only to
    `POST /api/auth/password` while `must_change_password` is set, and is invalidated the
    instant a password change succeeds; the only way to obtain a new one is a further
    administrator reset (FR-407, FR-572, FR-573, D440).
