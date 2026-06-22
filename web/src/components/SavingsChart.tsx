"use client";

import { useQuery } from "@tanstack/react-query";
import { getJSON } from "@/lib/api";
import { Area, AreaChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ChartContainer, ChartTooltipContent } from "@/components/ui/chart";
import { PiggyBank } from "lucide-react";
import { useMemo } from "react";

interface LedgerEntry {
  id: number;
  ts: string;
  source: string;
  bytes_in: number;
  bytes_out: number;
  tokens_saved: number;
  cost_usd_saved: number;
}

interface DayBucket {
  day: string;
  tokens_saved: number;
  cost_usd_saved: number;
  bytes_in: number;
  bytes_out: number;
  entries: number;
}

const ROLLING_DAYS = 7;

function bucketByDay(entries: LedgerEntry[]): DayBucket[] {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const map = new Map<string, DayBucket>();
  for (let i = ROLLING_DAYS - 1; i >= 0; i--) {
    const d = new Date(today);
    d.setDate(today.getDate() - i);
    const key = d.toISOString().slice(0, 10);
    map.set(key, {
      day: key,
      tokens_saved: 0,
      cost_usd_saved: 0,
      bytes_in: 0,
      bytes_out: 0,
      entries: 0,
    });
  }
  for (const e of entries) {
    const key = e.ts.slice(0, 10);
    const bucket = map.get(key);
    if (!bucket) continue;
    bucket.tokens_saved += e.tokens_saved;
    bucket.cost_usd_saved += e.cost_usd_saved;
    bucket.bytes_in += e.bytes_in;
    bucket.bytes_out += e.bytes_out;
    bucket.entries += 1;
  }
  return Array.from(map.values());
}

function fmtNumber(n: number): string {
  if (Math.abs(n) >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (Math.abs(n) >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return Math.round(n).toString();
}

function fmtDay(iso: string): string {
  const d = new Date(iso + "T00:00:00Z");
  return d.toLocaleDateString(undefined, { weekday: "short" });
}

function fmtUsd(n: number): string {
  if (n < 0.01 && n > 0) return "<$0.01";
  if (n === 0) return "$0";
  return `$${n.toFixed(2)}`;
}

export function SavingsChart() {
  const ledgerQ = useQuery({
    queryKey: ["savings", "ledger", ROLLING_DAYS],
    queryFn: () => getJSON<LedgerEntry[]>(`/api/ledger?limit=500`),
  });

  const buckets = useMemo(() => {
    if (!ledgerQ.data) return [];
    return bucketByDay(ledgerQ.data);
  }, [ledgerQ.data]);

  const totals = useMemo(() => {
    return buckets.reduce(
      (acc, b) => {
        acc.tokens += b.tokens_saved;
        acc.cost += b.cost_usd_saved;
        acc.entries += b.entries;
        return acc;
      },
      { tokens: 0, cost: 0, entries: 0 },
    );
  }, [buckets]);

  const chartConfig = {
    tokens_saved: { label: "Tokens saved", color: "hsl(24 95% 53%)" },
  } as const;

  return (
    <Card className="p-5">
      <CardHeader className="flex flex-row items-start justify-between gap-2 space-y-0 p-0">
        <div className="flex items-center gap-2">
          <PiggyBank className="size-4 text-muted-foreground" aria-hidden="true" />
          <CardTitle className="text-sm font-semibold tracking-tight">
            Token savings (7-day)
          </CardTitle>
        </div>
        <div className="text-right text-xs text-muted-foreground">
          <p>
            <span className="font-medium text-foreground">
              {fmtNumber(totals.tokens)}
            </span>{" "}
            tokens
          </p>
          <p>
            <span className="font-medium text-foreground">{fmtUsd(totals.cost)}</span>{" "}
            saved
          </p>
        </div>
      </CardHeader>
      <CardContent className="p-0 pt-4">
        {ledgerQ.isLoading ? (
          <div className="cairn-skeleton h-48" aria-label="Loading savings chart" />
        ) : ledgerQ.isError || buckets.every((b) => b.entries === 0) ? (
          <div className="flex h-48 flex-col items-center justify-center gap-1 text-center">
            <PiggyBank className="size-6 text-muted-foreground/50" aria-hidden="true" />
            <p className="text-sm font-medium">No savings recorded yet</p>
            <p className="max-w-xs text-xs text-muted-foreground">
              As you read, recall, and assemble context, Cairn logs the byte
              savings into a tamper-evident ledger. Recent entries appear here.
            </p>
          </div>
        ) : (
          <ChartContainer config={chartConfig} className="h-48 w-full">
            <AreaChart data={buckets} margin={{ left: 4, right: 4, top: 4, bottom: 0 }}>
              <defs>
                <linearGradient id="fillTokens" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="5%" stopColor="hsl(24 95% 53%)" stopOpacity={0.5} />
                  <stop offset="95%" stopColor="hsl(24 95% 53%)" stopOpacity={0.05} />
                </linearGradient>
              </defs>
              <CartesianGrid vertical={false} stroke="hsl(var(--border))" strokeDasharray="3 3" />
              <XAxis
                dataKey="day"
                tickFormatter={fmtDay}
                stroke="hsl(var(--muted-foreground))"
                fontSize={11}
                tickLine={false}
                axisLine={false}
              />
              <YAxis
                tickFormatter={fmtNumber}
                stroke="hsl(var(--muted-foreground))"
                fontSize={11}
                tickLine={false}
                axisLine={false}
                width={36}
              />
              <Tooltip
                content={
                  <ChartTooltipContent
                    labelFormatter={(v) => fmtDay(String(v))}
                    formatter={(value) => (
                      <span className="font-mono">{fmtNumber(Number(value))}</span>
                    )}
                  />
                }
              />
              <Area
                type="monotone"
                dataKey="tokens_saved"
                stroke="hsl(24 95% 53%)"
                fill="url(#fillTokens)"
                strokeWidth={2}
              />
            </AreaChart>
          </ChartContainer>
        )}
      </CardContent>
    </Card>
  );
}
