"use client";

import { useState } from "react";
import { postJSON, type Memory, type ScoredMemory } from "@/lib/api";
import { pushToast } from "@/lib/hooks";

export default function MemoryPage() {
  const [content, setContent] = useState("");
  const [busy, setBusy] = useState(false);

  async function remember() {
    if (!content.trim()) return;
    setBusy(true);
    try {
      const m = await postJSON<Memory>("/api/memory", { content });
      pushToast(`stored ${m.kind}/${m.tier} · ${m.id.slice(0, 8)}`, "success");
      setContent("");
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Failed to store memory", "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Memories</h1>
        <p className="mt-1 text-sm text-slate">
          Store a memory. Every memory is content-hashed, deduped, and given a tier
          (working / long-term / archive).
        </p>
      </header>

      <section className="rounded-xl border border-line bg-surface p-5">
        <h2 className="mb-3 text-xs uppercase tracking-[0.08em] text-slate">Remember</h2>
        <textarea
          value={content}
          onChange={(e) => setContent(e.target.value)}
          rows={4}
          placeholder="e.g. We chose SQLite + a content-hash blob store so compression stays lossless."
          className="w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-ember"
        />
        <button
          onClick={remember}
          disabled={busy}
          className="mt-3 rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206] disabled:opacity-50"
        >
          {busy ? "Storing…" : "Remember"}
        </button>
      </section>

      <p className="text-sm text-slate">
        To recall or wakeup, use the sidebar. Every remembered note is also picked up by
        Recall (BM25) and the dashboard Overview's recent-memory panel.
      </p>
    </div>
  );
}

