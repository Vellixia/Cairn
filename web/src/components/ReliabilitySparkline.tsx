"use client";

import { useMemo } from "react";
import { useLedgerQuery } from "@/lib/queries";
import { Sparkline } from "@/components/Sparkline";

const SAMPLES = 30;
const WINDOW_MS = 30 * 60 * 1000; // last 30 minutes

export function ReliabilitySparkline({ className }: { className?: string }) {
  const { data, isLoading } = useLedgerQuery(500);
  const series = useMemo(() => {
    if (!data) return [] as { x: number; y: number }[];
    const now = Date.now();
    const start = now - WINDOW_MS;
    const bucketSize = WINDOW_MS / SAMPLES;
    const buckets = new Array(SAMPLES).fill(0) as number[];
    for (const e of data) {
      const t = new Date(e.ts).getTime();
      if (t < start || t > now) continue;
      const idx = Math.min(SAMPLES - 1, Math.max(0, Math.floor((t - start) / bucketSize)));
      const saved = Math.max(0, e.bytes_in - e.bytes_out);
      buckets[idx] += saved;
    }
    // Normalize to 0-100 across the visible window. If nothing saved, baseline at 50.
    const max = Math.max(...buckets, 1);
    return buckets.map((v, i) => ({ x: i, y: v === 0 ? 50 : Math.round((v / max) * 100) }));
  }, [data]);

  if (isLoading) {
    return <div className={className} aria-hidden="true" />;
  }
  return (
    <div className={className}>
      <p className="text-xs uppercase tracking-wide text-muted-foreground">Savings · last 30 min</p>
      <div className="mt-1">
        <Sparkline data={series} height={48} />
      </div>
    </div>
  );
}
