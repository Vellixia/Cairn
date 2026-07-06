import type { HelpContent } from "@/components/HelpButton";

export type HelpCopy = HelpContent;

export const HELP: Record<string, HelpCopy> = {
  "/memory": {
    title: "Memory browser",
    what: "Every memory Cairn has - filterable, sortable, searchable - with a full-detail drawer showing provenance, scope, trust signals, and edges to related memories.",
    how: [
      "Filter by scope / tier / kind, toggle pinned or suspicious, and search content + concepts.",
      "Click a row for the full record; edge chips hop the drawer to related memories.",
      "The Wakeup-order toggle previews the ranked list a fresh agent session loads first.",
      "Pin and Delete are the only manual actions - agents do the real curation via MCP.",
    ],
    impact:
      "The primary observability surface: if an agent knows it, it is visible here.",
  },
  "/memory/graph": {
    title: "Memory graph",
    what: "A live map of relationships between memories, extracted from their edges.",
    how: [
      "Drag to pan, scroll to zoom. Click a node to focus it and its neighbours.",
      "Use the search box to jump to a specific memory.",
    ],
    impact:
      "Edges are auto-derived from co-recall and explicit links. Graph traversal is what makes proactive recall fire on related cues.",
  },
  "/memory/savings": {
    title: "Savings",
    what: "The tamper-evident ledger of every byte Cairn has saved you by reading less.",
    how: [
      "Filter by date or source. The total at the top is your running saved-bytes counter.",
      "Click Verify to re-check the chain. Any mismatch means the ledger was tampered with.",
    ],
    impact:
      "Bytes saved -> tokens saved -> USD saved. This page is the proof that the read modes are actually doing their job.",
  },
  "/you": {
    title: "Your profile",
    what: "Standing preferences every Cairn-backed agent honors, plus device tokens and settings.",
    how: [
      "Add or delete preferences directly here, or let an agent record one with the prefer tool.",
      "Issue and revoke device tokens under Tokens.",
    ],
    impact:
      "Preferences are injected at every session start, before wakeup memories - this is one of the few pages where manual input is the point (a preference is a deliberate, rare decision).",
  },
  "/you/tokens": {
    title: "Device tokens",
    what: "Issue, list, and revoke the tokens your CLI / MCP clients use to talk to this server.",
    how: [
      "Click Issue token. Pick a name and scope (admin / write / read).",
      "Copy the token from the response --- it is shown once. Revoke here when a device is lost.",
    ],
    impact:
      "Tokens are bearer credentials. Revoke immediately on loss; expired tokens are rejected, not auto-rotated.",
  },
  "/you/audit": {
    title: "Audit log",
    what: "The last 50 administrative events on this server: logins, token issues, rollbacks, exports.",
    how: [
      "Filter by kind or actor. Each row links to the relevant page.",
      "Audit entries are append-only. The chain is verified nightly.",
    ],
    impact:
      "The audit log is the source of truth for who did what when. It feeds into the savings chart and the activity timeline on the overview.",
  },
  "/you/sessions": {
    title: "Active sessions",
    what: "The active agent sessions connected to this server.",
    how: [
      "Click a session to see its anchor, current context, and the memories it has loaded.",
      "End a session to drop its working-tier memories and free the slot.",
    ],
    impact:
      "Each session holds working-tier memory. End idle sessions to keep the tier lean and recall fast.",
  },
  "/you/settings": {
    title: "Settings",
    what: "Your admin session, server info, and the full effective server configuration - every knob, its current value, and the env var that changes it.",
    how: [
      "Server configuration is read-only here: edit your .env (or environment) and restart the server to apply a change.",
      "Secrets are redacted to set / not set - values never leave the server.",
      "Sign out to invalidate this browser's session cookie.",
    ],
    impact:
      "One-time customization lives in the environment; this page makes the effective result visible so you never have to guess what the server is running with.",
  },
  "/projects": {
    title: "Projects",
    what: "Repos an agent with Cairn hooks has worked in, auto-detected on session start, with a memory count per project.",
    how: [
      "Click a project to see its Project-scoped memories and promotion activity.",
      "Registration is independent of scope isolation - it's discovery, not an access boundary.",
    ],
    impact:
      "Projects are read-only observability - there's no manual create/edit here. A project appears the first time an agent's SessionStart hook auto-detects the repo.",
  },
  "/documents": {
    title: "Documents",
    what: "Reference material ingested via `cairn documents ingest` - chunked text from files or URLs, searchable through this page's own search box.",
    how: [
      "Ingest via `cairn documents ingest <path|url>` (CLI) from inside a project to scope a document to it - this page is observability + a search preview, not an upload form.",
      "Documents ingested inside a project show up here AND on that project's own page; documents ingested outside any project are global and visible to every project.",
      "Use the search box to preview what a query would surface across every visible document.",
      "Delete removes the chunks; re-ingesting the same source later restores them.",
    ],
    impact:
      "Documents give agents external reference material (docs, READMEs, specs) alongside their own memories - project-scoped by default so a project's docs don't clutter every other project's context.",
  },
  "/automation": {
    title: "Automation",
    what: "Everything the machine does on its own: the autopilot digest, guard (edit-safety) decisions, the promotion/demotion trail, background jobs - plus the one review queue that wants a human glance.",
    how: [
      "Pick a lookback window for the digest tiles (24h / 48h / 7d).",
      "Review queue: promote or dismiss borderline (0.70-0.90) promotion candidates - the only manual decision left.",
      "Guard: drift decisions are made by the autopilot at verify time (CAIRN_DRIFT_AUTOPILOT); this log is the read-only audit trail. Danger edits are never auto-approved.",
      "Run now triggers a background job immediately - same code path the scheduler uses.",
      "Promotion log rows for a promoted memory get an Undo button.",
    ],
    impact:
      "Run history is in-memory since server start; the promotion log and drift log are durable. Undo reverts a memory to the scope it was promoted from.",
  },
  "/memory/architecture": {
    title: "Architecture report",
    what: "Structural analysis of the memory graph as code: nodes (files/memories), edges (relationships), communities, bridges, and cycles.",
    how: [
      "Open /memory/architecture (linked from the Memory browser header).",
      "Read the four KPIs (Nodes / Edges / Communities / Isolation) for a quick read.",
      "Click .md to download the full report as markdown.",
    ],
    impact:
      "Surfaces god nodes (high centrality), bridges (cut vertices), and cycles --- all candidates for refactoring.",
  },
  "/memory/heatmap": {
    title: "Activity heatmap",
    what: "Daily memory creation over the last 52 weeks, GitHub-style. Hover a cell to see the date and count.",
    how: [
      "Open /memory/heatmap (linked from the Memory browser header).",
      "Hover any cell to read the day + count.",
      "Compare against the recent activity card on / for spot trends.",
    ],
    impact:
      "Lets you see drift in memory-write cadence without scrolling the audit log.",
  },
};
