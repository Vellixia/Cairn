"use client";

import Link from "next/link";
import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, EyeOff } from "lucide-react";
import { api, type TraceDetail } from "@/lib/api";
import {
  ApiErrorState,
  Field,
  NotRecorded,
  humanize,
} from "@/components/control-plane";
import { ListSkeleton, PageHeader, formatDate } from "@/components/page";
import { ReferenceChip } from "@/components/reference";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

export default function TraceDetailPage({
  params,
}: {
  params: Promise<{ id: string; traceId: string }>;
}) {
  const { id, traceId } = use(params);
  const trace = useQuery({
    queryKey: ["retrieval-trace", traceId],
    queryFn: () => api.retrievalTrace(traceId),
  });

  return (
    <div>
      <Button
        variant="ghost"
        size="sm"
        className="mb-3 -ml-2"
        render={<Link href={`/projects/${id}/retrievals`} />}
      >
        <ArrowLeft />
        All retrievals
      </Button>

      <PageHeader title="Retrieval" subtitle={traceId} />

      {trace.isLoading && <ListSkeleton rows={4} />}
      {trace.error != null && <ApiErrorState error={trace.error} />}

      {trace.data && (
        <div className="space-y-4" data-testid="trace-detail">
          <Trigger trace={trace.data} projectId={id} />
          <Budget trace={trace.data} />
          <Selection trace={trace.data} projectId={id} />
          <BriefingNotice />
        </div>
      )}
    </div>
  );
}

/** What asked for context, and what happened to the answer. */
function Trigger({
  trace,
  projectId,
}: {
  trace: TraceDetail;
  projectId: string;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm font-medium">Trigger and delivery</CardTitle>
      </CardHeader>
      <CardContent>
        <dl className="grid gap-3 sm:grid-cols-3">
          <Field
            label="Trigger"
            testId="detail-trigger"
            value={<Badge variant="secondary">{humanize(trace.trigger)}</Badge>}
          />
          <Field
            label="Delivery point"
            testId="detail-delivery-point"
            value={humanize(trace.delivery_point)}
          />
          <Field
            label="Delivery state"
            testId="detail-delivery-state"
            value={
              <Badge
                variant={
                  trace.delivery_state === "failed" ? "destructive" : "outline"
                }
              >
                {humanize(trace.delivery_state)}
              </Badge>
            }
          />
          <Field
            label="Degradation level"
            testId="detail-degradation"
            value={
              trace.degradation_level ? (
                humanize(trace.degradation_level)
              ) : (
                // A trace that never generated a briefing has no level. That is
                // a different answer from "level 0", which claims a complete
                // briefing was assembled.
                <NotRecorded what="no briefing was built" />
              )
            }
          />
          <Field
            label="Receipt"
            testId="detail-acknowledgement"
            value={humanize(trace.acknowledgement_state)}
          />
          <Field
            label="Session"
            testId="detail-session"
            value={
              <Link
                href={`/projects/${projectId}/sessions/${trace.session_id}`}
                className="font-mono text-xs hover:underline"
              >
                {trace.session_id.slice(0, 8)}
              </Link>
            }
          />
          <Field label="At" value={formatDate(trace.created_at)} />
          {trace.failure_reason && (
            <Field
              label="Failure reason"
              testId="detail-failure-reason"
              value={
                <span className="text-destructive">
                  {humanize(trace.failure_reason)}
                </span>
              }
            />
          )}
        </dl>
      </CardContent>
    </Card>
  );
}

/**
 * What the retrieval cost, when the reader is the one who paid it.
 *
 * The server sends `budget` and `latency_ms` only to the account that made the
 * retrieval, so their absence is a withholding rather than a zero — and the two
 * must not look alike. A co-member reading a colleague's trace is told the
 * accounting is scoped elsewhere; rendering "0 tokens" would be a false number.
 */
function Budget({ trace }: { trace: TraceDetail }) {
  const withheld = trace.budget === undefined;
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm font-medium">Budget</CardTitle>
      </CardHeader>
      <CardContent>
        {withheld ? (
          <p className="text-muted-foreground text-sm" data-testid="budget-withheld">
            Budget and latency are scoped to the account that made this
            retrieval, and this is not yours. What was selected is a project
            fact; what it cost is not.
          </p>
        ) : (
          <dl className="grid gap-3 sm:grid-cols-3">
            <Field
              label="Budget"
              testId="budget-tokens"
              value={
                trace.budget?.tokens ?? <NotRecorded what="not recorded" />
              }
            />
            <Field
              label="Spent"
              testId="budget-spent"
              value={trace.budget?.spent ?? <NotRecorded what="not recorded" />}
            />
            <Field
              label="Latency"
              testId="budget-latency"
              value={
                trace.latency_ms == null ? (
                  <NotRecorded what="never generated" />
                ) : (
                  `${trace.latency_ms} ms`
                )
              }
            />
          </dl>
        )}
      </CardContent>
    </Card>
  );
}

/**
 * What was considered and what was selected, as references.
 *
 * Every row is a complete reference — a domain and an id for knowledge, a bare
 * pattern id for a pattern — because two domains can hold the same UUID and a
 * row that printed only the id would name nothing.
 *
 * A record the reader may not see is not in this list at all. The server drops
 * it rather than showing an opaque id, because an id still discloses that
 * somebody else's personal record exists and was retrieved here (FR-846a). So
 * the count below is what this reader may see, not what the retrieval weighed.
 */
function Selection({
  trace,
  projectId,
}: {
  trace: TraceDetail;
  projectId: string;
}) {
  const selected = trace.items.filter((i) => i.status === "selected").length;
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm font-medium">
          Selection ({selected} selected of {trace.items.length} considered)
        </CardTitle>
      </CardHeader>
      <CardContent>
        {trace.items.length === 0 ? (
          <p className="text-muted-foreground text-sm">
            Nothing in this retrieval is visible to you.
          </p>
        ) : (
          <ul className="space-y-2" data-testid="trace-items">
            {trace.items.map((item) => (
              <li
                key={item.reference_key}
                className="flex flex-wrap items-center gap-2 text-sm"
                data-testid="trace-item"
              >
                <span className="text-muted-foreground w-6 text-right font-mono text-xs">
                  {item.rank}
                </span>
                <Badge
                  variant={item.status === "selected" ? "default" : "outline"}
                  data-testid="trace-item-status"
                >
                  {item.status}
                </Badge>
                <ReferenceChip reference={item} projectId={projectId} />
                <span
                  className="text-muted-foreground text-xs"
                  data-testid="trace-item-rule"
                >
                  {/* FR-886's "the selection's explanation". Null means the row
                      predates the rule being recorded, not that no rule ran. */}
                  {item.selection_rule ? (
                    humanize(item.selection_rule)
                  ) : (
                    <NotRecorded what="rule not recorded" />
                  )}
                </span>
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

/**
 * The one thing this page will never show.
 *
 * Stated rather than silently omitted so a reader knows the absence is
 * deliberate. The assembled briefing is not withheld from the view — the server
 * never stored one, so there is nothing to withhold and nothing to leak
 * (FR-839, FR-886). Reconstructing it here from the selected items would
 * recreate exactly the artifact that decision avoids.
 */
function BriefingNotice() {
  return (
    <p
      className="text-muted-foreground flex items-start gap-2 text-xs"
      data-testid="no-briefing-notice"
    >
      <EyeOff className="mt-0.5 size-3.5 shrink-0" />
      <span>
        The briefing text itself is not shown and is not stored. This page
        records which records were selected and what the selection cost, never
        the prose that was assembled from them.
      </span>
    </p>
  );
}
