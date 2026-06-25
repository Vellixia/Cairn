import type { HelpContent } from "@/components/HelpButton";

export type HelpCopy = HelpContent;

export const HELP: Record<string, HelpCopy> = {
  "/memory": {
    title: "Memory",
    what: "Observe the memories Cairn has stored and how they are connected.",
    how: [
      "Browse Wakeup for the session-start bootstrap. Use Recall to search.",
      "Graph shows the provenance graph; Savings shows the cost ledger.",
    ],
    impact:
      "The agent writes and curates memories via MCP or CLI. This view is observability.",
  },
  "/memory/recall": {
    title: "Recall",
    what: "Search across every memory Cairn has, ranked by BM25 + semantic similarity.",
    how: [
      "Type a question or phrase. Empty query returns your most-recent items.",
      "Click a result to expand it.",
    ],
    impact:
      "Recall runs against the local Helix index in <50ms for 10k items.",
  },
  "/memory/wakeup": {
    title: "Wakeup",
    what: "The high-importance memories Cairn would surface at the start of a new session.",
    how: [
      "Browse the ranked list. Items at the top are most likely to be relevant right now.",
      "Click an item to read it in full.",
    ],
    impact:
      "Wakeup is the default context Cairn loads for a fresh agent. Trimming here directly shrinks every future session's token bill.",
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
      "Bytes saved → tokens saved → USD saved. This page is the proof that the read modes are actually doing their job.",
  },
  "/trust": {
    title: "Reliability score",
    what: "Cairn's edit-guard score: how often your edits round-trip cleanly through memory.",
    how: [
      "Watch the score trend. Each sample is one edit + one re-read.",
      "Drill into the Drift tab to see flagged samples and the AI's reasoning.",
    ],
    impact:
      "Score < 70 means drift is likely. Use the agent's rollback tool to recover.",
  },
  "/trust/drift": {
    title: "Drift center",
    what: "Every reliability sample flagged as ok, warn, or danger.",
    how: [
      "Filter by status. Click any sample to see the diff and the AI's reasoning.",
      "If a danger sample is wrong, mark it resolved. Cairn adjusts the score.",
    ],
    impact:
      "Drift is the leading indicator of reliability decay. Check this page weekly if you have heavy editing traffic.",
  },
  "/you": {
    title: "Your profile",
    what: "Standing preferences Cairn-backed agents honor, plus device tokens and settings.",
    how: [
      "Profile is read-only here — use `cairn prefer` or the prefer MCP tool to add rules.",
      "Issue and revoke device tokens under Tokens.",
    ],
    impact:
      "Preferences cascade into every session. Manage them from the agent, not this form.",
  },
  "/you/tokens": {
    title: "Device tokens",
    what: "Issue, list, and revoke the tokens your CLI / MCP clients use to talk to this server.",
    how: [
      "Click Issue token. Pick a name and scope (admin / write / read).",
      "Copy the token from the response — it is shown once. Revoke here when a device is lost.",
    ],
    impact:
      "Tokens are bearer credentials. Revoke immediately on loss; expired tokens are rejected, not auto-rotated.",
  },
  "/you/pair": {
    title: "Pair a device",
    what: "Generate a short-lived pairing code so a new device can fetch its own token out-of-band.",
    how: [
      "Click Generate code. Read the 6-character code to the new device out-of-band.",
      "The new device hits /api/devices/pair with the code and gets a token.",
    ],
    impact:
      "Pair codes expire in 10 minutes and are single-use. They avoid typing long tokens over an insecure channel.",
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
    what: "Server info, your admin session, and the sign-out button.",
    how: [
      "View the server version and uptime. Sign out to invalidate this browser's session cookie.",
      "Rotate your admin password from the CLI — invalidates all sessions.",
    ],
    impact:
      "Settings here are minimal because most config lives server-side. Sign out from a shared browser, not just close the tab.",
  },
};
