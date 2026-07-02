"use client";

import { useState } from "react";
import { useCompressionDemoQuery } from "@/lib/queries";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import { Beaker, Loader2, Sparkles } from "lucide-react";
import { cn } from "@/lib/utils";
import type { CompressionDemo } from "@/lib/api";

function fmtTokens(n: number): string {
  if (n < 1000) return `${n}`;
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

function statusColor(status: string): string {
  if (status === "Full") return "bg-muted text-muted-foreground";
  if (status === "Cached") return "bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200";
  if (status === "Diff") return "bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200";
  if (status === "Outline") return "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200";
  return "bg-muted text-muted-foreground";
}

export default function CompressionPage() {
  const [path, setPath] = useState("");
  const [submittedPath, setSubmittedPath] = useState<string | null>(null);
  const demo = useCompressionDemoQuery(submittedPath);

  function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setSubmittedPath(path.trim() || null);
  }

  return (
    <div className="space-y-6">
      <Card className="p-5">
        <CardHeader className="p-0 flex flex-row items-center justify-between">
          <CardTitle className="text-sm font-semibold tracking-tight flex items-center gap-2">
            <Beaker className="h-4 w-4" />
            Compression Lab
            <HelpButton content={HELP["/memory/compression"]} />
          </CardTitle>
        </CardHeader>
        <CardContent className="p-0 pt-4 space-y-3">
          <p className="text-sm text-muted-foreground">
            Pick a file and see all four read modes side-by-side. Use this to understand
            which files compress well and which need a full read.
          </p>
          <form onSubmit={onSubmit} className="flex gap-2">
            <Input
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="crates/cairn-core/src/lib.rs"
              className="font-mono"
            />
            <Button type="submit" disabled={!path.trim()}>
              Render
            </Button>
          </form>
        </CardContent>
      </Card>

      {submittedPath && (
        <KpiStrip demo={demo.data} loading={demo.isLoading} error={demo.error} />
      )}

      {submittedPath && demo.data && (
        <ModeGrid demo={demo.data} />
      )}

      {submittedPath && demo.isLoading && (
        <SkeletonRows />
      )}

      {!submittedPath && <EmptyState />}
    </div>
  );
}

function KpiStrip({
  demo,
  loading,
  error,
}: {
  demo: CompressionDemo | undefined;
  loading: boolean;
  error: Error | null;
}) {
  if (loading) {
    return (
      <div className="grid gap-4 md:grid-cols-3">
        <Skeleton className="h-24" />
        <Skeleton className="h-24" />
        <Skeleton className="h-24" />
      </div>
    );
  }
  if (error) {
    return (
      <Card className="p-4 border-red-300">
        <p className="text-sm text-red-700 dark:text-red-300">
          Failed to render: {error.message}
        </p>
      </Card>
    );
  }
  if (!demo) return null;
  return (
    <div className="grid gap-4 md:grid-cols-3">
      <Kpi label="Raw tokens (full)" value={fmtTokens(demo.raw_tokens)} />
      <Kpi
        label="Best mode"
        value={demo.best_mode}
        sub={`saved ${Math.round(demo.savings_ratio * 100)}%`}
      />
      <Kpi
        label="Tokens saved (vs full)"
        value={fmtTokens(demo.total_savings_tokens)}
        sub={`${demo.raw_lines} lines . ${fmtTokens(demo.raw_bytes)}b`}
      />
    </div>
  );
}

function Kpi({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <Card className="p-4">
      <p className="text-xs uppercase tracking-wide text-muted-foreground">{label}</p>
      <p className="mt-1 font-mono text-2xl font-semibold tabular-nums tracking-tight">
        {value}
      </p>
      {sub && <p className="mt-1 text-xs text-muted-foreground">{sub}</p>}
    </Card>
  );
}

function ModeGrid({ demo }: { demo: CompressionDemo }) {
  return (
    <div className="grid gap-4 lg:grid-cols-2 xl:grid-cols-4">
      {demo.views.map((v) => (
        <Card
          key={v.mode}
          className={cn(
            "p-4 flex flex-col",
            v.mode === demo.best_mode && "ring-2 ring-[hsl(var(--color-positive))]",
          )}
        >
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold">{v.mode}</span>
              {v.mode === demo.best_mode && (
                <Badge variant="outline" className="text-[10px] border-green-500 text-green-700 dark:text-green-300">
                  <Sparkles className="h-2.5 w-2.5 mr-0.5" />
                  best
                </Badge>
              )}
            </div>
            <Badge className={cn("text-[10px]", statusColor(v.status))}>
              {v.status}
            </Badge>
          </div>
          <div className="mt-2 text-xs text-muted-foreground">
            {fmtTokens(v.est_tokens)} tokens
            {v.est_tokens > 0 && demo.raw_tokens > 0 && (
              <span className="ml-1">
                ({Math.round(v.savings_vs_full * 100)}% vs full)
              </span>
            )}
          </div>
          {v.note && (
            <p className="mt-1 text-[10px] text-muted-foreground italic">{v.note}</p>
          )}
          <pre className="mt-3 flex-1 overflow-auto rounded bg-muted/40 p-2 text-[11px] font-mono whitespace-pre-wrap break-all max-h-96">
            {v.view || <span className="text-muted-foreground">(empty)</span>}
          </pre>
        </Card>
      ))}
    </div>
  );
}

function SkeletonRows() {
  return (
    <div className="grid gap-4 lg:grid-cols-2 xl:grid-cols-4">
      {[0, 1, 2, 3].map((i) => (
        <Skeleton key={i} className="h-80" />
      ))}
    </div>
  );
}

function EmptyState() {
  return (
    <Card className="p-8 text-center">
      <Loader2 className="h-6 w-6 mx-auto text-muted-foreground" />
      <p className="mt-3 text-sm text-muted-foreground">
        Type a file path above and press Render. Try a Rust source file for a good demo of
        signatures vs full.
      </p>
    </Card>
  );
}