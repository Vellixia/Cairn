"use client";

import { useState } from "react";
import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import { KpiCard } from "@/components/KpiCard";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Item,
  ItemActions,
  ItemContent,
  ItemTitle,
  ItemDescription,
} from "@/components/ui/item";
import { Sparkles, TrendingDown, ShieldCheck, Play } from "lucide-react";
import {
  useAutopilotDigestQuery,
  useCronJobsQuery,
  useCronHistoryQuery,
  useRunCronJobMutation,
  usePromotionLogQuery,
  useDemoteMemoryMutation,
} from "@/lib/queries";
import type { PromotionLogEntry } from "@/lib/api";

// Friendly labels for cron.rs::JOBS - kept in sync manually since the schedule strings
// themselves are 6-field (seconds-first) cron expressions, not human-readable.
const SCHEDULE_LABELS: Record<string, string> = {
  "session-gc": "Daily 02:00",
  "memory-decay": "Sun 03:00",
  "access-log-prune": "Monthly (1st) 04:00",
  "llm-intelligence": "Daily 03:30",
  "memory-demote": "Daily 04:00",
  tune: "Sun 05:00",
};

const REASON_LABEL: Record<string, string> = {
  "auto-threshold": "Auto (threshold)",
  manual: "Manual",
  "auto-demote": "Auto (demote)",
  "manual-undo": "Manual undo",
};

function formatReason(reason: string): string {
  return REASON_LABEL[reason] ?? reason;
}

export default function AutomationContent() {
  const [hours, setHours] = useState(24);
  const [historyJob, setHistoryJob] = useState<string>("all");

  const digest = useAutopilotDigestQuery(hours);
  const jobs = useCronJobsQuery();
  const history = useCronHistoryQuery(historyJob === "all" ? undefined : historyJob);
  const runJob = useRunCronJobMutation();
  const promotionLog = usePromotionLogQuery(50);
  const demote = useDemoteMemoryMutation();

  return (
    <div className="space-y-6 max-w-3xl">
      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Automation</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            What autopilot did while you were away, the background job schedule, and the
            promotion/demotion trail.
          </p>
        </div>
        <HelpButton content={HELP["/automation"]} />
      </header>

      <Card>
        <CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0">
          <div>
            <CardTitle>While you were away</CardTitle>
            <CardDescription>
              What full-auto promotion, demotion, and drift approval did in the last window.
            </CardDescription>
          </div>
          <Select value={String(hours)} onValueChange={(v) => setHours(Number(v))}>
            <SelectTrigger className="h-8 w-28 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="24">Last 24h</SelectItem>
              <SelectItem value="48">Last 48h</SelectItem>
              <SelectItem value="168">Last 7d</SelectItem>
            </SelectContent>
          </Select>
        </CardHeader>
        <CardContent>
          {digest.isLoading ? (
            <div className="grid grid-cols-3 gap-3">
              <Skeleton className="h-24" />
              <Skeleton className="h-24" />
              <Skeleton className="h-24" />
            </div>
          ) : (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
              <KpiCard
                label="Promoted"
                value={digest.data?.promoted ?? 0}
                icon={Sparkles}
                tone="positive"
              />
              <KpiCard
                label="Demoted"
                value={digest.data?.demoted ?? 0}
                icon={TrendingDown}
                tone="neutral"
              />
              <KpiCard
                label="Drift auto-approved"
                value={digest.data?.drift_auto_approved ?? 0}
                icon={ShieldCheck}
                tone="info"
              />
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Background jobs</CardTitle>
          <CardDescription>
            The fixed set of maintenance jobs and their schedule. Run now triggers the same
            code path the scheduler itself uses.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {jobs.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : (
            <ul className="space-y-2">
              {jobs.data?.map((j) => (
                <Item key={j.name} variant="outline" size="sm">
                  <ItemContent>
                    <ItemTitle className="font-mono text-xs">{j.name}</ItemTitle>
                    <ItemDescription className="flex items-center gap-2 flex-wrap">
                      <Badge variant="outline" className="mr-1.5 font-mono text-[10px]">
                        {SCHEDULE_LABELS[j.name] ?? j.schedule}
                      </Badge>
                      {j.last_run ? (
                        <>
                          <Badge
                            variant={j.last_run.outcome === "ok" ? "outline" : "destructive"}
                            className="font-mono text-[10px] uppercase"
                          >
                            {j.last_run.outcome}
                          </Badge>
                          <span className="text-[11px] text-muted-foreground">
                            {new Date(j.last_run.started_at).toLocaleString()}
                          </span>
                        </>
                      ) : (
                        <span className="text-[11px] text-muted-foreground">
                          Never run since server start
                        </span>
                      )}
                    </ItemDescription>
                  </ItemContent>
                  <ItemActions>
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={runJob.isPending}
                      onClick={() => runJob.mutate(j.name)}
                    >
                      <Play className="size-3.5" aria-hidden="true" />
                      Run now
                    </Button>
                  </ItemActions>
                </Item>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0">
          <div>
            <CardTitle>Run history</CardTitle>
            <CardDescription>
              In-memory since server start --- restarting the server clears it.
            </CardDescription>
          </div>
          <Select value={historyJob} onValueChange={setHistoryJob}>
            <SelectTrigger className="h-8 w-40 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All jobs</SelectItem>
              {Object.keys(SCHEDULE_LABELS).map((name) => (
                <SelectItem key={name} value={name}>
                  {name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </CardHeader>
        <CardContent>
          {history.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          ) : history.data && history.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">No runs recorded yet.</p>
          ) : (
            <ul className="space-y-1.5">
              {history.data
                ?.slice()
                .reverse()
                .map((run, i) => (
                  <li
                    key={`${run.job}-${run.started_at}-${i}`}
                    className="flex items-center gap-2 text-xs"
                  >
                    <Badge
                      variant={run.outcome === "ok" ? "outline" : "destructive"}
                      className="font-mono text-[10px] uppercase"
                    >
                      {run.outcome}
                    </Badge>
                    <span className="font-mono">{run.job}</span>
                    <span className="text-muted-foreground">
                      {new Date(run.started_at).toLocaleString()} . {run.duration_ms}ms
                    </span>
                    <span className="truncate text-muted-foreground">{run.detail}</span>
                  </li>
                ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Promotion log</CardTitle>
          <CardDescription>
            Every promotion and demotion, auto and manual alike, newest first.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {promotionLog.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-10 w-full" />
            </div>
          ) : promotionLog.data && promotionLog.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No promotions or demotions yet.
            </p>
          ) : (
            <ul className="space-y-1.5">
              {promotionLog.data?.map((entry: PromotionLogEntry) => (
                <li
                  key={entry.id}
                  className="flex items-center gap-2 text-xs rounded-md border border-line/40 bg-muted/30 px-3 py-2"
                >
                  <Badge
                    variant={entry.action === "promote" ? "secondary" : "outline"}
                    className="font-mono text-[10px] uppercase"
                  >
                    {entry.action}
                  </Badge>
                  <span className="font-mono truncate max-w-[140px]" title={entry.memory_id}>
                    {entry.memory_id}
                  </span>
                  <span className="text-muted-foreground">{formatReason(entry.reason)}</span>
                  <span className="font-mono text-[10px] text-muted-foreground">
                    {entry.score.toFixed(2)}
                  </span>
                  <span className="text-muted-foreground">
                    {new Date(entry.ts).toLocaleString()}
                  </span>
                  {entry.action === "promote" && (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="ml-auto h-6 px-2 text-[11px]"
                      disabled={demote.isPending}
                      onClick={() => demote.mutate(entry.memory_id)}
                    >
                      Undo
                    </Button>
                  )}
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
