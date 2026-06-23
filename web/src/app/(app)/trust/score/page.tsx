"use client";

import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import { useStatsQuery } from "@/lib/queries";
import { cn } from "@/lib/utils";

function scoreTone(score: number): string {
  if (score >= 80) return "text-emerald-500";
  if (score >= 50) return "text-amber-500";
  return "text-red-500";
}

export default function ReliabilityScorePage() {
  const stats = useStatsQuery();
  const rel = stats.data?.reliability;

  return (
    <div className="space-y-6 max-w-3xl">
      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Reliability score</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            How well Cairn has been guarding your edits. Updated every 10 seconds.
          </p>
        </div>
        <HelpButton content={HELP["/trust"]} />
      </header>

      {stats.isLoading ? (
        <Skeleton className="h-32 w-full" />
      ) : !rel ? (
        <Card>
          <CardContent className="pt-6">
            <p className="text-sm text-muted-foreground">
              No edit history yet. Run <code>cairn verify</code> or call
              <code className="ml-1">/api/guard/verify</code> to seed the score.
            </p>
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardHeader>
            <CardTitle>Score</CardTitle>
            <CardDescription>
              Live score from {rel.samples} sample{rel.samples === 1 ? "" : "s"}.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className={cn("text-7xl font-bold tabular-nums", scoreTone(rel.score))}>
              {rel.score}
              <span className="ml-1 text-base font-medium text-muted-foreground">/100</span>
            </div>
            <div className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-4">
              <Stat label="ok" value={rel.ok} tone="text-emerald-500" />
              <Stat label="warn" value={rel.warn} tone="text-amber-500" />
              <Stat label="danger" value={rel.danger} tone="text-red-500" />
              <Stat label="rollbacks" value={rel.rollbacks} />
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

function Stat({
  label,
  value,
  tone,
}: {
  label: string;
  value: number;
  tone?: string;
}) {
  return (
    <div className="rounded-lg border border-line bg-card px-4 py-3">
      <div className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
        {label}
      </div>
      <div className={cn("mt-1 font-mono text-2xl tabular-nums", tone ?? "text-foreground")}>
        {value}
      </div>
    </div>
  );
}