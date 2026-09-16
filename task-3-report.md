# Task 3 phase A — edge inventory

Scope inspected: `crates/cairn-store`, `crates/cairnd`; head `9658c55` plus
this phase's Task-cleanup repair. This is an inventory only. No broad edge
deletion started because current daemon handlers, recovery, and migration paths
still compile against the local-authority modules below.

## Retain boundary

| Required edge concern | Current owner/evidence |
| --- | --- |
| typed durable capture/command spools, receipts, retry state | `cairn-store/src/spool.rs`: `spool_event`, `claim_events`, `mark_event_*`, `spool_command`, `claim_commands`, `mark_command_*`, `release_*_claims`; schema `0008_safe_events.sql` |
| server/account/project binding | `cairnd/src/state.rs`: `ServerCredentials::{load,mutate_credentials,account_identity}`; `cairn-store/src/repo.rs`: project link fields; `cairnd/src/sync.rs`: `set_token`, `link` |
| bounded returned-context cache | `cairnd/src/deliver.rs`: `OutageCache::{put,get,clear}`; it is in-memory, account-bound, LRU-capped |
| hook/session correlation | `cairnd/src/arrival.rs`: `Arrivals`, `Ticket`; `cairn-store/src/spool.rs`: `session_event_seq`, `command_seq`; `cairnd/src/capture.rs` |
| integration ownership | `cairn-store/src/integrations.rs`: `upsert_agent`, `bind`, `unbind`, `bound_resources`; schema `0004_integrations.sql` |
| offline Task migration manifest | `cairn-store/src/transfer.rs`: export/cleanup/import; `removed_feature_manifest`; schema `0013`/`0014` |

## Delete candidates — local canonical knowledge, search, ranking, replicas

| Module | Exact surviving implementation |
| --- | --- |
| `cairn-store/src/repo.rs` | local project/session/observation/memory/handoff CRUD: `create_memory`, `create_memory_reconciled`, `memory`, `delete_memory`, `import_memory`, `list_sessions`, `latest_handoff`, `start_session`, `end_session`; local sync/deferred state: `last_sync_success`, `record_sync_success`, `pull_cursor`, `set_pull_cursor`, `defer_pulled_record`, `deferred_records`, `clear_deferred_record`, `note_deferred_attempt` |
| `cairn-store/src/search.rs` | local FTS/ranking/retrieval: `search`, `one`, `build_result`, `recency_contribution`, `memory_for_scope`, `search_personal`, `search_team`, `list_team`, `fts_query` |
| `cairn-store/src/knowledge.rs` | local relation, conflict, supersession, reinforcement construction; called by briefing/sync/handlers |
| `cairn-store/src/evidence.rs` | local evidence collection, verification-run write/read, local evidence views |
| `cairn-store/src/patterns.rs` | local reusable pattern and application store/recall |
| `cairn-store/src/global.rs` | personal/team replicas: `merge_synced_personal`, `merge_synced_team`, `merge_synced_pattern`, `recall_personal`, `recall_team`, writer-gap/adoption functions |
| `cairnd/src/briefing.rs` | local durable assembly: `build`, `global_candidates`, `level1_patterns`, `scope_memory`, `latest_handoff_for_branch`, `level0_warnings`, `level0_pins` |
| `cairnd/src/handlers.rs` | local memory/search/personal/team/pattern CRUD and views; main calls at lines 3085, 3183–3573, 3806–3821 |
| `cairnd/src/patterns.rs`, `promote.rs`, `verify.rs`, `drift.rs`, `continuity.rs`, `handoffs.rs`, `recover.rs` | local canonical promotion, verification, drift, checkpoint/handoff generation and recovery; all require replacement with server operations before deletion |

Schema candidates: `memories`, `memory_evidence`, `memory_evidence_facts`,
`memory_relations`, `evidence_facts`, `verification_runs`, `reusable_patterns`,
`pattern_applications`, FTS tables/triggers (`0002`, `0007`), personal/team
knowledge and applicability/relation tables (`0007`), observations, handoffs,
continuity checkpoints, cached patterns (`0009`).

## Delete candidates — replicas, cursors, namespace/authority, entity outbox

| Module | Exact surviving implementation |
| --- | --- |
| `cairn-store/src/outbox.rs` | entity outbox and retry workers: `enqueue`, `enqueue_global`, `claim`, `claim_excluding`, `claim_namespace*`, `mark_delivered`, `mark_failed`, `mark_retryable`, `mark_blocked`, `release_all_claims`, `rename_namespace`, namespace counters/payload builders |
| `cairn-store/src/cursor.rs` | namespace establishment, pull cursor, visibility, retry/capability state |
| `cairn-store/src/authority.rs` | local authority-mode state and transitions |
| `cairn-store/src/migrate.rs` | legacy authority/cutover runtime: `Phase`, `phase_*`, retained-local and pattern-claim APIs (keep only schema migration runner) |
| `cairnd/src/sync.rs` | namespace worker `run_worker`; entity drains `drain`, `drain_global`; establishment/linking `link`; pull/apply paths around `pull`, `apply_*`, deferred replay; account/member/admin proxy functions `auth_*`, `change_password`, `admin_user_*`, `project_member_*`; legacy/backfill/cutover helpers around `legacy_row_eligibility`, `pattern_eligibility` |
| `cairnd/src/migrate005.rs` | complete legacy migration orchestration: `inspect`, `claim_patterns`, `normalize_keys`, `drain`, `verify_possession`, `switch_authority`, `demote`, `run`, `status`, `retry_retained` |

Schema candidates: `outbox`, `sync_meta`, `sync_deferred`, `sync_cursor`,
`authority_mode`, `migration_state`, `retained_local`, `legacy_pattern_claims`,
writer identity/version columns, `cached_patterns`, and local global-memory
tables. Typed `event_spool`/`command_spool` must not be merged into `outbox`.

## Compiler-safe first seam

Do not drop schema/modules next. First route all durable context/search and
knowledge mutations in `cairnd/src/handlers.rs` and `cairnd/src/briefing.rs`
to one server client boundary, retaining only `deliver::OutageCache` fallback.
Then remove their local callers and tests, which isolates `search`, `repo`,
`knowledge`, `global`, `patterns`, and FTS as a compiler-visible deletion
slice. Separately replace `sync::run_worker` with one typed-spool delivery loop
before deleting `outbox`, `cursor`, `authority`, and `migrate005`; capture and
command spool semantics remain distinct.
