"use client";

import { useMemo } from "react";
import { PieChart, Pie, Cell, ResponsiveContainer, Legend, Tooltip } from "recharts";
import { useWakeupQuery } from "@/lib/queries";

const TIER_COLORS: Record<string, string> = {
  working: "hsl(var(--color-info))",
  short_term: "hsl(var(--color-warning))",
  long_term: "hsl(var(--color-positive))",
  pinned: "hsl(var(--color-danger))",
};

export function MemoryTierDonut({ className }: { className?: string }) {
  const { data, isLoading } = useWakeupQuery(200);
  const buckets = useMemo(() => {
    if (!data) return [] as { name: string; value: number }[];
    const counts: Record<string, number> = {};
    for (const m of data) {
      counts[m.tier] = (counts[m.tier] ?? 0) + 1;
    }
    return Object.entries(counts)
      .map(([name, value]) => ({ name, value }))
      .sort((a, b) => b.value - a.value);
  }, [data]);

  if (isLoading) {
    return (
      <div className={className} aria-hidden="true">
        <div className="mx-auto h-32 w-32 animate-pulse rounded-full bg-muted" />
      </div>
    );
  }
  if (buckets.length === 0) {
    return (
      <div className={className}>
        <p className="text-xs text-muted-foreground">No memory samples yet.</p>
      </div>
    );
  }
  const total = buckets.reduce((s, b) => s + b.value, 0);
  return (
    <div className={className}>
      <div className="mx-auto h-32 w-32">
        <ResponsiveContainer width="100%" height="100%">
          <PieChart>
            <Pie
              data={buckets}
              dataKey="value"
              nameKey="name"
              innerRadius={28}
              outerRadius={56}
              stroke="hsl(var(--background))"
              strokeWidth={2}
              isAnimationActive={false}
            >
              {buckets.map((b) => (
                <Cell key={b.name} fill={TIER_COLORS[b.name] ?? "hsl(var(--muted-foreground))"} />
              ))}
            </Pie>
            <Tooltip
              contentStyle={{
                background: "hsl(var(--popover))",
                border: "1px solid hsl(var(--border))",
                fontSize: 12,
              }}
              formatter={(value: number) => `${value} of ${total}`}
            />
          </PieChart>
        </ResponsiveContainer>
      </div>
      <ul className="mt-2 flex flex-wrap justify-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
        {buckets.map((b) => (
          <li key={b.name} className="flex items-center gap-1.5">
            <span
              className="inline-block h-2 w-2 rounded-full"
              style={{ background: TIER_COLORS[b.name] ?? "hsl(var(--muted-foreground))" }}
              aria-hidden="true"
            />
            <span className="capitalize">{b.name.replace("_", " ")}</span>
            <span className="font-mono text-foreground/80">{b.value}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
