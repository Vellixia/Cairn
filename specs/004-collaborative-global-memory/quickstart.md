# Quickstart: Cairn Collaborative Global Memory

**Feature**: `004-collaborative-global-memory`

Two people, two machines, a shared server, and the two new domains that follow a *person*
rather than a project: personal knowledge that trails one user across everything they touch,
and team knowledge that any member can propose but only an administrator can ratify. Project
knowledge — the thing Cairn already did — is unchanged, and stays first in line for every
budget and every search.

**The vocabulary is normative; the identifiers, ports and timestamps are not.** Identifiers
are elided as `0192f4…`, and a reader checking this document against a real run should score
the outcome words, the field names and the shape — not a literal UUID where an ellipsis
stands.

Every command below either exists on `main` today or is introduced by this feature. Anything
new is marked **NEW in 004** the first time it appears. Nothing below is aspirational: if a
command is not marked new, it is exactly what `crates/cairn/src/main.rs` and
`crates/cairn/src/mcp.rs` already ship.

## Prerequisites

```bash
cargo build --workspace --release
export PATH="$PWD/target/release:$PATH"

# A Postgres instance for cairn-server. Any reachable Postgres will do.
export DATABASE_URL=postgres://cairn:cairn@localhost:5433/cairn
```

Two people appear in this walkthrough: **Alice**, the administrator, and **Bob**, a member.
Two of Alice's machines appear too: **A1** (laptop) and **A2** (workstation). Each runs its
own `cairnd` against its own local Cairn store, and each links to the same shared project on
one `cairn-server`.

---

## 1. The server starts, and the deterministic admin bootstrap

An operator starts the server with the account it should seed — exactly the environment
variables `cairn-server` already reads today (`CAIRN_ADMIN_EMAIL`, `CAIRN_ADMIN_PASSWORD`,
re-applied on every start, `crates/cairn-server/src/main.rs:76-84`). What is **new in 004**
is that this seeded account is now assigned a role, not left an ordinary row indistinguishable
from anyone else (FR-414):

```bash
CAIRN_ADMIN_EMAIL=alice@example.com \
CAIRN_ADMIN_PASSWORD=correct-horse-battery \
cairn-server
#   cairn-server listening on 127.0.0.1:8080
#   schema 3 applied
#   admin bootstrap: alice@example.com -> role=admin (matched CAIRN_ADMIN_EMAIL, FR-414)
```

`schema 3` is this feature's schema version — `crates/cairn-server/src/db.rs` carries
`SCHEMA_VERSION = 2` on `main` today; 004 adds the migration that moves it to 3 (FR-521,
FR-528). A server with no `CAIRN_ADMIN_EMAIL` at all instead promotes whichever account is
oldest by `created_at`, and this rule alone is enough to guarantee at least one admin exists
after any migration (FR-413, FR-414, SC-405) — demonstrated later, in Step 9.

**`POST /api/auth/register` is gone.** No code path in `cairn-server` can create an account
without an administrator's action, which is the strongest form FR-401 can take. But "gone"
still has to answer, because an operator whose old client suddenly cannot create accounts is
owed more than a not-found (FR-587):

```bash
curl -s -X POST http://127.0.0.1:8080/api/auth/register \
  -H 'content-type: application/json' \
  -d '{"email":"anyone@example.com","display_name":"Anyone","password":"whatever1"}' \
  -w '\n%{http_code}\n'
#   {"code":"route_removed","message":"self-registration is disabled; an administrator creates
#    accounts with `POST /api/admin/users` (`cairn user create`)"}
#   410
```

`410 Gone`, not `404` — a `404` is indistinguishable from a typo'd URL or a route that never
existed, while `410` means "this existed and was deliberately retired", which is the fact an
operator debugging a suddenly-failing client actually needs. The body names the replacement
route and the CLI verb, so the diagnosis needs neither source nor release notes (SC-458).
`POST /api/projects/{id}/join` answers the same way, naming `cairn project member add`.

No account is created (SC-401).

---

## 2. Administered accounts, temporary passwords, and gated token minting

Alice authenticates as the account the server just bootstrapped, and asks it to create Bob's
account. **`cairn user create` is NEW in 004** — the CLI verb behind
`POST /api/admin/users` (design brief §4, FR-401, FR-402):

```bash
curl -s -c alice.cookies -X POST http://127.0.0.1:8080/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"alice@example.com","password":"correct-horse-battery"}'
#   {"id":"0192a1…"}

# Alice mints herself a personal API token (existing route, POST /api/tokens)
curl -s -b alice.cookies -X POST http://127.0.0.1:8080/api/tokens \
  -H 'content-type: application/json' -d '{"name":"alice-laptop"}'
#   {"id":"0192a2…","name":"alice-laptop","token":"catk_9fQ2…"}

cairn auth token set catk_9fQ2… --server http://127.0.0.1:8080

cairn user create --email bob@example.com --role member       # NEW in 004
#   User created: bob@example.com (member)
#   Temporary password: xK4-mQ2p-91Zt      ← shown exactly once
#   must_change_password: true
```

Requesting it again returns nothing — there is no route that reads a temporary password back
(FR-403, SC-402). Bob logs in with it and immediately hits the wall FR-407 builds:

```bash
curl -s -c bob.cookies -X POST http://127.0.0.1:8080/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"bob@example.com","password":"xK4-mQ2p-91Zt"}'
#   {"id":"0192b1…"}

curl -s -b bob.cookies -X POST http://127.0.0.1:8080/api/tokens \
  -H 'content-type: application/json' -d '{"name":"bob-laptop"}'
#   HTTP 403
#   {"error":{"code":"password_change_required","message":"change your password before doing anything else"}}
```

Every authenticated route refuses the same way except one (FR-407). **`cairn auth password`
is NEW in 004** — it reads the current and new password from stdin, the same discipline
`cairn auth token set` already applies to tokens so neither lands in shell history:

```bash
printf 'xK4-mQ2p-91Zt\nHarborLight42!\n' | cairn auth password bob@example.com --server http://127.0.0.1:8080
#   Password changed. must_change_password cleared (FR-405).
```

Now the same request that was refused a moment ago succeeds:

```bash
curl -s -c bob.cookies -X POST http://127.0.0.1:8080/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"bob@example.com","password":"HarborLight42!"}'
curl -s -b bob.cookies -X POST http://127.0.0.1:8080/api/tokens \
  -H 'content-type: application/json' -d '{"name":"bob-laptop"}'
#   {"id":"0192b2…","name":"bob-laptop","token":"catk_71Lm…"}

cairn auth token set catk_71Lm… --server http://127.0.0.1:8080
```

A temporary password that is never changed leaves the account permanently confined to
`POST /api/auth/password` — no token, ever, until an admin resets it (edge case, SC-403).

Later, Bob loses his password. **`cairn user reset-password` is NEW in 004** — only an
administrator can invoke it, against any account but their own environment-seeded one
(FR-553):

```bash
cairn user reset-password bob@example.com     # run by Alice
#   Password reset: bob@example.com
#   Temporary password: qR8-hT5v-33Km      ← shown exactly once, on this response only
#   must_change_password: true
```

Asking again — even Alice, who just ran the reset — returns nothing; there is no route that
reveals a temporary password a second time (FR-554). Bob's old password and his
still-unexpired token die immediately, not merely on next use (FR-555, FR-556, SC-442):

```bash
curl -s -c bob.cookies -X POST http://127.0.0.1:8080/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"bob@example.com","password":"HarborLight42!"}'
#   401

curl -s -b bob.cookies -X POST http://127.0.0.1:8080/api/tokens \
  -H 'content-type: application/json' -d '{"name":"bob-laptop-2"}'
#   401   ← catk_71Lm… revoked by the reset, not merely stale
```

The new temporary password behaves exactly like one from account creation: it authenticates
only to the password-change route, and nothing else, until Bob changes it (FR-557, FR-407,
FR-572, SC-442):

```bash
curl -s -c bob.cookies -X POST http://127.0.0.1:8080/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"bob@example.com","password":"qR8-hT5v-33Km"}'
curl -s -b bob.cookies -X POST http://127.0.0.1:8080/api/tokens \
  -H 'content-type: application/json' -d '{"name":"bob-laptop-2"}'
#   HTTP 403
#   {"error":{"code":"password_change_required","message":"change your password before doing anything else"}}

printf 'qR8-hT5v-33Km\nNewHarbor99!\n' | cairn auth password bob@example.com --server http://127.0.0.1:8080
#   Password changed. must_change_password cleared.
```

A reset is not a re-enable, and it must never become one. Alice creates a third account,
`carol@example.com`, and disables it before its temporary password is ever used — routine
offboarding:

```bash
cairn user create --email carol@example.com --role member
#   User created: carol@example.com (member)
#   Temporary password: bT3-pQ0k-77Rz      ← shown exactly once

cairn user disable carol@example.com          # NEW in 004
#   Disabled: carol@example.com
```

Alice resets the disabled account's password anyway. The credential changes; the account
does not re-enable, and authentication with the brand-new temporary password is refused all
the same (FR-558, SC-443):

```bash
cairn user reset-password carol@example.com
#   Password reset: carol@example.com
#   Temporary password: mN2-xL9r-04Qp      ← shown exactly once
#   must_change_password: true

curl -s -X POST http://127.0.0.1:8080/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"carol@example.com","password":"mN2-xL9r-04Qp"}'
#   401
#   {"error":{"code":"account_disabled","message":"account is disabled"}}
```

One account is permanently out of reach of this route. Resetting the account matching the
server's own environment configuration is refused by name, because that account's credential
is re-established from `CAIRN_ADMIN_PASSWORD` on every start and a reset would be silently
undone at the next restart (FR-559):

```bash
cairn user reset-password alice@example.com    # alice@example.com matches CAIRN_ADMIN_EMAIL
#   refused: password reset is not available for the account matching CAIRN_ADMIN_EMAIL
```

---

A token may also carry an expiry. Past it, the token is refused — and the refusal is byte-for-byte
the refusal a revoked token gets, so a stale token cannot be used to learn that it was ever
valid for this server (FR-585, SC-452):

```bash
curl -s -o /dev/null -w '%{http_code}\n' -H "Authorization: Bearer $EXPIRED" $CAIRN_SERVER/api/projects
#   401
curl -s -H "Authorization: Bearer $EXPIRED" $CAIRN_SERVER/api/projects
#   {"error":"invalid_token"}
curl -s -H "Authorization: Bearer $REVOKED" $CAIRN_SERVER/api/projects
#   {"error":"invalid_token"}        ← identical status, identical body
```

No legitimate caller needs to know which of the two happened, because the remedy is the same:
get a new token from an administrator.

## 3. Explicit membership, safe discovery, and a safe auto-link

Bob has a repository, but Alice has not added him to a shared project yet. He initializes and
links, on purpose specifying nothing:

```bash
cairn init
cairn link
#   Not linked.
#   No shared project matches your memberships.
```

This is FR-422 in effect: `GET /api/projects/lookup` now carries the membership filter the
prerequisite patch added, so a project Bob has never been added to is invisible to him even
if he can name its shared identity — discovery cannot grant anything (FR-426, SC-406).

Alice, from her own already-linked checkout of the same repository, adds him. **`cairn project
member add` is NEW in 004** (`POST /api/projects/{id}/members`, FR-419):

```bash
cairn project member add bob@example.com
#   Added bob@example.com to project 0192c0… (added by alice@example.com at 2026-08-21T10:02:11Z)

cairn project member list                                     # NEW in 004
#   alice@example.com   admin    added 2026-08-14T09:00:00Z (bootstrap)
#   bob@example.com      member   added 2026-08-21T10:02:11Z (by alice@example.com)
```

Bob runs `cairn link` again, still naming nothing:

```bash
cairn link
#   Linked to shared project 0192c0… (auto-selected: the only project you are a member of)
```

Before, lookup returned zero candidates and nothing was assumed. After membership exists,
exactly one candidate matches Bob's own memberships, and auto-link takes it without a prompt
(FR-424, SC-407). Had Bob belonged to two projects touching this remote, or none, `cairn link`
would ask him to specify one rather than guess (FR-425) — an explicit `cairn link --project
<uuid>` always still works and is never overridden by auto-link (FR-428).

If Alice later removes Bob, the next request he makes for that project — sync or read — is
refused, not silently starved (FR-421, SC-408):

```bash
cairn project member remove bob@example.com   # run by Alice, hypothetically
cairn sync now                                 # run by Bob, afterward
#   sync: project 0192c0… refused — no longer a member
```

(The rest of this walkthrough assumes Bob keeps his membership.)

---

## 4. Personal knowledge follows Alice across her projects

Alice is inside *this* repository when she records something about herself, not about the
project. **`cairn memory add` gains a `--domain` flag in 004** (mirroring `cairn_remember`'s
new `domain` field, default `project`, FR-431):

```bash
cairn memory add --domain personal --type convention \
  --topic-key editor.tab_width --value-key two_spaces \
  "I indent with two spaces, not four"
#   Recorded as personal knowledge (belongs to alice@example.com only).
```

She opens a second, unrelated project — no shared identity with the first, nothing linking
them — and asks what Cairn knows:

```bash
cd ~/src/completely-unrelated-project
cairn context
#   ...
#   ## Personal notes
#   - I indent with two spaces, not four
```

It arrived without naming the project it came from, exactly as FR-517 requires: personal
knowledge has no project-identity column to leak in the first place. Bob, working in either
project, never sees it — search and context for his account simply have no row to return
(FR-432, SC-409). **`cairn memory search` gains a `--domains` flag in 004** (mirroring
`cairn_search`'s new `domains` field, default all three, FR-472):

```bash
cairn memory search --domains personal --json | jq '.personal'   # run as Bob
#   []
```

Now Alice restricts one to a language. **`--applies-to` is NEW in 004** — the closed-vocabulary
applicability facts (`kind=value`, `kind` ∈ `language | tool`, FR-569), deliberately a
different flag from patterns' free-text `--applies-when`. `topic` is deliberately not a
member: it cannot be derived from files present in a working tree the way a language or a
tool can, so it is excluded from the vocabulary entirely rather than left in as a kind that
can never match (FR-569, D439). This is unrelated to `--topic-key` above, which names the
record's own subject key from Feature 003, not a project trait (FR-570):

```bash
cairn memory add --domain personal --type convention \
  --topic-key rust.error_handling --value-key thiserror \
  --applies-to language=rust \
  "Prefer thiserror over hand-rolled Display impls"
```

**`cairn traits` is NEW in 004** — it shows what this project's working tree derived, purely
from manifest and lockfile presence, never guessed (FR-437, FR-439):

```bash
cd ~/src/your-git-repo       # a Cargo project
cairn traits
#   language  rust
#   tool      cargo

cd ~/src/completely-unrelated-project   # a Node project
cairn traits
#   language  node
#   tool      pnpm
```

The `rust`-restricted note applies in the first project and not the second (FR-436, SC-410):

```bash
cairn context --json | jq '.briefing.memory.personal_notes | length'   # in the Cargo project
#   2
cairn context --json | jq '.briefing.memory.personal_notes | length'   # in the Node project
#   1
```

A value outside the closed vocabulary is refused outright, never silently dropped (FR-446):

```bash
cairn memory add --domain personal --applies-to language=rust,proprietary-widget-9000 \
  "won't compile"
#   refused: applicability value outside the closed vocabulary (language, tool only)
```

Forgetting a personal note removes it from every surface immediately (FR-441):

```bash
cairn memory forget 0192p1…      # the tab-width note
cairn context | grep -c 'two spaces'
#   0
```

---

## 5. Personal knowledge follows Alice to her second device

Alice's workstation (A2) is linked to the same account and, once synced, sees what her laptop
(A1) wrote (FR-444, SC-409 across devices):

```bash
# on A2
cairn sync now
cairn memory search --domains personal --json | jq -r '.personal[].content'
#   Prefer thiserror over hand-rolled Display impls
```

Now both machines go offline and each records a disagreeing note on the same subject —
neither has ever synced the other's write, and their clocks are not even set to agree:

```bash
# A1, offline                                    # A2, offline
cairn memory add --domain personal \              cairn memory add --domain personal \
  --topic-key editor.line_length \                  --topic-key editor.line_length \
  --value-key ninety_five \                          --value-key eighty \
  "I wrap at 95 columns"                            "I wrap at 80 columns"
```

Both reconnect. Neither timestamp nor arrival order decides anything (FR-493):

```bash
cairn sync now                       # on each device, either order
cairn memory subject editor.line_length --domain personal
#   **Reconciliation**: conflicted
#
#   ## Answers
#   - ninety_five — "I wrap at 95 columns" `0192d1…`
#   - eighty       — "I wrap at 80 columns" `0192d2…`
```

Note what the subject view does *not* print. A project memory's answer line carries its
verification state; a personal or team answer line has none to carry, because the record has no
verification field of any kind (FR-513, FR-517, D452). The reconciliation is identical; only the
column that could have laundered one project's deterministic check into a project-independent
claim is missing.

Both entries survive on both devices, marked disagreeing rather than one silently winning
(SC-411). This is the same deterministic `classify_proposal` / `derive_subject` machinery 003
already used for project memory (D406) — nothing new was invented to make personal knowledge
converge; it reuses the mechanism verbatim.

The other multi-writer hazard: two devices producing **byte-identical** content must not
collide as a duplicate write of each other (FR-491, SC-427). **`WriterIdentity`**, a single
opaque id per local store established once (FR-490), is folded into what makes a synchronized
write unique:

```bash
# A1 and A2, independently, same content
cairn memory add --domain personal --topic-key editor.eol --value-key lf "Always LF, never CRLF"
cairn sync now   # on each
cairn memory search --domains personal --json | jq '[.personal[] | select(.topic_key=="editor.eol")] | length'
#   2
```

Two rows, not one silently dropped as a duplicate of the other's write — the writer id, not
the content digest alone, is what the sync layer treats as identity here.

Last, the background-pull fix this feature makes (FR-489): A2 writes nothing at all for a
while, and A1's synced note still arrives without A2 pushing first (SC-412):

```bash
# A2 does nothing for a few minutes
cairn sync status
#   personal    linked · pending 0 · last success 2026-08-21T10:41:07Z
cairn memory search --domains personal --json | jq '.personal | length'
#   4    ← A1's later write arrived on a background pull, not a push A2 triggered
```

---

## 6. Team guidance: proposed by Bob, decided by Alice

Bob notices something true across every project on this server and proposes it. **`cairn team
propose` is NEW in 004** — the CLI-facing wrapper around `cairn_remember`'s `promote`
action with `target: "team"`, which is the only way a team entry is ever created
(FR-451, FR-455):

```bash
cairn memory add --topic-key testing.flaky_retries --value-key three \
  "Retry flaky integration tests up to 3 times before failing the build"
#   memory 0192m1… recorded (scope: project)

cairn team propose --memory 0192m1…
#   Proposed. state=proposed, id=0192t1…
#   Invisible to recall — including yours — until an admin ratifies it.
```

It is invisible everywhere, to everyone, including Bob (FR-452, SC-413):

```bash
cairn memory search --domains team --json | jq '.team'   # run as Bob
#   []
cairn context | grep -c 'flaky'
#   0
```

Bob cannot ratify his own proposal even by asking politely — ratification is CLI/server-only
and admin-only, never reachable from the agent tool surface at all (FR-453, FR-455):

```bash
cairn team ratify 0192t1… --expected-state proposed    # NEW in 004, run as Bob
#   refused: forbidden — ratification requires the admin role
```

Alice ratifies it. **`--expected-state` is NEW in 004** and is not decoration: it is the
compare-and-swap guard D409 specifies, reusing the same `expected_revision` / blind-write
pattern `crates/cairn-store/src/criteria.rs` already enforces for task criteria:

```bash
cairn team ratify 0192t1… --expected-state proposed     # run as Alice
#   Ratified. state=authoritative, ratified_by=alice@example.com at 2026-08-21T10:55:02Z
```

Now it is visible to everyone on the server, project member or not, labelled with who proposed
and who ratified it (FR-458, FR-463, SC-416):

```bash
cairn memory search --domains team --json | jq '.team[0]'   # run as anyone, any project
#   { "content": "Retry flaky integration tests up to 3 times before failing the build",
#     "proposed_by": "bob@example.com", "ratified_by": "alice@example.com",
#     "state": "authoritative", ... }
```

Two admins racing to ratify the same proposal never both succeed (FR-454, SC-415):

```bash
# admin1 and admin2, concurrently, both believing state=proposed
cairn team ratify 0192t1… --expected-state proposed
#   admin1: Ratified.
#   admin2: refused: expected state 'proposed' but the entry is already 'authoritative'
```

Alice retires it later; retiring is not reversible by re-ratifying — restoring the guidance
means a fresh proposal (FR-465):

```bash
cairn team retire 0192t1…       # NEW in 004
#   Retired. state=retired.
cairn memory search --domains team --json | jq '.team'
#   []
```

---

## 7. A promotion rejected by the privacy gate, then corrected

Alice has a project memory she wants to hoist to personal knowledge, but its content still
carries an absolute path from the machine it was written on. The gate (FR-506, FR-507, D416)
runs its fixed checks in order and stops at the first failure, naming it — never echoing the
text that tripped it:

```bash
cairn memory add --topic-key build.cache_dir --value-key tmp \
  "Build cache lives at /Users/alice/.cache/cairn-build, clear it if stale"
#   memory 0192m2… recorded
```

Promotion runs through `cairn_remember`'s `promote` action (`target: "personal"`), the only
surface capable of it — shown here over the MCP transport `cairn mcp` actually speaks
(JSON-RPC 2.0 over stdio, `crates/cairn/src/mcp.rs`):

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
  "name":"cairn_remember",
  "arguments":{"cwd":".","action":"promote","memory_id":"0192m2…","target":"personal"}
}}' | cairn mcp
#   { ... "result": { "promoted": false,
#       "rejection": { "check": "absolute_path", "class": "absolute_path" } } ... }
```

No partial record is left behind (SC-421). Alice fixes the content — no path, nothing that
names the project — and promotes again:

```bash
cairn memory add --topic-key build.cache_dir --value-key clears_on_stale \
  "Clear the build cache when a stale artifact is suspected"
#   memory 0192m3… recorded

echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
  "name":"cairn_remember",
  "arguments":{"cwd":".","action":"promote","memory_id":"0192m3…","target":"personal",
               "applicability":[{"kind":"tool","value":"cargo"}]}
}}' | cairn mcp
#   { ... "result": { "promoted": true, "personal_id": "0192p9…" } ... }
```

The verification a deterministic check earned against *this* project does not travel, and it
does not travel as a *value* either: a personal or team record has **no verification field of
any kind** — not an authority, not a state, not a timestamp (FR-513, FR-517). There is nothing
in the response above to reset, because there is nowhere on the record to hold it. The gate's
`verification_reset` check keeps its slot in the fixed order and refuses nothing; it exists to
make the absence explicit at the moment promotion happens. SC-422 asserts this field by field
against the **stored and serialized** forms in **both** stores, so a schema change that added
a verification column would fail the test rather than quietly re-open the transfer. Later, forgetting the source memory changes nothing about the promoted one; it never
held a live reference back, only a salted digest of the source project's identity (FR-516,
FR-519, SC-423):

```bash
cairn memory forget 0192m3…
cairn memory search --domains personal --json | jq '.personal[] | select(.id=="0192p9…") | .content'
#   "Clear the build cache when a stale artifact is suspected"    ← unaffected
```

A seeded adversarial corpus covering all **nine** rejection classes — absolute paths, `~/`
references, Windows drive letters, `file://` references, credentialed URLs, environment
assignments, secret-shaped runs, **tokens naming the project**, and **shell command
invocations** — is refused in every case, by name, with nothing partial left over (SC-421,
SC-424a). It is driven through every entry point, and it is driven through the `applicability`
array as well as `content`: an applicability *value* is screened by the same nine classes as
free text, so `{"kind":"tool","value":"acme-internal-deploy"}` is refused exactly as the same
string in a sentence would be (FR-578, SC-448). The `language | tool` vocabulary constrains a
fact's *kind*; nothing but the validator constrains its value. This walkthrough shows one
member of that corpus; the full corpus is exercised by test, not by hand.

That same corpus, and that same validator, does not only guard promotion. Before this
repair, direct personal creation never routed through the gate at all — an entry point the
design analysis found open. `validate_global_content` now runs on direct personal creation
too (FR-544, FR-545), so the identical content is refused the identical way whether it
arrives by promotion, as above, or straight through `cairn_remember`'s `create` action:

```bash
echo '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
  "name":"cairn_remember",
  "arguments":{"cwd":".","action":"create","domain":"personal",
               "topic_key":"build.tmp_dir","value_key":"alice_machine",
               "content":"Scratch files live at /Users/alice/tmp, safe to delete"}
}}' | cairn mcp
#   { ... "result": { "created": false,
#       "rejection": { "check": "absolute_path", "class": "absolute_path" } } ... }
```

The `class` is the same `absolute_path` Step 7 saw at promotion, because both paths call the
one shared validator (FR-546, SC-438). Nothing is recorded — no personal knowledge row, no
partial row, no outbox entry (FR-548, SC-440) — and the rejection names only the class,
never the path that tripped it, in the response, in any log line, and in this walkthrough
(FR-547, SC-439). The bypass the analysis found — a promotion-only check that the
non-promotion paths could simply skip — is closed at all **five** entry points, not patched at
one of them (FR-545).

Four of those five are in this client. The fifth is not, and that is the point. A client that
never runs the validator — modified, out of date, or simply buggy — used to write straight into
the server store, from which the content reached every other device Alice owns. Server-side
synchronization ingest is now itself an entry point (FR-545, FR-577). Simulating a client that
skips its own check:

```bash
CAIRN_TEST_SKIP_LOCAL_VALIDATION=1 cairn memory promote 0192m4… --target personal
cairn sync --once --json | jq '.namespaces.personal'
#   { "pushed": 0, "refused": 1, "state": "active",
#     "refusals": [ { "entity_id": "0192p…", "class": "project_identifying",
#                     "permanent": true } ] }
```

The server screens against the union of Alice's project memberships — it cannot know which
project her client was in, but it knows every project she could have been in, which is
strictly stronger, and which catches the one case her client structurally cannot: content
naming project X written while she was working in project Y. The record is absent from the
server store and never reaches her second device (SC-449).

Note `"state": "active"` and `"permanent": true`. This is **not** the capability refusal of
Step 9. A capability refusal parks the item, marks the namespace `blocked`, and re-probes,
because a server upgrade makes the same bytes acceptable. An ingest refusal can never succeed
on retry of the same bytes, so it must not park the item, must not enter `blocked`, and must
not apply the namespace backoff — otherwise one bad record throttles the lane forever
(FR-581). The two are distinguishable by the client precisely because their remedies are
opposite: wait for the server, versus fix the content.

---

## 8. Bounded recall: project truth always first

First, a context request whose project-priority sections already consume the whole budget.
Personal and team knowledge exist — Alice and Bob both have plenty by now — but the assembled
briefing is byte-identical to what it would be without them (FR-473, FR-475, SC-418):

```bash
cairn context --budget 300
#   ## Task: Retry backoff (todo)
#   ...
#   ---
#   298 of 300 estimated tokens; omitted: project_memory, personal_notes, team_guidance
```

`estimated_tokens <= budget` holds, as it always has (FR-480) — here it holds with zero global
contribution, because project sections took the reserve and all the ranked headroom (D420,
D421: the global fetch is not even called during reserve computation, so no arithmetic can
admit it into Level 0).

Now the same project, more headroom:

```bash
cairn context --budget 1200
#   ...
#   ## Personal notes
#   - Prefer thiserror over hand-rolled Display impls
#
#   ## Team guidance
#   - Retry flaky integration tests up to 3 times before failing the build
#
#   ---
#   1173 of 1200 estimated tokens; omitted: (none)
```

Personal appears ahead of team whenever both compete for the same leftover space (FR-476,
D422's specificity gradient), and neither exceeds its documented cap. The cap is a floor of two
quantities, not one rule (FR-474, FR-584, D421, D449, D450, SC-419, SC-451):

```text
global_allowance = min( floor(total_budget * 0.15), remaining_non_reserve )
```

The fraction is `0.15` and it is taken against the **total** budget, as FR-474 states. The
second term is the part worth reading twice: the pool global sections may draw from is the
**non-reserve** pool only. Feature 003 returns an unspent Level 0 reserve to the general pool,
and that released space stays unavailable to personal and team knowledge (FR-584). A project
with little critical state releases most of its reserve; if global sections could spend it,
that project would hand a large share of its briefing to project-independent guidance, which
is Principle VIII's failure mode wearing a budget's clothes rather than a scope's. Released
reserve goes back to *project* ranked content, which is what it was reserved for.

```bash
cairn context --budget 1200 --explain
#   budget 1200 · reserve 480 · reserve used 120 · released 360
#   non-reserve pool 720 · project level1 spent 640 · remaining_non_reserve 80
#   global_allowance = min(floor(1200 * 0.15), 80) = min(180, 80) = 80
#   INCLUDED
#     level1  personal_notes  applicability_match      41
#   EXCLUDED
#     level1  team_guidance   budget_exhausted         52
#   personal+team = 41 tokens <= 80    ← the 360 released tokens are NOT available here
```

The two terms bind in different situations, which is why stating only one leaves the boundary
undefined: the `0.15` fraction binds when the non-reserve pool is roomy, and the pool binds
when project content has nearly filled it — as above, where 360 tokens sit unspent in the
budget and global still gets 80. The invariant `estimated_tokens <= budget` holds under both
terms (SC-431).

The `applicability_match` / `budget_exhausted` reasons above appear **only** under `--explain`.
They are produced on the diagnostic path and returned in the structured explain output; the
rendered briefing has no field to put them in, so a renderer cannot leak them by forgetting to
omit them (FR-478, D451).

A request at minimum depth excludes both sections entirely, with nothing able to override that
(FR-477, SC-420):

```bash
cairn context --budget 1200 --reason session_start --json | jq '.briefing.memory.personal_notes, .briefing.memory.team_guidance'
#   null
#   null
```

And a domain-spanning search keeps the three arrays separate; the project count is identical
with or without the others (FR-469, FR-470, SC-417):

```bash
cairn memory search testing --domains project,personal,team --json | jq '{total, personal: (.personal|length), team: (.team|length)}'
#   { "total": 4, "personal": 0, "team": 1 }
```

---

## 9. An old server: project sync keeps flowing, global sits blocked

`cairn-server` has always been able to hold a deployment back from its newest schema, via a
flag that exists on `main` today (`--max-schema-version` / `CAIRN_MAX_SCHEMA_VERSION`,
`crates/cairn-server/src/main.rs`). Starting one at schema 2 — the version `main` ships before
this feature — is a real, honest way to reproduce "an old server" without running old code:

```bash
CAIRN_MAX_SCHEMA_VERSION=2 CAIRN_ADMIN_EMAIL=alice@example.com \
  CAIRN_ADMIN_PASSWORD=correct-horse-battery cairn-server --addr 127.0.0.1:8081
```

Point a store at it and sync. Project synchronization keeps running at full speed; only the
two new namespaces sit blocked, and each says so by name (FR-522, FR-498, SC-425):

```bash
cairn auth token set catk_9fQ2… --server http://127.0.0.1:8081
cairn link --project 0192c0…
cairn sync now
cairn sync status
#   project     linked · pending 0 · last success 2026-08-21T11:10:04Z
#   personal    blocked (server lacks capability: personal_knowledge) — 2 item(s) retained
#   team        blocked (server lacks capability: team_knowledge) — 1 item(s) retained
```

This is namespace-independent backoff (D427, FR-497): a failing personal or team namespace
never slows project's retries, because each namespace now tracks its own backoff and its own
pull cursor (FR-486, FR-487, FR-488) rather than the single process-global state `main` has
today. Nothing retained here is retried against this server — it sits recoverable, not failed
(FR-499).

`GET /api/version` shows why the client knew what to hold back at all — the same one-way
capability advertisement 003 already relied on, now naming two more capabilities (D428,
FR-529):

```bash
curl -s http://127.0.0.1:8081/api/version | jq '{schema_version, capabilities}'
#   { "schema_version": 2, "capabilities": ["memory_relations","memory_subject_identity","memory_verification"] }
curl -s http://127.0.0.1:8080/api/version | jq '{schema_version, capabilities}'
#   { "schema_version": 3, "capabilities": ["memory_relations","memory_subject_identity","memory_verification","personal_knowledge","team_knowledge"] }
```

While a namespace sits blocked, `cairnd` does not simply wait for someone to run `cairn sync
now` again — it re-probes the server's advertised capabilities itself, on a bounded,
backed-off schedule. The probe is a capability read, nothing more; it is not a retry of the
two held namespaces, which still never touch this server until the capability is actually
there (FR-561).

Now the peer at `127.0.0.1:8081` is **replaced**, not merely restarted: the schema-2 process
is killed outright, and a fresh one — built with schema 3 already applied — is started bound
to the same address a moment later. From the client's side this looks exactly like an
in-place upgrade of the same deployment:

```bash
# kill the schema-2 cairn-server bound to 127.0.0.1:8081, then:
CAIRN_ADMIN_EMAIL=alice@example.com \
  CAIRN_ADMIN_PASSWORD=correct-horse-battery cairn-server --addr 127.0.0.1:8081
#   cairn-server listening on 127.0.0.1:8081
#   schema 3 applied
```

Nothing runs on the client from here — no `cairn sync now`, no `cairn` command of any kind,
no restart of `cairnd`. The next scheduled probe simply observes the new capabilities on its
own, both namespaces return to eligible, and the entries held since the first `cairn sync
now` above release for delivery preserving their original idempotency keys, so nothing
already partially sent is ever applied twice (FR-562, FR-563, SC-426, SC-445):

```bash
cairn sync status
#   project     linked · pending 0 · last success 2026-08-21T11:16:03Z
#   personal    linked · pending 0 · last success 2026-08-21T11:19:18Z
#   team        linked · pending 0 · last success 2026-08-21T11:19:18Z
```

Compare this against the `cairn sync status` earlier in this section: the `project`
namespace never stopped advancing while `personal` and `team` sat blocked, and both later
caught up without a single command run on this machine (FR-563).

---

## 10. Two server instances: team knowledge is bound, personal knowledge is partitioned

Alice points her already-linked store at a second, independently bootstrapped deployment —
its own Postgres, its own instance identity, nothing shared with `127.0.0.1:8080` (FR-415):

```bash
CAIRN_ADMIN_EMAIL=alice@example.com CAIRN_ADMIN_PASSWORD=correct-horse-battery \
  DATABASE_URL=postgres://cairn:cairn@localhost:5434/cairn2 \
  cairn-server --addr 127.0.0.1:8082
#   cairn-server listening on 127.0.0.1:8082
#   schema 3 applied
#   admin bootstrap: alice@example.com -> role=admin

curl -s http://127.0.0.1:8082/api/version | jq -r '.server_instance_id'
#   0192z9…    ← a different instance than 127.0.0.1:8080's

curl -s -c alice2.cookies -X POST http://127.0.0.1:8082/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"alice@example.com","password":"correct-horse-battery"}'
curl -s -b alice2.cookies -X POST http://127.0.0.1:8082/api/tokens \
  -H 'content-type: application/json' -d '{"name":"alice-second-instance"}'
#   {"id":"0192z1…","name":"alice-second-instance","token":"catk_5hR8…"}

cairn auth token set catk_5hR8… --server http://127.0.0.1:8082
cairn link
#   Linked to shared project 0192c9… (auto-selected: the only project you are a member of)
```

The team entry Bob and Alice ratified back in Step 6 lives in this same local store, tagged
with the instance it came from (`127.0.0.1:8080`). Syncing team knowledge against this
unrelated second instance is refused, and the refusal is reported by name — not merged and
not silently dropped (FR-496, SC-428):

```bash
cairn sync now
cairn sync status
#   project     linked · pending 0 · last success 2026-08-21T12:03:11Z
#   personal    linked · pending 0 · last success 2026-08-21T12:03:12Z
#   team        refused — this store's team namespace already recorded server instance
#               0192a0… (from 127.0.0.1:8080); 127.0.0.1:8082 reports 0192z9…, and the
#               mismatch is refused rather than merged
```

Personal knowledge behaves entirely differently, on purpose (FR-567): it never carried a
server-instance column to mismatch on, so it is not refused at all. It is partitioned. The
`alice@example.com` account on `:8082` is a distinct identity from the one on `:8080`, so its
personal knowledge starts empty and grows independently:

```bash
cairn memory search --domains personal --json | jq '.personal | length'
#   0    ← a brand-new identity; nothing written under it yet

cairn memory add --domain personal --topic-key editor.tab_width --value-key two_spaces \
  "I indent with two spaces, not four"
cairn memory search --domains personal --json | jq '.personal | length'
#   1

cairn auth token set catk_9fQ2… --server http://127.0.0.1:8080    # switch back
cairn memory search --domains personal --json | jq '.personal | length'
#   3    ← the original identity's own entries, unaffected by the second instance's writes
```

One local store, two identities' personal knowledge, keyed by server instance and account
together so they never merge (FR-568) — and recall only ever surfaces the identity currently
linked, never the other one at the same time.

---

## What this proves

| Step | Demonstrates | Success Criteria |
|---|---|---|
| 1 | Self-registration route is gone; role bootstrap is deterministic | SC-401, SC-405 |
| 2 | Temporary password returned once; no token while password change is pending; change clears it; an administrator reset re-issues a one-time password, kills the old one and every token, and never re-enables a disabled account; an expired token is refused indistinguishably from a revoked one | SC-402, SC-403, SC-442, SC-443, SC-452 |
| 3 | Discovery hides non-membership; no route lets a caller add themselves; membership grants access; auto-link is safe | SC-406, SC-407, SC-408, SC-465 |
| 4 | Personal knowledge crosses projects, never crosses users, matches by applicability; derived traits never leave the machine | SC-409, SC-410, SC-469 |
| 5 | Cross-device conflict survives without a clock deciding; distinct-writer identity prevents false collision; background pull delivers | SC-411, SC-412, SC-427 |
| 6 | Proposal is invisible; no tool action can create an authoritative entry; only an admin ratifies; concurrent ratification yields one winner; two disagreeing authoritative entries both stay visible; retirement is final | SC-413, SC-414, SC-415, SC-416, SC-460, SC-466 |
| 7 | The promotion gate refuses by name and creates no partial record; a promoted record has no verification field at all; the identical validator refuses the identical content at direct personal creation and at server-side ingest, screens applicability values as well as free text, and echoes nothing back; a client that skips its own validation is refused permanently by the server without blocking the namespace | SC-421, SC-422, SC-423, SC-424, SC-424a, SC-438, SC-439, SC-440, SC-448, SC-449 |
| 8 | Reserved context stays project-only; global is capped by the floor of `0.15 * total` and the non-reserve pool, and cannot spend released reserve; ordering is personal before team; minimum depth excludes both; exclusion reasons appear only under `--explain`; an importance hint changes nothing; search arrays stay separate and are never cross-ranked | SC-417, SC-418, SC-419, SC-420, SC-451, SC-462, SC-463, SC-464, SC-468 |
| 9 | An old server keeps project sync at full speed while personal and team sit blocked by name; the client re-probes on its own, and once the peer is replaced at the same endpoint the held content delivers exactly once with no local command and no restart | SC-425, SC-426, SC-445 |
| 10 | Team knowledge from a second server instance is refused and reported, never merged; personal knowledge is never refused on that basis — it partitions by identity, and recall shows only the identity currently linked | SC-428 |

Every domain this feature adds is additive: a caller who never touches personal or team
knowledge sees zero difference in project search or context from one who does (FR-481) — which
is also why nothing in Steps 1–3 of Feature 003's own quickstart needs to change to remain
true after this one ships.
