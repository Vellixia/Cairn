"use client";

import { useQuery } from "@tanstack/react-query";
import { getJSON } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Gauge } from "lucide-react";
import { cn } from "@/lib/utils";

interface LedgerEntry {
  path: string;
  mode: string;
  sent_tokens: number;
  phi: number;
  pinned: boolean;
}

interface ContextPressure {
  utilization: number;
  remaining_tokens: number;
  entries_count: number;
  recommendation: string;
  eviction_candidates: LedgerEntry[];
}

const RECOMMENDATION_COLOR: Record<string, string> = {
  NoAction: "bg-muted text-muted-foreground",
  SuggestCompression: "bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200",
  ForceCompression: "bg-orange-100 text-orange-800 dark:bg-orange-900 dark:text-orange-200",
  EvictLeastRelevant: "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200",
};

const RECOMMENDATION_LABEL: Record<string, string> = {
  NoAction: "All good",
  SuggestCompression: "Suggest compression",
  ForceCompression: "Force compression",
  EvictLeastRelevant: "Evict least relevant",
};

function utilizationColor(ratio: number): string {
  if (ratio > 0.9) return "stroke-red-500";
  if (ratio > 0.75) return "stroke-orange-500";
  if (ratio > 0.5) return "stroke-yellow-500";
  return "stroke-green-500";
}

function utilizationBg(ratio: number): string {
  if (ratio > 0.9) return "text-red-600";
  if (ratio > 0.75) return "text-orange-600";
  if (ratio > 0.5) return "text-yellow-600";
  return "text-green-600";
}

export function ContextPressureGauge() {
  const p = useQuery({
    queryKey: ["context", "pressure"],
    queryFn: () => getJSON<ContextPressure>("/api/context/pressure"),
    refetchInterval: 30_000,
  });

  if (p.isLoading) return null;

  const pressure = p.data;
  if (!pressure) return null;

  const ratio = pressure.utilization;
  const r = 36;
  const circumference = 2 * Math.PI * r;
  const offset = circumference * (1 - ratio);

  return (
    <Card className="p-5">
      <CardHeader className="p-0">
        <CardTitle className="text-sm font-semibold tracking-tight flex items-center gap-2">
          <Gauge className="h-4 w-4" />
          Context pressure
        </CardTitle>
      </CardHeader>
      <CardContent className="p-0 pt-4">
        <div className="flex items-center gap-4">
          <div className="relative flex items-center justify-center">
            <svg width="96" height="96" viewBox="0 0 96 96" className="transform -rotate-90">
              <circle
                cx="48"
                cy="48"
                r={r}
                fill="none"
                stroke="currentColor"
                strokeWidth="6"
                className="text-muted"
              />
              <circle
                cx="48"
                cy="48"
                r={r}
                fill="none"
                strokeWidth="6"
                strokeLinecap="round"
                strokeDasharray={circumference}
                strokeDashoffset={offset}
                className={utilizationColor(ratio)}
              />
            </svg>
            <span className={cn("absolute text-lg font-bold", utilizationBg(ratio))}>
              {Math.round(ratio * 100)}%
            </span>
          </div>
          <div className="space-y-1 text-sm">
            <div className="flex items-center gap-2">
              <Badge className={cn("text-[10px]", RECOMMENDATION_COLOR[pressure.recommendation] || "")}>
                {RECOMMENDATION_LABEL[pressure.recommendation] || pressure.recommendation}
              </Badge>
            </div>
            <p className="text-muted-foreground text-xs">
              {pressure.remaining_tokens.toLocaleString()} tokens free
            </p>
            <p className="text-muted-foreground text-xs">
              {pressure.entries_count} entries tracked
            </p>
          </div>
        </div>

        {pressure.eviction_candidates.length > 0 && (
          <div className="mt-4 space-y-1.5">
            <p className="text-xs font-medium text-muted-foreground">Eviction candidates</p>
            {pressure.eviction_candidates.map((e) => (
              <div key={e.path} className="flex items-center justify-between text-xs">
                <span className="truncate max-w-[200px] font-mono">{e.path}</span>
                <span className="text-muted-foreground">{e.sent_tokens}tok</span>
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
