"use client";

import { useState } from "react";
import { useQuery, pushToast } from "@/lib/hooks";
import { postJSON, type Checkpoint, type RollbackReport } from "@/lib/api";

export default function CheckpointsPage() {
  const cps = useQuery<Checkpoint[]>("/api/guard/checkpoints");
  const [label, setLabel] = useState("");
  const [busy, setBusy] = useState(false);

  async function create() {
    setBusy(true);
    try {
      const q = label.trim() ? `?label=${encodeURIComponent(label.trim())}` : "";
      const cp = await postJSON<Checkpoint>(`/api/guard/checkpoint${q}`, {});
      pushToast(`Checkpoint ${cp.id.slice(0, 8)} · ${cp.files} files`, "success");
      setLabel("");
      cps.refetch();
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Checkpoint failed", "error");
    } finally {
      setBusy(false);
    }
  }

  async function rollback(id: string) {
    if (!window.confirm(`Roll back to checkpoint ${id.slice(0, 8)}? Tracked files on disk will be restored.`)) return;
    try {
      const r = await postJSON<RollbackReport>(`/api/guard/rollback?id=${encodeURIComponent(id)}`, {});
      pushToast(`Restored ${r.restored.length} · skipped ${r.skipped.length}`, "info");
      cps.refetch();
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Rollback failed", "error");
    }
  }

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Checkpoints</h1>
        <p className="mt-1 text-sm text-slate">
          Snapshot every file Cairn has tracked, then roll back any tracked file to that snapshot.
        </p>
      </header>

      <section className="rounded-xl border border-line bg-surface p-5 space-y-3">
        <div className="flex gap-2">
          <input
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            placeholder="label (optional)"
            className="flex-1 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-ember"
          />
          <button
            onClick={create}
            disabled={busy}
            className="rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206] disabled:opacity-50"
          >
            {busy ? "…" : "Checkpoint"}
          </button>
        </div>
      </section>

      <section className="rounded-xl border border-line bg-surface p-5">
        <h2 className="mb-3 text-xs uppercase tracking-[0.08em] text-slate">History</h2>
        {cps.loading ? (
          <p className="text-sm text-slate">Loading…</p>
        ) : cps.data && cps.data.length === 0 ? (
          <p className="text-sm text-slate">
            No checkpoints — your edits aren't being snapshotted yet. Create one before risky changes.
          </p>
        ) : cps.data && cps.data.length > 0 ? (
          <ul className="space-y-2">
            {cps.data.map((c) => (
              <li key={c.id} className="flex items-center justify-between rounded-lg border border-line bg-surface2 px-3 py-2 text-sm">
                <div>
                  <div className="text-offwhite">{c.label || "(unlabeled)"}</div>
                  <div className="text-[11px] text-slate font-mono">
                    {c.id.slice(0, 8)} · {c.files} files · {new Date(c.created_at).toLocaleString()}
                  </div>
                </div>
                <button
                  onClick={() => rollback(c.id)}
                  className="rounded-lg border border-line px-3 py-1.5 text-xs hover:bg-surface"
                >
                  Rollback
                </button>
              </li>
            ))}
          </ul>
        ) : null}
      </section>
    </div>
  );
}
