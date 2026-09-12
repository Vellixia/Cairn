"use client";

import { useQuery } from "@tanstack/react-query";
import { api, type SystemHealth } from "@/lib/api";
import { ApiErrorState, Field, humanize } from "@/components/control-plane";
import { ListSkeleton, PageHeader, formatDate } from "@/components/page";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

/**
 * Deployment-wide ingest, consolidation and retrieval health (FR-891).
 *
 * **The page does not check who is reading it.** `GET /api/system/health` takes
 * an administrator and refuses everyone else before its handler runs, so a
 * member who types this URL gets the server's refusal rendered as a refusal.
 * A client-side redirect would only hide the link; it would not be the guard,
 * and pretending otherwise is what FR-892 forbids.
 */
export default function SystemPage() {
  const health = useQuery({
    queryKey: ["system-health"],
    queryFn: () => api.systemHealth(),
  });

  return (
    <div>
      <PageHeader
        title="System health"
        subtitle="Ingest, consolidation and retrieval across every project on this server"
      />

      {health.isLoading && <ListSkeleton rows={3} />}
      {health.error != null && <ApiErrorState error={health.error} />}

      {health.data && (
        <div className="space-y-4" data-testid="system-health">
          <Ingest ingest={health.data.ingest} />
          <Consolidation consolidation={health.data.consolidation} />
          <Retrieval retrieval={health.data.retrieval} />
        </div>
      )}
    </div>
  );
}

/**
 * A whole section the deployment cannot answer for.
 *
 * Null is not zero here either: a server whose schema predates the tables
 * behind a section has not observed nothing, it has observed nothing yet
 * knowable. Rendering zeroes would report a healthy, idle system where the
 * truth is a server that has not been migrated (FR-880).
 */
function Unavailable({ what }: { what: string }) {
  return (
    <p className="text-muted-foreground text-sm" data-testid="section-unavailable">
      {what} is not established on this deployment: the tables behind it do not
      exist here, so nothing can be said either way.
    </p>
  );
}

function Ingest({ ingest }: { ingest: SystemHealth["ingest"] }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm font-medium">Ingest</CardTitle>
      </CardHeader>
      <CardContent data-testid="system-ingest">
        {ingest === null ? (
          <Unavailable what="Ingest" />
        ) : (
          <>
            <dl className="grid gap-3 sm:grid-cols-3">
              <Field
                label="Events received"
                testId="ingest-events"
                value={ingest.events_received}
              />
              <Field
                label="Capture failures"
                testId="ingest-failures"
                value={ingest.capture_failures}
              />
              <Field
                label="Last received"
                value={formatDate(ingest.last_received_at)}
              />
            </dl>
            {ingest.failures_by_disposition.length > 0 && (
              <div className="mt-4">
                <p className="text-muted-foreground text-xs">
                  {/* Broken out because the dispositions call for different
                      actions: a spool overflow is a capacity problem, a
                      redaction failure is a correctness one. */}
                  Failures by disposition
                </p>
                <div
                  className="mt-2 flex flex-wrap gap-2"
                  data-testid="ingest-dispositions"
                >
                  {ingest.failures_by_disposition.map((d) => (
                    <Badge key={d.disposition} variant="outline">
                      {humanize(d.disposition)}: {d.n}
                    </Badge>
                  ))}
                </div>
              </div>
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}

function Consolidation({
  consolidation,
}: {
  consolidation: SystemHealth["consolidation"];
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm font-medium">Consolidation</CardTitle>
      </CardHeader>
      <CardContent data-testid="system-consolidation">
        {consolidation === null ? (
          <Unavailable what="Consolidation" />
        ) : (
          <dl className="grid gap-3 sm:grid-cols-3">
            <Field
              label="Backlog"
              testId="consolidation-backlog"
              value={consolidation.backlog_depth}
            />
            <Field
              label="Oldest waiting"
              testId="consolidation-oldest"
              value={
                // "No backlog" and "waiting since" are different answers, and
                // `formatDate` already prints "never" for the first.
                formatDate(consolidation.oldest_enqueued_at)
              }
            />
            <Field
              label="Failed events"
              testId="consolidation-failed"
              value={consolidation.failed_events}
            />
            <Field
              label="Runs finished"
              value={consolidation.runs_finished}
            />
            <Field label="Runs failed" value={consolidation.runs_failed} />
            <Field
              label="Candidates"
              value={`${consolidation.candidates_accepted} accepted of ${consolidation.candidates_proposed} proposed, ${consolidation.candidates_refused} refused`}
            />
          </dl>
        )}
      </CardContent>
    </Card>
  );
}

function Retrieval({ retrieval }: { retrieval: SystemHealth["retrieval"] }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm font-medium">Retrieval</CardTitle>
      </CardHeader>
      <CardContent data-testid="system-retrieval">
        {retrieval === null ? (
          <Unavailable what="Retrieval" />
        ) : (
          <>
            <dl className="grid gap-3 sm:grid-cols-3">
              <Field
                label="Traces"
                testId="retrieval-traces"
                value={retrieval.traces}
              />
              <Field
                label="Failed"
                testId="retrieval-failed"
                value={retrieval.failed}
              />
              <Field
                label="Transmitted"
                value={retrieval.transmitted}
              />
              <Field
                label="Never generated"
                testId="retrieval-never-generated"
                value={retrieval.never_generated}
              />
              <Field
                label="Never transmitted"
                testId="retrieval-never-transmitted"
                value={retrieval.never_transmitted}
              />
              <Field
                label="Last trace"
                value={formatDate(retrieval.last_trace_at)}
              />
            </dl>
            <p className="text-muted-foreground mt-3 text-xs">
              {/* Two different backlogs, deliberately not one number: retrieval
                  that never finished, and a briefing nobody confirmed reached
                  an agent. */}
              A trace still awaiting generation is retrieval not finishing. One
              generated but never reported transmitted is a briefing whose
              arrival nobody confirmed.
            </p>
          </>
        )}
      </CardContent>
    </Card>
  );
}
