"use client";

import Link from "next/link";
import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Badge, Card, Empty, ErrorNote, PageHeader, ProjectNav } from "@/components/ui";

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

  if (overview.error != null) return <ErrorNote error={overview.error} />;
  if (!overview.data) return <Empty>Loading…</Empty>;

  const { project, counts, branches, recent_sessions } = overview.data;

  return (
    <main>
      <PageHeader title={project.name} subtitle={project.repository_remote ?? undefined} />
      <ProjectNav id={id} active="overview" />

      <div className="mb-6 grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Stat label="Open tasks" value={counts.open_tasks} />
        <Stat label="Tasks" value={counts.tasks} />
        <Stat label="Sessions" value={counts.sessions} />
        <Stat label="Memories" value={counts.memories} />
      </div>

      <h2 className="mb-2 text-sm font-medium uppercase tracking-wide text-neutral-500">
        Active branches
      </h2>
      {branches.length === 0 && <Empty>No sessions recorded yet.</Empty>}
      <ul className="mb-6 space-y-1">
        {branches.map((b) => (
          <li key={b.branch} className="text-sm">
            <code>{b.branch}</code>
            <span className="ml-2 text-neutral-500">
              {b.sessions} session{b.sessions === 1 ? "" : "s"}
            </span>
          </li>
        ))}
      </ul>

      <h2 className="mb-2 text-sm font-medium uppercase tracking-wide text-neutral-500">
        Recent sessions
      </h2>
      <ul className="space-y-2">
        {recent_sessions.map((s) => (
          <li key={s.id}>
            <Link href={`/projects/${id}/sessions/${s.id}`}>
              <Card className="flex items-center justify-between transition hover:border-neutral-400">
                <span className="text-sm">
                  <code>{s.branch}</code>
                  <span className="ml-2 text-neutral-500">{s.agent}</span>
                </span>
                <Badge>{s.status}</Badge>
              </Card>
            </Link>
          </li>
        ))}
      </ul>
    </main>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <Card>
      <div className="text-2xl font-semibold">{value}</div>
      <div className="text-xs uppercase tracking-wide text-neutral-500">
        {label}
      </div>
    </Card>
  );
}
