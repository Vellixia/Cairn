# Cairn v0.6.0 — The Cleanup Plan

> **Trim fat. Ship less surface. Get faster.**
>
> v0.6.0 = a focused cleanup sprint over the v0.5.0 release:
> delete dead code, drop unused agent integrations, kill the host-side
> server binary, promote env-only admin bootstrap, rename
> `cairn-cli` → `cairn`. No new features; no new endpoints; no new
> dependencies. Every commit must be independently buildable and
> pass `cargo test --workspace` (343 + 5 ignored, invariant
> preserved).
>
> This is a **single mega-PR** onto branch `v0.6.0`, stacked by
> topic — not a multi-PR rollout.

---

## §0. North Star

**Goal:** make Cairn smaller, clearer, and faster to install —
without losing any capability the user actually exercises.

**Why v0.6.0 specifically:** v0.5.0 shipped 22 crates, two
server binaries (one of them orphaned in the host tarball),
six agent integrations (four of them fragile), a `Login` and
`Update` subcommand nobody used, and a 39-file churn of
`cairn-cli` references that confuses every new contributor. The
product is good; the surface is too big.

**Three sentences on what "cleanup" means here:**
1. **One binary per role** — `cairn` (host) + `cairn-server`
   (in-container). No more `cairn-cli`, no more `cairn serve` on
   the host.
2. **One agent set** — Claude Code, Codex CLI, OpenCode. The
   other three were one-command setup over a fragile config
   schema.
3. **One way to bootstrap admin** — env vars, set in
   `docker-compose.yml`. The dashboard form was always
   unreviewed; the `cairn-server admin password` reset CLI was
   never wired to the new in-container server.

---

## §1. The 11 commits

Each commit is independently buildable, formatted, clippy-clean,
and passes the test suite. Commit 4, 5, 6, and 11 additionally pass
`./scripts/e2e.ps1` (20/20).

| #  | Commit                                                                  | Stacks on |
|----|-------------------------------------------------------------------------|-----------|
| 1  | `chore(deps): remove self_update, dotenvy; drop Login + Update`         | base      |
| 2  | `feat(env): promote CAIRN_ADMIN_USERNAME + CAIRN_ADMIN_PASSWORD`        | 1         |
| 3  | `refactor: rename crates/cairn-cli → cairn-client; binary → cairn`      | 1         |
| 4  | `feat: drop Cursor/VSCode/Windsurf; add Codex CLI`                      | 3         |
| 5  | `docs(setup): confirm admin ops live in the web UI + env-only`          | 2         |
| 6  | `refactor: delete cairn-server crate; entrypoint → cairn-api`           | 2         |
| 7  | `chore: drop dead code`                                                 | 6         |
| 8  | `chore: gitignore log/txt working-tree spam`                            | 1         |
| 9  | `docs: scrub stale cairn serve/token/pair-code/cairn-server refs`       | 6, 4      |
| 10 | `docs: archive PLAN_v0.5.0; write PLAN_v0.6.0; add ADR 028/029/030`     | 9         |
| 11 | `chore(release): version 0.6.0; CHANGELOG entry; Cargo.lock`            | 10        |

### 1.1 Commit 1 — `chore(deps): remove self_update, dotenvy from cairn-cli; drop Login + Update subcommands`

Removes two deps that were only used by `cairn login` and
`cairn update`. Both subcommands were in v0.5.0 but were never
exercised: `cairn login` printed "use the web UI", and
`cairn update` is now owned by the install script (`./scripts/install.sh`).

- `Cargo.toml`: drop `self_update`, `dotenvy` from
  `[workspace.dependencies]` (only `cairn-cli` was using them).
- `crates/cairn-cli/src/main.rs`: remove `Cmd::Login`,
  `Cmd::Update`, and their match arms.
- Test count: 343 → 343 (no test removed, no test added).

### 1.2 Commit 2 — `feat(env): promote CAIRN_ADMIN_USERNAME + CAIRN_ADMIN_PASSWORD; env-only admin bootstrap`

Promotes the two env vars from "optional override" to "the only
way to bootstrap the admin account." Removes the dashboard
form that pre-created the admin with a default password (a
v0.4.0 → v0.5.0 footgun).

- `crates/cairn-api/src/admin.rs`: new
  `pub fn bootstrap_admin_from_env(state: &AppState) -> Result<()>`.
- `crates/cairn-api/src/lib.rs`: `pub mod admin;`.
- `docker-compose.yml`: `cairn` service gains
  `CAIRN_ADMIN_USERNAME` + `CAIRN_ADMIN_PASSWORD` env entries.
- `.env.example`: documents the two vars, marks them required.
- `Dockerfile`: no change (env is set at compose time).

### 1.3 Commit 3 — `refactor: rename crates/cairn-cli → cairn-client; binary cairn-cli → cairn`

The bulk rename. Touches 39 files: docs, scripts, MCP config
examples, AGENTS.md, install scripts. Crate name changes to
match its architectural role.

- `crates/cairn-cli/` → `crates/cairn-client/`.
- `crates/cairn-client/Cargo.toml`: `name = "cairn-client"`,
  `[[bin]] name = "cairn"`.
- `Cargo.toml`: `[workspace.members]` and
  `[workspace.dependencies]` updated.
- All `cairn-cli` string references → `cairn` (scoped replace,
  no `cairn-cli-server` partial matches).
- Historical refs in `CHANGELOG.md`, `docs/PLAN_v0.5.0.md`,
  `docs/audits/*` left verbatim — they describe v0.5.0 reality.

### 1.4 Commit 4 — `feat: drop Cursor/VSCode/Windsurf; add Codex CLI`

Restricts `KNOWN` agents to three. Adds Codex CLI MCP
integration. The four removed agents had fragile MCP config
schemas that broke with upstream changes.

- `crates/cairn-client/src/setup.rs`: `KNOWN = &["claude-code",
  "codex", "opencode"]`. New `merge_codex_block`,
  `render_codex_block`, `install_codex` functions.
- `crates/cairn-client/src/doctor.rs`: `detect_agent` adds
  codex arm. `check_agents` iterates 3.
- `crates/cairn-client/src/rules.rs`: `KNOWN` matches setup.
- New tests: `codex_round_trip`, `codex_skips_unchanged`.

### 1.5 Commit 5 — `docs(setup): confirm admin ops live in the web UI + env-only bootstrap`

User-facing confirmation that admin ops (token create / revoke,
pair-code generation) live in the dashboard under **You →
Tokens** and **You → Pair**. The CLI never had these
subcommands in v0.5.0; the dashboard already drove the
`/api/devices/*` HTTP routes.

- `docs/ADMIN.md`: new file. Sections: env bootstrap, dashboard
  surface, curl equivalents, password rotation.
- `docs/UPGRADING.md`: v0.4.0 → v0.5.0 admin-rotation note
  updated for v0.5.0 → v0.6.0.

### 1.6 Commit 6 — `refactor: delete cairn-server crate; entrypoint → cairn-api::bin::cairn-server`

The 50-LOC wrapper crate is gone. The in-container server is a
`[[bin]]` in `cairn-api`.

- `crates/cairn-server/`: deleted (Cargo.toml, lib.rs, main.rs,
  pair.rs).
- `crates/cairn-api/src/bin/cairn_server.rs`: new. Calls
  `bootstrap_admin_from_env` then `cairn_api::serve`.
- `crates/cairn-api/Cargo.toml`: `[[bin]] name = "cairn-server"`.
  Gains `anyhow`, `tracing-subscriber`.
- `Dockerfile`: `ENTRYPOINT ["cairn-server"]`.
- `.github/workflows/release.yml`: matrix drops the second
  `bin` entry.

### 1.7 Commit 7 — `chore: drop dead code`

Removes functions that no caller exercises. All removed
functions verified by `rg` to have zero references.

- `crates/cairn-api/src/events.rs`: drop `KIND_STATS`,
  `KIND_CHECKPOINT`, `KIND_VECTOR`, `KIND_GRAPH` (kept
  `KIND_AUDIT`, `KIND_MEMORY`, `KIND_DRIFT`). `backfill` is
  now `pub` (was private dead).
- `crates/cairn-api/src/metrics.rs`: drop `source_breakdown`
  function and the `HashMap` import.
- `crates/cairn-ingest/src/lib.rs`: drop `write_tmp` helper.

### 1.8 Commit 8 — `chore: gitignore log/txt working-tree spam`

`.gitignore` blocks `/*.log` and `/*.txt` with an explicit
allowlist of source-of-truth files (`README.md`,
`CHANGELOG.md`, `Cargo.toml`, etc.). Catches the
`v060-c{N}-msg.txt` scratch files in the repo root.

### 1.9 Commit 9 — `docs: scrub stale cairn serve/token/pair-code/cairn-server admin refs`

Touches 33 files. Pure doc/comment cleanup. No code changes.

- `.claude/settings.json`: 4× `cairn-cli hook` → `cairn hook`.
- `.mcp.json`: `command: "cairn-cli"` → `command: "cairn"`.
- `.env.example`: stale `cairn serve`/`cairn-server` references
  → in-container server.
- `docker-compose.yml`: "run `cairn serve` directly" → "terminate
  TLS at a reverse proxy".
- `AGENTS.md`: 22-crate → 21-crate count, binary table
  rewritten, dep graph updated.
- `CONTRIBUTING.md`: 22 → 21 crates; `cargo run -p cairn-server --
  serve` → `docker compose up -d cairn` +
  `cargo run -p cairn-api --bin cairn-server -- serve`.
- `docs/UPGRADING.md`, `docs/TESTING.md`, `docs/WEB.md`,
  `docs/PLAN.md`, `docs/ROADMAP.md`: same scrub pass.
- `crates/cairn-mcp/src/resources.rs`: 7 "cairn-server" →
  "in-container server" in resource descriptions.
- `crates/cairn-client/src/{onboard,pair}.rs`: drop
  `cairn serve` / `cairn pair-code` references.
- `web/src/app/(app)/you/{sessions,settings}/page.tsx` +
  `web/src/app/(app)/trust/score/page.tsx`: drop `cairn-cli`
  and `cairn-server admin` references.
- `scripts/e2e*.ps1`: drop dead `E2E_BinCairnCli` var +
  `cairn-cli` references.

Historical refs in `CHANGELOG.md`, `docs/DECISIONS.md`,
`docs/PLAN_v0.5.0.md`, `docs/audits/*` left verbatim.

### 1.10 Commit 10 — `docs: archive PLAN_v0.5.0; write PLAN_v0.6.0; add ADR 028/029/030`

- `docs/PLAN_v0.5.0.md` → `docs/archive/PLAN_v0.5.0.md`.
- `docs/PLAN_v0.6.0.md`: this file.
- `docs/DECISIONS.md`:
  - **ADR-028**: drop Cursor/VSCode/Windsurf; add Codex CLI.
  - **ADR-029**: delete `cairn-server`; entrypoint in
    `cairn-api::bin::cairn-server`.
  - **ADR-030**: rename `cairn-cli` → `cairn`; crate →
    `cairn-client`.

### 1.11 Commit 11 — `chore(release): version 0.6.0; CHANGELOG entry; Cargo.lock`

Final commit for the PR.

- `Cargo.toml`: `[workspace.package] version = "0.6.0"`.
- `CHANGELOG.md`: v0.6.0 entry on top (newer-first order). Keep
  historical `cairn token create` line in v0.5.0 entry.
- `Cargo.lock`: regenerated via `cargo update --workspace`.

---

## §2. Test invariant

Across all 11 commits, `cargo test --workspace` must report
**343 passed, 5 ignored** — no test added, no test removed, no
test silently broken. Verified at commits 7 and 9 logs.

Pre-existing flakes (out of scope for v0.6.0):
- `cairn-api/src/ledger.rs:288` `ledger_detects_tampered_field`
  — pre-dates this work, not addressed.

---

## §3. What is NOT in v0.6.0

- **No new features.** No new endpoints, no new agent
  integrations beyond Codex, no new CLI subcommands.
- **No new dependencies.** `Cargo.lock` grows by 0 lines
  (only dep removals: `self_update`, `dotenvy`).
- **No breaking changes to the wire protocol.** The
  `/api/devices/*` routes are unchanged. Existing
  `cairn-cli` clients from v0.5.0 will print a warning about
  the binary rename but keep working (the user just has to
  update the MCP config to `command: "cairn"`).
- **No HTTP admin routes.** Admin ops live in the dashboard
  only. `POST /api/auth/pair` was already there for device
  pairing; no new admin-only route.
- **No follow-up work.** `save_admin` (password rotation via
  the in-container binary) is `#[allow(dead_code)]` and stays
  that way until v0.7.0.

---

## §4. Risk register

| Risk                                                          | Mitigation                                                                  |
|---------------------------------------------------------------|-----------------------------------------------------------------------------|
| v0.5.0 → v0.6.0 breaks the install script                     | `scripts/install.sh` + `scripts/install.ps1` updated to use `cairn` not `cairn-cli` |
| CI release matrix still produces a `cairn-cli` artifact       | Verify `.github/workflows/release.yml` matrix after C6                       |
| `cargo run -p cairn-server` fails on existing dev setups      | `CONTRIBUTING.md` updated; release notes in CHANGELOG.md                    |
| Bulk rename misses a `cairn-cli` string                        | Verified by `rg "cairn-cli" D:/code/Cairn --type-add rust --type-add json --type-add yaml --type-add toml` returning zero non-historical hits |
| Test count drifts                                              | Logged after every commit; invariant 343 + 5 ignored                       |
| PowerShell CRLF on Windows                                     | `cargo fmt --all` normalizes; `git add` re-stages                           |

---

## §5. Follow-ups for v0.7.0

- `save_admin` password rotation endpoint (currently
  `#[allow(dead_code)]`).
- `cairn run` (the harness subcommand) splits from
  `crates/cairn-client/src/main.rs` (~650 LOC main.rs) into
  a separate crate to shrink the client binary.
- Agent-specific dashboards: today all three agents share
  one config UI; v0.7.0 could show the per-agent hook
  registration as a separate row.
