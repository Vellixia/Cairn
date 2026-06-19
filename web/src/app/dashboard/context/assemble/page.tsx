"use client";

import { useState } from "react";
import { postJSON } from "@/lib/api";
import { pushToast } from "@/lib/hooks";

export default function AssemblePage() {
  const [paths, setPaths] = useState("");
  const [budget, setBudget] = useState(4000);
  const [view, setView] = useState<string | null>(null);
  const [reportJson, setReportJson] = useState<string | null>(null);

  async function run() {
    const ps = paths.split(/\s+/).filter(Boolean);
    if (ps.length === 0) return;
    try {
      const qs = ps.map((p) => `path=${encodeURIComponent(p)}`).join("&");
      const r = await postJSON<{ view: string; report?: unknown }>(
        `/api/context/assemble?${qs}&budget=${budget}`,
        {},
      );
      setView(r.view);
      setReportJson(r.report ? JSON.stringify(r.report, null, 2) : null);
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Assemble failed", "error");
    }
  }

  return (
    <div className="space-y-6 max-w-4xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Assemble</h1>
        <p className="mt-1 text-sm text-slate">
          Pack several files into a token budget. Edge-ordered, reports dropped items.
        </p>
      </header>

      <section className="rounded-xl border border-line bg-surface p-5 space-y-3">
        <label className="block">
          <span className="block text-xs uppercase tracking-wider text-slate mb-1">Paths (whitespace-separated)</span>
          <textarea
            value={paths}
            onChange={(e) => setPaths(e.target.value)}
            rows={3}
            placeholder="crates/cairn-core/src/lib.rs crates/cairn-api/src/lib.rs README.md"
            className="w-full rounded-lg border border-line bg-surface2 px-3 py-2 font-mono text-sm outline-none focus:border-ember"
          />
        </label>
        <div className="flex items-center gap-3">
          <label className="flex items-center gap-2 text-sm">
            <span className="text-slate">Budget (tokens)</span>
            <input
              type="number"
              value={budget}
              onChange={(e) => setBudget(parseInt(e.target.value, 10) || 1000)}
              className="w-28 rounded-lg border border-line bg-surface2 px-3 py-1.5 text-sm outline-none focus:border-ember"
            />
          </label>
          <button onClick={run} className="rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206]">
            Assemble
          </button>
        </div>
      </section>

      {view && (
        <section className="rounded-xl border border-line bg-surface p-5 space-y-2">
          <h2 className="text-xs uppercase tracking-[0.08em] text-slate">Output</h2>
          <pre className="max-h-[28rem] overflow-auto rounded-lg border border-line bg-surface2 p-3 font-mono text-xs text-[#cdd5e0] whitespace-pre-wrap">
            {view}
          </pre>
          {reportJson && (
            <pre className="rounded-md border border-line bg-surface2 p-2 font-mono text-[11px] text-slate overflow-x-auto">
              {reportJson}
            </pre>
          )}
        </section>
      )}
    </div>
  );
}
