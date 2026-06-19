"use client";

import { useState } from "react";
import { getJSON, type ScoredMemory } from "@/lib/api";
import { pushToast } from "@/lib/hooks";

export default function RecallPage() {
  const [q, setQ] = useState("");
  const [hits, setHits] = useState<ScoredMemory[] | null>(null);
  const [busy, setBusy] = useState(false);

  async function recall() {
    setBusy(true);
    try {
      const r = await getJSON<ScoredMemory[]>(
        `/api/memory/recall?limit=20&q=${encodeURIComponent(q)}`,
      );
      setHits(r);
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Recall failed", "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Recall</h1>
        <p className="mt-1 text-sm text-slate">
          BM25 lexical recall with semantic fallback when embeddings are enabled.
        </p>
      </header>

      <section className="rounded-xl border border-line bg-surface p-5">
        <div className="flex gap-2">
          <input
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") recall(); }}
            placeholder='e.g. "why SQLite"'
            className="flex-1 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-ember"
          />
          <button
            onClick={recall}
            disabled={busy}
            className="rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206] disabled:opacity-50"
          >
            {busy ? "…" : "Recall"}
          </button>
        </div>
      </section>

      <section className="rounded-xl border border-line bg-surface p-5">
        <h2 className="mb-3 text-xs uppercase tracking-[0.08em] text-slate">Results</h2>
        {hits === null && (
          <p className="text-sm text-slate">Search to see results.</p>
        )}
        {hits !== null && hits.length === 0 && (
          <p className="text-sm text-slate">No matches yet.</p>
        )}
        {hits !== null && hits.length > 0 && (
          <ul className="space-y-2">
            {hits.map((h) => (
              <li key={h.memory.id} className="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm">
                {h.memory.content}
                <div className="mt-1 text-[11px] text-slate">
                  <span className="text-ember font-mono">{h.score.toFixed(2)}</span> · {h.memory.kind} · {h.memory.tier}
                  {h.memory.concepts?.length > 0 && <> · {h.memory.concepts.join(", ")}</>}
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
