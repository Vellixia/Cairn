"use client";

import { useQuery } from "@/lib/hooks";
import { type Stats } from "@/lib/api";

export default function ReliabilityScorePage() {
  const stats = useQuery<Stats>("/api/stats", { pollMs: 5_000 });
  const rel = stats.data?.reliability;

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Reliability</h1>
        <p className="mt-1 text-sm text-slate">
          How well Cairn has been guarding your edits. Updates after every verify/checkpoint/rollback.
        </p>
      </header>

      <section className="rounded-xl border border-line bg-surface p-6">
        {!rel ? (
          <p className="text-sm text-slate">No edit history yet. Run <code>cairn-cli verify</code> or call <code>/api/guard/verify</code> to seed the score.</p>
        ) : (
          <>
            <div className={`text-6xl font-bold ${
              rel.score >= 80 ? "text-teal" : rel.score >= 50 ? "text-ember" : "text-[#f87171]"
            }`}>
              {rel.score}
              <span className="text-lg text-slate">/100</span>
            </div>
            <dl className="mt-4 grid grid-cols-2 sm:grid-cols-5 gap-3 text-sm">
              <Cell label="samples" value={rel.samples} />
              <Cell label="ok" value={rel.ok} accent="text-teal" />
              <Cell label="warn" value={rel.warn} accent="text-ember" />
              <Cell label="danger" value={rel.danger} accent="text-[#f87171]" />
              <Cell label="rollbacks" value={rel.rollbacks} />
            </dl>
          </>
        )}
      </section>
    </div>
  );
}

function Cell({ label, value, accent }: { label: string; value: number; accent?: string }) {
  return (
    <div className="rounded-md bg-surface2 px-3 py-2">
      <div className="text-[10px] uppercase tracking-wider text-slate">{label}</div>
      <div className={`font-mono ${accent ?? "text-offwhite"}`}>{value}</div>
    </div>
  );
}
