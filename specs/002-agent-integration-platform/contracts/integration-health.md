# Contract: Integration Health, Change Plans, and Desired State

**Feature**: `002-agent-integration-platform`

The machine-readable shapes behind `cairn agents`, `cairn doctor`, every `--dry-run`, and
`cairn repair`. All ride Feature 001's envelope: `{ "ok": true, "data": { … } }`.

One inspection engine produces all of them. Doctor is that engine with no plan applied;
preview is that engine plus a computed plan; connect, repair, and migrate are that engine
plus a plan that is then applied and re-inspected (FR-151).

## Detection — `cairn agents --json`

```json
{
  "ok": true,
  "data": {
    "agents": [
      {
        "agent": "codex",
        "detected": true,
        "version": "0.58.0",
        "compatibility": "compatible_unverified",
        "level": "mcp_plus",
        "missing_behaviors": ["automatic session completion (hooks awaiting trust)"],
        "capabilities": {
          "mcp_user_scope": "present",
          "mcp_project_scope": "present",
          "instructions_project": "present",
          "skill_user": "present",
          "lifecycle_session_open": "present_pending_activation",
          "lifecycle_tool_success": "present_pending_activation",
          "lifecycle_tool_failure": "present_pending_activation",
          "lifecycle_quiesce": "present_pending_activation",
          "lifecycle_pre_compaction": "present_pending_activation",
          "lifecycle_post_compaction": "present_pending_activation",
          "lifecycle_session_close": "present_pending_activation",
          "context_at_session_open": "present",
          "stable_session_identifier": "present",
          "handlers_require_trust": "yes"
        },
        "completion_guarantee": "pending_activation"
      }
    ],
    "manager": {
      "manager": "cc-switch",
      "detected": true,
      "version": "3.9.1",
      "compatibility": "compatible_unverified",
      "distributable_resources": ["mcp", "skill"],
      "target_apps": ["claude", "codex", "opencode"]
    }
  }
}
```

`missing_behaviors` is mandatory whenever `level` is below `full`, and it names behaviors in
plain language — never a score (FR-111). Detection performs no mutation and needs no network
(FR-105).

## Health report — `cairn doctor --json`

```json
{
  "ok": true,
  "data": {
    "core": {
      "cli_version": "0.1.0-alpha.2",
      "daemon_version": "0.1.0-alpha.2",
      "versions_aligned": true,
      "daemon_reachable": true,
      "local_schema_version": 4,
      "project_registered": true
    },
    "agents": [
      {
        "agent": "claude-code",
        "detected": true,
        "version": "2.1.220",
        "compatibility": "verified",
        "level": "full",
        "completion_guarantee": "demonstrated",
        "missing_behaviors": [],
        "lifecycle_coverage": {
          "present": ["session_opened", "tool_succeeded", "tool_failed",
                      "agent_quiesced", "context_compacting", "context_compacted",
                      "session_closed"],
          "absent": []
        },
        "resources": [
          {
            "kind": "instructions",
            "condition": "outdated",
            "owner": "direct",
            "scope": "project_shared",
            "location": "./CLAUDE.md",
            "installed": { "schema": 1, "revision": "a41f0c2b9e33" },
            "current":   { "schema": 1, "revision": "8f2b19c40a7d" },
            "detail": "Cairn's managed block is one revision behind",
            "remedy": "cairn repair claude-code"
          }
        ]
      },
      {
        "agent": "opencode",
        "level": "mcp_plus",
        "completion_guarantee": "not_demonstrated",
        "missing_behaviors": [
          "automatic session completion — OpenCode signals no session end; Cairn closes idle sessions by inactivity, which is recovery, not completion"
        ],
        "lifecycle_coverage": {
          "present": ["session_opened", "tool_succeeded", "agent_quiesced",
                      "context_compacting", "context_compacted"],
          "absent": ["tool_failed", "session_closed"]
        },
        "resources": [
          {
            "kind": "skill",
            "condition": "shared",
            "owner": "direct",
            "satisfied_by": "claude-code",
            "detail": "OpenCode reads ~/.claude/skills; a second copy would collide on skill name"
          }
        ]
      }
    ],
    "manager": {
      "manager": "cc-switch",
      "detected": true,
      "resources": [
        {
          "kind": "mcp",
          "bindings": [
            { "app": "claude",   "condition": "healthy" },
            { "app": "codex",    "condition": "healthy" },
            { "app": "opencode", "condition": "missing",
              "detail": "no cairn entry in ~/.config/opencode/opencode.json",
              "remedy": "cairn integration distribute --via cc-switch --resource mcp --apps opencode" }
          ]
        }
      ],
      "ownership_consistent": false
    },
    "summary": { "healthy": 9, "actionable": 2, "blocking": 0 }
  }
}
```

**Rules**

- `condition` is drawn from the closed `HealthCondition` set (see
  [data-model.md](../data-model.md)); it is never free text (FR-167).
- `lifecycle_coverage.absent` is mandatory and is what makes an honest level auditable
  (FR-168).
- `detail` names the problem; it never quotes user configuration beyond what identifies it,
  and never contains a credential (FR-171, SC-133).
- `remedy` is a command the developer can run, or the manual sequence where no command
  applies.
- Exit `0` when every condition is `healthy`, `shared`, or `unknown`; otherwise `1`.

### Condition semantics

| Condition | Cairn's action in `repair` | In `repair --force` |
|---|---|---|
| `healthy`, `shared` | none | none |
| `missing` | reinstall | reinstall |
| `outdated` | upgrade in place | upgrade in place |
| `duplicated` | remove the extra Cairn-owned copy | same |
| `modified` | **report only** | restore inside ownership boundary, after a recovery artifact |
| `damaged_markers` | report only | **report only** — Cairn will not guess which text was its own |
| `conflicting_owner` | report only | report only |
| `malformed_config` | report only | report only |
| `installed_not_activated` | report the activation step | same |
| `migrating` | offer resume or abort | same |
| `manager_action_required` | report the manager step | same |

## Change plan — any `--dry-run --json`

```json
{
  "ok": true,
  "data": {
    "dry_run": true,
    "changes": [
      { "action": "add", "agent": "codex", "kind": "mcp",
        "owner": "direct", "scope": "user",
        "target": "~/.codex/config.toml", "detail": "[mcp_servers.cairn]" },
      { "action": "update", "agent": "codex", "kind": "instructions",
        "owner": "direct", "scope": "project_shared",
        "target": "./AGENTS.md",
        "detail": "cairn:managed block id=agent-contract schema 1 (shared with opencode)" },
      { "action": "unchanged", "agent": "codex", "kind": "skill",
        "detail": "already at schema 1 revision 8f2b19c40a7d" }
    ],
    "untouched": [
      "3 other MCP servers in ~/.codex/config.toml",
      "2 user hooks in ~/.codex/hooks.json",
      "all content outside the cairn:managed markers in ./AGENTS.md"
    ],
    "blocking": [],
    "post_apply_actions": [
      { "kind": "activation", "agent": "codex",
        "instruction": "Codex will not run these hooks until you trust them: run `codex hooks trust`",
        "verify_with": "cairn doctor codex" }
    ]
  }
}
```

**Invariants**

- Producing a plan writes nothing at all, including no temporary files. Verified by
  checksumming every candidate file before and after (SC-118).
- `untouched` is mandatory — the blast radius is part of the contract (FR-161).
- `blocking` entries carry the manual sequence and are why the operation stops.
- Preview output contains no credential encountered while inspecting (FR-162).

## Apply results

`connect`, `repair`, `migrate`, and `disconnect` return the plan they executed plus an
outcome:

```json
{
  "ok": true,
  "data": {
    "applied": [ … same shape as changes … ],
    "verified": true,
    "recovery_artifacts": [
      "~/.cairn/recovery/claude-code/instructions/2026-08-11T09-14-02Z-3c9e11ab04d7.txt"
    ],
    "post_apply_actions": [ … ],
    "level_after": "full"
  }
}
```

**Partial failure** is never reported as success (FR-155):

```json
{
  "ok": false,
  "error": { "code": "partial_apply",
             "message": "2 of 4 changes applied; the integration is incomplete" },
  "data": {
    "applied":     [ { "kind": "mcp", … }, { "kind": "instructions", … } ],
    "not_applied": [ { "kind": "lifecycle", "reason": "permission_denied: ~/.codex/hooks.json" },
                     { "kind": "skill",     "reason": "not attempted after a prior failure" } ],
    "remedy": "fix the permission and re-run `cairn connect codex`; it is idempotent"
  }
}
```

Only `recovery_artifacts` paths appear — never their content (FR-239).

## Desired state — serialized form

Not written to any project file in this feature (FR-202). This is the shape a later feature
could expose, and the shape the determinism test asserts (SC-135).

```json
{
  "schema": 1,
  "agents": [
    {
      "agent": "codex",
      "enabled": true,
      "requested_level": null,
      "resources": [
        { "kind": "mcp",          "owner": "direct", "scope": "user",
          "desired_version": null, "desired_activation": "not_applicable" },
        { "kind": "lifecycle",    "owner": "direct", "scope": "user",
          "desired_version": null, "desired_activation": "active" },
        { "kind": "instructions", "owner": "direct", "scope": "project_shared",
          "desired_version": { "schema": 1, "revision": "8f2b19c40a7d" },
          "desired_activation": "not_applicable" },
        { "kind": "skill",        "owner": "direct", "scope": "user",
          "desired_version": { "schema": 1, "revision": "c07d4419b2ae" },
          "desired_activation": "not_applicable" }
      ]
    }
  ],
  "manager": {
    "manager": "cc-switch",
    "target_apps": ["claude", "codex", "opencode"],
    "resources": ["mcp", "skill"]
  }
}
```

**Invariants** (FR-201, FR-226, SC-135)

- Deterministic: identical inputs serialize byte-identically across runs and machines. Keys
  are emitted in a fixed order; lists are sorted by a stable key.
- Secret-free by construction: no field can hold a token, credential, or key.
- Path-free: no machine-specific absolute path. Locations are named by scope and resolved
  from the scope matrix at apply time; concrete paths live only in `ResourceState.location`,
  which is local and never serialized here.
- Exactly one entry per `(agent, kind)`.
- Every integration operation reads from this one model; none derives its own view of intent.
