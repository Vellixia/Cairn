"use client";

import Link from "next/link";
import { use } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { ApiErrorState } from "@/components/control-plane";
import { ListSkeleton, PageHeader, formatDate } from "@/components/page";
import { Button } from "@/components/ui/button";

/** Bounded trace history remains reachable even though compact nav surfaces its summary on Overview. */
export default function RetrievalsPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const traces = useInfiniteQuery({
    queryKey: ["retrieval-traces", id, "history"],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam }) => api.retrievalTraces(id, { cursor: pageParam }),
    getNextPageParam: (last) => last.cursor ?? undefined,
  });
  const rows = traces.data?.pages.flatMap((page) => page.traces) ?? [];
  return (
    <div data-testid="retrieval-history">
      <PageHeader title="Retrieval history" subtitle="Read-only delivery traces" />
      {traces.error != null && <ApiErrorState error={traces.error} />}
      {traces.isLoading && <ListSkeleton rows={5} />}
      {traces.data && (
        rows.length === 0 ? <p className="text-muted-foreground text-sm">No retrievals yet.</p> :
          <ul className="space-y-2" data-testid="retrieval-history-list">
            {rows.map((trace) => <li key={trace.trace_id}>
              <Link href={`/projects/${id}/retrievals/${trace.trace_id}`} className="hover:bg-accent/50 flex items-center justify-between gap-3 rounded-md px-3 py-2 text-sm transition" data-testid="retrieval-history-row" data-trace-id={trace.trace_id}>
                <span className="min-w-0 truncate">{trace.trigger} · {trace.delivery_state}</span>
                <span className="text-muted-foreground shrink-0 text-xs">{formatDate(trace.created_at)}</span>
              </Link>
            </li>)}
          </ul>
      )}
      {traces.hasNextPage && <div className="mt-4"><Button variant="outline" data-testid="trace-more" disabled={traces.isFetchingNextPage} onClick={() => traces.fetchNextPage()}>{traces.isFetchingNextPage ? "Loading…" : "Load more"}</Button></div>}
    </div>
  );
}
