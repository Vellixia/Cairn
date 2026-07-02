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
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Item,
  ItemActions,
  ItemContent,
  ItemTitle,
  ItemDescription,
} from "@/components/ui/item";
import {
  usePromotionCandidatesQuery,
  usePromoteMemoryMutation,
  useDismissPromotionMutation,
  useCronHistoryQuery,
} from "@/lib/queries";

export default function PromotionPage() {
  const candidates = usePromotionCandidatesQuery();
  const promote = usePromoteMemoryMutation();
  const dismiss = useDismissPromotionMutation();
  const history = useCronHistoryQuery();

  return (
    <div className="space-y-6 max-w-3xl">
      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">
            Promotion &amp; Intelligence
          </h1>
          <p className="mt-1 text-sm text-muted-foreground">
            What the nightly <code>llm-intelligence</code> job found worth reviewing, and what
            every background job did on its last run.
          </p>
        </div>
        <HelpButton content={HELP["/memory/promotion"]} />
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Promotion candidates</CardTitle>
          <CardDescription>
            {candidates.data
              ? `${candidates.data.length} candidate(s) . promo_score 0.70-0.90`
              : "Loading..."}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {candidates.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : candidates.data && candidates.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              Nothing waiting for review. Candidates appear here once the nightly
              <code> llm-intelligence</code> job scores a Project-scoped memory between 0.70 and
              0.90.
            </p>
          ) : (
            <ul className="space-y-2">
              {candidates.data?.map((m) => (
                <Item key={m.id} variant="outline" size="sm">
                  <ItemContent>
                    <ItemTitle className="line-clamp-2">{m.content}</ItemTitle>
                    <ItemDescription className="flex items-center gap-2 flex-wrap">
                      <Badge variant="outline" className="mr-1.5 font-mono text-[10px]">
                        {m.promo_score.toFixed(2)}
                      </Badge>
                      <Badge variant="outline" className="mr-1.5 font-mono text-[10px]">
                        {m.kind}
                      </Badge>
                      {m.tier}
                    </ItemDescription>
                  </ItemContent>
                  <ItemActions>
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={promote.isPending}
                      onClick={() => promote.mutate(m.id)}
                    >
                      Promote
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={dismiss.isPending}
                      onClick={() => dismiss.mutate(m.id)}
                    >
                      Dismiss
                    </Button>
                  </ItemActions>
                </Item>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Intelligence log</CardTitle>
          <CardDescription>Last 10 runs per job, newest last.</CardDescription>
        </CardHeader>
        <CardContent>
          {history.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          ) : history.data && history.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No jobs have run yet since the server started.
            </p>
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
    </div>
  );
}
