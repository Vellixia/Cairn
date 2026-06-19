"use client";

import { useState } from "react";
import { useQuery, pushToast } from "@/lib/hooks";
import { getJSON, postJSON, type Pool, type ShareExport, type Sensitivity } from "@/lib/api";

const BADGE: Record<Sensitivity, string> = {
  shareable: "border-teal text-teal",
  needs_review: "border-ember text-ember",
  private: "border-[#f87171] text-[#f87171]",
};

export default function PoolPage() {
  const pool = useQuery<Pool>("/api/pool");
  const [busy, setBusy] = useState(false);

  async function publish() {
    setBusy(true);
    try {
      const bundle = await getJSON<ShareExport>("/api/share/export");
      const res = await postJSON<{ accepted: number; rejected: number }>(
        "/api/pool/contribute",
        bundle,
      );
      pushToast(`Published · ${res.accepted} accepted, ${res.rejected} rejected`, "success");
      pool.refetch();
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Publish failed", "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Pool</h1>
        <p className="mt-1 text-sm text-slate">
          The collective, sanitized knowledge this server shares with other Cairn servers.
        </p>
      </header>

      <section className="rounded-xl border border-line bg-surface p-5 space-y-3">
        <div className="flex items-center gap-3">
          <button
            onClick={publish}
            disabled={busy}
            className="rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206] disabled:opacity-50"
          >
            {busy ? "…" : "Publish my shareable memories"}
          </button>
          {pool.data && <span className="text-sm text-slate">{pool.data.count} in pool</span>}
        </div>
      </section>

      <section className="rounded-xl border border-line bg-surface p-5">
        <h2 className="mb-3 text-xs uppercase tracking-[0.08em] text-slate">In pool</h2>
        {pool.loading ? (
          <p className="text-sm text-slate">Loading…</p>
        ) : pool.data && pool.data.memories.length === 0 ? (
          <p className="text-sm text-slate">Empty pool. Publish your shareable memories to start contributing.</p>
        ) : pool.data ? (
          <ul className="space-y-2">
            {pool.data.memories.map((m, i) => (
              <li key={i} className="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm">
                {m.content}
                <div className="mt-1 flex items-center gap-2 text-[11px] text-slate">
                  <span className={`rounded-full border px-2 py-0.5 ${BADGE[m.sensitivity]}`}>
                    {m.sensitivity}
                  </span>
                  <span>{m.kind}</span>
                  {m.redactions > 0 && <span>· {m.redactions} redaction{m.redactions === 1 ? "" : "s"}</span>}
                </div>
              </li>
            ))}
          </ul>
        ) : null}
      </section>

      <section className="rounded-xl border border-line bg-surface p-5">
        <h2 className="mb-3 text-xs uppercase tracking-[0.08em] text-slate">Federate across machines</h2>
        <p className="mb-2 text-sm text-slate">
          Run on the peer machine:
        </p>
        <pre className="rounded-md border border-line bg-surface2 p-3 font-mono text-xs text-[#cdd5e0] overflow-x-auto">{`cairn contribute --server ${typeof window !== "undefined" ? window.location.origin : "<server>"}
cairn pull --server ${typeof window !== "undefined" ? window.location.origin : "<server>"}`}</pre>
      </section>
    </div>
  );
}
