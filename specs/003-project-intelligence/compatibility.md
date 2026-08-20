# Compatibility Matrix: Cairn Project Intelligence

**Feature**: `003-project-intelligence` | **Baseline**: `main` @ `0b79b31` (v0.1.0-alpha.4)

What Feature 003 touches, what it leaves alone, and where it extends an existing contract. An
extension appears here only if it is **additive and backward compatible**; there are no breaking
changes.

## Feature 001 — Cairn MVP

| Contract | Status | Notes |
|---|---|---|
| Memory types (`fact`, `decision`, `convention`, `failure`, `procedure`) | **unchanged** | No type added |
| Memory scopes (`project`, `branch`, `task`, `session`) | **unchanged** | No scope added. Subjects are keyed *by* scope, narrowing rather than widening |
| Memory lifecycle states (`active`, `stale`, `superseded`) | **unchanged** | Verification is a separate, orthogonal axis (FR-362) |
| `superseded_by_id` link | **extended, compatible** | Becomes the view of a `supersedes` relation; every existing reader sees the same value (D47, FR-324) |
| Provenance: mandatory origin session, zero-or-more observations | **unchanged** | Evidence facts are additional, never a replacement (D51) |
| `memory_evidence` table | **unchanged** | Not modified in any way |
| `local_only` | **unchanged** | Extended in reach: nothing Feature 003 derives from a `local_only` memory leaves either (FR-504) |
| Deleted-evidence provenance ("evidence deleted") | **extended, compatible** | Same semantics applied to every new reference type (FR-505) |
| FTS5/BM25 lexical retrieval | **unchanged** | Same table, same triggers, same ranking. `topic_key` is an SQL filter, not an FTS column (B7) |
| Scope-first ranking (task 0, branch 1, project 2, session 3) | **unchanged** | `MemoryScope::bucket` untouched. Verification and importance order *within* a bucket only (FR-308) |
| `MemoryQuery` fields and defaults | **extended, compatible** | New optional filters (`verification`, `authority`, `corroborated`, `conflicted`, `topic_key`, `as_of`, `pinned`, `include_patterns`); `state` still defaults to `active` |
| Context budget in Cairn-estimated tokens | **unchanged** | Default still 3000 (FR-442) |
| `estimated_tokens <= budget` | **unchanged** | `try_spend` measure-before-emit preserved (I16, FR-445) |
| `truncated`, `omitted_sections`, `degraded` | **unchanged** | Still always present; `degraded` gains one new cause (subject scan cap) |
| High-priority sections (`task`, `repository`, `previous_handoff`) | **unchanged** | Now also inside Level 0, which strengthens the guarantee (FR-447) |
| Briefing section order | **extended, compatible** | The eight sections keep their relative order inside Levels 0 and 1 |
| Handoff triggers (`pre_compact`, `session_end`, `recovered`); no `stop` | **unchanged** | Feature 003 adds no trigger |
| Handoff derived-fields rule (only `agent_note` is narrative) | **unchanged** | The checkpoint anchors to the handoff rather than re-deriving (D55) |
| Session identity by `agent_session_key`, never worktree | **unchanged** | |
| Session states (`active`, `completed`, `interrupted`) | **unchanged** | |
| Sealed session close, `handoff_pending`, the maintenance sweep | **extended, compatible** | The checkpoint is written in the same synthesis step, so the existing sweep covers both |
| Task fields and `TaskStatus` | **unchanged** | A **local, unsynced** counter is added; status transitions still unrestricted and never set by Cairn (FR-487). The server's `tasks` table is untouched (D80) |
| `tasks.acceptance_criteria` as an array of strings | **retained** | Kept as a synchronized projection of `task_criteria` (D68, FR-492) |
| `cairn task update --acceptance-criteria` whole-list form | **unchanged in behaviour** | Diffs by text, preserving ids for unchanged entries |
| Six MCP tools | **unchanged** | Exactly six; extended by action and parameter (FR-495) |
| MCP error-code set; no `budget_exceeded` | **extended, compatible** | New codes added to the one stable set |
| `--json` envelope shape | **unchanged** | New fields are additive |
| Outbox + idempotency key + server `sync_state` claim | **unchanged** | Three entity types added and one state (`blocked`); the key, the claim, the stale-claim timeout and the drainers are untouched (FR-414, D81) |
| Outbox states | **extended, compatible** | `blocked` added. Existing `pending`/`in_flight`/`delivered`/`failed` rows keep their meaning; nothing becomes `blocked` by migration |
| Permanent-rejection handling | **corrected** | A **content** rejection stays permanently `failed`, exactly as today. A **capability** rejection now becomes `blocked` and recovers, where before it was stranded as `failed` forever (research, D81) |
| No observation entity type in the outbox | **unchanged, and extended** | The existing test is extended to cover every Feature 003 local-only record |
| Server has no observations table | **unchanged** | And gains no evidence, verification, checkpoint or pattern table |
| Wire field allowlist | **extended** | 16 forbidden field names and 6 forbidden entity types added (FR-506) |
| `import_memory` `INSERT OR IGNORE` | **unchanged, and completed** | Relations now import separately and local state re-derives — which fixes the pre-existing gap where a remote supersession never landed (B2, R5) |
| Redaction before write | **unchanged** | Every new text field passes through it |
| Privacy exclusions (`excluded_paths`, `excluded_commands`) | **unchanged, and honoured** | No evidence is created for an excluded path; the memory reports `evidence_excluded` |
| Capture deadline 250 ms, always exit 0, fail soft | **unchanged** | Drift marking is capped at 8 indexed lookups and defers rather than delaying (FR-475) |
| Context deadline 1500 ms, bounded fallback | **unchanged** | No verification runs at session open (FR-471) |
| `BEGIN IMMEDIATE` + bounded retry | **unchanged** | No second concurrency mechanism (FR-414) |
| Schema-version guard | **unchanged** | Local 4 → 5, server 1 → 2 |
| `mark_stale_scopes` on the maintenance tick | **unchanged** | Branch deletion still marks stale, never deletes (FR-383) |
| No embeddings, vector store or knowledge graph (FR-025) | **unchanged** | Reaffirmed and re-asserted by test (SC-321) |

## Feature 002 — Agent Integration Platform

| Contract | Status | Notes |
|---|---|---|
| Seven canonical lifecycle events | **unchanged** | Feature 003 adds none (FR-427; Feature 002 FR-113) |
| `agent_quiesced` semantics (checkpoint only, no handoff) | **unchanged** | No checkpoint is written at a turn boundary |
| `context_compacting` → durable handoff | **extended, compatible** | Also writes a continuity checkpoint anchored to that handoff |
| `context_compacted` → optional context re-delivery | **extended, compatible** | Where present, it restores the checkpoint. Where absent, `continuity_mode = agent_initiated` |
| `session_closed` sealed close | **extended, compatible** | The checkpoint is written in the same synthesis step |
| 14 capabilities; availability × confidence | **unchanged** | No capability added. `continuity_mode` is a *derived read* over two existing ones (D57) |
| Integration levels FULL / MCP_PLUS / MCP_ONLY / UNSUPPORTED | **unchanged** | Feature 003 does not affect level derivation |
| Honest-degradation rule (never claim what is only expected) | **unchanged, and applied** | `continuity_mode` never claims a rehydration guarantee an adapter cannot provide (FR-426, US6 #3/#4) |
| Adapter trait and the four adapters | **unchanged** | No adapter change is required by this feature |
| Vendor payload allowlist | **unchanged** | No new vendor field is read |
| `classify_tool`, `is_test_command`, `vendor_tool` provenance | **unchanged, and used** | `is_test_command` is what makes a captured `test_run` observation usable as Cairn-collected evidence (D52) |
| Cross-agent memory invariant (agent identity is provenance only) | **unchanged** | No scope, partition or filter keyed to an agent (FR-381, SC-327) |
| Agent usage contract: one canonical source, two renderings, size-bounded | **extended, compatible** | Four obligations added within the existing bound (FR-498) |
| Cairn Skill | **extended, compatible** | Same additions; the revision digest changes as it does for any content change |
| MCP `instructions` at initialize | **extended, compatible** | Generated from the same updated contract |
| MCP protocol version `2025-06-18` | **unchanged** | No protocol change |
| Local integration record never syncs | **unchanged** | And Feature 003 adds nothing to it |
| Configuration operations fail loudly; capture fails soft | **unchanged, and followed** | Promotion is configuration-class and fails loudly; drift marking is capture-class and fails soft |
| `capability_evidence` table | **unchanged** | Feature 003's evidence is a separate concept in separate tables; the naming echo is deliberate but they do not interact |

## Continuity mode by agent

Derived, not maintained (D57). These are the current outputs of the rule:

| Agent | Pre-compaction | Post-compaction | `continuity_mode` | What Cairn tells the developer |
|---|---|---|---|---|
| Claude Code | `PreCompact` | `PostCompact` (conditional) | `agent_initiated` | A checkpoint is written before compaction; call `cairn_context(reason=post_compaction)` to restore it |
| Codex | `PreCompact` | `PostCompact` (conditional) | `agent_initiated` | A checkpoint is written before compaction; call `cairn_context(reason=post_compaction)` to restore it |
| OpenCode | `experimental.session.compacting` (conditional) | `session.compacted` | `agent_initiated` | A checkpoint is written before compaction; call `cairn_context(reason=post_compaction)` to restore it |
| Generic MCP | none | none | `unavailable_automatic` | Not automatic for this client; a checkpoint exists at session close and via `cairn_session action=checkpoint` |

OpenCode is `agent_initiated` rather than `automatic` because its compaction hook is an experimental
capability Feature 002 already reports as conditional — a conditional capability does not establish an
automatic guarantee (Feature 002 FR-241).

## Existing data compatibility

| Existing state | Behaviour after upgrade |
|---|---|
| Memories with no topic key | Free-form; searchable and briefable exactly as before; never auto-reconciled (FR-313, US1 #3) |
| Memories with no evidence | `verification = unverified`; valid and unchanged (FR-356) |
| `stale` and `superseded` memories | Untouched. Supersessions become relations so they are visible to the derivation and to sync |
| Existing `superseded_by_id` chains ≥3 deep | Preserved; `rebuild_supersession` reproduces them exactly |
| `local_only` memories | Still never transmitted; still produce no outbox row |
| `memory_evidence` rows whose observation was deleted | Still resolve to "evidence deleted" |
| Sessions, handoffs, observations | Untouched |
| Sessions with `handoff_pending = 1` | The existing sweep still completes them; a checkpoint is written with the handoff |
| Sessions bound to a task before the upgrade | `task_snapshot_at_bind` is NULL — honestly unknown — and no divergence is reported for them |
| Tasks with empty `acceptance_criteria` | No criteria rows; readiness is `not_ready`; nothing breaks |
| Tasks with duplicate criterion strings | Distinct criteria with distinct ids; not merged |
| Pending, in-flight and failed outbox rows | Untouched and still deliverable |
| A `pull_cursor` mid-stream | Untouched; the next pull continues from it |
| Shared server data | Converges as memories and relations arrive; no server backfill |
| Feature 002 integration tables | Untouched |

## Surfaces that change

| Surface | Change |
|---|---|
| `cairn memory add` | `--topic-key`, `--value-key`, `--importance` |
| `cairn memory search` | `--verification`, `--conflicted`, `--topic-key`, `--as-of`, `--pinned`, `--include-patterns` |
| `cairn memory subject <key>` | **new** — members, canonical answer, reconciliation state, decisions |
| `cairn memory pin <id> [--off] [--reason …]` | **new** |
| `cairn memory reconcile …` | **new** |
| `cairn evidence add / list / show` | **new** |
| `cairn verify [--memory \| --task \| --all] [--explain]` | **new** |
| `cairn pattern list / show / promote / outcome / forget` | **new** |
| `cairn context [--explain] [--depth …]` | extended |
| `cairn session checkpoint` | **new** |
| `cairn task get` | now shows the local counter, the cross-device state digest, criteria with ids/states/verification/authority, blockers, progress, readiness |
| `cairn task criterion add/set/verify/remove` | **new** |
| `cairn task blocker open/clear` | **new** |
| `cairn task readiness / history` | **new** |
| `cairn status` | adds the share of project memories carrying a subject (FR-499), and any sync degradation including the blocked count |
| `cairn doctor` | adds `continuity_mode` per agent, and `--rebuild-derived` |
| `cairn sync status` | adds the degradation line when a server predates the feature |
| Web UI — memory list | verification state, conflict marker, subject; read-only |
| Web UI — task page | criteria states and derived progress; read-only |
| Web API `/api/projects/{id}/memories` | adds `verification` and `subject` |
| `GET /api/sync/changes` | adds optional `relations`, `criteria`, `blockers` arrays |
| `POST /api/sync/batch` | accepts three new entity types; rejects six by name |
| `GET /api/version` | adds `schema_version` and a `capabilities` array — additive, unauthenticated, and the probe that unblocks retained work (D81). An older server returns neither, and that absence is the answer |

The web UI stays read-only for Feature 003 state. Editing knowledge from a browser would be a second
write path into canonical knowledge, which the proposal boundary exists to prevent.

## Open notes — MEDIUM and LOW, non-blocking

Recorded so they are not rediscovered. None is a correctness, privacy, data-loss or corruption risk.

1. **(MEDIUM) `CHECK` constraints on `memories` are enforced in code, not in DDL.** SQLite cannot add
   a `CHECK` to an existing table without rebuilding it, which would rewrite every row and violate
   FR-513's literal reading. The predicates are enforced at the repository boundary and asserted by
   test. A future migration that rebuilds `memories` for another reason should add them. New tables
   carry theirs in DDL.

2. **(MEDIUM) `superseded_at` for pre-existing supersessions is approximated from `updated_at`.**
   Documented in [migration.md](./migration.md) §Step 2(b). Affects only historical `as_of` placement
   of memories superseded before this feature existed.

3. **(MEDIUM) `sessions.task_snapshot_at_bind` is NULL for pre-upgrade sessions,** so no divergence is
   reported for them. This is honest — the state they bound at is unknowable — and it self-corrects as
   sessions turn over.

4. **(LOW) Reconciliation depends on agents supplying topic keys, and now on their being specific
   enough.** Because a shared value key no longer merges, a coarse key costs deduplication rather than
   correctness — the failure mode moved from *wrong* to *less useful*, which is the right direction.
   Mitigated on three surfaces, measured by the adoption metric in `cairn status` (FR-499), and observed
   per agent by the non-gating effectiveness evaluation
   ([contracts/evaluation.md](./contracts/evaluation.md) §Topic-key effectiveness).

4a. **(LOW) `stale_at` is NULL for memories that went stale before the upgrade,** so their historical
   applicability is reported as unknown. Deliberate: no authoritative instant exists and inferring one
   would be a second approximation (D82).

4b. **(LOW) A `size`-class path fingerprint cannot see a same-length edit.** It applies only to files
   over the payload cap, where source edits are rare, and the class is reported so the weaker comparison
   is visible rather than implied (D79).

5. **(LOW) Pattern suggestion matches signals lexically.** A pattern whose signals are worded
   differently from the receiving project's error text will not surface. Accepted: a missed suggestion
   costs nothing, a false one costs trust. `pattern_signals_min` and the paired corpus bound the
   false-positive side.

6. **(LOW) The web UI does not surface patterns, evidence or checkpoints.** They are local-only, and
   the shared server holds none of them, so there is nothing for a browser to show. If a future
   feature shares patterns, the UI follows then.

7. **(LOW) `verification_basis` on the server is a list of verifier kinds with no ordering
   guarantee.** It is displayed as a set. Harmless, noted so no one reads meaning into the order.

8. **(LOW) Attested evidence never re-verifies on its own.** A recheck of attested evidence yields
   `needs_recheck` until the agent attests again, so an abandoned attested claim decays to
   `needs_recheck` and stays there. That is the intended honest resting state, not a stuck one.
