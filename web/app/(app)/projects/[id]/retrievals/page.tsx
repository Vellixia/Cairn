"use client";

import Link from "next/link";
import { Suspense, use } from "react";
import { useSearchParams } from "next/navigation";
import { useInfiniteQuery } from "@tanstack/react-query";
import { ArrowDownToLine, FilterX } from "lucide-react";
import { api, type TraceSummary } from "@/lib/api";
import { ApiErrorState, NotRecorded, humanize } from "@/components/control-plane";
import {
  EmptyState,
  ListSkeleton,
  PageHeader,
  formatDate,
} from "@/components/page";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";

/** Red for a failure, plain for everything else — no colour claims success. */
function deliveryVariant(state: TraceSummary["delivery_state"]) {
  if (state === "failed") return "destructive" as const;
  if (state === "transmitted") return "default" as const;
  return "outline" as const;
}

/**
 * The filters live in the URL, so the list reads them through
 * `useSearchParams` — which suspends during prerender and has to be under a
 * boundary, or `next build` refuses the whole route.
 */
export default function RetrievalsPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  return (
    <Suspense fallback={<ListSkeleton rows={4} />}>
      <Retrievals projectId={id} />
    </Suspense>
  );
}

function Retrievals({ projectId: id }: { projectId: string }) {
  const search = useSearchParams();

  // Both filters come from the URL rather than from component state, so memory
  // detail's "view all" link and a session's own link are shareable and survive
  // a reload.
  const referenceKey = search.get("reference_key") ?? undefined;
  const sessionId = search.get("session_id") ?? undefined;
  const filtered = referenceKey !== undefined || sessionId !== undefined;

  const traces = useInfiniteQuery({
    queryKey: ["retrieval-traces", id, referenceKey, sessionId],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam }) =>
      api.retrievalTraces(id, {
        cursor: pageParam,
        reference_key: referenceKey,
        session_id: sessionId,
      }),
    getNextPageParam: (last) => last.cursor ?? undefined,
  });

  const rows = traces.data?.pages.flatMap((p) => p.traces) ?? [];

  return (
    <div>
      <PageHeader
        title="Retrievals"
        subtitle="What each recall considered, selected and delivered"
      >
        {filtered && (
          <Button
            variant="outline"
            size="sm"
            data-testid="retrievals-clear-filter"
            render={<Link href={`/projects/${id}/retrievals`} />}
          >
            <FilterX />
            Clear filter
          </Button>
        )}
      </PageHeader>

      {filtered && (
        <p
          className="text-muted-foreground mb-4 text-xs"
          data-testid="retrievals-filter"
        >
          Filtered to{" "}
          {referenceKey && (
            <code className="font-mono">{referenceKey}</code>
          )}
          {referenceKey && sessionId && " and "}
          {sessionId && <code className="font-mono">session {sessionId}</code>}.
        </p>
      )}

      {traces.isLoading && <ListSkeleton rows={4} />}
      {traces.error != null && <ApiErrorState error={traces.error} />}

      {traces.data && rows.length === 0 && (
        <EmptyState
          title="No retrievals"
          description={
            filtered
              ? // Deliberately one message for two cases. A reference nobody
                // retrieved and a reference this reader may not see produce the
                // same empty page on the server, and saying which would answer
                // "does this record exist" for somebody else's knowledge
                // (FR-846a).
                "Nothing here matches that filter."
              : "Retrievals appear once an agent asks Cairn for context."
          }
        />
      )}

      <ul className="space-y-2" data-testid="trace-list">
        {rows.map((t) => (
          <li key={t.trace_id}>
            <Link
              href={`/projects/${id}/retrievals/${t.trace_id}`}
              data-testid="trace-row"
              data-trace-id={t.trace_id}
            >
              <Card className="hover:border-foreground/20 hover:bg-accent/40 transition">
                <CardContent>
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge variant="secondary" data-testid="trace-trigger">
                      {humanize(t.trigger)}
                    </Badge>
                    <Badge variant="outline" data-testid="trace-delivery-point">
                      {humanize(t.delivery_point)}
                    </Badge>
                    <Badge
                      variant={deliveryVariant(t.delivery_state)}
                      data-testid="trace-delivery-state"
                    >
                      {humanize(t.delivery_state)}
                    </Badge>
                    <span
                      className="text-muted-foreground ml-auto text-xs"
                      data-testid="trace-created-at"
                    >
                      {formatDate(t.created_at)}
                    </span>
                  </div>
                  <div className="text-muted-foreground mt-2 flex flex-wrap items-center gap-3 text-xs">
                    <span data-testid="trace-degradation">
                      {/* Null is honest: a briefing that was never built has no
                          degradation level, which is not the same as level 0. */}
                      {t.degradation_level ? (
                        `degradation ${humanize(t.degradation_level)}`
                      ) : (
                        <NotRecorded what="no briefing built" />
                      )}
                    </span>
                    <span data-testid="trace-acknowledgement">
                      receipt {humanize(t.acknowledgement_state)}
                    </span>
                    <span>
                      session{" "}
                      <code className="font-mono">
                        {t.session_id.slice(0, 8)}
                      </code>
                    </span>
                    {t.failure_reason && (
                      <span
                        className="text-destructive"
                        data-testid="trace-failure-reason"
                      >
                        {humanize(t.failure_reason)}
                      </span>
                    )}
                  </div>
                </CardContent>
              </Card>
            </Link>
          </li>
        ))}
      </ul>

      {traces.hasNextPage && (
        <div className="mt-4 flex justify-center">
          <Button
            variant="outline"
            size="sm"
            data-testid="trace-more"
            disabled={traces.isFetchingNextPage}
            onClick={() => traces.fetchNextPage()}
          >
            <ArrowDownToLine />
            {traces.isFetchingNextPage ? "Loading…" : "Load more"}
          </Button>
        </div>
      )}
    </div>
  );
}
