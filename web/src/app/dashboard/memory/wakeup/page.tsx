"use client";

import { useEffect, useState } from "react";
import { getJSON, type Memory } from "@/lib/api";

export default function WakeupPage() {
  const [memories, setMemories] = useState<Memory[] | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    getJSON<Memory[]>("/api/memory/wakeup?limit=50")
      .then(setMemories)
      .catch((e) => setErr(e instanceof Error ? e.message : "Failed to load"));
  }, []);

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Wakeup</h1>
        <p className="mt-1 text-sm text-slate">
          The "first thing the agent reads" — high-importance, recently-reinforced decisions
          and tasks. What every new session starts with.
        </p>
      </header>

      {err && (
        <p className="rounded-md border border-[#f87171] bg-surface2 px-3 py-2 text-sm text-[#f87171]">
          {err}
        </p>
      )}

      <section className="rounded-xl border border-line bg-surface p-5">
        {memories === null ? (
          <p className="text-sm text-slate">Loading…</p>
        ) : memories.length === 0 ? (
          <p className="text-sm text-slate">Nothing to wake up to yet.</p>
        ) : (
          <ul className="space-y-2">
            {memories.map((m) => (
              <li key={m.id} className="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm">
                {m.content}
                <div className="mt-1 text-[11px] text-slate">
                  <span className="text-ember font-mono">{m.kind}</span> · {m.tier} · importance {m.importance.toFixed(2)} · accessed {m.access_count}×
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
