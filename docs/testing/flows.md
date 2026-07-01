---
title: "Dashboard flow tests (chrome-devtools MCP)"
type: testing
status: living
updated: 2026-07-01
---

# Dashboard flow tests (chrome-devtools MCP)

The dashboard's flows are exercised by an AI agent using the `chrome-devtools` MCP server. No PowerShell, no agent-browser, no scripted assertions. The AI agent drives Chrome and asserts on real DOM state via accessibility snapshots + console messages.

## Conventions

- **Pre-conditions:** local cairn stack running at `http://127.0.0.1:7777`; admin credentials `admin / AuditPass2026!`. The browser context has no cached cookies.
- **Tools:** use `new_page`, `navigate_page`, `take_snapshot`, `fill`, `click`, `wait_for`, `list_console_messages`, `take_screenshot` from the chrome-devtools MCP.
- **Hard assertions:** every step that lists `Assert:` must be verified against the accessibility snapshot. If the assertion fails, the flow writes a markdown finding under `docs/testing/findings/<slug>.md`.
- **Console errors:** at the end of each flow, run `list_console_messages` and confirm no uncaught exceptions. Any `error` level entry from the dashboard's own JavaScript is a finding.
- **Screenshots:** one per flow. Save to `web/test/screenshots/<NN>-<flow>/<step>.png` when meaningful.
- **No fake passes:** if a step cannot be confirmed (timeout, no snapshot ref, screenshot looks identical to the previous one), write a finding. Never silently mark PASS.

## Flow list (13)

| # | Flow | Path | Risk |
|---|------|------|------|
| 01 | login-and-overview | `/` → `/login` → `/` | high |
| 02 | remember-and-recall | `/` → query → results | high |
| 03 | wakeup-and-graph | `/trust/wakeup` + `/memory/graph` | medium |
| 04 | anchor-and-drift | `/trust/anchor` + `/dashboard/reliability/drift` | high |
| 05 | registry-publish-install | `/registry` | medium |
| 06 | architecture-report-and-heatmap | `/memory/architecture` + `/memory/heatmap` | high |
| 07 | context-compression-lab | `/compression` | medium |
| 08 | token-issue-and-rotate | `/dashboard/security/tokens` | medium |
| 09 | sessions-and-audit | `/dashboard/reliability/sessions` | medium |
| 10 | assemble-budget | `/assemble` | medium |
| 11 | pwa-install-prompt | `/mobile` | high (known bug) |
| 12 | keyboard-palette | `/memory` (press `K`) | medium |
| 13 | error-envelope | direct HTTP call to a non-existent route | low |

## How to drive a single flow

For each numbered flow, the AI agent runs this script:

1. Open a new page: `new_page("http://127.0.0.1:7777/")`.
2. Navigate to the route described in the flow (call `navigate_page` with `type=url`).
3. After navigation, take an accessibility snapshot. If the page returned a Next.js error page (look for "Application error" or stack trace text in the snapshot), write a finding immediately and skip the rest.
4. For each step's `Assert:`, run a `take_snapshot` and verify the asserted ref or text is present in the snapshot.
5. Take a screenshot at the end. Save under `web/test/screenshots/<NN>-<flow>/final.png`.
6. At the end, run `list_console_messages` filtered by `types=["error"]`. Any uncaught entry → finding.

## Findings format

```markdown
# Finding: <one-line title>

**Flow:** <NN>-<flow>
**Severity:** low | medium | high | critical
**Discovered:** <YYYY-MM-DD>

## What happened
<one paragraph>

## Steps to reproduce
<numbered list>

## Expected
<one line>

## Actual
<one line>

## Console / network evidence
<quote any error JSON or stack trace>

## Suggested fix
<optional, one line>
```

Save to `docs/testing/findings/<flow-slug>-<short-desc>.md`.