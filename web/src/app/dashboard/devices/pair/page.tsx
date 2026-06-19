"use client";

import { useState } from "react";
import { postJSON, type PairCode } from "@/lib/api";
import { pushToast } from "@/lib/hooks";

export default function PairCodePage() {
  const [name, setName] = useState("");
  const [ttl, setTtl] = useState(10);
  const [pair, setPair] = useState<PairCode | null>(null);
  const [busy, setBusy] = useState(false);

  async function generate() {
    if (!name.trim()) return;
    setBusy(true);
    try {
      const p = await postJSON<PairCode>("/api/devices/pair-codes", { name, ttl_minutes: ttl });
      setPair(p);
      pushToast(`Pair code for "${p.name}" valid ${ttl} min`, "success");
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Generate failed", "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-6 max-w-2xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Pair a new device</h1>
        <p className="mt-1 text-sm text-slate">
          Generate a short code, then on the new device run{" "}
          <code className="font-mono">{`cairn pair <code> --server ${typeof window !== "undefined" ? window.location.origin : "<server>"}`}</code>.
          No long tokens to copy.
        </p>
      </header>

      <section className="rounded-xl border border-line bg-surface p-5 space-y-3">
        <div className="grid gap-3 md:grid-cols-[1fr_8rem_auto]">
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="device name (e.g. laptop)"
            className="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-ember"
          />
          <input
            type="number"
            min={1}
            max={60}
            value={ttl}
            onChange={(e) => setTtl(Math.max(1, Math.min(60, parseInt(e.target.value, 10) || 10)))}
            className="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-ember"
          />
          <button
            onClick={generate}
            disabled={busy}
            className="rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206] disabled:opacity-50"
          >
            {busy ? "…" : "Generate"}
          </button>
        </div>
        <p className="text-[11px] text-slate">TTL: 1–60 minutes (default 10).</p>
      </section>

      {pair && (
        <section className="rounded-xl border border-ember bg-surface p-5 text-center">
          <div className="font-mono text-4xl font-bold tracking-[0.3em] text-ember">{pair.code}</div>
          <div className="mt-2 text-xs text-slate">
            valid until {new Date(pair.expires_at).toLocaleString()} · single use
          </div>
          <button
            onClick={() => navigator.clipboard.writeText(pair.code).then(() => pushToast("Copied", "success"))}
            className="mt-3 rounded-lg border border-line px-3 py-1.5 text-xs hover:bg-surface2"
          >
            Copy code
          </button>
        </section>
      )}
    </div>
  );
}
