---
title: "09 — Trust flows & drift log filter bug"
type: finding
status: open
updated: 2026-07-01
severity: medium
---

# 09 — Trust flows & drift log filter bug

**Run:** 3 (Trust hub)
**Date:** 2026-06-30
**Severity:** bug (functional, not crash)
**Component:** `crates/cairn-api/src/lib.rs:1102`, `crates/cairn-session/src/lib.rs:298`

## Summary

`GET /api/guard/drift?status={pending|approved|rejected}` accepts the query parameter
but the handler ignores it. All filter values return the full unfiltered list, so the
dashboard's "Pending & resolved" panel never filters to truly-pending events and
the URL is misleading to operators and to the dashboard's own copy.

## Reproducer

```bash
# Create a danger event
curl -b cookies.txt -X POST http://127.0.0.1:7777/api/guard/verify \
  -H 'Content-Type: application/json' \
  -d '{"path":"/workspace/crates/cairn-store/src/memory_backend.rs","content":"// stub"}'
# => risk: "danger"

# Approve it
curl -b cookies.txt -X POST http://127.0.0.1:7777/api/guard/drift/1/approve
# => {"ok":true,"status":"approved"}

# Query with status=pending -> should be empty, actually returns the approved event
curl -b cookies.txt 'http://127.0.0.1:7777/api/guard/drift?status=pending'
# => [{id:1, status:"approved", ...}]

# Query with status=approved -> should return the event, does
curl -b cookies.txt 'http://127.0.0.1:7777/api/guard/drift?status=approved'
# => [{id:1, status:"approved", ...}]
```

## Root cause

`crates/cairn-api/src/lib.rs:1102`:

```rust
async fn list_drift(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<DriftEvent>>, ApiError> {
    Ok(Json(s.sessions.recent_drift(200, None)?))
}
```

The `?status=` query param is never extracted (no `Query<DriftFilter>` extractor) and
`None` is hardcoded. The underlying `recent_drift(limit, Option<DriftStatus>)`
already supports filtering — only the handler is broken.

## Impact

- Dashboard "Pending & resolved" panel always shows the full history.
- Metrics endpoint `drift_pending` (`crates/cairn-api/src/metrics.rs:264`) is correct
  because it uses `None` + client-side filter on the `metrics` struct. So the metrics
  count is fine, but the per-event list is wrong.
- Operators cannot use the URL to drill into "what's actually pending".

## Suggested fix

```rust
#[derive(Deserialize)]
struct DriftFilter { status: Option<String> }

async fn list_drift(
    State(state): State<Arc<AppState>>,
    Query(q): Query<DriftFilter>,
) -> Result<Json<Vec<DriftEvent>>, ApiError> {
    let status = match q.status.as_deref() {
        Some("pending")  => Some(DriftStatus::Pending),
        Some("approved") => Some(DriftStatus::Approved),
        Some("rejected") => Some(DriftStatus::Rejected),
        _ => None,
    };
    Ok(Json(s.sessions.recent_drift(200, status)?))
}
```

## What did work in Run 3

- `/trust?tab=score` renders correctly: 5-item sidebar, Mobile button in topbar,
  100/100 reliability from 3 samples, 0 warn / 0 danger / 0 rollbacks.
- `/trust?tab=drift` renders the drift panel and shows the danger event with
  Approve/Reject buttons (before approval).
- After `POST /api/guard/drift/1/approve`, the card collapses to show status
  "approved" and the buttons disappear — UI update is correct.
- `verify_edit` path: container sees `/workspace/...` (the host bind-mount
  in `docker-compose.yml:239` mounts the project at `/workspace:ro`).
  Initial reproduce attempt using `D:\code\Cairn\...` (host path) returned
  `baseline_lines: 0` (new-file path) because the container cannot resolve
  Windows paths. Switching to the container-visible `/workspace/...` path
  fired the danger event correctly.

## Workspace pollution side-effect

`verify_edit` writes the new content to the bind-mounted file. After the
reproducer runs, the source files
`crates/cairn-core/src/lib.rs`, `crates/cairn-api/src/lib.rs`, and
`crates/cairn-store/src/memory_backend.rs` were all stubbed. All three were
restored via `git checkout -- <path>` from the host. Going forward, drift
reproducers should target `/tmp/...` or copy the test file into the container
without overwriting cairn's own source.

## Decision

Per the "no mid-run rebuilds" rule, **documented only, not fixed**. Pin
included in the final batch commit on `0.7.1`.
