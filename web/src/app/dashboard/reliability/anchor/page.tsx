"use client";

import { useEffect, useState } from "react";
import { getJSON, postJSON } from "@/lib/api";
import { pushToast } from "@/lib/hooks";

export default function AnchorPage() {
  const [anchor, setAnchor] = useState<string | null>(null);
  const [goal, setGoal] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    getJSON<{ anchor: string | null }>("/api/guard/anchor")
      .then((r) => setAnchor(r.anchor))
      .catch(() => {});
  }, []);

  async function save() {
    if (!goal.trim()) return;
    setBusy(true);
    try {
      await postJSON("/api/guard/anchor", { goal });
      setAnchor(goal);
      setGoal("");
      pushToast("Anchor set", "success");
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Failed", "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-6 max-w-2xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Task anchor</h1>
        <p className="mt-1 text-sm text-slate">
          The goal re-injected at every session start. If you set one, you stop having to
          re-explain the task every time.
        </p>
      </header>

      <section className="rounded-xl border border-line bg-surface p-5 space-y-3">
        {anchor ? (
          <p className="rounded-md border border-line bg-surface2 px-3 py-2 text-sm text-offwhite">{anchor}</p>
        ) : (
          <p className="text-sm text-slate">No anchor set.</p>
        )}
        <div className="flex gap-2">
          <input
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") save(); }}
            placeholder='e.g. "Ship the HelixDB backend behind the store seam"'
            className="flex-1 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-ember"
          />
          <button
            onClick={save}
            disabled={busy}
            className="rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206] disabled:opacity-50"
          >
            {anchor ? "Update" : "Set"}
          </button>
        </div>
      </section>
    </div>
  );
}
