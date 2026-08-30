# Contract: Web Control Plane

**Feature**: `005-server-authoritative-autonomous-memory`

Feature 004 shipped ten screens and explicitly deferred web administration and team-curation
to this feature (research.md §5.3). This contract closes that gap: every screen below is
backed by a project-membership-enforcing or admin-enforcing read API (never a client-side
filter), every list is bounded and paginated, and every view that would otherwise show an
empty section for material it cannot see says so rather than rendering nothing (FR-892,
FR-893, FR-894a).

Stack unchanged: Next.js 15 App Router, React 19, Tailwind v4, `@tanstack/react-query`,
client components, cookie session auth (`web/lib/api.ts`).

---

## 1. Screens

| Screen | Route | Status | Purpose (FR) | Endpoints called |
|---|---|---|---|---|
| Dashboard | `/projects/[id]` | **EXTENDED** | Memory funnel (§3), FR-879, FR-880 | `GET /api/projects/{id}/funnel` |
| Activity feed | `/projects/[id]/activity` | **NEW** | Recent activity at semantic level (§4), FR-881, FR-882 | `GET /api/projects/{id}/activity` |
| Memory explorer | `/projects/[id]/memory` | **EXTENDED** | Full field set, filterable (FR-883) | `GET /api/projects/{id}/memories` (extended) |
| Memory detail | `/projects/[id]/memory/[memoryId]` | **NEW** | Content, provenance, relations, evidence summary, verification, reinforcement, retrieval usage, origin (FR-884, FR-885) | `GET /api/memories/{id}` (extended) |
| Retrieval traces | `/projects/[id]/retrievals` | **NEW** | List of past retrievals (FR-886) | `GET /api/projects/{id}/retrieval-traces` |
| Retrieval trace detail | `/projects/[id]/retrievals/[traceId]` | **NEW** | Trigger, candidates, selection, budget, delivery state — never briefing text (FR-839, FR-846a, FR-886) | `GET /api/retrieval-traces/{id}` |
| Agents / integration health | `/projects/[id]/agents` | **NEW** | Per-agent, per-capability health (FR-887) | `GET /api/projects/{id}/integration-health` |
| Domains | `/projects/[id]/domains` | **NEW** | Project, personal, team kept visibly separate; personal includes owner-only records of type `pattern` (FR-888, FR-708c) | `GET /api/projects/{id}/memories?domain=project`, `GET /api/personal/knowledge`, `GET /api/patterns`, `GET /api/team/knowledge` |
| Team curation | `/team` | **NEW** | Review, ratify, retire — admin-restricted actions (FR-889, FR-889a) | `GET /api/team/knowledge`, `POST /api/team/{id}/ratify`, `POST /api/team/{id}/retire` (ratify/retire pre-exist, `global.rs:1068-1160`) |
| System health | `/system` | **NEW** | Ingest, consolidation, retrieval backlogs and failures (FR-891) | `GET /api/system/health` |
| Admin users | `/admin/users` | **NEW** | Web equivalent of `cairn user` (FR-890) | `GET/POST /api/admin/users`, `PATCH /api/admin/users/{id}`, `POST /api/admin/users/{id}/reset-password` (all pre-existing, `api.rs:46-51`) |

Unaffected: `/tasks`, `/sessions`, `/sessions/[sessionId]`, `/tokens`, `/login` — no new
requirement touches them.

### 1.1 Left deliberately CLI/API-only (FR-890)

| Surface | Stays | Reason |
|---|---|---|
| `cairn team propose` (creating a team-knowledge proposal) | CLI | Creation travels the machine-oriented sync/batch protocol (idempotency keys, writer sequencing, daemon-held credential) — the web screen covers review/ratify/retire, not authorship |
| `cairn remember --personal`, `--team` (writing personal/team knowledge) | CLI | Same reason — write path is sync/batch, not a browser form |
| Project creation and membership (`POST /api/projects`, `/api/projects/{id}/members`) | CLI/API only | Not among this feature's ten screens; `/api/projects` already serves the CLI unchanged, and adding a web equivalent is out of scope here |
| Server instance identity, environment-seeded admin bootstrap | CLI/deploy-time only | Not a runtime admin action — nothing to click |

---

## 2. New API endpoints

| Method + path | Auth | Membership guard | Response shape (summary) |
|---|---|---|---|
| `GET /api/projects/{id}/funnel` | `SettledUser` | `require_member` — refusal, not empty (FR-894a) | `{ stages: [{ stage, count: int\|null }] }` — §3 |
| `GET /api/projects/{id}/activity` | `SettledUser` | `require_member` | `{ items: [...], cursor }`, paginated (§7) |
| `GET /api/projects/{id}/memories` | `SettledUser` | `require_member` (unchanged) | Existing shape + `importance`, `verification`, `verification_authority`, `origin_kind`, `reinforcement_count`, `relation_count` |
| `GET /api/memories/{id}` | `SettledUser` | `require_member` on the row's `project_id` (unchanged) | Existing shape + `relations[]`, `evidence_summary` (counts/ids, never content), `verification{state,authority,last_verified_at,stale}`, `reinforcement_count`, `retrieval_usage[]` (bounded, §7), `origin_kind` |
| `GET /api/projects/{id}/retrieval-traces` | `SettledUser` | `require_member` | `{ traces: [{trace_id, trigger, delivery_point, degradation_level, delivery_state, created_at}], cursor }` |
| `GET /api/retrieval-traces/{id}` | `SettledUser` | `require_member` on the trace's `project_id`, **plus** §6 withholding | `{ trigger, items: [{ref_kind, domain, knowledge_id, status, rank}], budget_tokens, budget_spent, latency_ms, delivery_state, failure_reason }` — `knowledge` requires a domain; `pattern` requires `domain: null`; no briefing text field exists |
| `GET /api/projects/{id}/integration-health` | `SettledUser` | `require_member` | `{ rows: [{agent, capability, stage, status, evidence_kind, observed_at, stale}] }` |
| `GET /api/personal/knowledge` | `SettledUser` | none (owner-scoped by construction — `owner_user_id = caller`, not a project concept) | `{ items: [...], cursor }` |
| `GET /api/patterns` | `SettledUser` | none (personal-domain patterns; `owner_user_id = caller`) | `{ items: [{pattern_id, domain: "personal", type: "pattern", ...}], cursor }` |
| `GET /api/team/knowledge` | `SettledUser` | none (server-global); visibility filtered per `sync-namespaces.md` §1a (`proposed` visible to author + admin only) | `{ items: [...], cursor }` |
| `GET /api/projects/{id}/consolidation-runs` | `SettledUser` | `require_member` | `{ runs: [{ run_id, started_at, finished_at, events_claimed, candidates_proposed, candidates_accepted, candidates_refused, refusal_reasons, extractor_kind }] }`, paginated (FR-894a) |
| `GET /api/system/health` | `AdminUser` | none — admin, not membership | `{ ingest: {...}, consolidation: {...}, retrieval: {...} }` — §5 |

Every row above is new **except** `ratify_team`/`retire_team` and the three admin-user
routes, which pre-exist and are reused unchanged (FR-894's "no view specified that no
endpoint can serve" is satisfied by citing exactly which routes exist today versus which this
feature adds).

### 2.1 FR-894a — refusal, never an empty list

Every project-scoped row above calls `auth::require_member(&state.pool, project_id,
user.id())` **before** its query, exactly as `project_memories`/`memory_detail` already do
(`api.rs:1246-1330`). A non-member gets `403 forbidden` (`error.rs:46-48`), not `{ "items":
[] }` — an empty list would tell a non-member the project exists and is simply empty for
them, which is indistinguishable from a missing guard. This is the same rule §2's table
states per-row; it is restated here because it is the one rule FR-892 (web matches API) is
vacuous without.

---

## 3. Funnel definition (FR-879, FR-880)

Twelve stages, in the order FR-879 lists them, each a `{ stage, count }` pair:

| Stage | Source |
|---|---|
| `active_agents` | distinct `agent` values with a `safe_events` row in the window |
| `sessions` | `sessions` rows for the project in the window |
| `safe_events_received` | `safe_events` rows |
| `capture_failures` | `capture_dispositions (server)` where `disposition = 'capture_deadline_exceeded'` |
| `consolidation_runs` | `consolidation_runs` rows |
| `candidates_produced` | `knowledge_candidates` rows |
| `knowledge_accepted` | `knowledge_candidates` where `decision = 'accepted'` only — a corroboration is not a distinct claim (FR-798a) |
| `candidates_rejected_or_duplicate` | `decision IN ('refused','duplicate')` |
| `reinforcements` | `decision = 'reinforced'` |
| `conflicts` | `decision = 'conflicted'` |
| `retrievals` | `retrieval_traces` rows |
| `delivery_failures` | `retrieval_traces` where `delivery_state = 'failed'` |

**Zero vs. unavailable (FR-880).** `count` is `0` when the query ran and found nothing, and
`null` when the underlying mechanism does not exist for this deployment or window — for
example, a schema below the one that added `verification_reports` reports that stage as
`null`, never `0`. The dashboard renders `0` as the number and `null` as `—`, never
collapsing the two. This is the same distinction `integration_health.status = 'no_evidence'`
already makes for a single capability (§5); the funnel makes it for a whole stage.

---

## 4. Activity feed (FR-881, FR-882)

Two item families, interleaved by time, newest first:

| Family | What it is | Default subset |
|---|---|---|
| Safe events | Arrivals from `safe_events` | `session_opened`, `session_resumed`, `session_closed`, `file_changed`, `test_result`, `decision_signal`, `capture_failed` |
| Candidate decisions | Outcomes from `knowledge_candidates` | `accepted`, `conflicted` |

**Why this default, stated rather than left to judgment (FR-882).** The excluded event kinds
— `tool_started`/`tool_succeeded`/`tool_failed`, `file_read`, `context_compacting`/
`context_compacted`, `subagent_started`/`subagent_completed`, `command_executed`,
`test_executed`, `research_activity`, `user_instruction_signal`, `capture_declined`,
`agent_quiesced` — fire once per tool call or internal transition and would make the feed a
firehose of routine activity rather than "what Cairn is receiving and learning." The included
seven are the ones that mark a session boundary, a durable artifact change, a test outcome, an
explicit decision, or a capture failure — each individually meaningful. `reinforced` and
`duplicate`/`refused` candidate decisions are excluded by default for the same reason
(reinforcement already has its own funnel stage, §3) but remain one click away.

**Widening.** A `kinds` query parameter accepts any subset of the full 21 event kinds
(data-model.md §1.2) plus the five candidate decisions; the UI's "show everything" control
sets it to the full set. Widening is always an explicit, remembered-per-view action, never
the initial state.

---

## 5. Health status vocabulary (FR-851–FR-856, FR-887)

| Status | Meaning | Display |
|---|---|---|
| `supported` | Vendor exposes it, Cairn uses it, evidence confirms it fired | Green, "working" |
| `unsupported_by_vendor` | The vendor does not expose this capability at all | Grey, "not offered by this agent" |
| `declined_by_cairn` | The vendor exposes it, Cairn chooses not to depend on it (e.g. OpenCode prompt-time, research.md §9.1) | Grey, "not enabled" — distinct from the vendor's own absence |
| `adapter_unimplemented` | Cairn has not built the adapter code for it yet | Amber, "not yet implemented" |
| `runtime_failure` | It fired, or should have, and failed | Red, "failing" |
| `no_evidence` | No observation exists either way (FR-856) | Grey, "no evidence" — **never** rendered as failing or as working |

Two cross-cutting flags on every row, not folded into `status` (FR-852, FR-853, FR-860):

- **`evidence_kind ∈ {introspection, observation}`.** A configuration file read back and
  found to match (`introspection`) is displayed with a distinct badge from a capability
  actually observed firing at runtime (`observation`) — the two no longer collapse into one
  `Confidence::Verified` value the way research.md §5.4 found them doing today.
- **`stale`**, computed client-side from `observed_at` against a per-capability freshness
  window. A `supported` row that is also `stale` reads "worked as of `observed_at`," not
  "working," so an integration that functioned last month is never reported as functioning
  now on that basis alone (FR-860).

---

## 6. Privacy rules per view (FR-892, FR-893, FR-846a)

| View | Rule |
|---|---|
| Memory detail | Evidence is local to the capturing machine; the section states "evidence content is local to `<session>` and not held here" rather than rendering empty (FR-893, matching the existing `evidence_content_available: false` field, `api.rs:1310-1329`) |
| Retrieval trace detail | Never renders briefing text — the response shape (§2) carries no such field, so there is nothing for the view to accidentally render (FR-839, FR-886). Each item is `KnowledgeRef(domain,id)` or `PatternRef(pattern_id)` in discriminator form. A referenced record the reader may not see (another account's personal knowledge, including a pattern the reader does not own) is omitted from `items[]` entirely, never shown as an opaque id that still discloses existence (FR-846a) |
| Domains — personal panel | Shows the caller's ordinary personal knowledge and personal-domain records of type `pattern`, visibly separated by type. Only the caller's own `owner_user_id`; never another account's, even to a project co-member or an admin, short of that admin's own account. A pattern never appears in the team panel unless its content separately became a team proposal and was ratified; the pattern itself remains personal. |
| Domains — team panel | `proposed` rows visible only to their author and to admins; `authoritative`/`retired` visible to all authenticated accounts (unchanged from `sync-namespaces.md` §1a, now surfaced in a read view for the first time) |
| Activity feed | No event kind ever carries user/assistant free text (data-model.md §1.3) — this is a property of the data, not the view, but the view renders no field capable of holding it either |
| System health, Admin users | `AdminUser` only; a non-admin's request is `403`, and the page itself checks `me().role` before rendering the nav entry — belt, not suspenders, per FR-892 |

Every rule above enforces what the API already enforces (§2); no view computes an
authorization decision the server has not already made.

---

## 7. Pagination and bounding (FR-895)

| List | Bound | Pagination |
|---|---|---|
| Memory explorer | `limit` ≤ 100, default 25 (existing clamp, `api.rs:1247`) | offset via `LIMIT`/relevance-ranked, unchanged |
| Activity feed | `limit` ≤ 100, default 50 | keyset on `(received_at, event_id)` — stable under concurrent insertion, unlike an offset that would skip or repeat rows as new events arrive |
| Retrieval traces list | `limit` ≤ 100, default 25 | keyset on `(created_at, trace_id)` |
| `retrieval_usage[]` on memory detail | 20 most recent | no further pagination — a "view all" link goes to the traces list filtered by that `KnowledgeRef` |
| Personal / pattern / team knowledge feeds | `limit` ≤ 100, default 25 | keyset, matching `retrieval_trace_items`'s existing 200-per-trace cap (data-model.md §3) in spirit |
| Admin users list | `limit` ≤ 100, default 50 | offset — bounded by account count, which is small by construction |

No list view in this contract returns an unbounded result, and none accepts a client-supplied
limit above its stated ceiling — a request above the ceiling is clamped, not refused, matching
the existing `project_memories` behavior (`.clamp(1, 100)`).

---

## 8. Team curation — concurrency preserved, not merely re-authorized (FR-889a)

The web screen's ratify/retire buttons call the **existing** `POST /api/team/{id}/ratify` and
`POST /api/team/{id}/retire` handlers unchanged (`global.rs:1068-1160`). This is stated
explicitly because the risk FR-889a names is a new handler, not a new button: a web-specific
route that reads state, checks it in application code, and then issues a separate `UPDATE`
would reopen the double-ratification race the existing single-statement compare-and-swap
already closes (`WHERE state = 'proposed'` / `WHERE state = 'authoritative'`, each its own
atomic `UPDATE ... RETURNING`), and would make "un-retire" expressible by giving a client a
window to retry a stale read against newly-changed state. **No new mutation endpoint is
introduced for team curation.** The web screen only adds a `GET /api/team/knowledge` read
path (§2) in front of actions that already exist and already carry this guarantee.

---

## 10. Cross-domain references, funnel counting and health attribution

- **Consolidation runs get a read API.** `GET /api/projects/{id}/consolidation-runs`,
  `SettledUser` + `require_member`, paginated, returning run id, timings, events claimed,
  candidates proposed/accepted/refused, refusal reasons and extractor kind. FR-894a names
  consolidation runs among the reads that need a membership guard; an unguarded or absent API
  fails it either way.
- **`knowledge_accepted` counts claims, not corroboration.** The funnel counts
  `decision = 'accepted'` only. Counting `'reinforced'` too would count a corroboration record
  as a distinct claim, which FR-798a forbids "anywhere a user reads a count of what Cairn
  knows". Reinforcements are reported in their own funnel stage.
- **`capture_dispositions` is a server table.** The funnel reads it there
  (`data-model.md` §6). The local counters are a machine-local mirror and are never the source
  the control plane reads.
- **Integration health is per machine.** Rows are keyed by `writer_id` as well as account and
  agent (FR-857), written by an authenticated report from the daemon, and displayed with the
  machine identified. A capability verified on one machine is not reported as verified
  everywhere.
