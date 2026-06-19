"use client";

import { useState } from "react";
import { getJSON, type ReadResult } from "@/lib/api";
import { pushToast } from "@/lib/hooks";

export default function ContextInspectorPage() {
  const [path, setPath] = useState("README.md");
  const [mode, setMode] = useState("auto");
  const [result, setResult] = useState<ReadResult | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);

  async function read() {
    setExpanded(null);
    try {
      const r = await getJSON<ReadResult>(
        `/api/context/read?path=${encodeURIComponent(path)}&mode=${encodeURIComponent(mode)}`,
      );
      setResult(r);
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Read failed", "error");
    }
  }

  async function expand() {
    if (!result) return;
    try {
      const r = await getJSON<{ content: string }>(
        `/api/context/expand?hash=${encodeURIComponent(result.hash)}`,
      );
      setExpanded(r.content);
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Expand failed", "error");
    }
  }

  return (
    <div className="space-y-6 max-w-4xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Context Inspector</h1>
        <p className="mt-1 text-sm text-slate">
          Read a file with cache hit, AST outline, or full content — and recover the
          byte-identical original on demand.
        </p>
      </header>

      <section className="rounded-xl border border-line bg-surface p-5 space-y-3">
        <div className="flex flex-wrap gap-2">
          <input
            value={path}
            onChange={(e) => setPath(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") read(); }}
            placeholder="path relative to the server, e.g. crates/cairn-core/src/model.rs"
            className="flex-1 min-w-[20rem] rounded-lg border border-line bg-surface2 px-3 py-2 font-mono text-sm outline-none focus:border-ember"
          />
          <select
            value={mode}
            onChange={(e) => setMode(e.target.value)}
            className="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm"
          >
            <option value="auto">auto</option>
            <option value="full">full</option>
            <option value="signatures">signatures</option>
            <option value="map">map</option>
          </select>
          <button
            onClick={read}
            className="rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206]"
          >
            Read
          </button>
        </div>

        {result && (
          <div className="space-y-3 pt-2">
            <div className="grid grid-cols-2 gap-y-1 text-sm md:grid-cols-4">
              <Stat k="status" v={result.status} />
              <Stat k="lines" v={String(result.lines)} />
              <Stat k="est. tokens" v={String(result.est_tokens)} />
              <Stat k="handle" v={result.handle.slice(0, 12) + "…"} />
            </div>
            <p className="text-xs text-slate">{result.note}</p>
            <button
              onClick={expand}
              className="rounded-lg border border-line px-3 py-1.5 text-xs hover:bg-surface2"
            >
              Expand → recover byte-identical original
            </button>
            <pre className="max-h-96 overflow-auto rounded-lg border border-line bg-surface2 p-3 font-mono text-xs text-[#cdd5e0]">
              {expanded ?? (result.view || "(cached view — expand to see the full original)")}
            </pre>
          </div>
        )}
      </section>
    </div>
  );
}

function Stat({ k, v }: { k: string; v: string }) {
  return (
    <div className="rounded-md bg-surface2 px-3 py-1.5">
      <div className="text-[10px] uppercase tracking-wider text-slate">{k}</div>
      <div className="font-mono text-teal truncate">{v}</div>
    </div>
  );
}
