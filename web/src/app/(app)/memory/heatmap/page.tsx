"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useHeatmapQuery } from "@/lib/queries";
import { useMemo, useState } from "react";

const LEVEL_COLORS = [
  "bg-zinc-100 dark:bg-zinc-800",
  "bg-emerald-200 dark:bg-emerald-900",
  "bg-emerald-400 dark:bg-emerald-700",
  "bg-emerald-600 dark:bg-emerald-500",
  "bg-emerald-800 dark:bg-emerald-300",
];

const DAY_LABELS = ["Mon", "", "Wed", "", "Fri", "", ""];

function getLevel(count: number, maxCount: number): number {
  if (count === 0) return 0;
  if (count <= maxCount * 0.25) return 1;
  if (count <= maxCount * 0.5) return 2;
  if (count <= maxCount * 0.75) return 3;
  return 4;
}

function generateGrid(data: Record<string, number>) {
  const now = new Date();
  const cells: { date: string; count: number; level: number; day: number; week: number }[] = [];

  const end = new Date(now);
  // Move to end of current week (Sunday)
  end.setDate(end.getDate() + (6 - end.getDay()));

  const start = new Date(end);
  start.setDate(start.getDate() - 364); // 52 weeks * 7 days - 1
  // Move to start of the week (Monday)
  start.setDate(start.getDate() - start.getDay() + 1);

  const maxCount = Math.max(1, ...Object.values(data));

  const cursor = new Date(start);
  let week = 0;
  while (cursor <= end) {
    const dayOfWeek = cursor.getDay(); // 0=Sun, 1=Mon, ...
    const dateStr = cursor.toISOString().slice(0, 10);
    const count = data[dateStr] ?? 0;
    cells.push({
      date: dateStr,
      count,
      level: getLevel(count, maxCount),
      day: dayOfWeek,
      week,
    });
    cursor.setDate(cursor.getDate() + 1);
    if (dayOfWeek === 6) week++;
  }

  return { cells, monthLabels: buildMonthLabels(start, end) };
}

function buildMonthLabels(start: Date, end: Date) {
  const labels: { label: string; week: number }[] = [];
  const cursor = new Date(start);
  let lastMonth = -1;
  let week = 0;
  while (cursor <= end) {
    if (cursor.getMonth() !== lastMonth) {
      labels.push({ label: cursor.toLocaleString("default", { month: "short" }), week });
      lastMonth = cursor.getMonth();
    }
    cursor.setDate(cursor.getDate() + 1);
    if (cursor.getDay() === 1) week++;
  }
  return labels;
}

export default function HeatmapPage() {
  const query = useHeatmapQuery();
  const [tooltip, setTooltip] = useState<{ date: string; count: number } | null>(null);

  const grid = useMemo(() => (query.data ? generateGrid(query.data) : null), [query.data]);

  const total = useMemo(
    () =>
      query.data
        ? Object.values(query.data).reduce((a, b) => a + b, 0)
        : 0,
    [query.data],
  );

  return (
    <div className="space-y-6 max-w-4xl">
      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Activity</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Daily memory creation over the last 52 weeks.
          </p>
        </div>
        <HelpButton content={HELP["/memory/heatmap"]} />
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Heatmap</CardTitle>
          <CardDescription>
            {query.isLoading
              ? "Loading..."
              : `${total} memories in the last 365 days.`}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {query.isLoading ? (
            <Skeleton className="h-32 w-full" />
          ) : grid ? (
            <div className="overflow-x-auto">
              <div className="inline-flex flex-col gap-1">
                {/* Month labels */}
                <div
                  className="relative h-4 text-[10px] text-muted-foreground"
                  style={{ marginLeft: "32px" }}
                >
                  {grid.monthLabels.map((ml, i) => (
                    <span
                      key={i}
                      className="absolute"
                      style={{ left: `${ml.week * 14}px` }}
                    >
                      {ml.label}
                    </span>
                  ))}
                </div>

                <div className="flex gap-0.5">
                  {/* Day labels */}
                  <div className="flex flex-col gap-0.5 pr-1.5 pt-0.5">
                    {DAY_LABELS.map((label, i) => (
                      <div
                        key={i}
                        className="h-[10px] text-[10px] leading-[10px] text-muted-foreground"
                      >
                        {label}
                      </div>
                    ))}
                  </div>

                  {/* Grid */}
                  <div className="flex flex-col flex-wrap gap-0.5" style={{ height: "7 * 12px + 6 * 2px" }}>
                    {grid.cells.map((cell, i) => (
                      <div
                        key={i}
                        className={`h-[10px] w-[10px] rounded-[2px] ${LEVEL_COLORS[cell.level]} cursor-pointer`}
                        onMouseEnter={() =>
                          setTooltip({ date: cell.date, count: cell.count })
                        }
                        onMouseLeave={() => setTooltip(null)}
                      />
                    ))}
                  </div>
                </div>

                {/* Tooltip */}
                {tooltip && (
                  <p className="mt-2 text-xs text-muted-foreground">
                    <span className="font-medium">{tooltip.date}</span> —{" "}
                    {tooltip.count} memory{tooltip.count === 1 ? "" : "ies"}
                  </p>
                )}

                {/* Legend */}
                <div className="mt-3 flex items-center gap-1.5 text-[10px] text-muted-foreground">
                  <span>Less</span>
                  {LEVEL_COLORS.map((color, i) => (
                    <div
                      key={i}
                      className={`h-[10px] w-[10px] rounded-[2px] ${color}`}
                    />
                  ))}
                  <span>More</span>
                </div>
              </div>
            </div>
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}
