"use client";

import { useState } from "react";
import { getJSON, type ShareExport } from "@/lib/api";
import { pushToast } from "@/lib/hooks";

export default function BundlePage() {
  const [bundle, setBundle] = useState<ShareExport | null>(null);
  const [busy, setBusy] = useState(false);

  async function build() {
    setBusy(true);
    try {
      const r = await getJSON<ShareExport>("/api/share/export");
      setBundle(r);
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Export failed", "error");
    } finally {
      setBusy(false);
    }
  }

  async function copy() {
    if (!bundle) return;
    const json = JSON.stringify(bundle, null, 2);
    await navigator.clipboard.writeText(json);
    pushToast("Bundle copied to clipboard", "success");
  }

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Bundles</h1>
        <p className="mt-1 text-sm text-slate">
          A sanitized, shareable export of every memory safe to pool with other Cairn servers.
          Imported with <code>cairn-cli import --share bundle.json</code>.
        </p>
      </header>

      <section className="rounded-xl border border-line bg-surface p-5 space-y-3">
        <button
          onClick={build}
          disabled={busy}
          className="rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206] disabled:opacity-50"
        >
          {busy ? "Building…" : "Build shareable bundle"}
        </button>
        {bundle && (
          <>
            <dl className="grid grid-cols-2 gap-y-1 text-sm">
              <Stat k="Scanned" v={String(bundle.total)} />
              <Stat k="Shareable" v={String(bundle.shared)} />
              <Stat k="Needs review" v={String(bundle.needs_review)} />
              <Stat k="Withheld (private)" v={String(bundle.withheld)} />
            </dl>
            <button onClick={copy} className="rounded-lg border border-line px-3 py-1.5 text-xs hover:bg-surface2">
              Copy JSON to clipboard
            </button>
          </>
        )}
      </section>
    </div>
  );
}

function Stat({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between border-b border-dashed border-line py-1">
      <span className="text-slate">{k}</span>
      <span className="font-mono text-teal">{v}</span>
    </div>
  );
}
