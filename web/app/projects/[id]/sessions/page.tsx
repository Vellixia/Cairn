"use client";

import Link from "next/link";
import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Badge, Card, Empty, ErrorNote, PageHeader, ProjectNav } from "@/components/ui";

export default function SessionsPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const sessions = useQuery({
    queryKey: ["sessions", id],
    queryFn: () => api.sessions(id),
  });

  return (
    <main>
      <PageHeader title="Sessions" subtitle="Newest first" />
      <ProjectNav id={id} active="sessions" />

      {sessions.error != null && <ErrorNote error={sessions.error} />}
      {sessions.data?.sessions.length === 0 && <Empty>No sessions yet.</Empty>}
      <ul className="space-y-2" data-testid="session-list">
        {sessions.data?.sessions.map((s) => (
          <li key={s.id}>
            <Link href={`/projects/${id}/sessions/${s.id}`}>
              <Card className="flex items-center justify-between gap-3 transition hover:border-neutral-400">
                <div className="text-sm">
                  <code>{s.branch}</code>
                  <span className="ml-2 text-neutral-500">{s.agent}</span>
                  <span className="ml-2 text-neutral-400">
                    {new Date(s.started_at).toLocaleString()}
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  {s.has_handoff && <Badge>handoff</Badge>}
                  <Badge>{s.status}</Badge>
                </div>
              </Card>
            </Link>
          </li>
        ))}
      </ul>
    </main>
  );
}
