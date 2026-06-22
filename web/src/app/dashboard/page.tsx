"use client";

import Link from "next/link";
import {
  useStatsQuery,
  useWakeupQuery,
  useAnchorQuery,
  useDevicesTokensQuery,
} from "@/lib/queries";
import { getJSON } from "@/lib/api";
import { useQuery } from "@tanstack/react-query";
import { KpiCard } from "@/components/KpiCard";
import { HealthRow } from "@/components/HealthRow";
import { ActivityTimeline } from "@/components/ActivityTimeline";
import { SavingsChart } from "@/components/SavingsChart";
import { DriftAnchorCard } from "@/components/DriftAnchorCard";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Item, ItemContent, ItemTitle, ItemDescription } from "@/components/ui/item";
import { Brain, Plug, ShieldCheck, Network } from "lucide-react";

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

export default function DashboardOverviewPage() {
  const stats = useStatsQuery();
  const memories = useWakeupQuery(5);
  const anchor = useAnchorQuery();
  const devices = useDevicesTokensQuery();
  const metrics = useQuery({
    queryKey: ["dashboard", "metrics"],
    queryFn: () => getJSON<MetricsResponse>("/api/metrics"),
    refetchInterval: 30_000,
  });
  const rel = stats.data?.reliability;
  const tokensSaved = metrics.data?.savings?.wakeup_tokens ?? 0;
  const tokensSavedRecall = metrics.data?.savings?.recall_tokens ?? 0;
  const tokensSavedTotal = tokensSaved + tokensSavedRecall;
  const activeDeviceCount = devices.data?.length ?? null;

  return (
    <div className="space-y-6">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight">Overview</h1>
        <p className="text-sm text-muted-foreground">
          Server health, reliability, recent memory, and the last few admin actions — at a glance.
        </p>
      </header>

      <section
        aria-label="Key performance indicators"
        className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4"
      >
        <KpiCard
          label="Memories"
          value={stats.data ? stats.data.memories : null}
          href="/dashboard/memory"
          icon={Brain}
          hint={
            anchor.data?.anchor
              ? "Stored under current anchor"
              : "Set an anchor to scope recall"
          }
          tone={anchor.data?.anchor ? "positive" : "neutral"}
        />
        <KpiCard
          label="Reliability"
          value={rel ? rel.score : null}
          suffix={rel ? "/100" : undefined}
          href="/dashboard/reliability"
          icon={ShieldCheck}
          hint={
            rel
              ? `${rel.samples} samples · ${rel.ok} ok · ${rel.warn} warn`
              : "No edit history yet"
          }
          tone={
            !rel
              ? "neutral"
              : rel.score >= 90
                ? "positive"
                : rel.score >= 70
                  ? "warning"
                  : "danger"
          }
        />
        <KpiCard
          label="Token savings"
          value={metrics.data ? tokensSavedTotal : null}
          icon={Plug}
          hint={
            metrics.data
              ? `Wakeup ${tokensSaved.toLocaleString()} · Recall ${tokensSavedRecall.toLocaleString()}`
              : "Last 7 days · see chart"
          }
          tone={tokensSavedTotal > 0 ? "positive" : "info"}
        />
        <KpiCard
          label="Active devices"
          value={activeDeviceCount}
          href="/dashboard/devices"
          icon={Network}
          hint="Issued device tokens"
          tone={activeDeviceCount && activeDeviceCount > 0 ? "positive" : "neutral"}
        />
      </section>

      <HealthRow />

      <section className="grid gap-4 lg:grid-cols-2">
        <ActivityTimeline limit={8} />
        <SavingsChart />
      </section>

      <DriftAnchorCard />

      <Card className="p-5">
        <CardHeader className="p-0">
          <CardTitle className="text-sm font-semibold tracking-tight">Recent memory</CardTitle>
        </CardHeader>
        <CardContent className="p-0 pt-4">
          {memories.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : memories.data && memories.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No memories yet.{" "}
              <Link
                href="/dashboard/memory"
                className="text-[hsl(var(--color-info))] hover:underline"
              >
                Capture the first one →
              </Link>
            </p>
          ) : memories.data ? (
            <ul className="space-y-1.5">
              {memories.data.slice(0, 5).map((m) => (
                <Item key={m.id} variant="outline" size="sm">
                  <ItemContent>
                    <ItemTitle className="line-clamp-2">{m.content}</ItemTitle>
                    <ItemDescription>
                      <Badge variant="outline" className="mr-1.5 font-mono text-[10px]">
                        {m.kind}
                      </Badge>
                      {m.tier} · {new Date(m.created_at).toLocaleString()}
                    </ItemDescription>
                  </ItemContent>
                </Item>
              ))}
            </ul>
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}
