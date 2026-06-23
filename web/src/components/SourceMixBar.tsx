"use client";

import { useMemo } from "react";
import { useLedgerQuery } from "@/lib/queries";

const SOURCE_COLORS: Record<string, string> = {
  wakeup: "hsl(var(--color-info))",
  recall: "hsl(var(--color-positive))",
  read: "hsl(var(--color-warning))",
  assemble: "hsl(265 70% 60%)",
  drift: "hsl(var(--color-danger))",
};

function sourceColor(s: string): string {
  return SOURCE_COLORS[s] ?? "hsl(var(--muted-foreground))";
}

function lastWeekCutoff(): number {
  return Date.now() - 7 * 24 * 60 * 60 * 1000;
}

export function SourceMixBar({ className }: { className?: string }) {
  const { data, isLoading } = useLedgerQuery(500);
  const buckets = useMemo(() => {
    if (!data) return [] as { source: string; count: number }[];
    const cutoff = lastWeekCutoff();
    const counts: Record<string, number> = {};
    for (const e of data) {
      const t = new Date(e.ts).getTime();
      if (t < cutoff) continue;
      counts[e.source] = (counts[e.source] ?? 0) + 1;
    }
    return Object.entries(counts)
      .map(([source, count]) => ({ source, count }))
      .sort((a, b) => b.count - a.count);
  }, [data]);

  if (isLoading) {
    return (
      <div className={className} aria-hidden="true">
        <div className="h-3 w-full animate-pulse rounded bg-muted" />
      </div>
    );
  }
  if (buckets.length === 0) {
    return (
      <div className={className}>
        <p className="text-xs text-muted-foreground">No source activity in the last 7 days.</p>
      </div>
    );
  }
  const total = buckets.reduce((s, b) => s + b.count, 0);
  return (
    <div className={className}>
      <div className="flex h-3 w-full overflow-hidden rounded-full border border-line" role="img" aria-label={`Source mix last 7 days, ${total} entries`}>
        {buckets.map((b) => {
          const pct = (b.count / total) * 100;
          return (
            <div
              key={b.source}
              className="h-full"
              style={{ width: `${pct}%`, background: sourceColor(b.source) }}
              title={`${b.source}: ${b.count} (${pct.toFixed(1)}%)`}
            />
          );
        })}
      </div>
      <ul className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
        {buckets.map((b) => (
          <li key={b.source} className="flex items-center gap-1.5">
            <span
              className="inline-block h-2 w-2 rounded-sm"
              style={{ background: sourceColor(b.source) }}
              aria-hidden="true"
            />
            <span className="capitalize">{b.source}</span>
            <span className="font-mono text-foreground/80">{b.count}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
