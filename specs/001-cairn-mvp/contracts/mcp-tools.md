# Contract: MCP Tool Surface

**Feature**: `001-cairn-mvp` | Six tools, no more (FR-040). Each tool takes an `action`
discriminator rather than expanding into one tool per operation.

Served by `cairn mcp` over stdio. Every tool forwards to the local daemon and returns
JSON. Every response carries `ok: boolean`; on failure it carries `error: {code,
message}` and the agent is expected to continue working.

Common error codes: `not_a_repository`, `no_active_session`, `ambiguous_session`,
`not_found`, `invalid_request`, `storage_unavailable`. There is no `budget_exceeded` — a
briefing is truncated to fit, never rejected for size (FR-029).

---

## `cairn_context`

Build the briefing for the current working context.

**Input**

| Field | Type | Required | Notes |
|---|---|---|---|
| `cwd` | string | yes | Directory to resolve the repository from |
| `reason` | enum | no | `session_start` \| `continuation` \| `refresh` (default `refresh`) |
| `token_budget` | int | no | Cairn-estimated-token budget; overrides the configured default (2000–4000) |

**Output**

```json
{
  "ok": true,
  "briefing": {
    "project": { "id": "…", "name": "…" },
    "repository": { "branch": "…", "commit": "…", "worktree_clean": false,
                    "staged": 1, "unstaged": 3, "untracked": 0 },
    "task": { "id": "…", "title": "…", "goal": "…", "acceptance_criteria": ["…"] },
    "previous_handoff": { "session_id": "…", "next_step": "…",
                          "remaining_work": ["…"], "changed_files": ["…"] },
    "decisions": ["…"],
    "known_failures": ["…"],
    "memory": { "task": ["…"], "branch": ["…"], "project": ["…"] },
    "no_prior_history": false
  },
  "estimated_tokens": 2840,
  "budget": 3000,
  "truncated": true,
  "omitted_sections": ["project_memory"],
  "degraded": false
}
```

Sections are filled in the priority order of D8 and dropped from the bottom when the
budget binds. Both `estimated_tokens` and `budget` are denominated in **Cairn-estimated
tokens**, not any model's tokenizer; `estimated_tokens` never exceeds `budget` because the
assembler measures each section with that estimator before emitting it and stops (FR-029). `truncated` and `omitted_sections` are
always present (FR-030). A repository Cairn has never seen returns
`no_prior_history: true` and still succeeds (FR-031).

`degraded: true` means the briefing could not be fully assembled within the context
deadline — storage was busy, the daemon was still starting, Git was slow. The tool still
returns `ok: true` with whatever was ready, and the caller is expected to proceed
(FR-046).

---

## `cairn_search`

Search memory. Exact filters plus lexical relevance, ranked by scope precedence.

**Input**

| Field | Type | Required | Notes |
|---|---|---|---|
| `cwd` | string | yes | |
| `query` | string | no | Omit to filter without a text query |
| `scope` | enum | no | `project` \| `branch` \| `task` \| `session` |
| `scope_key` | string | no | Requires `scope` |
| `type` | enum | no | `fact` \| `decision` \| `convention` \| `failure` \| `procedure` |
| `state` | enum | no | Default `active`; `stale` and `superseded` on request only |
| `limit` | int | no | Default 10, max 50 |

**Output**

```json
{
  "ok": true,
  "results": [
    { "id": "…", "type": "convention", "scope": "task", "scope_key": "…",
      "content": "…", "state": "active", "created_at": "…",
      "provenance": { "session_id": "…", "observation_ids": [], "evidence_count": 0 },
      "rank": { "scope_bucket": 0, "relevance": 8.41, "age_days": 2 } }
  ],
  "total": 3
}
```

Ordering is scope bucket first (task 0, branch 1, project 2, session 3), then relevance
and recency within a bucket (FR-024). Every result carries provenance (FR-026, SC-012).
Provenance is a mandatory `session_id` plus zero or more identifiers and a count;
`evidence_count: 0` is normal for memory recorded in manual MCP mode and is not an error
(FR-019). Observation *content* is resolved locally on demand and never travels with a
shared memory (FR-055). An identifier whose observation was deleted resolves to
`evidence deleted` rather than disappearing (FR-052).

---

## `cairn_remember`

Record durable knowledge, or change the state of existing knowledge.

**Input**

| Field | Type | Required | Notes |
|---|---|---|---|
| `cwd` | string | yes | |
| `action` | enum | yes | `create` \| `supersede` \| `forget` |
| `type` | enum | on `create` | Memory type |
| `scope` | enum | on `create` | Defaults to `task` when a task is bound, else `branch` |
| `content` | string | on `create` | Redacted before storage |
| `evidence_observation_ids` | string[] | no | Zero or more supporting observations; omit it in manual mode. Cairn never fabricates evidence (FR-019) |
| `local_only` | bool | no | Default false; never transmitted when true |
| `memory_id` | string | on `supersede`/`forget` | Target |

**Output**: `{ "ok": true, "memory": { … }, "superseded": "…" }`

`supersede` creates the replacement and marks the original `superseded`, retaining both
and the link (FR-020). `create` requires no evidence: `origin_session_id` comes from the
current session, and a memory with no supporting observations is fully valid (FR-019).
`forget` tombstones **that memory only** — never its evidence, its origin session, or any
other memory — and for shared memories queues an idempotent deletion (FR-052).

---

## `cairn_session`

Inspect and steer the current session.

**Input**

| Field | Type | Required | Notes |
|---|---|---|---|
| `cwd` | string | yes | |
| `action` | enum | yes | `current` \| `start` \| `bind_task` \| `end` |
| `agent` | string | on `start` | e.g. `claude-code` |
| `agent_session_key` | string | no | The agent's own session identifier; synthesized per connection when absent |
| `task_id` | string | on `bind_task` | |
| `status` | enum | on `end` | `completed` \| `interrupted` |

`end` is one of only three ways a session leaves `active`; the others are the `SessionEnd`
hook and daemon-start reconciliation. Cairn has no liveness signal and never infers one
(D16).

**Output**: `{ "ok": true, "session": { "id": "…", "status": "active",
"branch": "…", "commit": "…", "task_id": "…", "previous_session_id": "…",
"started_at": "…" } }`

`start` is idempotent **per agent session, not per worktree**: an existing session with the
same `agent_session_key` is returned rather than duplicated, and a second agent working in
the same worktree gets its own distinct session (FR-010). `current` resolves by
`agent_session_key` when the caller supplies one; where it cannot be resolved and several
sessions are active in the worktree, the tool returns `invalid_request` naming the
candidates rather than guessing.

---

## `cairn_task`

**Input**

| Field | Type | Required | Notes |
|---|---|---|---|
| `cwd` | string | yes | |
| `action` | enum | yes | `list` \| `get` \| `create` \| `update` |
| `task_id` | string | on `get`/`update` | |
| `title`, `goal` | string | on `create` | |
| `acceptance_criteria` | string[] | no | |
| `status` | enum | no | `todo` \| `in_progress` \| `done` \| `blocked` |

**Output**: `{ "ok": true, "task": { … } }` or `{ "ok": true, "tasks": [ … ] }`

---

## `cairn_handoff`

**Input**

| Field | Type | Required | Notes |
|---|---|---|---|
| `cwd` | string | yes | |
| `action` | enum | yes | `latest` \| `generate` \| `annotate` |
| `session_id` | string | no | Defaults to the current session |
| `trigger` | enum | on `generate` | `pre_compact` \| `session_end` — a turn checkpoint produces no handoff (FR-032) |
| `note` | string | on `annotate` | Bounded; stored as `agent_note`, attributed |

**Output**: `{ "ok": true, "handoff": { … } }` — the full structure from data-model.md.

`annotate` cannot alter derived fields. The record of what happened comes from
observations; the agent may add a note beside it, never in place of it (FR-034).
