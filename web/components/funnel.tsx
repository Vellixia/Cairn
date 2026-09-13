"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api, type FunnelStage } from "@/lib/api";
import { ApiErrorState, humanize } from "@/components/control-plane";
import { ListSkeleton } from "@/components/page";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

/**
 * The windows the dashboard offers, and the sentinel for "no window at all".
 *
 * `ALL` is a distinct option rather than a large number of days: the funnel
 * route treats an absent `days` as the project's whole history, and asking for
 * 3650 days instead would silently become 365 under the server's clamp.
 */
const ALL = "all";
const WINDOWS = [
  { value: ALL, label: "All time" },
  { value: "7", label: "Last 7 days" },
  { value: "30", label: "Last 30 days" },
  { value: "90", label: "Last 90 days" },
];

/**
 * The twelve-stage memory funnel (FR-879, FR-880).
 *
 * The stage list and its order come from the server, not from here. The order
 * is part of the contract — the funnel is read as a funnel, so a stage out of
 * sequence misreads as a stage that lost fewer records than it did — and a
 * second copy of the list in the browser would be a second place for it to
 * drift.
 */
export function MemoryFunnel({ projectId }: { projectId: string }) {
  const [window, setWindow] = useState(ALL);
  const days = window === ALL ? undefined : Number(window);

  const funnel = useQuery({
    queryKey: ["funnel", projectId, window],
    queryFn: () => api.funnel(projectId, days),
  });

  const unavailable =
    funnel.data?.stages.filter((s) => s.count === null).length ?? 0;

  return (
    <Card className="mb-6">
      <CardHeader className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <CardTitle className="text-sm font-medium">Memory funnel</CardTitle>
          <p className="text-muted-foreground mt-1 text-xs">
            What arrived, what consolidation made of it, and what was delivered
            back.
          </p>
        </div>
        <Select value={window} onValueChange={(v) => setWindow(v ?? ALL)}>
          <SelectTrigger
            className="w-40"
            data-testid="funnel-window"
            aria-label="Funnel window"
          >
            <SelectValue>
              {(v: string) =>
                WINDOWS.find((w) => w.value === v)?.label ?? "All time"
              }
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            {WINDOWS.map((w) => (
              <SelectItem key={w.value} value={w.value}>
                {w.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </CardHeader>
      <CardContent>
        {funnel.isLoading && <ListSkeleton rows={2} />}
        {funnel.error != null && <ApiErrorState error={funnel.error} />}

        {funnel.data && (
          <>
            <div
              className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4"
              data-testid="funnel"
            >
              {funnel.data.stages.map((stage) => (
                <FunnelTile key={stage.stage} stage={stage} />
              ))}
            </div>
            {unavailable > 0 && (
              <p
                className="text-muted-foreground mt-4 text-xs"
                data-testid="funnel-unavailable-note"
              >
                {unavailable} of {funnel.data.stages.length} stages are not
                established on this deployment: the mechanism behind them does
                not exist here, so nothing can be said either way. That is not
                the same as nothing having happened.
              </p>
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}

/**
 * One stage, with zero and unavailable kept apart (FR-880).
 *
 * **`count === null` must never become `0`.** Zero means the query ran against
 * the mechanism and found nothing; null means the mechanism does not exist on
 * this deployment. An operator acts differently on the two — one is a quiet
 * project, the other is a server that has not been migrated — so the tile
 * renders a number for one and an em dash with the words "not established" for
 * the other. A `?? 0` anywhere near this value would report "nobody looked" as
 * "nothing happened".
 */
function FunnelTile({ stage }: { stage: FunnelStage }) {
  const established = stage.count !== null;
  return (
    <div
      className="rounded-lg border p-3"
      data-testid={`funnel-stage-${stage.stage}`}
      data-count-state={established ? "counted" : "unavailable"}
    >
      {established ? (
        <div
          className="text-2xl font-semibold tabular-nums"
          data-testid={`funnel-count-${stage.stage}`}
        >
          {stage.count}
        </div>
      ) : (
        <div
          className="text-muted-foreground text-2xl font-semibold"
          data-testid={`funnel-count-${stage.stage}`}
          title="Not established on this deployment"
        >
          —
        </div>
      )}
      <div className="text-muted-foreground mt-0.5 text-xs">
        {humanize(stage.stage)}
      </div>
      {!established && (
        <div
          className="text-muted-foreground mt-1 text-[11px] italic"
          data-testid={`funnel-unavailable-${stage.stage}`}
        >
          Not established
        </div>
      )}
    </div>
  );
}
