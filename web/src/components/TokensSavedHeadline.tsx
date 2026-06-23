"use client";

import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { getJSON } from "@/lib/api";
import { useLedgerQuery } from "@/lib/queries";
import { ArrowDownRight, ArrowUpRight, Minus } from "lucide-react";

interface SavingsSnapshot {
  compact_bytes?: number;
  full_bytes?: number;
  saved_bytes?: number;
  saved_ratio?: number;
  calls?: number;
  hits?: number;
  bounces?: number;
  hit_rate?: number;
  bounce_rate?: number;
  wakeup_tokens?: number;
  recall_tokens?: number;
}
interface MetricsResponse {
  savings?: SavingsSnapshot;
  usd_saved?: number;
}

function formatBytes(n: number): string {
  if (!n || n < 1024) return `${n || 0} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function sevenDayCutoff(): number {
  return Date.now() - 7 * 24 * 60 * 60 * 1000;
}

function fourteenDayCutoff(): number {
  return Date.now() - 14 * 24 * 60 * 60 * 1000;
}

export function TokensSavedHeadline({ className }: { className?: string }) {
  const metrics = useQuery({
    queryKey: ["dashboard", "metrics"],
    queryFn: () => getJSON<MetricsResponse>("/api/metrics"),
    refetchInterval: 30_000,
  });
  const ledger = useLedgerQuery(1000);

  const { current, prior, delta, deltaPct } = useMemo(() => {
    if (!ledger.data) return { current: 0, prior: 0, delta: 0, deltaPct: 0 };
    const week = sevenDayCutoff();
    const fortnight = fourteenDayCutoff();
    let cur = 0;
    let pri = 0;
    for (const e of ledger.data) {
      const t = new Date(e.ts).getTime();
      const saved = Math.max(0, e.bytes_in - e.bytes_out);
      if (t >= week) cur += saved;
      else if (t >= fortnight) pri += saved;
    }
    const d = cur - pri;
    const pct = pri > 0 ? (d / pri) * 100 : cur > 0 ? 100 : 0;
    return { current: cur, prior: pri, delta: d, deltaPct: pct };
  }, [ledger.data]);

  const headline = metrics.data?.savings?.saved_bytes ?? 0;
  const display = Math.max(current, headline);
  const direction = delta > 0 ? "up" : delta < 0 ? "down" : "flat";
  const Icon = direction === "up" ? ArrowUpRight : direction === "down" ? ArrowDownRight : Minus;
  const tint =
    direction === "up"
      ? "text-[hsl(var(--color-positive))]"
      : direction === "down"
        ? "text-[hsl(var(--color-danger))]"
        : "text-muted-foreground";

  return (
    <div className={className}>
      <p className="text-xs uppercase tracking-wide text-muted-foreground">Bytes saved · last 7 days</p>
      <p className="mt-1 font-mono text-3xl font-semibold tabular-nums tracking-tight">
        {formatBytes(display)}
      </p>
      <p className={`mt-1 flex items-center gap-1 text-xs ${tint}`}>
        <Icon className="size-3.5" aria-hidden="true" />
        <span className="font-mono">
          {delta >= 0 ? "+" : ""}
          {formatBytes(Math.abs(delta))}
        </span>
        <span className="text-muted-foreground">
          ({deltaPct >= 0 ? "+" : ""}
          {deltaPct.toFixed(0)}%) vs prior 7 days
        </span>
      </p>
    </div>
  );
}
