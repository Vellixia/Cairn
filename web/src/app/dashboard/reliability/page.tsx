"use client";

import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useStatsQuery } from "@/lib/queries";

export default function ReliabilityScorePage() {
  const stats = useStatsQuery();
  const rel = stats.data?.reliability;

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Reliability</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          How well Cairn has been guarding your edits. Updates after every
          verify/checkpoint/rollback.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Score</CardTitle>
          <CardDescription>
            Live score, updated every 10 seconds.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {stats.isLoading ? (
            <Skeleton className="h-16 w-32" />
          ) : !rel ? (
            <p className="text-sm text-muted-foreground">
              No edit history yet. Run <code>cairn-cli verify</code> or call{" "}
              <code>/api/guard/verify</code> to seed the score.
            </p>
          ) : (
            <>
              <div
                className={`text-6xl font-bold ${
                  rel.score >= 80
                    ? "text-emerald-500"
                    : rel.score >= 50
                    ? "text-amber-500"
                    : "text-destructive"
                }`}
              >
                {rel.score}
                <span className="text-lg text-muted-foreground">/100</span>
              </div>
              <dl className="mt-4 grid grid-cols-2 sm:grid-cols-5 gap-3 text-sm">
                <Cell label="samples" value={rel.samples} />
                <Cell
                  label="ok"
                  value={rel.ok}
                  accent="text-emerald-500"
                />
                <Cell
                  label="warn"
                  value={rel.warn}
                  accent="text-amber-500"
                />
                <Cell
                  label="danger"
                  value={rel.danger}
                  accent="text-destructive"
                />
                <Cell label="rollbacks" value={rel.rollbacks} />
              </dl>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function Cell({
  label,
  value,
  accent,
}: {
  label: string;
  value: number;
  accent?: string;
}) {
  return (
    <div className="rounded-md bg-secondary px-3 py-2">
      <div className="text-[10px] uppercase tracking-wider text-muted-foreground">
        {label}
      </div>
      <div className={`font-mono ${accent ?? ""}`}>{value}</div>
    </div>
  );
}
