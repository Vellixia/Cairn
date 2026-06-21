# Cairn v0.5.0 — The Smart Memory Plan

> **Make AI have smart memory.**
>
> v0.5.0 = PLAN.md (v0.4.0 web dashboard shipped) + the six dimensions of smart memory
> + the integration moat (Memory Provenance Graph + Visualization).
>
> This plan builds on the existing foundation and is sequenced for trust: real-time
> foundation → CRUD/CRUD-adjacent UX → graph layer → session/reliability → assembler/savings
> → wizard/landing → depth polish, then CLI/MCP surface, distribution, federation, and the
> Phase 5 killer features.

---

## §0. North Star

**Goal:** make any AI agent have smart memory — across sessions, devices, projects, and time —
without losing a single byte and without burning context budget.

**Why v0.5.0 specifically:** the v0.4.0 web dashboard is shipped and dogfooded. The hard parts
of "ingest, compress, retrieve, assemble, verify" are done. What remains is making memory
**smart** — not just stored — and exposing it through a UX that earns the daily-driver slot
in a developer's workflow.

**Three sentences on what "smart" means here:**
1. **Memory that thinks for itself** — confidence scoring, provenance edges, contradiction
   detection, tier promotion, decay, and crystallization happen automatically.
2. **Memory you can see and shape** — every memory is editable, pinnable, promotable, and
   deletable from a dashboard that visualizes the memory graph as a first-class artifact.
3. **Memory that survives everything** — compaction, drift, crashes, devices, projects, and
   time. The original is always one `expand` away, the signed ledger proves it.

---

## §1. The Six Dimensions of Smart Memory

Cairn's value isn't any single feature — it's the integration. The six dimensions below are
how we measure "smart," and where each lives in the codebase.

### 1.1 Continuity — memory survives across sessions and devices

| Where today | Where v0.5.0 |
|---|---|
| `cairn-memory` 4-tier (working → episodic → semantic → procedural) | Same; add Cross-Session Protocol (CCP) auto-restore |
| `cairn-share` export/import for offline move | Same |
| Multi-device sync (LWW on `updated_at`) | Add session concept: `sessions/<id>.json` with tasks/findings/decisions/touched_files/next_steps |
| `SessionStart` hook injects anchor + profile + memories | Inject CCP block too (auto, ~400 tokens) |

**Credit:** Cross-Session Protocol pattern adopted from **lean-ctx** (`docs/reference/03-memory-and-knowledge.md` §1).

### 1.2 Scope — memory knows where it belongs

| Where today | Where v0.5.0 |
|---|---|
| Implicit project scoping via HelixDB key prefixes | Explicit `project_hash` field on every memory (lean-ctx `knowledge/<project-hash>` pattern) |
| `importance: f32` set at remember-time | Add `confidence: f32` that evolves with reinforcement and decay |
| No confidence surface in UI | Confidence bar in Wakeup/Recall/Memory Graph, sort/filter by min confidence |
| No universal gotchas | Universal gotchas (`knowledge/universal-gotchas.json` idea) for cross-project footguns |

**Credit:** Confidence scoring + reinforcement adopted from **agentmemory** (`src/functions/lessons.ts`).

### 1.3 Recall — memory retrieves the right thing at the right time

| Where today | Where v0.5.0 |
|---|---|
| BM25 lexical recall + 4-tier wakeup + Ebbinghaus decay | Finish hybrid search RRF: BM25 + HNSW vectors + graph leg |
| HNSW via HelixDB (partial) | Graph-proximity boost for recently touched files and memory nodes |
| No rerank | Add rerank + MMR diversity (post-retrieval) |
| No semantic search CLI | Add `cairn search` CLI + `/api/search` hybrid endpoint |

**Credit:** RRF formula and hybrid search fusion adopted from **lean-ctx** (`LEANCTX_FEATURE_CATALOG.md` §Hybrid Search Fusion).

### 1.4 Reliability — memory can be verified and recovered

| Where today | Where v0.5.0 |
|---|---|
| `cairn-guard`: verify, anchor, checkpoint, rollback, reliability score | Persist `verify` results into HelixDB drift log |
| `verify` flags large unreplaced deletions | Reliability center: list drift events, approve/reject flagged edits |
| Audit log is in-memory only | Move audit to HelixDB-backed store (restart-safe) |
| Reliability score visible in UI | Drift events detail view linked to affected file/memory |

### 1.5 Compression — memory shrinks the window without losing fidelity

| Where today | Where v0.5.0 |
|---|---|
| 4 read modes, ~13-tok re-reads, tree-sitter AST, shell compression | This is Cairn's strongest dimension — keep and polish |
| `cairn-cli bench` measures a codebase | Live `/api/metrics` + savings dashboard: cumulative tokens/$ saved, hit rate, bounce rate |
| No signed ledger | Signed JSON ledger of every compression/recover event |

### 1.6 Personalization — memory learns the user

| Where today | Where v0.5.0 |
|---|---|
| `cairn-profile`: `prefer`/`profile` auto-learned from prompts | Profile editor: view / approve / edit / delete preferences + do/don't rules |
| Preferences injected at session start | Same, but with explicit confidence + user override flag |
| No UI for preferences | New `/dashboard/profile` page + profile chip in topbar |

---

## §2. The Eight Big Ideas

These are the concrete initiatives that turn the six dimensions into shipped features.

### 2.1 Memory Provenance Graph — Cairn's integration moat

**What it is:** every memory gets a set of typed edges to other memories and to code artifacts.

- `memory → derived_from → memory` — a crystallized lesson came from action A and B
- `memory → contradicts → memory` — two memories conflict; needs user resolution
- `memory → supersedes → memory` — a newer memory replaces an older one
- `memory → applies_to → file|symbol|project` — lean-ctx-style graph relevance for code

**Why it matters:** lean-ctx has code graphs, agentmemory has session graphs, Cairn can have
**memory provenance graphs**. This is the unique integration moat: a single graph where code,
sessions, and knowledge connect.

**Where stored:** HelixDB native graph edges.

**Surfaced in:** Memory Graph Visualization (Idea 8) + Wakeup "related memories" hints.

### 2.2 Memory Graph Visualization — the demo video moment

**What it is:** a force-directed, interactive graph in the web dashboard where:

- Nodes = memories, sized by `confidence`, colored by `tier`
- Edges = `derived_from` / `contradicts` / `supersedes` / `applies_to`
- Clicking a node opens the memory detail panel with edit/promote/delete
- Filtering by tier, kind, project, confidence range
- "What does the agent know about X?" → subgraph centered on a query

**Library choice:** `react-force-graph-2d` (clean API, d3-force underneath, ~50KB). The
dashboard is Next.js static-export; react-force-graph-2d works with `ssr: false`.

**New route:** `/dashboard/memory/graph`.

### 2.3 Confidence + Reinforcement Loop — adopted from agentmemory

**What it is:** every memory gets a `confidence: f32` field.

- On each successful `recall` hit, `confidence = min(1.0, confidence + 0.1 * (1.0 - confidence))`
  (agentmemory's proven reinforcement curve).
- On consolidation, confidence decays by a rate inversely proportional to `importance`.
- When `confidence` falls below a threshold (default 0.1) and the memory has zero
  reinforcements, it is marked for review or deletion.

**Where:** `cairn-memory` schema + `recall` path + `consolidate` path.

**Default for existing memories:** 0.5 (neutral).

### 2.4 Cross-Session Protocol (CCP) — adopted from lean-ctx

**What it is:** at session start, Cairn injects a structured "where we were" block.

- `tasks: [{ id, title, progress }]`
- `findings: [{ text, source_file, confidence }]`
- `decisions: [{ text, rationale, confidence }]`
- `touched_files: [{ path, mode, handle }]`
- `next_steps: [text]`

**Storage:** `sessions/<id>.json` with `sessions/latest.json` pointer.

**Auto-restore:** the `SessionStart` hook reads `latest.json` and injects the block.

**CLI surface:** `cairn session task|finding|decision|status|save|load|reset`, plus
`sessions list|show|cleanup|doctor`.

### 2.5 Cost-Saving Ledger — new for Cairn

**What it is:** every tool call that saves context budget gets a ledger entry.

- `bytes_in`, `bytes_out`, `tokens_saved`, `cost_usd_saved`
- `source`: `read-cache`, `ast-signatures`, `shell-compress`, `wakeup`, `assemble`
- Cumulative per project, per device, per user

**Where:** `/api/ledger`, `/api/metrics`, dashboard "Savings & Recover" page.

**Signed:** SHA-256 HMAC per entry, verifiable offline.

### 2.6 Project-Aware Memory Hygiene — new for Cairn

**What it is:** tooling to keep memory from growing stale or contradictory.

- `cairn doctor memory` — counts by tier, importance distribution, decay schedule,
  contradiction count, graph density
- `cairn memory prune --dry-run` — preview consolidation removals
- `cairn memory crystallize` — summarize a completed session's working-tier memories into
  semantic-tier "crystals" (agentmemory's pattern)
- Web UI: Memory Workspace with bulk select + promote/delete/pin

### 2.7 Federation as First-Class — extends cairn-share

**What it is:** today `cairn share export` makes a sanitized bundle and `contribute`/`pull`
pools it. v0.5.0 turns this into a discoverable, signed package system.

- `.cairnpkg` file format: manifest.json + knowledge.jsonl + graph.sqlite + session.json +
  patterns.json + gotchas.json + signature.sha256
- Adopt the lean-ctx `.ctxpkg` design (SHA-256 integrity, atomic writes, knowledge merge with
  confidence capping, graph overlay, auto-load) as the **primary `.cairnpkg` format**. The
  `.ctxpkg` extension is recognized as an import alias for interoperability.
- CLI: `cairn pack create|list|info|install|remove|export|import|auto-load|publish|search`
- Registry: embedded in `cairn-server` for self-hosting; optional separate `cairn.sh/registry`
  service only if public traffic justifies it
- Web UI: Collective / Federation manager

**Decision:** `.cairnpkg` is the canonical extension; `.ctxpkg` imports are accepted. The
registry starts embedded in `cairn-server`.

### 2.8 SSE Real-Time Dashboard — simpler than WebSocket

**What it is:** replace 5s polling on Overview and Audit with Server-Sent Events.

- `GET /api/events` returns `text/event-stream`
- Event types: `stats_updated`, `memory_added`, `checkpoint_created`, `audit_event`, `drift_detected`
- Frontend uses native `EventSource` with reconnection
- This is one-way from server to browser — simpler than WebSocket, no upgrade dance

**Where:** `cairn-api` axum SSE endpoint + `web/src/lib/sse.ts` + React Query integration.

---

## §3. New Crate / API / CLI / MCP Surface

### 3.1 Schema additions

**`Memory` struct (`cairn-memory`)**

```rust
pub struct Memory {
    pub id: String,
    pub content: String,
    pub kind: MemoryKind,
    pub tier: MemoryTier,
    pub importance: f32,        // existing
    pub confidence: f32,        // NEW
    pub access_count: i64,      // existing
    pub created_at: u64,
    pub updated_at: u64,
    pub project_hash: Option<String>, // NEW
    pub derived_from: Vec<String>,    // NEW
    pub contradicts: Vec<String>,     // NEW
    pub supersedes: Vec<String>,      // NEW
    pub applies_to: Vec<String>,      // NEW
    pub scope: Scope,                 // NEW (Project / Universal / User)
}
```

**`Session` struct (`cairn-session` — new crate or in `cairn-memory`)**

```rust
pub struct Session {
    pub id: String,
    pub project_hash: String,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub tasks: Vec<Task>,
    pub findings: Vec<Finding>,
    pub decisions: Vec<Decision>,
    pub touched_files: Vec<TouchedFile>,
    pub next_steps: Vec<String>,
    pub memory_ids: Vec<String>, // linked memories
}
```

### 3.2 New / extended crates

| Crate | Responsibility |
|---|---|
| `cairn-memory` | Add confidence, edges, crystallize, timeline |
| `cairn-session` | NEW — CCP storage, latest.json pointer, session CLI |
| `cairn-graph` | NEW — property graph queries for code + memory (or extend `cairn-context`) |
| `cairn-ledger` | NEW — cost-savings ledger, metrics aggregation |
| `cairn-pack` | NEW — context package system |
| `cairn-api` | Add SSE, `/api/metrics`, `/api/sessions`, `/api/memory/*`, `/api/pack/*` |
| `cairn-mcp` | Add tools, resources, prompts, elicitation |
| `cairn-cli` | Add `onboard`, `doctor --fix`, `graph`, `impact`, `callgraph`, `memory timeline`, `memory crystallize`, `session`, `pack`, `metrics`, `search` |

### 3.3 MCP tool surface (16 → 40+)

**New Context tools:**
- `graph` — build/query the property graph
- `impact` — blast radius for a file or symbol
- `callgraph` — callers/callees

**New Memory tools:**
- `memory_edit`
- `memory_delete`
- `memory_pin`
- `memory_promote`
- `memory_timeline`
- `memory_crystallize`
- `memory_graph`

**New Session tools:**
- `session_task`
- `session_finding`
- `session_decision`
- `session_status`
- `session_save`
- `session_load`
- `session_reset`

**New Pack tools:**
- `pack_create`
- `pack_install`
- `pack_search`

**New Metrics tools:**
- `metrics`
- `ledger`

**MCP Resources (6):**
- `cairn://sessions/recent`
- `cairn://memory/wakeup`
- `cairn://memory/graph`
- `cairn://stats/live`
- `cairn://prefs/active`
- `cairn://ledger/savings`

**MCP Prompts (5):**
- `/recall-context`
- `/checkpoint-now`
- `/rollback`
- `/prefer-add`
- `/session-status`

### 3.4 REST API additions

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/events` | SSE stream |
| GET | `/api/metrics` | Token / $ / hit-rate / bounce-rate metrics |
| GET | `/api/ledger` | Signed savings ledger |
| POST | `/api/memory/<id>` | Edit memory |
| DELETE | `/api/memory/<id>` | Delete memory |
| POST | `/api/memory/<id>/pin` | Pin/unpin |
| POST | `/api/memory/<id>/promote` | Promote tier |
| POST | `/api/memory/crystallize` | Crystallize session |
| GET | `/api/memory/graph` | Memory provenance graph |
| GET | `/api/sessions` | List sessions |
| GET | `/api/sessions/<id>` | Session detail |
| POST | `/api/sessions/<id>/resume` | Resume task |
| GET | `/api/search` | Hybrid search RRF |
| GET | `/api/drift` | Drift events |
| POST | `/api/drift/<id>/approve` | Approve flagged edit |
| POST | `/api/drift/<id>/reject` | Reject flagged edit |
| GET/POST | `/api/profile/edit` | Edit preferences |
| GET | `/api/pack/search` | Search registry |
| POST | `/api/pack/publish` | Publish pack |

### 3.5 CLI additions

| Command | What |
|---|---|
| `cairn onboard` | Zero-prompt setup (adopt lean-ctx pattern) |
| `cairn doctor --fix` | Repair hooks/MCP configs non-destructively |
| `cairn graph build|related|context` | Property graph queries |
| `cairn impact <file>` | Blast radius |
| `cairn callgraph callers|callees <symbol>` | Call graph |
| `cairn memory timeline [query]` | Time-based memory search |
| `cairn memory crystallize [session]` | Promote working memories to semantic crystals |
| `cairn session task|finding|decision|status|save|load|reset` | CCP |
| `cairn sessions list|show|cleanup|doctor` | Session management |
| `cairn pack create|list|info|install|remove|export|import|auto-load|publish|search` | Packages |
| `cairn metrics` | Live savings metrics |
| `cairn search <query>` | Hybrid RRF search |

---

## §4. Web Dashboard Plan

### 4.1 New pages

| Route | Purpose |
|---|---|
| `/dashboard/profile` | Profile editor (view / approve / edit / delete preferences) |
| `/dashboard/memory/graph` | Memory Graph Visualization |
| `/dashboard/sessions` | Sessions list + live stream |
| `/dashboard/sessions/<id>` | Session detail + Resume task |
| `/dashboard/reliability/drift` | Drift events + approve/reject |
| `/dashboard/assembler` | Assembler playground (query + budget → drops + why + expand) |
| `/dashboard/savings` | Savings & recover dashboard |

### 4.2 Page upgrades

| Route | Upgrade |
|---|---|
| `/dashboard/memory` | Add edit/delete/pin/promote, contradiction hints |
| `/dashboard/memory/wakeup` | Add confidence bars, source links, related memory hints |
| `/dashboard/memory/recall` | Add hybrid search, confidence sort, graph link |
| `/dashboard/settings` | Add profile shortcut, token ledger, setup rerun |
| `/dashboard/context/assemble` | Add token budget slider, show-drops-why panel, expand buttons |
| `/dashboard/reliability` | Link to drift center |
| `/dashboard/reliability/anchor` | Suggest from sessions |
| `/dashboard/reliability/checkpoints` | Show session association |
| `/dashboard/devices/audit` | Move to HelixDB-backed, real-time via SSE |

### 4.3 Component additions

- `MemoryGraph` — react-force-graph-2d wrapper
- `MemoryCard` — editable memory card with confidence bar
- `ConfidenceBadge` — visual indicator
- `SessionCard` — task/finding/decision preview
- `AssemblerOutput` — drop-reason panel
- `SavingsChart` — recharts / shadcn chart component
- `DriftEventRow` — approve/reject actions
- `SetupWizard` v2 — embed provider, device pair, health check

### 4.4 Real-time plumbing

- `web/src/lib/sse.ts` — EventSource wrapper with reconnect
- Hook: `useEventSource()`
- React Query integration: invalidate queries on events
- Toast notifications for `audit_event`, `drift_detected`, `memory_added`

---

## §5. Execution Phases

### Phase 3.5 — Dashboard Depth (7 sprints)

**Sprint 1 — Foundation: audit to HelixDB + SSE + metrics**
- Move audit log from in-memory ring buffer to HelixDB (`crates/cairn-api` audit store)
- Add SSE endpoint `/api/events`
- Add `/api/metrics` (tokens saved, hit rate, bounce rate)
- Replace 5s polling on Overview/Audit with SSE
- **Testing:**
  - Unit test: audit events round-trip through HelixDB
  - Integration test: SSE event reaches dashboard <500ms latency
  - Test metrics endpoint returns non-empty JSON after a tool call

**Sprint 2 — Memory CRUD + Confidence + Profile**
- Add `confidence: f32` to Memory schema; default 0.5 for existing
- Implement reinforcement on `recall` hit
- Add memory edit/delete/pin mutations
- Add `/dashboard/profile` page
- **Testing:**
  - Unit test: confidence reinforcement curve matches agentmemory formula
  - Integration test: memory edit/delete reflected in Wakeup within one event
  - UI test: profile page renders active preferences

**Sprint 3 — Memory Graph + Crystallize + Edges**
- Add `derived_from`, `contradicts`, `supersedes`, `applies_to` fields
- Store edges in HelixDB graph
- Add `cairn memory crystallize` CLI + MCP tool
- Build `/dashboard/memory/graph` with react-force-graph-2d
- **Testing:**
  - Unit test: edge insert/query via HelixDB graph API
  - Integration test: crystallize promotes working memories to semantic tier
  - UI test: 50-node graph renders <1s in headless browser

**Sprint 4 — Sessions + Reliability Center**
- Add `cairn-session` crate + CCP
- Add `/api/sessions` + session detail + resume
- Build `/dashboard/sessions` and `/dashboard/sessions/<id>`
- Persist `verify` results to HelixDB drift log
- Build `/dashboard/reliability/drift` approve/reject UI
- **Testing:**
  - Unit test: CCP serialization round-trip
  - Integration test: session auto-restore on new chat injects CCP block
  - End-to-end test: approve/reject drift event updates reliability score

**Sprint 5 — Assembler Playground + Savings Dashboard**
- Upgrade `/dashboard/context/assemble` with budget slider
- Show drops + reasons + expand buttons
- Add `/api/ledger` + signed entries
- Build `/dashboard/savings`
- **Testing:**
  - Unit test: ledger entry HMAC verifies offline
  - Integration test: assembler reports in/dropped correctly
  - UI test: savings chart updates after a cached read

**Sprint 6 — Setup Wizard v2 + Landing Site**
- Add embed provider picker to setup form
- Add device-pair step + QR code
- Green-health check on completion
- Rewrite landing page (`/`) with hero + install commands + comparison
- **Testing:**
  - End-to-end test: first-run setup completes without manual `.env` editing
  - UI test: landing page renders hero, install commands, comparison table
  - Test default embed provider is local hashing

**Sprint 7 — Hybrid Search RRF + Rerank + CSP**
- Finish graph leg of hybrid search
- Add rerank + MMR diversity
- Add `/api/search` + `cairn search`
- Implement nonce-based CSP for prebuilt assets
- **Testing:**
  - Benchmark test: RRF recall beats BM25-only on LongMemEval fixture
  - Unit test: rerank scores sort correctly
  - Security test: CSP nonce present on all script tags

### Phase 4.0 — CLI + MCP Depth (5 sprints)

**Sprint 8 — One-Command Install**
- `cairn onboard` (zero-prompt)
- `cairn doctor --fix`
- `install.sh` (Linux/macOS) + `install.ps1` (Windows)
- `cairn update` against release binaries
- **Testing:**
  - CI job runs `install.sh` in fresh Ubuntu container
  - CI job runs `install.ps1` in Windows runner
  - `cairn doctor --fix` smoke test repairs a broken `.mcp.json`

**Sprint 9 — CLI Surface Expansion**
- `cairn graph`, `cairn impact`, `cairn callgraph`
- `cairn memory timeline`, `cairn memory crystallize`
- `cairn session`, `cairn sessions`
- `cairn metrics`
- **Testing:**
  - Each new command has `--help` and a smoke test
  - `cairn graph related` returns nodes linked to a file
  - `cairn callgraph callers` resolves a symbol from the codebase

**Sprint 10 — MCP Surface Expansion**
- Expand MCP tools 16 → 40+
- Add 5 MCP resources
- Add 5 MCP prompts
- Add MCP elicitation (pressure-gated)
- **Testing:**
  - MCP `tools/list` returns ≥40 tools
  - Each new tool has a round-trip integration test
  - `cairn://memory/graph` resource returns valid graph JSON

**Sprint 11 — Context Package System**
- `.cairnpkg` format (adopt lean-ctx `.ctxpkg` design)
- 9 CLI subcommands
- Registry backend `/api/pack/*`
- Multi-platform release binaries + SHA256SUMS
- **Testing:**
  - Round-trip: create → export → import preserves graph and signatures
  - `.ctxpkg` import alias loads correctly
  - Registry search returns published pack metadata

**Sprint 12 — Distribution Polish**
- Homebrew tap (`Vellixia/homebrew-tap`)
- Fly/Railway/Render one-click templates
- Non-root Docker volume init
- OpenCode README quickstart section
- **Testing:**
  - `brew install cairn` formula test in CI
  - Docker compose `up` with non-root volume succeeds
  - README install flow verified by fresh user (manual QA)

### Phase 4.1 — Collective + Federation (3 sprints)

**Sprint 13 — Pack Registry**
- Self-hosted registry endpoints
- Web UI: Collective / Federation manager
- Pack signing (Ed25519)
- **Testing:**
  - End-to-end: publish + search + install works via web UI
  - Signature verification rejects tampered pack
  - Federation manager lists installed packs with revocation status

**Sprint 14 — Federation Protocol**
- Trust scopes + signed packs
- Revocation propagation (`unshare` cascades)
- Provenance display
- **Testing:**
  - Integration test: subscriber receives revocation within 60s
  - Unit test: provenance chain validates across 3 pack hops
  - Security test: untrusted scope cannot install into trusted scope

**Sprint 15 — Privacy Hardening**
- Offline-first sync via automerge CRDT (replace LWW)
- Optional E2E encryption for sync
- Updated SECURITY.md + threat model
- **Testing:**
  - Integration test: sync works offline then reconciles on reconnect
  - Unit test: CRDT merge resolves concurrent preference edits
  - Threat model review sign-off

### Phase 4.2 — Benchmarks + Adoption (2 sprints)

> ✅ **Status (v0.5.0):** Both sprints shipped. `cairn-bench` crate holds the
> LongMemEval/horizon/retention benchmarks (`455c34b`); the public landing page
> lives at `web/src/app/page.tsx` with token savings, comparison table, install
> commands, and a demo placeholder.

**Sprint 16 — Benchmarks**
- LongMemEval / LoCoMo recall scores
- Task-success horizon benchmark
- Cairn-specific "smart memory retention" benchmark
- Publish in `docs/BENCHMARKS.md`
- **Testing:**
  - Benchmarks run in CI with locked dependencies
  - LongMemEval fixture score recorded
  - Results reproducible across 3 reruns (variance <5%)

**Sprint 17 — Marketing + Adoption**
- Final landing site with benchmarks
- Demo video / GIF
- Comparison table (honest)
- Docs polish cross-links
- **Testing:**
  - README install flow verified by fresh user (manual QA)
  - Landing page Lighthouse performance score ≥ 80
  - All docs cross-links validated by link checker

### Phase 5 — Proactive, Service & Cross-Platform (committed)

> ✅ **Status (v0.5.0):** Sprints 18, 19, 20, 22 shipped. Sprints 21 (browser
> extension) and 23 (mobile companion) deferred to v0.6.0 — both are
> JavaScript-only artefacts that ship independently of the Rust binary, so the
> v0.5.0 release is unblocked. See `docs/ROADMAP.md` for the verification
> rows.

Phase 5 items are no longer "future ideas" — they are committed v0.5.0 sprints.

**Sprint 18 — Proactive Recall**
- Intent detection hook in `cairn-mcp` before each agent turn
- Auto-inject up to 3 relevant memories when intent is detected
- User preference to disable per-project
- **Testing:**
  - Unit test intent detector on 50 synthetic prompts
  - Integration test: memory injected before agent turn when intent matches
  - Test opt-out: disabled project receives no proactive recall

**Sprint 19 — Cairn-as-a-Service (`cairn.sh`)**
- Multi-tenant mode in `cairn-server` (organization isolation)
- Embedded pack registry exposed at `/registry`
- Optional public `cairn.sh` reverse proxy to self-hosted registries
- **Testing:**
  - Integration test tenant isolation (project A cannot read project B)
  - Registry publish/search/install round-trip via `/registry`
  - Load test: 100 concurrent pack searches

**Sprint 20 — PWA + Push Notifications**
- Service worker for offline dashboard shell
- Push subscription endpoint `/api/push/subscribe`
- Push drift/revocation notifications to subscribed devices
- **Testing:**
  - Lighthouse PWA audit score ≥ 90
  - End-to-end push delivery for drift event
  - Offline dashboard renders cached shell

**Sprint 21 — Browser Extension**
- Manifest V3 extension: capture page text + selection as memory
- Native messaging to `cairn-server` (or HTTP to local server)
- "Add to Cairn" context menu
- **Testing:**
  - Unit test content extraction preserves source URL
  - Integration test: captured memory appears in `/api/memory` within 5s
  - Cross-browser smoke test (Chrome + Firefox)

**Sprint 22 — Voice / Transcript Ingestion**
- `/api/ingest/transcript` endpoint accepting VTT/SRT/JSON
- Chunk transcripts by speaker + timestamp; summarize to memories
- CLI: `cairn ingest transcript <file>`
- **Testing:**
  - Unit test transcript chunking boundaries
  - Integration test: 10-minute transcript produces ≥3 memories
  - Test memory source link points to timestamp

**Sprint 23 — Mobile Companion App**
- Capacitor or PWA-standalone app for approvals and quick stats
- Approve/reject drift, view pack installs, see savings card
- Biometric lock option
- **Testing:**
  - End-to-end drift approval from mobile viewport
  - Savings card reflects live metrics
  - Authentication session survives app background

---

## §6. Design Decisions

The following decisions are locked for v0.5.0:

| Decision | Resolution | Rationale |
|---|---|---|
| **Confidence field adopts agentmemory's reinforcement curve** | Locked | Proven, easy to implement, intuitive |
| **Memory graph edges use HelixDB native graph** | Locked | Graph-native queries, no secondary store |
| **Cross-Session Protocol adopts lean-ctx pattern** | Locked | Battle-tested, compact (~400 tokens), auto-restore works |
| **Context Package System adopts lean-ctx `.ctxpkg` design** | `.cairnpkg` canonical; `.ctxpkg` accepted as import alias | Free interop/ecosystem trust; distinct Cairn identity |
| **SSE instead of WebSocket for dashboard** | Locked | One-way is enough; simpler; native EventSource |
| **Memory Graph Visualization uses `react-force-graph-2d`** | Locked | Clean API, d3-force, works with Next.js static export |
| **Setup wizard v2 includes embed provider picker** | Locked; default = local hashing first | PLAN.md requirement; avoids post-install `.env` edits |
| **Audit log moves to HelixDB** | Locked | Currently lost on restart; trust-critical |
| **Savings ledger is signed** | Locked | Proves no-loss compression claim |
| **Memory graph exposed as MCP resource** | `cairn://memory/graph` plus tools | Natural complement to graph-native store |
| **Pack registry host** | Embedded in `cairn-server` via `/registry` routes | Self-hostable first; public `cairn.sh` can proxy later |
| **Phase 5 "Beyond" items** | Promoted to committed sprints in the phase map below | User wants all Phase 5 features committed to v0.5.0 |

---

## §7. Success Metrics

At the end of v0.5.0:

- **≥40 MCP tools** exposed (up from 16)
- **6 MCP resources + 5 MCP prompts**
- **≤500ms** SSE event latency
- **Memory Graph renders 50 nodes in <1s**
- **Confidence reinforcement visible in UI**
- **Drift events reviewable + approvable**
- **One-command install** works on Linux/macOS/Windows
- **Homebrew + one-click deploy** available
- **LongMemEval/LoCoMo scores published** honestly
- **No in-memory-only state** (audit, sessions, drift, ledger all durable)

---

## §8. Risks

| Risk | Mitigation |
|---|---|
| Schema migration on live HelixDB | Default `confidence = 0.5`; additive fields only; no rewrites |
| Graphviz library bloat | react-force-graph-2d is ~50KB; lazy-load on `/dashboard/memory/graph` |
| MCP tool explosion confusing users | Use dynamic tool categories (core / arch / memory / metrics / session / pack) |
| Package format interop disputes | Credit lean-ctx lineage; publish spec openly |
| Benchmark numbers not competitive | Publish methodology and honest deltas; iterate |
| Feature scope creep | Phase 5 items are now committed sprints with dedicated testing; keep scope fixed to the 23 sprints above |

---

## §9. See Also

- [PLAN.md](PLAN.md) — original v0.4.0 vision
- [ROADMAP.md](ROADMAP.md) — what is done, what is next
- [ARCHITECTURE.md](ARCHITECTURE.md) — current crate graph and API surface
- [WEB.md](WEB.md) — v0.4.0 dashboard surface (will be updated for v0.5.0)
- [BENCHMARKS.md](BENCHMARKS.md) — measured numbers + targets
- [DECISIONS.md](DECISIONS.md) — ADRs for the v0.5.0 decisions will be added here (see planned ADR-010 through ADR-016 below)

### §9.1 Planned v0.5.0 ADRs

| ADR | Decision |
|---|---|
| ADR-010 | Confidence field and reinforcement curve (agentmemory adoption) |
| ADR-011 | Memory provenance graph stored in HelixDB native graph |
| ADR-012 | Cross-Session Protocol (lean-ctx CCP adoption) |
| ADR-013 | Context package format: `.cairnpkg` canonical, `.ctxpkg` import alias |
| ADR-014 | SSE real-time dashboard vs WebSocket |
| ADR-015 | Default embedding provider: local hashing first |
| ADR-016 | Pack registry embedded in `cairn-server` for self-hosting |

---

## §10. Resolved Open Questions

The following questions were answered before implementation begins:

1. **`.cairnpkg` vs `.ctxpkg`** — `.cairnpkg` is the canonical extension; `.ctxpkg` files are
   accepted as an import alias for interoperability with lean-ctx.
2. **Phase 5 items** — promoted to committed roadmap and mapped into the sprints below.
3. **Default embed provider** — local hashing first (offline-first), with optional local ONNX and
   OpenAI-compatible endpoints selectable in the setup wizard.
4. **Memory graph visibility** — exposed both in the web dashboard and as the MCP resource
   `cairn://memory/graph`, plus `memory_graph` and related graph tools.
5. **`cairn.sh` registry** — the v0.5.0 registry is embedded in `cairn-server` for self-hosting.
   A public `cairn.sh` proxy can be added later without changing the protocol.

These resolutions will be captured as ADRs in `docs/DECISIONS.md`.

---

*This plan is a living document. As implementation starts, each sprint will produce or update
SPEC.md tasks, and the resolutions above will be mirrored into DECISIONS.md ADRs.*
