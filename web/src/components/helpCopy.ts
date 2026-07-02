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
      "Recall runs against the local SurrealDB index in <50ms for 10k items.",
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
  "/memory/promotion": {
    title: "Promotion & Intelligence",
    what: "v0.8.0: memories the nightly llm-intelligence job thinks are broadly useful enough to promote to Global scope, plus a log of recent background-job runs.",
    how: [
      "Promote moves a memory to Global scope and locks it against future suggestions.",
      "Dismiss locks it without changing scope (\"don't ask again\").",
      "The Intelligence Log shows what session-gc/memory-decay/access-log-prune/llm-intelligence did on their last run.",
    ],
    impact:
      "Nothing here is automatic yet - every promotion still needs a human click. Auto-promotion with an Undo is a future sprint (v0.8.0 Sprint 8).",
  },
  "/memory/compression": {
    title: "Compression Lab",
    what: "Side-by-side comparison of all four read modes for a single file.",
    how: [
      "Type a file path (e.g. crates/cairn-core/src/lib.rs) and press Render.",
      "Each column shows one mode's view, its token count, and the savings vs full.",
      "The cheapest mode is highlighted as 'best' - prefer that for context-bounded reads.",
    ],
    impact:
      "Choosing the right mode per file can cut agent token spend 50-90%. Files with strong structure (Rust, Python, Go) compress aggressively; data files and small snippets do not.",
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
      "Profile is read-only here --- use `cairn prefer` or the prefer MCP tool to add rules.",
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
      "Copy the token from the response --- it is shown once. Revoke here when a device is lost.",
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
      "Rotate your admin password from the CLI --- invalidates all sessions.",
    ],
    impact:
      "Settings here are minimal because most config lives server-side. Sign out from a shared browser, not just close the tab.",
  },
  "/memory/architecture": {
    title: "Architecture report",
    what: "Structural analysis of the memory graph as code: nodes (files/memories), edges (relationships), communities, bridges, and cycles.",
    how: [
      "Open /memory?tab=architecture or click the Architecture tab.",
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
      "Open /memory?tab=heatmap.",
      "Hover any cell to read the day + count.",
      "Compare against the recent activity card on / for spot trends.",
    ],
    impact:
      "Lets you see drift in memory-write cadence without scrolling the audit log.",
  },
  "/registry/packs": {
    title: "Pack registry",
    what: "Published .cairnpkg packs. Search, publish new ones, click a row to see versions and download.",
    how: [
      "Publish a pack via the Publish button (upload a .cairnpkg tarball).",
      "Click a pack name to see all its versions and download counts.",
      "Use the search box to filter by name or description.",
    ],
    impact:
      "Packs ship context, prompts, and tool configs to cairn-backed agents. Signed packs are trusted; unsigned are visible but flagged.",
  },
  "/registry": {
    title: "Pack registry",
    what: "Self-hosted .cairnpkg registry. Three sections: Packs (browse/publish), Trusted Keys (signing authorities), Revocations (audit trail).",
    how: [
      "Use the tab bar above to switch between Packs, Trusted Keys, and Revocations.",
      "Publish on the Packs tab. Manage signing keys on the Trusted Keys tab. Read the audit trail on Revocations.",
    ],
    impact:
      "Trust flows top-down: add a trusted key, then publish packs signed with it. Revocations are append-only.",
  },
  "/registry/trust": {
    title: "Trusted signing keys",
    what: "Ed25519 public keys the registry trusts to sign packs. Packs signed by unlisted keys still upload but are flagged unsigned.",
    how: [
      "Add a key by pasting its 64-char hex public key.",
      "Revoke a key to mark its signed packs as untrusted (the packs remain on disk).",
      "Rotate by adding a new key then revoking the old one.",
    ],
    impact:
      "Compromised key? Revoke first, then rotate. Revocations are append-only and surface in the revocations tab.",
  },
  "/registry/revocations": {
    title: "Revocation log",
    what: "Append-only record of every pack unpublish and key revoke. Audit trail for trust changes.",
    how: [
      "Read-only --- new revocations appear here automatically.",
      "Filter by kind (pack unpublish vs key revoke) or actor.",
      "Use this page to answer 'who removed X and when'.",
    ],
    impact:
      "Revocations cannot be undone. Operators should record the reason in the audit log before revoking.",
  },
};
