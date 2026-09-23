"use client";

import Link from "next/link";
import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { GitBranch } from "lucide-react";
import { api } from "@/lib/api";
import { ApiErrorState } from "@/components/control-plane";
import { ListSkeleton, PageHeader, formatDate } from "@/components/page";
import { StatusBadge } from "@/components/session";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

export default function ProjectOverviewPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const overview = useQuery({
    queryKey: ["project", id],
    queryFn: () => api.project(id),
  });
  // Compact overview panels use the same membership-protected APIs as the
  // former standalone pages. They summarize only bounded first pages; neither
  // client state nor this composition makes an authorization decision.
  const activity = useQuery({
    queryKey: ["activity", id, "overview"],
    queryFn: () => api.activity(id, { limit: 5 }),
  });
  const retrievals = useQuery({
    queryKey: ["retrieval-traces", id, "overview"],
    queryFn: () => api.retrievalTraces(id, { limit: 5 }),
  });
  const health = useQuery({
    queryKey: ["integration-health", id, "overview"],
    queryFn: () => api.integrationHealth(id),
  });
  const analytics = useQuery({ queryKey: ["analytics", id], queryFn: () => api.analytics(id) });
  // The header renders before the data does. Returning early instead made the
  // whole page swap out and jump once the request landed.
  const project = overview.data?.project;

  return (
    <div>
      <PageHeader
        title={project?.name ?? "Project"}
        subtitle={project?.repository_remote ?? undefined}
      />

      {/* The funnel loads on its own request. A project overview that waited for
          both would show nothing until the slower of the two landed, and the
          funnel is the slower one — twelve counts over the whole history. */}
      {overview.error != null && <ApiErrorState error={overview.error} />}
      {overview.isLoading && <ListSkeleton rows={4} />}

      {overview.data && (
        <>
          <div className="mb-6 grid grid-cols-2 gap-3 lg:grid-cols-4">
            <Stat
              label="Sessions"
              value={overview.data.counts.sessions}
              href={`/projects/${id}/sessions`}
            />
            <Stat
              label="Memories"
              value={overview.data.counts.memories}
              href={`/projects/${id}/memory`}
            />
          </div>

          <div className="grid gap-6 lg:grid-cols-2">
            <Card>
              <CardHeader>
                <CardTitle className="text-sm font-medium">
                  Active branches
                </CardTitle>
              </CardHeader>
              <CardContent>
                {overview.data.branches.length === 0 ? (
                  <p className="text-muted-foreground text-sm">
                    No sessions recorded yet.
                  </p>
                ) : (
                  <ul className="space-y-2">
                    {overview.data.branches.map((b) => (
                      <li
                        key={b.branch}
                        className="flex items-center justify-between gap-3 text-sm"
                      >
                        <span className="flex min-w-0 items-center gap-2">
                          <GitBranch className="text-muted-foreground size-3.5 shrink-0" />
                          <code className="truncate font-mono text-xs">
                            {b.branch}
                          </code>
                        </span>
                        <span className="text-muted-foreground shrink-0 text-xs">
                          {b.sessions} session{b.sessions === 1 ? "" : "s"}
                          {b.last_seen && ` · ${formatDate(b.last_seen)}`}
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="text-sm font-medium">
                  Recent sessions
                </CardTitle>
              </CardHeader>
              <CardContent>
                {overview.data.recent_sessions.length === 0 ? (
                  <p className="text-muted-foreground text-sm">
                    No sessions yet.
                  </p>
                ) : (
                  <ul className="space-y-1">
                    {overview.data.recent_sessions.map((s) => (
                      <li key={s.id}>
                        <Link
                          href={`/projects/${id}/sessions/${s.id}`}
                          className="hover:bg-accent/50 flex items-center justify-between gap-3 rounded-md px-2 py-1.5 transition"
                        >
                          <span className="min-w-0 truncate text-sm">
                            <code className="font-mono text-xs">
                              {s.branch}
                            </code>
                            <span className="text-muted-foreground ml-2">
                              {s.agent}
                            </span>
                          </span>
                          <StatusBadge status={s.status} />
                        </Link>
                      </li>
                    ))}
                  </ul>
                )}
              </CardContent>
            </Card>
          </div>

          <div className="mt-6 grid gap-6 lg:grid-cols-2">
            <OverviewPanel id="activity" title="Activity" testId="overview-activity">
              <SummaryRows
                empty="No accepted activity yet."
                rows={activity.data?.items.map((item) => `${item.kind} · ${formatDate(item.at)}`)}
              />
            </OverviewPanel>
            <OverviewPanel id="retrieval" title="Retrieval" testId="overview-retrieval">
              <SummaryRows empty="No retrievals yet." rows={analytics.data ? [`${analytics.data.delivery} delivered of ${analytics.data.retrieval}`, `${analytics.data.failures} failed`, analytics.data.latency.count ? `${Math.round(analytics.data.latency.average_ms)}ms average` : "No latency reports"] : retrievals.data?.traces.map((trace) => `${trace.trigger} · ${trace.delivery_state}`)} />
            </OverviewPanel>
            <OverviewPanel id="agents" title="Agent health" testId="overview-agent-health">
              <SummaryRows empty="No installation report yet; current health is unknown." rows={health.data?.rows.slice(0, 5).map((row) => `${row.agent} · ${row.capability} · ${row.observed_at ? `reported ${formatDate(row.observed_at)}` : "never reported"}${row.observed_at && Date.now() - new Date(row.observed_at).getTime() > 7 * 86400000 ? " · stale" : ""}`)} />
            </OverviewPanel>
          </div>
        </>
      )}
    </div>
  );
}

function OverviewPanel({ id, title, testId, children }: { id: string; title: string; testId: string; children: React.ReactNode }) {
  return <Card id={id} data-testid={testId}><CardHeader><CardTitle className="text-sm font-medium">{title}</CardTitle></CardHeader><CardContent>{children}</CardContent></Card>;
}

function SummaryRows({ rows, empty }: { rows?: string[]; empty: string }) {
  if (!rows) return <p className="text-muted-foreground text-sm">Loading…</p>;
  if (rows.length === 0) return <p className="text-muted-foreground text-sm">{empty}</p>;
  return <ul className="space-y-1 text-sm">{rows.map((row, index) => <li key={`${row}-${index}`} className="truncate">{row}</li>)}</ul>;
}

/** A count nobody can click is a dead end; each one opens its own section. */
function Stat({
  label,
  value,
  href,
}: {
  label: string;
  value: number;
  href: string;
}) {
  return (
    <Link href={href} className="block">
      <Card className="hover:border-foreground/20 hover:bg-accent/40 gap-0 p-4 transition">
        <div className="text-2xl font-semibold tabular-nums">{value}</div>
        <div className="text-muted-foreground text-xs">{label}</div>
      </Card>
    </Link>
  );
}
