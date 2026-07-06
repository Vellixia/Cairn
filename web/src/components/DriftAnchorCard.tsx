"use client";

import { useAnchorQuery } from "@/lib/queries";
import { useQuery } from "@tanstack/react-query";
import { getJSON, type Stats } from "@/lib/api";
import { Anchor, ShieldAlert, Target } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import Link from "next/link";

export function DriftAnchorCard() {
  const anchorQ = useAnchorQuery();
  const statsQ = useQuery({
    queryKey: ["drift-anchor", "stats"],
    queryFn: () => getJSON<Stats>("/api/stats"),
  });

  const currentAnchor = anchorQ.data?.anchor ?? null;
  const reliability = statsQ.data?.reliability;
  const reliabilityPct = reliability ? Math.round(reliability.score) : null;

  const reliabilityTone =
    reliabilityPct == null
      ? "text-muted-foreground"
      : reliabilityPct >= 90
        ? "text-[hsl(var(--color-positive))]"
        : reliabilityPct >= 70
          ? "text-[hsl(var(--color-warning))]"
          : "text-[hsl(var(--color-danger))]";

  return (
    <Card className="p-5">
      <CardHeader className="flex flex-row items-center justify-between gap-2 space-y-0 p-0">
        <div className="flex items-center gap-2">
          <Target className="size-4 text-muted-foreground" aria-hidden="true" />
          <CardTitle className="text-sm font-semibold tracking-tight">Anchor &amp; drift</CardTitle>
        </div>
        {reliabilityPct != null ? (
          <span className={cn("text-xs font-medium", reliabilityTone)}>
            {reliabilityPct}% reliable
          </span>
        ) : null}
      </CardHeader>
      <CardContent className="space-y-3 p-0 pt-4">
        <div className="flex items-start gap-2">
          <Anchor className="mt-2.5 size-3.5 text-muted-foreground" aria-hidden="true" />
          <div className="min-w-0 flex-1 space-y-2">
            {anchorQ.isLoading ? (
              <Skeleton className="h-4 w-3/4 bg-muted" />
            ) : currentAnchor ? (
              <p className="break-words text-sm text-foreground">{currentAnchor}</p>
            ) : (
              <p className="text-sm text-muted-foreground">
                No anchor set yet --- the agent sets one via the `anchor` tool, or it&apos;s
                auto-derived from the first prompt of a session.
              </p>
            )}
          </div>
        </div>
        {reliability ? (
          <div className="grid grid-cols-3 gap-2 rounded-md border border-line/50 bg-muted/30 p-2 text-center text-xs">
            <div>
              <p className="text-base font-semibold">{reliability.ok}</p>
              <p className="text-muted-foreground">OK</p>
            </div>
            <div>
              <p className="text-base font-semibold text-[hsl(var(--color-warning))]">
                {reliability.warn}
              </p>
              <p className="text-muted-foreground">Warn</p>
            </div>
            <div>
              <p className="text-base font-semibold text-[hsl(var(--color-danger))]">
                {reliability.danger}
              </p>
              <p className="text-muted-foreground">Danger</p>
            </div>
          </div>
        ) : null}
        <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
          <span className="flex items-center gap-1">
            <ShieldAlert className="size-3" aria-hidden="true" />
            {reliability
              ? `${reliability.samples} reliability samples, ${reliability.rollbacks} rollbacks`
              : "Reliability scoring off until a checkpoint exists"}
          </span>
          <Link
            href="/automation"
            className="text-[hsl(var(--color-info))] hover:underline"
          >
            Guard log
          </Link>
        </div>
      </CardContent>
    </Card>
  );
}
