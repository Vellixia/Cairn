"use client";

import Link from "next/link";
import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { GitBranch } from "lucide-react";
import { api } from "@/lib/api";
import {
  ErrorState,
  ListSkeleton,
  PageHeader,
  formatDate,
} from "@/components/page";
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

  // The header renders before the data does. Returning early instead made the
  // whole page swap out and jump once the request landed.
  const project = overview.data?.project;

  return (
    <div>
      <PageHeader
        title={project?.name ?? "Project"}
        subtitle={project?.repository_remote ?? undefined}
      />

      {overview.error != null && <ErrorState error={overview.error} />}
      {overview.isLoading && <ListSkeleton rows={4} />}

      {overview.data && (
        <>
          <div className="mb-6 grid grid-cols-2 gap-3 lg:grid-cols-4">
            <Stat
              label="Open tasks"
              value={overview.data.counts.open_tasks}
              href={`/projects/${id}/tasks`}
            />
            <Stat
              label="Tasks"
              value={overview.data.counts.tasks}
              href={`/projects/${id}/tasks`}
            />
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
        </>
      )}
    </div>
  );
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
